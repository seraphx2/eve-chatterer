//! Where the app keeps its files: settings.json, pilots.json, the cache (the
//! notification icon) and WebView2's data.
//!
//! Installed copies use the usual per-user AppData folders, which survive an
//! uninstall and reinstall. A portable copy (a file named `portable` beside
//! the exe; the portable zip ships one) keeps everything in a `data` folder
//! beside the exe instead, so the folder *is* the app: copy it anywhere and the
//! settings come along, delete it and nothing is left behind (apart from the
//! notification registration, which Windows requires in the registry).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

/// The marker file beside the exe that makes a copy portable.
pub const PORTABLE_MARKER: &str = "portable";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub portable: bool,
    /// settings.json and pilots.json.
    pub config: PathBuf,
    pub cache: PathBuf,
    /// WebView2's data; `None` leaves Tauri's default (AppData).
    #[serde(skip)]
    pub webview: Option<PathBuf>,
    /// Set when the portable folder couldn't be written and the app fell back
    /// to AppData, so the user can be told.
    pub problem: Option<String>,
}

static STORAGE: OnceLock<Storage> = OnceLock::new();

/// Decides once, at startup, before anything reads or writes a file.
pub fn init(app: &AppHandle) {
    let _ = STORAGE.set(resolve(app));
}

pub fn get() -> &'static Storage {
    STORAGE.get().expect("storage::init runs first in setup")
}

pub fn config_dir() -> PathBuf {
    get().config.clone()
}

pub fn settings_path() -> PathBuf {
    get().config.join("settings.json")
}

pub fn pilots_path() -> PathBuf {
    get().config.join("pilots.json")
}

pub fn cache_dir() -> PathBuf {
    get().cache.clone()
}

pub fn webview_dir() -> Option<PathBuf> {
    get().webview.clone()
}

/// Points a window's WebView2 data at the portable folder when portable. Every
/// window in the process must use the same folder, so all of them go through here.
pub fn with_webview_dir<'a, R: tauri::Runtime, M: Manager<R>>(b: tauri::WebviewWindowBuilder<'a, R, M>) -> tauri::WebviewWindowBuilder<'a, R, M> {
    match webview_dir() {
        Some(dir) => b.data_directory(dir),
        None => b,
    }
}

fn resolve(app: &AppHandle) -> Storage {
    let appdata = |problem: Option<String>| Storage {
        portable: false,
        config: app.path().app_config_dir().unwrap_or_else(|_| fallback("config")),
        cache: app.path().app_cache_dir().unwrap_or_else(|_| fallback("cache")),
        webview: None,
        problem,
    };
    let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) else {
        return appdata(None);
    };
    if !exe_dir.join(PORTABLE_MARKER).is_file() {
        return appdata(None);
    }
    let data = exe_dir.join("data");
    match writable(&data) {
        Ok(()) => Storage { portable: true, config: data.clone(), cache: data.join("cache"), webview: Some(data.join("webview")), problem: None },
        Err(e) => {
            let msg = format!("The portable folder {} can't be written ({e}), so settings are being kept in AppData instead. Move the EVE Chatterer folder somewhere you can write to, such as your Documents.", data.display());
            eprintln!("{msg}");
            appdata(Some(msg))
        }
    }
}

/// Creates the folder if needed and proves a file can be written in it.
fn writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(".write-test");
    std::fs::write(&probe, b"ok")?;
    std::fs::remove_file(&probe)
}

/// Only if Windows can't name the AppData folders at all.
fn fallback(what: &str) -> PathBuf {
    std::env::temp_dir().join("eve-chatterer").join(what)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writable_folder_passes_and_leaves_no_probe_behind() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        writable(&data).unwrap();
        assert!(data.is_dir());
        assert_eq!(std::fs::read_dir(&data).unwrap().count(), 0);
    }
}
