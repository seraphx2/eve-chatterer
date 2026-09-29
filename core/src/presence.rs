//! Where the user's attention is: which EVE clients exist, which one has
//! focus, which are on screen, where they are.
//!
//! The model and the focus logic here are platform-neutral and tested. The
//! Windows sampler (`Sampler`) fills a `Snapshot` by polling; polling
//! `GetForegroundWindow()` at a few Hz is the truth, and it is all the router
//! needs (a WinEvent hook would only wake us sooner; see docs/DESIGN.md).

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Does `self` cover `other` to within `slack` pixels on every side?
    pub fn covers(&self, other: &Rect, slack: i32) -> bool {
        self.left <= other.left + slack
            && self.top <= other.top + slack
            && self.right >= other.right - slack
            && self.bottom >= other.bottom - slack
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientState {
    pub character: String,
    pub pid: u32,
    pub hwnd: isize,
    pub minimized: bool,
    /// Hidden by DWM (another virtual desktop).
    pub cloaked: bool,
    pub visible_flag: bool,
    /// Window bounds; `None` while minimized or cloaked.
    pub rect: Option<Rect>,
    /// The monitor it is on; the last known one while minimized or cloaked.
    pub monitor: Option<Rect>,
}

impl ClientState {
    /// `IsWindowVisible` alone is true for minimized windows and for windows on
    /// another virtual desktop, so all three checks are needed.
    pub fn on_screen(&self) -> bool {
        self.visible_flag && !self.minimized && !self.cloaked
    }

    /// True for borderless-style modes (Fixed Window, Fullscreen), where an
    /// overlay should anchor to the monitor rather than follow the window.
    pub fn covers_monitor(&self) -> bool {
        matches!((&self.rect, &self.monitor), (Some(r), Some(m)) if r.covers(m, 8))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundInfo {
    pub hwnd: isize,
    pub exe: Option<String>,
    pub class: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub clients: Vec<ClientState>,
    /// Character of the client that effectively has focus (transient shell
    /// windows such as the alt-tab switcher do not take it away).
    pub focused: Option<String>,
    pub foreground: Option<ForegroundInfo>,
    /// Time since the last keyboard or mouse input.
    pub idle: Duration,
    /// Would a Windows toast be shown right now?
    pub notifications_ok: bool,
}

impl Snapshot {
    pub fn client(&self, character: &str) -> Option<&ClientState> {
        self.clients.iter().find(|c| c.character.eq_ignore_ascii_case(character))
    }

    pub fn any_on_screen(&self) -> bool {
        self.clients.iter().any(ClientState::on_screen)
    }

    pub fn is_focused(&self, character: &str) -> bool {
        self.focused.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(character))
    }

    /// The client the user is actually looking at, if any.
    pub fn focused_client(&self) -> Option<&ClientState> {
        self.focused.as_deref().and_then(|f| self.client(f))
    }
}

/// Explorer shell surfaces that briefly take the foreground during alt-tab,
/// taskbar use and virtual-desktop switching (docs/FINDINGS.md #4). They do
/// not mean the user left EVE.
pub fn is_shell_transient(class: &str) -> bool {
    matches!(
        class,
        "XamlExplorerHostIslandWindow" // "Task Switching": the Windows 11 alt-tab UI
            | "ForegroundStaging"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "VirtualDesktopHotkeySwitcher"
            | "ApplicationManager_DesktopShellWindow"
            | "MultitaskingViewFrame"
    )
}

/// Turns raw foreground observations into the effective focused character.
pub struct FocusTracker {
    grace: Duration,
    focused: Option<String>,
    transient_since: Option<Instant>,
}

impl FocusTracker {
    /// `grace` is how long a shell surface may hold the foreground before we
    /// conclude the user really left the client.
    pub fn new(grace: Duration) -> FocusTracker {
        FocusTracker { grace, focused: None, transient_since: None }
    }

    pub fn observe(&mut self, fg: Option<&ForegroundInfo>, clients: &[ClientState], now: Instant) -> Option<String> {
        let eve = fg.and_then(|f| clients.iter().find(|c| c.hwnd == f.hwnd));
        let transient = fg.is_none_or(|f| is_shell_transient(&f.class));
        if let Some(c) = eve {
            self.focused = Some(c.character.clone());
            self.transient_since = None;
        } else if transient {
            let since = *self.transient_since.get_or_insert(now);
            if now.duration_since(since) >= self.grace {
                self.focused = None;
            }
        } else {
            self.focused = None;
            self.transient_since = None;
        }
        self.focused.clone()
    }
}

#[cfg(windows)]
pub use win::Sampler;

#[cfg(windows)]
mod win {
    use super::*;
    use crate::winapi;
    use std::collections::HashMap;
    use windows::Win32::Foundation::HWND;

    /// Samples the desktop. Call about every 250 ms so the focus tracker sees
    /// the transitions; a sample is cheap (one window enumeration).
    pub struct Sampler {
        tracker: FocusTracker,
        last_monitor: HashMap<String, Rect>,
    }

    impl Sampler {
        pub fn new() -> Sampler {
            Sampler { tracker: FocusTracker::new(Duration::from_millis(1500)), last_monitor: HashMap::new() }
        }

        pub fn sample(&mut self, now: Instant) -> Snapshot {
            let mut clients = vec![];
            for (pid, hwnd, character) in winapi::eve_clients() {
                let h = HWND(hwnd as *mut _);
                let minimized = winapi::is_minimized(h);
                let cloaked = winapi::cloaked(h) != 0;
                let showing = !minimized && !cloaked;
                // The client area, not the frame: overlays belong inside the
                // game's viewing area, never over a windowed client's title bar.
                let rect = if showing { winapi::client_rect_of(h) } else { None };
                let monitor = if showing { winapi::monitor_rect_of(h) } else { None };
                if let Some(m) = monitor {
                    self.last_monitor.insert(character.clone(), m);
                }
                let monitor = monitor.or_else(|| self.last_monitor.get(&character).copied());
                clients.push(ClientState {
                    character,
                    pid,
                    hwnd,
                    minimized,
                    cloaked,
                    visible_flag: winapi::is_visible(h),
                    rect,
                    monitor,
                });
            }
            let foreground = winapi::foreground().map(|h| {
                let (_, exe, _) = winapi::resolve(h);
                ForegroundInfo { hwnd: h.0 as isize, exe, class: winapi::class_of(h), title: winapi::title_of(h) }
            });
            let focused = self.tracker.observe(foreground.as_ref(), &clients, now);
            Snapshot { clients, focused, foreground, idle: winapi::idle_duration(), notifications_ok: winapi::notifications_ok() }
        }
    }

    impl Default for Sampler {
        fn default() -> Self {
            Sampler::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(name: &str, hwnd: isize) -> ClientState {
        let mon = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
        ClientState {
            character: name.into(),
            pid: 1,
            hwnd,
            minimized: false,
            cloaked: false,
            visible_flag: true,
            rect: Some(mon),
            monitor: Some(mon),
        }
    }

    fn fg(hwnd: isize, class: &str) -> ForegroundInfo {
        ForegroundInfo { hwnd, exe: None, class: class.into(), title: String::new() }
    }

    #[test]
    fn on_screen_needs_all_three_checks() {
        let mut c = client("Jarna", 1);
        assert!(c.on_screen());
        c.minimized = true;
        assert!(!c.on_screen());
        c.minimized = false;
        c.cloaked = true; // other virtual desktop: IsWindowVisible still says true
        assert!(!c.on_screen());
        c.cloaked = false;
        c.visible_flag = false;
        assert!(!c.on_screen());
    }

    #[test]
    fn covers_monitor_distinguishes_borderless_from_windowed() {
        let mut c = client("Jarna", 1);
        assert!(c.covers_monitor());
        c.rect = Some(Rect { left: 300, top: 200, right: 1500, bottom: 900 });
        assert!(!c.covers_monitor());
        c.rect = Some(Rect { left: -2, top: -2, right: 1922, bottom: 1082 }); // slightly oversized frame
        assert!(c.covers_monitor());
        c.rect = None;
        assert!(!c.covers_monitor());
    }

    #[test]
    fn focus_follows_eve_windows_and_drops_for_other_apps() {
        let clients = [client("Jarna", 10), client("Psianna", 20)];
        let mut t = FocusTracker::new(Duration::from_millis(1500));
        let t0 = Instant::now();
        assert_eq!(t.observe(Some(&fg(10, "trinityWindow")), &clients, t0).as_deref(), Some("Jarna"));
        assert_eq!(t.observe(Some(&fg(20, "trinityWindow")), &clients, t0).as_deref(), Some("Psianna"));
        assert_eq!(t.observe(Some(&fg(99, "MozillaWindowClass")), &clients, t0), None);
    }

    #[test]
    fn the_alt_tab_switcher_does_not_steal_focus_until_the_grace_runs_out() {
        let clients = [client("Jarna", 10)];
        let mut t = FocusTracker::new(Duration::from_millis(1500));
        let t0 = Instant::now();
        t.observe(Some(&fg(10, "trinityWindow")), &clients, t0);
        let switcher = fg(77, "XamlExplorerHostIslandWindow");
        assert_eq!(t.observe(Some(&switcher), &clients, t0 + Duration::from_millis(300)).as_deref(), Some("Jarna"));
        assert_eq!(t.observe(Some(&switcher), &clients, t0 + Duration::from_millis(1000)).as_deref(), Some("Jarna"));
        // Held open longer than the grace: the user has effectively left.
        assert_eq!(t.observe(Some(&switcher), &clients, t0 + Duration::from_millis(2000)), None);
    }

    #[test]
    fn landing_on_eve_after_the_switcher_takes_focus_immediately() {
        let clients = [client("Jarna", 10), client("Psianna", 20)];
        let mut t = FocusTracker::new(Duration::from_millis(1500));
        let t0 = Instant::now();
        t.observe(Some(&fg(10, "trinityWindow")), &clients, t0);
        t.observe(Some(&fg(77, "XamlExplorerHostIslandWindow")), &clients, t0 + Duration::from_millis(200));
        assert_eq!(t.observe(Some(&fg(20, "trinityWindow")), &clients, t0 + Duration::from_millis(600)).as_deref(), Some("Psianna"));
    }

    #[test]
    fn a_null_foreground_is_treated_as_transient() {
        let clients = [client("Jarna", 10)];
        let mut t = FocusTracker::new(Duration::from_millis(1500));
        let t0 = Instant::now();
        t.observe(Some(&fg(10, "trinityWindow")), &clients, t0);
        assert_eq!(t.observe(None, &clients, t0 + Duration::from_millis(100)).as_deref(), Some("Jarna"));
    }

    #[test]
    fn snapshot_lookups_ignore_case() {
        let s = Snapshot {
            clients: vec![client("Psianna Archeia", 20)],
            focused: Some("Psianna Archeia".into()),
            foreground: None,
            idle: Duration::ZERO,
            notifications_ok: true,
        };
        assert!(s.client("psianna archeia").is_some());
        assert!(s.is_focused("PSIANNA ARCHEIA"));
        assert!(!s.is_focused("Jarna"));
        assert!(s.any_on_screen());
    }
}
