//! Follows one growing log file by polling.

use crate::logfmt::{self, ChatLine, Header};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// A session that just began: replay from the header so no early line is missed.
    Beginning,
    /// A session that already has history: begin after the last complete line.
    End,
}

#[derive(Debug)]
pub struct Tailer {
    path: PathBuf,
    offset: u64,
    header: Option<Header>,
}

impl Tailer {
    pub fn open(path: impl Into<PathBuf>, start: Start) -> io::Result<Tailer> {
        let path = path.into();
        // std opens with FILE_SHARE_READ | WRITE | DELETE on Windows, so EVE is never blocked.
        let mut f = File::open(&path)?;
        let header = logfmt::read_header(&mut f)?;
        let offset = match start {
            Start::Beginning => 0,
            Start::End => logfmt::last_line_boundary(&mut f)?,
        };
        Ok(Tailer { path, offset, header })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The header, once EVE has written it.
    pub fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    /// Reads the chat lines appended since the previous call. Opens the file
    /// through a real handle each time: directory metadata and directory events
    /// lag behind EVE's cached writes (docs/FINDINGS.md #1).
    pub fn poll(&mut self) -> io::Result<Vec<ChatLine>> {
        let mut f = File::open(&self.path)?;
        let read = logfmt::read_complete_lines(&mut f, self.offset)?;
        self.offset = read.offset;
        if read.truncated {
            self.header = None;
        }
        if self.header.is_none() {
            self.header = logfmt::read_header(&mut f)?;
        }
        Ok(read.lines.iter().filter_map(|l| logfmt::parse_line(l)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logfmt::testutil::*;
    use std::fs::OpenOptions;
    use std::io::Write;

    fn append(path: &Path, text: &str) {
        let mut f = OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(&text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    }

    fn new_log(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, file_bytes(body)).unwrap();
        p
    }

    #[test]
    fn adopting_at_end_skips_history_then_follows() {
        let dir = tempfile::tempdir().unwrap();
        let p = new_log(dir.path(), "Local_20260926_210356_1.txt", &(header("local", "Local", "Holden") + &line("2026.09.27 01:00:00", "A", "old")));
        let mut t = Tailer::open(&p, Start::End).unwrap();
        assert_eq!(t.header().unwrap().listener, "Holden");
        assert!(t.poll().unwrap().is_empty(), "history must not replay");
        append(&p, &line("2026.09.27 01:00:05", "B", "new"));
        let got = t.poll().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "new");
        assert!(t.poll().unwrap().is_empty());
    }

    #[test]
    fn a_new_session_replays_from_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let p = new_log(dir.path(), "Local_20260926_210356_1.txt", &(header("local", "Local", "Holden") + &line("2026.09.27 01:00:00", "A", "first")));
        let mut t = Tailer::open(&p, Start::Beginning).unwrap();
        let got = t.poll().unwrap();
        assert_eq!(got.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["first"]);
    }

    #[test]
    fn header_arrives_late_and_partial_lines_wait() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Local_20260926_210356_1.txt");
        std::fs::write(&p, [0xFF, 0xFE]).unwrap(); // just created, BOM only
        let mut t = Tailer::open(&p, Start::Beginning).unwrap();
        assert!(t.header().is_none());
        append(&p, &header("local", "Local", "Holden"));
        assert!(t.poll().unwrap().is_empty());
        assert_eq!(t.header().unwrap().listener, "Holden");
        let l = line("2026.09.27 01:00:00", "A", "hello");
        append(&p, &l[..l.len() - 4]); // no line ending yet
        assert!(t.poll().unwrap().is_empty());
        append(&p, &l[l.len() - 4..]);
        assert_eq!(t.poll().unwrap()[0].text, "hello");
    }

    #[test]
    fn deleted_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let p = new_log(dir.path(), "Local_20260926_210356_1.txt", &header("local", "Local", "Holden"));
        let mut t = Tailer::open(&p, Start::End).unwrap();
        std::fs::remove_file(&p).unwrap();
        assert_eq!(t.poll().unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
