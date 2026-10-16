# Workflow: tracking work and verifying it

How work is planned and recorded in r1os's docs vault, how commits are written, and how to check a change in each layer.

**The vault is the maintainers' own** (`docs/`, git-ignored, never published). The tracking sections below apply when it's there (`docs/maintainer.md` exists); otherwise use issues and pull requests (CONTRIBUTING.md). The verifying sections apply to everyone.

## The docs vault

**The Obsidian vault is the only tracker** (`docs/README.md`, CLAUDE.md). Plans, tasks, backlog, decisions, research and open questions live in `docs/`: never in chat, scratch files, code comments or TODOs.
- **Epic:** `docs/kanban/epics/R<N> Theme.md` from `docs/templates/epic.md`, with a section and table row in `docs/roadmap.md`. Epics run from R0 to R9, with half steps (R4.5…); R9 is the website.
- **Task:** `docs/kanban/tasks/T-NNN Short title.md` (the next free number) from `docs/templates/task.md`, a card on `docs/kanban/board.md` (`- [ ] [[T-NNN Title]] #p1 #effort/s #needs-max`), linked from its roadmap item. Properties: `type: task`, `id`, `epic: "[[R… …]]"`, `status` (backlog | in-progress | done), `priority` (P0 now · P1 next · P2 soon · P3 later), `effort` (XS hours · S 1–2 days · M ≤ a week · L 1–2 weeks · XL split it), `area` (r1d, host, shell, image, companion, prompts, tools, docs, device, web), `needs_max` (the property's name: needs the owner for a decision, a device write, their accounts or hands), `created`, `done`. Body: *Why*, *Done when* checkboxes, *Notes* (roadmap item, related ADRs/specs/research; "In the code (date)" pointers).
- **Before starting:** find or create the task; move its card to In Progress; `status: in-progress`. Work the owner asks for gets a task unless it's a few minutes' fix.
- **When done:** tick *Done when*; `status: done`, `done: <date>`; card to Complete; update the epic's *Where it stands*; check off the roadmap item with the date (and commit); name the task in the commit.
- **Found work you won't do now** (bug, follow-up, a check on the R1) → a Backlog task.
- **Board hygiene:** a card's lane matches its note's `status`; lanes sorted by priority, then effort.
- **ADRs:** `docs/decisions/NNNN-slug.md` from `0000-template.md`, properties `type: adr`, `id`, `status`, `date`, alias `ADR-NNNN`, `superseded_in_part` when later ADRs changed part. Never edit an accepted ADR's decision; supersede it with a new ADR and note it in the old one's Status line. Keep the `status` property and Status line in step. ADRs record the decider's words and the date.
- **Open questions:** rows in `docs/open-questions.md` (`Q<N>`, how we'll answer, milestone); resolved ones move to "Resolved" with evidence.
- **Research:** `docs/research/YYYY-MM-DD-slug.md`, dated snapshots. Append `## Update YYYY-MM-DD (…): …` sections; never rewrite. Mark unverified claims **(unverified)**. Measurements state how they were measured, the image version and the date.
- **Links:** relative markdown links in bodies (GitHub and agents can follow them); wikilinks only in properties and on the board. New docs get `type` and `tags`.
- **Never commit** `docs/.obsidian/workspace.json`, plugin code, or plugin settings that hold keys.
- **Commits:** `T-NNN: lowercase summary in plain words` (several: `T-104, T-037, T-095: …`), a body saying what changed and what was checked on the R1 ("On the R1: …"), ending with the attribution line. Commit or push only when asked.
- **Writing style:** plain, short words; British-leaning spelling ("colours", "behaviour"); sentences that say what happens ("r1d keeps…", "the host draws…"); the owner's requests quoted with a date ("<who>, YYYY-MM-DD: …"); dates on everything verified ("*(YYYY-MM-DD, on the R1)*"); bold lead-ins in bullet lists; no marketing tone; UI words lowercase as on the device.

---

## Verifying a change

**r1d (Rust)**
- `cargo test -p r1d` (gestures, app platform, core.db, voice helpers, prompt/example sync, approvals, cards, the app window's order and cart words…); `cargo test -p r1d live_ -- --ignored` hits real JSON and a real feed.
- `cargo clippy -p r1d --all-targets -- -D warnings`; Android lint: `CARGO_CMD=clippy tools/build-r1d.sh -- -D warnings`.
- Build for the R1: `tools/build-r1d.sh` (needs `tools/fetch-speech.sh` once) → `target/aarch64-linux-android/release/r1d`.

**Shell (simulator UI, app.html)**
- `bun run --cwd shell typecheck`, `bun run --cwd shell build`; hot reload `bun run --cwd shell dev` (:5171, proxies to :7171).
- Simulator: `tools/sim.sh`, open `http://127.0.0.1:7171` (↑/↓ wheel, Space button), drive it with `R1H_TARGET=sim tools/r1h …`. No agent box: builds and agent turns fail there. The simulator lags the R1's native shell (no light mode, approvals, pinned apps…).

**Host app (Kotlin)**
- `tools/build-host.sh` → `out/r1os-host.apk`, signed with r1os's platform key (`tools/make-keys.sh` once; `R1OS_KEYS` if they live elsewhere); deploy with `tools/deploy.sh --host` (ask first). Native shell changes can only be checked on the R1 (`r1h screenshot`, `r1h perf` for frame times).

**The image**
- `tools/build-image.sh` in Linux (WSL works), as root: stages `init sync patch layer build export`, and `layer policy` for the SELinux policy alone (~7 min). `export` refuses an image whose `framework-res.apk` or host isn't signed with r1os's platform key. Before an image build: `tools/build-host.sh`, `tools/fetch-aurora.sh`, `tools/build-bootlog.sh`, and `tools/r1h bootanim default` if an R1 runs r1os (the animated boot animation into `out/bootanim`; without it the image gets a still, `device/aosp/bootanimation-still.png`, so a first build needs no R1). Flashing is a device write: ask.

**Companion**
- `bun run --cwd companion tauri build --no-bundle` → `target/release/r1os-companion.exe`; window only, demo R1: `bun run --cwd companion dev` (:1420); link test: `cargo run -p r1-link --bin r1-link-pc -- pair`.

**Website and this skill**
- `bun run --cwd web typecheck`, `bun run --cwd web build` → `web/dist`; `bun run --cwd web dev` (:5180); hosted on Railway (`railway.json` builds `web/Dockerfile` from the repo's root: Bun, then `bun web/serve.ts`). The docs pages are `web/src/docs/content/*.md`; this skill is `web/public/skills/r1os/`.

**Prompts / app builds**
- `cargo test -p r1d` (example sync); `tools/r1h install fixtures/apps/water.enee.yaml` to check a spec; `tools/r1h create "<brief>"` (spends a little); evals `python3 tools/eval-apps.py [--model …] [--only 1,3] [--edits]` (asks first; reports to `artifacts/evals/`).

**On the R1** (after the owner OKs the first deploy of the session)
1. Check `tools/r1h events` for recent `hw` input (is someone using it?). `adb root` if just rebooted.
2. `tools/deploy.sh` (or `--host`), then poke the display with `adb shell input keyevent 59` before each step.
3. `tools/r1h state | events | wheel N | button tap|double|hold MS | launch <id> | screenshot [name] | logs`.
4. By hand: `T=$(source tools/lib.sh; r1d_token)`, then `curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/…` (after any `r1h` call sets up the forward).
5. Voice without the mic: `POST /harness/say {"text": "…"}` in a test chat; System One only: `POST /harness/system1 {"text"}`; STT benchmark: `POST /harness/hear {"path":"/data/r1os/models/stt/test.wav"}`; Translator: `POST /harness/translate {path, theirs}`.
6. Agents: `python3 tools/agent-test.py [codex] "message"` (delete the chats after); context: `POST /harness/tidy {chat}`; abilities: `GET /harness/abilities/{agent}`.
7. Jail: `GET /harness/jail` → uid 2950, `context: u:r:r1d_agent:s0`, `CapBnd: 0000000000000000`, `NoNewPrivs: 1`, `Seccomp: 2`.
8. SELinux: `logcat -b all -d | grep avc: | grep -E "r1d|r1os"` should print nothing after the feature ran.
9. Image checks: `getprop ro.r1os.image.version`, `getprop init.svc.r1d` → `running`, `getenforce` → `Enforcing`.

**The app window and the browser** (never finish anything real: [rules.md](rules.md))
- One op without a model: `POST /harness/app_window {op, args, agent?, chat?}`, straight to the host with no cards (`{op: "open", args: {package: "…"}}`, then `read`, `click`…). The browser's twin is `/harness/browser`.
- A commit card on the screen as it is, with nothing pressed whatever the answer: `POST /harness/commit_card {agent, chat}` → `{allowed, pressed: false, order}`.
- A test chat as a dry run: `POST /harness/dry_run {chat, on: true}`.
- Real agent turns, scored: `python3 tools/eval-control.py [--only b1,u2] [--agent <name>] [--keep] [--list]` (scenarios in `fixtures/evals/control.json`). Each runs in a dry-run test chat, declines every final step, counts the cart before and after, waits while the owner uses the R1, and pauses to recharge below 20%. It spends a little on the subscription: it asks first (`YES=1` skips). Reports to `artifacts/evals/`.
- `tools/r1h perf` shows each step as `app.<op>`; `tools/r1h power` reports the R1's own screen state too (an open window keeps Android "Awake").

**Docked mode and touch**
- `POST /harness/tilt {side: 1}` turns r1os as if on its right side (`0` upright, `null` back to the accelerometer); `POST /harness/rest {step: 0|1|2}` holds the glance screen's rest (`null` its own timer).
- Touches by adb: `adb shell input swipe 240 5 240 420 350` (the glance screen, pulled down) or `adb shell input swipe 240 636 240 150` on home (the launcher), upright; `adb shell input tap X Y`.
- Talking by touch: `POST /api/voice/talk {op: start}` then `{op: cancel}` (or `send`, only in a test chat: it's a real hold).

**Measure** (any change to latency, frames or energy: before and after, appended to `docs/research/2026-10-11-baseline.md` as a dated section)
- `tools/r1h power [seconds] [--json]`: battery, CPU per process, suspends, wakeups (on USB there are no suspends and no real drain; unplug + `r1h power mark a` / `mark b` / `diff a b` for drain, using Wi-Fi adb or no adb at all).
- `tools/r1h perf [reset]`: r1d's marks (`stt.load`, `voice.release_to_final.short|full`, `reply.first_word.warm|cold|pc`, `reply.done.*`, `voice.release_to_first_word`, `app.<op>`) and the host's frame times (`dumpsys gfxinfo dev.r1os.host`).
- Baseline numbers to compare against (image v0.2.14–v0.2.17): screen-off idle ~0.9–1.2% of all cores; home idle 6.4%; warm Claude first word ~1.6 s, cold ~10 s; release → final words ~0.4–0.9 s; wheel on home median 14–16 ms frames; MemAvailable ~2.2 GB with one warm Claude, ≥1.93 GB with three agents answering; standby ~1.6%/h before the power policy.
- The app window (T-143, T-148): with reads running at 15 fps, the host's frames 11 ms at the 90th percentile and 13 ms at the 99th; Uber Eats in the window ~216% of the R1's 800% (most of it Chrome's renderer).
