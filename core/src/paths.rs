//! Where EVE keeps its chat logs.

use std::path::PathBuf;

/// The user's Documents folder as Windows resolves it. This follows OneDrive
/// redirection, unlike guessing `%USERPROFILE%\Documents`.
#[cfg(windows)]
pub fn documents_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

#[cfg(not(windows))]
pub fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents"))
}

/// Where EVE writes its chat logs, `<Documents>\EVE\logs\Chatlogs`, whether
/// or not it exists yet (it appears once "Log chat to file" is on in EVE).
pub fn chatlogs_path() -> Option<PathBuf> {
    Some(documents_dir()?.join("EVE").join("logs").join("Chatlogs"))
}

/// `chatlogs_path()`, if it exists.
pub fn chatlogs_dir() -> Option<PathBuf> {
    chatlogs_path().filter(|p| p.is_dir())
}
