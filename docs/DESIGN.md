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

Memory rule: the Rust core is the always-on part. WebView2 windows (settings, overlays) are created on demand and destroyed after ~45 s idle, sharing one WebView2 environment. All state lives in Rust so webviews can be destroyed freely. Measured (FINDINGS #8, release build): about 6 MB private resident; about 230 MB private while overlays are showing (two monitors), with a first-alert cold start of about half a second. The lifetime meter steps about 4 times a second instead of animating every frame, which avoids about +10 points of compositor (dwm.exe) GPU while overlays are on screen (FINDINGS #9); keep continuous animations off the overlays.

App layout (`app/`): `src-tauri` is the Rust side (`runner` drives the core on a background thread; `overlay` creates one transparent, click-through, non-activating window per monitor on demand and reaps idle ones; `testalerts` has the synthetic alerts and the `--selftest` / `--soak` measurement modes; `diag` writes timing lines when `EVE_CHATTERER_DIAG` is set); `src` is Svelte: `overlay/` (Panel, Strip, Beacon) and `settings/` (a status page for now). There is no permanent window: the app lives in the tray and never exits when its last window closes.

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

Distribution (2026-09-29, patterned on dev-prompt, Windows only): per-user NSIS installer that starts the app at login on a fresh install (never re-enabled by an update; General page toggle via tauri-plugin-autostart); a portable zip that never self-updates; signed self-update from GitHub Releases, checked from Rust every 6 h (first check 20 s after start), announced once per version by a notification plus an "Install update" tray item, installed in passive mode with a restart. See CLAUDE.md "Branches and releases".

Presence and routing (implemented): `core/src/presence.rs` samples the desktop by **polling** (`GetForegroundWindow` plus a window enumeration, about every 250 ms) into a `Snapshot` of client states (focused, minimized, cloaked, visible, bounds, monitor, last known monitor) plus idle time and whether a toast would show. The WinEvent hook is not used: the router only needs the truth at the moment an alert arrives, and the hook would only wake us sooner (kept as a possible latency optimization). A `FocusTracker` ignores explorer shell surfaces (alt-tab switcher, taskbar, desktop switcher) for up to 1.5 s so they do not read as "left EVE". `core/src/router.rs` is pure logic over an `Alert` and a `Snapshot`: per target it applies suppression (focused only by default; visible; none), forces "away" (idle 5 min) to skip suppression and prefer a toast, then picks deliveries per the table above, with the overlay anchor chosen from whether the client covers its monitor. `Engine::observe_clients` marks known pilots live from their windows and reports `ChatLoggingOff` once when a client has no log after 90 s. A client at the login screen (title "EVE", no character) is not a client yet.

Milestone 1 status: workspace is `core/` (library), `cli/` (`chatter`), `tools/` (probes). Done and tested: time and log-format parsing (UTF-16LE, per-line BOM, partial lines, truncation, positional header fallback), live-set tracker (filename parsing, newest per character+channel, supersede, OneDrive placeholder guard, poll), pilot registry (id-keyed, silent historic vs live announcement, JSON persistence), rules (own name, keywords, regex, ignore/always lists, MOTD sender filter, per-pilot overrides), cross-character merge, and the engine. Also done: presence (Windows sampler), the router, and `observe_clients`, with the CLI printing the routing decision for each alert (`--suppress`, `--delivery`). Remaining for the milestone: a live end-to-end check with two characters, and replayable fixtures. Pilot "live" in the core means it produced a line or its session is under 2 minutes old; presence (a running client window) should also call `Engine::mark_live`.

## Finding the live log files

- Poll, do not rely on directory events (FINDINGS #1). The polled set is the **newest file per (character id, channel)**, derived from filenames only. Never trust directory last-write times.
- Directory `Create` events (believed reliable, untested) plus a slow names-only rescan discover new sessions.
- Narrow the polled set further to characters with a running client (window title) so multiboxers with many alts stay cheap.
- Documents folder must come from the Windows known-folder API (the owner's is under OneDrive). Never guess a path.
- The Chatlogs folder may not exist yet: EVE creates it once "Log chat to file" is on, often after the app was installed. The app still starts normally (Settings works), says it's waiting, and starts following the folder when it appears (the rescan retries), with a notification then.
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
| No client focused, some client visible | Native toast (changed 2026-09-28: overlays now belong to their EVE client and sit behind whatever app has focus, so an overlay here would go unseen) |
| No client visible (minimized, other desktop, other fullscreen app) | Native toast with a "switch to <pilot>" action |
| User idle for N minutes | Toast (persists in Action Center), optional sound |
| Toast cannot display (quiet time, DND) | Fall back to overlay; sound is a separate option |

Chat toasts (owner decisions 2026-09-29; `app/src-tauri/src/toast.rs`, mockup `docs/design/windows-notification.html`) are native Windows notifications sent straight through WinRT, not a custom popup: only native ones honor Do Not Disturb, Focus and full-screen apps and keep Action Center history (FINDINGS #11, #12). They carry our look where Windows allows: a rendered badge image (`badge.rs`: the tag on the overlays' steel plate, pilot-accent edge, reason-colored notch), "Sender in Channel", the message, a "Pilot · reason" line, and Switch to (brings that client forward) / Dismiss. How long one stays follows the overlay style's lifetime: Strip short (~7 s), Panel long (~25 s), Beacon sticky until clicked; a mention of the pilot's name is always sticky, whatever the style. Overlays have no sticky equivalent because they are click-through. Repeat lines from the same pilot and channel replace one notification with a running count; mentions have their own slot so chatter never replaces (and un-sticks) one; rate-capped lines update the count in place (replacing it pulled it off screen). The app registers its own identity (HKCU `AppUserModelId`) at startup; all app notifications use it.

Configurable per rule and pilot: overlay / toast / both / sound only. New overlays appear on the virtual desktop the user is currently on. Toasts and overlays are both coalesced (one per channel or pilot, updated in place).

## Settings screen (mockup finalized 2026-09-27, ready to implement)

Mockup: `docs/design/settings-screen.html`. Sidebar/tree layout (owner reference: EVE's Inventory window — collapsible tree on the left, detail pane on the right), not a matrix:
```
Characters             <- CCP's own term; "Pilot" stays as the internal Rust name (Pilot, PilotRegistry,
  Defaults                 pilots.json) since renaming that is a large mechanical change for no functional
  Holden                    benefit, but every user-facing label says "Character"
  Naomi Nagata
  ...
Audio                 <- which sound FILE plays: No sound (global) / one shared file (replaceable) /
                          a file per character. Separate from whether a channel makes a sound at all —
                          that's still each channel's own Sound field (see below).
General               <- startup-with-Windows checkbox (tauri-plugin-autostart, proven in dev-prompt), other app-wide options
About                 <- version, changelog, check-for-updates (tauri-plugin-updater, proven in dev-prompt: see updater.ts's check/install/currentVersion wrapper and the updater_mode() self/managed/unmanaged gate)
```

Owner decision 2026-09-27: **channel settings are authored per character, not as a separate global "channels" section** — individual public channels are inherently per-character (each discovers its own), and even the universal kinds (Corp/Alliance/Fleet/Private) default per character rather than force a single shared page, so "Defaults" is simply the top entry in the Characters tree. **Every channel row, on Defaults and on every character's page and on every individually discovered public channel, shows the identical full field set** (Mode, Style, Rate cap, Sound) — no row is ever sparser than another; an earlier mockup pass trimmed some rows for brevity and that was wrong. **Tracked keywords are per character too**, for the same reason and using the same override mechanism (a character can hold its own list instead of Defaults'), not a global-only list.

**Live inheritance, not copy-at-creation** (owner decision, same day): a newly detected character gets an *empty* settings layer — nothing is copied from Defaults. It resolves through to whatever Defaults currently says for every field it hasn't touched, forever, and automatically follows any later change to Defaults. Only a character's own explicit override (`Some(_)` in its `Layer`, vs `None`) freezes a field against future default changes. This matches the core's existing `Settings::resolve()` and needs no new mechanism, just a UI over it — and matches how the owner already talks about changes ("treat any change to Corp as applying to Alliance").

**Deviation indicator is per FIELD, not per row** (revised from the first mockup pass, since a single row can have some fields overridden and others inherited at once — e.g. a character might override just a channel's Style and nothing else on that row): the overridden field's label gets a small persistent marker (owner reference: PrusaSlicer's modified-vs-profile dot, but permanent — every field's `Option<T>` already IS the saved state, there's no separate unsaved-changes session) plus an inline "↺ use default" revert control next to that one field. An untouched field just shows the inherited value with no decoration. Internal implementation notes (e.g. "Corp is kept identical to Alliance") never appear in this UI — that's a note to future code-editors, not something a user needs to know; Corp and Alliance render as two perfectly ordinary, independently-editable rows.

**The Defaults page itself never shows deviation chrome** (bug found and fixed 2026-09-27, real implementation): `Settings::with_defaults()` pre-populates `settings.kinds[kind]` with real values from the moment the app starts (Fleet/Corp/Alliance get Everything+Panel+cap, etc.) — those are the shipped starting point, not something the user set, but the naive "does this exact layer have a value" check lit up as an override on the very page that *is* the source everyone else inherits from. Owner correction: the Defaults page isn't inheriting from anything a user should think about, so it must never show a pip or a "use default" link, even though the underlying field IS genuinely set at that layer. Fix (`ChannelRow.svelte`, `KeywordsSection.svelte`): a `showDeviation` flag gates only the decorative pip/revert/highlight, `pilotId !== null`; the same underlying "does this field have a value" check still drives real control state (checkbox checked, whether the rate-cap number input shows) on every page, Defaults included — only the chrome that implies "you changed this away from something" is suppressed there.

**No Save button — every edit autosaves, debounced** (owner decision 2026-09-27: "action = save... instead of a save button, just real time"). `Settings.svelte`'s `onedit()` schedules `save_settings` 600 ms after the *last* edit rather than sending one per keystroke/click, so a burst of changes (typing a keyword, dragging a rate cap) collapses into one write; an edit that lands while a save is already in flight is captured and re-sent the moment that save finishes, so nothing is lost to the race. The savebar keeps its status text (Unsaved changes / Saving… / All changes saved / a validation error) with no button. Flushes immediately, bypassing the debounce, on `visibilitychange` (covers the settings window's close button, which `lib.rs` hides rather than destroys) — the one gap this doesn't close is quitting the whole app via the tray within the sub-second debounce window right after an edit, which was judged not worth synchronous quit-blocking IPC to close.

**Audio is a separate top-level page, not a per-channel field for "which file"** (owner decision 2026-09-27, after two passes — the first mockup wrongly removed per-channel Sound entirely when adding this page): three choices — No sound (global kill switch), one shared sound file (built-in initially, replaceable via Browse so a single custom preference never needs repeating per character), or a distinct file per character (any character left unset falls back to the shared file). This page only decides *which file* would play; *whether* a given channel makes a sound at all remains a normal cascaded per-channel/per-character `Sound` field, on equal footing with Mode/Style/Rate cap, so a channel can be silenced independently down to one character, one channel.

**Audio as built** (2026-09-29): settings.json holds an app-wide `audio` section (`core/src/audio.rs`, not layered): mode (off / shared / per character), the shared file (none = a built-in sound synthesized in code, nothing to license: two clean synth tones F5 → C6 that swell, hold and release, with a detuned layer and a short soft tail; owner-tuned 2026-09-29 after rejecting struck/bell shapes as "physical instruments" and square-wave lo-fi as "too 80's"), files by character id, volume, and a cooldown. Rules (`SoundGate`): one sound at a time; after a sound, nothing else plays for the cooldown (default 10 s, owner request, adjustable); a mention ignores the cooldown and cuts off whatever is playing. One message makes one sound however many characters it alerted (a mention's character picks the file, else the first). Chosen files play at most 15 s; an unreadable file falls back to the built-in sound. The volume slider is squared (perceptual), default 50%. The output device is opened per sound, so a device change is picked up. Windows notifications are always silent (`<audio silent="true"/>`): the app's own sound is the only one, so the cooldown and "No sound" hold everywhere.

Small UI-polish notes worth preserving from the mockup review: character status badge reads "Online," not "Playing"; every `<button>` needs an explicit `border: none` (plus `appearance: none`) — leaving the native OS border un-reset while adding a custom background produces the "colored button with a stock beveled border" look, easy to miss and easy to get right with one blanket reset rule.

Next: wire this into the real Svelte settings window and the settings load/save commands, replacing the current placeholder status-only window (`app/src/settings/Settings.svelte`).

## Overlay anchoring

Overlays are separate topmost, click-through, non-activating windows in screen coordinates (not injected into the game). Per-pilot anchor mode:
- **Monitor**: default when the client covers its monitor (Fixed Window, Fullscreen).
- **Follow client window**: default for Windowed mode; tracks the client's bounds, hides when it is minimized or cloaked.
- **The screen the user is focused on always wins** (`core/src/router.rs::anchor_for`), even when the *alerted* pilot's own client is also genuinely on screen elsewhere (e.g. each client fixed to its own monitor). Found live in two stages, both 2026-09-27, same underlying principle: (1) owner, focused on Holden: "if im on holden, and something happens in Naomi's chat rooms, the notification needs to show up on Holden's screen, not Naomi's" — first fix only redirected when the alerted pilot's client was *hidden*; (2) owner, focused on Naomi: "holden's Local notifications are still showing up on Holden's screens" — revealed the first fix wasn't enough, because Holden's client was still genuinely on screen (its own monitor), so the old "is it visible at all" check let it win over "is it what the user is looking at." An alert shown on a screen nobody is watching defeats the point of an alert, no matter whose screen it technically is.
- Priority when placing an overlay: (1) the focused client's screen, if on screen; (2) otherwise the alerted pilot's own screen, if on screen (a neutral choice when nobody in particular is focused); (3) otherwise any other on-screen client; (4) otherwise the alerted pilot's own last known monitor, as a least-bad guess (Auto mode would usually toast instead in this last case; it only matters when a toast is blocked and falls back to an overlay).
- Automatic choice from whether the client window fills its monitor; per-pilot override.
- Suppression (whether to show the alert at all) is a separate, already-correct concern from anchoring (which *screen* to use if shown): `Suppression::FocusedOnly` (default) already skips only the alert for the pilot the user is currently focused on, and `Suppression::AllowAll` already exists per pilot/channel for "never suppress even my own focused character". Exposed in the settings screen as "Suppress", after Style (`ChannelRow.svelte`, 2026-09-28): "When client is focused" / "When client is visible" / "Never" (wording by the owner, 2026-09-29). A character's own typed lines still never alert that character, whatever this is set to (owner, 2026-09-28: alerting on your own message on your own screen would only confuse people).

## Overlay reposition & resize (owner decisions 2026-09-28)

Modeled on Discord's in-game overlay: a hotkey (Ctrl+Alt+O by default, changeable on the General page: `hotkey.rs`, which registers a new combination before releasing the old one, and a recorder ported from dev-prompt that refuses Windows-reserved combinations and warns about common ones) drops click-through so the player can drag each character's alert region and resize its width, rather than a position grid in the settings screen. Found and settled live, in order:
- **One window per character, not per monitor** (`overlay.rs`). Per-monitor windows were shared by every character on that monitor, so "this character's position" had no window to belong to.
- **Only the focused client's character** is offered for positioning (every on-screen client when focus is elsewhere). Offering every known character stacked offline alts on top of each other.
- **The placeholder is a real sample Panel** inside a dashed outline, so what is positioned is what an alert looks like at that width.
- **The box belongs to the client, not to whoever an alert is for** (owner decision 2026-09-30): every alert drawn over a client uses that client's character's box, width and stack, including another character's alert shown there because that screen is the one being looked at. The alert itself still names the character it is for. With no client under it, the alerted character's own placement applies.
- **Width only, per character, not per style**; clamped to 320-760 logical px (`pilots::MIN/MAX_OVERLAY_WIDTH`) for readability, not arbitrary limits.
- **Drag and resize are ours, not the OS's.** The page reports pointer deltas; Rust moves the window. A native window drag looked and behaved like dragging a desktop window and could leave the game or the monitor.
- **Constrained to the game's viewing area**: the client area (`winapi::client_rect_of`), excluding a windowed client's title bar and borders; the monitor for fullscreen.
- **Saved relative to that area** (`OverlayPlacement { fx, fy, width }`, fractions of its free space), so it survives the client moving or resizing. A box in the lower half makes alerts stack upward from it.
- **The box's position is only ever computed, never read back** from the window during a session; reading back an asynchronously moved window drifted ("like it's in water"). Fractions use a fixed reference height (`BOX_H`) on both save and restore; using the measured height on one side made the box creep on every open/close. Since 2026-09-30 a save also records the **anchored edge** (`OverlayPlacement::edge`): the box's top edge in the upper half, its bottom edge in the lower half, where its alerts grow up from. Alerts start exactly on that edge whatever height the box measured; `BOX_H` alone had put bottom-half stacks off by the box's height difference. Older saves place by the fractions until repositioned.
- **Overlays belong to their EVE client**: each is an owned window of it (`platform::set_owner`), so it sits directly above that client rather than topmost over every app; it is pinned to the client's virtual desktop and hidden in step with the client's cloak events (a desktop switch otherwise flashed it on the new desktop); and a WinEvent hook (`clientmoves.rs`, out-of-context: nothing loaded into EVE) moves it as the client is dragged. Owning windows across processes can tie the two UI threads' input together; measured, a frozen app UI thread doesn't stall EVE's input (FINDINGS #16).
- **Not injected.** Discord draws inside the game by hooking its renderer; CLAUDE.md forbids injecting into the client. The owned window is the closest equivalent without it. An opt-in injected renderer could only follow written approval from CCP.

## Overlay styles

Three styles, used together for different situations (mockups with the exact look: `docs/design/alert-styles.html`, derived from the owner's EVE Drones window: translucent steel glass, sharp corners, thin type, three-segment bar, one blue badge):
- **Panel** (about 420 x 118): header with pilot and channel, sender, message, reason label, lifetime meter. Default for keyword and regex matches.
- **Strip** (about 420 x 34): one line, pilot-colored edge and a short tag (up to 5 characters). Default for always-alert channels/senders and for folding bursts. Owner decision 2026-09-27: the badge started as a single letter, which two characters (possibly on different accounts) starting with the same letter can't tell apart — each `Pilot` now has an optional `tag` (`core/src/pilots.rs`, editable on the character's own settings page, `ChannelsPage.svelte`), and `Pilot::display_tag()` falls back to deriving one from the name (first letter of each whitespace-separated word, any run of spaces collapsed, e.g. "Naomi Nagata" -> "PA") when it's unset. Both cases are capped at 5 characters and shown uppercase. Computed server-side (`runner::tag_for`) and sent as `OverlayAlert.tag`, since the frontend has no way to know a pilot's own tag from the name alone.
- **Beacon** (about 460 x 150): larger, five soft pulses on arrival (owner choice 2026-09-30; about 4.5 s), up to three lines of message. Default for own-name mentions.

Shared details: the **lifetime meter** is three segments that drain in turn; the **blue badge** is the count of lines folded into an alert; the top notch/edge takes the reason color (mention amber `#e3a53a`, keyword cyan `#55c4d6`, always blue `#6f9fe0`); each pilot has an accent color. Overlays are click-through, so they contain no buttons or close marks; anything interactive is a toast. No blur of the game behind them (WebView2 cannot; Windows acrylic on the window is a later option). Font: Barlow, to be bundled with the app.

Style choice: `router::StyleMap` maps the reason to a style (mention Beacon, keyword/regex Panel, always Strip) with a per-pilot override; both are settings. Burst folding (a fast run of alerts collapses into a Strip stack with a count badge) needs memory of recent alerts, so it lives in the overlay manager, not the pure router. A Beacon takes the top of the stack with Strips beneath; lifetime can differ per style (Beacon longest).

## Tracked keywords/patterns: per-entry channel scope and mute exemption (owner decision 2026-09-27)

Originally one flat character-wide list each for keywords and regexes (`Layer.keywords`/`Layer.regexes: Option<Vec<String>>`), always fired under every mode including "Nothing," everywhere. Two things changed:

1. **Targeting specific channels.** Considered layering the list per channel (like Mode/Style/everything else), but rejected: "the same string/regex in 3 of 5 channels" turns into unmanageable duplicate per-channel lists fast — a real case for nullsec alliance members wanting a term in Corp+Alliance+Local but not Fleet+Private. Chosen instead: **scope lives on the entry, not the layer.** `Layer.keywords`/`regexes` merged into one `Layer.tracked: Option<Vec<TrackedRule>>`, where `TrackedRule { text, kind: Keyword|Regex, only_in: Vec<ChannelKind>, even_when_muted: bool }` — `only_in` empty means every channel, non-empty means just those kinds. `Settings::resolve()` only ever reads `tracked` from the Global and Pilot(base) layers (never Kind/Channel/PilotKind/PilotChannel, unlike everything else) and filters by `only_in` against the channel being resolved. Originally a character's own list fully *replaced* Defaults'; changed 2026-09-29 (owner): the two **merge**, since tracking only ever adds things to watch for and there is nothing to conflict. The same text and kind at both levels is one entry, the character's copy winning; a character's page lists only its own entries, with nothing to flag or override.
2. **Muted-channel exemption is per entry, not global** (owner: "since we are adding this to the system for managing a keyword, we should just add this to that screen as well"): `even_when_muted` (default true, matching the original only behavior) lets one specific term go quiet on a channel set to "Nothing" while everything else still overrides it. `rules::CompiledRules::tracked()` takes a `muted: bool` (true only for `Mode::Nothing`) and a term with `even_when_muted: false` is excluded there — `always_senders` is untouched, a separate always-on feature.

UI (`TrackedListSection.svelte`, one merged "Tracked" section, not separate keyword/pattern sections — see below): the Add dialog got a String/Regex toggle plus a channel-scope picker (chip row: "All channels" or specific kinds) and the mute-exemption checkbox; clicking a chip's text reopens the same dialog pre-filled to edit it in place (chips aren't just add-then-immutable). A scoped or mute-exempt entry shows a small dim suffix on its chip (`· Local`, `· ignores Nothing`) and a fuller explanation in its hover title.

(Earlier the same day, keywords and regexes had briefly gotten their own separate "Tracked keywords"/"Tracked patterns" sections in the UI. Owner pushback: a plain keyword is already valid regex syntax with no special meaning, so two sections just to hold two lists felt like needless duplication — but naively merging into "everything is regex" was rejected too, since ordinary chat text often contains real regex metacharacters (a system name like "4.4", corp tags like "[FLYSF]", "(Auction)") that would silently mean something else. Resolution, matching regexr.com's convention: one list, a String/Regex toggle decides intent explicitly per entry rather than inferring it from the text, and Regex mode decorates the input with slashes that are purely visual chrome, never part of the stored value.)

## Always alert / Ignore senders (owner decision 2026-09-28)

`RuleSet.always_senders`/`ignore_senders` (`core/src/rules.rs`) existed since the very first settings layer and already resolved correctly — an always-alert allow-list and a block-list, checked in `tracked()`/`ignored()` — but never got a settings-screen control, unlike everything else. No core change needed to add one: `Layer.always_senders`/`ignore_senders: Option<Vec<String>>` already exist and already ride the same live-inheritance mechanism as everything else (Defaults sets the baseline). Originally a character's own list fully replaced Defaults' the moment it had one, so a name added to Defaults later never reached it; changed 2026-09-29 (owner) to **merge by name**: Defaults decide every name, and a character overrides only the names it lists itself, as Alert, Ignore, or "Normal" (`Layer.normal_senders`, to undo one of Defaults' entries for that character). Within one layer, Ignore beats Alert beats Normal, as in `rules`. Loading drops the copies of Defaults' entries the old copy-on-first-edit UI left in characters (`drop_copies_of_defaults`), for both lists. Deliberately *not* given per-channel scoping or a mute-exemption toggle like Tracked entries got — "ignore this person" doesn't usually need to vary by channel the way a keyword did for a nullsec alliance member, and "always" overriding a muted channel is the entire point of that list, not a per-entry option.

UI (`SenderListSection.svelte`, "Always alert / Ignore"): one merged, alphabetically-sorted list rather than two lists or two visual groups — chosen over grouping by type because a mixed list scans better once it has more than a couple of entries, same reasoning as the Tracked list being one section instead of two. Each chip gets a glyph *and* a color, not color alone (a checkmark in `--always` blue for Always-alert, a slashed circle in `--danger` for Ignore), so the distinction doesn't rely on color perception. Add/edit dialog is the same shell as Tracked's, with an Always/Ignore toggle in place of String/Regex and a plain exact-name input, no scope picker, no mute checkbox. The section's own note text says explicitly that both override a muted channel, so nobody has to go read the code to find that out.

## Granular settings (implemented in `core/src/settings.rs`)

Owner requirement: decisions per channel and per character, because different characters have different jobs and their tolerance for "annoyance" differs.

Channels come in kinds, classified from the header channel id (`core/src/channel.rs`, FINDINGS #3): **fixed** (Local, Corp), **situational** (Alliance while in an alliance, Fleet while in a fleet), **public** (CCP's and player-made, stable id), and **private messages**. Fleet and private ids are new every time, so they can only be configured as a kind ("all fleets", "all private messages"); Local, Corp, Alliance and public channels have stable ids and can also be configured by id (shown by name).

Every setting is optional at each level and inherits downward (least to most specific):
```
global -> channel kind -> one channel (by id) -> pilot -> pilot + kind -> pilot + channel
```
Owner decision: defaults come from the channel kind, and **a pilot's own setting overrides them** ("if a pilot changes that, it's for a reason"). Two kinds of setting:
- **Preferences** take the most specific value that is set; the tracked list and the sender lists are the exception: they merge across levels (see "Tracked" and "Always alert / Ignore senders" above). They are: `mode`, the content rules, `delivery`, `suppression`, `style`, `sound`.
- **Limits** are ceilings that all apply: a rate cap at any level counts per pilot over the last minute, and a channel cannot lift a pilot's cap. Over the cap an alert is dropped or folded into the count badge (the strictest exceeded cap decides). Limited alerts are not counted, and suppressed alerts never count. A mention of the character's own name is never capped and never counted (owner decision 2026-09-30): it's the one alert that must always get its Beacon, notification and sound. A folded line goes where its alert would have gone. It joins the count of that pilot and channel's alert if one is still showing there; otherwise (expired, dismissed, never shown) it shows as a Strip, or a new short notification, so a capped line is never lost.

### Notification modes (`prefs::Mode`, revised 2026-09-27)

Named after Discord's own per-channel levels, and **exactly three values**: `Nothing`, `Mentions`, `Everything`. An earlier version added a fourth value, `Matching` (mentions widened to include keywords), sitting between `Mentions` and `Everything` — the owner corrected this: keyword/regex/always-alert-sender tracking ("special things you want to track for") is **not a rung on the mode ladder, it is a separate, independent layer that runs alongside every mode, `Nothing` included** ("Matching is a parallel concept, not a hybrid... it works independently, alongside the others"). Muting a channel for general chatter must not silence something explicitly tracked — e.g. a busy trade channel is muted, but a keyword for one item you're hunting still alerts.

Implementation (`rules::CompiledRules`): `tracked()` (always-senders, keywords, regexes — the independent layer) is checked under every mode; `mentioned()` (own name) is added for `Mentions`; `Everything` prefers a specific tracked reason over its generic "every line" one when both would fire. `evaluate_mode`:
- `Nothing` → `tracked()` only.
- `Mentions` → `mentioned()` or `tracked()` (own name checked first).
- `Everything` → `tracked()` or `mentioned()` or the generic always-channel reason.

Built-in defaults by kind (proposals for the owner to tune; `Settings::with_defaults`; owner decision 2026-09-27: PM/Fleet/Corp default to `Everything` unless stated otherwise, since they're addressed to or shared with a small circle; Alliance moved into the same group as Corp per a second owner decision the same day — "treat any future change to Corp as applying to Alliance too", since they're the same relationship at a different scope): private messages `Everything` as a Beacon, uncapped; Fleet, Corp and Alliance `Everything` as a Panel (the reason's default style for "everything" is Strip, which turned out too easy to miss live-testing a Fleet alert — a name mention still gets Beacon), capped at 6/minute (they can still get busy); Local `Mentions` (mention or a tracked keyword/sender), capped at 6/minute; public channels `Mentions`, capped at 4/minute; past the cap, folded into the count badge. Keep Corp and Alliance's defaults identical when editing either. First annoyance controls shipped (owner approved): mode, rate cap, sound on/off; the structure allows more (BACKLOG).

Code: `prefs` (the vocabulary), `settings` (`Layer`, `PilotSettings`, `Settings`, `resolve`, and `SettingsBook` which caches resolved behavior per pilot and channel because every line is checked), `rules` (content rules and `evaluate_mode`), `engine` (resolves per listener and puts each target's resolved `Prefs` on the alert), `router` (uses those prefs), `governor` (rate caps, needs memory). Settings and pilots persist as JSON through `core/src/store.rs`: a save is flushed to disk before it replaces the old file, and the old one is kept as `<name>.bak`. A file the app can't read (a crash mid-write, a sync conflict, a newer version's values) is never overwritten: it's renamed to `<name>.corrupt-<unix secs>.json`, the backup is used if it reads, else the defaults, and the user gets a notification naming the kept file. A missing file means the built-in defaults. Settings UI implication: a pilots x channels matrix that shows inherited values in a muted color and overrides in full color.

## Rules UX

A few first-class rules (name mention, keywords, per-channel) with regex behind an "advanced" toggle.

Cross-character duplicates (implemented in `core/src/merge.rs`): the same (channel, sender, text) seen by different listeners within ~2 s is merged into **one line with a `seen_by` list**. Rules are then evaluated per listener (own-name is per pilot), so an `Alert` carries `targets` (pilots whose rules matched, each with its reason) and `seen_by` (every character whose log had the line). The router, which knows about windows, applies suppression per target using presence, and can use `seen_by` to know which screens already showed the line. A first copy is held ~750 ms, only when another character is following the same channel id, so the second copy can join; a lone copy is emitted after the hold. A copy that arrives after its line already went out still joins `seen_by`, and is evaluated for its own character alone: that character's rules (its own name above all) never saw the line, so skipping it would lose a mention. Local is a separate channel per solar system but all share the channel id `local`, so two characters in different systems also pay the 750 ms hold (accepted; tunable).

## Log format compatibility

Parse defensively: explicit UTF-16LE decoding, a BOM before every line, complete-line consumption only, header read by position with keys as a check (localized headers are an open question, see BACKLOG), UTC timestamps everywhere, sender/text split on the first `>` after the timestamp. The old app offered a configurable regex "in case the log format changes"; keep an equivalent escape hatch.

## Prior art

MyUncleSam/EveChatNotifier (C#, MIT): polls file sizes on a timer, rescans the folder for new files, filters by last-write time (default 8 h), and offered a "move old logs" mode purely to keep the polled set small. Useful edge cases: MOTD lines from the `EVE System` sender at login (its text-based filter broke on localization), multi-client duplicate notifications (crude 1 per second throttle), ignore-own-messages, always/ignore lists per channel and pilot. Its weaknesses: decodes UTF-8 after a mid-file seek (works only because of the per-line BOM), no partial-line handling, no truncation handling, no live-set logic. py-eve-chat-mon independently reports the same event problem and polls.
