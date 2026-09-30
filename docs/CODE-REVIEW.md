# Code review (2026-09-30)

A review of the whole codebase before the next release, at commit `61a9066` on `dev`. `core/`, `app/src-tauri/`, `app/src/`, `cli/` and the workflows were read in full. `tools/` was only skimmed (dev-only probes), and the CSS wasn't reviewed in depth. Tests, clippy and a build were not run for this review.

Line numbers refer to that commit; function names are given so each spot can still be found once the code moves. Each item says what is wrong, how it goes wrong, and the fix. Like BACKLOG.md, this holds open items only: remove an item when it's fixed (git history keeps it), and delete this file when nothing is left. Measurements go to FINDINGS.md and decisions to DESIGN.md.

## Plan (agreed with the owner 2026-09-30)

- **Before the next release:** H1, H2, H3, H4, H5, M1, M4, M5, plus the H6 measurement.
- **Owner decision needed for H4:** are own-name mentions (and private messages?) exempt from rate caps?
- **Everything else:** after the release.

## High

### H1. Startup warnings set up WinRT as multithreaded on the UI thread, which can crash the first window

- **Where:**
  - `app/src-tauri/src/lib.rs:396-402` (`setup`)
  - `app/src-tauri/src/toast.rs:139-143` (`WINRT`)
  - `toast.rs:293-303` (`plain`)
- **What:** `setup` runs on the UI thread. After `toast::init`, it calls `toast::plain` when the reposition hotkey couldn't be registered or the portable folder isn't writable. `plain` starts with `WINRT.with(..)`, which calls `RoInitialize(RO_INIT_MULTITHREADED)` on whatever thread calls it. `toast::init`'s own comment (`toast.rs:148-152`) records what that does on the UI thread: the next window creation failed with RPC_E_CHANGED_MODE, which panicked the app when the settings window opened.
- **Triggers:**
  - another app holds Ctrl+Alt+O;
  - the saved hotkey text doesn't parse;
  - settings.json doesn't parse (the hotkey read at `lib.rs:393` falls back to "", then to the default with a warning);
  - the portable `data` folder is read-only.
- **Fix:** nothing may touch a COM or WinRT apartment on the UI thread. Give notifications one dedicated thread fed by a channel (as `audio.rs` does for sound). `init`, `chat`, `fold` and `plain` then only post to it, so no caller can set up the wrong apartment.

### H2. An unreadable settings.json or pilots.json is replaced by defaults, then overwritten

- **Where:**
  - `app/src-tauri/src/runner.rs:176-184` (`run`)
  - `core/src/settings.rs:401-408` and `core/src/pilots.rs:292-299` (`save`)
- **What:** when a file won't parse, the app starts with defaults and says so only through `eprintln!`, which release builds have no console to show.
  - **pilots.json:** the first tick registers every session from the 14-day window, and `PilotInLogs` / `NewPilot` call `save_pilots`. The file is overwritten within about half a second. Tags, placements, known channels, corp/alliance and first-seen dates are lost, and every character with recent activity gets a "New character" notification.
  - **settings.json:** overwritten by the next autosave from the Settings window, losing every layer.
- **Causes:**
  - `save` writes a temp file and renames it without flushing it to disk (`File::sync_all`), so a crash or power loss can leave an empty or partial file;
  - a downgrade: an older build reading a file with an enum value it doesn't know (a new `Mode`, `DeliveryMode` or `ChannelKind` key) rejects the whole file;
  - hand edits.
- **Fix:** one persistence helper in core (see R2) that:
  - writes the temp file, flushes it, renames it, and keeps the previous good file as `.bak`;
  - on a parse failure, renames the bad file to `<name>.corrupt-<stamp>` (never overwrites it), falls back to `.bak`, and reports the problem to the app;
  - is paired with a notification in the app naming the saved copy.

  Also consider accepting unknown enum values as the default, so a downgrade loses one field rather than the whole file.

### H3. A late second copy of a merged line is never evaluated for its character

- **Where:** `core/src/merge.rs:65-75` (`Merger::push`) and `core/src/engine.rs:193-206` (`tick`).
- **What:** in a channel two characters follow, the first copy (character A) is held 750 ms, then emitted and evaluated only for `seen_by` = [A]. B's copy arriving afterwards matches the emitted entry (same channel, sender and text, within ±2 s, different character). It is added to `seen_by`, and `push` returns `None`, so B's rules never run.
- **Example:** Bob writes "Naomi, dock up" in Local. Holden's copy is emitted and doesn't match Holden's rules. Naomi's copy is read about 1.2 s later, and her mention never alerts.
- **When:** B's copy has to be read after the hold has run out, roughly more than 1 s behind A's (500 ms ticks, 750 ms hold). FINDINGS #3 has copies stamped a second apart (2.2% of lines), but nobody has measured how late copies actually arrive. A starved background client (FINDINGS #15) is the likely case.
- **Fix:**
  - When a copy joins an entry that was already emitted, hand it back as a late line for that character only.
  - The engine evaluates it for that character and emits an alert if it's a target, skipping characters already alerted for the line, so a keyword both matched isn't shown twice.
  - Add an engine test with the second copy arriving one tick after the hold.
  - Measure the arrival lag between two clients and record it in FINDINGS.

### H4. Rate caps swallow mentions, and a fold with nothing to fold into is a drop

- **Where:**
  - `core/src/governor.rs:36-57` (`admit`)
  - `core/src/settings.rs:179-206` (`with_defaults`)
  - `app/src-tauri/src/runner.rs:386-390`
  - `app/src/overlay/Overlay.svelte:46-49` (`fold`)
  - `app/src-tauri/src/toast.rs:277-290` (`fold`)
- **What:** caps count every alert per character, whatever the reason. The defaults are Fleet, Corp and Alliance on Everything at 6/min, Local at 6/min and public channels at 4/min, all set to fold.
  - The seventh alert in a minute (for example "Holden, you're anchor" in a busy fleet) becomes `Limited(Fold)`: no Beacon, no notification, and no sound, since sound only follows `Deliver`.
  - The overlay page bumps the count on a visible alert with the same character and channel id. If that alert has already expired (Panel 9 s, Strip 6 s), nothing happens.
  - `toast::fold` only updates a notification sent within the last 2 min, and only if that alert went to a notification.
  - So a fold is often a silent loss.
- **Fix:**
  - The owner decides whether own-name mentions (and private messages?) skip caps.
  - Either way, a fold must never lose the line: with nothing showing to fold into, show it (a Strip carrying the count). That needs memory of what's currently shown, which DESIGN already places in the overlay manager ("burst folding").

### H5. Lock order and blocking cross-thread Win32 calls can deadlock the app

- **Where:**
  - `app/src-tauri/src/runner.rs:239-248` (`publish_pilots`)
  - `app/src-tauri/src/overlay.rs:290-350` (`show`; `is_visible` at 331)
  - `overlay.rs:397-452` (`enter_reposition`)
  - `overlay.rs:580-592` (`set_owner_cloaked`)
  - `overlay.rs:832-876` (`set_click_through`, `show_without_activating`, `set_owner`)
  - the synchronous commands in `lib.rs`
  - `app/src/settings/Settings.svelte:87-89`
- **The cycle:**
  1. The runner, in `publish_pilots`, holds `engine`, then `status`, then takes `slots` through `window_count()`.
  2. The reposition-toggle thread (or the client-moves thread when a client is uncloaked) holds `slots` while calling `SetWindowLongPtrW`, a non-async `SetWindowPos` or `ShowWindow` on an overlay. The UI thread owns those windows, so these calls send messages and wait for it.
  3. The UI thread is inside a synchronous Tauri command, since those run on the main thread, that takes `engine`. `get_settings_data` does this every 4 s from the Settings page, even while the window is hidden.

  The UI thread waits on the runner, the runner waits on the toggle thread, and the toggle thread waits on the UI thread. A Rust `Mutex` wait doesn't pump messages, so it never resolves: tray, overlays and Settings all freeze (see also H6).
- **Also:** `show()` calls `window.is_visible()`, a Tauri getter that waits on the UI thread, while holding `slots`, only to print a log line.
- **Fix (all three):**
  1. Make no Win32 call that waits on the UI thread while holding `slots` or `sessions`. Collect the work under the lock, release it, then act. Or use the async forms (`SWP_ASYNCWINDOWPOS`, `ShowWindowAsync`) and make the extended-style changes on the UI thread through `run_on_main_thread`, with no lock held.
  2. Never take `slots` while holding `engine` or `status`. Remove `publish_pilots` and `get_status`, which are unused (see Dead code).
  3. Make every command that takes an app mutex `async`, so the UI thread never waits on one.

  Write the lock order down in `state.rs`.

### H6. Cross-process owned overlays may tie our UI thread's input queue to EVE's (needs a measurement)

- **Where:** `app/src-tauri/src/overlay.rs:869-876` (`platform::set_owner`), and DESIGN "Overlays belong to their EVE client".
- **What:** making EVE's window the owner of our overlay creates a cross-process owner/owned relationship, and Windows then attaches the input queues of the two threads (Raymond Chen, "Is it legal to have a cross-process parent/child or owner/owned window relationship?", The Old New Thing). If that holds here, EVE's input on that client can stall whenever our UI thread stops pumping messages: H5, WebView2 building a window (0.4 s cold), or any long command. Keyboard and focus state are shared too.
- **Gap:** FINDINGS #15 measured frame times with `--selftest` / `--soak`, whose overlays are created with `owner: None` (`testalerts.rs:61`). The owned path has never been measured.
- **Measurement:**
  1. With an overlay owned by a client, block our UI thread for about 5 s on purpose (a debug-only trigger) and check whether EVE's camera and keyboard stay responsive.
  2. Repeat the PresentMon capture with real, owned alerts.
  3. Record both in FINDINGS.

  If input does stall, the owner decides between keeping ownership (and guaranteeing the UI thread never blocks) and going back to topmost windows with our own z-order and cloak handling.

## Medium

### M1. With no Chatlogs folder at startup, the engine is never built and nothing retries

- **Where:** `app/src-tauri/src/runner.rs:185`, and the "Still starting up" errors in `lib.rs`.
- **What:** a new user who installs before turning on "Log chat to file" in EVE:
  - sees "Still starting up — try again in a moment." in Settings forever, since every command needs the engine;
  - can't change the hotkey;
  - turns logging on later, and the app doesn't notice until it restarts.
- **Fix:**
  - Build the engine (settings, pilots) regardless of the folder.
  - Make the log folder a separate state that the runner re-checks (every ~10 s) and starts following when it appears.
  - Have Settings say it's waiting for EVE's chat log folder instead of showing an error.

### M2. The Settings window is never destroyed, and it polls while hidden

- **Where:** `app/src-tauri/src/lib.rs:294-306` (`open_settings`), `app/src/settings/Settings.svelte:87-89`.
- **What:**
  - Closing hides the window so it reopens instantly, but its WebView2 stays alive for the rest of the session. That keeps the whole WebView2 process tree alive, so after opening Settings once the app never gets back to its ~6 MB idle footprint (DESIGN memory rule, FINDINGS #8).
  - Its 4 s `get_settings_data` poll keeps running while hidden, serializing all settings and the pilot registry on the UI thread (and feeding H5).
- **Fix:**
  - Destroy the window after it has been hidden for a while (e.g. the overlays' 45 s).
  - Stop polling while hidden.
  - Replace polling with events from Rust (F7).

### M3. One chat line can produce one notification or overlay per character

- **Where:** `core/src/router.rs:144-173` (`route`), `app/src-tauri/src/runner.rs:311-393` (`deliver`).
- **What:** each target is routed on its own.
  - With no client focused (browser, Discord, the Settings window), a Defaults keyword such as "@all" in a Local both characters are in gives two identical notifications.
  - With Suppress set to Never, it gives two overlays on the same screen.

  This undoes the cross-character merge. DESIGN's "the router can use `seen_by` to know which screens already showed the line" was never built.
- **Fix:** group deliveries per alert: one notification or overlay per destination, listing every character it's for, with the strongest reason and style. Keep suppression and caps per character.

### M4. The Tracked "Add" dialog defaults "Still alert even when muted" to off

- **Where:** `app/src/settings/TrackedListSection.svelte:57-64` (`openAdd` sets `dialogEvenWhenMuted = false`).
- **What:** the default contradicts:
  - the section's own note ("Checked under every mode above, including 'Nothing' by default");
  - `TrackedRule::even_when_muted`, which defaults to true;
  - DESIGN.

  A keyword added to watch a muted trade channel silently never fires.
- **Fix:** default the dialog to true, after the owner confirms that's the intended default.

### M5. One invalid regex blocks every later save

- **Where:** `app/src-tauri/src/lib.rs:52-66` (`save_settings`); `TrackedListSection.svelte` doesn't validate.
- **What:**
  - `save_settings` refuses the whole settings object when any pattern fails to compile.
  - The page stays "dirty", the periodic reload stops, and every later edit fails to save as well. Quitting from the tray loses all of them.
  - The chip doesn't show which entry is the bad one.

  `Settings::resolve` already skips bad patterns, so refusing everything isn't needed for safety.
- **Fix:**
  - Validate in the dialog with the Rust regex engine: add a `check_regex` command, since JavaScript's RegExp syntax differs.
  - Refuse to add a bad pattern there.
  - Keep the check in `save_settings` as a guard, and mark the bad chip.

### M6. "Switch to" stops working on notifications older than 2 minutes

- **Where:** `app/src-tauri/src/toast.rs:118` (`BURST`), `toast.rs:261` (`bursts.retain`).
- **What:** the `ToastNotification`, and with it its click handler, is dropped when another chat notification arrives more than 2 min later. Sticky mentions wait on screen or in Action Center, and the away case is exactly when someone clicks late. The click then does nothing.
- **Fix:** keep shown notifications alive independently of the burst window (e.g. the last N by tag), or register a COM activator (the limit noted in FINDINGS #12).

### M7. A quick double press of the reposition hotkey can leave an interactive box over the game

- **Where:** `app/src-tauri/src/reposition.rs:25-37` (`toggle`), and `lib.rs:322-328`, which starts a new thread per press.
- **What:**
  1. Press 1 sets the flag and starts `enter`; building the window takes ~0.4 s cold.
  2. Press 2 clears the flag and runs `exit`, which finds no sessions yet.
  3. `enter` then finishes and adds its sessions with click-through off.

  The box sits over the game taking clicks while the app thinks reposition mode is off, and one more press is needed.
- **Fix:** run toggles on one worker fed by a channel, or hold a toggle mutex for the whole enter or exit.

### M8. The rate cap field suggests a character can raise the Defaults cap

- **Where:** `app/src/settings/ChannelRow.svelte:96-113`.
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
- **L2. A reload can overwrite an edit.** `app/src/settings/Settings.svelte:28-35, 87-89`.
  - The 4 s reload checks `dirty` before it asks. An edit made while `get_settings_data` is in flight is replaced by the older response, and the debounced save then writes that older state.
  - Fix: an edit counter, so a response is dropped if an edit happened since it was requested. Or remove the poll (F7).
- **L3. Own-name matching is a plain substring.** `core/src/rules.rs:139-142` (`mentions_own_name`).
  - Short names over-match ("Ash" in "wash"), and a first-name mention ("Naomi" for "Naomi Nagata") never matches.
  - Owner question: word boundaries, and whether first names count.
- **L4. `drop_copies_of_defaults` runs on every load.** `core/src/settings.rs:378-398`.
  - It was meant as a one-time migration, but it keeps erasing any character entry equal to Defaults'. A later change to Defaults then reaches a character that had deliberately set the same value.
  - Fix: a settings format version, with migrations run once.
- **L5. The settings cache grows with Fleet and Private ids.** `core/src/settings.rs:436-444` (`SettingsBook::resolved`).
  - The cache key includes the channel id although those kinds never use it, so every new fleet or conversation adds an entry with compiled regexes, until the next edit clears the cache.
  - Fix: use "" as the channel id in the key for kinds without stable ids.
- **L6. `last_seen` doesn't mean last active.** `core/src/engine.rs:246-248`, `core/src/pilots.rs:229-233`.
  - It's bumped every tick for every followed session (any log from the last 14 days), and only saved when pilots.json is saved for some other reason.
  - The "last active N days ago" caption and the planned auto-cleanup (BACKLOG) both need real activity: bump it when a line is seen.
- **L7. `follow()` asks Explorer for each overlay's desktop four times a second while holding the slots lock.** `app/src-tauri/src/overlay.rs:518-533`.
  - `GetWindowDesktopId` is a cross-process COM call, so a stalled Explorer stalls the runner while it holds `slots`.
  - Fix: re-pin on cloak events (already hooked) and on a slow timer.
- **L8. The CSP is null and every command is open to every window.** `app/src-tauri/tauri.conf.json:15-17`, `capabilities/default.json`.
  - Overlays render other players' chat. Svelte escapes it and there's no `{@html}`, but a strict CSP and per-window command permissions (an app manifest in `build.rs`) would contain a future slip.
  - Overlays need only `overlay_ready` and the four reposition commands.
- **L9. `reg.exe` and `explorer` are started by name.** `app/src-tauri/src/toast.rs:189-202`, `lib.rs:183`.
  - Windows searches the exe's own folder first.
  - Fix: use `RegSetKeyValueW` (lib.rs already uses the registry API) and the full path to explorer.exe. This also removes two process launches on every start, which currently block the UI thread because `toast::init` joins its thread.
- **L10. The whole folder is rescanned every 5 s.** `core/src/engine.rs:44`, `core/src/liveset.rs:185-243`.
  - Long-time players have tens of thousands of files. FINDINGS #1 shows Create events are timely, and DESIGN planned to use them for discovery.
  - Fix: discover through events, keeping a slow rescan as the fallback.
- **L11. Smaller overlay issues.**
  - An idle window can be reaped between `enter_reposition`'s existence check and its lock, and that character gets no box (`overlay.rs:400-437`).
  - After reposition mode ends, the window stays box-sized, and `follow_client` re-places it from the old layout until the next alert (`overlay.rs:545-574`, `596-613`).
  - `label_for` gives two names the same label when they differ only in punctuation (only `name:` keys), and `key_from_label` doesn't reverse it (`overlay.rs:245-251`).
- **L12. Control characters in toast XML.** `app/src-tauri/src/toast.rs:53-55` (`xml_escape`).
  - They're invalid in XML 1.0, so a line containing one fails `LoadXml` and the notification is dropped with only a log line.
  - Fix: strip them in `xml_escape`.
- **L13. `pick_sound_file` blocks a runtime worker while its dialog is open.** `app/src-tauri/src/lib.rs:136-146`.
  - It calls `blocking_pick_file` inside an async command.
  - Fix: use the callback form.

## Docs out of step with the code

- **D1. Away mode is live, but the docs and the UI say it isn't.**
  - The router treats 5 min without input as away (`router.rs:145`, `RouterConfig::idle_after`), and `Sampler` measures idle time with `GetLastInputInfo`.
  - BACKLOG "Away mode" says `route` "takes an away input; nothing sets it yet (always false)", and that the focused character's alerts stay suppressed while away.
  - The General page shows the setting as "coming later" (`GeneralPage.svelte:91-94`).
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
- **D8. The TypeScript placement type doesn't match Rust.** `OverlayPlacement` in `app/src/settings/model.ts:131-138` is `{monitorLeft, monitorTop, x, y, width}`; Rust's is `{fx, fy, width, edge?}`. It only works because nothing reads anything but `width`.
- **D9. The overlay page's meter default differs from the docs.** `app/src/overlay/main.ts:13` falls back to "smooth" when the URL has no `meter`; the documented default is "stepped". Rust always passes the parameter, so only a bare dev load differs.
- **D10. A comment claims the UI reads `repositioning`.** `app/src-tauri/src/state.rs:59-61` says the tray and Settings read it; only `reposition.rs` does.
- **D11. FINDINGS #8 describes the old window styles.** It records overlays as permanently `WS_EX_TOPMOST | WS_EX_LAYERED`. Since 2026-09-30 they are owned (not topmost while owned) and layered only while click-through (#14). Add a note rather than rewriting the measurement. FINDINGS #15 didn't cover owned overlays (H6).

## Dead code

- **The `get_status` command and `publish_pilots`.** The command, most of `Status` (`pilots`, `alertsShown`, `overlayWindows`; only `testalerts` reads `status.pilots`) and `Runner::publish_pilots` (`lib.rs:26-29`, `state.rs:20-29`, `runner.rs:239-248`). Removing them also removes the engine → status → slots lock nesting in H5.
- **`settings::Resolved::bad_regexes`** (`settings.rs:168-169`): only a test reads it.
- **`runner::notify`** takes an `AppHandle` it ignores (`runner.rs:144-147`).
- **Unused parts of `model.ts`:**
  - `emptyLayer`, `layerRefEquals` and `resolveBaseField` are never used.
  - `resolve()` computes `ownName`, `ignoreOwnMessages`, `ignoreSystem`, `systemSenders`, `ignoreSenders`, `alwaysSenders` and `delivery`, which nothing reads.
  - Its sender lists replace rather than merge as Rust does, so they would mislead anyone who started using them.
- **`Dialog.svelte`'s prompt mode** (`mode`, `message`, `initialValue`, `placeholder`, its own input): every caller uses `children` in confirm mode.

## Shared code and streamlining

### Backend

- **R1. One driver for the CLI and the app.**
  - `cli/src/main.rs:235-313` and `app/src-tauri/src/runner.rs:208-237` are the same loop: sample every 250 ms; every 500 ms `observe_clients` and `tick`; then `route` and `governor.apply`.
  - A core `Driver` that owns the Engine, Sampler, Governor and RouterConfig and yields events and routed alerts removes the copy, and makes that orchestration testable. Nothing tests it today.
- **R2. One persistence helper.** `Settings::load/save` and `PilotRegistry::load/save` are the same code. One helper is where H2's fix lives (flush, backup, quarantine).
- **R3. Engine access in `lib.rs`.**
  - Six commands repeat the lock and the "Still starting up" error.
  - `config_dir().join("settings.json" / "pilots.json")` appears in 8 places (`lib.rs`, `runner.rs`, `reposition.rs`).
  - Add `AppState::with_engine(|e| ..)` (async-friendly, for H5), `storage::settings_path()` / `pilots_path()`, and a single `save_pilots`.
- **R4. `overlay.rs` duplicates.**
  - `show()` and `enter_reposition()` repeat the create-outside-the-lock, insert-if-absent steps and the `Slot {..}` literal; make them one `ensure_slot()`.
  - `show()` re-implements `deliver()` inline.
- **R5. Monitor helpers.** `runner::monitor_rects`, `runner::primary_rect` and `overlay::scale_for` each walk Tauri's monitor list and convert it. One `monitors()` returning the rect and scale covers all three.
- **R6. Compile each regex once.** `Settings::resolve` compiles every pattern twice: once to check it, then again in `CompiledRules::compile`. Compile once and collect the failures.
- **R7. Shared test helper.** The UTF-16 `append` helper is copied into the tailer, liveset and engine tests; move it into `logfmt::testutil`.
- **R8. Smaller duplicates.**
  - FNV hashing is written twice (`runner::accent_for`, `toast::short_tag`).
  - UTC clock formatting is written twice (`winapi::ts`, the CLI's `clock`); it belongs in `time.rs`.
  - `testalerts` looks up a pilot id by name from `status.pilots` twice.

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
- **F7. Events instead of polling.** Settings polls every 4 s (all settings and pilots) and About every 5 s (update status). Only the pilots, who's online and the update status change outside the window, so push those from Rust as events. That fixes L2, most of M2's cost, and one side of H5.
- **F8. Generated types.** `model.ts`, `overlay/types.ts`, and the inline `UpdateStatus` (AboutPage) and `Locations` (GeneralPage) types copy Rust structs by hand, and D8 has already drifted. Generate them from Rust (e.g. `ts-rs` or `specta`) during the build.
