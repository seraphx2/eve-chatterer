# Code review (2026-09-30)

A review of the whole codebase before the next release, at commit `61a9066` on `dev`. `core/`, `app/src-tauri/`, `app/src/`, `cli/` and the workflows were read in full. `tools/` was only skimmed (dev-only probes), and the CSS wasn't reviewed in depth. Tests, clippy and a build were not run for this review.

Line numbers refer to that commit; function names are given so each spot can still be found once the code moves. Each item says what is wrong, how it goes wrong, and the fix. Like BACKLOG.md, this holds open items only: remove an item when it's fixed (git history keeps it), and delete this file when nothing is left. Measurements go to FINDINGS.md and decisions to DESIGN.md. IDs aren't reused, so gaps are items already fixed.

## Plan (agreed with the owner 2026-09-30)

- **Before the next release:** H1–H5, M1, M4 and M5 are fixed. What remains is the H6 measurement (`--freezetest`, needs the owner with EVE running) and the owner's check of the fixes with two clients.
- **H4 decision (owner, 2026-09-30):** own-name mentions skip rate caps and don't count toward them.
- **Everything else:** after the release.

## High

### H6. Cross-process owned overlays may tie our UI thread's input queue to EVE's (needs a measurement)

- **Where:** `app/src-tauri/src/overlay.rs` (`platform::set_owner`), and DESIGN "Overlays belong to their EVE client".
- **What:** making EVE's window the owner of our overlay creates a cross-process owner/owned relationship, and Windows then attaches the input queues of the two threads (Raymond Chen, "Is it legal to have a cross-process parent/child or owner/owned window relationship?", The Old New Thing). If that holds here, EVE's input on that client can stall whenever our UI thread stops pumping messages: a hang, WebView2 building a window (0.4 s cold), or any long piece of work on that thread. Keyboard and focus state are shared too.
- **Gap:** FINDINGS #15 measured frame times with `--selftest` / `--soak`, whose overlays are created with `owner: None` (`testalerts.rs`). The owned path has never been measured.
- **Measurement:** run the app with `--freezetest`, then switch to the desktop with the EVE clients.
  1. Every 8 s it shows one alert of each style over every client on that desktop, owned by it like a real alert.
  2. At 30, 60 and 90 s it blocks our UI thread for 5 s. A Beacon over each client warns 3 s ahead, one chime marks the start and two the end. Move the camera and type in EVE chat between the chimes.
  3. Afterwards, `%TEMP%\eve-chatterer-freezetest.log` confirms how many clients had overlays at each round and gives wall-clock times, to compare frame times in a PresentMon capture run alongside.
  4. Record the result in FINDINGS.
- **First run (2026-09-30):** no stutter noticed, but inconclusive: the prompts were on the console on another virtual desktop, so the freezes weren't timed with camera movement, and no log recorded whether overlays were over the clients. The in-game cues and the log were added for that.

  If input does stall, the owner decides between keeping ownership (and guaranteeing the UI thread never blocks) and going back to topmost windows with our own z-order and cloak handling.

## Medium

### M2. The Settings window is never destroyed, and it polls while hidden

- **Where:** `app/src-tauri/src/lib.rs` (`open_settings`), `app/src/settings/Settings.svelte` (the 4 s `load` interval).
- **What:**
  - Closing hides the window so it reopens instantly, but its WebView2 stays alive for the rest of the session. That keeps the whole WebView2 process tree alive, so after opening Settings once the app never gets back to its ~6 MB idle footprint (DESIGN memory rule, FINDINGS #8).
  - Its 4 s `get_settings_data` poll keeps running while hidden, serializing all settings and the pilot registry each time.
- **Fix:**
  - Destroy the window after it has been hidden for a while (e.g. the overlays' 45 s).
  - Stop polling while hidden.
  - Replace polling with events from Rust (F7).

### M3. One chat line can produce one notification or overlay per character

- **Where:** `core/src/router.rs` (`route`), `app/src-tauri/src/runner.rs` (`deliver`).
- **What:** each target is routed on its own.
  - With no client focused (browser, Discord, the Settings window), a Defaults keyword such as "@all" in a Local both characters are in gives two identical notifications.
  - With Suppress set to Never, it gives two overlays on the same screen.
  - Since the H3 fix, a copy that arrives after its line went out is evaluated for its own character as a separate alert. A keyword both characters match can then show twice in the same way.

  This undoes the cross-character merge. DESIGN's "the router can use `seen_by` to know which screens already showed the line" was never built.
- **Fix:** group deliveries per alert: one notification or overlay per destination, listing every character it's for, with the strongest reason and style. Keep suppression and caps per character. Remember recently delivered lines per destination for a few seconds, so a late copy's alert joins the one already shown.

### M6. "Switch to" stops working on notifications older than 2 minutes

- **Where:** `app/src-tauri/src/toast.rs` (`BURST`, and `bursts.retain` in `State::chat`).
- **What:** the `ToastNotification`, and with it its click handler, is dropped when another chat notification arrives more than 2 min later. Sticky mentions wait on screen or in Action Center, and the away case is exactly when someone clicks late. The click then does nothing.
- **Fix:** keep shown notifications alive independently of the burst window (e.g. the last N by tag), or register a COM activator (the limit noted in FINDINGS #12).

### M7. A quick double press of the reposition hotkey can leave an interactive box over the game

- **Where:** `app/src-tauri/src/reposition.rs` (`toggle`), and the shortcut handler in `lib.rs`, which starts a new thread per press.
- **What:**
  1. Press 1 sets the flag and starts `enter`; building the window takes ~0.4 s cold.
  2. Press 2 clears the flag and runs `exit`, which finds no sessions yet.
  3. `enter` then finishes and adds its sessions with click-through off.

  The box sits over the game taking clicks while the app thinks reposition mode is off, and one more press is needed.
- **Fix:** run toggles on one worker fed by a channel, or hold a toggle mutex for the whole enter or exit.

### M8. The rate cap field suggests a character can raise the Defaults cap

- **Where:** `app/src/settings/ChannelRow.svelte` (the Rate cap field).
- **What:**
  - Every level's cap applies and the strictest wins, so setting a character's Local cap to 20 when Defaults says 6 changes nothing. The row still shows "default: 6/min" as if the new number replaced it.
  - The number input always writes `over: "fold"`, turning a hand-set Drop into Fold.
  - "default" shows the first (least specific) cap, not the strictest.
- **Fix:** show the effective limit (the strictest inherited cap), say that a higher number has no effect, and keep the stored `over`.

### M9. The release job exposes the updater signing key to actions pinned only by tag

- **Where:** `.github/workflows/release.yml:56-99`.
- **What:**
  - `tauri-apps/tauri-action@v1`, `Swatinem/rust-cache@v2`, `dtolnay/rust-toolchain@stable` and `actions/*@v7` all run in the job that has `TAURI_SIGNING_PRIVATE_KEY`.
  - If a tag of any third-party action is moved, it could read the key, and whoever holds the key can ship updates every installed copy accepts.
  - The npm build also runs with the key in its environment (tauri-action runs `beforeBuildCommand`).
- **Fix:** pin actions to full commit SHAs; Dependabot already watches github-actions and keeps pinned SHAs current. The key is already scoped to the tauri-action step; keep it that way.

## Low

- **L1. A half-written header can be cached for the whole session.** `core/src/logfmt.rs:157-162` (`read_header`), `core/src/tailer.rs:28, 55-57`.
  - `read_header` parses whatever bytes exist, including an unterminated last line.
  - A Listener line caught mid-write gives a cut-off name that is never re-read; only truncation resets the header.
  - Fix: parse only up to the last CRLF, and accept the header only once its "Session started" line is complete.
- **L2. A reload can overwrite an edit.** `app/src/settings/Settings.svelte` (`load`, the 4 s interval).
  - The 4 s reload checks `dirty` before it asks. An edit made while `get_settings_data` is in flight is replaced by the older response, and the debounced save then writes that older state.
  - Fix: an edit counter, so a response is dropped if an edit happened since it was requested. Or remove the poll (F7).
- **L3. Own-name matching is a plain substring.** `core/src/rules.rs` (`mentions_own_name`).
  - Short names over-match ("Ash" in "wash"), and a first-name mention ("Naomi" for "Naomi Nagata") never matches.
  - Owner question: word boundaries, and whether first names count.
- **L4. `drop_copies_of_defaults` runs on every load.** `core/src/settings.rs` (`drop_copies_of_defaults`, called from `upgraded`).
  - It was meant as a one-time migration, but it keeps erasing any character entry equal to Defaults'. A later change to Defaults then reaches a character that had deliberately set the same value.
  - Fix: a settings format version, with migrations run once.
- **L5. The settings cache grows with Fleet and Private ids.** `core/src/settings.rs` (`SettingsBook::resolved`).
  - The cache key includes the channel id although those kinds never use it, so every new fleet or conversation adds an entry with compiled regexes, until the next edit clears the cache.
  - Fix: use "" as the channel id in the key for kinds without stable ids.
- **L6. `last_seen` doesn't mean last active.** `core/src/engine.rs` (`observe_pilots`), `core/src/pilots.rs` (`note_channel`).
  - It's bumped every tick for every followed session (any log from the last 14 days), and only saved when pilots.json is saved for some other reason.
  - The "last active N days ago" caption and the planned auto-cleanup (BACKLOG) both need real activity: bump it when a line is seen.
- **L7. `follow()` asks Explorer for each overlay's desktop four times a second while holding the slots lock.** `app/src-tauri/src/overlay.rs` (`follow`, `pin_to_owner_desktop`).
  - `GetWindowDesktopId` is a cross-process COM call, so a stalled Explorer stalls the runner while it holds `slots`.
  - Fix: re-pin on cloak events (already hooked) and on a slow timer.
- **L8. The CSP is null and every command is open to every window.** `app/src-tauri/tauri.conf.json:15-17`, `capabilities/default.json`.
  - Overlays render other players' chat. Svelte escapes it and there's no `{@html}`, but a strict CSP and per-window command permissions (an app manifest in `build.rs`) would contain a future slip.
  - Overlays need only `overlay_ready` and the four reposition commands.
- **L9. `reg.exe` and `explorer` are started by name.** `app/src-tauri/src/toast.rs` (`register_identity`), `lib.rs` (`open_folder`).
  - Windows searches the exe's own folder first.
  - Fix: use `RegSetKeyValueW` (lib.rs already uses the registry API) and the full path to explorer.exe. This also saves two process launches on every start.
- **L10. The whole folder is rescanned every 5 s.** `core/src/engine.rs` (`rescan_every`), `core/src/liveset.rs` (`rescan`).
  - Long-time players have tens of thousands of files. FINDINGS #1 shows Create events are timely, and DESIGN planned to use them for discovery.
  - Fix: discover through events, keeping a slow rescan as the fallback.
- **L11. Smaller overlay issues.**
  - An idle window can be reaped between `enter_reposition`'s `ensure_slot` and its lock, and that character gets no box.
  - After reposition mode ends, the window stays box-sized, and `follow_client` re-places it from the old layout until the next alert.
  - `label_for` gives two names the same label when they differ only in punctuation (only `name:` keys), and `key_from_label` doesn't reverse it.
- **L12. Control characters in toast XML.** `app/src-tauri/src/toast.rs` (`xml_escape`).
  - They're invalid in XML 1.0, so a line containing one fails `LoadXml` and the notification is dropped with only a log line.
  - Fix: strip them in `xml_escape`.
- **L13. `pick_sound_file` blocks a runtime worker while its dialog is open.** `app/src-tauri/src/lib.rs` (`pick_sound_file`).
  - It calls `blocking_pick_file` inside an async command.
  - Fix: use the callback form.

## Docs out of step with the code

- **D1. Away mode is live, but the docs and the UI say it isn't.**
  - The router treats 5 min without input as away (`router.rs`, `RouterConfig::idle_after`), and `Sampler` measures idle time with `GetLastInputInfo`.
  - BACKLOG "Away mode" says `route` "takes an away input; nothing sets it yet (always false)", and that the focused character's alerts stay suppressed while away.
  - The General page shows the setting as "coming later" (`GeneralPage.svelte`).
  - Update BACKLOG, and either wire the setting or reword the page.
- **D2. Probe locations.**
  - CLAUDE.md says "shared Win32 helpers in `tools/src/winutil.rs`"; they are now `tools/src/lib.rs` re-exporting `core::winapi` as `winutil`.
  - The FINDINGS intro says "Probes live in `src/bin/`; shared helpers in `src/winutil.rs`".
  - The DESIGN intro says "pre-implementation … probes in `src/bin/`".
  - CLAUDE.md's tools list is missing `notifstate` and `toastprobe`.
- **D3. DESIGN still describes the Settings window as a placeholder.**
  - "`settings/` (a status page for now)" under App layout.
  - "Next: wire this into the real Svelte settings window" at the end of "Settings screen".
- **D4. DESIGN's memory rule is wrong for Settings.** It says settings and overlay windows are destroyed after ~45 s idle, but the Settings window is hidden, never destroyed (M2).
- **D5. DESIGN's architecture block describes code that doesn't exist.**
  - It names `dedupe` and `coalesce` modules; the code has `merge`, `governor` and the overlay manager.
  - "Discovery via Create events" isn't built (L10).
  - "Config in %APPDATA%" leaves out portable mode.
- **D6. DESIGN "Rules UX" is out of date on merging.**
  - It says two characters in Local in different systems still pay the 750 ms hold, but `LiveSet::listeners_in_channel` compares each channel's instance (the solar system, for Local) when both logs name it, so they don't.
  - "The router … can use seen_by to know which screens already showed the line" isn't built (M3).
- **D7. Other stale DESIGN lines.**
  - "Settings UI implication: a pilots x channels matrix" (Granular settings) was superseded by the tree layout.
  - "presence … should also call `Engine::mark_live`" (Milestone 1 status) is done by `observe_clients`.
  - "regex behind an 'advanced' toggle" (Rules UX) was superseded by the String/Regex toggle.
- **D8. The TypeScript placement type doesn't match Rust.** `OverlayPlacement` in `app/src/settings/model.ts` is `{monitorLeft, monitorTop, x, y, width}`; Rust's is `{fx, fy, width, edge?}`. It only works because nothing reads anything but `width`.
- **D9. The overlay page's meter default differs from the docs.** `app/src/overlay/main.ts:13` falls back to "smooth" when the URL has no `meter`; the documented default is "stepped". Rust always passes the parameter, so only a bare dev load differs.
- **D11. FINDINGS #8 describes the old window styles.** It records overlays as permanently `WS_EX_TOPMOST | WS_EX_LAYERED`. Since 2026-09-30 they are owned (not topmost while owned) and layered only while click-through (#14). Add a note rather than rewriting the measurement. FINDINGS #15 didn't cover owned overlays (H6).

## Dead code

- **Unused parts of `model.ts`:**
  - `emptyLayer`, `layerRefEquals` and `resolveBaseField` are never used.
  - `resolve()` computes `ownName`, `ignoreOwnMessages`, `ignoreSystem`, `systemSenders`, `ignoreSenders`, `alwaysSenders` and `delivery`, which nothing reads.
  - Its sender lists replace rather than merge as Rust does, so they would mislead anyone who started using them.
- **`Dialog.svelte`'s prompt mode** (`mode`, `message`, `initialValue`, `placeholder`, its own input): every caller uses `children` in confirm mode.

## Shared code and streamlining

### Backend

- **R1. One driver for the CLI and the app.**
  - `cli/src/main.rs` (the main loop) and `app/src-tauri/src/runner.rs` (`run_loop`) are the same loop: sample every 250 ms; every 500 ms `observe_clients` and `tick`; then `route` and `governor.apply`.
  - A core `Driver` that owns the Engine, Sampler, Governor and RouterConfig and yields events and routed alerts removes the copy, and makes that orchestration testable. Nothing tests it today.
- **R5. Monitor helpers.** `runner::monitor_rects` and `runner::primary_rect` each walk Tauri's monitor list and convert it, and both wait on the UI thread. `overlay::scale_for` already reads Windows directly (`winapi::scale_at`); do the same for these two with one helper.
- **R7. Shared test helper.** The UTF-16 `append` helper is copied into the tailer, liveset and engine tests; move it into `logfmt::testutil`.
- **R8. Smaller duplicates.**
  - FNV hashing is written twice (`runner::accent_for`, `toast::short_tag`).
  - UTC clock formatting is written twice (`winapi::ts`, the CLI's `clock`); it belongs in `time.rs`.

### Frontend

- **F1. A `Field` component for `ChannelRow.svelte`.** Label, pip, control and "↺ use default" repeat five times. A `Field` component (label, own, showDeviation, onrevert, and the control as a snippet) covers all five.
- **F2. One segmented control.** Mode (ChannelRow), String/Regex (TrackedListSection) and Alert/Ignore/Normal (SenderListSection) are three hand-built copies of one `Segmented` component.
- **F3. A shared chip list.** `TrackedListSection` and `SenderListSection` have the same structure:
  - a chip list with "+ Add";
  - an add/edit Dialog and a remove confirmation;
  - a focus-on-open `$effect`, which Dialog already does;
  - the same inline-styled "×" button.

  A `ChipList` component can hold all of that; each section keeps only its chip content and dialog body.
- **F4. A `NavRow` component for the sidebar.** `Settings.svelte` has five copies of the nav button with inline `style="width:100%;text-align:left"` and inline SVG; replace them with a `NavRow` component and a CSS class.
- **F5. Shared overlay pieces.** Panel, Beacon and Strip each carry the same lifetime meter and count badge markup; make them `Meter` and `CountBadge` components.
- **F6. Consistent error styling and one hotkey default.**
  - `ChannelsPage` colors its error notes with inline `style="color:var(--danger)"` three times, where `GeneralPage` uses the `section-note error` class; use the class everywhere.
  - "Ctrl+Alt+O" is hard-coded twice in `GeneralPage`; have Rust send the default.
- **F7. Events instead of polling.** Settings polls every 4 s (all settings and pilots) and About every 5 s (update status). Only the pilots, who's online and the update status change outside the window, so push those from Rust as events. That fixes L2 and most of M2's cost.
- **F8. Generated types.** `model.ts`, `overlay/types.ts`, and the inline `UpdateStatus` (AboutPage) and `Locations` (GeneralPage) types copy Rust structs by hand, and D8 has already drifted. Generate them from Rust (e.g. `ts-rs` or `specta`) during the build.
