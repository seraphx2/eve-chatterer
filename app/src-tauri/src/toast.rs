//! Windows notifications, sent straight through WinRT (docs/FINDINGS.md
//! #12; design: docs/design/windows-notification.html). They honor Do Not
//! Disturb, Focus and full-screen apps, and keep Action Center history,
//! which no custom popup could (FINDINGS #11).
//!
//! A chat alert carries the pilot's badge image, "Sender in Channel", the
//! message, a "Pilot · reason" line, and Switch to / Dismiss buttons.
//! Repeats from the same pilot and channel replace one notification with a
//! running count instead of stacking.

/// One chat alert, as a notification.
#[derive(Clone, Debug)]
pub struct ChatToast {
    /// Groups repeats: the same pilot and channel replace one notification.
    pub key: String,
    pub pilot: String,
    pub tag: String,
    pub accent: String,
    pub tone: &'static str,
    pub sender: String,
    pub channel: String,
    pub text: String,
    pub reason: String,
    /// The overlay style this alert would have had ("strip", "panel",
    /// "beacon"); sets how long the notification stays (see `presence`).
    pub style: &'static str,
    /// It mentions the pilot by name: always sticky, whatever the style.
    pub mention: bool,
}

/// How insistent a notification is, mirroring the overlays' lifetimes
/// (Strip 6 s < Panel 9 s < Beacon 12 s): Windows' default (~7 s), `long`
/// (~25 s), or `reminder`, which stays on screen until clicked or dismissed.
/// Windows offers no other lengths. A mention of the pilot's name is always
/// sticky, even in a channel whose Style is set lower (owner, 2026-09-29).
fn presence(style: &str, mention: bool) -> &'static str {
    match style {
        _ if mention => r#" scenario="reminder""#,
        "beacon" => r#" scenario="reminder""#,
        "panel" => r#" duration="long""#,
        _ => "",
    }
}

/// The key a chat alert (and its rate-cap folds) group under: a new line
/// replaces the notification with the same key. Mentions get a key of their
/// own, so ordinary chatter in the same channel never replaces (and so
/// un-sticks) a mention's notification.
pub fn key_for(pilot: &str, channel: &str, mention: bool) -> String {
    format!("{}|{}|{}", pilot.to_lowercase(), channel.to_lowercase(), if mention { "mention" } else { "chat" })
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The small "Pilot · reason" line, with the folded line count once there
/// is more than one.
fn attribution(t: &ChatToast, count: u32) -> String {
    let lines = if count > 1 { format!(" · {count} lines") } else { String::new() };
    format!("{} · {}{lines}", t.pilot, t.reason)
}

/// The notification's XML. `logo` is a file path to the badge PNG. The
/// attribution line is a data binding (`{attribution}`), filled in from the
/// notification's data, so a rate-cap fold can update the count on the
/// notification already on screen instead of replacing it (replacing it
/// pulled it off screen after about a second).
/// Always silent: sound is the app's own (audio.rs), with its cooldown
/// and the Audio page's choice, never Windows' notification sound on top.
fn chat_xml(t: &ChatToast, logo: Option<&str>) -> String {
    let first = t.pilot.split_whitespace().next().unwrap_or(&t.pilot);
    let switch = xml_escape(&format!("switch={}", t.pilot));
    let image = logo.map(|p| format!(r#"<image placement="appLogoOverride" src="file:///{}"/>"#, xml_escape(&p.replace('\\', "/")))).unwrap_or_default();
    format!(
        r#"<toast launch="{switch}" activationType="foreground"{presence}><visual><binding template="ToastGeneric"><text>{sender} in {channel}</text><text>{text}</text><text placement="attribution">{{attribution}}</text>{image}</binding></visual><actions><action content="Switch to {first}" arguments="{switch}" activationType="foreground"/><action content="Dismiss" arguments="dismiss" activationType="system"/></actions><audio silent="true"/></toast>"#,
        sender = xml_escape(&t.sender),
        channel = xml_escape(&t.channel),
        text = xml_escape(&t.text),
        first = xml_escape(first),
        presence = presence(t.style, t.mention),
    )
}

fn plain_xml(title: &str, body: &str) -> String {
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio silent="true"/></toast>"#,
        xml_escape(title),
        xml_escape(body)
    )
}

#[cfg(windows)]
mod imp {
    use super::{attribution, chat_xml, plain_xml, ChatToast};
    use crate::badge;
    use crate::state::AppState;
    use std::collections::HashMap;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Manager};
    use windows::core::{Interface, HSTRING, IInspectable};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{NotificationData, ToastActivatedEventArgs, ToastNotification, ToastNotificationManager, ToastNotifier};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    use windows::Win32::UI::WindowsAndMessaging::{
        FlashWindowEx, IsIconic, SetForegroundWindow, ShowWindow, FLASHWINFO, FLASHW_ALL, FLASHW_TIMERNOFG, SW_RESTORE,
    };

    /// The app's identity in the notification center; the same one the
    /// installer uses (tauri.conf.json `identifier`).
    const AUMID: &str = "io.github.seraphx2.evechatterer";
    /// Repeats within this long of the previous one join its notification.
    const BURST: Duration = Duration::from_secs(120);
    const APP_ICON: &[u8] = include_bytes!("../icons/128x128.png");

    struct Burst {
        count: u32,
        last: Instant,
        toast: ChatToast,
        // Kept alive so its click handler stays attached.
        _shown: ToastNotification,
    }

    struct State {
        notifier: ToastNotifier,
        badges: PathBuf,
        font: Option<ab_glyph::FontVec>,
        bursts: HashMap<String, Burst>,
    }

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static STATE: Mutex<Option<State>> = Mutex::new(None);

    thread_local! {
        static WINRT: () = unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        };
    }

    /// Registers the app's identity (per user, no admin) so Windows shows
    /// its name and icon, and gets the notifier ready.
    ///
    /// Runs on its own short-lived thread: `init` is called from Tauri's
    /// setup, on the UI thread, and initializing WinRT there (multithreaded)
    /// makes the UI thread's own COM setup fail the next time it creates a
    /// window (RPC_E_CHANGED_MODE), which panicked the app when the settings
    /// window opened. Nothing here may run on the UI thread.
    pub fn init(app: &AppHandle) {
        let app = app.clone();
        let done = std::thread::Builder::new().name("notifications-init".into()).spawn(move || init_here(&app));
        match done {
            Ok(t) => {
                let _ = t.join();
            }
            Err(e) => eprintln!("notifications: could not start: {e}"),
        }
    }

    fn init_here(app: &AppHandle) {
        let _ = APP.set(app.clone());
        WINRT.with(|_| ());
        let Ok(cache) = app.path().app_cache_dir() else {
            eprintln!("notifications: no cache folder; they will not show");
            return;
        };
        let badges = cache.join("badges");
        let _ = std::fs::create_dir_all(&badges);
        let icon = cache.join("app-icon.png");
        let _ = std::fs::write(&icon, APP_ICON);
        register_identity(&icon.display().to_string());
        let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("notifications: could not create the notifier: {e}");
                return;
            }
        };
        // The tag's typeface: Windows' own Segoe UI Semibold (the overlays'
        // Barlow only ships inside the web bundle).
        let fonts = std::env::var("WINDIR").map(|w| PathBuf::from(w).join("Fonts")).unwrap_or_else(|_| PathBuf::from(r"C:\Windows\Fonts"));
        let font = ["seguisb.ttf", "segoeui.ttf"]
            .iter()
            .find_map(|f| std::fs::read(fonts.join(f)).ok().and_then(|b| ab_glyph::FontVec::try_from_vec(b).ok()));
        *STATE.lock().unwrap() = Some(State { notifier, badges, font, bursts: HashMap::new() });
    }

    fn register_identity(icon: &str) {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let key = format!(r"HKCU\Software\Classes\AppUserModelId\{AUMID}");
        for (name, value) in [("DisplayName", "EVE Chatterer"), ("IconUri", icon)] {
            let ok = std::process::Command::new("reg")
                .args(["add", &key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .is_ok_and(|o| o.status.success());
            if !ok {
                eprintln!("notifications: could not register {name}; they may show without the app's name");
            }
        }
    }

    fn badge_path(state: &State, t: &ChatToast) -> Option<String> {
        let safe: String = format!("{}-{}-{}", t.tag, t.accent.trim_start_matches('#'), t.tone).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let path = state.badges.join(format!("{safe}.png"));
        if !path.exists() {
            let png = badge::render(&t.tag, badge::parse_hex(&t.accent), badge::tone_color(t.tone), state.font.as_ref())?;
            std::fs::write(&path, png).ok()?;
        }
        Some(path.display().to_string())
    }

    /// A short, stable notification tag for a burst key (Windows caps tags
    /// at 64 characters; pilot and channel names could exceed that).
    fn short_tag(key: &str) -> String {
        let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        format!("{h:016x}")
    }

    const GROUP: &str = "chat";

    /// The values bound into a chat notification (`{attribution}`), and a
    /// sequence number so Windows never applies an older update over a newer.
    fn chat_data(attribution: &str, seq: u32) -> windows::core::Result<NotificationData> {
        let data = NotificationData::new()?;
        data.Values()?.Insert(&HSTRING::from("attribution"), &HSTRING::from(attribution))?;
        data.SetSequenceNumber(seq)?;
        Ok(data)
    }

    fn show(state: &State, xml: &str, tag: Option<&str>, data: Option<NotificationData>) -> windows::core::Result<ToastNotification> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        if let Some(tag) = tag {
            toast.SetTag(&HSTRING::from(tag))?;
            toast.SetGroup(&HSTRING::from(GROUP))?;
        }
        if let Some(data) = data {
            toast.SetData(&data)?;
        }
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(|_, a| {
            let args = a.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok()).and_then(|a| a.Arguments().ok()).map(|h| h.to_string());
            if let Some(pilot) = args.as_deref().and_then(|a| a.strip_prefix("switch=")) {
                switch_to(pilot);
            }
            Ok(())
        }))?;
        state.notifier.Show(&toast)?;
        Ok(toast)
    }

    /// Shows a chat alert. A new line from the same pilot and channel
    /// replaces its notification (new message, new popup) and carries the
    /// running line count forward.
    pub fn chat(t: ChatToast) {
        WINRT.with(|_| ());
        let mut guard = STATE.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        state.bursts.retain(|_, b| b.last.elapsed() < BURST);
        let count = state.bursts.get(&t.key).map_or(1, |b| b.count + 1);
        let logo = badge_path(state, &t);
        let xml = chat_xml(&t, logo.as_deref());
        let shown = chat_data(&attribution(&t, count), count).and_then(|data| show(state, &xml, Some(&short_tag(&t.key)), Some(data)));
        match shown {
            Ok(shown) => {
                state.bursts.insert(t.key.clone(), Burst { count, last: Instant::now(), toast: t, _shown: shown });
            }
            Err(e) => eprintln!("notification failed: {e}"),
        }
    }

    /// A line past its pilot's rate cap: bump the line count on that pilot
    /// and channel's notification in place. It stays on screen with its own
    /// timer; replacing it instead pulled it off screen within a second.
    pub fn fold(key: &str) {
        WINRT.with(|_| ());
        let mut guard = STATE.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        let Some(b) = state.bursts.get_mut(key).filter(|b| b.last.elapsed() < BURST) else { return };
        b.count += 1;
        b.last = Instant::now();
        let text = attribution(&b.toast, b.count);
        let result = chat_data(&text, b.count)
            .and_then(|data| state.notifier.UpdateWithTagAndGroup(&data, &HSTRING::from(short_tag(key)), &HSTRING::from(GROUP)));
        if let Err(e) = result {
            eprintln!("notification update failed: {e}");
        }
    }

    /// An app-level notification (new character, logging off, errors).
    pub fn plain(title: &str, body: &str) {
        WINRT.with(|_| ());
        let guard = STATE.lock().unwrap();
        let Some(state) = guard.as_ref() else {
            eprintln!("{title}: {body}");
            return;
        };
        if let Err(e) = show(state, &plain_xml(title, body), None, None) {
            eprintln!("notification failed: {e}");
        }
    }

    /// "Switch to <pilot>": bring that character's client forward. Windows
    /// only lets a process take the foreground in some situations (a click on
    /// its notification is usually one); when it refuses, flash the client's
    /// taskbar button instead so the click still leads somewhere.
    fn switch_to(pilot: &str) {
        let Some(app) = APP.get() else { return };
        let hwnd = app
            .state::<AppState>()
            .last_snapshot
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| s.clients.iter().find(|c| c.character.eq_ignore_ascii_case(pilot)).map(|c| c.hwnd));
        let Some(hwnd) = hwnd else {
            println!("switch to {pilot}: no client running for that character");
            return;
        };
        let h = HWND(hwnd as *mut _);
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            if SetForegroundWindow(h).as_bool() {
                println!("switch to {pilot}: brought forward");
            } else {
                let info = FLASHWINFO {
                    cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                    hwnd: h,
                    dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
                    uCount: 0,
                    dwTimeout: 0,
                };
                let _ = FlashWindowEx(&info);
                println!("switch to {pilot}: Windows refused the foreground; flashing its taskbar button");
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{chat, fold, init, plain};

#[cfg(not(windows))]
mod imp {
    use super::ChatToast;
    pub fn init(_: &tauri::AppHandle) {}
    pub fn chat(t: ChatToast) {
        eprintln!("{} in {}: {}", t.sender, t.channel, t.text);
    }
    pub fn fold(_: &str) {}
    pub fn plain(title: &str, body: &str) {
        eprintln!("{title}: {body}");
    }
}

#[cfg(not(windows))]
pub use imp::{chat, fold, init, plain};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ChatToast {
        ChatToast {
            key: key_for("Naomi Nagata", "Local", false),
            pilot: "Naomi Nagata".into(),
            tag: "PA".into(),
            accent: "#a58af0".into(),
            tone: "mention",
            sender: "Bob & <Co>".into(),
            channel: "Local".into(),
            text: "say \"hi\"".into(),
            reason: "Mentioned you".into(),
            style: "beacon",
            mention: false,
        }
    }

    #[test]
    fn a_mention_is_always_sticky_whatever_the_style() {
        for style in ["strip", "panel", "beacon"] {
            let x = chat_xml(&ChatToast { style, mention: true, ..sample() }, None);
            assert!(x.contains(r#"scenario="reminder""#), "{style}: {x}");
        }
    }

    #[test]
    fn the_style_sets_how_long_it_stays_like_the_overlays_lifetimes() {
        let with = |style| chat_xml(&ChatToast { style, ..sample() }, None);
        assert!(with("beacon").contains(r#"scenario="reminder""#));
        assert!(with("panel").contains(r#"duration="long""#));
        let strip = with("strip");
        assert!(!strip.contains("scenario") && !strip.contains("duration"), "{strip}");
    }

    #[test]
    fn chat_xml_escapes_everything_and_lays_out_like_the_mockup() {
        let x = chat_xml(&sample(), Some(r"C:\cache\badges\PA.png"));
        assert!(x.contains("<text>Bob &amp; &lt;Co&gt; in Local</text>"), "{x}");
        assert!(x.contains("<text>say &quot;hi&quot;</text>"), "{x}");
        // Bound, so a fold can update it in place.
        assert!(x.contains(r#"<text placement="attribution">{attribution}</text>"#), "{x}");
        assert!(x.contains("file:///C:/cache/badges/PA.png"), "{x}");
        assert!(x.contains(r#"content="Switch to Naomi""#), "{x}");
        assert!(x.contains(r#"arguments="switch=Naomi Nagata""#), "{x}");
    }

    #[test]
    fn a_folded_notification_shows_its_line_count() {
        assert_eq!(attribution(&sample(), 1), "Naomi Nagata · Mentioned you");
        assert_eq!(attribution(&sample(), 3), "Naomi Nagata · Mentioned you · 3 lines");
    }

    #[test]
    fn repeats_group_by_pilot_and_channel_regardless_of_case() {
        assert_eq!(key_for("Holden", "Local", false), key_for("holden", "LOCAL", false));
        assert_ne!(key_for("Holden", "Local", false), key_for("Holden", "Corp", false));
    }

    #[test]
    fn a_mention_has_its_own_slot_so_chatter_cannot_replace_it() {
        assert_ne!(key_for("Holden", "Local", true), key_for("Holden", "Local", false));
    }
}
