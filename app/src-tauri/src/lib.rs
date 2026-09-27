//! EVE Chatterer: a tray app that watches EVE chat logs and shows alerts as
//! overlays over the game. The always-on part is the Rust core (see
//! `runner`); WebView2 windows (overlays, settings) exist only while needed.

mod overlay;
mod runner;
mod state;
mod testalerts;

use state::{AppState, Status};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> Status {
    state.status.lock().unwrap().clone()
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
    let app = tauri::Builder::default()
        // A second launch just brings up the settings of the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| open_settings(app)))
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![get_status, send_test, overlay_ready])
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
