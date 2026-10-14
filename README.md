# r1os

An agent-first OS for the Rabbit R1, open source under the MIT licence.

The R1 boots straight into r1os's own shell, built for its wheel, side button, mic, speaker and rotating camera. You
talk to agents: Claude Code or Codex, the unmodified CLIs, under your own login, in a sandbox on the R1. They answer,
remember, run routines, use apps on the R1 for you, and build small apps for it when you ask. Rabbit's kernel and
vendor stay as the hardware layer; r1os's own Android system image sits on top.

> **Status:** early, and in daily use on its maintainer's R1. Installing it on another R1 still takes a developer
> (build the image, flash it from the command line). A desktop companion that installs it for anyone is being built.
> Not affiliated with Rabbit Inc., Anthropic or OpenAI.

## Try it without an R1

```bash
tools/sim.sh      # then open http://127.0.0.1:7171: ↑/↓ is the wheel, Space is the side button
```

The desktop companion, with a pretend R1: `pnpm install`, then `pnpm --dir companion dev`.

## Read more

| If you want to… | Read |
|---|---|
| Know what it does and how to use it | [Introduction](web/src/docs/content/introduction.md), [Using r1os](web/src/docs/content/using.md) |
| Understand how it's built | [How it's built](web/src/docs/content/architecture.md) |
| Install it on an R1 | [Install](web/src/docs/content/start.md): read the warnings first, since a bad write can brick an R1 |
| Contribute | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Report a security hole | [SECURITY.md](SECURITY.md) (privately, please) |
| Work on it with an AI agent | [CLAUDE.md](CLAUDE.md) |

These pages are also the website's docs (`web/`).

## Layout

```text
r1os/
├── crates/
│   ├── r1d/          the core daemon (Rust, root on the R1): input, agents, apps, speech, the API the shell uses
│   ├── r1-link/      the R1 ↔ computer link (iroh)
│   ├── r1-usb/       the companion's installer: drivers, backups, the flash
│   ├── r1-bootlog/   verbose boot's program
│   ├── enee-spec/    the app spec compiler
│   └── enee-data/    each app's storage
├── host/             the Android host app (Kotlin): the R1's home screen and native shell
├── shell/            the desktop simulator's UI, and the page each mini-app runs in
├── companion/        the desktop companion (Tauri 2, React)
├── web/              the website, its docs, and the agent skill (web/public/skills/r1os)
├── packages/         shared front-end packages (the app runtime, icons)
├── agent-box/        the Alpine sandbox the agents run in
├── device/           r1os's image layer (AOSP), overlays, the old Magisk module
├── prompts/          the agents' app-building prompts
├── fixtures/         sample apps and evaluation briefs
└── tools/            build, deploy, simulator, the R1 harness (r1h), flashing, backups (Git Bash on Windows)
```

`firmware/`, `backups/` and `out/` are git-ignored: stock images, device-unique partition dumps (never commit or share
them), and build output.

## Licence

MIT: see [LICENSE](LICENSE). Third-party credits, fonts and the models r1os fetches are in [NOTICE.md](NOTICE.md).
