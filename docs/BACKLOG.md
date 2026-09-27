# Backlog

## Before the overlay milestone
- Measure a minimal Tauri transparent/click-through/non-activating overlay: idle RAM with 1 vs 2 windows on 2 monitors (shared WebView2 environment), and CPU/GPU/frame-time impact while an alert animates over a running EVE client.
- Overlay behavior at non-100% DPI scaling and mixed-DPI monitors.
- Verify `Create` events in `probe --no-poll` (open a new channel or log a character in).
- Test OneDrive placeholder behavior safely (attribute check before opening; never hydrate files only to adopt them).
- Spike the interactive WinRT toast (buttons, deep link into a pilot's settings) from the NSIS-installed build; needs an app user model id.

## Open questions
- Are chat log headers localized on non-English clients? Plan: read the header by position (channel id, name, listener, start time) with keys as a check; find a non-English sample.
- Is the launcher-provided character selection ever visible without the log? (No: window title gives the name, the log gives the id.)
- Tune the cross-character dedupe tolerance (start at +-2 s) against a busier hub capture.

## Features parked for later
- **Opt-in "archive old logs"** (off by default): move logs older than N days that have had no writes for a long time into an `Archive` subfolder, never delete, never touch anything in the live set, skip anything that cannot be opened. Not needed for performance (the live set makes cost independent of file count); housekeeping only.
- Idle detection (`GetLastInputInfo`) to switch to persistent toast + sound when the user is away. The old app had an idle-detector module.
- Optional spoken alerts (Windows speech synthesis).
- Sound: a single playing sound at a time, click-to-dismiss (the old app's behavior) is a good baseline.
- ESI name-to-id lookup was considered for pairing a window with a character id and rejected for now (network call, unnecessary since the log gives the pair).
- Per-pilot accent colors and per-channel styles; live mini-feed/ticker of matched lines in a screen corner.
- "Chat logging looks off" detection and toast.

## Known nuisances to handle in code
- Explorer shell windows steal foreground events (see FINDINGS #4).
- `Task Switching` can be the last hook event; always confirm with `GetForegroundWindow()`.
- Partial trailing lines while EVE is mid-write; file shrinking/truncation; deleted files.
