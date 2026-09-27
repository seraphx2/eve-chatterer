# EVE Chatterer

A modern EVE Online chat-log notifier: Rust core + Tauri v2 + Svelte, in-game WebView2 overlays, per-pilot settings. Read `docs/DESIGN.md` (decisions), `docs/FINDINGS.md` (measured facts, do not re-derive), and `docs/BACKLOG.md` before changing behavior.

## Rules
- Poll the live log set (newest file per character id + channel, from filenames). Do not rely on `notify`/directory events for growth of live logs, and do not trust directory last-write times.
- Decode logs as UTF-16LE explicitly; a BOM precedes every line; consume only complete lines; all timestamps are UTC.
- Never inject into the EVE client, never read its process command line, never move or delete the user's logs by default.
- Alert windows are topmost, click-through and non-activating; they must never take focus.
- Resolve the Documents folder with the known-folder API (it may be under OneDrive).
- Key pilots by character id; names never change in EVE and are a stable secondary key.
- Update `docs/FINDINGS.md` when a measurement changes and add to its corrections log when something previously believed turns out wrong.

## Layout
- `src/bin/probe.rs` log-folder watcher/tailer probe (`--no-poll` to test events alone)
- `src/bin/synth.rs` synthetic EVE-style writer + watcher for event-behavior tests
- `src/bin/focus.rs` foreground-window/EVE-client state probe (writes `focus-log.txt`)
- `src/bin/overlay.rs` overlay feasibility probe (Ctrl+Alt+1/2/3, writes `overlay-log.txt`)
- `src/winutil.rs` shared Win32 helpers

Run a probe: `cargo run --bin <name>` from this directory (Windows only for focus/overlay).
