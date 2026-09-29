//! Alert sounds. One player thread owns the audio output; everything else
//! sends it requests, so a slow device or a large file never holds up the
//! runner or the settings window.
//!
//! The rules (one sound at a time, a quiet time after each, mentions cut
//! through) are `eve_chatterer_core::audio::SoundGate`; this is the playing.

use eve_chatterer_core::audio::{Play, Source, SoundGate};
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, Sink, Source as _};
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A chosen file is cut off after this, so a long track picked by mistake
/// can't hold the one sound slot for minutes.
const MAX_LENGTH: Duration = Duration::from_secs(15);
const RATE: u32 = 44_100;
const PEAK: f32 = 0.5;

enum Request {
    Alert { source: Source, gain: f32, cooldown: Duration, mention: bool },
    /// The Audio page's play button: always plays, never counts as an alert.
    Preview { source: Source, gain: f32 },
}

static PLAYER: OnceLock<Mutex<Sender<Request>>> = OnceLock::new();

pub fn start() {
    let (tx, rx) = channel::<Request>();
    let spawned = std::thread::Builder::new().name("audio".into()).spawn(move || {
        let mut gate = SoundGate::new();
        // Opened per sound and dropped when it ends, so a change of default
        // device (headphones plugged in) is picked up by the next sound.
        let mut current: Option<(OutputStream, Sink)> = None;
        loop {
            let req = match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => {
                    if current.as_ref().is_some_and(|(_, s)| s.empty()) {
                        current = None;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let playing = current.as_ref().is_some_and(|(_, s)| !s.empty());
            let (source, gain) = match req {
                Request::Alert { source, gain, cooldown, mention } => match gate.check(Instant::now(), cooldown, mention, playing) {
                    Play::Skip => continue,
                    Play::Start | Play::Interrupt => (source, gain),
                },
                Request::Preview { source, gain } => (source, gain),
            };
            current = None; // stops anything still playing
            match play(&source, gain) {
                Ok(p) => current = Some(p),
                Err(e) => eprintln!("could not play a sound: {e}"),
            }
        }
    });
    match spawned {
        Ok(_) => {
            let _ = PLAYER.set(Mutex::new(tx));
        }
        Err(e) => eprintln!("could not start the audio thread: {e}"),
    }
}

fn send(r: Request) {
    if let Some(tx) = PLAYER.get() {
        let _ = tx.lock().unwrap().send(r);
    }
}

/// An alert's sound, subject to the cooldown.
pub fn alert(source: Source, gain: f32, cooldown: Duration, mention: bool) {
    send(Request::Alert { source, gain, cooldown, mention });
}

pub fn preview(source: Source, gain: f32) {
    send(Request::Preview { source, gain });
}

fn play(source: &Source, gain: f32) -> Result<(OutputStream, Sink), String> {
    let (stream, handle) = OutputStream::try_default().map_err(|e| format!("no audio output: {e}"))?;
    let sink = Sink::try_new(&handle).map_err(|e| e.to_string())?;
    sink.set_volume(gain);
    match source {
        Source::BuiltIn => sink.append(built_in_tone()),
        Source::File(path) => match open(path) {
            Ok(d) => sink.append(d.convert_samples::<f32>().take_duration(MAX_LENGTH)),
            Err(e) => {
                eprintln!("could not play {path}, using the built-in sound instead: {e}");
                sink.append(built_in_tone());
            }
        },
    }
    Ok((stream, sink))
}

fn open(path: &str) -> Result<Decoder<BufReader<File>>, String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    Decoder::new(BufReader::new(f)).map_err(|e| e.to_string())
}

/// The built-in sound: two clean synth tones a fifth apart (F5, C6), each
/// swelling in, holding and releasing rather than decaying like something
/// struck (which is what made earlier attempts sound like instruments), with
/// a slightly detuned layer for sheen and a soft, diffuse tail. Made here
/// rather than shipped as a file, so there is nothing to license or lose.
fn built_in_tone() -> SamplesBuffer<f32> {
    SamplesBuffer::new(1, RATE, tone_samples())
}

fn tone_samples() -> Vec<f32> {
    use std::f32::consts::TAU;
    let rate = RATE as f32;
    let len = (0.7 * rate) as usize;
    let mut dry = vec![0.0f32; len];
    // (start s, held length s, Hz, level)
    const TONES: [(f32, f32, f32, f32); 2] = [(0.0, 0.06, 698.46, 0.8), (0.085, 0.11, 1046.5, 1.0)];
    const ATTACK: f32 = 0.002;
    const RELEASE: f32 = 0.02;
    for (start, held, freq, level) in TONES {
        let first = (start * rate) as usize;
        let n = ((held + RELEASE) * rate) as usize;
        for (k, s) in dry.iter_mut().skip(first).take(n).enumerate() {
            let t = k as f32 / rate;
            // Smooth (raised-cosine) swell and release around a flat hold.
            let rise = 0.5 - 0.5 * (std::f32::consts::PI * (t / ATTACK).min(1.0)).cos();
            let fall = if t <= held { 1.0 } else { 0.5 + 0.5 * (std::f32::consts::PI * ((t - held) / RELEASE).min(1.0)).cos() };
            let w = TAU * freq * t;
            let detuned = TAU * freq * 1.004 * t; // about 7 cents: a gentle chorus
            let tone = 0.58 * w.sin() + 0.26 * detuned.sin() + 0.16 * (2.0 * w).sin();
            *s += level * rise * fall * tone;
        }
    }
    // A small, soft space: several quiet, darkened echoes close together
    // blur into a short tail instead of audible repeats.
    let mut dark = dry.clone();
    let a = (-TAU * 3000.0 / rate).exp();
    let mut y = 0.0;
    for s in dark.iter_mut() {
        y = (1.0 - a) * *s + a * y;
        *s = y;
    }
    let mut out = dry.clone();
    for (delay, gain) in [(0.023f32, 0.16f32), (0.041, 0.13), (0.067, 0.1), (0.097, 0.08), (0.139, 0.06), (0.191, 0.04)] {
        let d = (delay * rate) as usize;
        for i in d..len {
            out[i] += dark[i - d] * gain;
        }
    }
    // Fade the tail to exact silence.
    let fade = (0.08 * RATE as f32) as usize;
    for (k, s) in out.iter_mut().rev().take(fade).enumerate() {
        *s *= k as f32 / fade as f32;
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        // Short bright blips sound louder than their level; 0.5 sits near an
        // ordinary notification sound at the same volume.
        out.iter_mut().for_each(|s| *s *= PEAK / peak);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_tone_is_short_and_never_clips() {
        let s = tone_samples();
        assert!(s.len() < RATE as usize);
        assert!(s.iter().all(|x| x.abs() <= PEAK + 1e-6));
        assert!(s[0].abs() < 0.01 && s.last().unwrap().abs() < 1e-6, "starts and ends quietly");
    }
}
