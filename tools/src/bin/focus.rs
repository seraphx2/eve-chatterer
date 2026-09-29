//! Foreground-window probe.
//!
//! Questions it answers:
//!   1. Does SetWinEventHook(EVENT_SYSTEM_FOREGROUND) report every focus
//!      change between two EVE clients (and everything else), with no polling?
//!   2. Can we get exe + title + character name ("EVE - Jarna") cheaply and
//!      without elevation?
//!   3. Do titles change under us (login screen -> character) and do we see it?
//!
//! A polling cross-check reports MISMATCH if the hook ever falls behind
//! GetForegroundWindow(), and CLIENTS whenever an EVE client's focus,
//! minimized or cloaked state changes. Logs to focus-log.txt.

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only.");
}

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(windows)]
mod win {
    use eve_chatterer_tools::winutil::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::Duration;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetForegroundWindow, GetMessageW, TranslateMessage, EVENT_OBJECT_NAMECHANGE,
        EVENT_SYSTEM_FOREGROUND, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    };

    static LAST_HOOK_HWND: AtomicIsize = AtomicIsize::new(0);
    static LAST_SEEN: Mutex<Option<(isize, String)>> = Mutex::new(None);
    static LOG: OnceLock<Mutex<File>> = OnceLock::new();

    /// Echoes to the console and appends to focus-log.txt in the working directory.
    fn out(msg: impl AsRef<str>) {
        let line = format!("[{}] {}", ts(), msg.as_ref());
        println!("{line}");
        if let Some(f) = LOG.get() {
            if let Ok(mut f) = f.lock() {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    fn report(kind: &str, h: HWND) {
        let title = title_of(h);
        let key = h.0 as isize;
        {
            let mut last = LAST_SEEN.lock().unwrap();
            if kind == "RENAMED" && last.as_ref() == Some(&(key, title.clone())) {
                return;
            }
            *last = Some((key, title.clone()));
        }
        let (pid, exe, note) = resolve(h);
        let exe_disp = match &exe {
            Some(p) => file_name(p).to_string(),
            None => "<no access>".to_string(),
        };
        let mut line = format!("{kind:<10} pid={pid} exe={exe_disp} class={:?} title={title:?}", class_of(h));
        if let Some(c) = character_of(&exe, &title) {
            line.push_str(&format!("  => EVE CLIENT, character: {c}"));
        }
        if !note.is_empty() {
            line.push_str(&format!("  [{note}]"));
        }
        out(line);
    }

    unsafe extern "system" fn on_event(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        id_child: i32,
        _thread: u32,
        _ms: u32,
    ) {
        if hwnd.0.is_null() {
            return;
        }
        match event {
            EVENT_SYSTEM_FOREGROUND => {
                LAST_HOOK_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
                report("FOREGROUND", hwnd);
            }
            EVENT_OBJECT_NAMECHANGE
                // Fires for every window on the desktop; only the foreground window's own title matters.
                if id_object == OBJID_WINDOW.0 && id_child == 0 && hwnd == GetForegroundWindow() => {
                    report("RENAMED", hwnd);
                }
            _ => {}
        }
    }

    pub fn run() {
        if let Ok(f) = OpenOptions::new().create(true).append(true).open("focus-log.txt") {
            LOG.set(Mutex::new(f)).ok();
        }
        out("focus probe starting (logging to focus-log.txt). Alt-tab between windows; Ctrl+C to stop.");

        let clients = eve_clients();
        out(format!("STARTUP   {} EVE client window(s) found (any desktop)", clients.len()));
        for (pid, hwnd, c) in &clients {
            out(format!("          pid={pid} hwnd={hwnd:#x} character: {c}"));
        }

        let fg = unsafe { GetForegroundWindow() };
        if !fg.0.is_null() {
            LAST_HOOK_HWND.store(fg.0 as isize, Ordering::Relaxed);
            report("INITIAL", fg);
        }

        // Polling cross-check, independent of the hook. GetForegroundWindow is the ground truth.
        thread::spawn(|| {
            let mut off = 0;
            let mut last_state = String::new();
            loop {
                thread::sleep(Duration::from_millis(250));
                let fgw = unsafe { GetForegroundWindow() };
                let fg = fgw.0 as isize;
                let hooked = LAST_HOOK_HWND.load(Ordering::Relaxed);
                if fg != 0 && fg != hooked {
                    off += 1;
                    if off == 2 {
                        out(format!(
                            "MISMATCH   real foreground: {}\n                       hook last said: {}",
                            brief(fgw),
                            brief(HWND(hooked as *mut _))
                        ));
                    }
                } else {
                    off = 0;
                }
                let state = clients_state(fg);
                if state != last_state {
                    out(format!("CLIENTS    {state}"));
                    last_state = state;
                }
            }
        });

        unsafe {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let h1 = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_event), 0, 0, flags);
            let h2 = SetWinEventHook(EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE, None, Some(on_event), 0, 0, flags);
            if h1.0.is_null() || h2.0.is_null() {
                out("ERROR      SetWinEventHook failed");
                return;
            }
            out("READY      hooks installed (foreground + title changes)");
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWinEvent(h1);
            let _ = UnhookWinEvent(h2);
        }
    }
}
