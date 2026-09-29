# Backlog

Open work only. Finished items are removed (git history has them); decisions and measurements live in DESIGN.md and FINDINGS.md.

## Code signing (owner decision 2026-09-29: Certum, after payday)
Releases are unsigned today (the README covers SmartScreen's "Run anyway"). The plan, chosen over SignPath (acceptance favors established projects), Azure Trusted Signing (~$120/yr) and the Microsoft Store (MSIX, its own project):
- **Certificate:** Certum "Open Source Code Signing" in the **cloud** (SimplySign), about €58/yr, issued to the owner personally; also usable for dev-prompt (open source only). Signing builds SmartScreen reputation on the certificate, so the warning fades over time and stays gone for new releases; it doesn't disappear on day one.
- **Runner:** SimplySign's login is interactive (phone one-time code), so the release job runs on the owner's PC as a self-hosted runner, **as the owner's own account**, started only for a release. Flow: owner logs into SimplySign, Claude starts the runner (`run.cmd`), merges dev into main, watches the release, stops the runner. A release merged with the runner off just waits queued. (Optional extra: a separate limited Windows account for the runner service; only if SimplySign's virtual card works from another account, which is unverified.)
- **Protection:** fork PR workflows require approval for all outside contributors (set 2026-09-29), because PR runs use the PR's workflow files and could otherwise target the runner by label. Never approve a PR touching `.github/`.
- **Build:** Tauri's `bundle.windows.signCommand` (signtool with the Certum cert) signs the app exe before bundling and the installer after, so the updater `.sig` is made from the signed installer. The job fails fast if the certificate isn't reachable (SimplySign logged out). Test with a draft release first. Only `release.yml` moves to the runner; CI stays on GitHub's machines.

## Microsoft Store (later, its own project)
MSIX package (Microsoft signs it: no SmartScreen warning at all). Needs `makeappx` in the release workflow, a third update mode ("store": the in-app updater stays off), package identity for notifications instead of the HKCU registration, the MSIX startup task instead of the Run entry, a privacy policy, and a Store certification review per release.

## Measurements still open
- Overlay frame-time impact on EVE (FINDINGS #9 only has GPU utilization): needs a present-level capture such as PresentMon, with both clients in a static scene so EVE's own load does not swamp the effect. Also whether an idle open overlay window costs the compositor anything.
- Idle RAM and CPU with one overlay window versus two.
- CPU while animating is about a quarter of one core; check what dominates (likely WebView2 rendering the shadows) and whether the shadows, the arrival animation or the Beacon pulse can be made cheaper.
- Overlays at non-100% DPI scaling and on mixed-DPI monitors (also in FINDINGS' untested list).
- OneDrive cloud-only placeholders: confirm the attribute check keeps the app from hydrating old files (also in FINDINGS' untested list).
- Verify `Create` events in `probe --no-poll` (open a new channel or log a character in).

## Overlays
- Optional frosted glass through Windows acrylic on the overlay window; check its GPU cost over EVE first.
- The reposition hotkey (Ctrl+Alt+O) is fixed; make it configurable.
- A saved position's vertical fraction is measured against the fixed `BOX_H`; alert stacks are placed from the same reference, so a box whose measured height differs lands a few pixels off. Harmless so far.

## Settings
- Owner to confirm the shipped defaults after living with them: the style per channel kind (private Beacon, Fleet/Corp/Alliance Panel), lifetimes, and rate caps (Local 6/min, public 4/min, Fleet/Corp/Alliance 6/min, folding).
- More annoyance controls: repeat suppression (same sender and text within N seconds), overlay lifetime and maximum stack, minimum priority. The layer structure already allows adding fields.
- Known public channels are only removed by hand (the Remove action, which refuses while the channel has its own settings). Add automatic cleanup: drop entries not seen live in N days that carry no settings of their own.

## Features
- Idle detection (`GetLastInputInfo`) to switch to notifications (and optionally sound) when the user is away. The General page already shows the placeholder option. The old app had an idle-detector module.
- Optional spoken alerts (Windows speech synthesis).
- A live mini-feed/ticker of matched lines in a screen corner.
- Opt-in "archive old logs" (off by default): move logs older than N days with no writes for a long time into an `Archive` subfolder; never delete, never touch the live set, skip anything that can't be opened. Housekeeping only (the live set makes cost independent of file count).

## Minor: unnecessary merge hold for Corp/Alliance across unrelated characters
`core/src/merge.rs`'s "shared channel" check keys only on the header `channel_id`, which for Corp and Alliance is the literal string `corp`/`alliance` regardless of *which* corp or alliance. Two characters in different corporations are treated as sharing a channel, so a corp alert pays the ~750 ms merge hold for a duplicate that can never arrive. Not a correctness bug. Fix: key "shared" on channel_id plus something that identifies the corp/alliance instance, if the log header exposes one (check a real header); otherwise leave it, since the cost is small and only Corp/Alliance are affected.

## Open questions
- Are chat log headers localized on non-English clients? Plan: read the header by position (channel id, name, listener, start time) with keys as a check; find a non-English sample.
- Tune the cross-character dedupe tolerance (start at ±2 s) against a busier hub capture.

## Not planned: Linux (research notes, unmeasured)
Windows only for now (CONTRIBUTING.md). Kept for reference if that changes:
- `paths.rs`'s Linux fallback (`$HOME/Documents`) is wrong for the real case: EVE has no native Linux client, so logs live inside a Wine/Proton prefix (e.g. `~/.local/share/Steam/steamapps/compatdata/<appid>/pfx/drive_c/users/steamuser/Documents/EVE/logs/Chatlogs`).
- Presence/focus: plausible on X11 via EWMH; no portable way on Wayland for an unprivileged app to query the focused window (wlroots compositors expose a protocol, GNOME/KDE largely don't).
- Overlay window (topmost, click-through, non-activating): well understood on X11 (override-redirect); Wayland deliberately restricts it. EVE under Proton commonly renders via XWayland, so an X11 approach might still reach the game window; unverified.
- Notifications (freedesktop D-Bus) and idle detection (logind) are solid.
- Before any real work: a Linux probe (window enumeration, focus, a topmost click-through window) on a real box with EVE under Proton, with the same rigor as the Windows probes in `tools/`.
