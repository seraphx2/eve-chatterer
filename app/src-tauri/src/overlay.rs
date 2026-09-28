//! Overlay windows: one transparent, click-through, non-activating window per
//! monitor, created when an alert first needs it and destroyed after it has
//! been idle (docs/DESIGN.md, "Stack": the resident footprint is the Rust core;
//! WebView2 exists only while alerts are showing).

use crate::diag::diag;
use eve_chatterer_core::presence::Rect;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Window size in logical (CSS) pixels. The window is transparent and
/// click-through, so the empty part costs nothing but compositing.
pub const WIN_W: f64 = 520.0;
pub const WIN_H: f64 = 640.0;
/// How long a window may sit without an alert before it is destroyed.
const IDLE_CLOSE: Duration = Duration::from_secs(45);

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
}

#[derive(Serialize, Clone, Debug)]
pub struct Fold {
    pub pilot: String,
    pub channel: String,
}

enum Msg {
    Alert(OverlayAlert),
    Fold(Fold),
}

struct Slot {
    window: WebviewWindow,
    /// The page has loaded and is listening; until then messages are queued.
    ready: bool,
    queue: Vec<Msg>,
    last_used: Instant,
    /// When the first message had to wait for the page (cold-start timing).
    waiting_since: Option<Instant>,
}

pub struct Overlays {
    slots: Mutex<HashMap<String, Slot>>,
    next_id: AtomicU64,
}

fn label_for(monitor: &Rect) -> String {
    // Window labels allow letters, digits, '-', '_', '/' and ':', so a negative origin is fine.
    format!("overlay-{}_{}", monitor.left, monitor.top)
}

fn send(app: &AppHandle, label: &str, msg: &Msg) {
    let r = match msg {
        Msg::Alert(a) => app.emit_to(label, "overlay:alert", a),
        Msg::Fold(f) => app.emit_to(label, "overlay:fold", f),
    };
    match r {
        Ok(()) => println!("       [overlay] emitted to {label}"),
        Err(e) => println!("       [overlay] emit_to {label} FAILED: {e}"),
    }
}

impl Overlays {
    pub fn new() -> Overlays {
        Overlays { slots: Mutex::new(HashMap::new()), next_id: AtomicU64::new(1) }
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn window_count(&self) -> usize {
        self.slots.lock().unwrap().len()
    }

    /// Shows an alert on `monitor`, centered near the top of `region` (the
    /// monitor itself, or a client window's bounds).
    pub fn show(&self, app: &AppHandle, monitor: Rect, region: Rect, alert: OverlayAlert) {
        let label = label_for(&monitor);
        let existed = self.slots.lock().unwrap().contains_key(&label);
        if !existed {
            // Built outside the lock: creating a window pumps the UI thread.
            let started = Instant::now();
            match create(app, &label) {
                Ok(window) => {
                    diag(format!("{label}: window built in {} ms", started.elapsed().as_millis()));
                    let mut slots = self.slots.lock().unwrap();
                    if slots.contains_key(&label) {
                        let _ = window.destroy(); // another thread created it first
                    } else {
                        slots.insert(
                            label.clone(),
                            Slot { window, ready: false, queue: vec![], last_used: Instant::now(), waiting_since: Some(started) },
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
        place(app, &slot.window, &monitor, &region);
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

    /// Destroys windows that have been idle, freeing their WebView2 processes.
    pub fn reap(&self) {
        let mut slots = self.slots.lock().unwrap();
        let idle: Vec<String> =
            slots.iter().filter(|(_, s)| s.last_used.elapsed() > IDLE_CLOSE).map(|(l, _)| l.clone()).collect();
        for label in idle {
            if let Some(slot) = slots.remove(&label) {
                let _ = slot.window.destroy();
            }
        }
    }
}

/// How the overlay's lifetime meter animates: "smooth", "stepped" or "off".
/// Stepped is the default: continuous animation makes the desktop compositor
/// recompose the region over the game every frame (docs/FINDINGS.md #9).
/// `EVE_CHATTERER_METER` overrides it for measurement.
fn meter_mode() -> String {
    std::env::var("EVE_CHATTERER_METER").ok().filter(|m| ["smooth", "stepped", "off"].contains(&m.as_str())).unwrap_or_else(|| "stepped".into())
}

fn create(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    let url = WebviewUrl::App(format!("overlay.html?meter={}", meter_mode()).into());
    let window = WebviewWindowBuilder::new(app, label, url)
        .title("EVE Chatterer overlay")
        .inner_size(WIN_W, WIN_H)
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

fn place(app: &AppHandle, window: &WebviewWindow, monitor: &Rect, region: &Rect) {
    let scale = scale_for(app, monitor);
    let w = (WIN_W * scale).round() as i32;
    let h = (WIN_H * scale).round() as i32;
    let x = (region.left + (region.width() - w) / 2).clamp(monitor.left, (monitor.right - w).max(monitor.left));
    let y = region.top.clamp(monitor.top, (monitor.bottom - h).max(monitor.top));
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[cfg(windows)]
mod platform {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tauri::WebviewWindow;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    fn hwnd(window: &WebviewWindow) -> Option<HWND> {
        let handle = window.window_handle().ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(w) => Some(HWND(w.hwnd.get() as *mut _)),
            _ => None,
        }
    }

    /// The window must never take focus or appear in Alt-Tab. Click-through
    /// (`WS_EX_TRANSPARENT`) is already set by `set_ignore_cursor_events`.
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
}

#[cfg(not(windows))]
mod platform {
    use tauri::WebviewWindow;

    pub fn never_activate(_: &WebviewWindow) {}

    pub fn show_without_activating(window: &WebviewWindow) {
        let _ = window.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_valid_and_distinct_for_negative_monitor_origins() {
        let left = Rect { left: -1920, top: 0, right: 0, bottom: 1080 };
        let right = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
        let above = Rect { left: 0, top: -1080, right: 1920, bottom: 0 };
        let labels = [label_for(&left), label_for(&right), label_for(&above)];
        for l in &labels {
            assert!(l.chars().all(|c| c.is_ascii_alphanumeric() || "-_/:".contains(c)), "{l}");
            assert!(l.starts_with("overlay-"), "{l}");
        }
        assert_eq!(labels.iter().collect::<std::collections::HashSet<_>>().len(), 3, "{labels:?}");
    }
}
