//! EVE Chatterer: a tray app that watches EVE chat logs and shows alerts as
//! overlays over the game. The always-on part is the Rust core (see
//! `runner`); WebView2 windows (overlays, settings) exist only while needed.

mod badge;
mod clientmoves;
mod diag;
mod overlay;
mod reposition;
mod runner;
mod state;
mod testalerts;
mod toast;

use eve_chatterer_core::settings::Settings;
use state::{AppState, SettingsData, Status};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Toggles reposition mode for every known character. Chosen to be unlikely
/// to collide with EVE's own bindings or Windows shortcuts; not yet
/// user-configurable (docs/BACKLOG.md).
fn reposition_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyO)
}

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
    let online = state
        .last_snapshot
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.clients.iter().filter_map(|c| engine.pilots().by_name(&c.character).map(|p| p.id.clone())).collect())
        .unwrap_or_default();
    Ok(SettingsData { settings: engine.settings().settings().clone(), pilots: engine.pilots().iter().cloned().collect(), online })
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

/// Sets (or, given blank/whitespace, clears) a character's own Strip-badge
/// tag. `PilotRegistry::set_tag` normalizes it (trim, cap, uppercase), so
/// nothing needs validating here first.
#[tauri::command]
fn set_pilot_tag(app: AppHandle, state: State<'_, AppState>, pilot_id: String, tag: String) -> Result<(), String> {
    let mut guard = state.engine.lock().unwrap();
    let engine = guard.as_mut().ok_or("Still starting up — try again in a moment.")?;
    engine.pilots_mut().set_tag(&pilot_id, Some(&tag));
    let pilots_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("pilots.json");
    engine.pilots().save(&pilots_path).map_err(|e| e.to_string())
}

/// Clears a character's saved overlay position/width, so its alerts go back
/// to the default centered placement. The settings screen's escape hatch for
/// when a pin no longer makes sense (a monitor was removed, the window was
/// dragged somewhere awkward) — reposition mode itself has no delete
/// gesture, only drag/resize.
#[tauri::command]
fn clear_pilot_placement(app: AppHandle, state: State<'_, AppState>, pilot_id: String) -> Result<(), String> {
    let mut guard = state.engine.lock().unwrap();
    let engine = guard.as_mut().ok_or("Still starting up — try again in a moment.")?;
    engine.pilots_mut().set_placement(&pilot_id, None);
    let pilots_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("pilots.json");
    engine.pilots().save(&pilots_path).map_err(|e| e.to_string())
}

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
            .center()
            .build();
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
                    if *shortcut == reposition_shortcut() && event.state() == ShortcutState::Pressed {
                        // Off the UI thread: toggling takes the overlay locks,
                        // which the client-move watcher also takes while it
                        // may be waiting on this thread (clientmoves.rs).
                        let app = app.clone();
                        std::thread::spawn(move || reposition::toggle(&app));
                    }
                })
                .build(),
        )
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            get_status,
            overlay_ready,
            get_settings_data,
            save_settings,
            remove_known_channel,
            set_pilot_tag,
            clear_pilot_placement,
            reposition_gesture_start,
            reposition_move,
            reposition_resize,
            reposition_box_height
        ])
        .setup(|app| {
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
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        open_settings(tray.app_handle());
                    }
                })
                .build(app)?;

            if let Err(e) = app.global_shortcut().register(reposition_shortcut()) {
                eprintln!("could not register the reposition hotkey (Ctrl+Alt+O): {e}");
            }

            toast::init(app.handle());
            clientmoves::start(app.handle().clone());
            runner::spawn(app.handle().clone());
            if std::env::args().any(|a| a == "--selftest") {
                testalerts::selftest(app.handle());
            }
            if std::env::args().any(|a| a == "--toasttest") {
                testalerts::toasttest(app.handle());
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
