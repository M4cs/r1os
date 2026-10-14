---
title: Apps
description: Write, build and edit apps in EneeSpec.
group: build
order: 1
---

# Apps

An app is an **EneeSpec**: one YAML file that r1d compiles and runs. It has no network of its own and sees only its own tables; anything more needs a permission you grant. Your agent writes one from a sentence ("make me a water tracker"), or you write it.

The full reference is `prompts/builder.md`, with the design guide in `prompts/design.md` and working examples in `fixtures/apps/`.

## The format

A spec starts with `enee: 1`, then these sections, in order. Only `app` and `views` are required.

| Section | What it holds |
|---|---|
| `app` | `id`, `name`, `icon` (Phosphor), `archetype`, `accent`, `description` |
| `permissions` | What it needs beyond its own data, each with a reason |
| `settings` | Values you tune: a type and a default |
| `data` | SQLite tables: `field: type`, `!` for not null, `= default` |
| `queries` | Named read-only `SELECT`s, re-run when their tables change |
| `state` | Screen-only values |
| `services` | Web data: `http.json`, `http.rss` or `http.fetch`, with `every` |
| `actions` | Named changes: `params` and `do` steps (`sql`, `toast`, `navigate`, `copy`, `play`…) |
| `views` | Blocks: `stat`, `timer`, `button`, `wheel`, `chart`, `list`, `game`, `card`… |
| `triggers` | What runs while it's closed: `every`, `at` or `on` |
| `logic` | Its own JavaScript |
| `glance` | A widget for the glance screen |
| `checks` | One to three tests, run before install |

Views bind with `{{path | filter}}`, like `{{today.total}}`; arithmetic belongs in SQL.

**Design for the wheel and voice:** about 330 × 410 px of big type, about four wheel stops on the first screen, no text fields. What you'd type becomes an action with params that your agent runs when you say it ("add milk").

## Example: a reminder

A nudge to stretch every hour, outside quiet hours. It asks for `schedule` and `notify` when first opened.

```yaml
enee: 1
app: { id: stretch, name: Stretch, icon: person-simple-tai-chi, archetype: habit, accent: green, description: A nudge to stand up and stretch. }
permissions:
  schedule: Remind you even when the app is closed
  notify: Show the reminder
settings:
  every: { type: duration, default: 1h, label: Remind me every, min: 60 }
  quiet: { type: time_range, default: "22:00-08:00", label: Quiet hours }
data:
  stretches:
    at: datetime = now
queries:
  today: SELECT count(*) AS n FROM stretches WHERE is_today(at)
actions:
  done:
    do:
      - sql: INSERT INTO stretches DEFAULT VALUES
      - toast: Nice
views:
  main:
    - stat: { label: Today, value: "{{today.n}}", caption: stretches }
    - button: { label: I stretched, action: done, icon: check }
triggers:
  nudge:
    every: "{{settings.every}}"
    skip: "{{settings.quiet}}"
    do: [ { notify: { title: Time to stretch, body: "{{today.n}} so far today" } } ]
checks:
  - do: { action: done }
    expect: { today.n: 1 }
```

## Example: a weather widget

The weather where you are, from wttr.in (no key), with a glance card.

```yaml
enee: 1
app: { id: weather-live, name: Weather now, icon: cloud-sun, archetype: widget, accent: cyan, description: The weather where you are. }
permissions:
  net.http: { hosts: [wttr.in], reason: Get the weather where you are }
services:
  now: { use: http.json, url: "https://wttr.in/?format=j1", every: 30m }
views:
  main:
    - stat:
        label: "{{services.now.nearest_area.0.areaName.0.value | default:Weather}}"
        value: "{{services.now.current_condition.0.temp_C | default:–}}°"
        caption: "{{services.now.current_condition.0.weatherDesc.0.value | default:Waiting for the forecast}}"
    - list:
        data: services.now.weather
        title: "{{row.date | date}}"
        caption: "{{row.mintempC}}° to {{row.maxtempC}}°"
        empty: The outlook shows up here
    - text: { text: "{{services.now.error}}", tone: muted, visible: "{{services.now.error}}" }
glance:
  type: weather
  place: "{{services.now.nearest_area.0.areaName.0.value | default:Here}}"
  temp: "{{services.now.current_condition.0.temp_C}}"
  condition: "{{services.now.current_condition.0.weatherDesc.0.value}}"
  feels: "{{services.now.current_condition.0.FeelsLikeC}}"
  days: { each: services.now.weather, take: 2, day: "{{row.date | date}}", high: "{{row.maxtempC}}", low: "{{row.mintempC}}", condition: "{{row.hourly.4.weatherDesc.0.value}}" }
```

## How a build runs

Ask your agent, or `POST /api/create {brief, model?, big?}`. Most apps take 10–30 seconds, streaming `build` events, and one build runs at a time (a second gets 409).

1. **The model writes the spec** in one turn with no tools: sonnet by default; haiku, opus or codex in Settings.
2. **r1d checks it:** that it compiles and can run on the device, its own checks, and its web data, fetched once.
3. **Problems go back** to the model, up to twice (three times with `big: true`).
4. **It installs** as a new app, never over another (a taken id becomes `<id>-2`), and asks about its permissions when first opened.

`POST /api/builds/stop`, or two clicks on its card in the chat, stops a build.

## Edit, fix, undo

- **Edit:** tell your agent, hold the button on the app's page in Manage apps, or `POST /api/apps/{id}/edit {request}`. The model returns only what changes; r1d snapshots the data before an edit drops tables or columns.
- **Undo** restores the previous version and its data from the snapshot. Every version is kept.
- **Fix** (Manage apps, or `fix_app`) rebuilds the app with the errors it reported.
- **Remove** sends it to the trash for 7 days; restoring brings its data back.

## Permissions

There are six: `net.http: { hosts: [..], reason }` (web data and `r1.fetch`, to those hosts), `schedule` (triggers, and web data while closed), `notify`, `open.url`, `clipboard.write` and `audio.play`.

Only r1os's own screens grant them. An agent can only ask, on a card, every time; an edit that adds a host asks again. Web data is fetched while the app is on screen, or, with `schedule` or a widget, at most every 15 minutes while the display is on, never with it off.

## Widgets

A widget is an app's `glance:` card, about 330 × 170 px, of one type: `stats`, `bars`, `line`, `weather`, `headlines`, `steps`, `table`, `link` or `image`. Its fields bind the app's queries, settings and services. r1d fills it in while the app is closed, so it can't bind `logic`. `PUT /api/glance {apps}` picks and orders them.

## Games and logic

- **`game`** blocks run a small JavaScript game on a canvas, in a sandboxed worker with no network, with the wheel and button through an `r1` object.
- **`logic:`** is the app's own code for rules and calculations. It publishes `values` for views (`{{logic.x}}`), runs the app's actions, fetches only granted hosts, and gets a trial run before install.

## Apps by voice

Agents use apps without opening them (`describe_app`, `query_app`, `run_action`), and what they run can be undone for 10 minutes. System One logs simple things with no agent at all ("log two glasses of water").

## From the command line

```bash
tools/r1h apps                                    # installed apps
tools/r1h install fixtures/apps/water.enee.yaml   # compile and install
tools/r1h create "Count the coffees I drink each day"
tools/r1h export water-log [file] [--data]        # --data: with its data and settings
tools/r1h remove water-log
```

Through the API, `POST /api/apps {source}` installs a spec (422 with what's wrong) and `POST /api/create {brief}` builds one. Each build costs a little on your subscription. To give an app to another device on your Wi-Fi, share it by QR from its page.
