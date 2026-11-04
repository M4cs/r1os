---
title: Your own agents
description: Use r1os from Claude Code, Codex or your own tools.
group: extend
order: 1
---

# Your own agents

Ways to bring your own agents and tools to r1os. All of it is built except what's [planned](#planned). Coding agents working on r1os itself have [the r1os skill](/docs/skill).

An outside agent can't call r1d's own MCP endpoint: r1d gives its tokens only to processes it starts.

## Your Hermes agent

If you run [Hermes Agent](https://hermes-agent.nousresearch.com/docs/), the R1 can be its voice and wheel: hold the button and talk, and your gateway answers with its own memory, skills, tools and models. Nothing extra runs on the R1 but a connection during a turn. Tried on an R1 against a real Hermes v0.22.7 gateway with a real model (Liquid's `lfm2.5` on Ollama), over USB; not yet over a network.

1. **Turn on its API server,** where Hermes runs, in `~/.hermes/.env`: `API_SERVER_ENABLED=true` and `API_SERVER_KEY=` a long random key. Set `API_SERVER_HOST` to an address the R1 can reach, then run `hermes gateway`. Anyone who can reach it with that key can run commands on that machine, so keep it to a private network, or sandbox Hermes's terminal.
2. **Add it in the companion,** under accounts: its address and key. The R1 checks them, then keeps them. Or `POST /api/accounts/hermes/login {url, key}`.
3. **Give an agent the brain:** in its space or the companion. New chats use it.

What to expect:

- **Reaching it:** a private address or name (Tailscale, your home network) can be plain `http`. Anywhere else has to be `https`, because the key would cross the internet in the clear. A profile goes in the address: `https://…/p/work`.
- **Approvals:** when Hermes asks before a dangerous command, it shows as a card on the R1, every time, with no "always". What it asks about is Hermes's setting, `approvals.mode`: by default (`smart`) its own check allows or refuses most commands and only asks about the unclear ones, and `manual` always asks. A turn another agent started is refused without a card.
- **Questions mid-turn:** Hermes's API server doesn't give its model the tool for asking the user. If yours does, Hermes is told to make its best guess and say what it assumed, as the R1 can't answer during a turn.
- **If the gateway restarts during a turn,** the chat says so and picks up on your next message, in the same session. In Docker that next message can sit for up to 5 minutes: Hermes holds the session for the process that died, and the new one has the same process ID. Stopping the turn works meanwhile.
- **Its tools are its own,** on its machine. It can't use the R1's camera, apps, alarms or cards. Your other agents can, and can [ask it](/docs/agents#agents-working-together) for what only it can do.
- **Its own chats:** each R1 chat is a Hermes session of its own. Hermes won't let the R1 pick up a Telegram or CLI conversation, but memory and skills are shared.
- **A local model on Ollama:** Hermes needs at least 64,000 tokens of context, so start it with `OLLAMA_CONTEXT_LENGTH=64000`. An unloaded model can take a minute or two to answer, in silence, and Ollama unloads one after 5 idle minutes: `OLLAMA_KEEP_ALIVE` keeps it longer. A model that can't see needs a separate vision provider set in Hermes, or it just says it can't see your photos. A small model may also confuse itself with the R1 when asked.
- **Pushes:** Hermes's cron results don't reach the R1 yet.
- **The key** is kept in a root-only file on the R1, out of its backups, and never sent back. Whoever holds it can run things on that machine: see [Privacy and security](/docs/privacy).

## Agents on your PC

An r1os agent can work on a linked computer. The companion runs the real Claude Code or Codex CLI there, in a workspace in your Documents plus the folders you allow, and streams each turn back. You talk to it as to any agent, or type in the companion.

- **Permission mode** for new chats: `manual`, `acceptEdits`, `auto` or `plan`, never `bypassPermissions`.
- **Approvals on the device:** files in its folders are allowed; commands, anything outside them, the browser and writing `.claude`, `.git` or `.mcp.json` ask. A Codex agent can still read outside its folders without asking; that's planned to ask too. Your own Claude Code settings don't apply.
- **Your Chrome:** a Claude agent can drive your signed-in Chrome through Claude in Chrome. Looking runs; clicks, typing and forms ask once per turn; logins, passwords, payments and CAPTCHAs stay yours.
- **Handing off:** agents on the device and on the PC can `ask_agent` each other.

## Android apps and the browser

Agents use your Android apps and r1os's browser while you watch in their chat, and every order, payment or message asks first. See [Android apps](/docs/android) and [the browser](/docs/agents#the-browser).

## Connections

r1os holds no tokens for these; agents use what their CLIs support. (A Hermes agent uses what its gateway has.)

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
