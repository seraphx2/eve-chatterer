# Findings: measured facts and how to re-check them

Everything here was measured on the project owner's machine (Windows 11, two 1920x1080 monitors, Documents redirected into OneDrive, two clients: Jarna and Psianna Archeia) unless stated. Probes live in `src/bin/`; shared helpers in `src/winutil.rs`. Logs: `probe-log.txt`, `focus-log.txt`, `overlay-log.txt` (gitignored).

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
- Open: `Create` events for new files are believed timely (py-eve-chat-mon says so) but our probe never exercised them.

## 2. Poll cost and scale (debug build, local disk)

| Case | Files | Live pairs polled | Startup scan | Steady CPU |
|---|---|---|---|---|
| Real folder | 44 | 14 | 0 ms | 1.6 ms/s (0.2% of a core) |
| Synthetic | 5,000 | 200 | 15 ms | 11.7 ms/s (1.2% of a core) |

Cost scales with polled (character, channel) pairs, about 0.05 ms/s each, not with total file count. Not measured: OneDrive cloud-only placeholders (opening one may download it), release build, long soak.

## 3. Log format

- Filename: `<Channel>_<YYYYMMDD>_<HHMMSS>_<characterId>.txt`. Stamp is **UTC** (verified against file creation time at UTC-4). Channel names can contain spaces and parentheses (`Private Chat (2)`); private-chat channel ids are GUID-like.
- One file per login session per channel. **Local does not rotate on system jumps.** A new login creates new files.
- Encoding UTF-16LE with a BOM at file start **and a BOM before every line**. Lines end with CRLF. In-line timestamps `[ YYYY.MM.DD HH:MM:SS ]` are UTC.
- Header order: `Channel ID`, `Channel Name`, `Listener`, `Session started`; header keys were English on this (English) client. Localization is unverified.
- Two characters in the same channel write the same lines to separate files; a line near a second boundary can carry stamps one second apart in the two files (seen for Jita Local). Cross-character dedupe must tolerate this.
- Login MOTD lines come from sender `EVE System`.
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
- `SHQueryUserNotificationState` is `BUSY` while an EVE client is focused in most trials but not all (Fixed Window Jarna gave `ACCEPTS_NOTIFICATIONS` once, `BUSY` earlier). It is not a reliable indicator of display mode; use it only as a hint about whether a system toast would show.
- EVE display modes: Windowed (ordinary), Fixed Window (borderless-like, stays up unfocused, spans monitors), Fullscreen (exclusive; hides when unfocused). It looks like Windows still composites Fullscreen (banner drew over it), which may not hold on every GPU/driver.

## 6. Reference footprint: dev-prompt (Tauri, tray + hotkey overlay)

`dev-prompt.exe` 36.6 MB working set (10.7 MB private). Its WebView2 tree, 6 processes for one hidden window: about 304 MB working set, roughly 230 MB private. Single warm window created at startup and shown/hidden. This is a full app, not a minimal overlay; the minimal-overlay number is still to be measured.

## Corrections log (things believed early that were wrong)

- "Directory events are fine" and "the 0 ms LAG lines are a race": wrong; they were the poller triggering the notification.
- "Filename stamp is local time": wrong, it is UTC.
- "OneDrive is the cause": wrong, plain NTFS behaves the same (OneDrive is slightly better).
- "Psianna is in exclusive Fullscreen" (from minimize behavior): wrong; the owner had switched her to Windowed to test minimizing.
- "Her Local file had no writes while the probe ran": wrong; the probe had not adopted the file (mtime-based scan missed it).
- The old app's repo looked unlicensed (no root LICENSE): it is MIT (README and `EveChatNotifier/License.txt`).

## Untested (verify before relying on)

- Fullscreen overlay on GPUs/drivers other than the owner's.
- `Create` events for new session files.
- OneDrive placeholder hydration when adopting old files.
- Localized (non-English) log headers.
- Overlay rendering at different DPI scaling; overlay GPU/frame-time impact while EVE runs.
- Release-build CPU and a long soak test.
- Interactive Windows toast (WinRT, needs an app identity) from an installed build.
