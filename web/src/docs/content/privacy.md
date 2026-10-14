---
title: Privacy and security
description: What leaves the R1, who can reach it, and what isn't safe yet.
group: the os
order: 9.5
---

# Privacy and security

r1os has no account, no server and no telemetry. What you say to an agent goes to the company behind it, under your own account, and that's most of what leaves the R1. This page says what leaves it and where it goes, who can reach the R1 and what each of them gets, and what isn't safe yet.

Found a hole? Report it privately, through the repository's Security tab, as its `SECURITY.md` says.

## What leaves the R1

### To your agents' providers

Your agents are the unmodified `claude` and `codex` CLIs, signed in with your own Claude or ChatGPT account. What an agent works with goes to Anthropic or OpenAI, under that account's terms:

- **Your messages, as text.** Speech turns into text on the R1, and no audio is sent anywhere. Push-to-talk audio isn't kept.
- **Photos:** the ones you attach, and any photo an agent opens. Agents can look at your photos without asking.
- **Each turn's context:** your shared memories and the agent's own, the time and time zone, and which Android app is in front.
- **What agents read with their tools:** your notifications, the journal, memories, recordings' transcripts and web pages. Reading doesn't ask; changing things does ([how agents ask](/docs/agents)).
- **In Android apps:** each step's screenshot and the screen's text. Secure screens come out black and password fields blank ([Android apps](/docs/android)).
- **The Translator** uses models on the R1. A language pair without one goes to Claude Haiku, as text.
- **The Recorder's summaries**, off unless you turn them on, send the transcript to Claude Haiku.
- **Connectors and ChatGPT apps** you set up in the CLIs go to their own providers.

The CLIs run with their own settings. Whatever they send by default (usage statistics, error reports, update checks), they send: see Anthropic's and OpenAI's documentation.

### To other services

- **The time zone.** When the R1 joins a network (at most every 10 minutes), it asks ipinfo.io, or else wttr.in, for the time zone of its public address. Each of them sees the R1's IP address. Pick a zone in Settings › Date & time and the R1 stops asking.
- **Mini-apps** reach only the web hosts they declare and you allow: https only, public addresses only, through r1d.
- **Agents** browse, fetch and run commands with the network, as any app with internet access can. That includes your local network.
- **Android apps:** Aurora Store browses Google Play's catalog with anonymous sessions from Aurora's own service. Google sees that anonymous account, the R1's IP address, and what you search for and download. Apps you install behave as they would on any Android phone.
- **Android itself** keeps AOSP's connectivity checks and network time.
- **Setting up the agent box** downloads Alpine's packages and Claude Code's installer, and the CLIs update themselves.
- **The link to a computer** goes directly when it can, or through n0's relays when it can't: see [below](#who-can-reach-the-r1).

### Nothing at all

- **No telemetry, analytics or crash reports** in r1d, the host app, the shell, the companion or this website.
- **No location.** It's off, and nothing in r1os uses it. An Android app that needs it can have it turned on in Android's settings, and it stays on.
- **This website** loads nothing from anyone else: no analytics, and its own fonts.

### Kept on the R1

Everything r1os keeps is in `/data/r1os`, readable only by root. That's your chats, memories, journal, photos, recordings, apps and their data, the CLIs' sign-ins, and notifications that came while the screen was off (kept 24 hours, at most 300). r1d's log records how long things were, never what you said or wrote.

## Who can reach the R1

| Who | How | What they get |
|---|---|---|
| **Anyone with a USB cable** | adb, which is on and doesn't ask today | **Root:** everything on the R1, including the CLIs' sign-ins. This is the biggest gap ([below](#not-safe-yet)). The PIN lock doesn't stop it |
| **A computer on your Wi-Fi** | Wireless debugging, only if you turn it on in Android's developer options | The same as a USB cable |
| **Your local network** | Two short-lived servers: the sign-in page (port 7180, while you sign an agent in from your phone: up to 10 minutes, one try) and sharing (a random port, up to 10 minutes) | One page or one file, behind a one-time token in the link. **Plain HTTP:** someone who can watch your Wi-Fi's traffic could read the sign-in code |
| **A linked computer** | The link: the R1 dials the computer, never the reverse, and you confirm a matching code on both at pairing | What Settings can do. It can read chats, memories, media, notifications, and a full backup. It can change agents, apps, Wi-Fi and the CLIs' sign-ins, and talk to your agents. Not the test harness, approvals, the PIN lock or the camera motor ([Companion](/docs/companion)) |
| **n0's relays and DNS** | The link, when the R1 and the computer can't reach each other directly | Both devices' IDs, IP addresses, and when and how much they talk. Not what they say: the link is QUIC, encrypted between the two devices' own keys (iroh's design; r1os hasn't audited it) |
| **Android apps you install** | Android's own sandbox | Their own Android permissions, asked for by Android's own prompts. Not r1d's API (it needs a token they can't get), and not `/data/r1os` |
| **Agents** | Their sandbox, and r1os's tools | Their own workspace, the shared sign-ins, the network. Device changes ask first ([How it's built](/docs/architecture#the-agent-jail)) |
| **Mini-apps** | Their page, with a token for that app alone | Their own tables. The web and anything more need a permission you grant |
| **Someone holding the R1** | Its screens | Everything you'd see, unless the PIN lock is on |

r1d serves its API on `127.0.0.1` only, behind a token that changes at every boot, which only r1os's own host app is given. See [How it's built](/docs/architecture#trust-boundaries).

## Not safe yet

r1os is early. These are known, and each one has a fix planned:

- **A USB cable means root.** The image is a userdebug build, and adb doesn't ask. Planned: adb authorization, or a user build, before a general release.
- **Every agent runs as the same user** and shares one sign-in home, so any agent's process can read every agent's CLI sign-ins. Planned: a credential broker.
- **The sign-in page is plain HTTP** on your local network.
- **Android apps' mic and camera use has no indicator.** r1os turns Android's off: r1d opens the mic on every button press, and the indicator kept pulling the status bar down. The R1's own screen shows when r1os is listening, but not when another app is.
- **Agents reach your local network** as well as the internet.
- **Some of the image's system components** are still signed with AOSP's public test keys.
- **No signed releases or over-the-air updates yet:** updates are a flash by hand. The companion isn't code-signed.
- **No factory reset** that wipes your data and sign-ins.

If you can make any of these worse than described here, please report it privately (the repository's `SECURITY.md`).
