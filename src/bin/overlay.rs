//! Overlay feasibility probe.
//!
//! Can an alert window show over an EVE client in each display mode without
//! stealing focus or making the game minimize? Global hotkeys:
//!   Ctrl+Alt+1   banner on the monitor of the current foreground window
//!   Ctrl+Alt+2   banner on every monitor
//!   Ctrl+Alt+3   log state only (no window)
//!
//! The banner is a topmost, click-through, non-activating layered window that
//! disappears after 3 seconds. Each hotkey logs the foreground window, what
//! SHQueryUserNotificationState says, and (500 ms later and again after the
//! banner is gone) whether focus moved and every EVE client's state.
//! It also logs every change of the notification state on its own.
//! Everything goes to overlay-log.txt. Ctrl+C to stop.

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
    use eve_chatterer::winutil::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::Duration;
    use windows::core::{w, BOOL};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, EnumDisplayMonitors, FillRect,
        GetMonitorInfoW, MonitorFromWindow, SetBkMode, SetTextColor, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HDC,
        HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT};
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetForegroundWindow,
        GetMessageW, IsWindowVisible, KillTimer, RegisterClassW, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
        ShowWindow, TranslateMessage, HWND_TOPMOST, LWA_ALPHA, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SW_SHOWNOACTIVATE, WM_HOTKEY, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    static LOG: OnceLock<Mutex<File>> = OnceLock::new();
    static TEXT: Mutex<String> = Mutex::new(String::new());

    fn out(msg: impl AsRef<str>) {
        let line = format!("[{}] {}", ts(), msg.as_ref());
        println!("{line}");
        if let Some(f) = LOG.get() {
            if let Ok(mut f) = f.lock() {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    fn notif_state() -> String {
        match unsafe { SHQueryUserNotificationState() } {
            Ok(s) => match s.0 {
                1 => "NOT_PRESENT".into(),
                2 => "BUSY (fullscreen app or similar)".into(),
                3 => "RUNNING_D3D_FULL_SCREEN".into(),
                4 => "PRESENTATION_MODE".into(),
                5 => "ACCEPTS_NOTIFICATIONS".into(),
                6 => "QUIET_TIME".into(),
                7 => "APP (UWP fullscreen)".into(),
                n => format!("unknown({n})"),
            },
            Err(e) => format!("error {e}"),
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let brush = CreateSolidBrush(COLORREF(0x0040_2010)); // BGR: dark blue
                FillRect(hdc, &rc, brush);
                let _ = DeleteObject(brush.into());
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00FF_FFFF));
                let mut text: Vec<u16> = TEXT.lock().unwrap().encode_utf16().collect();
                DrawTextW(hdc, &mut text, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = KillTimer(Some(hwnd), 1);
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    unsafe extern "system" fn enum_mon(_m: HMONITOR, _dc: HDC, rc: *mut RECT, lparam: LPARAM) -> BOOL {
        let v = &mut *(lparam.0 as *mut Vec<RECT>);
        v.push(*rc);
        BOOL(1)
    }

    fn all_monitors() -> Vec<RECT> {
        let mut v: Vec<RECT> = vec![];
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(enum_mon), LPARAM(&mut v as *mut _ as isize));
        }
        v
    }

    fn monitor_of(h: HWND) -> RECT {
        unsafe {
            let m = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(m, &mut mi);
            mi.rcMonitor
        }
    }

    fn show_banner(mon: RECT) {
        const W: i32 = 760;
        const H: i32 = 110;
        let x = mon.left + (mon.right - mon.left - W) / 2;
        let y = mon.top + 60;
        unsafe {
            let hinst = GetModuleHandleW(None).unwrap();
            let hwnd = match CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("EveChatterOverlayProbe"),
                w!("overlay probe"),
                WS_POPUP,
                x,
                y,
                W,
                H,
                None,
                None,
                Some(hinst.into()),
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    out(format!("OVERLAY   CreateWindowExW failed: {e}"));
                    return;
                }
            };
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 235, LWA_ALPHA);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            SetTimer(Some(hwnd), 1, 3000, None);
            out(format!(
                "OVERLAY   hwnd={:#x} on monitor rect ({},{})-({},{}), IsWindowVisible={}",
                hwnd.0 as isize,
                mon.left,
                mon.top,
                mon.right,
                mon.bottom,
                IsWindowVisible(hwnd).as_bool()
            ));
        }
    }

    fn on_hotkey(id: usize) {
        let fg = unsafe { GetForegroundWindow() };
        let fg_key = fg.0 as isize;
        out(format!("HOTKEY {id}  notif-state: {}  foreground: {}", notif_state(), brief(fg)));
        out(format!("          clients before: {}", clients_state(fg_key)));
        if id != 3 {
            *TEXT.lock().unwrap() = "OVERLAY TEST  -  can you see this?".to_string();
            let rects = if id == 1 { vec![monitor_of(fg)] } else { all_monitors() };
            for r in rects {
                show_banner(r);
            }
        }
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            let now = unsafe { GetForegroundWindow() };
            out(format!(
                "AFTER 0.5s  focus kept: {}  foreground: {}",
                if now.0 as isize == fg_key { "YES" } else { "NO" },
                brief(now)
            ));
            out(format!("          clients: {}", clients_state(now.0 as isize)));
            thread::sleep(Duration::from_millis(3200));
            let later = unsafe { GetForegroundWindow() };
            out(format!("AFTER 3.7s  (banner gone) foreground: {}", brief(later)));
            out(format!("          clients: {}", clients_state(later.0 as isize)));
        });
    }

    pub fn run() {
        if let Ok(f) = OpenOptions::new().create(true).append(true).open("overlay-log.txt") {
            LOG.set(Mutex::new(f)).ok();
        }
        out("overlay probe starting (logging to overlay-log.txt)");

        unsafe {
            let hinst = GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                lpszClassName: w!("EveChatterOverlayProbe"),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                out("ERROR     RegisterClassW failed");
                return;
            }
            let mods = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
            for (id, vk) in [(1, 0x31u32), (2, 0x32), (3, 0x33)] {
                if RegisterHotKey(None, id, mods, vk).is_err() {
                    out(format!("ERROR     could not register Ctrl+Alt+{id} (already in use?)"));
                }
            }
        }
        out(format!("STATE     monitors: {}   notif-state now: {}", all_monitors().len(), notif_state()));
        out("READY     Ctrl+Alt+1 = banner on the foreground window's monitor, 2 = all monitors, 3 = log only");

        // Log every change of the notification state, with whatever is in the foreground.
        thread::spawn(|| {
            let mut last = String::new();
            loop {
                let s = notif_state();
                if s != last {
                    let fg = unsafe { GetForegroundWindow() };
                    out(format!("NOTIF      {s}   (foreground: {})", brief(fg)));
                    last = s;
                }
                thread::sleep(Duration::from_millis(500));
            }
        });

        unsafe {
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY {
                    on_hotkey(msg.wParam.0);
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            for id in 1..=3 {
                let _ = UnregisterHotKey(None, id);
            }
        }
    }
}
