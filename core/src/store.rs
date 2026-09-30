//! Loading and saving the app's JSON files (settings, pilots) so that a
//! crash, a power cut, a sync conflict or a file from a newer version can
//! never cost the user their data.
//!
//! Saving flushes a temporary file to disk and renames it over the old one,
//! after copying the old one to `<name>.bak`. Opening a file that can't be
//! read never discards it: it's renamed to `<name>.corrupt-<unix secs>.json`,
//! the backup is tried, and the caller gets a problem to tell the user about.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// What `open` found.
#[derive(Debug)]
pub struct Opened<T> {
    /// `None` when there is nothing usable: a first run, or an unreadable file
    /// with no readable backup. The caller starts from its defaults.
    pub value: Option<T>,
    /// Something the user should be told (the file was unreadable).
    pub problem: Option<String>,
}

/// Reads `path` strictly and changes nothing: `Ok(None)` when it doesn't
/// exist, an error when it can't be read or parsed. For read-only callers
/// such as the CLI.
pub fn load<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads `path` for the app. An unreadable file is moved aside (never
/// overwritten later) and the backup is used if it can be read.
pub fn open<T: DeserializeOwned>(path: &Path) -> Opened<T> {
    let err = match load(path) {
        Ok(value) => return Opened { value, problem: None },
        Err(e) => e,
    };
    let name = file_name(path);
    let kept = match quarantine(path) {
        Ok(p) => format!("It was kept as {}.", file_name(&p)),
        Err(e) => format!("It couldn't be moved aside ({e}), so it stays as it is."),
    };
    match load::<T>(&backup_path(path)) {
        Ok(Some(value)) => Opened {
            value: Some(value),
            problem: Some(format!("{name} couldn't be read ({err}), so the previous copy was restored. {kept}")),
        },
        _ => Opened { value: None, problem: Some(format!("{name} couldn't be read ({err}), so it starts from the defaults. {kept}")) },
    }
}

/// Writes `value` to `path` so that `path` always holds either the old or the
/// new contents, never a partial file, and the old contents stay in the backup.
pub fn save<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    if path.is_file() {
        let backup = backup_path(path);
        fs::copy(path, &backup)?;
        OpenOptions::new().write(true).open(&backup)?.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// `settings.json` -> `settings.json.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".bak");
    PathBuf::from(p)
}

/// Renames an unreadable file to `<stem>.corrupt-<unix secs>.json` beside it.
fn quarantine(path: &Path) -> io::Result<PathBuf> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let target = path.with_file_name(format!("{stem}.corrupt-{secs}.json"));
    fs::rename(path, &target)?;
    Ok(target)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    type Doc = BTreeMap<String, u32>;

    fn doc(n: u32) -> Doc {
        BTreeMap::from([("n".to_string(), n)])
    }

    #[test]
    fn a_missing_file_is_nothing_and_no_problem() {
        let dir = tempfile::tempdir().unwrap();
        let o = open::<Doc>(&dir.path().join("settings.json"));
        assert!(o.value.is_none() && o.problem.is_none());
    }

    #[test]
    fn saving_round_trips_and_keeps_the_previous_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.json");
        save(&path, &doc(1)).unwrap();
        assert!(!backup_path(&path).exists(), "nothing to back up on the first save");
        save(&path, &doc(2)).unwrap();
        assert_eq!(load::<Doc>(&path).unwrap(), Some(doc(2)));
        assert_eq!(load::<Doc>(&backup_path(&path)).unwrap(), Some(doc(1)));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn an_unreadable_file_is_kept_aside_and_the_backup_restored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        save(&path, &doc(1)).unwrap();
        save(&path, &doc(2)).unwrap();
        fs::write(&path, "{ truncated").unwrap();

        let o = open::<Doc>(&path);
        assert_eq!(o.value, Some(doc(1)));
        let problem = o.problem.unwrap();
        assert!(problem.contains("previous copy was restored"), "{problem}");
        assert!(!path.exists(), "the bad file is moved, so a later save can't overwrite it");
        let kept: Vec<_> = fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.contains(".corrupt-")).collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(fs::read_to_string(dir.path().join(&kept[0])).unwrap(), "{ truncated");
    }

    #[test]
    fn an_unreadable_file_without_a_backup_starts_from_defaults_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pilots.json");
        fs::write(&path, "not json").unwrap();
        let o = open::<Doc>(&path);
        assert!(o.value.is_none());
        assert!(o.problem.unwrap().contains("starts from the defaults"));
        assert!(!path.exists());
    }

    #[test]
    fn strict_load_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "not json").unwrap();
        assert!(load::<Doc>(&path).is_err());
        assert!(path.exists());
    }
}
