---
title: Companion
description: Pair a computer, manage r1os, lend agents your PC.
group: the os
order: 3
---

# Companion

**r1os Companion** is a desktop app that links to your R1. Manage r1os with a big screen and a keyboard, sign in to Claude and Codex with your computer's browser, chat with your agents, and lend them your computer to work on. It lives in the tray, starts with your computer, and wears your r1os look.

**Status:** Windows today; macOS and Linux are planned. Code signing and automatic updates aren't done yet.

## Pairing

1. **On the computer,** the companion shows a one-time QR code, good for 5 minutes.
2. **On the R1,** open Settings › Computers › **link a computer**, or ask an agent to. The camera reads the code.
3. **Both screens show the same 4-digit code.** Confirm on the computer, and each side pins the other's key.

One R1 can link several computers, and one computer several R1s. Either side can unlink; unlink a lost R1 and its key stops working.

## The link

- **End-to-end encrypted** (QUIC, through iroh) and authenticated by the pinned keys: direct on the same network, otherwise hole-punched or relayed.
- **The R1 never listens.** It dials out to computers it has pinned, and keeps no port open.
- **Up while it's needed:** while the R1's screen is on, or something runs for that computer. It lets go after an idle time (5 minutes by default), and the companion can't wake a sleeping R1.
- **Settings powers, no more.** The companion can do what r1os's Settings can. The PIN lock, approvals and the system image stay on the R1, which shows a note when something is changed from a computer.

## What you can manage

- **Agents:** profiles, looks, order, levels, memories and chats.
- **Apps:** settings, permissions, export and import, widgets and home.
- **The look,** with a live preview, and **settings,** with Wi-Fi typed on a real keyboard.
- **Alarms, reminders, timers and routines.**
- **Files:** photos, recordings and the journal; drop photos on the page to send them.
- **Connections and skills,** your own MCP server included.
- **Backup** of r1os's data, never the agent box, the CLIs' logins or the link's key.

## Chats

The same chats, typed. Replies stream with their tool steps and cards; add up to four photos to a message, or stop a reply. Approvals are still answered on the R1.

## Agents on your computer

An agent set to work on a linked computer runs its turns there, with the real `claude` or `codex` CLI in the folders you allow, as your user: its folders and levels keep it in bounds, not a sandbox. Its approvals come to the R1, and messages wait while the computer is off. See [Your own agents](/docs/extend#agents-on-your-pc).

## Installing r1os over USB

The companion also installs r1os, one step at a time:
1. **Windows' drivers.**
2. **A backup** of the R1's own partitions, into a folder you choose (Documents\r1os backups until you do). It holds the R1's IMEI, so pick somewhere only you can reach.
3. **The unlock.** The companion guides it; you request it on Rabbit's portal.
4. **The flash.**
5. **The first setup.**

Each step says what it does, what to do with your hands, and what to do if it goes wrong. It finds out what state the R1 is in rather than assuming.

- **Nothing is written by a single click.**
  - The flash shows its plan and needs `ERASE` typed.
  - It won't run without a backup of that R1 that checks out, unless you type "I understand" on the backup step.
  - The first setup erases nothing. It shows its plan and starts when you click.
- **Everything happens from the bootloader.** The flash writes `vbmeta`, then `super` with the R1's own vendor kept where it is, then wipes `userdata`. It never uses fastbootd.
- **The first setup** puts on r1d, the shell, speech-to-text, the models and the agent box, from an r1os checkout you choose (releases come later).
  - It plans only what isn't there yet.
  - An R1 that isn't on Wi-Fi gets the rest now, and its agent box once it's online.

**Status:** the flash has been checked on a computer against a real R1's partition layout, but it hasn't yet run end to end on an R1. Until it has, install with the [guide](/docs/start). To try the wizard without an R1, run `pnpm --dir companion dev` and choose **install r1os**.

## Signing in

- **Claude:** the companion starts the CLI's login on the R1 and opens the link in your browser; you paste the code into the companion. It crosses the link once, straight to the CLI, and is forgotten.
- **Codex:** its device-code flow; the companion opens the page and shows the code.
- **No computer?** The R1 shows a QR code to sign in from your phone.

r1os never stores your credentials or copies them between devices; your computer's CLIs keep their own sign-ins.
