---
title: Contributing
description: Build, test and deploy r1os safely.
group: contribute
order: 1
---

# Contributing

## The repo

r1os is open source under the MIT licence, at [github.com/M4cs/r1os](https://github.com/M4cs/r1os). Its layout:

```text
r1os/
├── crates/       r1d, the app format, the link, the boot log
├── host/         the Android host app (Kotlin)
├── shell/        the simulator's UI, the mini-app page
├── packages/     the app runtime, icons
├── companion/    the desktop companion (Tauri 2)
├── agent-box/    the Alpine rootfs
├── device/       the image's layer, patches, overlays
├── prompts/      the app builder's prompts
├── fixtures/     example apps, evals
├── tools/        build, deploy, drive, measure
├── web/          this website, the agent skill
├── docs/         specs, decisions, research, roadmap
└── firmware/ backups/ out/   git-ignored: never commit them
```

## What you need

| For | You need |
|---|---|
| r1d, the shell, the website | Rust 1.95+, pnpm |
| r1d for the device, the host app | The Android SDK and NDK, JDK 17+ |
| Speech models, evals | uv, Python 3.11 |
| The system image | Linux (WSL works); see [Install](/docs/start) |

The tools are bash: on Windows, use Git Bash or WSL (and `MSYS_NO_PATHCONV=1` for adb by hand). Python scripts use `python3`; on Windows, `py -3`. Run `tools/fetch-speech.sh` once before building r1d, and `tools/fetch-models.sh` for System One and the Translator.

## Build and test

| Goal | Command |
|---|---|
| r1d tests and lint | `cargo test -p r1d` · `cargo clippy -p r1d --all-targets -- -D warnings` |
| r1d for the device | `tools/build-r1d.sh` (lint: `CARGO_CMD=clippy tools/build-r1d.sh -- -D warnings`) |
| Shell | `pnpm --dir shell build` · `pnpm --dir shell typecheck` |
| Host app | `tools/build-host.sh` → `out/r1os-host.apk` |
| Companion | `pnpm --dir companion tauri build --no-bundle` |
| Website | `pnpm --dir web dev` · `pnpm --dir web build` · `pnpm --dir web typecheck` |
| Drive the device | `tools/r1h state \| events \| wheel N \| button tap \| launch <id> \| screenshot \| logs` |
| Measure | `tools/r1h power` (battery, CPU, wakeups) · `tools/r1h perf` (latency, frames) |
| Agent box | `tools/agent-box.sh install \| status \| run -- CMD` (all but `status` write to the device) |
| Evals | `python3 tools/eval-apps.py` (app builds) · `python3 tools/eval-control.py` (agents in apps and the browser) |

- **Evals** run real agent turns on the device: a little on your subscription, so they ask first. `eval-control.py` uses dry-run chats, so nothing is ordered, paid for or sent.
- **`prompts/example.enee.yaml`** must equal `builder.md`'s example and pass its checks (`cargo test -p r1d` enforces it).
- **Docs pages** are `web/src/docs/content/*.md`, and the agent skill `web/public/skills/r1os/`. When a rule or gotcha changes, change the skill too.

## The image

Building and flashing it is in [Install](/docs/start). `tools/build-image.sh`'s stages (`init`, `sync`, `patch`, `layer`, `build`, `export`) each rerun alone; `layer policy` rebuilds just the SELinux policy, with AOSP's checks.

The image and the host app are signed with r1os's own keys, made once by `tools/make-keys.sh` outside the repo (`R1OS_KEYS` sets where); builds stop without them. Never commit, print or upload them: whoever holds `platform` has r1os's system powers.

## The simulator

`tools/sim.sh` runs `r1d --sim` at http://127.0.0.1:7171, through the real gesture engine: ↑/↓ or the mouse wheel are the wheel, Space the side button. `--dev` adds hot reload; `R1H_TARGET=sim tools/r1h …` drives it. With no agent box, chats and app builds fail there; apps installed from files work.

## Deploying

`tools/deploy.sh` builds r1d and the shell, pushes them and any fetched models to `/data/r1os`, relabels them for SELinux and restarts r1d; `--host` also installs and compiles the host app. It writes to the device.

Pair wireless debugging once (`adb pair <ip>:<port> <code>`, from the device's pairing-code screen) and the tools find it on the network. Fastboot is USB only.

## Device safety

The R1 has no volume keys and no recovery menu, so a bad write to the wrong partition can brick it.

- **Ask before anything that writes to the device.** Read-only is safe: `getprop`, `getevent`, `dumpsys`, `fastboot getvar`, `tools/device-report.sh`, `tools/backup.sh`, `tools/hw-check.sh` without `--motor`.
- **These write:** `fastboot flash/erase/set_active`, `dd`, `pm uninstall`, motor or sysfs writes, Magisk modules, and in `tools/`: `deploy.sh`, `root.sh`, `flash-boot.sh`, `flash-gsi.sh`, `flash-stock-base.sh`, `fastboot-entry.py`, `grow-data.sh`, `install-module.sh`, `edges-overlay.sh`, `agent-box.sh` (all but `status`), `hw-check.sh --motor` and `r1h motor`. Installing an Android app is a write too. Destructive scripts ask first.
- **Only ever write** `boot_*` (Magisk-patched from the device's own image), `dtbo_*`, `vbmeta*`, `system_*`, `product_*`, `super` and `userdata`. Never any other partition: the preloader, `lk`, `nvram`, `persist`, the modem, the TEE…
- **Never** run `fastboot flashing lock` on a modified image, replace Rabbit's kernel, or take a Rabbit OTA.
- **Back up first:** `tools/backup.sh` writes `backups/` with a `SHA256SUMS` file. Backups hold the IMEI and radio calibration: never commit or share them.
- **Files put in `/data/r1os` by hand** need `restorecon -RF <path>` as root. Never `chown -R` or `chmod -R` all of `/data/r1os`.

## Testing on the R1

- **Check `tools/r1h events` for recent `hw` input first:** someone may be using it. Stop long measurements when it's touched.
- The display sleeps after 60 s, and harness input doesn't count: poke it with a harmless key (`adb shell input keyevent 59`).
- Asleep, it wakes to the glance screen, which swallows `launch` until a double click goes home.
- After a reboot, `adb root` before the tools.
- A hold uses the real mic and a real chat. Script words with `POST /harness/say` in a test chat, then delete it.
- Long eval runs can drain the battery, even on USB.
- Run r1d by hand with every fd redirected; stop it with `pkill -x r1d`, not `-f`.

## Battery rules

- **Idle means idle:** with the screen off, nothing of ours runs on a timer, polls or animates. Loops run only while visible, and hold still once Android dims the display.
- **Events, not polling:** react to `/ws` events; batch network work for when the screen is on.
- **Cheap pixels:** animate transform and opacity. 60 fps is for what answers you; idle life ticks slowly, in whole pixels.
- **Measure before and after** any change to latency, frames or energy.

## Tracking work

`docs/` is the source of truth, in plain markdown. A behaviour change updates its spec in the same change, hard-to-reverse choices get an ADR in `docs/decisions/`, and work is a task note in `docs/kanban/tasks/`, named in its commit (`T-012`).

## Licence

r1os is under the MIT licence (`LICENSE`). Third-party parts keep their own, listed in `NOTICE.md`.
