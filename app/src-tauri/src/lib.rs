//! EVE Chatterer: a tray app that watches EVE chat logs and shows alerts as
//! overlays over the game. The always-on part is the Rust core (see
//! `runner`); WebView2 windows (overlays, settings) exist only while needed.

mod diag;
mod overlay;
mod runner;
mod state;
mod testalerts;

use eve_chatterer_core::settings::Settings;
use state::{AppState, SettingsData, Status};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> Status {
    state.status.lock().unwrap().clone()
}

/// Everything the settings window needs: the raw layered settings plus every
/// known character (each with the public channels it's actually been seen
/// in). "Not ready yet" only during the brief startup window before the
/// engine has found the log folder.
#[tauri::command]
fn get_settings_data(state: State<'_, AppState>) -> Result<SettingsData, String> {
    let guard = state.engine.lock().unwrap();
    let engine = guard.as_ref().ok_or("Still starting up — try again in a moment.")?;
    Ok(SettingsData { settings: engine.settings().settings().clone(), pilots: engine.pilots().iter().cloned().collect() })
}

/// Validates, persists to settings.json, and applies to the running engine
/// immediately (live inheritance: every character re-resolves on its next
/// alert, no restart needed).
#[tauri::command]
fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    let bad = settings.invalid_regexes();
    if !bad.is_empty() {
        return Err(format!("These patterns don't compile: {}", bad.join(", ")));
    }
    let cfg_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    settings.save(&cfg_dir.join("settings.json")).map_err(|e| e.to_string())?;
    let mut guard = state.engine.lock().unwrap();
    let engine = guard.as_mut().ok_or("Still starting up — try again in a moment.")?;
    engine.settings_mut().edit(|s| *s = settings);
    Ok(())
}

/// Forgets a known channel outright. Refuses if the pilot has any settings of
/// its own for that channel, so this can never silently discard a configured
/// rule (docs/BACKLOG.md, "Known channels never get pruned").
#[tauri::command]
fn remove_known_channel(app: AppHandle, state: State<'_, AppState>, pilot_id: String, channel_id: String) -> Result<(), String> {
    let mut guard = state.engine.lock().unwrap();
    let engine = guard.as_mut().ok_or("Still starting up — try again in a moment.")?;
    let has_override = engine.settings().settings().pilots.get(&pilot_id).is_some_and(|p| p.channels.contains_key(&channel_id));
    if has_override {
        return Err("This channel has settings of its own — reset them to Defaults first, then remove it.".into());
    }
    engine.pilots_mut().remove_channel(&pilot_id, &channel_id);
    let pilots_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("pilots.json");
    engine.pilots().save(&pilots_path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_test(app: AppHandle, kind: String) -> Result<(), String> {
    testalerts::send(&app, &kind);
    Ok(())
}

/// The overlay page calls this once it is listening for alerts.
#[tauri::command]
async fn overlay_ready(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    state.overlays.ready(&app, &label);
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
            .inner_size(760.0, 680.0)
            .min_inner_size(520.0, 420.0)
            .build();
        if let Err(e) = built {
            eprintln!("could not open the settings window: {e}");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    diag::init();
    let app = tauri::Builder::default()
        // A second launch just brings up the settings of the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| open_settings(app)))
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![get_status, send_test, overlay_ready, get_settings_data, save_settings, remove_known_channel])
        .setup(|app| {
            let settings_i = MenuItem::with_id(app, "tray-settings", "Settings…", true, None::<&str>)?;
            let test_i = MenuItem::with_id(app, "tray-test", "Try the overlays", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "tray-quit", "Quit EVE Chatterer", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_i, &test_i, &PredefinedMenuItem::separator(app)?, &quit_i])?;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("bundled window icon").clone())
                .tooltip("EVE Chatterer")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "tray-settings" => open_settings(app),
                    "tray-test" => {
                        let app = app.clone();
                        std::thread::spawn(move || testalerts::send(&app, "all"));
                    }
                    "tray-quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        open_settings(tray.app_handle());
                    }
                })
                .build(app)?;

            runner::spawn(app.handle().clone());
            if std::env::args().any(|a| a == "--selftest") {
                testalerts::selftest(app.handle());
            }
            if std::env::args().any(|a| a == "--soak") {
                testalerts::soak(app.handle());
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
