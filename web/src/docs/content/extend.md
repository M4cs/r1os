---
title: Your own agents
description: Use r1os from Claude Code, Codex or your own tools.
group: extend
order: 1
---

# Your own agents

Ways to bring your own agents and tools to r1os. All of it is built except what's [planned](#planned). Coding agents working on r1os itself have [the r1os skill](/docs/skill).

An outside agent can't call r1d's own MCP endpoint: r1d gives its tokens only to processes it starts.

## Agents on your PC

An r1os agent can work on a linked computer. The companion runs the real Claude Code or Codex CLI there, in a workspace in your Documents plus the folders you allow, and streams each turn back. You talk to it as to any agent, or type in the companion.

- **Permission mode** for new chats: `manual`, `acceptEdits`, `auto` or `plan`, never `bypassPermissions`.
- **Approvals on the device:** files in its folders are allowed; commands, anything outside them, the browser and writing `.claude`, `.git` or `.mcp.json` ask. Your own Claude Code settings don't apply.
- **Your Chrome:** a Claude agent can drive your signed-in Chrome through Claude in Chrome. Looking runs; clicks, typing and forms ask once per turn; logins, passwords, payments and CAPTCHAs stay yours.
- **Handing off:** agents on the device and on the PC can `ask_agent` each other.

## Android apps and the browser

Agents use your Android apps and r1os's browser while you watch in their chat, and every order, payment or message asks first. See [Android apps](/docs/android) and [the browser](/docs/agents#the-browser).

## Connections

r1os holds no tokens; agents use what their CLIs support.

- **Claude agents:** your claude.ai connectors. Reads run; actions ask on the device, with **always allow** per agent and tool.
- **Codex agents:** your ChatGPT account's apps. Writes ask too.
- **Your own MCP server with an API key:** add it from the companion or with `POST /api/connections/add {name, url, key?, header?}`. The CLI keeps the key.
- **A server that signs in with OAuth:** add it on claude.ai as a custom connector.

## Skills

Skills install once for every agent. Ask one ("add the pdf skill from anthropics/skills") and allow it on the device, or use the companion or `POST /api/skills {source, skills?}`, where `source` is anything `npx skills add` takes. Each is on or off per agent, and your claude.ai skills sync to Claude agents.

## Scripting the device

From a [clone of r1os](https://github.com/M4cs/r1os), any program on your computer, a coding agent included, can drive the device over adb:

```bash
tools/r1h state                     # state and recent events
tools/r1h wheel 3 && tools/r1h button tap
tools/r1h screenshot home
python3 tools/agent-test.py "what's the weather?"   # a new chat, with tools and timings
```

Or use [the API](/docs/api): `POST /api/chats`, `POST /api/chats/{id}/messages {text}`, and the reply as `chat` events on `/ws`. These are real chats on your subscription: delete them after. `R1H_TARGET=sim` drives [the simulator](/docs/develop#the-simulator) instead, which has no agent box. Before scripting a device in use, see [testing on the R1](/docs/develop#testing-on-the-r1).

## Apps from outside

Have the device build an app from a brief, or write one yourself (or with your own agent) from `prompts/builder.md` and install it: see [Apps](/docs/apps#from-the-command-line). The compiler says what's wrong, and the app's checks run before it installs. Share one with `tools/r1h export <id> --data`, or by QR from its page.

## Planned

- **App work with the screen off.** Today an agent's task in an app waits for the screen.
- **microG,** offered when an app needs Google services.
- **Local models:** the companion runs or reaches a model server on your PC (Ollama, LM Studio, llama.cpp) as an agent's brain, with no subscription.
- **Runs started by events,** like a new email starting a routine.
- **Codex's browser** on the PC, once its CLI has one.
