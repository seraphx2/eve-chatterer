# Backlog

Open work only. Finished items are removed (git history has them); decisions and measurements live in DESIGN.md and FINDINGS.md.

## Code signing (owner decision 2026-09-29: Certum, after payday)

Releases are unsigned today (the README covers SmartScreen's "Run anyway"). The plan, chosen over SignPath (acceptance favors established projects), Azure Trusted Signing (~$120/yr) and the Microsoft Store (MSIX, its own project):

- **Certificate:** Certum "Open Source Code Signing" in the **cloud** (SimplySign), about €58/yr, issued to the owner personally; also usable for dev-prompt (open source only). Signing builds SmartScreen reputation on the certificate, so the warning fades over time and stays gone for new releases; it doesn't disappear on day one.
- **Runner:** SimplySign's login is interactive (phone one-time code), so the release job runs on the owner's PC as a self-hosted runner, **as the owner's own account**, started only for a release. Flow: owner logs into SimplySign, Claude starts the runner (`run.cmd`), merges dev into main, watches the release, stops the runner. A release merged with the runner off just waits queued. (Optional extra: a separate limited Windows account for the runner service; only if SimplySign's virtual card works from another account, which is unverified.)
- **Protection:** fork PR workflows require approval for all outside contributors (set 2026-09-29), because PR runs use the PR's workflow files and could otherwise target the runner by label. Never approve a PR touching `.github/`.
- **Build:** Tauri's `bundle.windows.signCommand` (signtool with the Certum cert) signs the app exe before bundling and the installer after, so the updater `.sig` is made from the signed installer. The job fails fast if the certificate isn't reachable (SimplySign logged out). Test with a draft release first. Only `release.yml` moves to the runner; CI stays on GitHub's machines.

## Localization: translating the app (owner request 2026-09-30)

- **Languages:** EVE's own: English, German, French, Russian, Japanese, Chinese (Simplified), Korean, Spanish. After English, Russian, German and Chinese matter most by player numbers.
- **Two places hold text, both need it:** the Svelte UI (Settings, overlay labels such as "Mentioned you") and the Rust side (notification text and buttons, `runner::reason_text`, tray menu, update and chat-logging messages).
- **Approach:** extract every hard-coded string into translation keys as one piece of work (a few hundred, mostly Settings). Frontend via a build-time-checked library (Paraglide suits Svelte); Rust reads the same translation files so both sides stay in step. Language follows Windows' display language, with an override in Settings > General.
- **Translations:** Claude drafts every language first (decent, not native; EVE jargon needs care), then a free open-source platform (Weblate or Crowdin) lets players correct them without touching code.
- **Watch for:** longer strings (German) wrapping or cropping in Settings rows and overlays; Japanese/Chinese/Korean need system-font fallback since Barlow lacks those characters; plurals ("3 lines") and dates per language.
- **Plan:** plumbing and extraction once, then ship English plus drafted Russian, German and Chinese; the rest follow as files.
- Separate from reading non-English *logs* (Open questions below), which needs a real sample first.

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

## Away mode and webhooks: Discord and Slack (owner request 2026-09-29; build as one piece)

**Why away matters:** today the app assumes you're watching whenever EVE is on screen. Stepped away (AFK mining, ratting, docked), two things go wrong: the focused character's alerts are suppressed entirely ("it can see its own chat"), and other characters' overlays vanish after seconds with no history. So a mention while you're away can be lost completely.

- **Away detection:** no mouse or keyboard input for N minutes (`GetLastInputInfo`; the General page already shows the placeholder "Treat input idle for 5 minutes as away") means away. While away: nothing is suppressed for being focused, alerts go to Windows notifications instead of overlays (they wait in Action Center with Switch to), and sound can play. Any input ends it. `router::route` already takes an `away` input; nothing sets it yet (always false).
- **Webhooks, Discord and Slack** (a couple of other EVE tools do Discord): while away, also post the alert to a Discord or Slack channel, so the phone app pings you. Both are "incoming webhooks": a secret URL taking a small JSON message. Only the message format (Discord embeds, Slack attachments/blocks) and rate limits differ.
  - **Webhook manager:** one place (its own "Webhooks" Settings page) where you add webhooks by **name** plus URL, with a Send test button. The type is detected from the URL (`discord.com/api/webhooks/...` or `hooks.slack.com/services/...`); anything else is refused with a clear message. The names populate a dropdown used everywhere else, so one webhook is reused across characters and channels, and the dropdown doesn't care which service a name is.
  - **Choosing where alerts go:** a "Webhook" dropdown (None or a named webhook) as a normal layered field: set it on Defaults, override per character, and per channel, like Style or Sound. Mentions are the obvious default to send; other channels opt in.
  - **When:** only while away by default; possibly an "always" option.
  - **Look:** shaped like our alerts on both services (Discord embed, Slack attachment with a colored edge): character name and accent color, "Sender in Channel", the message, the reason. One formatting function per service. Busy channels fold into one message with a count, which also keeps under the rate limits (Slack allows about 1 message/second per webhook).
  - **The URL is a secret** (anyone with it can post): stored only in local settings, masked in the UI, never logged, never in the repo.
  - **Later, same shape:** Microsoft Teams, and a generic JSON option for people's own tools.
  - **Privacy:** this sends other players' chat off the PC. Clearly labeled opt-in; the README's "only talks to GitHub" statement must be updated. Some corps/alliances forbid relaying their chat outside the game, so the UI should make the per-channel choice deliberate.
  - **Reliability:** retry on failure, respect both services' 429 rate-limit responses, never drop a mention silently.

## Features

- Optional spoken alerts (Windows speech synthesis).
- A live mini-feed/ticker of matched lines in a screen corner.
- Opt-in "archive old logs" (off by default): move logs older than N days with no writes for a long time into an `Archive` subfolder; never delete, never touch the live set, skip anything that can't be opened. Housekeeping only (the live set makes cost independent of file count).

## Open questions

- Are chat log headers localized on non-English clients? Plan: read the header by position (channel id, name, listener, start time) with keys as a check; find a non-English sample.
- Confirm the Alliance log starts with `EVE System > Channel changed to Alliance : <name>` like Corp does (FINDINGS #3); the corp/alliance display and the merge check assume it.
- Tune the cross-character dedupe tolerance (start at ±2 s) against a busier hub capture.

## Not planned: Linux (research notes, unmeasured)

Windows only for now (CONTRIBUTING.md). Kept for reference if that changes:

- `paths.rs`'s Linux fallback (`$HOME/Documents`) is wrong for the real case: EVE has no native Linux client, so logs live inside a Wine/Proton prefix (e.g. `~/.local/share/Steam/steamapps/compatdata/<appid>/pfx/drive_c/users/steamuser/Documents/EVE/logs/Chatlogs`).
- Presence/focus: plausible on X11 via EWMH; no portable way on Wayland for an unprivileged app to query the focused window (wlroots compositors expose a protocol, GNOME/KDE largely don't).
- Overlay window (topmost, click-through, non-activating): well understood on X11 (override-redirect); Wayland deliberately restricts it. EVE under Proton commonly renders via XWayland, so an X11 approach might still reach the game window; unverified.
- Notifications (freedesktop D-Bus) and idle detection (logind) are solid.
- Before any real work: a Linux probe (window enumeration, focus, a topmost click-through window) on a real box with EVE under Proton, with the same rigor as the Windows probes in `tools/`.
