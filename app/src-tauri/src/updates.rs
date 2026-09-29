//! Self-update from GitHub Releases, driven from Rust rather than a window:
//! the app lives in the tray and usually has no window open, so checks run
//! on a timer here, an available update is announced with a notification and
//! a tray menu item, and the About page just shows and drives this state.
//!
//! Releases are signed (tauri.conf.json `plugins.updater.pubkey`); the
//! updater refuses anything not signed with the matching private key.

use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::{AppHandle, Wry};
use tauri_plugin_updater::UpdaterExt;

/// First check shortly after startup (not during it), then this often.
const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
pub const TRAY_ITEM: &str = "tray-update";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// "self": this install can update itself (the installer build).
    /// "unmanaged": nothing updates it (the portable zip, a dev build).
    pub mode: &'static str,
    /// A newer version, when one was found.
    pub available: Option<String>,
    pub notes: Option<String>,
    pub checking: bool,
    pub installing: bool,
    /// Unix seconds of the last completed check.
    pub last_checked: Option<u64>,
    pub error: Option<String>,
}

static STATUS: Mutex<UpdateStatus> = Mutex::new(UpdateStatus {
    mode: "unmanaged",
    available: None,
    notes: None,
    checking: false,
    installing: false,
    last_checked: None,
    error: None,
});
static TRAY_MENU: OnceLock<Menu<Wry>> = OnceLock::new();
static TRAY_ENTRY: Mutex<Option<MenuItem<Wry>>> = Mutex::new(None);
/// The version already announced this session, so it's announced once.
static ANNOUNCED: Mutex<Option<String>> = Mutex::new(None);

/// How this build receives updates. Reads the marker the bundler patches
/// into the exe, and also requires the installer's uninstaller beside it: the
/// portable zip ships the same exe the bundler built (it may carry the
/// marker), but it isn't installed, so it must never try to update itself.
fn mode() -> &'static str {
    use tauri::utils::{config::BundleType, platform::bundle_type};
    let installed = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("uninstall.exe").is_file())).unwrap_or(false);
    match bundle_type() {
        Some(BundleType::Nsis | BundleType::Msi) if installed => "self",
        _ => "unmanaged",
    }
}

pub fn status() -> UpdateStatus {
    STATUS.lock().unwrap().clone()
}

/// Starts the periodic check (installer builds only). `menu` is the tray
/// menu, which gains an "Install update" entry when one is found.
pub fn start(app: AppHandle, menu: Menu<Wry>) {
    STATUS.lock().unwrap().mode = mode();
    let _ = TRAY_MENU.set(menu);
    if mode() != "self" {
        return;
    }
    let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            let _ = tauri::async_runtime::block_on(check(&app));
            std::thread::sleep(EVERY);
        }
    });
    if let Err(e) = spawned {
        eprintln!("could not start the update checker: {e}");
    }
}

/// Checks now. Failures (offline, GitHub down) are kept in the status for the
/// About page but never announced: the next check retries.
pub async fn check(app: &AppHandle) -> UpdateStatus {
    {
        let mut s = STATUS.lock().unwrap();
        if s.mode != "self" || s.checking || s.installing {
            return s.clone();
        }
        s.checking = true;
        s.error = None;
    }
    let result = match app.updater() {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    let found = {
        let mut s = STATUS.lock().unwrap();
        s.checking = false;
        s.last_checked = SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        match result {
            Ok(Some(u)) => {
                s.available = Some(u.version.clone());
                s.notes = u.body.clone().filter(|b| !b.trim().is_empty());
            }
            Ok(None) => {
                s.available = None;
                s.notes = None;
            }
            Err(e) => {
                eprintln!("update check failed: {e}");
                s.error = Some(e);
            }
        }
        s.available.clone()
    };
    if let Some(v) = found {
        announce(app, &v);
    }
    status()
}

fn announce(app: &AppHandle, version: &str) {
    {
        let mut done = ANNOUNCED.lock().unwrap();
        if done.as_deref() == Some(version) {
            return;
        }
        *done = Some(version.to_string());
    }
    crate::toast::plain(
        "EVE Chatterer update available",
        &format!("Version {version} is ready. Right-click the tray icon and choose \"Install update\", or open Settings > About."),
    );
    let text = format!("Install update {version}");
    let mut entry = TRAY_ENTRY.lock().unwrap();
    match entry.as_ref() {
        Some(item) => {
            let _ = item.set_text(&text);
        }
        None => {
            let Some(menu) = TRAY_MENU.get() else { return };
            match MenuItem::with_id(app, TRAY_ITEM, &text, true, None::<&str>) {
                Ok(item) => {
                    if menu.insert(&item, 0).is_ok() {
                        *entry = Some(item);
                    }
                }
                Err(e) => eprintln!("could not add the update menu item: {e}"),
            }
        }
    }
}

/// Downloads and installs the update, then restarts into it. On Windows the
/// installer runs in passive mode (a progress bar, no questions) and closes
/// this app itself before replacing it.
pub async fn install(app: AppHandle) -> Result<(), String> {
    {
        let mut s = STATUS.lock().unwrap();
        if s.mode != "self" {
            return Err("This copy can't update itself. Download the new version from GitHub.".into());
        }
        if s.installing {
            return Ok(());
        }
        s.installing = true;
        s.error = None;
    }
    let result = async {
        let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
        let Some(update) = update else {
            return Err("No update is available any more.".to_string());
        };
        update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())
    }
    .await;
    match result {
        Ok(()) => app.restart(),
        Err(e) => {
            eprintln!("update install failed: {e}");
            let mut s = STATUS.lock().unwrap();
            s.installing = false;
            s.error = Some(e.clone());
            Err(e)
        }
    }
}
