//! Optional timing diagnostics for measurement runs. Off unless the
//! environment variable `EVE_CHATTERER_DIAG` is set; then lines are appended
//! to `eve-chatterer-diag.log` in the temp folder.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Call once at startup so timestamps count from launch.
pub fn init() {
    START.get_or_init(Instant::now);
}

pub fn diag(msg: impl AsRef<str>) {
    if std::env::var_os("EVE_CHATTERER_DIAG").is_none() {
        return;
    }
    let ms = START.get_or_init(Instant::now).elapsed().as_millis();
    let path = std::env::temp_dir().join("eve-chatterer-diag.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "+{ms:>6}ms  {}", msg.as_ref());
    }
}
