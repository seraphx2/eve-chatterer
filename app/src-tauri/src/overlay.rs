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
use eve_chatterer_core::pilots::{OverlayPlacement, MAX_OVERLAY_WIDTH, MIN_OVERLAY_WIDTH};
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
    pub channel: String,
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

#[derive(Serialize, Clone, Debug)]
pub struct Fold {
    pub pilot: String,
    pub channel: String,
}

/// Sent to a window entering reposition mode, so it can show a placeholder
/// (dashed outline, name, drag/resize hints) instead of real alerts.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepositionInfo {
    pub name: String,
    pub tag: String,
    pub accent: String,
}

// Everything sent to an overlay page goes through the slot's queue until the
// page reports ready: a window created for this very message would otherwise
// miss it (the page is not listening yet), which is exactly what made the
// reposition hotkey need several presses.
enum Msg {
    Alert(OverlayAlert),
    Fold(Fold),
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
    pub custom_pos: Option<(f64, f64)>,
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
    pub pos: Option<(f64, f64)>,
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

    // Shows an alert for the pilot identified by `key`: centered near the
    // top of the region, or at the pilot's saved spot in it (see `place`).
    pub fn show(&self, app: &AppHandle, key: &str, p: Placement, mut alert: OverlayAlert) {
        let (width, region) = (p.width, p.region);
        let label = label_for(key);
        let existed = self.slots.lock().unwrap().contains_key(&label);
        if !existed {
            // Built outside the lock: creating a window pumps the UI thread.
            let started = Instant::now();
            match create(app, &label, width) {
                Ok(window) => {
                    diag(format!("{label}: window built in {} ms", started.elapsed().as_millis()));
                    let mut slots = self.slots.lock().unwrap();
                    if slots.contains_key(&label) {
                        let _ = window.destroy(); // another thread created it first
                    } else {
                        slots.insert(
                            label.clone(),
                            Slot { window, ready: false, queue: vec![], last_used: Instant::now(), waiting_since: Some(started), owner: None, layout: None, scale: 1.0, desktop: None, hidden_by_cloak: false },
                        );
                    }
                }
                Err(e) => {
                    println!("       [overlay] could not create window {label}: {e}");
                    return;
                }
            }
        }
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
            let (scale, up) = place(app, &slot.window, &p);
            alert.stack_up = up;
            slot.scale = scale;
            slot.layout = Some(p);
        }
        let visible = slot.window.is_visible().unwrap_or(false);
        platform::show_without_activating(&slot.window);
        println!(
            "       [overlay] {label} {} (was visible: {visible}, ready: {}), placed at region ({},{})-({},{})",
            if existed { "reused" } else { "created" },
            slot.ready,
            region.left,
            region.top,
            region.right,
            region.bottom
        );
        slot.last_used = Instant::now();
        let msg = Msg::Alert(alert);
        if slot.ready {
            send(app, &label, &msg);
        } else {
            println!("       [overlay] {label}: not ready yet, queuing (queue len will be {})", slot.queue.len() + 1);
            slot.queue.push(msg);
        }
    }

    /// A line past its pilot's rate cap: tell every open overlay to bump the badge.
    pub fn fold(&self, app: &AppHandle, f: Fold) {
        let mut slots = self.slots.lock().unwrap();
        for (label, slot) in slots.iter_mut() {
            if slot.ready {
                send(app, label, &Msg::Fold(f.clone()));
                slot.last_used = Instant::now();
            }
        }
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
            let existed = self.slots.lock().unwrap().contains_key(&label);
            if !existed {
                match create(app, &label, t.width) {
                    Ok(window) => {
                        let mut slots = self.slots.lock().unwrap();
                        if !slots.contains_key(&label) {
                            slots.insert(
                                label.clone(),
                                Slot { window, ready: false, queue: vec![], last_used: Instant::now(), waiting_since: None, owner: None, layout: None, scale: 1.0, desktop: None, hidden_by_cloak: false },
                            );
                        }
                    }
                    Err(e) => {
                        println!("       [overlay] reposition: could not create window {label}: {e}");
                        continue;
                    }
                }
            }
            let scale = scale_for(app, &t.monitor);
            let s = Session {
                region: t.region,
                scale,
                frac: t.pos.unwrap_or((0.5, 0.0)),
                w: (win_w_for(t.width) * scale).round() as i32,
                box_h: (BOX_H * scale).round() as i32,
                start: (0, 0, 0),
            };
            let mut slots = self.slots.lock().unwrap();
            let Some(slot) = slots.get_mut(&label) else { continue };
            adopt(slot, t.owner);
            let (x, y) = s.origin();
            let _ = slot.window.set_size(PhysicalSize::new(s.w as u32, s.box_h as u32));
            let _ = slot.window.set_position(PhysicalPosition::new(x, y));
            let _ = slot.window.set_ignore_cursor_events(false);
            platform::show_without_activating(&slot.window);
            deliver(app, &label, slot, Msg::RepositionEnter(RepositionInfo { name: t.name, tag: t.tag, accent: t.accent }));
            self.sessions.lock().unwrap().insert(label, s);
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
        s.box_h = box_h;
        let (x, y) = s.origin();
        let _ = slot.window.set_size(PhysicalSize::new(s.w as u32, s.box_h as u32));
        platform::move_to(&slot.window, x, y);
    }

    /// Keeps every overlay inside its EVE client as the client moves or is
    /// resized, called on every presence sample. Positioned boxes keep their
    /// relative spot; alerts are re-placed exactly as when they were shown.
    pub fn follow(&self, app: &AppHandle, snap: &Snapshot) {
        for c in &snap.clients {
            // Minimized clients have no rect: their overlays hide with them.
            if let (Some(region), Some(monitor)) = (c.rect, c.monitor) {
                self.follow_client(app, c.hwnd, region, monitor);
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
    pub fn follow_client(&self, app: &AppHandle, owner: isize, region: Rect, monitor: Rect) {
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
                    slot.scale = place(app, &slot.window, &p).0;
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
            let mut slots = self.slots.lock().unwrap();
            let Some(slot) = slots.get_mut(&label) else {
                out.push((key, None));
                continue;
            };
            deliver(app, &label, slot, Msg::RepositionExit);
            let _ = slot.window.set_ignore_cursor_events(true);
            slot.last_used = Instant::now();
            drop(slots);
            out.push((key, Some(OverlayPlacement { fx: s.frac.0, fy: s.frac.1, width: f64::from(s.w) / s.scale - MARGIN * 2.0 })));
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

fn create(app: &AppHandle, label: &str, width: f64) -> tauri::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("overlay.html?meter={}", meter_mode()).into());
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
        .shadow(false)
        .build()?;
    window.set_ignore_cursor_events(true)?; // click-through
    platform::never_activate(&window);
    Ok(window)
}

/// The scale factor of the monitor whose top-left corner is at `monitor`.
fn scale_for(app: &AppHandle, monitor: &Rect) -> f64 {
    app.available_monitors()
        .ok()
        .and_then(|ms| ms.into_iter().find(|m| m.position().x == monitor.left && m.position().y == monitor.top))
        .map(|m| m.scale_factor())
        .unwrap_or(1.0)
}

/// Top-left of the positioned box (physical) for a saved fractional spot in
/// `region`, given the box's physical width and height.
fn box_origin(region: &Rect, (fx, fy): (f64, f64), w: i32, h: i32) -> (i32, i32) {
    let x = region.left + (fx * f64::from((region.width() - w).max(0))).round() as i32;
    let y = region.top + (fy * f64::from((region.height() - h).max(0))).round() as i32;
    clamp_into(region, x, y, w, h)
}

/// Sizes and positions a window for real alerts, returning the scale it used
/// and whether its alerts should stack upward (see `geometry`).
fn place(app: &AppHandle, window: &WebviewWindow, p: &Placement) -> (f64, bool) {
    let scale = scale_for(app, &p.monitor);
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
            let (bx, by) = box_origin(region, pos, w, box_h);
            if pos.1 > 0.5 {
                (bx, by + box_h + top - h, true)
            } else {
                (bx, by - top, false)
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
        GetWindowLongPtrW, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, ShowWindow, ShowWindowAsync, SW_HIDE, GWLP_HWNDPARENT, GWL_EXSTYLE, HWND_NOTOPMOST,
        HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
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

    pub fn show_without_activating(window: &WebviewWindow) {
        if let Some(h) = hwnd(window) {
            unsafe {
                let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
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
            let _ = SetWindowPos(h, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use tauri::WebviewWindow;

    pub fn never_activate(_: &WebviewWindow) {}

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
        let keys = [overlay_key(Some("12345"), "Jarna"), overlay_key(None, "Test Person / Jr."), overlay_key(None, "Psianna Archeia")];
        let labels: Vec<String> = keys.iter().map(|k| label_for(k)).collect();
        for l in &labels {
            assert!(l.chars().all(|c| c.is_ascii_alphanumeric() || "-_/:".contains(c)), "{l}");
            assert!(l.starts_with("overlay-"), "{l}");
        }
        assert_eq!(labels.iter().collect::<std::collections::HashSet<_>>().len(), 3, "{labels:?}");
    }

    #[test]
    fn overlay_key_prefers_the_id_and_falls_back_to_a_lowercased_name() {
        assert_eq!(overlay_key(Some("42"), "Jarna"), "id:42");
        assert_eq!(overlay_key(None, "Jarna"), "name:jarna");
        assert_eq!(overlay_key(Some(""), "Jarna"), "name:jarna");
    }

    #[test]
    fn key_from_label_round_trips_through_label_for_for_simple_keys() {
        assert_eq!(key_from_label(&label_for("id:42")), "id:42");
    }
}
