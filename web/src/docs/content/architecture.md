---
title: How it's built
description: The layers, the agent jail, trust and speed.
group: the os
order: 9
---

# How it's built

## The layers

```text
┌ Host app: Kotlin, the home screen ────────────────────────────┐
└────────── localhost HTTP + WebSocket, per-boot token ─────────┘
┌ r1d: Rust, an init service in its own SELinux domain ─────────┐
└────────── chroot, uid 2950, seccomp, cgroup, SELinux ─────────┘
┌ Agent box: Alpine arm64, with claude, codex, git and Node ────┐
└───────────────────────────────────────────────────────────────┘
  Android apps from Google Play's catalog, via Aurora Store
  r1os's own Android 15 image: AOSP + TrebleDroid's patches
  Rabbit's kernel 4.19, vendor and bootloader, never modified
```

## r1d

One Rust daemon that outlives UI restarts, run by init on the device (or as the simulator with `--sim`) and serving `127.0.0.1:7171`. It owns:

- input, gestures and the camera motor;
- apps: the compiler, each app's database, permissions and a safe web fetcher;
- agents: chats, memory, processes, their `/mcp` tools, and their steps in Android apps and the browser;
- speech, time (one kernel alarm for everything), the link to computers and a test harness.

## The host

The host app is the home screen. It draws the shell in native views and does what only an Android app can: camera, mic, playback, sensors, Wi-Fi, notifications. Each open mini-app gets a WebView, and input reaches it only as r1d's `/ws` events.

For agents, it runs the **app window** (an Android app on a hidden 600 × 1200 display at 15 fps, read off the UI thread by an accessibility service that's on only during a task; see [Android apps](/docs/android)) and the **browser**, a web page they open, read and click.

## The agent jail

The box is an Alpine arm64 rootfs with the CLIs, git and Node. Between fork and exec, each agent process:

1. joins a 1.5 GB memory cgroup;
2. gets its own mounts: a fresh `/proc`, a 512 MB `/tmp`, a minimal `/dev`, its workspace;
3. chroots into the box;
4. drops to uid 2950 with no capabilities;
5. sets `no_new_privs` and a seccomp filter;
6. enters the `r1d_agent` SELinux domain.

It sees its workspace, its chats and the shared home with the CLIs' logins. The CLIs' own sandbox can't run on Rabbit's kernel, so this is the boundary. `GET /harness/jail` shows it: uid 2950, `CapBnd: 0000000000000000`, `NoNewPrivs: 1`, `Seccomp: 2`, `u:r:r1d_agent:s0`.

## Storage

Everything is in `/data/r1os`, readable only by root: `core.db` (agents, chats, memories, grants, alarms), `apps/` (specs), `data/` (each app's database and snapshots), `trash/` (removed apps, for 7 days), `media/`, `agent/` (the box and each agent's files) and `models/`.

## Trust boundaries

- **r1d and the host** are trusted. Only the host gets r1d's per-boot token, which `/api`, `/harness` and `/ws` need.
- **USB adb** isn't authenticated yet: a computer with a cable gets root, and with it r1d's token and the CLIs' logins. An approval prompt is planned.
- **Signing:** the image and the host carry r1os's own keys; an app signed with AOSP's public test keys gets no system powers.
- **Mini-apps** reach only their own tables. The web (their declared https hosts, through r1d's fetcher) and anything more need a permission you grant. Games and logic run in a worker with no network of its own.
- **Agents** reach r1d only through `/mcp`; device changes ask unless you've allowed them. They share one uid, so each can read the CLIs' credentials; a credential broker is planned.
- **In Android apps,** agents ask before a new app, a first tap in a task, and every order, payment or message ([approvals](/docs/android#approvals)). Secure screens are black to them, and agents are told to treat screen text as information, not instructions. Each step's screenshot and text go to the model provider.
- **Other Android apps** stay in Android's sandbox, without r1d's token.
- **Linked computers** are dialled by the device, never the reverse, and get its Settings powers, no more.

## Speed and battery

Measured on the device before and after each change; every "before" is an earlier r1os build. Times are medians where they vary. CPU is a share of all cores, measured plugged in; current was measured unplugged.

| What | Before | After |
|---|---|---|
| Boot to the screen on, from the kernel's start | 72 s | 36 s, one boot (the boot animation lights it at ~29 s) |
| Idle CPU on home | 24.2% | 6.4% |
| Frames: the wheel, talking, replies streaming | — | 13–14 ms, 60 fps |
| Opening a busy page, worst frame after the first | 176 ms | 18–24 ms |
| Letting go → your words, once you've stopped talking | 865 ms | 380 ms (147–613, 2 turns) |
| Your message → Claude's first word | — | 1.6 s warm; 7.5–10.3 s cold, so r1d warms Claude ahead of time |
| App window: a scroll · opening a running app | 4.96 s · 9.3 s | 0.95 s · 2.0 s |
| Your frames while an agent reads an app, 99th percentile | 1,950 ms | 73 ms (13 ms with the window at 15 fps), one test |
| CPU with the app window open | 343% of 800% | 216% |
| Memory available after the agent's app closes | 1.5 GB | 2.5 GB |
| Memory available after idle apps close | 1.73 GB | 2.62 GB, one run |
| CPU with the screen off | 3.5% | 0.7% |
| Asleep overnight | — | 16 mA, ~1.6%/h, one night |
| Docked glance screen, untouched | 154 mA | 128 mA, 4-minute runs |

The overnight figure predates the current power policy. A full day per charge is the goal, not yet measured. The rules new code follows are in [Contributing](/docs/develop#battery-rules).
