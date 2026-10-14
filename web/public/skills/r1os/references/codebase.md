# r1os codebase map

Where everything lives, one line each, from the module headers. Paths are relative to the repository root. For `tools/`, the last column says whether a script writes to the device: if it does, ask the R1's owner first (see [rules.md](rules.md)).

## Top level

| Path | What it is |
|---|---|
| `crates/r1d/` | The core daemon (Rust, root on the R1): evdev input, gestures, motor, data, permissions, app compiler, agents, the app window and browser tools, speech, link, harness. HTTP/WS on 127.0.0.1:7171 |
| `crates/enee-spec/` | EneeSpec compiler (YAML app spec → compiled app), ported from eneeware (ADR-0005) |
| `crates/enee-data/` | Per-app SQLite: migrations, undo (ported from eneeware) |
| `crates/r1-link/` | The R1↔computer link protocol over iroh (pairing code, messages, short code); `bin/r1-link-pc` test program |
| `crates/r1-bootlog/` | Verbose boot's program (`/system/bin/r1os-bootlog`, T-101) |
| `host/` | Kotlin host app `dev.r1os.host` (HOME, signed with r1os's platform key): the native shell (ADR-0017), mic, camera, keyboard, WebViews for mini-apps, the agents' browser and app window. No AndroidX/Compose |
| `shell/` | React + Vite: the **desktop simulator's** UI (`index.html`) and `app.html`, the page each mini-app runs in on the R1. Only `app.html` runs on the R1 |
| `companion/` | Tauri 2 desktop app (Windows first): links to the R1, manages it, runs agents on the computer (`src-tauri/`), window in `src/` |
| `web/` | The website (Vite, React Router, TypeScript): the home page (`src/home/`), the R1 drawn in the browser (`src/device/`), the docs (`src/docs/content/*.md`) and this skill (`public/skills/r1os/`). It reuses the shell's `Character` and the companion's R1 screen and look through the `@shell` and `@companion` aliases |
| `packages/runtime` | `@r1os/runtime`: json-render runtime for compiled enee apps (catalog, StateStore, `$x` directive, action runner, logic) |
| `packages/icons` | `@r1os/icons`: Phosphor icons, lazy per-icon SVG, catalog, app tiles |
| `agent-box/` | `setup-rootfs.sh` (builds the Alpine box on the R1) and `agent-exec.sh` (legacy BusyBox entry; r1d's own `jail.rs` replaced it for r1d's sessions) |
| `device/aosp/` | r1os's image layer: `vendor_r1os/` (r1os.mk, `init/r1os.rc`, `bin/r1os-r1d` wrapper, `sepolicy/*.te`, keylayout, permissions, overlay), AOSP manifests, TrebleDroid patches |
| `device/overlays/` | Framework overlays installed on an R1 set up before the image had them built in (`edges/`: Android's edge swipes off, T-127) |
| `device/magisk/r1os-base/` | The old Magisk module from before r1os's own image: service.sh restart loop, keylayout, features |
| `prompts/` | `builder.md`, `design.md`, `create.md`, `edit.md`, `example.enee.yaml`: compiled into r1d with `include_str!` (`crates/r1d/src/create.rs`) |
| `fixtures/apps/` | Sample `.enee.yaml` apps (water, coffee, news, weather-live, stretch, dice, radio…) |
| `fixtures/evals/` | `briefs.json` and `edits.json` (app build evals), `control.json` (app window and browser scenarios) |
| `tools/` | Bash + Python scripts: build, deploy, simulator, harness CLI `r1h`, evals, signing keys, flashing, backup |
| `docs/` | Obsidian vault and source of truth: specs, ADRs, research, runbooks, kanban, roadmap, open questions |
| `third_party/rubik/`, `NOTICE.md` | Font and third-party credits |
| `reports/`, `research_notes/` | Loose research (local LLMs on the R1) |
| `firmware/`, `backups/`, `out/` | **Git-ignored.** Stock firmware/GSIs/Magisk; partition dumps (IMEI, radio calibration); build outputs. Never commit, upload or paste |
| `artifacts/` | Git-ignored harness output (`r1h screenshot` → `artifacts/screenshots/`, `artifacts/power/`, `artifacts/evals/`) |
| `.r1os-sim/` | Git-ignored simulator data dir (`r1d --sim --data`) |
| `pnpm-workspace.yaml` | `shell`, `companion`, `web` (the website and this skill), `packages/*` |
| `Cargo.toml` | Workspace: r1d, r1-link, r1-bootlog, enee-spec, enee-data, companion/src-tauri. Release profile: `opt-level="s"`, thin LTO, strip, **`panic = "abort"`** |

r1os's signing keys are never in the repository (ADR-0062): `tools/make-keys.sh` makes them outside it, and `R1OS_KEYS` points there.

## `crates/r1d/src` (one line each, from module headers)

| Module | What it does |
|---|---|
| `main.rs` | Entry; flags `--listen 127.0.0.1:7171 --shell --data --sim/--hw --harness --hold-ms 350 --double-ms 280 --wheel-guard-ms 120`; defaults `/data/r1os` on the R1, `.r1os-sim` + `shell/dist` on a computer |
| `server.rs` | All HTTP + `/ws` routes, static shell files with index.html fallback, `/harness/*` |
| `auth.rs` | Per-boot bearer token for `/api`, `/harness`, `/ws`; app page tokens; `GET /token` only for the host's uid (ADR-0026/0036) |
| `hub.rs` | Shared state, event stream, gesture task; evdev or harness → gestures → stamped events → `/ws` + 256-event ring buffer |
| `input.rs` | Raw evdev: wheel `och1970_holl_key` (KEY_UP/DOWN), side button `mtk-kpd` (KEY_POWER 116); not grabbed |
| `gesture.rs` | Pure state machine (tap, double, triple, hold, wheel suppression); unit-tested |
| `voice.rs` | Push-to-talk: mic socket 127.0.0.1:7173, sherpa-onnx streaming STT (Kroko), 400 ms tail, volume on hold+wheel, talking by touch's cancel |
| `system1/` | System One: on-device intent reading (rules + all-MiniLM-L6-v2 embedder on ONNX Runtime; `embed`, `extract`, `tokenizer`) (ADR-0032); also matches app window and browser targets by meaning |
| `translator/` | The Translator: Whisper base (`speech.rs`), OPUS-MT on ONNX (`mt.rs`), Piper voices (ADR-0034) |
| `tts.rs` | sherpa-onnx TTS loader/bench (voice output itself deferred, Q14/Q46) |
| `jail.rs` | The agent box enforced by r1d: memcg 1.5 GB, private mounts, chroot, uid 2950, no caps, no_new_privs, seccomp, `r1d_agent` SELinux; one jail per agent |
| `session.rs` | Agent CLI processes: `MAX_PROCS = 3`, warm Claude sessions (stream-json), spares, idle stop (15 min screen on / 3 min off) |
| `agent.rs` | One-shot no-tools build sessions (`claude -p` or `codex exec`) in the jail |
| `chat.rs` | Conversations: Claude warm sessions, Codex per turn, the system prompt (with its section on the app window), tidy at half the window (≤100k) |
| `mcp.rs` | r1d's MCP server (`POST /mcp`, JSON-RPC over HTTP), ~90 tools; token per agent process → `{agent, chat}`; app window and browser tools one at a time |
| `appwin.rs` | Agents in Android apps (ADR-0061): the `app_*` tools' requests to the host (`app_window` event, `/api/apps/window/result`), first-use, act and final-step cards, the order read from a checkout, dry runs, per-app tips |
| `browser.rs` | The R1's browser driven by agents' `browser_*` tools; the host's WebView answers at `/api/browser/result` (ADR-0054) |
| `stops.rs` | Stopping a running turn (Claude interrupt via stream-json `control_request`; Codex killed) |
| `mail.rs` | Agents messaging each other (`ask_agent`, threads); refuses risky tools in agent-started turns (Q36) |
| `abilities.rs` | What each agent can/can't do (where it works, browser, apps), for its prompt and `list_agents` |
| `accounts.rs` | Signing the box into Claude (`claude auth login`, LAN pairing page) and Codex (`codex login --device-auth`) |
| `connections.rs` | claude.ai connectors / ChatGPT apps as the CLIs report them; r1os holds no tokens (ADR-0052) |
| `skills.rs` | Agent Skills via `npx skills add` in the box (T-086) |
| `core.rs` | `core.db`: agents, chats, messages, memories, mail, settings, apps agents may use; migrations |
| `apps.rs` | Installed enee apps and data on disk; versions, trash, export/import |
| `create.rs` | Building/editing apps: builder prompts, compile, checks, web-data probe, ≤2 repairs, install; prompt/example sync test |
| `logic_check.rs` | Trial run of an app's `logic` script in the box (QuickJS) before install (ADR-0041) |
| `broker.rs` | App permissions (`net.http`, `schedule`, `notify`, `open.url`, `clipboard.write`, `audio.play`) decided only on shell screens |
| `fetch.rs` | SSRF-safe fetcher for apps (https, declared hosts, public addresses only, redirects re-checked) |
| `feed.rs` | RSS/Atom parsing (apps' `http.rss`, agents' `read_feed`) |
| `services.rs` | Apps' live web data while open (or in background with `schedule`, screen on only) |
| `scheduler.rs` | App triggers (timed, `notify`) in r1d, persisted run times |
| `wake.rs` | One `timerfd` on `CLOCK_REALTIME_ALARM` + `EPOLLWAKEUP` for the next due thing; R1 sleeps again after |
| `alarms.rs`, `timers.rs`, `routines.rs` | Alarms/reminders/to-dos (ADR-0028), timers/pomodoros (ADR-0050), agent turns on a schedule (R7) |
| `notices.rs` | An app's reminders kept until seen (T-049) |
| `whatsup.rs` | "What's up": unread, reminders, alarms, timers, routines, journal, notifications, playing; no agent, no network |
| `missed.rs` | Android notifications that came while the display was off, kept a day, matched to connections |
| `glance.rs` | Glance-screen widgets: apps' `glance:` cards filled from their data (ADR-0042) |
| `look.rs` | The device's look (mode, accent, font, backgrounds, glance layout), merged object by object (ADR-0043) |
| `approvals.rs` | Agents' changes and acts via ask/allow/never levels per area and an on-device card (ADR-0045) |
| `device_settings.rs` | Brightness, timeout, volume, sounds, haptics, docked, time zone, build model… in one place |
| `device.rs` | Camera motor (sysfs `step_motor_ms35774/orientation`, one move at a time), the battery, and read-only system facts |
| `power.rs` | Power policy applied at start (no background scans, no location, cell radio off without a SIM, Doze sooner, no doze display) (T-037) |
| `pin.rs` | 4-digit wheel PIN lock, salted hash, back-off (ADR-0016) |
| `media.rs`, `recorder.rs`, `notes.rs`, `journal.rs` | Photos/recordings store, Recorder, Journal voice notes, Journal (ADR-0031/0033/0037) |
| `player.rs` | Apps' background audio via the host's media service (ADR-0040) |
| `cards.rs` | Cards in chats (`show_card`): flat catalog checked in r1d, drawn natively (ADR-0039) |
| `android.rs` | Installed Android apps, opening them, Aurora's page, AOT-compiling the host (`compile_host`), music URIs |
| `screens.rs` | r1os's own screens by name for `open_screen` and System One |
| `share.rs` | Sharing one item by QR on the Wi-Fi; installing an app from another R1's QR (ADR-0056) |
| `link.rs`, `remote.rs`, `backup.rs` | Linked computers over iroh (R1 only dials), agents running on a computer, backups to the companion |
| `boot.rs` | Boot animation zip and verbose-boot flag on /metadata |
| `perf.rs` | Latency marks for `r1h perf` (`/harness/perf`, app window steps as `app.<op>`), in memory |
| `tool.rs` | Runs Android's `cmd` with a deadline (see the gotcha on SELinux call-backs) |
| `wifi.rs` | Wi-Fi via `cmd wifi` |

## Host app (`host/app/src/main/java/dev/r1os/host/`)

- **Root package:** `MainActivity` (HOME; swallows DPAD/BUTTON_1/BACK; shell at 213 dpi = 360×480 dp canvas), `HostApp` (the process: r1d's events from whatever started it), `Mic`/`MicService` (lends the mic to r1d on :7173; foreground mic service over other apps), `AgentBrowser` (the agents' WebView browser), `AgentApps` (the app window: the virtual display, injected input, reads on its own thread), `AppReader` (the accessibility service that reads it, on only while a window's open), `AgentPointer` (an agent's pointer over the browser and app previews), `BackgroundApps` (closes cached Android apps left alone for 10 minutes), `AndroidApps`, `Notifications` (+ listener), `MediaSessions`, `Player`, `Bluetooth`, `DeviceSettings`, `Awake` (partial wake lock for screen-off routine turns), `OverActivity` (layer over apps that hide overlays), `Adb`/`AdbConfirmActivity` (answers adb confirmations in r1os's UI).
- **`net/`:** `R1d.kt` (the native shell's only link to r1d: JSON over HTTP on a pool, `/ws` events on the main thread; `Ev` event wrapper), `WebSocketClient.kt` (minimal RFC 6455 client).
- **`shell/`:** every native screen. `Shell.kt` (root; docked mode's tilt and touch), `Screen` (the base and targets), `HomeScreen`, `Glance` (with `Clocks`), `ChatScreen`, `AgentScreens`, `AgentReplyCard` (talking over an open app), `FocusList` (the wheel-driven list every screen below home uses), `Pages`, `Dock` (banner, reply card, voice and volume overlays), `VoiceOverlay`, `VolumeOverlay`, `MiniApp` (WebView per mini-app, kept pages), `AppScreens`, `Consent`, `ApprovalScreen`, `PinDial`, `CameraScreen`/`CameraParts`, `Deck`/`DeckScreens` (Recorder, Journal), `MediaScreens`, `TranslatorScreen`, `AlarmsScreen`/`AlarmView`/`RemindersScreens`/`TimerScreen`, `NotificationsScreen`, `SettingsScreens`, `DeviceScreens`, `ControlsScreens`, `LookScreens`, `SetupPanels`, `ConnectionsScreens`, `ComputerScreens`, `BluetoothScreen`, `ShareScreens`, `BrowserScreen`/`BrowserPreview`, `AppWindowScreen`/`AppPreview` (the app window full screen and small in the chat), `PointerView`, `CardView`, `LinkScreen`, `OnboardingScreen`, `WhatsUpScreens`, `NoticeBanner`, `HomeStopsScreen`, `AdbConfirmScreen`.
- **`ui/`:** `Theme.kt` (look tokens; use them, never literal colours), `Look.kt`, `Backdrop.kt`, `CharacterView.kt` (agents drawn natively), `Dim.kt` (mirrors Android's pre-timeout dim), `Spring.kt`, `Sounds.kt`, `Haptics.kt`, `Icons.kt`/`SvgPath.kt`, `Noise.kt`, `BootAnimation.kt`.
- **`camera/`:** `Camera` (Camera2), `Motor` (via r1d), `Viewfinder`, `Photos`, `Film`.
- **`ime/`:** `R1Keyboard` (r1os's IME, default for every app; screen turns to landscape while typing) and `KeyboardView` (ADR-0055).

## `shell/src` (simulator UI + mini-app page)

`r1.ts` (link to r1d: `/ws`, `/api`, `/harness`; **protocol types**), `api.ts` (typed calls for agents, chats, memory…), `main.tsx`, `App.tsx` (root), `sim.tsx` (simulator frame at 427×569 CSS px; keys → harness), `input.ts` (routing; unconsumed double tap goes home), `home.tsx`, `space.tsx`, `chat.tsx`, `voice.tsx`, `launcher.tsx`, `manage.tsx`, `create.tsx`, `onboarding.tsx`, `setup.tsx`, `settings.tsx`, `apps.tsx` (built-in skeleton apps), `tool-labels.ts`, `ui/Character.tsx` + `character.css`, `ui/noise.ts`, `styles.css`, `ui/screens.css`. Mini-app page: `app-main.tsx` (`app.html#<id>`, no socket; input via `window.r1Event`, calls back through `R1Native`), `app.css`, `apps/AppView.tsx`, `apps/host.ts` (runtime Host backed by r1d: queries by name, steps by reference), `apps/focus.ts` (wheel focus over runtime components), `apps/theme.css` (r1os look over the runtime).

## `companion/`

- `src-tauri/src`: `main.rs` (tray app; link runs start to quit), `link.rs` (iroh endpoint R1s dial; its key and trusted R1s live in the companion's app-data folder), `runner.rs` (agents on the computer: the unmodified `claude` CLI in the agent's folders), `codex.rs` (Codex app-server JSON-RPC).
- `src`: `App.tsx`, `link.ts` (calls to an R1's allowed API; r1d's `link::allowed` decides), `theme.ts`, `motion.ts`, `demo.ts`/`demo-more.ts` (fake R1 for `pnpm --dir companion dev`), `r1screen/` (R1 home/glance painted like the host), `views/` (About/backup, Accounts, Agents, Alarms, Apps, Chats, Computer, Connections, Files, Look, Pair, R1View, Settings).

## `tools/` (script → what → device effect)

Python scripts run with `python3` (`py -3` on Windows); bash scripts source `lib.sh`.

| Script | What it does | Device |
|---|---|---|
| `lib.sh` | Shared helpers: `MSYS_NO_PATHCONV=1`, `adb_ready` (USB first, else mDNS wireless), `r1d_forward` (7172→7171), `r1d_token`, `KEYS` (the signing keys' folder; `R1OS_KEYS`), `confirm` (fails without TTY; `YES=1`), `require_backup` | — |
| `device-report.sh` | Snapshot (what's flashed, kernel, root, wheel/button/motor, kernel features); redacts serials/IMEIs | **read-only** |
| `backup.sh` | Dumps device-unique partitions + boot chains + mmcblk0boot* to `backups/<time>-slot<x>/` with SHA256SUMS | **read-only** (needs root) |
| `hw-check.sh` | Raw wheel/button events | read-only; **`--motor` writes** (moves camera) |
| `r1h` | Harness CLI (state, events, wheel, button, launch, screenshot, logs, apps, install, remove, export, create, power, perf, bootanim, screen, open, motor) | state/events/logs/screenshot/power/perf read-only; input commands drive the UI; `motor` **ask**; `create` spends subscription |
| `sim.sh` | Desktop simulator (`--dev` adds Vite on :5171) | computer only |
| `make-keys.sh` | r1os's own signing keys, once, outside the repo (refuses to overwrite) (ADR-0062) | computer only |
| `build-r1d.sh` | NDK cross-compile r1d (sets `CARGO_ENCODED_RUSTFLAGS`, unsets `RUSTFLAGS`) | computer only |
| `build-host.sh` | Gradle → `out/r1os-host.apk` (release by default), re-signed with r1os's platform key | computer only |
| `build-bootlog.sh` | Cross-compile `r1-bootlog` | computer only |
| `build-image.sh` | AOSP 15 + TrebleDroid + r1os layer, in a Linux build environment (WSL works); stages `init sync patch layer build export`, plus `policy` | computer only |
| `fetch-speech.sh`, `fetch-models.sh`, `fetch-aurora.sh` | Pinned, hash-checked downloads into `out/` (`translator-models.txt` pins the Translator's) | computer only |
| `optimize-stt.py` | ONNX → `.ort` for the R1 (ORT minor must match the R1's) | computer only |
| `power.py` | Diffs two power snapshots (used by `r1h power`) | computer only |
| `deploy.sh [--host] [--no-build]` | Pushes r1d, shell, libs, models; `restorecon -RF`; `pkill -x r1d`; `--host` installs, AOT-compiles, sets HOME and IME | **writes** (ask before first deploy of a session) |
| `agent-box.sh` | `install`, `codex-host`, `run -- CMD`, `root -- CMD`, `shell`, `status` | `status` read-only; others **write** |
| `agent-test.py` | New chat through r1d's API, prints tools/timings/replies | spends subscription; delete chats after |
| `eval-apps.py` | Builds briefs on the R1, reports to `artifacts/evals/`, removes apps | spends; asks first (`YES=1` skips) |
| `eval-control.py` | App window and browser scenarios (`fixtures/evals/control.json`) as real agent turns in dry-run test chats; declines final steps, counts the cart, waits while the owner uses the R1, pauses to recharge; reports to `artifacts/evals/` | spends; asks first (`YES=1` skips); drives the R1 |
| `edges-overlay.sh [off]` | Builds, signs (platform key) and installs the overlay that turns off Android's edge swipes, for an R1 whose image predates it | **writes** |
| `grow-data.sh` | Online `resize2fs` of /data | **writes** |
| `flash-boot.sh` | Writes `boot_<slot>` (dd from Android with readback, or fastboot) | **writes** |
| `root.sh` | Magisk-patches the device's own boot and flashes it | **writes** |
| `install-module.sh` | Installs a Magisk module | **writes** |
| `flash-gsi.sh` | GSI to system, vbmeta off, wipes data | **writes, wipes** |
| `flash-stock-base.sh` | Path B stock base (boot, dtbo, vbmeta, super) | **writes** |
| `fastboot-entry.py` | Preloader serial handshake → fastboot from power-off | changes device mode; ask |
