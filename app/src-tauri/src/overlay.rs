//! Overlay windows: one transparent, click-through, non-activating window per
//! *character* (docs/DESIGN.md, "Stack": the resident footprint is the Rust
//! core; WebView2 exists only while alerts are showing, or while the window
//! is being repositioned), created when an alert first needs it and
//! destroyed after it has been idle.
//!
//! Windows used to be keyed by monitor instead of by pilot, so two
//! characters sharing a monitor fought over one window's placement. Keying
//! by pilot fixes that and is also what makes per-character reposition
//! ("Overlay reposition & resize", docs/DESIGN.md) coherent: there is always
//! exactly one window to drag per character.

use crate::diag::diag;
use eve_chatterer_core::pilots::{Edge, OverlayPlacement, MAX_OVERLAY_WIDTH, MIN_OVERLAY_WIDTH};
use eve_chatterer_core::presence::{Rect, Snapshot};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Window height in logical (CSS) pixels: fixed; only width is
/// user-resizable (docs/DESIGN.md, "Overlay reposition & resize": scoped to
/// the character's alert region, not per-style, and width-only so the
/// window never needs to grow taller than the stack of alerts it was
/// designed for).
pub const WIN_H: f64 = 640.0;
/// The default content width (what `.stack` renders at) for a character with
/// no saved placement. Logical pixels.
pub const DEFAULT_OVERLAY_WIDTH: f64 = 460.0;
/// Empty margin around the content on each side, in logical pixels, so glow
/// and shadow have room. Mirrored in overlay.css's `.stack` width calc.
const MARGIN: f64 = 30.0;
/// Height of the draggable box in reposition mode, logical pixels: the
/// window shrinks to just this while being positioned, so it is the box (not
/// a 640px-tall invisible window) that gets clamped inside the game.
/// Mirrored by `.stack.reposition` in overlay.css.
const BOX_H: f64 = 170.0;
/// Gap between the window edge and the alert stack (`.stack` top/bottom in
/// overlay.css).
const STACK_EDGE: f64 = 24.0;
/// How long a window may sit without an alert before it is destroyed.
const IDLE_CLOSE: Duration = Duration::from_secs(45);

fn win_w_for(content_width: f64) -> f64 {
    content_width + MARGIN * 2.0
}

/// Mirrors `Alert` in app/src/overlay/types.ts.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OverlayAlert {
    pub id: u64,
    pub style: &'static str,
    pub pilot: String,
    /// The Strip style's badge text: the pilot's own tag, or one derived from
    /// its name (`eve_chatterer_core::pilots::Pilot::display_tag`).
    pub tag: String,
    pub accent: String,
    /// What the player sees (`runner::channel_label`).
    pub channel: String,
    /// The log's channel id, for matching folds: labels can repeat (every
    /// private conversation is "Private chat").
    pub channel_id: String,
    pub sender: String,
    pub text: String,
    pub reason: String,
    pub tone: &'static str,
    pub lifetime_ms: u32,
    pub count: u32,
    /// Set by `Overlays::show`: this window's alerts grow upward from a box
    /// placed in the lower half of the game.
    pub stack_up: bool,
}

/// Sent to a window entering reposition mode, so it can show a placeholder
/// (dashed outline, name, drag/resize hints) instead of real alerts.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepositionInfo {
    pub name: String,
    pub tag: String,
    pub accent: String,
    /// The hotkey that ends reposition mode, as set in Settings > General.
    pub hotkey: String,
}

// Everything sent to an overlay page goes through the slot's queue until the
// page reports ready: a window created for this very message would otherwise
// miss it (the page is not listening yet), which is exactly what made the
// reposition hotkey need several presses.
enum Msg {
    Alert(OverlayAlert),
    /// A capped line: the page bumps the count of that pilot and channel's
    /// alert if one is showing, and otherwise shows this one.
    Fold(OverlayAlert),
    RepositionEnter(RepositionInfo),
    RepositionExit,
}

struct Slot {
    window: WebviewWindow,
    // The page has loaded and is listening; until then messages are queued.
    ready: bool,
    queue: Vec<Msg>,
    last_used: Instant,
    // When the first message had to wait for the page (cold-start timing).
    waiting_since: Option<Instant>,
    // The EVE client window this overlay belongs to (see `Placement::owner`).
    owner: Option<isize>,
    // Where it was last placed, so `follow` can re-place it when its client
    // moves or resizes, and the scale that placement used.
    layout: Option<Placement>,
    scale: f64,
    // The virtual desktop it was last pinned to (its owner's).
    desktop: Option<u128>,
    // Hidden because its owner was cloaked (desktop switch); shown again
    // when the owner is uncloaked.
    hidden_by_cloak: bool,
}

/// Where an overlay goes. `owner`, when known, is the EVE client window the
/// region belongs to: the overlay becomes an owned window of it, so Windows
/// keeps it directly above that client and nothing else, moves it with the
/// client between virtual desktops, and hides it with the client. This is the
/// closest we can get to Discord's in-game overlay without injecting into the
/// game, which the project rules forbid (CLAUDE.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub monitor: Rect,
    pub region: Rect,
    pub width: f64,
    /// The character's saved spot (only `fx`, `fy` and `edge` are used here).
    pub custom_pos: Option<OverlayPlacement>,
    pub owner: Option<isize>,
}

/// A pilot to show a draggable/resizable placeholder for while entering
/// reposition mode. `pos`, given, is its saved fractional position within
/// `region`; `None` falls back to the same centered placement a real alert
/// would get.
pub struct RepositionTarget {
    pub key: String,
    pub name: String,
    pub tag: String,
    pub accent: String,
    pub monitor: Rect,
    /// The EVE client window (or its monitor, when fullscreen): the box can
    /// never leave it.
    pub region: Rect,
    pub pos: Option<OverlayPlacement>,
    pub width: f64,
    pub owner: Option<isize>,
}

/// One window in reposition mode. Moves and resizes are done here, clamped to
/// `region`, never by the OS: a native window drag could leave the game (or
/// the monitor) and looks like dragging a desktop window.
///
/// `frac` and `w` are the only record of where the box is: every move is
/// computed from them, never read back from the window (asynchronous moves
/// mean a read can lag a step behind, and compounding that is what made the
/// box drift like it was in water).
struct Session {
    region: Rect,
    scale: f64,
    /// The box's spot as a fraction of the region's free space; what is saved.
    frac: (f64, f64),
    /// Physical window width.
    w: i32,
    /// The box's physical height, as measured by the page (it changes as the
    /// sample text re-wraps at different widths). Only used for clamping:
    /// fractions always use the fixed `BOX_H`, so saving and restoring agree
    /// whatever the box measured.
    box_h: i32,
    /// Physical x, y and width when the current drag/resize gesture began.
    start: (i32, i32, i32),
}

impl Session {
    fn ref_h(&self) -> i32 {
        (BOX_H * self.scale).round() as i32
    }

    /// Where the box goes: its remembered spot, kept inside the region.
    fn origin(&self) -> (i32, i32) {
        let (x, y) = box_origin(&self.region, self.frac, self.w, self.ref_h());
        clamp_into(&self.region, x, y, self.w, self.box_h)
    }

    fn set_origin(&mut self, x: i32, y: i32) {
        self.frac = (fraction(x - self.region.left, self.region.width() - self.w), fraction(y - self.region.top, self.region.height() - self.ref_h()));
    }

    /// A box whose middle is in the lower half of the region: its alerts grow
    /// upward from its bottom edge.
    fn anchors_bottom(&self, y: i32, h: i32) -> bool {
        y + h / 2 > self.region.top + self.region.height() / 2
    }

    /// What to save: the width, the old-style fractions, and the anchored edge
    /// where the box actually is (see `OverlayPlacement::edge`).
    fn placement(&self) -> OverlayPlacement {
        let (_, y) = self.origin();
        let bottom = self.anchors_bottom(y, self.box_h);
        let edge_y = if bottom { y + self.box_h } else { y };
        let h = self.region.height().max(1);
        OverlayPlacement {
            fx: self.frac.0,
            fy: self.frac.1,
            width: f64::from(self.w) / self.scale - MARGIN * 2.0,
            edge: Some(Edge { bottom, y: (f64::from(edge_y - self.region.top) / f64::from(h)).clamp(0.0, 1.0) }),
        }
    }
}

pub struct Overlays {
    slots: Mutex<HashMap<String, Slot>>,
    next_id: AtomicU64,
    // Windows (by label) currently in reposition mode: show() still delivers
    // alert content to them, but skips moving the window, so it never fights
    // the drag in progress.
    sessions: Mutex<HashMap<String, Session>>,
}

/// A stable per-pilot key for routing overlay windows to the right window,
/// distinct from the Tauri window label itself (`label_for` sanitizes it
/// further for Tauri's allowed label character set). Falls back to the name,
/// lowercased, when there is no id yet (a synthetic/test alert, or a pilot
/// not yet registered); names are unique in EVE, so this is still a safe
/// per-character key even without one.
pub fn overlay_key(pilot_id: Option<&str>, pilot_name: &str) -> String {
    match pilot_id {
        Some(id) if !id.is_empty() => format!("id:{id}"),
        _ => format!("name:{}", pilot_name.to_lowercase()),
    }
}

fn label_for(key: &str) -> String {
    // Window labels allow letters, digits, -, _, / and :; a character name
    // can have spaces, apostrophes, etc., so sanitize rather than assume the
    // key is already valid.
    let sanitized: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || "-_/:".contains(c) { c } else { '_' }).collect();
    format!("overlay-{sanitized}")
}

fn send(app: &AppHandle, label: &str, msg: &Msg) {
    let r = match msg {
        Msg::Alert(a) => app.emit_to(label, "overlay:alert", a),
        Msg::Fold(f) => app.emit_to(label, "overlay:fold", f),
        Msg::RepositionEnter(r) => app.emit_to(label, "overlay:reposition-enter", r),
        Msg::RepositionExit => app.emit_to(label, "overlay:reposition-exit", ()),
    };
    match r {
        Ok(()) => println!("       [overlay] emitted to {label}"),
        Err(e) => println!("       [overlay] emit_to {label} FAILED: {e}"),
    }
}

/// Sends now if the page is listening, else queues for `ready`.
fn deliver(app: &AppHandle, label: &str, slot: &mut Slot, msg: Msg) {
    if slot.ready {
        send(app, label, &msg);
    } else {
        slot.queue.push(msg);
    }
}

impl Overlays {
    pub fn new() -> Overlays {
        Overlays { slots: Mutex::new(HashMap::new()), next_id: AtomicU64::new(1), sessions: Mutex::new(HashMap::new()) }
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn window_count(&self) -> usize {
        self.slots.lock().unwrap().len()
    }

    /// Makes sure window `label` exists, building it if needed. Built outside
    /// the lock: building a window waits on the UI thread. `Some(true)` if it
    /// was built just now, `None` if it couldn't be.
    fn ensure_slot(&self, app: &AppHandle, label: &str, width: f64, time_first_message: bool) -> Option<bool> {
        if self.slots.lock().unwrap().contains_key(label) {
            return Some(false);
        }
        let started = Instant::now();
        let window = match create(app, label, width) {
            Ok(w) => w,
            Err(e) => {
                println!("       [overlay] could not create window {label}: {e}");
                return None;
            }
        };
        diag(format!("{label}: window built in {} ms", started.elapsed().as_millis()));
        let mut slots = self.slots.lock().unwrap();
        if slots.contains_key(label) {
            let _ = window.destroy(); // another thread built it first (posted, doesn't wait)
            return Some(false);
        }
        let waiting_since = time_first_message.then_some(started);
        slots.insert(
            label.to_string(),
            Slot { window, ready: false, queue: vec![], last_used: Instant::now(), waiting_since, owner: None, layout: None, scale: 1.0, desktop: None, hidden_by_cloak: false },
        );
        Some(true)
    }

    // Shows an alert in the window identified by `key`: centered near the
    // top of the region, or at the saved spot in it (see `place`).
    pub fn show(&self, app: &AppHandle, key: &str, p: Placement, alert: OverlayAlert) {
        self.present(app, key, p, alert, false);
    }

    /// A line past its pilot's rate cap, sent to the window its alert would
    /// have gone to: the page bumps the count on that pilot and channel's
    /// alert if one is showing, or shows `alert` (a Strip) instead, so a
    /// capped line is never lost.
    pub fn fold(&self, app: &AppHandle, key: &str, p: Placement, alert: OverlayAlert) {
        self.present(app, key, p, alert, true);
    }

    fn present(&self, app: &AppHandle, key: &str, p: Placement, mut alert: OverlayAlert, fold: bool) {
        let region = p.region;
        let label = label_for(key);
        let Some(built) = self.ensure_slot(app, &label, p.width, true) else { return };
        let mut slots = self.slots.lock().unwrap();
        let Some(slot) = slots.get_mut(&label) else {
            println!("       [overlay] {label}: slot vanished right after creation");
            return;
        };
        let mid_reposition = self.sessions.lock().unwrap().contains_key(&label);
        if mid_reposition {
            println!("       [overlay] {label}: mid-reposition, delivering without moving the window");
        } else {
            adopt(slot, p.owner);
            let (scale, up) = place(&slot.window, &p);
            alert.stack_up = up;
            slot.scale = scale;
            slot.layout = Some(p);
        }
        platform::show_without_activating(&slot.window);
        println!(
            "       [overlay] {label} {} (ready: {}), placed at region ({},{})-({},{})",
            if built { "created" } else { "reused" },
            slot.ready,
            region.left,
            region.top,
            region.right,
            region.bottom
        );
        slot.last_used = Instant::now();
        let msg = if fold { Msg::Fold(alert) } else { Msg::Alert(alert) };
        if !slot.ready {
            println!("       [overlay] {label}: not ready yet, queuing (queue len will be {})", slot.queue.len() + 1);
        }
        deliver(app, &label, slot, msg);
    }

    /// The overlay page has loaded: flush what was queued while it started.
    pub fn ready(&self, app: &AppHandle, label: &str) {
        let mut slots = self.slots.lock().unwrap();
        if let Some(slot) = slots.get_mut(label) {
            slot.ready = true;
            if let Some(since) = slot.waiting_since.take() {
                diag(format!("{label}: page ready {} ms after the alert asked for it, {} queued", since.elapsed().as_millis(), slot.queue.len()));
            }
            for msg in slot.queue.drain(..) {
                send(app, label, &msg);
            }
        }
    }

    // Destroys windows that have been idle, freeing their WebView2
    // processes. A window mid-reposition is never idle, however long since
    // its last alert; it should not vanish out from under a drag.
    pub fn reap(&self) {
        // Lock order everywhere: slots, then sessions.
        let mut slots = self.slots.lock().unwrap();
        let active = self.sessions.lock().unwrap();
        let idle: Vec<String> =
            slots.iter().filter(|(l, s)| !active.contains_key(*l) && s.last_used.elapsed() > IDLE_CLOSE).map(|(l, _)| l.clone()).collect();
        drop(active);
        for label in idle {
            if let Some(slot) = slots.remove(&label) {
                let _ = slot.window.destroy();
            }
        }
    }

    // Enters reposition mode for every given pilot: shows its placeholder at
    // its current effective position inside its region, and drops
    // click-through so the mouse can reach it.
    pub fn enter_reposition(&self, app: &AppHandle, targets: Vec<RepositionTarget>) {
        for t in targets {
            let label = label_for(&t.key);
            if self.ensure_slot(app, &label, t.width, false).is_none() {
                continue;
            }
            let scale = scale_for(&t.monitor);
            let mut s = Session {
                region: t.region,
                scale,
                frac: (0.5, 0.0),
                w: (win_w_for(t.width) * scale).round() as i32,
                box_h: (BOX_H * scale).round() as i32,
                start: (0, 0, 0),
            };
            if let Some(pos) = t.pos {
                s.frac = (pos.fx, pos.fy);
                // A saved anchored edge puts the box's top (or bottom) exactly there.
                if let Some(e) = pos.edge {
                    let (x, _) = s.origin();
                    let edge_y = edge_to_y(&s.region, e);
                    s.set_origin(x, if e.bottom { edge_y - s.box_h } else { edge_y });
                }
            }
            let window = {
                let mut slots = self.slots.lock().unwrap();
                let Some(slot) = slots.get_mut(&label) else { continue };
                adopt(slot, t.owner);
                let (x, y) = s.origin();
                // Posted to the UI thread, not waited on.
                let _ = slot.window.set_size(PhysicalSize::new(s.w as u32, s.box_h as u32));
                let _ = slot.window.set_position(PhysicalPosition::new(x, y));
                let hotkey = crate::hotkey::current_text().unwrap_or_else(|| eve_chatterer_core::settings::DEFAULT_REPOSITION_HOTKEY.to_string());
                deliver(app, &label, slot, Msg::RepositionEnter(RepositionInfo { name: t.name, tag: t.tag, accent: t.accent, hotkey }));
                self.sessions.lock().unwrap().insert(label, s);
                slot.window.clone()
            };
            // Outside the locks: changing the window's style waits on the UI
            // thread, which owns it (state.rs, "Locking rules").
            platform::set_click_through(&window, false);
            platform::show_without_activating(&window);
        }
    }

    /// A drag or resize gesture began on `label`: remember where from, so
    /// the deltas that follow are applied to a fixed starting point.
    pub fn gesture_start(&self, label: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = sessions.get_mut(label) else { return };
        let (x, y) = s.origin();
        s.start = (x, y, s.w);
    }

    /// Moves `label` by (dx, dy) logical pixels from where the gesture began,
    /// clamped so the whole box stays inside its region.
    pub fn gesture_move(&self, label: &str, dx: f64, dy: f64) {
        let slots = self.slots.lock().unwrap();
        let mut sessions = self.sessions.lock().unwrap();
        let (Some(slot), Some(s)) = (slots.get(label), sessions.get_mut(label)) else { return };
        let (x0, y0, _) = s.start;
        let (x, y) = clamp_into(&s.region, x0 + (dx * s.scale).round() as i32, y0 + (dy * s.scale).round() as i32, s.w, s.box_h);
        s.set_origin(x, y);
        platform::move_to(&slot.window, x, y);
    }

    /// Widens/narrows `label` by `dx` logical pixels from the gesture's start
    /// (its left edge stays put), within the readable range and the region.
    pub fn gesture_resize(&self, label: &str, dx: f64) {
        let slots = self.slots.lock().unwrap();
        let mut sessions = self.sessions.lock().unwrap();
        let (Some(slot), Some(s)) = (slots.get(label), sessions.get_mut(label)) else { return };
        let (x0, y0, w0) = s.start;
        let min = (win_w_for(MIN_OVERLAY_WIDTH) * s.scale).round() as i32;
        let max = ((win_w_for(MAX_OVERLAY_WIDTH) * s.scale).round() as i32).min(s.region.right - x0).max(min);
        s.w = (w0 + (dx * s.scale).round() as i32).clamp(min, max);
        let (x, y) = clamp_into(&s.region, x0, y0, s.w, s.box_h);
        s.set_origin(x, y);
        let _ = slot.window.set_size(PhysicalSize::new(s.w as u32, s.box_h as u32));
        platform::move_to(&slot.window, x, y);
    }

    /// The page measured its box at `h` logical pixels tall (the sample text
    /// re-wraps as the width changes): size the window to exactly fit it.
    /// The remembered spot does not change, only how it is clamped.
    pub fn set_box_height(&self, label: &str, h: f64) {
        let slots = self.slots.lock().unwrap();
        let mut sessions = self.sessions.lock().unwrap();
        let (Some(slot), Some(s)) = (slots.get(label), sessions.get_mut(label)) else { return };
        let box_h = (h * s.scale).round() as i32;
        if box_h == s.box_h || box_h <= 0 {
            return;
        }
        // A box in the lower half keeps its bottom edge where it is (that's
        // the edge its alerts will grow up from); one in the top half keeps its top.
        let (x, y) = s.origin();
        let old_h = s.box_h;
        s.box_h = box_h;
        if s.anchors_bottom(y, old_h) {
            s.set_origin(x, y + old_h - box_h);
        }
        let (x, y) = s.origin();
        let _ = slot.window.set_size(PhysicalSize::new(s.w as u32, s.box_h as u32));
        platform::move_to(&slot.window, x, y);
    }

    /// Keeps every overlay inside its EVE client as the client moves or is
    /// resized, called on every presence sample. Positioned boxes keep their
    /// relative spot; alerts are re-placed exactly as when they were shown.
    pub fn follow(&self, snap: &Snapshot) {
        for c in &snap.clients {
            // Minimized clients have no rect: their overlays hide with them.
            if let (Some(region), Some(monitor)) = (c.rect, c.monitor) {
                self.follow_client(c.hwnd, region, monitor);
            }
        }
        // Keep each overlay on its client's virtual desktop, in case the
        // client itself was moved to another one since it was pinned.
        let mut slots = self.slots.lock().unwrap();
        for slot in slots.values_mut() {
            if let Some(owner) = slot.owner {
                pin_to_owner_desktop(slot, owner);
            }
        }
    }

    /// Is `hwnd` the owner of any overlay? The event hook's cheap filter.
    pub fn is_owner(&self, hwnd: isize) -> bool {
        self.slots.lock().unwrap().values().any(|s| s.owner == Some(hwnd))
    }

    /// The client `owner` now has viewing area `region` on `monitor`: move
    /// its overlays to match. Called from the move hook as the client is
    /// dragged, and on every presence sample. Positions are always computed
    /// from what was placed (never read back from the window, which may not
    /// have caught up with an earlier move yet), so nothing drifts.
    pub fn follow_client(&self, owner: isize, region: Rect, monitor: Rect) {
        let mut slots = self.slots.lock().unwrap();
        let mut sessions = self.sessions.lock().unwrap();
        for (label, slot) in slots.iter_mut() {
            if slot.owner != Some(owner) {
                continue;
            }
            if let Some(s) = sessions.get_mut(label) {
                if s.region != region {
                    s.region = region;
                    let (x, y) = s.origin();
                    platform::move_to(&slot.window, x, y);
                }
            } else if let Some(p) = slot.layout.as_mut() {
                if p.region == region {
                    continue;
                }
                p.region = region;
                if p.monitor != monitor {
                    // A different monitor may have a different scale: size it again.
                    p.monitor = monitor;
                    let p = *p;
                    slot.scale = place(&slot.window, &p).0;
                } else {
                    let (x, y, ..) = geometry(p, slot.scale);
                    platform::move_to(&slot.window, x, y);
                }
            }
        }
    }

    /// The client `owner` was cloaked (the shell hides a window's desktop
    /// contents the instant you switch desktops) or uncloaked: hide or show
    /// its overlays in the same moment, so none of them flashes on the
    /// desktop you are switching to.
    pub fn set_owner_cloaked(&self, owner: isize, cloaked: bool) {
        let mut slots = self.slots.lock().unwrap();
        for slot in slots.values_mut().filter(|s| s.owner == Some(owner)) {
            if cloaked {
                if platform::hide(&slot.window) {
                    slot.hidden_by_cloak = true;
                }
            } else if slot.hidden_by_cloak {
                slot.hidden_by_cloak = false;
                platform::show_without_activating(&slot.window);
            }
        }
    }

    // Exits reposition mode: saves each box's remembered spot and width,
    // restores click-through, and returns what to persist, by pilot key.
    pub fn exit_reposition(&self, app: &AppHandle) -> Vec<(String, Option<OverlayPlacement>)> {
        let sessions: Vec<(String, Session)> = self.sessions.lock().unwrap().drain().collect();
        let mut out = Vec::with_capacity(sessions.len());
        for (label, s) in sessions {
            let key = key_from_label(&label);
            let window = {
                let mut slots = self.slots.lock().unwrap();
                let Some(slot) = slots.get_mut(&label) else {
                    out.push((key, None));
                    continue;
                };
                deliver(app, &label, slot, Msg::RepositionExit);
                slot.last_used = Instant::now();
                slot.window.clone()
            };
            // Outside the lock: it waits on the UI thread.
            platform::set_click_through(&window, true);
            out.push((key, Some(s.placement())));
        }
        out
    }
}

/// Hands the slot's window to `owner` (the EVE client it is drawn over), if
/// that changed since last time.
fn adopt(slot: &mut Slot, owner: Option<isize>) {
    if slot.owner != owner {
        platform::set_owner(&slot.window, owner);
        slot.owner = owner;
        slot.desktop = None;
    }
    if let Some(owner) = owner {
        pin_to_owner_desktop(slot, owner);
    }
}

/// Puts the overlay on the same virtual desktop as its client, so the shell
/// hides it together with the client the moment desktops switch (owned
/// windows from another process are not always moved with their owner,
/// which showed as a split-second flash on the new desktop).
fn pin_to_owner_desktop(slot: &mut Slot, owner: isize) {
    let Some(desktop) = platform::desktop_of(owner) else { return };
    if slot.desktop != Some(desktop) && platform::move_to_desktop(&slot.window, desktop) {
        slot.desktop = Some(desktop);
    }
}

/// Clamps a w x h window at (x, y) so it lies entirely inside `region`
/// (pinned to the top-left if the region is smaller than the window).
fn clamp_into(region: &Rect, x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    (x.clamp(region.left, (region.right - w).max(region.left)), y.clamp(region.top, (region.bottom - h).max(region.top)))
}

fn fraction(offset: i32, free: i32) -> f64 {
    if free <= 0 {
        0.5
    } else {
        (offset as f64 / free as f64).clamp(0.0, 1.0)
    }
}

fn key_from_label(label: &str) -> String {
    label.strip_prefix("overlay-").unwrap_or(label).to_string()
}

/// How the overlay's lifetime meter animates: "smooth", "stepped" or "off".
/// Stepped is the default: continuous animation makes the desktop compositor
/// recompose the region over the game every frame (docs/FINDINGS.md #9).
/// `EVE_CHATTERER_METER` overrides it for measurement.
fn meter_mode() -> String {
    std::env::var("EVE_CHATTERER_METER").ok().filter(|m| ["smooth", "stepped", "off"].contains(&m.as_str())).unwrap_or_else(|| "stepped".into())
}

/// Measurement only: `EVE_CHATTERER_FX=noshadow,noarrive,nopulse` (any
/// subset) switches those effects off, to find what the CPU while animating
/// goes to. Unknown words are dropped; unset means everything on.
fn fx_off() -> String {
    const KNOWN: [&str; 3] = ["noshadow", "noarrive", "nopulse"];
    let raw = std::env::var("EVE_CHATTERER_FX").unwrap_or_default();
    raw.split(',').map(str::trim).filter(|w| KNOWN.contains(w)).collect::<Vec<_>>().join("+")
}

fn create(app: &AppHandle, label: &str, width: f64) -> tauri::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("overlay.html?meter={}&fx={}", meter_mode(), fx_off()).into());
    let window = WebviewWindowBuilder::new(app, label, url)
        .title("EVE Chatterer overlay")
        .inner_size(win_w_for(width), WIN_H)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .resizable(false)
        .shadow(false);
    let window = crate::storage::with_webview_dir(window).build()?;
    window.set_ignore_cursor_events(true)?; // click-through
    platform::never_activate(&window);
    Ok(window)
}

/// The scale factor of `monitor`. From Windows directly: Tauri's monitor
/// list waits on the UI thread, and this is asked while holding the lock.
#[cfg(windows)]
fn scale_for(monitor: &Rect) -> f64 {
    eve_chatterer_core::winapi::scale_at(monitor.left + monitor.width() / 2, monitor.top + monitor.height() / 2)
}

#[cfg(not(windows))]
fn scale_for(_: &Rect) -> f64 {
    1.0
}

/// Top-left of the positioned box (physical) for a saved fractional spot in
/// `region`, given the box's physical width and height.
fn box_origin(region: &Rect, (fx, fy): (f64, f64), w: i32, h: i32) -> (i32, i32) {
    let x = region.left + (fx * f64::from((region.width() - w).max(0))).round() as i32;
    let y = region.top + (fy * f64::from((region.height() - h).max(0))).round() as i32;
    clamp_into(region, x, y, w, h)
}

/// The physical y of a saved anchored edge in `region`.
fn edge_to_y(region: &Rect, e: Edge) -> i32 {
    (region.top + (e.y * f64::from(region.height())).round() as i32).clamp(region.top, region.bottom)
}

/// Sizes and positions a window for real alerts, returning the scale it used
/// and whether its alerts should stack upward (see `geometry`). Tauri posts
/// the size and position to the UI thread without waiting.
fn place(window: &WebviewWindow, p: &Placement) -> (f64, bool) {
    let scale = scale_for(&p.monitor);
    let (x, y, w, h, up) = geometry(p, scale);
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = window.set_position(PhysicalPosition::new(x, y));
    (scale, up)
}

/// Physical (x, y, w, h) for an alert window, and whether its alerts stack
/// upward: a box saved in the lower half of the region grows up from it, so
/// the stack never runs off the bottom of the game.
fn geometry(p: &Placement, scale: f64) -> (i32, i32, i32, i32, bool) {
    let (monitor, region, width, custom_pos) = (&p.monitor, &p.region, p.width, p.custom_pos);
    let w = (win_w_for(width) * scale).round() as i32;
    let h = (WIN_H * scale).round() as i32;
    let (x, y, up) = match custom_pos {
        Some(pos) => {
            let box_h = (BOX_H * scale).round() as i32;
            let top = (STACK_EDGE * scale).round() as i32;
            let (bx, by) = box_origin(region, (pos.fx, pos.fy), w, box_h);
            match pos.edge {
                // The stack's first alert starts exactly on the saved edge.
                Some(e) if e.bottom => (bx, edge_to_y(region, e) + top - h, true),
                Some(e) => (bx, edge_to_y(region, e) - top, false),
                // Saved before edges existed: the box's top plus `BOX_H`.
                None if pos.fy > 0.5 => (bx, by + box_h + top - h, true),
                None => (bx, by - top, false),
            }
        }
        None => {
            let (x, y) = clamp_into(monitor, region.left + (region.width() - w) / 2, region.top, w, h);
            (x, y, false)
        }
    };
    (x, y, w, h, up)
}

#[cfg(windows)]
mod platform {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tauri::WebviewWindow;
    use windows::core::GUID;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, ShowWindowAsync, SW_HIDE, GWLP_HWNDPARENT, GWL_EXSTYLE, HWND_NOTOPMOST,
        HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };

    fn hwnd(window: &WebviewWindow) -> Option<HWND> {
        let handle = window.window_handle().ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(w) => Some(HWND(w.hwnd.get() as *mut _)),
            _ => None,
        }
    }

    /// Moves without resizing, reordering or activating, and without waiting
    /// for the UI thread to process it (the window belongs to that thread).
    pub fn move_to(window: &WebviewWindow, x: i32, y: i32) {
        if let Some(h) = hwnd(window) {
            unsafe {
                let _ = SetWindowPos(h, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
            }
        }
    }

    thread_local! {
        // COM is per thread; the shell's virtual desktop manager is cheap to
        // keep once created. `None` if it isn't available (very old Windows).
        static DESKTOPS: Option<IVirtualDesktopManager> = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL).ok()
        };
    }

    /// The virtual desktop a (top-level) window is on.
    pub fn desktop_of(owner: isize) -> Option<u128> {
        DESKTOPS.with(|m| unsafe { m.as_ref()?.GetWindowDesktopId(HWND(owner as *mut _)).ok() }).map(|g| g.to_u128())
    }

    /// Moves one of our own windows to a virtual desktop. Only works for
    /// windows this process owns, which the overlays are.
    pub fn move_to_desktop(window: &WebviewWindow, desktop: u128) -> bool {
        let Some(h) = hwnd(window) else { return false };
        DESKTOPS.with(|m| m.as_ref().is_some_and(|m| unsafe { m.MoveWindowToDesktop(h, &GUID::from_u128(desktop)) }.is_ok()))
    }

    // The window must never take focus or appear in Alt-Tab, whether it is
    // click-through right now or (mid-reposition) briefly is not.
    // Click-through itself (WS_EX_TRANSPARENT) is toggled separately by
    // set_ignore_cursor_events, which this does not touch.
    pub fn never_activate(window: &WebviewWindow) {
        let Some(h) = hwnd(window) else { return };
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize);
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }

    /// Turns click-through on or off by flipping only `WS_EX_TRANSPARENT`.
    /// Not through Tauri's `set_ignore_cursor_events`: that rewrites the
    /// whole extended style from its own records, dropping our
    /// `WS_EX_NOACTIVATE`, and then `ShowWindow(SW_SHOW)`s the window. With
    /// that gone, grabbing the reposition box activated it, EVE stopped being
    /// the fullscreen foreground window, and Windows showed the taskbar.
    ///
    /// `WS_EX_LAYERED` goes with it, as Tauri does: Windows hit-tests a
    /// layered window against the size it had when it became layered, so
    /// leaving it on made a box widened in reposition mode take the mouse
    /// only across its old width (the rest fell through to the game).
    pub fn set_click_through(window: &WebviewWindow, on: bool) {
        let Some(h) = hwnd(window) else { return };
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize;
            let through = (WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0) as isize;
            let ex = if on { ex | through } else { ex & !through };
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex);
            let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_ASYNCWINDOWPOS);
        }
    }

    /// Shows without activating, and without waiting for the UI thread
    /// that owns the window (`ShowWindow` would).
    pub fn show_without_activating(window: &WebviewWindow) {
        if let Some(h) = hwnd(window) {
            unsafe {
                let _ = ShowWindowAsync(h, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// Hides a visible window without waiting on the UI thread that owns it.
    /// Returns whether it was visible (so only those are shown again).
    pub fn hide(window: &WebviewWindow) -> bool {
        let Some(h) = hwnd(window) else { return false };
        unsafe {
            if !IsWindowVisible(h).as_bool() {
                return false;
            }
            let _ = ShowWindowAsync(h, SW_HIDE);
        }
        true
    }

    /// Makes `owner` (an EVE client window) own this overlay, or releases it.
    /// An owned window always sits directly above its owner, follows it to
    /// other virtual desktops and hides when it is minimized, so it no longer
    /// needs to be topmost over every other app. With no owner (nothing to
    /// anchor to) it falls back to topmost.
    pub fn set_owner(window: &WebviewWindow, owner: Option<isize>) {
        let Some(h) = hwnd(window) else { return };
        unsafe {
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, owner.unwrap_or(0));
            let after = if owner.is_some() { HWND_NOTOPMOST } else { HWND_TOPMOST };
            let _ = SetWindowPos(h, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use tauri::WebviewWindow;

    pub fn never_activate(_: &WebviewWindow) {}

    pub fn set_click_through(window: &WebviewWindow, on: bool) {
        let _ = window.set_ignore_cursor_events(on);
    }

    pub fn set_owner(_: &WebviewWindow, _: Option<isize>) {}

    pub fn hide(window: &WebviewWindow) -> bool {
        window.is_visible().unwrap_or(false) && window.hide().is_ok()
    }

    pub fn move_to(window: &WebviewWindow, x: i32, y: i32) {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }

    pub fn desktop_of(_: isize) -> Option<u128> {
        None
    }

    pub fn move_to_desktop(_: &WebviewWindow, _: u128) -> bool {
        false
    }

    pub fn show_without_activating(window: &WebviewWindow) {
        let _ = window.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_valid_and_distinct_for_names_with_odd_characters() {
        let keys = [overlay_key(Some("12345"), "Holden"), overlay_key(None, "Test Person / Jr."), overlay_key(None, "Naomi Nagata")];
        let labels: Vec<String> = keys.iter().map(|k| label_for(k)).collect();
        for l in &labels {
            assert!(l.chars().all(|c| c.is_ascii_alphanumeric() || "-_/:".contains(c)), "{l}");
            assert!(l.starts_with("overlay-"), "{l}");
        }
        assert_eq!(labels.iter().collect::<std::collections::HashSet<_>>().len(), 3, "{labels:?}");
    }

    #[test]
    fn overlay_key_prefers_the_id_and_falls_back_to_a_lowercased_name() {
        assert_eq!(overlay_key(Some("42"), "Holden"), "id:42");
        assert_eq!(overlay_key(None, "Holden"), "name:holden");
        assert_eq!(overlay_key(Some(""), "Holden"), "name:holden");
    }

    #[test]
    fn key_from_label_round_trips_through_label_for_for_simple_keys() {
        assert_eq!(key_from_label(&label_for("id:42")), "id:42");
    }

    const REGION: Rect = Rect { left: 100, top: 50, right: 1700, bottom: 950 };

    /// A reposition session with a box of this height placed at (x, y).
    fn session_at(y: i32, box_h: i32) -> Session {
        let mut s = Session { region: REGION, scale: 1.0, frac: (0.5, 0.0), w: 520, box_h, start: (0, 0, 0) };
        s.set_origin(400, y);
        s
    }

    /// Where the saved placement puts the edge the alert stack starts from.
    fn alert_edge(p: OverlayPlacement) -> (i32, bool) {
        let placement = Placement { monitor: REGION, region: REGION, width: 460.0, custom_pos: Some(p), owner: None };
        let (_, y, _, h, up) = geometry(&placement, 1.0);
        let pad = STACK_EDGE as i32;
        (if up { y + h - pad } else { y + pad }, up)
    }

    #[test]
    fn alerts_start_exactly_on_the_boxs_edge_whatever_its_height() {
        for box_h in [120, 170, 240] {
            // Top half: alerts grow down from the box's top edge.
            let s = session_at(200, box_h);
            assert_eq!(alert_edge(s.placement()), (200, false), "top, box {box_h}");
            // Lower half: they grow up from its bottom edge. This is what the
            // fixed BOX_H used to get wrong by (box_h - BOX_H).
            let s = session_at(700, box_h);
            assert_eq!(alert_edge(s.placement()), (700 + box_h, true), "bottom, box {box_h}");
        }
    }

    #[test]
    fn a_placement_saved_before_edges_still_places_the_old_way() {
        let old = OverlayPlacement { fx: 0.5, fy: 0.9, width: 460.0, edge: None };
        let (edge, up) = alert_edge(old);
        assert!(up);
        let box_top = REGION.top + (0.9 * f64::from(REGION.height() - BOX_H as i32)).round() as i32;
        assert_eq!(edge, box_top + BOX_H as i32);
    }
}
