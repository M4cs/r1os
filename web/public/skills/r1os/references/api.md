# The r1d API

## Basics

- r1d listens on `127.0.0.1:7171`: HTTP for commands, `/ws` for events.
- **On the R1**, `/api`, `/harness` and `/ws` need r1d's per-boot token as `Authorization: Bearer <token>`. It's made fresh each time r1d starts and kept in `/data/r1os/run/token` (root only). Without it: 401.
- **The simulator** (`r1d --sim`) is open, with no token.
- `/mcp` takes per-process agent tokens instead (see the MCP tools below). The shell's static files need nothing.
- Errors are JSON with an `error` field.

From a development PC (the tools forward local port 7172 to the R1's 7171 over adb):

```bash
T=$(source tools/lib.sh; r1d_token)     # read as root over adb
tools/r1h state                          # sets up the forward
curl -s -H "Authorization: Bearer $T" 127.0.0.1:7172/api/state
```

## Device and system

| Path | Method | Purpose |
|---|---|---|
| `/api/state` | GET | Version, uptime, input devices, motor, battery, memory, storage, counters |
| `/api/screen` | POST | `{on}`: wake or sleep the display |
| `/api/motor` | POST | `{deg}` (0–180), `{face: you\|world}` or `{home: true}`: turn the camera |
| `/api/device` | GET | The R1's settings (brightness, timeout, volume, sounds, haptics, time zone, build model…) and their options |
| `/api/look` | GET, PUT | The look: mode, accent, font, glance layout, backgrounds |
| `/api/home` | GET, PUT | Apps pinned to home (`{pins}`, 12 at most) |
| `/api/glance` | GET, PUT | The glance screen's widgets, filled in (`{apps}` sets which and in what order) |
| `/api/time` | GET, POST | `{zone}` or `{automatic}` |
| `/api/voice` | GET | Push-to-talk's state: model loading, ready or missing |
| `/api/voice/talk` | POST | `{op: start \| send \| cancel}`: talking by touch (docked mode's mic), the side button's own hold; a talk nobody sends ends after a minute |
| `/api/wifi`, `/api/wifi/scan`, `/api/wifi/connect` | GET, POST | Wi-Fi status, scan, connect |
| `/api/whatsup` | GET | Unread replies, reminders, alarms, timers, routines, journal, notifications, playing |
| `/api/missed` | GET, DELETE | Android notifications that came while the display was off |
| `/api/power` | POST | `{action: off\|restart}` |
| `/api/backup`, `/api/backup/file` | GET | r1os's data (never the agent box) for a backup |

## Agents and chats

| Path | Method | Purpose |
|---|---|---|
| `/api/agents` | GET, POST | List agents (with state, focus, the budget), or make one |
| `/api/agents/{id}` | GET, PUT, DELETE | One agent's profile (PUT is the whole profile) |
| `/api/agents/order` | PUT | `{ids}`: the wheel's order |
| `/api/agents/{id}/can` | PUT | `{area, level}`: what it may change |
| `/api/agents/paused` | PUT | Pause agents waking each other |
| `/api/chats` | GET, POST | `?agent=&kind=`; POST `{agent?, brain?, mode?}` starts one |
| `/api/chats/{id}` | GET, DELETE | A chat's messages |
| `/api/chats/{id}/messages` | POST | `{text, app?, images?}`: send; the reply streams as `chat` events |
| `/api/chats/warm` | POST | `{agent?, chat?}`: start a process before the words arrive |
| `/api/chats/{id}/seen`, `/stop` | POST | Mark replies seen; stop the running turn |
| `/api/memories` | GET | `?agent=`: shared facts plus that agent's own |
| `/api/mail` | GET | Messages between agents |
| `/api/approvals`, `/api/approvals/{id}` | GET, POST | Waiting requests; `{allow, always?}` (the shell's card answers) |
| `/api/accounts`, `/api/accounts/{brain}/login`, `/logout` | GET, POST | Sign the CLIs in and out |
| `/api/connections` (+ `/check`, `/login`, `/agent`, `/always`, `/add`, `/remove`) | GET, POST | The CLIs' connectors and your own MCP servers |
| `/api/skills`, `/api/skills/on`, `/api/skills/{name}` | GET, POST, DELETE | Agent Skills |

## Apps

| Path | Method | Purpose |
|---|---|---|
| `/api/apps` | GET, POST | List; POST `{source, data?, settings?}` compiles and installs (422 with diagnostics) |
| `/api/apps/{id}` | GET, DELETE | The compiled app and its source; remove (to the trash) |
| `/api/apps/{id}/export` | GET | The app with its data and settings |
| `/api/create` | POST | `{brief, model?, big?}`: start a build |
| `/api/builds/stop`, `/api/builds/model` | POST; GET, PUT | Stop a build; the build model |
| `/api/apps/{id}/edit`, `/undo`, `/fix`, `/restore` | POST | Edit by request, undo a change, fix reported problems, restore from the trash |
| `/api/apps/{id}/history`, `/api/trash` | GET | Versions; the trash |
| `/api/apps/{id}/settings`, `/permissions`, `/erase` | GET, POST | Settings, permission decisions, start over |
| `/api/apps/{id}/run`, `/undo_run` | POST | Run an action without opening it; undo it |
| `/api/apps/{id}/session` | POST | A page token for the app's runtime endpoints (`init`, `query`, `step`, `state`, `services`, `open_url`, `copy`, `media`, `fetch`) |

## Time, media and more

| Path | Purpose |
|---|---|
| `/api/alarms`, `/api/timers`, `/api/routines` | Alarms and reminders (and the to-do list), timers and pomodoros, routines |
| `/api/media`, `/api/media/{id}`, `/file`, `/peaks` | Photos and recordings |
| `/api/recorder`, `/api/journal`, `/api/translator` | The Recorder, the Journal, the Translator |
| `/api/player` | What's playing |
| `/api/android/apps`, `/api/android/open`, `/api/android/notifications` | Android apps, opening them (allowlisted), their notifications |
| `/api/android/front`, `/api/android/key` | The Android app in front; a key (`center`, `back`) for it |
| `/api/share`, `/api/scan` | Share one item by QR on the Wi-Fi; read a QR from a camera frame |
| `/api/link` (+ `/pair`, `/scan`, `/ping`, `/close`, `/{id}`) | Linked computers |
| `/api/lock` | The PIN lock (not reachable from a linked computer) |

## The host's side

The host app answers r1d's requests and reports what only Android knows. Not for agents or scripts, but useful when reading the code:

| Path | Purpose |
|---|---|
| `POST /api/apps/window/result {id, ok, result?, error?}` | The host's answer to an `app_window` request (the app window, ADR-0061) |
| `POST /api/browser/result {id, ok, result?, error?}` | The host's answer to a `browser` request (the R1's browser, ADR-0054) |
| `POST /api/battery {charge_ms}` | Android's estimate of the time to a full charge, for the `battery` tool |
| `POST /api/display {on}` | The display going on or off: apps' background web data waits while it's off; with a PIN set, off locks the R1 |
| `PUT /api/android/apps` | The installed Android apps |

## Events on `/ws`

The first message is `{"type": "hello", "state": …}`; then one JSON event per message:

```json
{ "seq": 12, "t_ms": 73295, "source": "hw", "type": "gesture", "kind": "double_tap" }
```

`source` is `hw` (the real wheel and button), `sim` (harness input) or `api`. `/ws` carries events only; commands go over HTTP.

| Group | Events |
|---|---|
| Input | `wheel {delta}`, `wheel_suppressed`, `button {down}`, `gesture {kind, held_ms?, woke?}`, `motor {deg, moving}`, `volume {on}`, `tilt {side}` and `rest {step, brightness}` (from the harness, for docked mode) |
| Voice | `voice {state, text?, intent?}`, `voice_level`, `intent`, `talk_cancel` |
| Agents | `agent_state {agent, state, chat, by?}`, `chat {agent, chat, kind, text?…}` (user, delta, tool, build, done, error, note, mail, mail_out, card, stopped, routine, quiet), `unread`, `mail`, `agents_changed`, `account` |
| Apps | `build {stage…}`, `app_installed`, `app_removed`, `app_changed {app, tables}`, `app_services`, `app_permissions`, `app_errors`, `app_player`, `open`, `open_link`, `copy`, `notify` |
| The R1 | `look`, `home`, `glance`, `device_set`, `lock`, `approval_asked`, `approval_done`, `open_screen`, `connections_changed`, `link`, `changed_from`, `bootanim` |
| For the host | `browser {id, op, agent, chat, args}` and `app_window {id, op, agent, chat, args}` (do `op`, answer at the result route), `android_media`, `android_notice` |
| Time | `alarm`, `reminder`, `alarms_changed`, `timer`, `timer_phase`, `timers_changed`, `routine`, `routines_changed`, `awake` |
| Media | `media_changed`, `recording`, `recording_level`, `journal_changed`, `note`, `translator_stage`, `translation`, `player` |

The TypeScript types live in `shell/src/r1.ts` and are kept in sync with the spec and r1d.

## The harness (development)

`/harness/*` is for driving and measuring the device. Most of it needs r1d started with `--harness`; `perf`, `abilities`, `browser`, `app_window`, `dry_run` and `commit_card` are always there (behind the token on the R1).

| Path | Purpose |
|---|---|
| `GET /harness/state`, `GET /harness/events?after=` | State plus recent events; the event ring buffer |
| `POST /harness/wheel {delta}`, `POST /harness/button {action, ms?}` | Simulated input through the real gesture engine |
| `POST /harness/tilt {side}` | How the R1 is held, for docked mode: 1 on its right side, -1 its left, 0 upright, `null` back to the accelerometer |
| `POST /harness/rest {step, brightness?}` | Hold the glance screen's rest at a step (0 awake, 1 still, 2 low; `null` its own timer) |
| `GET /harness/screenshot` | A PNG of the screen (device only) |
| `POST /harness/say {text}` | A scripted utterance: the same `voice` events as a real hold, no mic |
| `POST /harness/open {app}`, `POST /harness/trigger {app, trigger}` | Open an app; fire one of its triggers now |
| `GET /harness/jail` | Probe the agent jail |
| `GET /harness/perf`, `DELETE /harness/perf` | Latency marks (speech, first reply word, whole replies, app window steps as `app.<op>`); reset them |
| `POST /harness/system1 {text}` | What System One makes of some words |
| `POST /harness/hear`, `/translate`, `/tts`, `/paradee` | Speech-to-text and the Translator timed on a WAV; text-to-speech voices timed |
| `POST /harness/tidy {chat}`, `POST /harness/summarize` | Tidy a chat into a fresh session now; the Recorder's title and summary for some words |
| `GET /harness/abilities/{agent}` | What an agent's prompt says it can do |
| `POST /harness/browser {op, args, agent?, chat?}` | One browser op as a tool would ask it, and the host's answer |
| `POST /harness/app_window {op, args, agent?, chat?}` | One app window op straight to the host, no cards (`{op: "size"}` changes the window's size, for measuring) |
| `POST /harness/dry_run {chat, on}` | Mark a test chat a dry run: nothing that adds to a cart or finishes anything is pressed in it |
| `POST /harness/commit_card {agent, chat, step?}` | Show the commit card for the app window's screen as it is: `{allowed, pressed: false, order}`; nothing is pressed |
| `GET /harness/net`, `POST /harness/bootanim` | r1d's network, step by step; draw the boot animation now |

## The look API

`PUT /api/look` merges only what you send, checks the result, saves it and pushes `look {look}`; the shell rebuilds itself in it. `null` puts a field back to its default.

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
| `glance.align` | `left`, `center` |
| `glance.agent` | `center`, `left`, `right`, `hidden` |
| `glance.hide` | any of `date`, `status`, `alarm`, `notifications`, `playing`, `widgets`, `greeting` |
| `glance.background`, `home.background` | `{kind: plain\|tint\|gradient\|mesh\|texture\|photo, color?, colors?, style?, grain?, flow?, photo?, dim?}` |

Agents' eyes and wear are part of the agent's profile (`PUT /api/agents/{id}`).

## r1d's MCP endpoint and tools

Agents act on the R1 through r1d's MCP server: JSON-RPC 2.0 over HTTP at `POST /mcp` (`initialize`, `tools/list`, `tools/call`). Each agent process gets its own bearer token, mapped to its agent and chat, so every tool acts as the caller. The CLIs know the server as `r1`, so tools show up as `mcp__r1__<name>`.

```json
{ "mcpServers": { "r1": {
    "type": "http",
    "url": "http://127.0.0.1:7171/mcp",
    "headers": { "Authorization": "Bearer <a token r1d issued for this process>" } } } }
```

The tools, by area:

| Area | Tools |
|---|---|
| Using apps | `list_apps`, `describe_app`, `query_app`, `run_action`, `undo_action`, `open_app` |
| Building and managing apps | `create_app`, `edit_app`, `fix_app`, `install_app`, `stop_build`, `app_history`, `undo_app_change`, `delete_app`, `restore_app`, `set_app_settings`, `set_app_permission` |
| Cards and news | `show_card`, `read_feed` |
| Memory | `remember`, `list_memories`, `forget` |
| Alarms, reminders, timers, routines | `set_alarm`, `set_reminder`, `tick_todo`, `set_timer`, `list_alarms`, `cancel_alarm`, `stop_alarm`, `set_routine`, `list_routines`, `cancel_routine` |
| Catching up | `whats_up`, `what_you_missed` |
| Photos, recordings, journal | `list_media`, `view_photo`, `label_media`, `start_recording`, `stop_recording`, `read_recording`, `journal_add`, `journal_search`, `journal_list` |
| Music | `control_media`, `play_music` |
| Android apps | `list_android_apps`, `open_android_app`, `show_android_app`, `list_notifications`, `open_notification`, `dismiss_notification`, `reply_notification` |
| Android apps, in the app window | `app_open`, `app_read`, `app_screenshot`, `app_click`, `app_type`, `app_scroll`, `app_back`, `app_find`, `app_wait`, `app_ask_user`, `app_close`, `app_do` |
| The R1's browser | `browser_open`, `browser_read`, `browser_text`, `browser_find`, `browser_click`, `browser_type`, `browser_select`, `browser_key`, `browser_scroll`, `browser_screenshot`, `browser_back`, `browser_close`, `browser_do`, `browser_ask_user` |
| Look, home and widgets | `set_look`, `set_agent_look`, `home_apps`, `glance_widgets` |
| The R1's settings | `device_settings`, `change_device_settings`, `set_time_zone`, `battery`, `open_screen` |
| Other agents | `list_agents`, `ask_agent`, `tell_agent` |
| Skills | `add_skill` |

One more, `permission`, isn't for agents: it's the CLI's permission prompt for connector tools (reads go ahead; actions ask on the R1).

**The app window's tools** (ADR-0061) drive an installed Android app on a display of its own while the user watches in the chat:
- `app_open {package?, link?, restart?}` opens it (an app already open on the R1's own screen comes in as it is); `link` opens it on a page (an app's own link, or a web address it claims).
- Every step answers with the screen as lines with refs (`[a12]`) and its picture (600×1200). `app_click` takes a ref, words (`target`) or x,y, and `final: true` for a step that commits; `app_type` takes a ref or words and `submit`; `app_find` scrolls until `target` shows (up to 8 scrolls).
- Acts already wait for their screen to settle (up to 6 s); `app_wait` is only for a screen still loading.
- `app_do {steps}` runs up to 10 steps in order (`{tool: click, target: "Search"}`…), stopping at the first that fails.
- `app_ask_user {why}` hands the app to the user (sign-in, a password or code, payment screens, a CAPTCHA, Android's permission prompts) and waits up to 5 minutes; `app_close {for_user: true}` gives it to them on the R1's own screen.
- They run one at a time, in the order sent. The browser's `browser_*` tools work the same way, in a WebView on the R1 (web pages only, never the R1 itself or the local network).

## Changing the R1, with your approval

Tools that change the R1, or act for you, go through approvals. Each agent has a level per area, set in its space (`PUT /api/agents/{id}/can {area, level}`):

| Area | Covers |
|---|---|
| `look` | The look, home's pinned apps, the glance screen's widgets, agents' looks |
| `device` | The R1's settings: brightness, screen timeout, volume, sounds, haptics, time zone, build model, pausing agents… |
| `apps` | Apps' settings and permissions |
| `browser` | Acting in the R1's browser: looking runs; the first click, typing or key in a turn asks, and "allow" covers the rest of that turn |
| `android` | Acting in an Android app in the app window, the same way |
| `files` | An agent working on a computer: changing files in its own folders (allowed by default) |
| `commands` | An agent working on a computer: running commands there |
| `outside` | An agent working on a computer: anything outside its folders (Codex's reads don't ask yet) |

Levels are **ask** (the default, except `files`: an approval card, waking the display, and the tool waits up to 2 minutes), **allow** and **never**. Some asks come whatever the level (only **never** stops them), with no "always":
- granting an app a permission;
- the first time agents use an Android app ("let agents use Uber?", naming the app's terms when they forbid automation; a yes is kept for every agent);
- **a final step in an app**: placing an order, requesting a ride, paying, subscribing or sending a message. The commit card names the app and shows the order read from its screen (items, total, where it goes, how it's paid); no level skips it, and it's refused outright in a turn another agent's message started.

Connectors' action tools ask on their own cards, whose "always" allows that one tool; adding a skill asks too. Destructive tools (`delete_app`, actions marked `confirm:`) also need `confirmed: true`, after the agent has asked you.
