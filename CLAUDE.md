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

## Layout (Cargo workspace)
- `core/` library, no UI: `logfmt` (parsing + complete-line reads), `liveset` (live files + poll), `tailer`, `pilots`, `rules`, `merge` (cross-character dedupe), `engine` (drives it all; `tick()` every ~500 ms), `paths` (known-folder lookup), `time`
- `cli/` the `chatter` binary: `cargo run -p eve-chatterer-cli -- --verbose` follows the real logs read-only and prints alerts
- `tools/` measurement probes: `probe` (log watcher, `--no-poll`), `synth` (synthetic EVE-style writer/watcher), `focus` (foreground/client state, writes `focus-log.txt`), `overlay` (overlay feasibility, Ctrl+Alt+1/2/3, writes `overlay-log.txt`), plus shared Win32 helpers in `tools/src/winutil.rs`

Tests: `cargo test -p eve-chatterer-core`. Run a probe: `cargo run -p eve-chatterer-tools --bin <name>` (Windows only for focus/overlay).
