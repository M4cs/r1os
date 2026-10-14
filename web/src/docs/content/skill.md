---
title: Agent skill
description: Context for coding agents working on r1os.
group: extend
order: 2
---

# Agent skill

The r1os skill gives a coding agent what it needs before it works on r1os or drives a device running it. `SKILL.md` holds the essentials and loads when the work calls for it; the agent reads the references only as it needs them:

- **`codebase.md`:** folders, r1d's modules, the host, and the scripts in `tools/`, marked where they write to the device.
- **`how-it-works.md`:** ports, the token, input, the agent jail, apps, storage, what must stay in sync.
- **`rules.md`:** the rules never to break.
- **`gotchas.md`:** symptoms, causes and fixes.
- **`workflow.md`:** the docs, commits, checking a change.
- **`api.md`:** r1d's routes, events and harness, and the agents' tools.
- **`enee-spec.md`:** the app format.

## Where it works

- **Claude Code:** the command below installs it in a project's `.claude/skills/r1os`, or in `~/.claude/skills/r1os` for every project.
- **Other agents that read skills:** the same folder layout.
- **An agent on r1os:** say "add the skill at" and the link to its `SKILL.md`. It asks you on the device first.
