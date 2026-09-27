//! Win32 helpers: window/process lookup and EVE client state.

use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible,
};

/// UTC wall clock, HH:MM:SS.mmmZ.
pub fn ts() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let s = d.as_secs() % 86400;
    format!("{:02}:{:02}:{:02}.{:03}Z", s / 3600, s % 3600 / 60, s % 60, d.subsec_millis())
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
/// and works for most elevated processes too.
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

/// The character name if this is an EVE client window ("EVE - Name").
pub fn character_of(exe: &Option<String>, title: &str) -> Option<String> {
    let is_eve = exe.as_deref().is_some_and(|e| file_name(e).eq_ignore_ascii_case("exefile.exe"));
    if !is_eve {
        return None;
    }
    Some(title.strip_prefix("EVE - ").map(str::to_string).unwrap_or_else(|| "<no character yet>".into()))
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

/// One line describing every EVE client: focus, minimized, visible, cloaked.
pub fn clients_state(fg: isize) -> String {
    eve_clients()
        .iter()
        .map(|(_, hwnd, c)| {
            let h = HWND(*hwnd as *mut _);
            format!(
                "{c}[{} {} {} cloak={}]",
                if *hwnd == fg { "FOCUSED" } else { "-" },
                if unsafe { IsIconic(h) }.as_bool() { "MINIMIZED" } else { "-" },
                if unsafe { IsWindowVisible(h) }.as_bool() { "visible" } else { "hidden" },
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
