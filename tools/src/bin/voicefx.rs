//! Tries a "ship computer" treatment on a spoken alert (BACKLOG: spoken
//! alerts). Plays each WAV given twice: as recorded, then through the filter.
//!
//!   cargo run -p eve-chatterer-tools --bin voicefx -- <speech.wav>...
//!
//! Make a WAV with Windows' own voices, e.g. in PowerShell:
//!   Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
//!   $s.SelectVoice("Microsoft Zira Desktop"); $s.SetOutputToWaveFile("zira.wav"); $s.Speak("..."); $s.Dispose()

use rodio::source::SineWave;
use rodio::{Decoder, OutputStream, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::time::Duration;

fn open(path: &str) -> impl Source<Item = f32> + Clone + Send + 'static {
    let f = File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Decoder::new(BufReader::new(f)).unwrap_or_else(|e| panic!("{path}: {e}")).convert_samples::<f32>().buffered()
}

/// Radio band (cuts the boom and the air), a short soft chirp in front like a
/// comms channel opening, and two quiet close echoes for a hard metal room.
fn ship_computer(voice: impl Source<Item = f32> + Clone + Send + 'static) -> impl Source<Item = f32> + Send + 'static {
    let band = voice.high_pass(380).low_pass(3200).amplify(1.6).buffered();
    let echo1 = band.clone().delay(Duration::from_millis(45)).amplify(0.28);
    let echo2 = band.clone().delay(Duration::from_millis(110)).amplify(0.14);
    let chirp = SineWave::new(1480.0)
        .take_duration(Duration::from_millis(45))
        .amplify(0.12)
        .mix(SineWave::new(1975.0).take_duration(Duration::from_millis(45)).delay(Duration::from_millis(55)).amplify(0.12));
    chirp.then_after(Duration::from_millis(130), band.mix(echo1).mix(echo2))
}

/// `a` then `b` starting `gap` after `a` begins.
trait ThenAfter: Source<Item = f32> + Sized + Send + 'static {
    fn then_after<S: Source<Item = f32> + Send + 'static>(self, gap: Duration, b: S) -> rodio::source::Mix<Self, rodio::source::Delay<S>> {
        self.mix(b.delay(gap))
    }
}
impl<T: Source<Item = f32> + Sized + Send + 'static> ThenAfter for T {}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: voicefx <speech.wav>...");
        std::process::exit(2);
    }
    let (_stream, handle) = OutputStream::try_default().expect("no audio output");
    let sink = Sink::try_new(&handle).expect("no audio sink");
    for path in &files {
        println!("{path}: as recorded");
        sink.append(open(path));
        sink.sleep_until_end();
        std::thread::sleep(Duration::from_millis(600));
        println!("{path}: ship computer");
        sink.append(ship_computer(open(path)));
        sink.sleep_until_end();
        std::thread::sleep(Duration::from_millis(900));
    }
}
