//! Shared app state: what the settings window shows and the overlay windows.

use crate::overlay::Overlays;
use eve_chatterer_core::engine::Engine;
use eve_chatterer_core::pilots::Pilot;
use eve_chatterer_core::presence::Snapshot;
use eve_chatterer_core::settings::Settings;
use serde::Serialize;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

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

/// Everything the settings window needs in one shot: the raw layered
/// settings, and every known character with the channels it's actually been
/// seen in (only public/unknown ones vary per character; see `docs/DESIGN.md`
/// "Settings screen").
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SettingsData {
    pub settings: Settings,
    pub pilots: Vec<Pilot>,
}

pub struct AppState {
    pub overlays: Overlays,
    pub status: Mutex<Status>,
    pub alerts_shown: AtomicU64,
    /// `None` until the runner thread has found the log folder and built the
    /// engine; commands that need it should report "not ready yet" rather
    /// than panic during that brief startup window.
    pub engine: Arc<Mutex<Option<Engine>>>,
    /// The most recent presence sample, refreshed every poll by the runner
    /// thread. Entering reposition mode reads it to start each pilot's
    /// placeholder over its actual client window when one is on screen,
    /// instead of always defaulting to the primary monitor.
    pub last_snapshot: Mutex<Option<Snapshot>>,
    /// True while a global-hotkey reposition session is open. Toggled by
    /// `reposition::toggle`; read by the tray/settings UI to reflect state.
    pub repositioning: Mutex<bool>,
}

impl AppState {
    pub fn new() -> AppState {
        AppState {
            overlays: Overlays::new(),
            status: Mutex::new(Status::default()),
            alerts_shown: AtomicU64::new(0),
            engine: Arc::new(Mutex::new(None)),
            last_snapshot: Mutex::new(None),
            repositioning: Mutex::new(false),
        }
    }
}
