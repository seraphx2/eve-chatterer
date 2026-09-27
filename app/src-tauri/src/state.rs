//! Shared app state: what the settings window shows and the overlay windows.

use crate::overlay::Overlays;
use serde::Serialize;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PilotView {
    pub id: String,
    pub name: String,
    pub live: bool,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub log_folder: Option<String>,
    pub running: bool,
    pub pilots: Vec<PilotView>,
    pub alerts_shown: u64,
    /// Overlay windows currently open (they close after a while idle).
    pub overlay_windows: usize,
}

pub struct AppState {
    pub overlays: Overlays,
    pub status: Mutex<Status>,
    pub alerts_shown: AtomicU64,
}

impl AppState {
    pub fn new() -> AppState {
        AppState { overlays: Overlays::new(), status: Mutex::new(Status::default()), alerts_shown: AtomicU64::new(0) }
    }
}
