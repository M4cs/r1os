# CLAUDE.md: working on r1os

r1os turns a Rabbit R1 into an agent-first handheld. Rabbit's kernel and vendor stay as the hardware
layer. On top sit our own Android system image, a Rust core daemon (`r1d`), a thin Kotlin host app
with a WebView shell, and the unmodified `claude` / `codex` CLIs in an Alpine chroot. Read
[How it's built](web/src/docs/content/architecture.md) first.

**Maintainers:** if `docs/maintainer.md` exists (the maintainers' private, git-ignored vault), read it before anything
else and follow it as well as this file. It holds how their work is tracked and their machine's details.

## Docs

- The public docs are the website's pages, `web/src/docs/content/*.md`. The agent skill is `web/public/skills/r1os/`.
- **Behaviour changes update those pages in the same change.** A rule or gotcha that changes here changes in the skill
  too.
- ADR numbers (ADR-0001…) and task numbers (T-001…) in this file, code comments and commit messages name the maintainers'
  private decision records and tasks. They're kept as design history, as are the "(Max, date)" notes on a decision. What a
  contributor needs from them is said where they're cited.
- Work you find but won't do now (a bug, a follow-up) becomes an issue, not a TODO in code.

## Device safety (hard rules)

The R1 has no volume keys and no recovery menu. A bad write to the wrong partition can hard-brick it.

- **Ask the person you're working for, in chat, before anything that writes to the device**, naming the exact command
  or script. That covers:
  - `fastboot flash/erase/set_active/reboot fastboot` and `fastboot -w`;
  - `dd of=…`, Magisk module installs, `pm uninstall`;
  - motor or sysfs writes;
  - any `tools/` script that isn't read-only.
- **Read-only is fine without asking:**
  - `tools/device-report.sh`, `tools/backup.sh` (it only reads), `getprop`, `getevent`, `dumpsys`, `fastboot getvar`;
  - `tools/hw-check.sh` without `--motor` (input capture);
  - `fastboot flashing get_unlock_ability`.
- **These write, so ask first:** `tools/hw-check.sh --motor` (moves the camera), `tools/grow-data.sh` (resizes /data), `tools/flash-boot.sh`, `tools/install-module.sh`.
- Destructive scripts call `confirm`, which fails without a TTY. Run them with `YES=1` **only** after that exact step was approved in chat.
- **Never write these partitions:**
  - `preloader*`, `lk*`, `nvram`, `nvdata`, `nvcfg`, `protect1`, `protect2`, `proinfo`, `persist`, `seccfg`, `frp`;
  - `otp`, `sec1`, `md_udc`, `para`, `boot_para`;
  - `md1img*`, `tee*`, `gz*`, `scp*`, `sspm*`, `spmfw*`, `dpm*`, `init_boot_*`, `vendor_boot_*`, `mmcblk0boot*`.
  - We only ever write `boot_*` (Magisk-patched from the device's own image), `dtbo_*`, `vbmeta*`, `system_*`, `product_*`, `super` and `userdata`.
- **Never run `fastboot flashing lock`** on modified images; it can brick the device.
- **Don't send the R1 into fastbootd** (`fastboot reboot fastboot`). On 2026-10-14 a trip there and back, with no writes, left an R1's slot b unbootable, and it needed a reflash to boot. The install writes `super` from the bootloader instead (ADR-0067, `crates/r1-usb/src/superimg.rs`).
- **Never replace Rabbit's kernel** (`boot_*` comes from Rabbit's firmware or from Magisk patching the device's own boot image). The scroll-wheel driver (`CONFIG_OCH1970`) isn't in Rabbit's released kernel source, so a self-built kernel loses the wheel (ADR-0001).
- **Don't let the R1 take a Rabbit OTA** once it's running our system.
- Before any wipe, a backup of this R1 must verify: the flash scripts' `require_backup` matches the serial at the start of its `proinfo` and checks its `SHA256SUMS` (in `backups/`, or the folder chosen in the companion, `Documents\r1os backups` until one is).
- `firmware/`, `backups/` and `out/` are git-ignored. Never commit them, upload them or paste their contents.
  - r1os's signing keys (`~/.r1os/keys`, `$KEYS`; ADR-0062) live outside the repo: never copy them into it, print them or upload them. Whoever holds `platform` has r1os's system powers.
  - Backups contain the IMEI and radio calibration.
  - Never print serial numbers or IMEIs into docs or chat. `tools/device-report.sh` redacts them.

## Agents and credentials

- r1os drives the **unmodified** `claude` and `codex` CLIs (ADR-0003).
- Never use the Claude Agent SDK with a subscription login; Anthropic doesn't allow it. Drive the `claude` CLI.
- Never pass `--bare`.
- r1os never stores long-lived Claude or OpenAI credentials. The CLIs own them inside the agent box.
  - The pairing flow may forward a one-time login code into a CLI's stdin. It must never log or persist it.
- **Hermes is the third brain (ADR-0070): the owner's own gateway, reached over its API (`crates/r1d/src/hermes.rs`), never run in the box.** It gets none of r1d's tools.
  - Its key is the one credential r1d keeps: in `<data>/hermes.json`, never `core.db` (a backup copies that, and promises no credentials), never logged, never sent back by the API.
  - Every `match` on a chat's brain must handle `hermes`: `chat.rs`'s `_` arm and `remote.rs`'s brain line run anything else as Claude.

## Building, running & testing

| Goal | Command |
|---|---|
| r1d tests + lint (host) | `cargo test -p r1d` · `cargo clippy -p r1d --all-targets -- -D warnings` |
| r1d for the R1 | `tools/build-r1d.sh` (NDK clang; it sets its own `RUSTFLAGS`, so a global `RUSTFLAGS` such as `+crt-static` can't break it) · lint: `CARGO_CMD=clippy tools/build-r1d.sh -- -D warnings` |
| Everything in Rust | `cargo test --workspace` · `cargo clippy --workspace --all-targets -- -D warnings` (as CI runs them; the companion's front end must be built first: `bun run --cwd companion build`). CI uses the newest stable Rust, whose clippy can flag what an older one passes: `rustup toolchain install stable` or `cargo +<version> clippy …` to match it |
| Shell | `bun run --cwd shell build` · `bun run --cwd shell typecheck` · hot reload: `bun run --cwd shell dev` (proxies to r1d on :7171) |
| Host app | `tools/build-host.sh` → `out/r1os-host.apk` (Gradle 9.4.1, AGP 9.2.1 built-in Kotlin), signed with r1os's platform key from `~/.r1os/keys` (made once by `tools/make-keys.sh`; the image uses the same keys, ADR-0062) |
| Desktop simulator | `tools/sim.sh`, then open http://127.0.0.1:7171 (↑/↓ = wheel, Space = button) · drive it: `R1H_TARGET=sim tools/r1h …` |
| Deploy to the R1 | `tools/deploy.sh [--host]`. It writes to the device: ask before the first deploy of a session. |
| Apps | `tools/r1h apps` · `tools/r1h install fixtures/apps/water.enee.yaml` · `tools/r1h create "<brief>"` (builds in the agent box on the R1; costs a little on the signed-in subscription) · `tools/r1h export <id> [file] [--data]` (`--data`: with its data and settings, which `install` puts back) · `tools/r1h remove <id>` |
| App build evals | `py -3 tools/eval-apps.py [--model haiku\|sonnet\|opus\|codex] [--brain claude\|codex] [--only 1,3] [--edits] [--keep]` (`--edits`: `fixtures/evals/edits.json`, an installed app changed and its data checked): builds `fixtures/evals/briefs.json` on the R1 and reports to `artifacts/evals/` (each build costs a little: it asks first, `YES=1` skips; it removes the apps after) |
| Look | The device's look: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/look` · change only what changes: `PUT /api/look {"mode": "light", "glance": {"clock": "dial"}}` (ADR-0043); the shell rebuilds itself in it. Agents' `eyes` and `wear` are in `PUT /api/agents/{id}` (the whole profile). New screens use `Theme` tokens, never literal colours (light mode) |
| Wardrobe | Agents' forms, eyes and everything they wear are one catalog, `shell/src/ui/wardrobe/wardrobe.json`, which the shell, the R1's host and r1d all read (ADR-0066). Check a change on every form in the sheet: `bun run --cwd shell dev`, then http://localhost:5171/wardrobe.html (`?rows=head&forms=orb,bot&pins`; `snapPng(cell, cols)` in its console for a sharp PNG). New items follow ADR-0066's drawing rules: opaque gradient stops, a contact shadow clipped to the body, `tone` or a material's colour |
| Widgets | The glance screen's, filled in: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/glance` · which and in what order: `PUT /api/glance {"apps": ["water-log", …]}` (ADR-0042). A widget is an app's `glance:` card; turn the wheel down on the glance screen for the next |
| Companion ([docs](web/src/docs/content/companion.md)) | `bun run --cwd companion tauri build --no-bundle` → `target/release/r1os-companion.exe` (Tauri 2; its key and trusted R1s in `%APPDATA%\r1os\companion`) · its window without Tauri, with a demo R1: `bun run --cwd companion dev` (http://localhost:1420) · the link's test program: `cargo run -p r1-link --bin r1-link-pc -- pair` |
| Stock R1 backup | `cargo run -p r1-usb --example backup -- status \| fetch \| run \| verify <folder> <identity> \| latest <identity> \| gpt <file>`: a booted R1 with root over adb at USB speed; otherwise the R1's own partitions and boot chain at power-on with mtkclient's `rl` (it and uv pinned, in `%LOCALAPPDATA%\r1os\companion\backup-tools`; on Windows over Rabbit's MediaTek serial port driver, `drivers -- install --mtk`), into the folder chosen in the companion (`Documents\r1os backups` until one is; `R1OS_BACKUPS` names another) as `<identity>\<date>` with `SHA256SUMS`, each partition checked against the R1's GPT. `run` only reads, but the preloader route needs the person's hands (the R1 powered off, then plugged in): ask first. Fetch the tools from the terminal that will run them (see the Windows notes) |
| Flash (ADR-0067) | `cargo run -p r1-usb --example flash -- stock \| read-r1 \| status \| check \| run <system .img> \| super-head \| super-build \| super-probe`: everything from the bootloader, never fastbootd. Only `run` writes (vbmeta, `super` with the R1's own vendor kept, then the wipe), after the plan and a typed `ERASE`. It hasn't run on an R1 yet: don't, without explicit approval and a backup |
| Website | `bun run --cwd web dev` (http://localhost:5180) · `bun run --cwd web build` → `web/dist` · `bun run --cwd web typecheck` · served as Railway does: `bun web/serve.ts` (`web/dist`, on `$PORT`); Railway builds `web/Dockerfile` from the repo's root (`railway.json`; `docker build -f web/Dockerfile .` to try it). Docs pages are `web/src/docs/content/*.md`; the r1os skill for agents is `web/public/skills/r1os/`: when a rule or gotcha here changes, change the skill too |
| Agent box | `tools/agent-box.sh install \| status \| codex-host \| run -- CMD \| root -- CMD` (`codex-host` adds Codex's code-mode host, without which Codex's tools fail closed) |
| Drive the agent | `py -3 tools/agent-test.py [codex] "message" …`: a new chat through r1d's API, printing tools, timings and replies (spends a little; delete the chats after) |
| Hermes brain | `POST /api/accounts/hermes/login {url, key}` adds the owner's gateway ([docs](web/src/docs/content/extend.md)); `POST /api/chats {agent, brain: "hermes"}`. Works in the simulator, which has no box. Marks: `reply.first_word.hermes`, `reply.done.hermes` |
| Drive the R1 | `tools/r1h state \| events \| wheel N \| button tap\|double\|hold MS \| launch <app-id> \| screenshot \| logs` (`motor` moves the camera: ask first). It forwards local port 7172 to the R1's 7171, so the simulator can keep 7171. |
| Measure | `tools/r1h power [seconds] [--json]`: over a window (default 60 s), the battery used (unplug the R1 and use Wi-Fi adb for a drain figure), CPU per process, suspends and wakeups (`tools/power.py` reads the device's own counters) · `tools/r1h perf [reset]`: r1d's latency marks (`/harness/perf`: STT load, release → final words, first reply word warm or cold, whole replies, release → first reply word, app window steps as `app.<op>`) and the host's frame times. Measure before and after any change to latency, frames or energy |
| r1d by hand | On the R1, `/api`, `/harness` and `/ws` need r1d's per-boot token (ADR-0026): `T=$(source tools/lib.sh; r1d_token)`, then `curl -H "Authorization: Bearer $T" …` (after any `tools/r1h` call sets up the forward). r1d makes a new one each start. The simulator is open. |
| Speech-to-text | `tools/fetch-speech.sh` (once, needs `uv`: sherpa-onnx's Android libs, the Kroko model, and its `.ort` copy for the R1 into `out/speech`; needed by `tools/build-r1d.sh`) · state: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/voice` · benchmark: `curl -s -H "Authorization: Bearer $T" -X POST -d '{"path":"/data/r1os/models/stt/test.wav"}' -H 'Content-Type: application/json' 127.0.0.1:7172/harness/hear` · drive the UI without a mic: `POST /harness/say {"text": "…"}` |
| Check the agent jail | `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/harness/jail`: expect uid 2950, `context: u:r:r1d_agent:s0` (image v0.2.7+), `CapBnd: 0000000000000000`, `NoNewPrivs: 1`, `Seccomp: 2` |
| On-device models | `tools/fetch-models.sh` (System One's embedder into `out/system1`; the Translator's ~2.2 GB into `out/translate`, pinned in `tools/translator-models.txt`; `TRANSLATOR=0` skips it). `tools/deploy.sh` pushes what's there, once. Their licences are in NOTICE.md |
| System One | `POST /harness/system1 {"text": "…"}`: what it makes of some words, without saying them (ADR-0032) |
| Translator | `POST /harness/translate {"path": "/data/r1os/models/stt/x.wav", "theirs": "es"}` (16 kHz WAV; `cp`, not `mv`, it there from `/data/local/tmp`: from image v0.2.8 r1d can't read that directory): the whole pipeline timed by stage; `"live": true` feeds the open Translator screen instead. Typed: `POST /api/translator/text {"text", "from", "to", "speak"}` (ADR-0034) |
| TTS benchmark | `POST /harness/tts {dir, texts, threads?, sid?, steps?, provider?, out?}` times a sherpa-onnx voice package on the R1; `/harness/paradee {model, ids, threads}` does Paradee-8M. Voice output itself is deferred |
| Android apps | `tools/fetch-aurora.sh` (once before an image build: Aurora Store's pinned `preload` APK into `out/aurora`, its certificate checked) · list: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/android/apps` · open one, or Aurora on its page: `POST /api/android/open {"package": "…"}` / `{"store": "…"}` (`true` for Aurora's home) |
| Image extras | Before an image build: `tools/build-bootlog.sh` (verbose boot's program into `out/bootlog`) and, with an R1 running r1os, `tools/r1h bootanim default` (r1os's own boot animation into `out/bootanim`, which the host draws on the R1, and its still into `device/aosp/bootanimation-still.png`: commit it if it changed). Without `out/bootanim`, as on a first build, the image gets that still. Verbose boot: `PUT /api/boot {"verbose": true}`. The host builds `release` by default and `tools/deploy.sh --host` compiles it ahead of time |
| Edge swipes | Android's own swipe from an edge is off (ADR-0059): an image from now on has it built in; an R1 set up before gets `tools/edges-overlay.sh` (a framework overlay signed with the image's platform key; it writes to the device, so ask; `off` undoes it). Drive r1os's swipes with `adb shell input swipe 240 5 240 420 350` (the glance screen down) or `240 636 240 150` on home (the launcher) |
| SELinux policy | `wsl -d r1os-build -u root -- bash "$ROOT/tools/build-image.sh" layer policy`: just the policy, with AOSP's neverallow checks (~7 min); then `layer build export` for the image (ADR-0035) |
| Third-party notices | `py -3 tools/notices.py` → `out/notices/*.txt`: every package r1d, r1-bootlog, the companion, the shell and the site ship, with the licence texts they carry. It names copyleft licences and packages with no text, to look at before a release |
| WebView inspection | `chrome://inspect` lists r1os's pages on today's userdebug image whatever the host asks: Android's WebView keeps debugging on for every app on a debug build and ignores `setWebContentsDebuggingEnabled(false)`. On a user build, only a debuggable host, or `adb shell setprop persist.r1os.webview_debug 1` (root) and a host restart |

- The app platform (storage, hosting, the build pipeline) is described in [Apps](web/src/docs/content/apps.md).
  - `prompts/example.enee.yaml` must equal builder.md's example and compile with passing checks (`cargo test -p r1d`).
  - Keep prompts in step with what r1d runs: no services, triggers or side-effect steps until r1d supports them.
- **Files put in `/data/r1os` by hand need `restorecon -RF <path>` afterwards** (as root). From image v0.2.7 its labels matter to r1d's SELinux domain, and files moved in from `/data/local/tmp` keep that directory's label, which init may not fix (ADR-0035). `tools/deploy.sh` and `tools/agent-box.sh` do it themselves.
- **Never `chown -R`/`chmod -R` all of `/data/r1os`.** `agent/` holds the agent user's home (uid 2950, its CLI logins), the box, and `agent/agents/` (root-only; each agent's workspace and chat stores). A recursive chown broke it on 2026-10-08.
- The protocol and gesture rules are in [API](web/src/docs/content/api.md) and `crates/r1d`. Keep the docs, `shell/src/r1.ts` types and `crates/r1d` in sync.
- **The shell reads input only from r1d's `/ws` events, never DOM key events.** The host app swallows DPAD/BUTTON_1/BACK.
- **Battery comes first** ([How it's built](web/src/docs/content/architecture.md)).
  - Don't add timers, polling or animations that keep running with the screen off.
  - Prefer `/ws` events to polling.
  - Measure before and after when a change touches latency, frame time or energy.
- **Third-party Android apps are allowed**: r1d's API and mic take a token (ADR-0026). Installing one is still a device write: ask first.
- **Testing on the R1:**
  - The display sleeps after 60 s, and neither harness input nor `tools/r1h screen on` counts as activity once it's on. Poke it before each step with a harmless key: `MSYS_NO_PATHCONV=1 adb shell input keyevent 59` (Shift).
  - A sleeping R1 wakes to the glance screen, which swallows `launch` until a double click goes home. On home, a double click opens the agent's space instead: take a screenshot before blind sequences.
  - Before driving it, check `tools/r1h events` for recent `hw` input: its owner may be using it, and their input mixes with yours.
  - After a reboot adbd runs as the shell user: `adb root` before the tools, which read r1d's token as root.
  - **A hold listens with the real microphone**: whatever is said in the room goes to the agent's current chat, which is the owner's own. Script words with `POST /harness/say` in a chat you started for the test (New chat in a space), and delete that chat after. To show the overlay without sending, cancel with **two** wheel ticks while held (the first is forgiven).
- When running r1d on the device by hand, redirect every fd (`</dev/null >>log 2>&1 &`), or `adb shell` hangs on the child.
- Use `pkill -x r1d`, not `pkill -f r1d`: `-f` matches the `adb shell` command line itself.

## Windows notes

- Scripts in `tools/` are bash for **Git Bash** on Windows. They use `uv` and Python 3.11 (`py -3`); don't use `jq` in them.
- `tools/lib.sh` exports `MSYS_NO_PATHCONV=1`. Without it, Git Bash rewrites `/proc/version` into `C:/Program Files/Git/proc/version` before adb sees it.
  - When running adb by hand in Git Bash, prefix it: `MSYS_NO_PATHCONV=1 adb shell cat /proc/version`.
- Platform-tools are usually at `%LOCALAPPDATA%\Android\Sdk\platform-tools`, with the NDK in the same SDK.
- **Over Wi-Fi:** with no USB device attached, the tools connect to the R1 by itself over wireless debugging (`adb_ready` in `tools/lib.sh` finds it by mDNS; the port changes whenever wireless debugging restarts).
  - A PC pairs once: run `adb pair <ip>:<port> <code>` from the R1's "Pair device with pairing code" screen.
  - If several devices are attached, `ANDROID_SERIAL` picks one; otherwise USB wins.
  - Fastboot is USB only.
- **In the Claude desktop app, your writes under `AppData\Local` are redirected** to `AppData\Local\Packages\Claude_…\LocalCache\Local` (it's a packaged app), which the person's own terminal doesn't see. Junctions made there name the real path and dangle. Anything they'll run from their terminal (the backup's tools, drivers), they fetch there. `AppData\Local\Temp` and the repo aren't redirected.
- **Fastboot on Windows:**
  - The R1's bootloader enumerates as `0E8D:201C`, the same ID as Android's adb (`REV_0100`, class `FF/42/03`; booted it's `REV_0223`, `FF/42/01`), and Windows has nothing to match it with.
  - r1os's own driver binds it in both modes, every time: `cargo run -p r1-usb --example drivers -- install` (or the companion's wizard). `-- status` reads whether it's there; `-- undo` takes it out.
  - When `fastboot devices` is empty and Device Manager shows "Android" ⚠ (code 28), it isn't installed: the install shows Windows' admin prompt, so the person runs it. Agents don't change Windows drivers.
  - Prefer `tools/flash-boot.sh`'s on-device dd path (root over adb) for boot writes.
- Entering fastboot from power-off goes through the MediaTek preloader's USB serial port.
  - That needs the MediaTek preloader VCOM driver, which Rabbit publishes in the `drivers` release of `rabbit-hmi-oss/firmware`.
  - From a booted system, `adb reboot bootloader` is simpler.
