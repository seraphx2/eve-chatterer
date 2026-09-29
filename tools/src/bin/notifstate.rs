//! What `SHQueryUserNotificationState` reports while the user toggles
//! Windows' notification controls: Do Not Disturb, a Focus session,
//! presentation mode, a full-screen app. The app uses this to decide whether
//! a popup may show; this probe measures which of those it actually sees.
//!
//! Prints (and appends to `notif-log.txt`) every change, plus a heartbeat
//! every 10 s. Ctrl+C to stop.

#[cfg(windows)]
fn main() {
    use eve_chatterer_core::winapi::ts;
    use std::io::Write;
    use std::time::{Duration, Instant};
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;

    fn name(v: i32) -> &'static str {
        match v {
            1 => "NOT_PRESENT (screensaver, locked, or fast user switching)",
            2 => "BUSY (full-screen app)",
            3 => "RUNNING_D3D_FULL_SCREEN (exclusive full-screen Direct3D)",
            4 => "PRESENTATION_MODE",
            5 => "ACCEPTS_NOTIFICATIONS",
            6 => "QUIET_TIME (first hour after setup/upgrade)",
            7 => "APP (a Windows Store app is running full screen)",
            _ => "unknown",
        }
    }

    let mut log = std::fs::OpenOptions::new().create(true).append(true).open("notif-log.txt").expect("open notif-log.txt");
    let mut say = |line: String| {
        println!("{line}");
        let _ = writeln!(log, "{line}");
    };
    say(format!("{} probe started; toggle Do Not Disturb, a Focus session, presentation mode, etc.", ts()));

    // Windows 11 22H2+ Focus sessions, via WinRT (FINDINGS #11 showed the
    // shell query above does not see Focus or Do Not Disturb).
    unsafe {
        let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
    }
    let focus = match windows::UI::Shell::FocusSessionManager::IsSupported() {
        Ok(true) => match windows::UI::Shell::FocusSessionManager::GetDefault() {
            Ok(m) => Some(m),
            Err(e) => {
                say(format!("{} FocusSessionManager::GetDefault failed: {e}", ts()));
                None
            }
        },
        Ok(false) => {
            say(format!("{} FocusSessionManager not supported on this Windows", ts()));
            None
        }
        Err(e) => {
            say(format!("{} FocusSessionManager::IsSupported failed: {e}", ts()));
            None
        }
    };
    let focus_text = |m: &Option<windows::UI::Shell::FocusSessionManager>| match m {
        None => "focus=n/a".to_string(),
        Some(m) => match m.IsFocusActive() {
            Ok(b) => format!("focus={}", if b { "ACTIVE" } else { "off" }),
            Err(e) => format!("focus=error({e})"),
        },
    };

    let mut last: Option<(i32, String)> = None;
    let mut beat = Instant::now();
    loop {
        let v = match unsafe { SHQueryUserNotificationState() } {
            Ok(s) => s.0,
            Err(e) => {
                say(format!("{} error: {e}", ts()));
                -1
            }
        };
        let f = focus_text(&focus);
        let now = (v, f.clone());
        if last.as_ref() != Some(&now) {
            say(format!("{} CHANGED -> shell={v} {} | {f}", ts(), name(v)));
            last = Some(now);
            beat = Instant::now();
        } else if beat.elapsed() >= Duration::from_secs(10) {
            say(format!("{}   still shell={v} {} | {f}", ts(), name(v)));
            beat = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("notifstate is Windows only");
}
