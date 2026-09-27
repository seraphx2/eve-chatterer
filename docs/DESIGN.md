# EVE Chatterer: design decisions

Status: pre-implementation. Feasibility is proven by the probes in `src/bin/` (see FINDINGS.md). Decisions below were agreed with the project owner during design; the reasoning is kept so they are not re-litigated by accident.

## Goal

A modern replacement for the 9-year-old EveChatNotifier: watch EVE Online chat logs and alert the player about lines they care about, with in-game overlays instead of a bland tray toast, per-character (pilot) settings, and a very low idle footprint.

Non-goals / hard rules:
- **No pilot-arrival or Local join/leave alerts.** Those are not in the logs. Only chat lines.
- **No injection into the EVE client** and no reading of the client's process command line (the launcher may pass login tokens there). Window title, exe path and window state are enough.
- **Never move, rename or delete the user's logs by default.** Old-log housekeeping is an opt-in feature (see BACKLOG.md).
- Alert windows must never take focus or intercept input.

## Stack

Tauri v2 + Rust core + Svelte, rich WebView2 overlays (owner's decision: "richer option"). Reference app for tray/hotkey/autostart/updater/installer patterns: `D:\git\dev-prompt` (its overlay uses `focus: true`; ours must be non-activating).

Memory rule: the Rust core is the always-on part (tens of MB). WebView2 windows (settings, overlays) are created on demand and destroyed after ~45 s idle, sharing one WebView2 environment. All state lives in Rust so webviews can be destroyed freely. WebView2 baseline is the real cost, not alert volume; dev-prompt measured about 300 MB working set for a full app. A minimal-overlay measurement is still to be done (BACKLOG).

## Architecture

```
core/   (Rust lib, no UI, testable headless against recorded logs)
  logfmt     header + line parsing, BOM-per-line, UTF-16LE tailer (complete lines only)
  liveset    filename parsing, newest file per (character id, channel), supersede, 500 ms poll,
             discovery via Create events + slow names-only rescan, OneDrive placeholder guard
  pilots     registry keyed by character id (see below)
  rules      own-name mention, keywords, regex, per-channel/sender allow+ignore, MOTD sender filter
  dedupe     cross-character duplicate suppression, ~+-2 s tolerance
  coalesce   per-channel rate limit and stacking
  presence   focus hook + GetForegroundWindow truth, client registry, per-client state
app/    (Tauri v2 + Svelte) tray, settings (on demand), overlay manager, config in %APPDATA%
tools/  today's probes kept as dev tools (probe, synth, focus, overlay)
```

Data flow: `pollers -> tailer -> parser -> rules -> dedupe -> router -> overlay manager -> webview`. The router consults presence to decide who is on screen and where.

Milestones: (1) headless core + CLI that prints alerts from real logs, with replayable fixtures; (2) overlays; (3) settings UI + rule editor; (4) sound/voice, autostart, updater, installer.

Presence and routing (implemented): `core/src/presence.rs` samples the desktop by **polling** (`GetForegroundWindow` plus a window enumeration, about every 250 ms) into a `Snapshot` of client states (focused, minimized, cloaked, visible, bounds, monitor, last known monitor) plus idle time and whether a toast would show. The WinEvent hook is not used: the router only needs the truth at the moment an alert arrives, and the hook would only wake us sooner (kept as a possible latency optimization). A `FocusTracker` ignores explorer shell surfaces (alt-tab switcher, taskbar, desktop switcher) for up to 1.5 s so they do not read as "left EVE". `core/src/router.rs` is pure logic over an `Alert` and a `Snapshot`: per target it applies suppression (focused only by default; visible; none), forces "away" (idle 5 min) to skip suppression and prefer a toast, then picks deliveries per the table above, with the overlay anchor chosen from whether the client covers its monitor. `Engine::observe_clients` marks known pilots live from their windows and reports `ChatLoggingOff` once when a client has no log after 90 s. A client at the login screen (title "EVE", no character) is not a client yet.

Milestone 1 status: workspace is `core/` (library), `cli/` (`chatter`), `tools/` (probes). Done and tested: time and log-format parsing (UTF-16LE, per-line BOM, partial lines, truncation, positional header fallback), live-set tracker (filename parsing, newest per character+channel, supersede, OneDrive placeholder guard, poll), pilot registry (id-keyed, silent historic vs live announcement, JSON persistence), rules (own name, keywords, regex, ignore/always lists, MOTD sender filter, per-pilot overrides), cross-character merge, and the engine. Also done: presence (Windows sampler), the router, and `observe_clients`, with the CLI printing the routing decision for each alert (`--suppress`, `--delivery`). Remaining for the milestone: a live end-to-end check with two characters, and replayable fixtures. Pilot "live" in the core means it produced a line or its session is under 2 minutes old; presence (a running client window) should also call `Engine::mark_live`.

## Finding the live log files

- Poll, do not rely on directory events (FINDINGS #1). The polled set is the **newest file per (character id, channel)**, derived from filenames only. Never trust directory last-write times.
- Directory `Create` events (believed reliable, untested) plus a slow names-only rescan discover new sessions.
- Narrow the polled set further to characters with a running client (window title) so multiboxers with many alts stay cheap.
- Documents folder must come from the Windows known-folder API (the owner's is under OneDrive). Never guess a path.
- Cloud-only OneDrive placeholder files must not be opened just to adopt them (check the file attributes first).

## Pilots (characters)

- A pilot is an EVE character. Config is keyed by **character id**. Names cannot change in EVE, so the name is also a stable, unique join key.
- A new pilot is detected from a live signal: a new client window "EVE - <Name>" for an unknown character, then the name-to-id pair is confirmed from that character's chat log (header `Listener` = name, filename = id), usually seconds after login.
- On confirmation: create a mostly-empty config that inherits global defaults (only overrides are stored), show a native toast (interactive, "Configure" opens straight to that pilot's settings), badge the tray, and mark the pilot New in settings.
- First launch must not toast for old alts found in historical logs. Register silently or show one "found N characters" step; list never-seen-live characters as "seen in logs".
- If a client window exists but no chat log appears, toast that chat logging looks off ("log chat to file" in EVE settings).
- Per-pilot settings: sound, overlay anchor mode, rules, delivery mode, accent/colors.

## When to alert (suppression)

Default: alert for everything except the client the user is currently focused on (that pilot's matches are suppressed; other pilots' still alert). If focus is on a non-EVE window, everything goes through. Options: "allow all through", and an advanced "also suppress any client visible on screen".

"Visible" for a client means all of: not minimized (`IsIconic`), not cloaked (`DWMWA_CLOAKED == 0`), `IsWindowVisible`. `IsWindowVisible` alone is true for minimized windows and for windows on other virtual desktops.

## How to alert (delivery)

Overlays for in-game events, native Windows notifications for app-level events (new pilot, update, missing log folder, logging off). For chat alerts, switch by context:

| Situation | Delivery |
|---|---|
| A client is focused (its own matches suppressed) | Overlay for other pilots |
| No client focused, some client visible | Overlay |
| No client visible (minimized, other desktop, other fullscreen app) | Native toast with a "switch to <pilot>" action |
| User idle for N minutes | Toast (persists in Action Center), optional sound |
| Toast cannot display (quiet time, DND) | Fall back to overlay; sound is a separate option |

Configurable per rule and pilot: overlay / toast / both / sound only. New overlays appear on the virtual desktop the user is currently on. Toasts and overlays are both coalesced (one per channel or pilot, updated in place).

## Overlay anchoring

Overlays are separate topmost, click-through, non-activating windows in screen coordinates (not injected into the game). Per-pilot anchor mode:
- **Monitor**: default when the client covers its monitor (Fixed Window, Fullscreen).
- **Follow client window**: default for Windowed mode; tracks the client's bounds, hides when it is minimized or cloaked.
- **Fallback**: no visible client window (Fullscreen hidden/minimized): use that pilot's last known monitor.
- Automatic choice from whether the client window fills its monitor; per-pilot override.

## Rules UX

A few first-class rules (name mention, keywords, per-channel) with regex behind an "advanced" toggle.

Cross-character duplicates (implemented in `core/src/merge.rs`): the same (channel, sender, text) seen by different listeners within ~2 s is merged into **one line with a `seen_by` list**. Rules are then evaluated per listener (own-name is per pilot), so an `Alert` carries `targets` (pilots whose rules matched, each with its reason) and `seen_by` (every character whose log had the line). The router, which knows about windows, applies suppression per target using presence, and can use `seen_by` to know which screens already showed the line. A first copy is held ~750 ms, only when another character is following the same channel id, so the second copy can join; a lone copy is emitted after the hold. Local is a separate channel per solar system but all share the channel id `local`, so two characters in different systems also pay the 750 ms hold (accepted; tunable).

## Log format compatibility

Parse defensively: explicit UTF-16LE decoding, a BOM before every line, complete-line consumption only, header read by position with keys as a check (localized headers are an open question, see BACKLOG), UTC timestamps everywhere, sender/text split on the first `>` after the timestamp. The old app offered a configurable regex "in case the log format changes"; keep an equivalent escape hatch.

## Prior art

MyUncleSam/EveChatNotifier (C#, MIT): polls file sizes on a timer, rescans the folder for new files, filters by last-write time (default 8 h), and offered a "move old logs" mode purely to keep the polled set small. Useful edge cases: MOTD lines from the `EVE System` sender at login (its text-based filter broke on localization), multi-client duplicate notifications (crude 1 per second throttle), ignore-own-messages, always/ignore lists per channel and pilot. Its weaknesses: decodes UTF-8 after a mid-file seek (works only because of the per-line BOM), no partial-line handling, no truncation handling, no live-set logic. py-eve-chat-mon independently reports the same event problem and polls.
