//! Shared app state: the engine, the latest presence sample and the overlay
//! windows.
//!
//! Locking rules (a hang here freezes the tray, the overlays and Settings):
//! - The UI thread never waits on any of these locks. Every command that
//!   takes one is `async`, so it runs off the UI thread.
//! - No lock is held across a call that waits on the UI thread: a Tauri
//!   getter, a window build, or a Win32 call that sends to a window the UI
//!   thread owns (`SetWindowLongPtrW`, `ShowWindow`, `SetWindowPos` without
//!   `SWP_ASYNCWINDOWPOS`).
//! - Order, where two are held: `engine` before `last_snapshot`; the
//!   overlays' `slots` before `sessions`. Never take an overlay lock while
//!   holding `engine`.

use crate::overlay::Overlays;
use eve_chatterer_core::engine::Engine;
use eve_chatterer_core::pilots::Pilot;
use eve_chatterer_core::presence::Snapshot;
use eve_chatterer_core::settings::Settings;
use serde::Serialize;
use std::sync::{Arc, Mutex};

/// Everything the settings window needs in one shot: the raw layered
/// settings, and every known character with the channels it's actually been
/// seen in (only public/unknown ones vary per character; see `docs/DESIGN.md`
/// "Settings screen").
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SettingsData {
    pub settings: Settings,
    pub pilots: Vec<Pilot>,
    /// Ids of the characters with an EVE client running right now (from the
    /// latest presence sample). `Pilot::live` only means "has ever played",
    /// which is not the same thing and was shown as "Online" by mistake.
    pub online: Vec<String>,
}

pub struct AppState {
    pub overlays: Overlays,
    /// Built during setup, before any window or thread can ask for it.
    pub engine: Arc<Mutex<Engine>>,
    /// The most recent presence sample, refreshed every poll by the runner
    /// thread. Entering reposition mode reads it to start each pilot's
    /// placeholder over its actual client window when one is on screen,
    /// instead of always defaulting to the primary monitor.
    pub last_snapshot: Mutex<Option<Snapshot>>,
    /// True while a global-hotkey reposition session is open. Toggled by
    /// `reposition::toggle`.
    pub repositioning: Mutex<bool>,
}

impl AppState {
    pub fn new(engine: Engine) -> AppState {
        AppState { overlays: Overlays::new(), engine: Arc::new(Mutex::new(engine)), last_snapshot: Mutex::new(None), repositioning: Mutex::new(false) }
    }
}

/// Saves the pilot registry (tags, placements, known channels).
pub fn save_pilots(engine: &Engine) -> Result<(), String> {
    engine.pilots().save(&crate::storage::pilots_path()).map_err(|e| format!("Couldn't save pilots.json: {e}"))
}
