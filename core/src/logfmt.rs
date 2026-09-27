//! EVE chat log format: header and line parsing, and reading complete lines
//! from a growing UTF-16LE file.
//!
//! Facts this relies on (see docs/FINDINGS.md #3): UTF-16LE, a BOM before every
//! line, CRLF endings, UTC timestamps `[ YYYY.MM.DD HH:MM:SS ] Sender > text`,
//! header order Channel ID / Channel Name / Listener / Session started.

use crate::time::Stamp;
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Header {
    pub channel_id: String,
    pub channel_name: String,
    pub listener: String,
    pub session_started: Option<Stamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub stamp: Stamp,
    pub sender: String,
    pub text: String,
}

fn clean(s: &str) -> &str {
    s.trim_matches(|c: char| c == '\u{feff}' || c.is_whitespace())
}

/// Parses the header block. Fields are matched by their English key when
/// present and by position otherwise (channel id, channel name, listener,
/// session start), so a localized client still works.
pub fn parse_header(text: &str) -> Option<Header> {
    let mut fields: Vec<(String, String)> = vec![];
    for raw in text.lines() {
        let line = clean(raw);
        if line.starts_with('[') {
            break; // first chat line: the header is over
        }
        if let Some((k, v)) = line.split_once(':') {
            fields.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let field = |key: &str, pos: usize| {
        fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .or_else(|| fields.get(pos))
            .map(|(_, v)| v.clone())
    };
    let listener = field("Listener", 2)?;
    if listener.is_empty() {
        return None;
    }
    Some(Header {
        channel_id: field("Channel ID", 0).unwrap_or_default(),
        channel_name: field("Channel Name", 1).unwrap_or_default(),
        listener,
        session_started: field("Session started", 3).and_then(|v| Stamp::parse_log(&v)),
    })
}

/// Parses `[ 2026.09.27 01:37:31 ] Sender > text`. Returns `None` for header
/// lines, blank lines and anything else that is not a chat line.
pub fn parse_line(raw: &str) -> Option<ChatLine> {
    let rest = clean(raw).strip_prefix('[')?;
    let (ts, after) = rest.split_once(']')?;
    let stamp = Stamp::parse_log(ts)?;
    let (sender, text) = after.split_once('>')?;
    Some(ChatLine { stamp, sender: sender.trim().to_string(), text: text.trim().to_string() })
}

pub fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// End index of the last complete line (just past a `\n` code unit) in a
/// buffer that starts on a UTF-16 code-unit boundary; 0 if there is none.
fn last_line_end(buf: &[u8]) -> usize {
    let mut end = 0;
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == 0x0A && buf[i + 1] == 0 {
            end = i + 2;
        }
        i += 2;
    }
    end
}

#[derive(Debug)]
pub struct Consumed {
    /// Complete lines appended since the previous offset (unparsed text).
    pub lines: Vec<String>,
    /// Offset to pass next time: just past the last complete line consumed.
    pub offset: u64,
    /// True if the file was shorter than the previous offset and we restarted at 0.
    pub truncated: bool,
}

/// Reads the complete lines between `offset` and the end of `src`. A partial
/// trailing line is left for the next call, so a half-written line is never
/// reported. `offset` must be even (or 0).
pub fn read_complete_lines<R: Read + Seek>(src: &mut R, offset: u64) -> io::Result<Consumed> {
    let len = src.seek(SeekFrom::End(0))?;
    let (offset, truncated) = if len < offset { (0, true) } else { (offset & !1, false) };
    if len <= offset {
        return Ok(Consumed { lines: vec![], offset, truncated });
    }
    src.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; (len - offset) as usize];
    src.read_exact(&mut buf)?;
    let end = last_line_end(&buf);
    if end == 0 {
        return Ok(Consumed { lines: vec![], offset, truncated });
    }
    let text = decode_utf16le(&buf[..end]);
    Ok(Consumed { lines: text.lines().map(str::to_string).collect(), offset: offset + end as u64, truncated })
}

/// Offset just past the last complete line of `src`, so adopting an existing
/// file never replays history or starts mid-line.
pub fn last_line_boundary<R: Read + Seek>(src: &mut R) -> io::Result<u64> {
    let len = src.seek(SeekFrom::End(0))?;
    let start = len.saturating_sub(8192) & !1;
    src.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; (len - start) as usize];
    src.read_exact(&mut buf)?;
    Ok(start + last_line_end(&buf) as u64)
}

/// Reads the header from the start of a log file (first 4 KiB).
pub fn read_header<R: Read + Seek>(src: &mut R) -> io::Result<Option<Header>> {
    src.seek(SeekFrom::Start(0))?;
    let mut buf = vec![0u8; 4096];
    let n = src.read(&mut buf)?;
    Ok(parse_header(&decode_utf16le(&buf[..n & !1])))
}

#[cfg(test)]
pub(crate) mod testutil {
    /// UTF-16LE with a BOM at the start.
    pub fn file_bytes(text: &str) -> Vec<u8> {
        let mut v = vec![0xFF, 0xFE];
        v.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        v
    }

    pub fn header(channel_id: &str, channel: &str, listener: &str) -> String {
        format!(
            "\r\n---------------------------------------------------------------\r\n  \
             Channel ID:      {channel_id}\r\n  Channel Name:    {channel}\r\n  Listener:        {listener}\r\n  \
             Session started: 2026.09.26 21:03:56\r\n---------------------------------------------------------------\r\n"
        )
    }

    /// One chat line the way EVE writes it: a BOM, then the text, then CRLF.
    pub fn line(stamp: &str, sender: &str, text: &str) -> String {
        format!("\u{feff}[ {stamp} ] {sender} > {text}\r\n")
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_a_real_shaped_header() {
        let text = format!("\u{feff}{}", header("local", "Local", "Jarna"));
        let h = parse_header(&text).unwrap();
        assert_eq!(h.channel_id, "local");
        assert_eq!(h.channel_name, "Local");
        assert_eq!(h.listener, "Jarna");
        assert_eq!(h.session_started, Stamp::parse_log("2026.09.26 21:03:56"));
    }

    #[test]
    fn header_falls_back_to_position_when_keys_are_localized() {
        let text = "\r\n---\r\n  Kanal-ID:   local\r\n  Kanalname:  Lokal\r\n  Zuhörer:    Jarna\r\n  Sitzung gestartet: 2026.09.26 21:03:56\r\n---\r\n";
        let h = parse_header(text).unwrap();
        assert_eq!((h.channel_name.as_str(), h.listener.as_str()), ("Lokal", "Jarna"));
        assert!(h.session_started.is_some());
    }

    #[test]
    fn header_stops_at_first_chat_line_and_needs_a_listener() {
        assert!(parse_header("[ 2026.09.27 01:37:31 ] a > b: c\r\n").is_none());
        assert!(parse_header("").is_none());
    }

    #[test]
    fn parses_chat_lines_with_bom_and_odd_text() {
        let l = parse_line(&line("2026.09.27 01:37:31", "Rilakss", "Предложение гиперсети: Zirnitra*").trim_end().to_string()).unwrap();
        assert_eq!(l.sender, "Rilakss");
        assert_eq!(l.text, "Предложение гиперсети: Zirnitra*");
        // A '>' or ']' inside the message must not confuse the split.
        let l = parse_line("[ 2026.09.27 01:37:31 ] Bob > a > b ] c").unwrap();
        assert_eq!((l.sender.as_str(), l.text.as_str()), ("Bob", "a > b ] c"));
        assert!(parse_line("  Channel ID:      local").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn reads_only_complete_lines_and_resumes() {
        let mut bytes = file_bytes(&(header("local", "Local", "Jarna") + &line("2026.09.27 01:00:00", "A", "one")));
        let mut c = Cursor::new(bytes.clone());
        let first = read_complete_lines(&mut c, 0).unwrap();
        assert_eq!(first.lines.iter().filter(|l| parse_line(l).is_some()).count(), 1);

        // EVE is mid-write: half of the next line, no newline yet.
        let partial = line("2026.09.27 01:00:05", "B", "two");
        let cut = partial.encode_utf16().count() - 3; // drop the tail incl. CRLF
        let half: Vec<u8> = partial.encode_utf16().take(cut).flat_map(|u| u.to_le_bytes()).collect();
        bytes.extend_from_slice(&half);
        let mut c = Cursor::new(bytes.clone());
        let mid = read_complete_lines(&mut c, first.offset).unwrap();
        assert!(mid.lines.is_empty());
        assert_eq!(mid.offset, first.offset, "partial line must not advance the offset");

        // The rest of the line arrives.
        let rest: Vec<u8> = partial.encode_utf16().skip(cut).flat_map(|u| u.to_le_bytes()).collect();
        bytes.extend_from_slice(&rest);
        let mut c = Cursor::new(bytes);
        let done = read_complete_lines(&mut c, mid.offset).unwrap();
        let parsed: Vec<_> = done.lines.iter().filter_map(|l| parse_line(l)).collect();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].text, "two");
    }

    #[test]
    fn truncation_restarts_from_the_top() {
        let bytes = file_bytes(&(header("local", "Local", "Jarna") + &line("2026.09.27 01:00:00", "A", "one")));
        let mut c = Cursor::new(bytes);
        let r = read_complete_lines(&mut c, 99_999).unwrap();
        assert!(r.truncated);
        assert!(!r.lines.is_empty());
    }

    #[test]
    fn last_line_boundary_skips_history() {
        let bytes = file_bytes(&(header("local", "Local", "Jarna") + &line("2026.09.27 01:00:00", "A", "one") + &line("2026.09.27 01:00:01", "B", "two")));
        let len = bytes.len() as u64;
        let mut c = Cursor::new(bytes);
        assert_eq!(last_line_boundary(&mut c).unwrap(), len);
        assert!(read_complete_lines(&mut c, len).unwrap().lines.is_empty());
    }

    #[test]
    fn reads_the_header_from_a_file() {
        let mut c = Cursor::new(file_bytes(&header("local", "Local", "Jarna")));
        assert_eq!(read_header(&mut c).unwrap().unwrap().listener, "Jarna");
    }
}
