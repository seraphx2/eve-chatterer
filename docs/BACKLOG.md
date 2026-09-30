# Backlog

Open work only. Finished items are removed (git history has them); decisions and measurements live in DESIGN.md and FINDINGS.md.

## Code signing (owner decision 2026-09-29: Certum, after payday)

Releases are unsigned today (the README covers SmartScreen's "Run anyway"). The plan, chosen over SignPath (acceptance favors established projects), Azure Trusted Signing (~$120/yr) and the Microsoft Store (MSIX, its own project):

- **Certificate:** Certum "Open Source Code Signing" in the **cloud** (SimplySign), about €58/yr, issued to the owner personally; also usable for dev-prompt (open source only). Signing builds SmartScreen reputation on the certificate, so the warning fades over time and stays gone for new releases; it doesn't disappear on day one.
- **Runner:** SimplySign's login is interactive (phone one-time code), so the release job runs on the owner's PC as a self-hosted runner, **as the owner's own account**, started only for a release. Flow: owner logs into SimplySign, Claude starts the runner (`run.cmd`), merges dev into main, watches the release, stops the runner. A release merged with the runner off just waits queued. (Optional extra: a separate limited Windows account for the runner service; only if SimplySign's virtual card works from another account, which is unverified.)
- **Protection:** fork PR workflows require approval for all outside contributors (set 2026-09-29), because PR runs use the PR's workflow files and could otherwise target the runner by label. Never approve a PR touching `.github/`.
- **Build:** Tauri's `bundle.windows.signCommand` (signtool with the Certum cert) signs the app exe before bundling and the installer after, so the updater `.sig` is made from the signed installer. The job fails fast if the certificate isn't reachable (SimplySign logged out). Test with a draft release first. Only `release.yml` moves to the runner; CI stays on GitHub's machines.

## Chat feed: a running history of matched lines in each client

Mockup: `docs/design/mini-feed.html`. Alerts pop up and vanish; the feed is what you glance at to catch up on what you missed.

- **Look:** a small panel in a free corner of the EVE client, in the overlays' steel glass but more transparent and never animated. Strip-style rows (character tag, channel, sender, message cut to one line), newest at the bottom, the last ~6 lines. Older lines fade, the age sits on the right, a colored tick says why the line matched (amber mention, cyan keyword, blue always-alert), and the newest line is washed in the character's color for its first minute.
- **Behavior:** owned by its client like the alerts: follows it, hides when it's minimized or on another desktop, click-through. Positioned per client with Ctrl+Alt+O.
- **Settings:** Chat feed on/off, and Show: this character only / all characters (all: every character's lines, colored by character). A layered setting at the character level, not per channel: Defaults sets it and each character's page can override it, with the usual pip and use-default revert. For example on with all characters for the main, off for docked alts.

## Localization: translating the app (owner request 2026-09-30)

- **Languages:** EVE's own: English, German, French, Russian, Japanese, Chinese (Simplified), Korean, Spanish. After English, Russian, German and Chinese matter most by player numbers.
- **Two places hold text, both need it:** the Svelte UI (Settings, overlay labels such as "Mentioned you") and the Rust side (notification text and buttons, `runner::reason_text`, tray menu, update and chat-logging messages).
- **Approach:** extract every hard-coded string into translation keys as one piece of work (a few hundred, mostly Settings). Frontend via a build-time-checked library (Paraglide suits Svelte); Rust reads the same translation files so both sides stay in step. Language follows Windows' display language, with an override in Settings > General.
- **Translations:** Claude drafts every language first (decent, not native; EVE jargon needs care), then a free open-source platform (Weblate or Crowdin) lets players correct them without touching code.
- **Watch for:** longer strings (German) wrapping or cropping in Settings rows and overlays; Japanese/Chinese/Korean need system-font fallback since Barlow lacks those characters; plurals ("3 lines") and dates per language.
- **Plan:** plumbing and extraction once, then ship English plus drafted Russian, German and Chinese; the rest follow as files.
- Separate from reading non-English *logs* (Open questions below), which needs a real sample first.

## Spoken alerts: Piper voices with a "ship computer" filter (owner request 2026-09-30)

Tried 2026-09-30 with `tools/src/bin/voicefx.rs` (plays a WAV as recorded, then filtered). Windows' built-in voices (David, Zira, Mark) work but sound robotic; the owner liked the filter's echo a lot and wants it paired with Piper's neural voices.

- **Voice engine: Piper, downloaded on demand, never bundled.** When the user turns voices on, the app downloads the unmodified Piper release from Piper's own GitHub release, plus the chosen voice (about 60 MB each, from the rhasspy/piper-voices collection). Because users get Piper straight from its authors and we only run it as a separate program, our code stays MIT (owner decision 2026-09-30: stay MIT). Never compile Piper or espeak-ng into the app. Show where it comes from, and its license, in Settings.
- **Licensing to verify before shipping:** the original rhasspy/piper (MIT) is archived and its successor (OHF-Voice/piper1-gpl) is GPL-3, partly because espeak-ng underneath is GPL. Decide which release to download. **Each voice has its own license** (some non-commercial only): offer only voices whose model card allows it, checked per voice, not from memory.
- **Speed:** measured 0.6 s per short line starting Piper fresh each time (1.8 s for the first, loading the voice). Keep one Piper process running with the voice loaded and feed it lines (expected 0.1-0.3 s), stream its raw audio out instead of writing files, load the voice at startup so the first alert isn't the slow one, and play the channel-open chirp immediately to cover any delay.
- **Filter (rodio), as in voicefx:** a radio band (high-pass ~380 Hz, low-pass ~3.2 kHz), a short two-note chirp before the voice, and two quiet close echoes (45 ms, 110 ms) for a hard metal room. Make the strength adjustable (off / subtle / full).
- **What it says and when:** short and consistent, for example "Holden, mentioned in Local by Amos Burton" and optionally the message. It goes through the same audio player and rules as the alert sounds: volume, quiet time, one at a time, mentions cut through. Per channel it's another layered choice next to Sound: off, sound, or voice.
- **Windows voices** stay available as a no-download fallback.
- Candidate voices tried: Alan and Jenny (British), Amy and Lessac (American). Owner to pick favorites.

## Measurements still open

- Overlay frame-time impact on EVE (FINDINGS #9 only has GPU utilization): needs a present-level capture such as PresentMon, with both clients in a static scene so EVE's own load does not swamp the effect. Also whether an idle open overlay window costs the compositor anything.
- Idle RAM and CPU with one overlay window versus two.
- CPU while animating is about a quarter of one core; check what dominates (likely WebView2 rendering the shadows) and whether the shadows, the arrival animation or the Beacon pulse can be made cheaper.
- Overlays at non-100% DPI scaling and on mixed-DPI monitors (also in FINDINGS' untested list).
- OneDrive cloud-only placeholders: confirm the attribute check keeps the app from hydrating old files (also in FINDINGS' untested list).
- Verify `Create` events in `probe --no-poll` (open a new channel or log a character in).

## Overlays

- Optional frosted glass through Windows acrylic on the overlay window; check its GPU cost over EVE first.

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
