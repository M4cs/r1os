---
title: Introduction
description: What r1os is and what it stands for.
group: start
order: 1
---

# Introduction

r1os is an agent-first operating system. It covers most of the everyday things Rabbit's OS3 does, then goes further: your agents are the real Claude Code and Codex, on the device, with a full Android 15 device underneath. It runs on the Rabbit R1, and it's open source under the [MIT licence](/docs/develop#licence), [on GitHub](https://github.com/M4cs/r1os).

> r1os is an independent project. It isn't made by, affiliated with or endorsed by Rabbit Inc., Anthropic, OpenAI or Google.

![r1os at a glance, as real screens from an R1: first run, every day and the built-in tools.](/media/r1os-overview.png)

## What it does

- **Agents you talk to.** Hold the button and talk to one of up to eight agents, each with its own character, chats, memory and files. It answers, browses the web, uses your apps or builds you one in seconds.
- **The real CLIs.** The unmodified `claude` and `codex`, sandboxed on the device, on your own Claude or ChatGPT plan. No API keys, no middleman.
- **Everyday tools.** Alarms, timers, reminders, a recorder that transcribes, a translator, the camera, a journal, music and routines. Speech-to-text runs on the device.
- **A full Android device.** Google Play's catalog through Aurora Store, with no Google account. Apps that need Google Play Services may not work yet.
- **Agents in your apps.** On a screen of their own while you watch, asking before they order, pay or send. See [Android apps](/docs/android).
- **Dock mode.** On its side, r1os turns landscape, works by touch and stands as a bedside clock. See [Dock mode](/docs/using#dock-mode).

Not yet, next to OS3 as of October 2026: spoken answers, languages other than English, over-the-air updates, factory reset, lost mode, AI photo styles, and location and directions.

## Speed

Quick while the screen is on, idle the moment it sleeps, and every change measured on the device ([the numbers](/docs/architecture#speed-and-battery)):

- The screen is on 36 s after the kernel starts, down from 72 s.
- A warm Claude agent's first word comes about 1.6 s after your message.
- The wheel, talking and replies run at 60 fps.
- Asleep, r1os itself polls nothing.

## Principles

- **Wheel and button first.** Every screen works with them alone; touch leads only when docked.
- **Your logins stay yours.** r1os drives the unmodified CLIs, never the Claude Agent SDK with a subscription login. It stores no Claude, OpenAI or connected-service credentials.
- **Apps are specs.** r1os compiles and checks the apps agents write, and runs them behind permissions you grant.
- **Never brick.** Backups before writes, Rabbit's kernel left alone, and a way back to stock.

## How it's built

Rabbit's kernel (the only one with the wheel's driver), vendor and bootloader stay. Above them: r1os's own Android 15 image, `r1d` (a Rust daemon for the device, apps and agents), a native shell, and an Alpine box for the CLIs. See [How it's built](/docs/architecture).

Next: [Install](/docs/start), then [Using r1os](/docs/using).
