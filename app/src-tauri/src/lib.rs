//! EVE Chatterer: a tray app that watches EVE chat logs and shows alerts as
//! overlays over the game. The always-on part is the Rust core (see
//! `runner`); WebView2 windows (overlays, settings) exist only while needed.

mod audio;
mod badge;
mod clientmoves;
mod diag;
mod hotkey;
mod overlay;
mod reposition;
mod runner;
mod state;
mod storage;
mod testalerts;
mod toast;
mod updates;

use eve_chatterer_core::pilots::PilotRegistry;
use eve_chatterer_core::settings::Settings;
use state::{AppState, SettingsData};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::ShortcutState;

// Every command that takes an app lock is `async`, so it runs off the UI
// thread: the UI thread must never wait on one (state.rs, "Locking rules").

/// Everything the settings window needs: the raw layered settings plus every
/// known character (each with the public channels it's actually been seen
/// in).
#[tauri::command]
async fn get_settings_data(state: State<'_, AppState>) -> Result<SettingsData, String> {
    let engine = state.engine.lock().unwrap();
    let online = state
        .last_snapshot
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.clients.iter().filter_map(|c| engine.pilots().by_name(&c.character).map(|p| p.id.clone())).collect())
        .unwrap_or_default();
    Ok(SettingsData { settings: engine.settings().settings().clone(), pilots: engine.pilots().iter().cloned().collect(), online })
}

/// Persists to settings.json and applies to the running engine immediately
/// (live inheritance: every character re-resolves on its next alert, no
/// restart needed). Returns the tracked patterns that don't compile: they're
/// saved anyway, since refusing the save would lose every other edit with
/// them, and matching skips them until they're fixed.
#[tauri::command]
async fn save_settings(state: State<'_, AppState>, mut settings: Settings) -> Result<Vec<String>, String> {
    let mut engine = state.engine.lock().unwrap();
    // Only `set_reposition_hotkey` changes the hotkey (it has to register it
    // with Windows first), so an autosave from a stale page can't undo it.
    settings.general.reposition_hotkey = engine.settings().settings().general.reposition_hotkey.clone();
    // Written under the engine lock, so two saves land in the order applied.
    settings.save(&storage::settings_path()).map_err(|e| format!("Couldn't save settings.json: {e}"))?;
    let bad = settings.invalid_regexes();
    engine.settings_mut().edit(|s| *s = settings);
    Ok(bad)
}

/// The Tracked dialog's check of a pattern, with the same engine and flags
/// matching uses (JavaScript's own RegExp differs). `None` when it compiles,
/// otherwise why not.
#[tauri::command]
fn check_regex(pattern: String) -> Option<String> {
    let e = eve_chatterer_core::rules::compile_pattern(&pattern).err()?.to_string();
    // The regex crate's message quotes the pattern with our (?i) prefix and a
    // caret line; its last line is the reason.
    Some(e.lines().rev().find_map(|l| l.trim().strip_prefix("error: ")).unwrap_or(e.trim()).to_string())
}

/// Changes the reposition hotkey: registers it with Windows first (refused if
/// another app holds it, and then nothing changes), then saves it.
#[tauri::command]
async fn set_reposition_hotkey(app: AppHandle, state: State<'_, AppState>, accel: String) -> Result<(), String> {
    // Registered before taking the engine lock: it waits on the UI thread.
    hotkey::change(&app, &accel)?;
    let mut engine = state.engine.lock().unwrap();
    engine.settings_mut().edit(|s| s.general.reposition_hotkey = accel.trim().to_string());
    engine.settings().settings().save(&storage::settings_path()).map_err(|e| format!("Couldn't save settings.json: {e}"))
}

/// Forgets a known channel outright. Refuses if the pilot has any settings of
/// its own for that channel, so this can never silently discard a configured
/// rule (docs/BACKLOG.md, "Known channels never get pruned").
#[tauri::command]
async fn remove_known_channel(state: State<'_, AppState>, pilot_id: String, channel_id: String) -> Result<(), String> {
    let mut engine = state.engine.lock().unwrap();
    let has_override = engine.settings().settings().pilots.get(&pilot_id).is_some_and(|p| p.channels.contains_key(&channel_id));
    if has_override {
        return Err("This channel has settings of its own — reset them to Defaults first, then remove it.".into());
    }
    engine.pilots_mut().remove_channel(&pilot_id, &channel_id);
    state::save_pilots(&engine)
}

/// Sets (or, given blank/whitespace, clears) a character's own Strip-badge
/// tag. `PilotRegistry::set_tag` normalizes it (trim, cap, uppercase), so
/// nothing needs validating here first.
#[tauri::command]
async fn set_pilot_tag(state: State<'_, AppState>, pilot_id: String, tag: String) -> Result<(), String> {
    let mut engine = state.engine.lock().unwrap();
    engine.pilots_mut().set_tag(&pilot_id, Some(&tag));
    state::save_pilots(&engine)
}

/// Clears a character's saved overlay position/width, so its alerts go back
/// to the default centered placement. The settings screen's escape hatch for
/// when a pin no longer makes sense (a monitor was removed, the window was
/// dragged somewhere awkward) — reposition mode itself has no delete
/// gesture, only drag/resize.
#[tauri::command]
async fn clear_pilot_placement(state: State<'_, AppState>, pilot_id: String) -> Result<(), String> {
    let mut engine = state.engine.lock().unwrap();
    engine.pilots_mut().set_placement(&pilot_id, None);
    state::save_pilots(&engine)
}

/// The Audio page's play button: the given file (or the built-in sound for
/// none) at the given volume, now, whatever the cooldown.
#[tauri::command]
async fn preview_sound(file: Option<String>, volume: u8) -> Result<(), String> {
    let source = file.filter(|f| !f.trim().is_empty()).map_or(eve_chatterer_core::audio::Source::BuiltIn, eve_chatterer_core::audio::Source::File);
    audio::preview(source, eve_chatterer_core::audio::gain_for(volume));
    Ok(())
}

/// The Audio page's Browse button. None when the picker was cancelled.
#[tauri::command]
async fn pick_sound_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut picker = app.dialog().file().set_title("Choose a sound").add_filter("Sounds", &["wav", "mp3", "ogg", "flac"]);
    if let Some(w) = app.get_webview_window("settings") {
        picker = picker.set_parent(&w);
    }
    match picker.blocking_pick_file() {
        None => Ok(None),
        Some(p) => p.into_path().map(|p| Some(p.display().to_string())).map_err(|e| e.to_string()),
    }
}

/// The General page's "File locations": where settings live (storage.rs),
/// the chat log folder the app found, and the program folder.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Locations {
    storage: storage::Storage,
    /// `None` when there's no `Documents\EVE\logs\Chatlogs` yet.
    chat_logs: Option<String>,
    program: Option<String>,
}

#[tauri::command]
fn get_locations() -> Locations {
    Locations {
        storage: storage::get().clone(),
        chat_logs: eve_chatterer_core::paths::chatlogs_dir().map(|p| p.display().to_string()),
        program: program_dir().map(|p| p.display().to_string()),
    }
}

fn program_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()?.parent().map(std::path::Path::to_path_buf)
}

/// Opens one of the known folders in Explorer. Takes a name, not a path, so
/// the page can only open these folders.
#[tauri::command]
fn open_folder(which: String) -> Result<(), String> {
    let dir = match which.as_str() {
        "chat_logs" => eve_chatterer_core::paths::chatlogs_dir().ok_or("EVE's chat log folder doesn't exist yet.")?,
        "settings" => storage::config_dir(),
        "program" => program_dir().ok_or("Couldn't find the program folder.")?,
        other => return Err(format!("Unknown folder: {other}")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't open {}: {e}", dir.display()))?;
    std::process::Command::new("explorer").arg(&dir).spawn().map_err(|e| format!("Couldn't open {}: {e}", dir.display()))?;
    Ok(())
}

/// Opens a link from the release notes in the default browser. Web links
/// only: the notes come from the network, and this must never start a
/// program. Sync on purpose: it runs on the UI thread, where COM (which
/// ShellExecute may need) is already set up.
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    if !url.starts_with("https://") || url.chars().any(char::is_control) {
        return Err("Only web links can be opened.".into());
    }
    open_in_browser(&url)
}

#[cfg(windows)]
fn open_in_browser(url: &str) -> Result<(), String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let r = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // Anything above 32 means it worked (ShellExecute's documented contract).
    if r.0 as isize > 32 { Ok(()) } else { Err(format!("Couldn't open {url}.")) }
}

#[cfg(not(windows))]
fn open_in_browser(url: &str) -> Result<(), String> {
    Err(format!("Opening {url} isn't supported here."))
}

/// Update state for the About page (updates.rs).
#[tauri::command]
fn get_update_status() -> updates::UpdateStatus {
    updates::status()
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) -> Result<updates::UpdateStatus, String> {
    Ok(updates::check(&app).await)
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    updates::install(app).await
}

/// Whether Windows starts the app at login. Windows' own Run entry is the
/// truth (the installer creates it on a fresh install, same as dev-prompt).
#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Sync on purpose, so quick toggles apply in click order.
#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if !enabled {
        clear_startup_approved();
    }
    Ok(())
}

/// Turning start-at-login off removes the Run value but leaves the Task
/// Manager on/off state Windows keeps beside it; nothing else would ever
/// remove it.
#[cfg(windows)]
fn clear_startup_approved() {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, HKEY_CURRENT_USER};
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"), w!("EVE Chatterer"));
    }
}

/// The argument the login entry starts the app with, so a login launch is
/// distinguishable from a manual one. Nothing branches on it yet: every
/// launch starts quietly in the tray.
const AUTOSTART_ARG: &str = "--autostart";

/// The overlay page calls this once it is listening for alerts.
#[tauri::command]
async fn overlay_ready(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    state.overlays.ready(&app, &label);
    Ok(())
}

/// Reposition-mode gestures from the overlay page. The page reports pointer
/// deltas; the move/resize itself happens here, clamped inside the game, so
/// it never behaves like dragging an ordinary desktop window.
#[tauri::command]
async fn reposition_gesture_start(state: State<'_, AppState>, label: String) -> Result<(), String> {
    state.overlays.gesture_start(&label);
    Ok(())
}

#[tauri::command]
async fn reposition_move(state: State<'_, AppState>, label: String, dx: f64, dy: f64) -> Result<(), String> {
    state.overlays.gesture_move(&label, dx, dy);
    Ok(())
}

#[tauri::command]
async fn reposition_resize(state: State<'_, AppState>, label: String, dx: f64) -> Result<(), String> {
    state.overlays.gesture_resize(&label, dx);
    Ok(())
}

/// The page's measured box height, so the window always fits it exactly.
#[tauri::command]
async fn reposition_box_height(state: State<'_, AppState>, label: String, height: f64) -> Result<(), String> {
    state.overlays.set_box_height(&label, height);
    Ok(())
}

/// Opens the settings window, or brings the existing one forward. Windows are
/// created off the UI thread to avoid deadlocking WebView2's startup.
fn open_settings(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = app.get_webview_window("settings") {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            return;
        }
        let built = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html".into()))
            .title("EVE Chatterer")
            .inner_size(1100.0, 800.0)
            .min_inner_size(1100.0, 800.0)
            .center();
        let built = storage::with_webview_dir(built).build();
        match built {
            // Hide instead of letting the OS close button tear the webview
            // down, so the next open reuses the warm one above (instant)
            // instead of a full reload (Svelte remount + settings refetched
            // over IPC). Only the tray's "Quit" (`app.exit`) actually ends
            // the process; that bypasses window events entirely.
            Ok(w) => {
                let hide = w.clone();
                w.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hide.hide();
                    }
                });
            }
            Err(e) => eprintln!("could not open the settings window: {e}"),
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    diag::init();
    let app = tauri::Builder::default()
        // A second launch just brings up the settings of the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| open_settings(app)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if hotkey::is_reposition(shortcut) && event.state() == ShortcutState::Pressed {
                        // Off the UI thread: toggling takes the overlay locks,
                        // which the client-move watcher also takes while it
                        // may be waiting on this thread (clientmoves.rs).
                        let app = app.clone();
                        std::thread::spawn(move || reposition::toggle(&app));
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![AUTOSTART_ARG])))
        .invoke_handler(tauri::generate_handler![
            overlay_ready,
            get_settings_data,
            save_settings,
            check_regex,
            remove_known_channel,
            set_pilot_tag,
            clear_pilot_placement,
            reposition_gesture_start,
            reposition_move,
            reposition_resize,
            reposition_box_height,
            preview_sound,
            pick_sound_file,
            get_locations,
            open_folder,
            get_update_status,
            check_for_updates,
            open_link,
            install_update,
            get_autostart,
            set_autostart,
            set_reposition_hotkey
        ])
        .setup(|app| {
            // First: everything below may read or write the app's files.
            storage::init(app.handle());
            // Next: anything below may need to tell the user something.
            toast::init(app.handle());
            // A file that can't be read is kept aside, never overwritten, and
            // the user is told (core/src/store.rs).
            let (settings, settings_problem) = Settings::open(&storage::settings_path());
            let (pilots, pilots_problem) = PilotRegistry::open(&storage::pilots_path());
            for problem in [settings_problem, pilots_problem].into_iter().flatten() {
                toast::plain("EVE Chatterer couldn't read its settings", &problem);
            }
            let settings_i = MenuItem::with_id(app, "tray-settings", "Settings…", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "tray-quit", "Quit EVE Chatterer", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_i, &PredefinedMenuItem::separator(app)?, &quit_i])?;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("bundled window icon").clone())
                .tooltip("EVE Chatterer")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "tray-settings" => open_settings(app),
                    "tray-quit" => app.exit(0),
                    updates::TRAY_ITEM => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = updates::install(app).await {
                                toast::plain("Could not install the update", &e);
                            }
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        open_settings(tray.app_handle());
                    }
                })
                .build(app)?;

            if let Some(problem) = hotkey::register_at_startup(app.handle(), &settings.general.reposition_hotkey) {
                toast::plain("Reposition hotkey", &problem);
            }
            if let Some(problem) = &storage::get().problem {
                toast::plain("EVE Chatterer isn't portable right now", problem);
            }
            // Before anything that reads the state: commands, the runner, the
            // move hook. Building the engine does no I/O.
            app.manage(AppState::new(runner::build_engine(settings, pilots)));
            audio::start();
            updates::start(app.handle().clone(), menu.clone());
            clientmoves::start(app.handle().clone());
            runner::spawn(app.handle().clone());
            if std::env::args().any(|a| a == "--selftest") {
                testalerts::selftest(app.handle());
            }
            if std::env::args().any(|a| a == "--toasttest") {
                testalerts::toasttest(app.handle());
            }
            if std::env::args().any(|a| a == "--soundtest") {
                testalerts::soundtest();
            }
            if std::env::args().any(|a| a == "--soak") {
                testalerts::soak(app.handle());
            }
            if std::env::args().any(|a| a == "--freezetest") {
                testalerts::freezetest(app.handle());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building EVE Chatterer");

    app.run(|_app, event| {
        // The app has no permanent window: closing the last overlay or the
        // settings window must not end it. Only an explicit exit does.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
