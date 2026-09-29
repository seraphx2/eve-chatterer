//! Tracks EVE client windows as they are dragged or resized, so each
//! overlay moves with its client immediately instead of catching up on the
//! next presence sample (up to 250 ms later).
//!
//! A WinEvent hook: Windows tells us when *any* top-level window moves, we
//! keep only the clients that own an overlay. It is out-of-context (nothing
//! is loaded into EVE's process; see CLAUDE.md) and runs on its own thread
//! with its own message loop, never on the UI thread, because it takes the
//! overlay locks.

#[cfg(windows)]
mod imp {
    use crate::state::AppState;
    use eve_chatterer_core::winapi;
    use std::sync::OnceLock;
    use tauri::{AppHandle, Manager};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_UNCLOAKED, MSG,
        OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    };

    static APP: OnceLock<AppHandle> = OnceLock::new();

    pub fn start(app: AppHandle) {
        if APP.set(app).is_err() {
            return; // already running
        }
        let started = std::thread::Builder::new().name("client-moves".into()).spawn(|| unsafe {
            let hook = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(on_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if hook.is_invalid() {
                eprintln!("could not watch EVE client moves; overlays will follow on the next presence sample instead");
            }
            // The shell cloaks a window the instant its desktop is switched
            // away from (and uncloaks it on the way back); mirroring that on
            // the overlays keeps them from flashing on the other desktop.
            let cloak = SetWinEventHook(
                EVENT_OBJECT_CLOAKED,
                EVENT_OBJECT_UNCLOAKED,
                None,
                Some(on_cloak),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if cloak.is_invalid() {
                eprintln!("could not watch EVE client cloaking; overlays may flash when switching virtual desktops");
            }
            if hook.is_invalid() && cloak.is_invalid() {
                return;
            }
            // Out-of-context events are delivered through this thread's queue.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        if let Err(e) = started {
            eprintln!("could not start the client-move watcher: {e}");
        }
    }

    unsafe extern "system" fn on_event(_: HWINEVENTHOOK, _: u32, hwnd: HWND, id_object: i32, id_child: i32, _: u32, _: u32) {
        // Location changes fire for carets, cursors and child controls too;
        // only whole windows matter.
        if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
            return;
        }
        let Some(app) = APP.get() else { return };
        let owner = hwnd.0 as isize;
        let overlays = &app.state::<AppState>().overlays;
        if !overlays.is_owner(owner) || winapi::is_minimized(hwnd) {
            return;
        }
        if let (Some(region), Some(monitor)) = (winapi::client_rect_of(hwnd), winapi::monitor_rect_of(hwnd)) {
            overlays.follow_client(app, owner, region, monitor);
        }
    }

    unsafe extern "system" fn on_cloak(_: HWINEVENTHOOK, event: u32, hwnd: HWND, id_object: i32, id_child: i32, _: u32, _: u32) {
        if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
            return;
        }
        let Some(app) = APP.get() else { return };
        let owner = hwnd.0 as isize;
        let overlays = &app.state::<AppState>().overlays;
        if overlays.is_owner(owner) {
            overlays.set_owner_cloaked(owner, event == EVENT_OBJECT_CLOAKED);
        }
    }
}

#[cfg(windows)]
pub use imp::start;

#[cfg(not(windows))]
pub fn start(_: tauri::AppHandle) {}
