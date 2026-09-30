//! Reposition mode: a global hotkey toggles a draggable, resizable
//! placeholder window for every known character, so the player can position
//! each one's overlay without waiting for a real alert (docs/DESIGN.md,
//! "Overlay reposition & resize"). Modeled on Discord's in-game overlay: a
//! hotkey drops click-through, rather than a settings-screen position grid.

use crate::overlay::{overlay_key, RepositionTarget, DEFAULT_OVERLAY_WIDTH};
use crate::state::AppState;
use eve_chatterer_core::presence::Rect;
use tauri::{AppHandle, Manager};

/// The pilot's own on-screen client: (monitor, viewing area, window). The
/// box is positioned inside that client and owned by it, like everything
/// else the overlay draws.
fn client_region(app: &AppHandle, pilot_name: &str) -> Option<(Rect, Rect, isize)> {
    let state = app.state::<AppState>();
    let snap = state.last_snapshot.lock().unwrap();
    let c = snap.as_ref()?.clients.iter().find(|c| c.character.eq_ignore_ascii_case(pilot_name) && c.on_screen())?;
    let m = c.monitor?;
    Some((m, c.rect.unwrap_or(m), c.hwnd))
}

/// Toggles reposition mode: enters it if it is not active, exits (saving
/// every window's final position) if it is.
pub fn toggle(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut active = state.repositioning.lock().unwrap();
    if *active {
        *active = false;
        drop(active);
        exit(app);
    } else {
        *active = true;
        drop(active);
        enter(app);
    }
}

fn enter(app: &AppHandle) {
    let state = app.state::<AppState>();
    let targets: Vec<RepositionTarget> = {
        let guard = state.engine.lock().unwrap();
        let Some(engine) = guard.as_ref() else {
            *state.repositioning.lock().unwrap() = false;
            return;
        };
        // Only the character the player is actually looking at: the focused
        // client, or (focus elsewhere, e.g. on the settings window) every
        // client on screen. Offline alts and hidden clients would all pile up
        // on the primary monitor on top of each other.
        let wanted: Vec<String> = {
            let snap = state.last_snapshot.lock().unwrap();
            match snap.as_ref() {
                Some(s) => match &s.focused {
                    Some(f) => vec![f.to_lowercase()],
                    None => s.clients.iter().filter(|c| c.on_screen()).map(|c| c.character.to_lowercase()).collect(),
                },
                None => vec![],
            }
        };
        engine
            .pilots()
            .iter()
            .filter(|p| wanted.contains(&p.name.to_lowercase()))
            .filter_map(|p| {
                let (monitor, region, hwnd) = client_region(app, &p.name)?;
                Some(RepositionTarget {
                    key: overlay_key(Some(&p.id), &p.name),
                    name: p.name.clone(),
                    tag: p.display_tag(),
                    accent: crate::runner::accent_for(&p.name),
                    monitor,
                    region,
                    pos: p.placement.map(|pl| (pl.fx, pl.fy)),
                    width: p.placement.map(|pl| pl.width).unwrap_or(DEFAULT_OVERLAY_WIDTH),
                    owner: Some(hwnd),
                })
            })
            .collect()
    };
    if targets.is_empty() {
        println!("[reposition] no EVE client focused or on screen, nothing to position");
        *state.repositioning.lock().unwrap() = false;
        return;
    }
    println!("[reposition] entering for {} character(s)", targets.len());
    state.overlays.enter_reposition(app, targets);
}

fn exit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let results = state.overlays.exit_reposition(app);
    println!("[reposition] exiting, {} window(s) to save", results.len());
    let mut guard = state.engine.lock().unwrap();
    let Some(engine) = guard.as_mut() else { return };
    for (key, placement) in results {
        // `None` here means the read-back failed (e.g. the window vanished
        // mid-session) rather than "the user wants this cleared" - leave
        // whatever was saved before untouched rather than erasing it.
        let Some(placement) = placement else { continue };
        // Reposition mode only ever targets real, registered pilots (see
        // `enter`), so every key here is "id:<pilot id>".
        let Some(id) = key.strip_prefix("id:") else { continue };
        engine.pilots_mut().set_placement(id, Some(placement));
    }
    if let Err(e) = engine.pilots().save(&crate::storage::config_dir().join("pilots.json")) {
        eprintln!("could not save pilots.json after reposition: {e}");
    }
}
