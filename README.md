# r1os

A lightweight, agent-first OS for the Rabbit R1 (working name).

The R1 boots straight into our own shell. The wheel, side button, mic, speaker and rotating camera
are first-class. An on-device coding agent builds small apps for the device: Claude Code or Codex,
running cloud models under your own login. It's the [eneeware](../eneeware) idea, in your hand.

> **Status:** R0, device bring-up. See [docs/roadmap.md](docs/roadmap.md).

## Start here

| If you want to… | Read |
|---|---|
| Wipe the R1 and put the dev base on it | [docs/runbooks/fresh-start.md](docs/runbooks/fresh-start.md) |
| Understand the design | [docs/architecture.md](docs/architecture.md), then [docs/decisions/](docs/decisions/) |
| Know what we know about the hardware | [docs/research/2026-10-08-r1-platform.md](docs/research/2026-10-08-r1-platform.md) |
| Work on this repo as an AI agent | [CLAUDE.md](CLAUDE.md) |

## Layout

```text
r1os/
├── crates/r1d/    the core daemon (Rust): evdev input, gestures, motor, HTTP/WS for the shell, harness
├── shell/         the UI (React + Vite), served by r1d into the host's WebView
├── host/          the Android host app (Kotlin): the R1's home screen, a full-screen WebView
├── device/        what we put on the R1: the r1os-base Magisk module now, our own system image later (R4)
├── tools/         build, deploy, simulator, harness CLI (r1h), flashing, backup (Git Bash on Windows)
├── docs/          vision, architecture, specs, ADRs, research, roadmap, runbooks (source of truth)
├── firmware/      (git-ignored) stock firmware, GSI images, Magisk APK
├── backups/       (git-ignored) partition dumps: device-unique, never commit or share
└── out/           (git-ignored) build outputs and device reports
```

Planned: `agent-box/` (the agents' Linux userland, R2).

## Try it without the R1

```bash
tools/sim.sh      # then open http://127.0.0.1:7171 — ↑/↓ is the wheel, Space is the side button
```

Not affiliated with Rabbit Inc., Anthropic or OpenAI. Third-party credits are in [NOTICE.md](NOTICE.md).
