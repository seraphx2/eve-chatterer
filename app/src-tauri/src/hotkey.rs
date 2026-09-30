//! The global hotkey that toggles overlay reposition mode. Configurable on
//! the General page (settings.json `general.repositionHotkey`); the page's
//! recorder checks a combination against common Windows and app shortcuts
//! first (app/src/settings/hotkeys.ts), and this registers it with Windows.

use std::str::FromStr;
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

static CURRENT: Mutex<Option<Shortcut>> = Mutex::new(None);

pub fn parse(accel: &str) -> Result<Shortcut, String> {
    Shortcut::from_str(accel.trim()).map_err(|_| format!("\"{accel}\" isn't a key combination Windows understands."))
}

/// Whether a shortcut event is the reposition hotkey.
pub fn is_reposition(s: &Shortcut) -> bool {
    CURRENT.lock().unwrap().as_ref().is_some_and(|c| c == s)
}

/// At startup: the saved hotkey, or the default if it can't be registered
/// (another app holds it, or the saved text is broken). Returns a problem to
/// tell the user about, if any.
pub fn register_at_startup(app: &AppHandle, saved: &str) -> Option<String> {
    let default = eve_chatterer_core::settings::DEFAULT_REPOSITION_HOTKEY;
    let first = parse(saved).and_then(|s| app.global_shortcut().register(s).map(|_| s).map_err(|e| e.to_string()));
    match first {
        Ok(s) => {
            *CURRENT.lock().unwrap() = Some(s);
            None
        }
        Err(e) => {
            eprintln!("could not register the reposition hotkey {saved}: {e}");
            let fallback = (saved != default).then(|| parse(default).ok()).flatten().filter(|s| app.global_shortcut().register(*s).is_ok());
            *CURRENT.lock().unwrap() = fallback;
            Some(match fallback {
                Some(_) => format!("{saved} couldn't be used (another app may have it), so the reposition hotkey is {default} for now. Change it in Settings > General."),
                None => format!("The reposition hotkey {saved} couldn't be set up (another app may have it). Pick another in Settings > General."),
            })
        }
    }
}

/// Switches to a new hotkey. The new one is registered before the old one is
/// released, so if Windows refuses it (another app holds it) nothing changes.
pub fn change(app: &AppHandle, accel: &str) -> Result<(), String> {
    let new = parse(accel)?;
    let old = *CURRENT.lock().unwrap();
    if old == Some(new) {
        return Ok(());
    }
    app.global_shortcut()
        .register(new)
        .map_err(|_| format!("{accel} is already used by another app or by Windows. Pick another combination."))?;
    if let Some(old) = old {
        let _ = app.global_shortcut().unregister(old);
    }
    *CURRENT.lock().unwrap() = Some(new);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::{Code, Modifiers};

    #[test]
    fn the_recorders_accelerators_parse() {
        let o = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyO);
        assert_eq!(parse("Ctrl+Alt+O").unwrap(), o, "the default");
        assert_eq!(parse(" Ctrl+Alt+O ").unwrap(), o);
        assert!(parse("Ctrl+Shift+F9").is_ok());
        assert!(parse("Alt+Super+Digit3").is_ok());
        // What the recorder sends for digits and named keys (KeyboardEvent.code, "Key"/"Digit" stripped).
        for accel in ["Ctrl+Alt+1", "Ctrl+Alt+Comma", "Ctrl+Alt+ArrowUp", "Ctrl+Shift+Space", "Ctrl+Alt+F5", "Ctrl+Alt+Minus", "Shift+Super+Numpad5"] {
            assert!(parse(accel).is_ok(), "{accel}");
        }
        assert!(parse("").is_err());
        assert!(parse("Ctrl+Nope").is_err());
    }
}
