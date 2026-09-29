# Backlog

## Overlay measurements still open
- Frame-time impact on EVE (FINDINGS #9 only has GPU utilization): needs a present-level capture such as PresentMon, with both clients in a static scene so EVE's own load does not swamp the effect. Also whether an idle open window costs the compositor anything.
- Idle RAM and CPU with 1 window versus 2 (single-monitor users).
- CPU while animating is about a quarter of one core; check what dominates (likely WebView2 rendering the shadows) and whether the shadows, the arrival animation or the Beacon pulse can be made cheaper.

## Before the overlay milestone (done: see FINDINGS #8 and #9)
- Overlay behavior at non-100% DPI scaling and mixed-DPI monitors.
- Verify `Create` events in `probe --no-poll` (open a new channel or log a character in).
- Test OneDrive placeholder behavior safely (attribute check before opening; never hydrate files only to adopt them).
- Spike the interactive WinRT toast (buttons, deep link into a pilot's settings) from the NSIS-installed build; needs an app user model id.

## Overlay design (see DESIGN.md "Overlay styles")
- Owner to confirm the default style mapping and the sizes/lifetimes; adjust `docs/design/alert-styles.html` and `StyleMap` defaults if they change.
- Burst folding in the overlay manager: thresholds (for example more than N alerts in M seconds fold to a Strip stack), stack limit per monitor, ordering (Beacon on top).
- Bundle Barlow with the app (the mockup loads it from Google Fonts).
- Optional frosted glass through Windows acrylic on the overlay window; check its GPU cost over EVE first.

## Known channels never get pruned
`Pilot.channels` (core/src/pilots.rs) is a cumulative, append-only record — once a character is seen in a public channel it stays listed on that character's Settings page forever, even after the channel closes and ages out of the live-tracking set entirely. No UI exists to remove one. Fine for a handful of channels; will clutter a character's Settings page over months of joining trade/recruitment channels. Options: a "remove" action per channel row (only when it has no override set, so removing it never silently discards a configured rule), and/or auto-drop entries not seen live in N days that also carry no override.

## Settings
- More annoyance controls beyond the three shipped (mode, rate cap, sound): repeat suppression (same sender and text within N seconds), overlay lifetime and maximum stack, away behavior, minimum priority. The layer structure already allows adding fields.
- Settings UI: superseded by the finalized mockup (`docs/design/settings-screen.html`, see DESIGN.md "Settings screen") — a sidebar/tree (Characters > Defaults/each character, Audio, General, About), not a matrix, with a per-field deviation marker and revert control. Remaining work: wire it into `app/src/settings/Settings.svelte` and add the settings load/save Tauri commands.
- Where the settings and pilot registry files live (proposal: `%APPDATA%\eve-chatterer\`), and migrating older files as fields are added.
- Confirm the built-in kind defaults with the owner (private = everything as a Beacon; Local/Alliance capped at 6 per minute, public at 4, folding).
- The overlay manager must honor `Outcome::Limited(Fold)` by incrementing the count badge of the alert that is already showing.

## Overlay reposition (see DESIGN.md "Overlay reposition & resize")
- The reposition hotkey (Ctrl+Alt+O) is fixed; make it configurable.
- A saved position's vertical fraction is measured against the fixed `BOX_H`; alert stacks are placed from the same reference, so a box whose measured height differs lands a few pixels off. Harmless so far.

## Linux support (unresearched, general knowledge only — not measured like the Windows findings)
- `paths.rs`'s Linux fallback (`$HOME/Documents`) is wrong for the real case: EVE has no native Linux client, so logs live inside a Wine/Proton prefix (e.g. `~/.local/share/Steam/steamapps/compatdata/<appid>/pfx/drive_c/users/steamuser/Documents/EVE/logs/Chatlogs`). Needs a real fix (search known compatdata paths, or ask the user) before Linux support means anything.
- Presence/focus tracking: plausible on X11 via EWMH properties (same idea as the Windows implementation); no standard, portable way on Wayland for an unprivileged app to query the focused window — wlroots compositors (Sway, Hyprland) expose a protocol for it, GNOME/KDE largely don't. Directly affects focus-based suppression.
- The overlay window (topmost, click-through, non-activating, exact placement): well-understood on X11 (override-redirect, what most Linux overlay tools use); Wayland compositors deliberately restrict this for arbitrary apps. Biggest risk to the app's core feature on Linux. Mitigating factor: EVE under Proton commonly renders via XWayland even in a Wayland session, so an X11-based approach might still reach the game window in practice — unverified.
- Toasts (freedesktop D-Bus notification spec) and idle detection (systemd-logind D-Bus) are solid and arguably easier than Windows.
- Before spending real engineering time: build a Linux probe (window enumeration, focus, a topmost/click-through window) on an actual box with EVE under Proton, the same rigor as the Windows probes in `tools/`, rather than trusting the analysis above.

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

## Minor: unnecessary merge hold for Corp/Alliance across unrelated characters
`core/src/merge.rs`'s "shared channel" check keys only on the header `channel_id`, which for Corp and Alliance is the literal string `corp`/`alliance` regardless of *which* corp or alliance. Two characters in different corporations both get treated as sharing a channel, so a corp alert pays the ~750ms merge hold for a duplicate that can never arrive. Found live-testing (2026-09-27): Jarna and Psianna are in different corps, Corp alerts still worked correctly, just delayed. Not a correctness bug (real corp-mates still merge fine; unrelated corps just never happen to match sender+text+timestamp). Fix: key "shared" on channel_id plus something that actually identifies the corp/alliance instance, if the log header exposes one (check a real header) — otherwise leave as is, since the cost is small and only Corp/Alliance are affected (Fleet ids and Local system are already unique per instance).

## Known nuisances to handle in code
- Explorer shell windows steal foreground events (see FINDINGS #4).
- `Task Switching` can be the last hook event; always confirm with `GetForegroundWindow()`.
- Partial trailing lines while EVE is mid-write; file shrinking/truncation; deleted files.
