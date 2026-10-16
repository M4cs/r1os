# How r1os works

The mechanics an agent needs before changing anything: layers, ports, the token, input, docked mode, the host and r1d, the agent jail, the app window, apps, storage, the look, and what must stay in sync.

## Layers and processes
- Rabbit kernel 4.19 + vendor + bootloader (never modified) → r1os's own Android 15 system image (AOSP `android-15.0.0_r36` + TrebleDroid, userdebug, so `adb root` works; `ro.r1os.image.version`; signed with r1os's own keys, ADR-0062) → r1d (root, init service `r1d` in its own SELinux domain, enforcing) → host app (HOME, a system app signed with r1os's platform key, running as `platform_app`) → agent box (Alpine arm64 chroot, `claude` + `codex` CLIs) (`web/src/docs/content/architecture.md`).
- **Android apps** you install (Aurora Store) run beside r1os in Android's own sandbox, without r1d's token. Agents can drive them in the app window (below).
- **r1d on the R1:** `/system/bin/r1os-r1d` (wrapper in `device/aosp/vendor_r1os/bin`) runs the binary `tools/deploy.sh` put in `/data/r1os/bin/r1d`; logs to `/data/r1os/log/r1d.log` (rotated at 1 MB). init restarts it when it exits, which is how deploy swaps binaries (`pkill -x r1d`).
- **r1d on a computer (`--sim`)**: no hardware, input via the harness, simulated motor, data in `.r1os-sim/`, **no token** (open), **no agent box** (builds and agent turns fail with "the agent box only exists on the R1").

## Ports
| Port | Where | What |
|---|---|---|
| 7171 | R1 loopback | r1d (HTTP + `/ws`); also the simulator's port on a computer |
| 7172 | computer | `adb forward tcp:7172 tcp:7171` by `tools/lib.sh` (`R1D_LOCAL_PORT`), used by `r1h`, `agent-test.py`, the evals, `curl` recipes |
| 7173 | R1 loopback | The host's mic socket into r1d: hello `r1os-mic 1 16000 <token>`, then 16 kHz mono s16le while r1d says `S` until `E` |
| 5171 | computer | Vite hot reload for the shell (proxies to r1d on 7171) |
| 1420 | computer | `bun run --cwd companion dev` (window with a demo R1) |
| 5180 | computer | `bun run --cwd web dev`: the website |
| random | R1 LAN | Share-by-QR server, `/s/<128-bit token>/<name>`, 10 min (`share.rs`) |

## The per-boot token (ADR-0026, ADR-0036)
- `/api`, `/harness`, `/ws` need `Authorization: Bearer <token>`. r1d makes a new token **each start** and writes it to `/data/r1os/run/token` (0600, root). `T=$(source tools/lib.sh; r1d_token)` reads it as root over adb.
- The host gets it with `GET /token`; r1d answers only a socket the kernel lists (`/proc/net/tcp`) as the host's uid (403 otherwise). AOSP's SELinux forbids any daemon writing into an app's files, so r1d doesn't hand it over that way.
- Mini-app pages get a page token (`POST /api/apps/{id}/session`) good only for their own app's runtime endpoints.
- Not covered: `/mcp` (per-agent-process tokens), static files, the simulator. `/data/r1os/auth-off` disables the check (a way back in).
- Root adb can read it too, and USB adb isn't authenticated yet (gotchas #13).

## Input, gestures, the harness
- Wheel and button come from evdev, not Android's input stack (Android swallows the waking press). Not grabbed, so the button still wakes the screen (keylayout `BUTTON_1 WAKE`).
- One path: raw edges (evdev or `/harness/wheel|button`) → `gesture.rs` → stamped events (`seq`, `t_ms`, `source: hw|sim|api`) → `/ws` and a ring buffer. Simulated input exercises the real gesture logic.
- Defaults: hold 350 ms, double 280 ms, wheel ticks within 120 ms of a button edge dropped as `wheel_suppressed` (mechanical ghost ticks, Q12). A triple press only exists with a shortcut set.
- Push-to-talk: capture starts on button down; `hold_start` → `voice: listening/partial`; release → 400 ms tail → 80 ms drain → `voice: final` (~0.7 s; shorter when you'd stopped talking). Hold + wheel: first tick forgiven, second cancels talk and becomes `volume {on}`. Only after onboarding; audio never leaves RAM; 60 s cap.
- Harness endpoints (`--harness`): `/harness/state|events|wheel|button|tilt|rest|screenshot|jail|say|open|trigger|net|abilities|system1|summarize|tidy|translate|hear|tts|paradee|perf|browser|app_window|commit_card|dry_run|bootanim` ([api.md](api.md) has what each does).

## Docked mode and touch (T-103, T-117, T-156, T-157)
- **Docked mode** is a setting (Settings › Display & sound; `docked` in `/api/device`). While charging, the glance screen keeps the display on as a bedside clock. On its side (its right side at the bottom) the screen turns to landscape, plugged in or not, and stays on above 20% battery.
- **At rest:** two minutes untouched, nothing moves on the glance screen, the backlight drops to 6/255 and the accelerometer stops. A touch, the wheel, the button or a notification brings it back. The accelerometer listens only while docked mode is on and the display is on.
- **The rest of r1os turns too**, the same screens in landscape, and touch stands in for the wheel and button (ADR-0014's one exception):
  - a swipe up from the glance screen's bottom edge unlocks it, the screen riding up with the finger as a sheet (with a PIN, only a little, then the dial); a swipe down from the top (the first 40 dp) brings it back down over any r1os screen;
  - on home, a vertical swipe moves through your agents, a swipe up from the bottom edge opens the launcher, and one tap on the agent opens its current chat;
  - a tap on a page's header lights it as Back, and a second goes back;
  - a mic in home's bottom-right corner talks: `POST /api/voice/talk {op: start | send | cancel}`, the side button's own hold (so r1d and the shell can't tell them apart); a talk nobody sends ends by itself after a minute.
- **How the host knows:** its accelerometer, or the `tilt {side}` event. Try it without hands with `POST /harness/tilt {side}` (1 on its right side, -1 its left, 0 upright, `null` back to the accelerometer), and hold rest's steps with `POST /harness/rest {step, brightness?}` (0 awake, 1 still, 2 low, `null` its own timer).
- **Asleep is plain sleep:** the panel runs in video mode, so it can't doze; SystemUI's doze is turned off at every start, and docked, the display keeps its turn while it's off.

## Host ↔ r1d
- The host keeps the **only** `/ws` connection on the R1; actions go over HTTP (`net/R1d.kt`). Events are pushed; commands are never sent over `/ws`.
- The native shell draws everything (ADR-0017). Each open mini-app is a WebView on `app.html#<id>`; it receives the wheel, clicks and its `app_*` events via `window.r1Event` and calls back via `R1Native` (`ready`, `overscroll`).
- The host does Android-only I/O and reports to r1d: mic (`Mic.kt`), camera frames/photos (`POST /api/media`, QR frames to `/api/link/scan`), device settings (`POST /api/device`), the Android apps list (`PUT /api/android/apps`), notifications, media sessions, DNS servers, the charge estimate (`POST /api/battery`), browser and app window results.
- Reconnect = reload: when `/ws` comes back after a deploy, the native shell reloads agents and mini-app pages; the React shell reloads itself.

## Agents in the jail (ADR-0003, ADR-0010, ADR-0011, `jail.rs`, `web/src/docs/content/agents.md`)
- Box: Alpine 3.24 rootfs at `/data/r1os/agent/rootfs`; shared home `/data/r1os/agent/home` (CLI logins and settings, one sign-in for all agents); per agent `agents/<id>/work` → `/work` (`/work/files` workspace, `/work/.sessions` per-process prompt + MCP config), plus its own Claude projects/file-history and Codex sessions bound over the home's. `agents/` is root-only.
- Each process: memcg `/dev/memcg/r1os-agent` 1.5 GB, private mount ns, fresh /proc, 512 MB /tmp, minimal /dev, chroot, uid/gid 2950 + inet 3003, no caps, no_new_privs, seccomp (mount/ns/ptrace/module/bpf/keyring/clock/reboot → EPERM), setcon `u:r:r1d_agent:s0`. No user namespaces on Rabbit's kernel (Q2), so the CLIs' bubblewrap sandboxes can't run: **the jail is the boundary**.
- Claude chats: one warm `claude -p --input-format stream-json --output-format stream-json --model sonnet --effort low --permission-mode manual --permission-prompt-tool mcp__r1__permission --mcp-config … --system-prompt-file …` per active chat (`session.rs`), env `MCP_TOOL_TIMEOUT=180000`, `ENABLE_TOOL_SEARCH=false`. Codex: one `codex exec --json --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox` per turn (the jail confines it).
- Budget `MAX_PROCS = 3` (~245 MB warm Claude, ~300 MB Codex turn); the least recently used idle warm session is stopped for room, otherwise "waiting". Idle stop: 15 min screen on, 3 min screen off (`session.rs`).
- Agents act on the R1 only through r1d's MCP endpoint (`http://127.0.0.1:7171/mcp`, bearer token per process → `{agent, chat}`). Changes go through approvals by area (ask/allow/never; [api.md](api.md)). A turn started by another agent's message can't delete/change apps, run confirmed actions or change shared memory (Q36).
- Talking carries on the agent's **current chat** (its most recent, however old; ADR-0057). New chats start only from New chat. Chats are tidied into a fresh session at half the context window (≤100k tokens).

## The app window: agents in Android apps (ADR-0061, ADR-0063, `appwin.rs`, `AgentApps.kt`)
- **What it is:** an installed Android app on a virtual display the host owns: trusted, **600×1200 at density 180** (about 533×1067 dp, a large phone's layout), with its own focus (the wheel and button stay r1os's) and its own power group, no lock screen. It draws at **15 fps** into an `ImageReader` whose latest frame is the agent's picture and what you watch: a small live preview in the chat, or the full window with a touch. Only the harness can change its size (`POST /harness/app_window {op: "size"}`), not agents.
- **The route:** an agent's `app_*` tool → r1d emits `app_window {id, op, agent, chat, args}` → the host acts → it answers at `POST /api/apps/window/result`. A tool waits up to 30 s (`app_ask_user` up to 5 minutes). The window's tools run **one at a time, in the order they came**; `app_do` holds the window across its steps. The browser (`browser_*`, ADR-0054, `AgentBrowser.kt`) works the same way with its own turn.
- **Reading:** `AppReader`, an accessibility service that's **on only while a window is open** (every app does accessibility work while any service runs). Reads, finds and the step's picture run on the host's own `r1os-app-ops` thread, never the UI thread. Every step's answer carries the screen as lines with refs (`[a12]`) and its picture.
- **Acting:** touches and keys injected into that display (the platform signature's `INJECT_EVENTS`); web fields get a tap, select-all, delete and real keys. Each act waits for its screen to settle (up to 6 s).
- **What asks:** an app agents haven't used ("let agents use Uber?", once, kept for every agent); the first tap, typing or key in a turn, by the agent's `android` level; a final step (order, ride, payment, subscription, message) every time, on a commit card with the order read from the app's screen.
- **What the agent never gets:** the display isn't secure, so screens Android marks secure come out black and their words are withheld; password, card and one-time-code fields refuse its typing; Android's permission prompts are for the user. It hands those over with `app_ask_user`: the app shows full screen and your touches go into it.
- **Handing over:** "let me use it" (`app_close {for_user: true}`, or the full window's button) moves the app onto the R1's own screen as it is; an app already open there moves into the window as it is.
- **Memory and power:** the window is made on first use and closes two minutes after the last request (unless you're watching or it's waiting for you). An app opened only for the agent is closed a minute after the window closes (not one that was yours, nor while something plays). Cached Android apps you haven't used for 10 minutes are closed (`BackgroundApps`, 30 s after the screen goes off and 10 minutes after you come back to r1os; never on a timer of its own). When the R1's screen goes off, the window's display sleeps with it.
- **Planned, not built:** agents carrying on in an app with the screen off while the R1 sleeps (a framework patch, T-114).

## Apps (ADR-0005/0006/0017/0023/0041, `web/src/docs/content/apps.md`)
- An app is an EneeSpec YAML compiled by `enee-spec`; rendered in its own WebView by `packages/runtime` with r1os's theme. What the R1 runs is narrower than the language (`broker::r1_problems`).
- **Build** (`POST /api/create {brief, model?, big?}`): one build at a time (409 otherwise); a no-tools turn (`--tools "" --strict-mcp-config --restricted --disable-slash-commands --no-session-persistence --effort low`) with `prompts/builder.md` + `design.md` as system prompt and `create.md` + brief as message; extract the ```` ```enee ```` block; compile, R1 checks, `run_checks`; probe each http service once; logic trial run; ≤2 repairs; install. Progress as `build {stage}` events. A new app never replaces another (`<id>-2`…); only `edit_app` changes one. Model setting: haiku / sonnet (default) / opus / codex.
- **Storage** (`/data/r1os`, or `.r1os-sim`): `apps/<id>/enee.yaml` (source of truth) and `versions/<ms>.json` (plain files, not git); `data/<id>/app.db` (+ `state.json`, `services.json`, `errors.json`); `trash/<id>-<ms>/` (7 days); grants and trigger runs in `core.db`.
- Web data only through r1d's fetcher, behind permissions the user grants on the shell's consent card (never by the app or an agent).

## Where data lives on the R1
| Path | Contents |
|---|---|
| `/data/r1os` | Root-only (0700). Four SELinux labels: r1d data, `r1os_code_file` (`bin/`, `lib/`), box userland, agents' files |
| `/data/r1os/bin/r1d`, `lib/` | r1d binary; sherpa-onnx + ONNX Runtime (`RUNPATH=/data/r1os/lib`) |
| `/data/r1os/shell` | Built shell (`shell/dist`) |
| `/data/r1os/core.db` | Agents, chats, messages, memories, mail, alarms, timers, routines, look, grants, apps agents may use, settings |
| `/data/r1os/apps`, `data`, `trash`, `media`, `cache/cards` | Apps, app data, trash, photos/recordings, card images |
| `/data/r1os/models/{stt,system1,translate}` | Speech (Kroko `.ort`), System One embedder, Translator (~2.2 GB) |
| `/data/r1os/agent/{rootfs,home,work,agents/<id>}` | The box (above) |
| `/data/r1os/link/key`, `run/token`, `log/r1d.log`, `boot/bootanimation.zip` | Link key (root-only), per-boot token, log, boot animation (bind-mounted over the image's) |
| `/metadata/r1os/verbose_boot` | Verbose boot flag (a file, since persistent props load only after /data) |

## The look (ADR-0043)
- `GET /api/look` → `{look, options}`; `PUT /api/look` with **only what changes** (`{"mode":"light","glance":{"clock":"dial"}}`), merged object by object (`null` = back to default), checked whole, stored, pushed as `look {look}`; the shell rebuilds in it. Agents use `set_look`/`set_agent_look` (approval-gated); agents' `eyes`/`wear` via `PUT /api/agents/{id}` (whole profile).
- New screens use `Theme` tokens (`host/…/ui/Theme.kt`), never literal colours, so light mode works. Labels are lowercase (display-only), focus is a solid accent fill (never an outline).

## Sync rules
- **Protocol:** the API docs (`web/src/docs/content/api.md`) ↔ `shell/src/r1.ts` types ↔ `crates/r1d`. In practice also the native consumer (`host/…/net/R1d.kt` and the screens) and, for routes a computer may call, `link::allowed` in `crates/r1d/src/link.rs` and `companion/src/link.ts`.
- **Prompts:** `prompts/example.enee.yaml` must equal builder.md's example and compile with passing checks (`create.rs` test `the_builder_example_compiles_and_passes_its_checks`; `cargo test -p r1d`). Keep prompts in step with what r1d runs: a section or step goes into the prompts only once r1d supports it, and leaves them if r1d drops it. Prompts are `include_str!`'d, so editing them means rebuilding and redeploying r1d.
- **Agents' instructions:** a tool's behaviour changes with its description in `mcp.rs` and, where it matters, the chat system prompt in `chat.rs` (which has a section on the app window); per-app tips live in `appwin.rs`.
- **Docs:** a behaviour change updates its spec/architecture doc in the same change; hard-to-reverse choices get an ADR. When a rule or gotcha in `CLAUDE.md` changes, this skill (`web/public/skills/r1os/`) changes too.
