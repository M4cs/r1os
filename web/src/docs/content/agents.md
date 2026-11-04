---
title: Agents
description: How agents run, remember, use tools and ask first.
group: the os
order: 2
---

# Agents

An agent is a name, look, personality, role and brain; you can have up to 8, in your order. Each has its own chats, kept as long as you like, threads with other agents, memory, and a workspace of files in its jail.

## Brains

Each agent's brain is Claude (the default), Codex or your own Hermes gateway, and a chat keeps the brain it started with. One sign-in per CLI, or one gateway, serves every agent.

- **Claude** runs as one warm `claude -p` per chat, with its own tools, r1d's and your claude.ai connectors. Anything not allowed outright asks on the device.
- **Codex** runs `codex exec` per message, with web search and r1d's tools. Its own sandbox can't run on Rabbit's kernel, so r1d's [jail](/docs/architecture#the-agent-jail) confines it.
- **Hermes** runs on your own machine or server, not on the R1, with its own tools, memory and skills and none of r1d's. See [Your own agents](/docs/extend#your-hermes-agent).

## Processes

Up to 3 CLI processes run at once; the least recently used idle one makes room. One turn runs per chat, and different chats run in parallel.

A cold Claude takes over 5 s to start, so r1d warms one as you start holding the button. Warm, the first word comes about 1.6 s after your message. Past half the context window, a chat carries on in a fresh session from a short note; the transcript keeps everything.

## Memory

Short facts in the agent's prompt: shared ones about you, for every agent, and its own. Agents keep and forget them as you ask, but can't forget another agent's own. Up to 50 shared and 50 own, of 300 characters each.

## Tools

Agents call r1d's tools over MCP (`mcp__r1__<name>`). Each process has its own token, so every tool acts as that agent, in that chat.

| Area | Agents can |
|---|---|
| Apps | Use, build, edit, fix, undo, remove and restore them, and set their settings and permissions |
| Time | Set and cancel alarms, reminders, timers and routines |
| Media | Photos, recordings, the journal, music and what's playing |
| Android apps | Open them, handle notifications, and work them in the [app window](/docs/android#the-app-window) |
| The browser | Open, read, click, type and hand a page to you |
| The device | The look, home, widgets, settings, time zone and battery |
| Everything else | Cards, news, catching up, memory, other agents and skills |

Every tool, by name, is in the skill's API reference: [the skill's files](/docs/skill#skill-files), under api.md.

## Approvals

Each agent has a level per area, set in its space: **ask** (the default: a [card](/docs/using#approvals); the tool waits up to 2 minutes), **allow** or **never**.

| Area | Covers |
|---|---|
| `look` | The look, home and widgets |
| `device` | Settings |
| `apps` | Apps' settings and permissions |
| `browser`, `android` | Acting in a page or an Android app |
| `files`, `commands`, `outside` | An agent on your computer ([Your own agents](/docs/extend#agents-on-your-pc)) |

Connector writes and new skills ask on cards of their own. Some cards come whatever the level: granting an app a permission, an Android app's first use, and every final step in one ([Android apps](/docs/android#approvals)). Destructive tools also need `confirmed: true`, after the agent has asked you.

## The browser

Agents have a browser of their own, for pages that need JavaScript or something done on a site. It shows live in the chat with the agent's pointer. Looking runs; acting asks once per task, and sign-ins, passwords and codes are handed to you. It opens public web pages only, never the device or your local network, and closes two minutes after its last use.

## Agents working together

An agent can hand work to one that has what it lacks, like your computer or a connected account. `ask_agent` returns at once, and the answer comes back to the chat that asked; `tell_agent` leaves a note.

- **Limits:** 4 asks per chain, nothing wakes past 10 messages, 10 wakes per agent per hour, one such turn at a time. **Pause** (Settings › Agents) wakes no one.
- **No risky steps:** a turn another agent started can't change apps, the look, settings or permissions, run `confirm:` actions or an Android app's final step, write shared memory or forget anything.

## Stopping

Two clicks, "stop", or `POST /api/chats/{id}/stop`. Claude is interrupted and stays warm; Codex is stopped and resumes next time; Hermes is asked to stop at its gateway. Waiting cards answer no, and the next turn is told.
