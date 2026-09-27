//! Win32 helpers: window/process lookup and EVE client state. Windows only.

use crate::presence::Rect;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::SHQueryUserNotificationState;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible,
};

/// UTC wall clock, HH:MM:SS.mmmZ.
pub fn ts() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let s = d.as_secs() % 86400;
    format!("{:02}:{:02}:{:02}.{:03}Z", s / 3600, s % 3600 / 60, s % 60, d.subsec_millis())
}

pub fn foreground() -> Option<HWND> {
    let h = unsafe { GetForegroundWindow() };
    (!h.0.is_null()).then_some(h)
}

pub fn title_of(h: HWND) -> String {
    let mut b = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut b) };
    String::from_utf16_lossy(&b[..n.max(0) as usize])
}

pub fn class_of(h: HWND) -> String {
    let mut b = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut b) };
    String::from_utf16_lossy(&b[..n.max(0) as usize])
}

pub fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// Full exe path. PROCESS_QUERY_LIMITED_INFORMATION needs no admin rights
/// and works for most elevated processes too. Never reads the command line.
pub fn exe_of(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

struct Uwp {
    host_pid: u32,
    found: u32,
}

unsafe extern "system" fn enum_child(h: HWND, lparam: LPARAM) -> BOOL {
    let u = &mut *(lparam.0 as *mut Uwp);
    if class_of(h) == "Windows.UI.Core.CoreWindow" {
        let pid = pid_of(h);
        if pid != u.host_pid {
            u.found = pid;
            return BOOL(0);
        }
    }
    BOOL(1)
}

/// (pid, exe, note). UWP apps are hosted by ApplicationFrameHost.exe; the
/// real process is the owner of the child CoreWindow.
pub fn resolve(h: HWND) -> (u32, Option<String>, &'static str) {
    let pid = pid_of(h);
    let exe = exe_of(pid);
    if exe.as_deref().is_some_and(|e| file_name(e).eq_ignore_ascii_case("ApplicationFrameHost.exe")) {
        let mut u = Uwp { host_pid: pid, found: 0 };
        unsafe {
            let _ = EnumChildWindows(Some(h), Some(enum_child), LPARAM(&mut u as *mut Uwp as isize));
        }
        if u.found != 0 {
            return (u.found, exe_of(u.found), "UWP: resolved through ApplicationFrameHost");
        }
    }
    (pid, exe, "")
}

/// The character name if this is an EVE client window with a character
/// ("EVE - Name"). A client at the login screen has no character yet.
pub fn character_of(exe: &Option<String>, title: &str) -> Option<String> {
    let is_eve = exe.as_deref().is_some_and(|e| file_name(e).eq_ignore_ascii_case("exefile.exe"));
    let name = title.strip_prefix("EVE - ")?.trim();
    (is_eve && !name.is_empty()).then(|| name.to_string())
}

/// Every top-level EVE client window on any virtual desktop (no visibility filter).
unsafe extern "system" fn enum_top(h: HWND, lparam: LPARAM) -> BOOL {
    let title = title_of(h);
    if title.starts_with("EVE") {
        let exe = exe_of(pid_of(h));
        if let Some(c) = character_of(&exe, &title) {
            let v = &mut *(lparam.0 as *mut Vec<(u32, isize, String)>);
            v.push((pid_of(h), h.0 as isize, c));
        }
    }
    BOOL(1)
}

/// (pid, hwnd, character), sorted by character.
pub fn eve_clients() -> Vec<(u32, isize, String)> {
    let mut v: Vec<(u32, isize, String)> = vec![];
    unsafe {
        let _ = EnumWindows(Some(enum_top), LPARAM(&mut v as *mut _ as isize));
    }
    v.sort_by(|a, b| a.2.cmp(&b.2));
    v
}

/// DWM cloak state. Nonzero means hidden (2 = on another virtual desktop).
pub fn cloaked(h: HWND) -> u32 {
    let mut v = 0u32;
    unsafe {
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut _, 4);
    }
    v
}

pub fn is_minimized(h: HWND) -> bool {
    unsafe { IsIconic(h) }.as_bool()
}

pub fn is_visible(h: HWND) -> bool {
    unsafe { IsWindowVisible(h) }.as_bool()
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Rect {
        Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

/// Visible frame of a window (without the invisible resize border).
pub fn rect_of(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32)
            .ok()?;
    }
    Some(r.into())
}

pub fn monitor_rect_of(h: HWND) -> Option<Rect> {
    unsafe {
        let m = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(m, &mut mi).as_bool().then(|| mi.rcMonitor.into())
    }
}

/// Time since the last keyboard or mouse input, system-wide.
pub fn idle_duration() -> Duration {
    unsafe {
        let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if GetLastInputInfo(&mut li).as_bool() {
            Duration::from_millis(u64::from(GetTickCount().wrapping_sub(li.dwTime)))
        } else {
            Duration::ZERO
        }
    }
}

/// True when a Windows toast would be shown right now. Not a display-mode
/// detector: it read `BUSY` for some EVE display modes and not others
/// (docs/FINDINGS.md #5).
pub fn notifications_ok() -> bool {
    matches!(unsafe { SHQueryUserNotificationState() }, Ok(s) if s.0 == 5)
}

/// One line describing every EVE client: focus, minimized, visible, cloaked.
pub fn clients_state(fg: isize) -> String {
    eve_clients()
        .iter()
        .map(|(_, hwnd, c)| {
            let h = HWND(*hwnd as *mut _);
            format!(
                "{c}[{} {} {} cloak={}]",
                if *hwnd == fg { "FOCUSED" } else { "-" },
                if is_minimized(h) { "MINIMIZED" } else { "-" },
                if is_visible(h) { "visible" } else { "hidden" },
                cloaked(h)
            )
        })
        .collect::<Vec<_>>()
        .join("  ")
}

pub fn brief(h: HWND) -> String {
    let (pid, exe, _) = resolve(h);
    format!(
        "{:#x} pid={pid} exe={} class={:?} title={:?}",
        h.0 as isize,
        exe.as_deref().map(file_name).unwrap_or("<no access>"),
        class_of(h),
        title_of(h)
    )
}
