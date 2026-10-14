# CLAUDE.md: working on r1os

r1os turns a Rabbit R1 into an agent-first handheld. Rabbit's kernel and vendor stay as the hardware
layer. On top sit our own Android system image, a Rust core daemon (`r1d`), a thin Kotlin host app
with a WebView shell, and the unmodified `claude` / `codex` CLIs in an Alpine chroot. Read
[docs/architecture.md](docs/architecture.md) first.

## Docs are the source of truth

- Same conventions as eneeware:
  - Behavior changes update the spec or architecture doc in the same change.
  - Hard-to-reverse choices get an ADR (`docs/decisions/0000-template.md`, next number).
  - Finished roadmap items are checked off in `docs/roadmap.md`.
  - Unresolved items go in `docs/open-questions.md`.
- Never edit an accepted ADR's decision. Supersede it with a new ADR.
- Research notes in `docs/research/` are dated snapshots. Append a dated update section instead of rewriting, and mark unverified claims **(unverified)**.

## Tracking work: the Obsidian vault

`docs/` is an Obsidian vault, and it is the one place r1os's work is tracked ([docs/README.md](docs/README.md)). Plans, tasks, the backlog, decisions, research and open questions live there as notes: not in chat, scratch files, code comments or TODO lists.

- **Where things go:**
  - A milestone is an epic note in `docs/kanban/epics/` (`R<N> Theme`, from `docs/templates/epic.md`), with its section and table row in `docs/roadmap.md`.
  - A piece of work is a task note in `docs/kanban/tasks/` (`T-NNN Short title`, the next free number, from `docs/templates/task.md`), with a card on `docs/kanban/board.md` and a link from its roadmap item.
  - Decisions, research and unresolved questions stay ADRs, dated research notes and rows in `docs/open-questions.md`; tasks link to them.
- **Before starting work, find its task**, or make one. Move its card to In Progress and set `status: in-progress`. Work Max asks for in chat gets a task too, unless it's a few minutes' fix.
- **When it's done:**
  - tick its *Done when* boxes, set `status: done` and `done:` to the date;
  - move the card to Complete;
  - update the epic's *Where it stands*;
  - check the roadmap item off with the commit, and name the task in the commit message (`T-012`).
- **Work you find but won't do now** (a bug, a follow-up, a check on the R1) becomes a Backlog task, not a line in chat or a TODO in code.
- **Every task has:**
  - `epic`;
  - `priority`: P0 now · P1 next · P2 soon · P3 later or parked;
  - `effort`: XS hours · S a day or two · M up to a week · L one to two weeks · XL split it;
  - `area`, and `needs_max` for a decision, a device write, his accounts or his hands.
  - Its card carries the same as tags (`#p1 #effort/s #needs-max`). Keep each lane in priority, then effort, order.
- **Keep the notes true:**
  - a card's lane matches its note's `status`;
  - an ADR's `status` property matches its Status line;
  - new docs get `type` and `tags` properties (ADRs also `id`, `status`, `date` and the `ADR-NNNN` alias).
- **Links** stay relative markdown links, so GitHub and agents can follow them. Use wikilinks only in properties and on the board.
- **Tools:** use the Obsidian MCP server when it's connected; otherwise edit the files directly (they're plain markdown). Never commit `docs/.obsidian/workspace.json`, plugin code, or plugin settings that hold keys.

## Device safety (hard rules)

The R1 has no volume keys and no recovery menu. A bad write to the wrong partition can hard-brick it.

- **Ask Max in chat before anything that writes to the device**, naming the exact command or script. That covers:
  - `fastboot flash/erase/set_active/reboot fastboot` and `fastboot -w`;
  - `dd of=…`, Magisk module installs, `pm uninstall`;
  - motor or sysfs writes;
  - any `tools/` script that isn't read-only.
- **Read-only is fine without asking:**
  - `tools/device-report.sh`, `tools/backup.sh` (it only reads), `getprop`, `getevent`, `dumpsys`, `fastboot getvar`;
  - `tools/hw-check.sh` without `--motor` (input capture);
  - `fastboot flashing get_unlock_ability`.
- **These write, so ask first:** `tools/hw-check.sh --motor` (moves the camera), `tools/grow-data.sh` (resizes /data), `tools/flash-boot.sh`, `tools/install-module.sh`.
- Destructive scripts call `confirm`, which fails without a TTY. Run them with `YES=1` **only** after Max approved that exact step in chat.
- **Never write these partitions:**
  - `preloader*`, `lk*`, `nvram`, `nvdata`, `nvcfg`, `protect1`, `protect2`, `proinfo`, `persist`, `seccfg`, `frp`;
  - `otp`, `sec1`, `md_udc`, `para`, `boot_para`;
  - `md1img*`, `tee*`, `gz*`, `scp*`, `sspm*`, `spmfw*`, `dpm*`, `init_boot_*`, `vendor_boot_*`, `mmcblk0boot*`.
  - We only ever write `boot_*` (Magisk-patched from the device's own image), `dtbo_*`, `vbmeta*`, `system_*`, `product_*`, `super` and `userdata`.
- **Never run `fastboot flashing lock`** on modified images; it can brick the device.
- **Don't send the R1 into fastbootd** (`fastboot reboot fastboot`). On 2026-10-14 a trip there and back, with no writes, left slot b unbootable, and the R1 needed a reflash to boot ([T-182](docs/kanban/tasks/T-182%20A%20trip%20through%20fastbootd%20marked%20slot%20b%20unbootable.md)). The install writes `super` from the bootloader instead ([ADR-0067](docs/decisions/0067-the-install-writes-super-from-the-bootloader.md)).
- **Never replace Rabbit's kernel** (`boot_*` comes from Rabbit's firmware or from Magisk patching the device's own boot image). The scroll-wheel driver (`CONFIG_OCH1970`) isn't in Rabbit's released kernel source, so a self-built kernel loses the wheel ([ADR-0001](docs/decisions/0001-rabbit-kernel-and-vendor-our-own-system-image.md)).
- **Don't let the R1 take a Rabbit OTA** once it's running our system.
- Before any wipe, a backup of this R1 must verify: the flash scripts' `require_backup` matches the serial at the start of its `proinfo` and checks its `SHA256SUMS` (in `backups/`, or the companion's `Documents\r1os backups`).
- `firmware/`, `backups/` and `out/` are git-ignored. Never commit them, upload them or paste their contents.
  - r1os's signing keys (`C:\Users\<you>\.r1os\keys`, ADR-0062) live outside the repo: never copy them into it, print them or upload them. Whoever holds `platform` has r1os's system powers.
  - Backups contain the IMEI and radio calibration.
  - Never print serial numbers or IMEIs into docs or chat. `tools/device-report.sh` redacts them.

## Agents and credentials

- r1os drives the **unmodified** `claude` and `codex` CLIs ([ADR-0003](docs/decisions/0003-agents-in-an-alpine-chroot-driving-the-clis.md)).
- Never use the Claude Agent SDK with a subscription login; Anthropic doesn't allow it. Drive the `claude` CLI.
- Never pass `--bare`.
- r1os never stores long-lived Claude or OpenAI credentials. The CLIs own them inside the agent box.
  - The pairing flow may forward a one-time login code into a CLI's stdin. It must never log or persist it.

## Building, running & testing

| Goal | Command |
|---|---|
| r1d tests + lint (host) | `cargo test -p r1d` · `cargo clippy -p r1d --all-targets -- -D warnings` |
| r1d for the R1 | `tools/build-r1d.sh` (NDK clang; it sets its own `RUSTFLAGS` because Max's global `RUSTFLAGS=+crt-static` would break it) · lint: `CARGO_CMD=clippy tools/build-r1d.sh -- -D warnings` |
| Shell | `pnpm --dir shell build` · `pnpm --dir shell typecheck` · hot reload: `pnpm --dir shell dev` (proxies to r1d on :7171) |
| Host app | `tools/build-host.sh` → `out/r1os-host.apk` (Gradle 9.4.1, AGP 9.2.1 built-in Kotlin; same cached toolchain as ~/boop-android), signed with r1os's platform key from `C:\Users\<you>\.r1os\keys` (made once by `tools/make-keys.sh`; the image uses the same keys, [ADR-0062](docs/decisions/0062-r1os-signs-its-image-and-host-with-its-own-keys.md)) |
| Desktop simulator | `tools/sim.sh`, then open http://127.0.0.1:7171 (↑/↓ = wheel, Space = button) · drive it: `R1H_TARGET=sim tools/r1h …` |
| Deploy to the R1 | `tools/deploy.sh [--host]`. It writes to the device: ask Max before the first deploy of a session. |
| Apps | `tools/r1h apps` · `tools/r1h install fixtures/apps/water.enee.yaml` · `tools/r1h create "<brief>"` (builds in the agent box on the R1; costs a little on Max's subscription) · `tools/r1h export <id> [file] [--data]` (`--data`: with its data and settings, which `install` puts back) · `tools/r1h remove <id>` |
| App build evals | `py -3 tools/eval-apps.py [--model haiku\|sonnet\|opus\|codex] [--brain claude\|codex] [--only 1,3] [--edits] [--keep]` (`--edits`: `fixtures/evals/edits.json`, an installed app changed and its data checked): builds `fixtures/evals/briefs.json` on the R1 and reports to `artifacts/evals/` (each build costs a little: it asks first, `YES=1` skips; it removes the apps after) |
| Look | The device's look: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/look` · change only what changes: `PUT /api/look {"mode": "light", "glance": {"clock": "dial"}}` ([ADR-0043](docs/decisions/0043-the-look-is-yours.md)); the shell rebuilds itself in it. Agents' `eyes` and `wear` are in `PUT /api/agents/{id}` (the whole profile). New screens use `Theme` tokens, never literal colours (light mode) |
| Wardrobe | Agents' forms, eyes and everything they wear are one catalog, `shell/src/ui/wardrobe/wardrobe.json`, which the shell, the R1's host and r1d all read ([ADR-0066](docs/decisions/0066-agents-looks-come-from-one-wardrobe.md); format in [experience.md §3.1](docs/specs/experience.md#31-the-wardrobe)). Check a change on every form in the sheet: `pnpm --dir shell dev`, then http://localhost:5171/wardrobe.html (`?rows=head&forms=orb,bot&pins`; `snapPng(cell, cols)` in its console for a sharp PNG). New items follow ADR-0066's drawing rules: opaque gradient stops, a contact shadow clipped to the body, `tone` or a material's colour |
| Widgets | The glance screen's, filled in: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/glance` · which and in what order: `PUT /api/glance {"apps": ["water-log", …]}` ([ADR-0042](docs/decisions/0042-widgets-are-apps-glance-cards.md)). A widget is an app's `glance:` card; turn the wheel down on the glance screen for the next |
| Companion ([spec](docs/specs/companion.md)) | `pnpm --dir companion tauri build --no-bundle` → `target/release/r1os-companion.exe` (Tauri 2; its key and trusted R1s in `%APPDATA%\r1os\companion`) · its window without Tauri, with a demo R1: `pnpm --dir companion dev` (http://localhost:1420) · the link's test program: `cargo run -p r1-link --bin r1-link-pc -- pair` |
| Stock R1 backup (T-169) | `cargo run -p r1-usb --example backup -- status \| fetch \| run \| verify <folder> <identity> \| latest <identity> \| gpt <file>`: the R1's own partitions and boot chain at power-on with mtkclient's `rl` (it and uv pinned, in `%LOCALAPPDATA%\r1os\companion\backup-tools`; on Windows over Rabbit's MediaTek serial port driver, `drivers -- install --mtk`), into `Documents\r1os backups\<identity>\<date>` with `SHA256SUMS`, each partition checked against the R1's GPT. `run` only reads, but needs Max's hands (the R1 powered off, then plugged in): ask first. Max fetches the tools from his own terminal (see the Windows notes) |
| Website ([R9](docs/kanban/epics/R9%20The%20website.md)) | `pnpm --dir web dev` (http://localhost:5180) · `pnpm --dir web build` → `web/dist` · `pnpm --dir web typecheck`. Docs pages are `web/src/docs/content/*.md`; the r1os skill for agents is `web/public/skills/r1os/`: when a rule or gotcha here changes, change the skill too |
| Agent box | `tools/agent-box.sh install \| status \| codex-host \| run -- CMD \| root -- CMD` (`codex-host` adds Codex's code-mode host, without which Codex's tools fail closed) |
| Drive the agent | `py -3 tools/agent-test.py [codex] "message" …`: a new chat through r1d's API, printing tools, timings and replies (spends a little; delete the chats after) |
| Drive the R1 | `tools/r1h state \| events \| wheel N \| button tap\|double\|hold MS \| launch <app-id> \| screenshot \| logs` (`motor` moves the camera: ask first). It forwards local port 7172 to the R1's 7171, so the simulator can keep 7171. |
| Measure | `tools/r1h power [seconds] [--json]`: over a window (default 60 s), the battery used (unplug the R1 and use Wi-Fi adb for a drain figure), CPU per process, suspends and wakeups (`tools/power.py` reads the device's own counters) · `tools/r1h perf [reset]`: r1d's latency marks (`/harness/perf`: STT load, release → final words, first reply word warm or cold, whole replies, release → first reply word, app window steps as `app.<op>`) and the host's frame times. Measure before and after any change to latency, frames or energy |
| r1d by hand | On the R1, `/api`, `/harness` and `/ws` need r1d's per-boot token ([ADR-0026](docs/decisions/0026-r1d-takes-a-per-boot-token.md)): `T=$(source tools/lib.sh; r1d_token)`, then `curl -H "Authorization: Bearer $T" …` (after any `tools/r1h` call sets up the forward). r1d makes a new one each start. The simulator is open. |
| Speech-to-text | `tools/fetch-speech.sh` (once, needs `uv`: sherpa-onnx's Android libs, the Kroko model, and its `.ort` copy for the R1 into `out/speech`; needed by `tools/build-r1d.sh`) · state: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/voice` · benchmark: `curl -s -H "Authorization: Bearer $T" -X POST -d '{"path":"/data/r1os/models/stt/test.wav"}' -H 'Content-Type: application/json' 127.0.0.1:7172/harness/hear` · drive the UI without a mic: `POST /harness/say {"text": "…"}` |
| Check the agent jail | `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/harness/jail`: expect uid 2950, `context: u:r:r1d_agent:s0` (image v0.2.7+), `CapBnd: 0000000000000000`, `NoNewPrivs: 1`, `Seccomp: 2` |
| On-device models | `tools/fetch-models.sh` (System One's embedder into `out/system1`; the Translator's ~2.2 GB into `out/translate`, pinned in `tools/translator-models.txt`; `TRANSLATOR=0` skips it). `tools/deploy.sh` pushes what's there, once |
| System One | `POST /harness/system1 {"text": "…"}`: what it makes of some words, without saying them ([ADR-0032](docs/decisions/0032-system-one-on-the-device.md)) |
| Translator | `POST /harness/translate {"path": "/data/r1os/models/stt/x.wav", "theirs": "es"}` (16 kHz WAV; `cp`, not `mv`, it there from `/data/local/tmp`: from image v0.2.8 r1d can't read that directory): the whole pipeline timed by stage; `"live": true` feeds the open Translator screen instead. Typed: `POST /api/translator/text {"text", "from", "to", "speak"}` ([ADR-0034](docs/decisions/0034-the-translator.md)) |
| TTS benchmark | `POST /harness/tts {dir, texts, threads?, sid?, steps?, provider?, out?}` times a sherpa-onnx voice package on the R1; `/harness/paradee {model, ids, threads}` does Paradee-8M. Voice output itself is deferred ([research](docs/research/2026-10-10-on-device-tts.md)) |
| Android apps | `tools/fetch-aurora.sh` (once before an image build: Aurora Store's pinned `preload` APK into `out/aurora`, its certificate checked) · list: `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/android/apps` · open one, or Aurora on its page: `POST /api/android/open {"package": "…"}` / `{"store": "…"}` (`true` for Aurora's home) |
| Image extras | Before an image build: `tools/build-bootlog.sh` (verbose boot's program into `out/bootlog`, T-101) and `tools/r1h bootanim default` (r1os's own boot animation into `out/bootanim`, T-029; the host draws it on the R1). Verbose boot: `PUT /api/boot {"verbose": true}`. The host builds `release` by default and `tools/deploy.sh --host` compiles it ahead of time (T-100) |
| Edge swipes | Android's own swipe from an edge is off (T-127, ADR-0059): an image from now on has it built in; an R1 set up before gets `tools/edges-overlay.sh` (a framework overlay signed with the image's platform key; it writes to the device, so ask; `off` undoes it). Drive r1os's swipes with `adb shell input swipe 240 5 240 420 350` (the glance screen down) or `240 636 240 150` on home (the launcher) |
| SELinux policy | `wsl -d r1os-build -u root -- bash ".../tools/build-image.sh" layer policy`: just the policy, with AOSP's neverallow checks (~7 min); then `layer build export` for the image ([ADR-0035](docs/decisions/0035-real-selinux-rules-for-r1d-and-the-agent-box.md)) |

- The app platform (storage, hosting, the build pipeline) is in [docs/specs/apps.md](docs/specs/apps.md).
  - `prompts/example.enee.yaml` must equal builder.md's example and compile with passing checks (`cargo test -p r1d`).
  - Keep prompts in step with what r1d runs: no services, triggers or side-effect steps until r1d supports them.
- **Files put in `/data/r1os` by hand need `restorecon -RF <path>` afterwards** (as root). From image v0.2.7 its labels matter to r1d's SELinux domain, and files moved in from `/data/local/tmp` keep that directory's label, which init may not fix (ADR-0035). `tools/deploy.sh` and `tools/agent-box.sh` do it themselves.
- **Never `chown -R`/`chmod -R` all of `/data/r1os`.** `agent/` holds the agent user's home (uid 2950, its CLI logins), the box, and `agent/agents/` (root-only; each agent's workspace and chat stores). A recursive chown broke it on 2026-10-08.
- The protocol and gesture rules are in [docs/specs/r1d.md](docs/specs/r1d.md). Keep the spec, `shell/src/r1.ts` types and `crates/r1d` in sync.
- **The shell reads input only from r1d's `/ws` events, never DOM key events.** The host app swallows DPAD/BUTTON_1/BACK.
- **Battery comes first** ([docs/architecture.md §7](docs/architecture.md#7-power-and-performance)).
  - Don't add timers, polling or animations that keep running with the screen off.
  - Prefer `/ws` events to polling.
  - Measure before and after when a change touches latency, frame time or energy.
- **Third-party Android apps are allowed** (Max, 2026-10-09, Q32): r1d's API and mic take a token ([ADR-0026](docs/decisions/0026-r1d-takes-a-per-boot-token.md)). Installing one is still a device write: ask Max first.
- **Testing on the R1:**
  - The display sleeps after 60 s, and neither harness input nor `tools/r1h screen on` counts as activity once it's on. Poke it before each step with a harmless key: `MSYS_NO_PATHCONV=1 adb shell input keyevent 59` (Shift).
  - A sleeping R1 wakes to the glance screen, which swallows `launch` until a double click goes home. On home, a double click opens the agent's space instead: take a screenshot before blind sequences.
  - Before driving it, check `tools/r1h events` for recent `hw` input: Max may be using it, and his input mixes with yours.
  - After a reboot adbd runs as the shell user: `adb root` before the tools, which read r1d's token as root.
  - **A hold listens with the real microphone**: whatever is said in the room goes to the agent's current chat, which is Max's own (ADR-0057). Script words with `POST /harness/say` in a chat you started for the test (New chat in a space), and delete that chat after. To show the overlay without sending, cancel with **two** wheel ticks while held (the first is forgiven).
- When running r1d on the device by hand, redirect every fd (`</dev/null >>log 2>&1 &`), or `adb shell` hangs on the child.
- Use `pkill -x r1d`, not `pkill -f r1d`: `-f` matches the `adb shell` command line itself.

## Windows dev machine notes

- Max's PC runs Windows 10. Scripts in `tools/` are bash for **Git Bash**.
- `tools/lib.sh` exports `MSYS_NO_PATHCONV=1`. Without it, Git Bash rewrites `/proc/version` into `C:/Program Files/Git/proc/version` before adb sees it.
  - When running adb by hand in Git Bash, prefix it: `MSYS_NO_PATHCONV=1 adb shell cat /proc/version`.
- Platform-tools are at `%LOCALAPPDATA%\Android\Sdk\platform-tools` (adb/fastboot 37). The NDK is in the same SDK.
- **Over Wi-Fi:** with no USB device attached, the tools connect to the R1 by itself over wireless debugging (`adb_ready` in `tools/lib.sh` finds it by mDNS; the port changes whenever wireless debugging restarts).
  - This PC is already paired. Another PC pairs once: Max runs `adb pair <ip>:<port> <code>` from the R1's "Pair device with pairing code" screen.
  - If several devices are attached, `ANDROID_SERIAL` picks one; otherwise USB wins.
  - Fastboot is USB only.
- `uv` and Python 3.11 (`py -3`) are available. `jq` isn't, so don't use it in scripts.
- **In the Claude desktop app, your writes under `AppData\Local` are redirected** to `AppData\Local\Packages\Claude_…\LocalCache\Local` (it's a packaged app), which Max's own terminal doesn't see. Junctions made there name the real path and dangle. For anything Max will run from his terminal (the backup's tools, drivers), he fetches it there. `AppData\Local\Temp` and the repo aren't redirected.
- **Fastboot on Windows:**
  - The R1's bootloader enumerates as `0E8D:201C`, the same ID as Android's adb (`REV_0100`, class `FF/42/03`; booted it's `REV_0223`, `FF/42/01`), and Windows has nothing to match it with.
  - r1os's own driver binds it in both modes, every time: `cargo run -p r1-usb --example drivers -- install` (or the companion's wizard; T-168, installed on Max's PC 2026-10-14). `-- status` reads whether it's there; `-- undo` takes it out.
  - When `fastboot devices` is empty and Device Manager shows "Android" ⚠ (code 28), it isn't installed: the install shows Windows' admin prompt, so the user runs it. Agents don't change Windows drivers.
  - Prefer `tools/flash-boot.sh`'s on-device dd path (root over adb) for boot writes.
- Entering fastboot from power-off goes through the MediaTek preloader's USB serial port.
  - That needs the MediaTek preloader VCOM driver, which Rabbit publishes in the `drivers` release of `rabbit-hmi-oss/firmware`.
  - From a booted system, `adb reboot bootloader` is simpler.
