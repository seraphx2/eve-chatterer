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
- `core/` library, no UI: `logfmt` (parsing + complete-line reads), `liveset` (live files + poll), `tailer`, `channel` (kind from the header id: Local, Corp, Alliance, Fleet, Private, Public), `pilots`, `prefs` (settings vocabulary), `settings` (layered global > kind > channel > pilot > pilot+kind/channel; `SettingsBook` caches resolution), `rules` (content rules + mode), `merge` (cross-character dedupe), `engine` (drives it all; `tick()` every ~500 ms, `observe_clients()` from presence), `presence` (client/focus snapshot; Windows `Sampler` polls at ~250 ms), `router` (suppression + delivery decisions, pure), `governor` (rate caps, stateful), `winapi` (Win32 helpers, Windows only), `paths` (known-folder lookup), `time`
- Settings rule: pilot settings override kind defaults; preferences take the most specific level, limits (rate caps) apply at every level.
- `cli/` the `chatter` binary: `cargo run -p eve-chatterer-cli -- --verbose` follows the real logs read-only and prints alerts
- `app/` the Tauri v2 + Svelte tray app (`src-tauri` Rust, `src/overlay` and `src/settings` Svelte). Dev: `npm install` then `npm run tauri dev` inside `app/` (Vite on port 1430). A self-contained build for measuring: `cargo build -p eve-chatterer-app --release --features tauri/custom-protocol` after `npm run build` in `app/`. Flags: `--selftest` (overlays, burst, exit at 90 s), `--soak` (continuous alerts, exit at ~60 s), `--toasttest` (five Windows notifications covering each length and the mention slot; from `app/`: `npm run tauri dev -- -- -- --toasttest`), `--soundtest` (walks the sound cooldown and mention rules with the built-in alert tone, printing what should be heard); env `EVE_CHATTERER_METER=smooth|stepped|off`, `EVE_CHATTERER_DIAG=1` (timing log in the temp folder).
- `tools/` measurement probes: `probe` (log watcher, `--no-poll`), `synth` (synthetic EVE-style writer/watcher), `focus` (foreground/client state, writes `focus-log.txt`), `overlay` (overlay feasibility, Ctrl+Alt+1/2/3, writes `overlay-log.txt`), plus shared Win32 helpers in `tools/src/winutil.rs`

- `tools/scripts/feed.ps1` synthetic two-character log feeder for live routing checks (writes only to a temp dir you give it): `pwsh tools/scripts/feed.ps1 -Dir <temp> -Seconds 150` alongside `chatter --dir <temp> --keyword chatterer-test`

Tests: `cargo test -p eve-chatterer-core`. Run a probe: `cargo run -p eve-chatterer-tools --bin <name>` (Windows only for focus/overlay).
