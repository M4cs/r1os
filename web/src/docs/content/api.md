---
title: API
description: r1d's routes, events and test harness.
group: build
order: 2
---

# API

## Auth

r1d serves `127.0.0.1:7171`: HTTP for commands, `/ws` for events. Errors are JSON with an `error` field.

- **On the device,** `/api`, `/harness` and `/ws` need r1d's per-boot token as `Authorization: Bearer <token>`, or answer 401. It's in `/data/r1os/run/token` (root only) and changes whenever r1d starts.
- **On the device, every request must be addressed to r1d:** a `Host` of `127.0.0.1`, `localhost` or `[::1]`, on any port. Any other name gets 421, so a web page on a name that resolves to 127.0.0.1 can't reach r1d.
- **The simulator** needs no token. `/mcp` takes per-process agent tokens instead ([Agents](/docs/agents)).

From a computer with adb, the tools forward local port 7172 to the device's 7171 and read the token as root:

```bash
T=$(source tools/lib.sh; r1d_token)
tools/r1h state                          # sets up the forward
curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/state
```

## Routes

The main ones. Every route, with its body and answer, is in the repo's `docs/specs/r1d.md`.

| Area | Routes |
|---|---|
| Device | `GET /api/state` · `POST /api/screen {on}` · `POST /api/motor {deg}` · `/api/device` · `/api/time` · `/api/wifi` · `PUT /api/boot {verbose}` · `POST /api/power {action}` · `GET /api/backup` |
| Look and home | `/api/look` ([below](#the-look)) · `PUT /api/home {pins}` · `PUT /api/glance {apps}` (widgets, in order) |
| Voice | `GET /api/voice` · `POST /api/voice/talk {op: start\|send\|cancel}` (talking by touch, as when docked) |
| Agents | `/api/agents` · `PUT /api/agents/{id}` (the whole profile) · `PUT /api/agents/{id}/can {area, level}` · `/api/memories` · `/api/accounts` · `/api/connections` · `/api/skills` |
| Chats | `POST /api/chats {agent?, brain?}` · `POST /api/chats/{id}/messages {text}` (the reply streams as `chat` events) · `POST /api/chats/warm` · `POST /api/chats/{id}/stop` |
| Approvals | `GET /api/approvals` · `POST /api/approvals/{id} {allow, always?}` |
| Apps | `POST /api/apps {source}` (422 with diagnostics) · `POST /api/create {brief}` · `/api/apps/{id}/edit`, `/undo`, `/fix`, `/restore`, `/export`, `/settings`, `/permissions` · `POST /api/apps/{id}/run {action, params}` |
| Android | `GET /api/android/apps` · `POST /api/android/open {package \| link \| store \| screen}` (allowlisted) · `GET /api/android/notifications` |
| Everything else | `/api/alarms` · `/api/timers` · `/api/routines` · `/api/media` · `/api/recorder` · `/api/journal` · `/api/translator` · `/api/player` · `/api/share` · `/api/scan` · `/api/link` · `/api/lock` |

`/api/apps/window/result` and `/api/browser/result` carry the host's answers to agents' `app_window` and `browser` steps.

## Events

`/ws` sends `{"type": "hello", "state": …}`, then one JSON event per message. Commands go over HTTP.

```json
{ "seq": 12, "t_ms": 73295, "source": "hw", "type": "gesture", "kind": "double_tap" }
```

`source` is `hw` (the real wheel and button), `sim` (harness input) or `api`.

| Group | Events |
|---|---|
| Input | `wheel`, `button`, `gesture {kind}`, `motor`, `volume` |
| Voice | `voice {state, text?}`, `voice_level`, `intent` |
| Agents | `agent_state`, `chat`, `unread`, `mail`, `agents_changed`, `account`, `app_window`, `browser` |
| Apps | `build`, `app_installed`, `app_removed`, `app_changed`, `app_errors`, `open`, `notify`… |
| Device | `look`, `home`, `glance`, `lock`, `approval_asked`, `approval_done`, `open_screen`, `link`… |
| Time and media | `alarm`, `reminder`, `timer`, `routine`, `media_changed`, `recording`, `translation`… |

The TypeScript types are in `shell/src/r1.ts`, kept in sync with r1d.

## The harness

`/harness/*` drives and measures the device for tests, behind the same token.

| Route | Does |
|---|---|
| `POST /harness/wheel {delta}`, `/button {action, ms?}` | Input through the real gesture engine |
| `GET /harness/state`, `/events?after=`, `/screenshot` | State, recent events, a PNG of the screen |
| `POST /harness/say {text}` | Words as a hold would send them, no mic |
| `POST /harness/app_window`, `/browser {op, args}` | An agent's step, with no cards |
| `POST /harness/dry_run {chat, on}` | A test chat that presses nothing that adds to a cart, orders, pays or sends |
| `POST /harness/commit_card` | The final-step card for the app on screen; nothing is pressed |
| `POST /harness/tilt {side}`, `/rest {step}` | Docked mode and the resting glance screen, hands-free |
| `GET /harness/jail`, `/perf` | The agent jail; latency marks, app window steps included |
| `POST /harness/system1 {text}`, `/hear`, `/translate` | What System One makes of words; speech timed on a WAV |

## The look

`PUT /api/look` merges what you send (`null` resets a field), and the shell redraws in it.

```bash
curl -s -X PUT -H "Authorization: Bearer $T" -H 'Content-Type: application/json' \
  -d '{"mode": "light", "accent": "teal", "glance": {"clock": "dial"}}' 127.0.0.1:7172/api/look
```

| Field | Values |
|---|---|
| `mode` | `dark`, `light` |
| `accent` | `orange`, `amber`, `yellow`, `lime`, `green`, `teal`, `cyan`, `blue`, `indigo`, `purple`, `pink`, `red`, or `#rrggbb` |
| `font` | `rubik`, `roboto`, `flex`, `source`, `serif`, `mono`, `typewriter`, `casual`, `script`, `smallcaps` |
| `glance.clock` | `bold`, `light`, `condensed`, `stacked`, `serif`, `mono`, `words`, `dial` |
| `glance.align` · `glance.agent` | `left`, `center` · `center`, `left`, `right`, `hidden` |
| `glance.hide` | any of `date`, `status`, `alarm`, `notifications`, `playing`, `widgets`, `greeting` |
| `glance.background`, `home.background` | `{kind: plain\|tint\|gradient\|mesh\|texture\|photo, color?, colors?, …}` |

Agents' eyes and wear are part of their profile (`PUT /api/agents/{id}`).
