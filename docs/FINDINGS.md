# Findings: measured facts and how to re-check them

Everything here was measured on the project owner's machine (Windows 11, two 1920x1080 monitors, Documents redirected into OneDrive, two clients: Holden and Naomi Nagata) unless stated. Probes live in `src/bin/`; shared helpers in `src/winutil.rs`. Logs: `probe-log.txt`, `focus-log.txt`, `overlay-log.txt` (gitignored).

## 1. Directory events are unreliable for live log growth; polling works

- Synthetic writer (tool: `synth`, 20 one-per-second appends, events only):

| Where | Write mode | Events | Writes seen within 2 s |
|---|---|---|---|
| Local NTFS (D:) | plain | 3 | 2 of 20 |
| Local NTFS (D:) | flush after each write | 22 | 20 of 20 |
| Local NTFS (D:) | plain + 500 ms poll | 22 | 20 of 20 |
| OneDrive | plain | 11 | 9 of 20 |
| OneDrive | flush | 22 | 20 of 20 |
| OneDrive | plain + 500 ms poll | 24 | 20 of 20 |

- Real EVE files: `probe --no-poll` got **0 events in 120 s** while chatting between two characters; with polling on, 10 lines in 30 s, all delivered within the same second.
- So it is NTFS write caching (size changes are reported when the cache flushes), not OneDrive. Documented Win32 behavior for `FILE_NOTIFY_CHANGE_SIZE`. Independent confirmation: py-eve-chat-mon's README reports the same and polls; the old EveChatNotifier polls file sizes.
- Consequence: poll the live set (~500 ms) by opening the file and reading from a saved offset. Events are at most a hint.
- `Create` events for new files are timely (measured 2026-09-30, `probe --no-poll`, OneDrive Documents): a character login (Local + Corp) and joining a new channel each produced a Create event in the same second as the filename's start time, with the header already readable (~600 bytes). Growth still needs polling; discovery of new files can rely on events (polling the directory remains the fallback).

## 2. Poll cost and scale (debug build, local disk)

| Case | Files | Live pairs polled | Startup scan | Steady CPU |
|---|---|---|---|---|
| Real folder | 44 | 14 | 0 ms | 1.6 ms/s (0.2% of a core) |
| Synthetic | 5,000 | 200 | 15 ms | 11.7 ms/s (1.2% of a core) |

Cost scales with polled (character, channel) pairs, about 0.05 ms/s each, not with total file count. Not measured: release build, long soak.

OneDrive cloud-only files are never downloaded by the app (2026-09-30, release build): seven logs made cloud-only with `attrib +U -P` (three the newest for their character and channel within the 14-day live window, so ones it would follow; four superseded) stayed cloud-only through startup and 40 s of rescans. Control: one direct read downloaded a file at once, so the check would have caught it. `liveset` reads the cloud flags (`OFFLINE`, `RECALL_ON_OPEN`, `RECALL_ON_DATA_ACCESS`) from the directory listing and never opens such a file. Restored afterwards (`attrib -U`, then `-P` to drop the pin).

## 3. Log format

- Filename: `<Channel>_<YYYYMMDD>_<HHMMSS>_<characterId>.txt`. Stamp is **UTC** (verified against file creation time at UTC-4). Channel names can contain spaces and parentheses (`Private Chat (2)`); private-chat channel ids are GUID-like.
- One file per login session per channel. **Local does not rotate on system jumps.** A new login creates new files.
- Encoding UTF-16LE with a BOM at file start **and a BOM before every line**. Lines end with CRLF. In-line timestamps `[ YYYY.MM.DD HH:MM:SS ]` are UTC.
- Header order: `Channel ID`, `Channel Name`, `Listener`, `Session started`; header keys were English on this (English) client. Localization is unverified.
- Two characters in the same channel write the same lines to separate files; a line near a second boundary can carry stamps one second apart in the two files (seen for Jita Local). Cross-character dedupe must tolerate this.
- Channel ids carry a prefix that identifies the kind (headers of real logs): `local`, `corp`, `fleet_1368512310460` (Fleet; a new id per fleet), `private_<32 hex>` (private chat, named "Private Chat (N)", a new id per conversation), `system_263238_263361` (public channel; "EVE University", the id is stable and the header name matches the in-game name). Alliance: id `alliance`, name `Alliance`, file `Alliance_20260925_121918_496528567.txt` (confirmed from an alliance member's log header; the owner is not in an alliance). Unknown prefixes classify as `Unknown` (see `core/src/channel.rs`). Fleet and private ids are ephemeral, so only their kind can be configured; the other kinds have stable ids.
- The old EveChatNotifier parsed the channel id and never used it; it matched channels by name only.
- Login MOTD lines come from sender `EVE System`.
- Right after the header, EVE writes `EVE System > Channel changed to Corp : <corporation name>` (owner-confirmed 2026-09-30, also seen in #6's Corp headers), then the MOTD line. A new log starts every login, so this names each character's corp per session. The header's `Channel ID` is just `corp` for everyone, so this line is the only way to tell corps apart. Assumed the same shape for Alliance (`Channel changed to Alliance : <name>`); not yet seen in a real alliance log. The app reads it into `Header::instance` from the top of the file and again if it appears mid-session.
- Pilot join/leave and arrivals are not logged.

## 4. Focus tracking

- `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` (out of context) reported 17 of 17 focus gains on EVE clients across two clients, alt-tab, clicks and virtual-desktop switches, within ~100 ms, no admin rights.
- Windows 11's alt-tab UI (`XamlExplorerHostIslandWindow`, title "Task Switching") can post a stray last event 2-3 ms after the real destination (5 of 5 non-EVE landings; 0 of 17 EVE landings). Treat the hook as a wake-up and `GetForegroundWindow()` as truth; ignore explorer shell windows (`ForegroundStaging`, `Shell_SecondaryTrayWnd`, `VirtualDesktopHotkeySwitcher`, `Task Switching`).
- EVE client: exe `exefile.exe`, class `trinityWindow`, title `EVE - <Character>` (and plain `EVE` before a character is chosen, unverified). Character is readable from the title without elevation.
- Client state: `MINIMIZED` via `IsIconic`; other virtual desktop via `DWMWA_CLOAKED == 2`. `IsWindowVisible` stays true in both cases. Exclusive Fullscreen minimizes the client when it loses focus.
- UWP shell apps (Search, Start) show up as their own exe, not `ApplicationFrameHost.exe`, in this data.

## 5. Overlays

- A topmost, click-through (`WS_EX_TRANSPARENT`), non-activating (`WS_EX_NOACTIVATE`) layered window appeared over EVE clients in **Fixed Window** and over a **Fullscreen** client on both monitors, without stealing focus (`focus kept: YES` in every trial), minimizing anything, or blocking mouse input (owner confirmed camera control through it).
- Overlays are monitor-anchored (screen coordinates), not attached to the game window. An overlay still appeared on the monitor of a minimized Fullscreen client.
- `SHQueryUserNotificationState` is `BUSY` while an EVE client is focused in most trials but not all (Fixed Window Holden gave `ACCEPTS_NOTIFICATIONS` once, `BUSY` earlier). It is not a reliable indicator of display mode; use it only as a hint about whether a system toast would show.
- EVE display modes: Windowed (ordinary), Fixed Window (borderless-like, stays up unfocused, spans monitors), Fullscreen (exclusive; hides when unfocused). It looks like Windows still composites Fullscreen (banner drew over it), which may not hold on every GPU/driver.

## 6. Reference footprint: dev-prompt (Tauri, tray + hotkey overlay)

`dev-prompt.exe` 36.6 MB working set (10.7 MB private). Its WebView2 tree, 6 processes for one hidden window: about 304 MB working set, roughly 230 MB private. Single warm window created at startup and shown/hidden. This is a full app, not a minimal overlay; the minimal-overlay number is still to be measured.

## 7. End-to-end routing check (headless core + presence + router)

Setup: `tools/scripts/feed.ps1` writes synthetic logs (never the real folder) for two characters named like the owner's live clients ("Holden", "Naomi Nagata") in one shared channel, appending a line every 4 s with the second copy stamped one second later; `chatter --dir <temp> --keyword chatterer-test` reads them while presence samples the real windows. 150 s run, 32 alerts, the owner moving between focus states by hand.

Result: all 32 decisions matched the presence state at that moment. Focused pilot suppressed and the other one got an overlay on its own monitor (Holden left, Naomi right, from real window geometry); browser focused gave overlays for both; both clients on another virtual desktop gave toasts for both. Each line produced one alert (32 lines, 32 alerts) carrying both characters in `seen_by`. Alerts printed within about 1 s of the write (includes the 750 ms merge hold).

Not exercised: alt-tab held longer than the 1.5 s grace with an alert firing during it, and the 5-minute away path (both unit-tested only). Fullscreen (minimize-on-unfocus) client not run through this yet.

## 8. The app's footprint (release build, Tauri v2, two monitors)

`eve-chatterer-app.exe --selftest` (fires overlays on both monitors, a burst, then waits), whole process tree sampled from outside:

| Stage | Processes | Working set | Private |
|---|---|---|---|
| Idle, before any alert | 1 | 21.7 MB | 2.5 MB |
| Overlays showing (2 windows) | 8 | 455-481 MB | 220-240 MB |
| Windows open, no alerts | 8 | ~476 MB | ~220 MB |
| After the idle teardown (45 s after the last alert) | 1 | 32.5 MB | 5.8 MB |

- The resident cost is the Rust core (about 6 MB private). The WebView2 tree exists only while alerts show plus 45 s; two windows cost about what dev-prompt's single window does (about 230 MB private), so a second monitor does not double it. The exe is 8.2 MB.
- CPU across the tree: about 25% of one core while alerts animate, about 1% with windows open and quiet.
- What the animating CPU goes to (2026-09-30, release `--soak` on both monitors, meter off, `EVE_CHATTERER_FX` switching one effect off per run, tree CPU sampled 12-55 s, two rounds): all on 7.8% / 5.8% mean (peaks 68% / 44% of one core); no shadows 5.3% / 8.7%; no arrival animation 9.1% / 7.7%; **no Beacon pulse 1.5% / 2.6%**; all three off 1.5% / 1.4%. The pulse is ~75-80% of it: it animates a 26 px blurred `box-shadow`, which the browser re-rasterizes every frame for 900 ms. Shadows and the arrival animation are inside the round-to-round noise. Fixed the same day: the glow is drawn once on its own layer and only its opacity/scale animate. Re-measured (three rounds): all on 4.0% / 4.3% / 5.9% mean, peaks 17-35% (were 44-68%); no pulse 3.2% / 2.9% / 1.7%. The remaining ~2 points are compositing the blurred layer while it fades.
- One window versus two, with per-client overlays and real alerts (2026-09-30, release build, two clients, whole tree sampled every 2 s):

  | State | Processes | Working set | Private | CPU (one core) |
  |---|---|---|---|---|
  | Idle, no overlay (fresh start / after a teardown) | 1 | 19 / 34 MB | 4 / 7 MB | ~0.7% |
  | First window being built (cold WebView2) | 7 | ~340 MB | ~150 MB | 60-85% for 2-4 s |
  | One window, alert showing then quiet | 7 | 360-370 MB | ~147 MB | 0-1.5% |
  | Two windows, quiet | 7-8 | 420-494 MB | 194-227 MB | 0-1.5% (brief 10-15% on a new alert) |

  The second window costs about 50-80 MB private and ~60-130 MB working set, a fraction of the first (which pays for the whole WebView2 browser/GPU process set). Idle CPU doesn't change with the window count. Teardown still returns the tree to one process ~45 s after the last alert.
- Cold start (first alert of a session, WebView2 not running): first window built in 406 ms, page ready 496 ms after the alert asked for it; the second monitor's window (warm environment) 117 ms / 149 ms. Add the 240 ms arrival animation.
- Overlay windows verified as `WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED`, 520 x 640 centered on each monitor; the focus probe saw zero foreground events from the app while overlays showed.

## 9. Compositor cost of animated overlays over the game

GPU utilization from the `\GPU Engine(*)\Utilization Percentage` counters (must be sampled from the session itself; inside a background job they return invalid data), `--soak` keeping alerts on screen for about 50 s per condition, two passes in opposite orders:

| Condition | dwm.exe GPU (pass 1 / pass 2) | our app GPU | EVE (both clients) |
|---|---|---|---|
| Lifetime meter smooth (every frame) | 33.4% / 33.0% | 1.9% | no consistent change |
| Lifetime meter stepped (about 4 updates/s) | 22.5% / 23.6% | 0.3-0.6% | no consistent change |
| Lifetime meter off (static) | 22.0% / 23.5% | 0.1-0.3% | no consistent change |
| No overlay | 25.1% / 16.3% | 0 | (baseline) |

- Continuous animation makes the desktop compositor recompose the region under a topmost window every frame: about +10 points of dwm.exe GPU. Stepping the meter removes that; the overlay's own rendering is negligible (under 2% of the GPU). Stepped is the default (`EVE_CHATTERER_METER=smooth|stepped|off` overrides it for measurement).
- EVE's own GPU load swings by 10+ points on its own (54%, 75% and 67% baselines seen in different runs, a baseline stdev of 9.9), which is larger than any overlay effect we could resolve. GPU utilization showed no consistent effect on EVE; frame time was measured later (#15).
- Whether overlay presence alone costs the compositor anything is unresolved (stepped/off vs no overlay was -2.6 and +7.3 points, inside the noise). A cleaner test needs both clients in a static scene.

## 10. Live verification of the full app against real chat (2026-09-27)

With `eve-chatterer-app` running in `tauri dev` against the owner's real Chatlogs folder (not a synthetic feeder), each real chat line was checked against the app's console diagnostics (added this session: `runner.rs` logs every `Event::Alert` and its routing decision; `overlay.rs` logs window create/reuse/ready/emit).

| Test | Channel kind | Result |
|---|---|---|
| Own name typed by the other character | Local | Correct: Beacon, routed to the named pilot's monitor, suppressed for neither (typer was focused, target wasn't) |
| Private message | Private | Correct: fired on every line (mode `Everything`), Beacon |
| Fleet chat, no name/keyword in the text | Fleet | Correct once the default style was fixed (see below) |
| Corp chat, no name/keyword | Corp | Correctly **silent** — the two characters turned out to be in different corporations, so it never appeared in the other's log at all. Confirmed by reading both real Corp headers (`Channel changed to Corp : Sukebe Corporation` vs `... Probe Launcher Offline`). This is a true negative, not a miss: the engine only evaluates a line for pilots whose own log actually contains it. |

Two real bugs found this way, both fixed same day:
1. The very first alert test (a Local mention) produced no visible overlay, with no diagnostics yet in place to explain why; every later attempt worked once logging was added. Root cause unconfirmed — most likely a presence-sampling or cold-start artifact on that one instance, not a reproducible defect (see BACKLOG).
2. Fleet/Corp/Alliance defaulted to the `Strip` style (mode `Everything`'s reason maps to `Strip` in the router's style map), which a live Fleet alert showed was too subtle to reliably notice: 34px, 6 s, no animation beyond a fade. Changed their default style to `Panel` (9 s, larger, shows the reason). Beacon (mentions) was unaffected and remains the most visible tier.

Alliance itself was never observed directly (the owner is not in an alliance); its channel id (`alliance`) was confirmed separately from a third party's real log header (docs/FINDINGS.md #3), and its settings defaults are kept identical to Corp by a standing rule (docs/DESIGN.md), so its correctness rests on Corp's proven code path plus that one header confirmation, not on a live alert.

## 11. What `SHQueryUserNotificationState` sees (2026-09-29, Windows 11, `tools` bin `notifstate`)

Polled every 250 ms while the owner toggled each control on and off, several seconds apart:

| Toggled | Reported |
|---|---|
| Do Not Disturb (notification panel bell) | stayed 5 `ACCEPTS_NOTIFICATIONS` |
| Focus session (notification panel) | stayed 5 `ACCEPTS_NOTIFICATIONS` |
| EVE switched to Fullscreen | 2 `BUSY` for the ~8 s it was fullscreen, then back to 5 |

So this API (the app's `notifications_ok`) does **not** reflect Windows 11 Do Not Disturb or Focus. It does see a full-screen app, EVE included. Anything that must honor DND/Focus needs another source; this one alone would show a custom popup straight through DND.

Second run, same probe with WinRT `Windows.UI.Shell.FocusSessionManager` added (supported on the owner's Windows 11):

| Toggled | `IsFocusActive` |
|---|---|
| Focus session started, then stopped | ACTIVE for the session, then off |
| Do Not Disturb alone, on ~5 s then off | stayed off |

`FocusSessionManager` sees Focus sessions but **not** the plain Do Not Disturb bell, even though a Focus session itself turns DND on. Between the two documented sources, a manually switched-on DND is invisible to the app. (An earlier DND-then-Focus run looked like one continuous 10 s ACTIVE span; the DND-only rerun showed that span was the Focus session alone.)

## 12. Rich Windows notifications from an unpackaged build (2026-09-29, `tools` bin `toastprobe`)

Sent straight through WinRT (`ToastNotificationManager::CreateToastNotifierWithId`), not the Tauri plugin, from a plain `target\debug` exe, after registering an app identity under `HKCU\Software\Classes\AppUserModelId\<id>` (`DisplayName`, `IconUri`):
- The `appLogoOverride` image (a local PNG via `file:///`) showed; the owner confirmed. So did the `placement="attribution"` line.
- Clicking a `foreground` button raised `Activated` in the running process about 1 s after showing, with the button's `arguments` (`switch=Holden`).
- Unclicked, it went to the notification center after ~6 s (`Dismissed: TimedOut`).
- `Setting()` failed with "Element not found" on the first run just after registering the identity, and read `Enabled` on the next run.

Limit: `Activated` reaches only the process that sent the notification while it is running. A click after the app has quit does nothing unless the app is also registered as a COM activator for Windows to launch; not built or tested.

## 13. Acrylic ("frosted glass") behind overlays doesn't work (2026-09-30)

Tried `window_vibrancy::apply_acrylic` on the overlay windows (the crate dev-prompt uses) with the `--selftest` alerts, Windows 11. Result: **no blur at all**, just a flat grey fill behind the whole window. Windows 11 draws acrylic only for the active window and falls back to a solid color otherwise, and overlays are deliberately never active (they must never take focus). The fill also covers the whole window, not just the alert boxes (fixable by clipping the window region, as dev-prompt does, but moot). The older undocumented accent-policy blur ignores activation but is known to lag while windows move and could break with any update. Decision: keep the CSS tinted glass; frosted glass dropped.

## 14. Overlays at mixed DPI scaling (2026-09-30, release build, two clients)

Left monitor at 150%, right at 100%, one client on each. Alerts on the 150% monitor render sharp and 1.5x larger in the right spot; repositioning (move, resize) there saves and restores correctly; a client moved to the other monitor gets its alerts at that monitor's scale. Geometry is computed per monitor (`scale_for`) and needed no change.

Bugs found along the way (each only visible with two clients on screen, or after a resize), all fixed:
- Every overlay page listened with Tauri's global `listen`, which hears events emitted to *any* window, so each overlay also drew the other client's alerts and reposition box. Pages now listen on their own window.
- Keeping `WS_EX_LAYERED` on permanently (the 2026-09-30 click-through change, `platform::set_click_through`) made Windows hit-test the window at the size it had when it became layered: a box widened in reposition mode took the mouse only across its old width, the rest fell through to the game. Layered now toggles together with `WS_EX_TRANSPARENT`, as Tauri does.
- An alert shown over another character's client used the alerted character's box and width, not that client's; now the client's (DESIGN, "The box belongs to the client").

## 15. EVE frame times with overlays (2026-09-30, PresentMon 2.6, release build)

One PresentMon capture of `exefile.exe` while the conditions changed on a schedule: app off, `--selftest` windows open and quiet, `--soak` animating (all three styles on both monitors every 8 s), app off again. Two docked clients in the hangar, one focused, no input.

| Condition | Focused client: FPS / 1% low | Unfocused client: FPS / 1% low |
|---|---|---|
| Baseline 1, app off | 143.3 / 76.4 | 23.9 / 13.5 |
| Overlays open, idle | 143.1 / 77.3 | 24.4 / 14.0 |
| Overlays animating (soak) | 142.2 / 75.1 | 17.1 / 8.2 |
| Baseline 2, app off | 142.0 / 75.3 | 21.1 / 10.9 |

- The focused client is unaffected: every condition within ~1 FPS, as are the two baselines; GPU time per frame 5.7-5.9 ms throughout.
- An idle open overlay costs nothing on either client.
- The unfocused client (EVE throttles it to ~24 FPS; ~19 ms of CPU per frame, so CPU-bound) dropped to ~17 FPS while the soak animated, below both baselines: the overlays' CPU work competes with an already starved client. The soak is far heavier than real use (real alerts animate for under 5 s), so in practice this is a brief dip on a background client; no change made.

## Corrections log (things believed early that were wrong)

- "EVE's GPU rose by about 18 points while overlays animated" (first GPU run, 54% to 72%): not supported. The A/B runs showed EVE's own load varies by more than that with no overlay; the consistent effect is on dwm.exe and it comes from continuous animation.

- "Directory events are fine" and "the 0 ms LAG lines are a race": wrong; they were the poller triggering the notification.
- "Filename stamp is local time": wrong, it is UTC.
- "OneDrive is the cause": wrong, plain NTFS behaves the same (OneDrive is slightly better).
- "Naomi is in exclusive Fullscreen" (from minimize behavior): wrong; the owner had switched her to Windowed to test minimizing.
- "Her Local file had no writes while the probe ran": wrong; the probe had not adopted the file (mtime-based scan missed it).
- "A toast that can't display (quiet time, DND) is detected and falls back to an overlay" (DESIGN.md delivery table): only partly; the check used (`SHQueryUserNotificationState`) does not see Windows 11 Do Not Disturb or Focus (#11), so during DND a toast is sent and Windows files it silently instead.
- The old app's repo looked unlicensed (no root LICENSE): it is MIT (README and `EveChatNotifier/License.txt`).

## Untested (verify before relying on)

- Fullscreen overlay on GPUs/drivers other than the owner's.
- Localized (non-English) log headers.
- Release-build CPU and a long soak test.
- Interactive Windows toast from an installed (NSIS) build, and clicking one after the app has quit (#12 covered the running dev build only).
