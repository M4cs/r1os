# EneeSpec: the app format

## What an app is

An r1os app is an **EneeSpec**: a compact YAML file that r1d compiles into a running mini-app (views, named SQL queries, actions, tables, settings, web data and checks). Apps are spec-only: they have no network of their own, they reach only their own SQLite tables, and anything beyond their own data needs a permission you grant. The language is ported from eneeware, cut down to what the R1 runs.

Agents write specs for you from a sentence ("make me a water tracker"), but you can write or edit one yourself and install it.

## The shape of an `enee.yaml`

Sections in this order; only `enee`, `app` and `views` are required.

| Section | What it holds |
|---|---|
| `enee: 1` | The language version |
| `app` | `id` (kebab-case), `name`, `icon` (a Phosphor name), `archetype`, `accent`, `description` |
| `permissions` | What it uses beyond its own data, each with a reason you'll read |
| `settings` | Values you tune (int, real, text, bool, duration, time, time_range, date, enum), each with a default |
| `data` | SQLite tables: `field: type`, `!` for not null, `= default`; `id`, `created_at`, `updated_at` are automatic |
| `queries` | Named read-only `SELECT`s that re-run when their tables change; helpers like `is_today(at)`, `local_date(at)`, `days_ago(n)`, `streak(day)` |
| `state` | Screen-only values; `_persist` keeps keys across restarts |
| `services` | Web data: `http.json`, `http.rss` or `http.fetch`, each with an https URL and `every` |
| `actions` | Named changes: `params` and `do` steps (`sql`, `toast`, `set`, `navigate`, `refresh`, `open_url`, `copy`, `play`/`pause`/`resume`/`toggle`/`stop`); `confirm:` asks first |
| `views` | Lists of blocks: `stat`, `timer`, `gauge`, `progress`, `heading`, `text`, `button`, `quick_actions`, `wheel`, `chart`, `calendar_heatmap`, `list`, `game`, `logic_view`, `row`, `section`/`card`… |
| `triggers` | What it does while closed: `every`, `at` or `on` (start, open, close, `change:<table>`), with `skip`, `when`, and `sql`/`notify`/`refresh` steps |
| `logic` | The app's own JavaScript, for what blocks, SQL and services can't do |
| `glance` | A widget: one card for the glance screen |
| `checks` | One to three tests run on an empty database before install |

Views bind with `{{path | filter}}`: `{{today.total}}`, `{{settings.daily_goal}}`, `{{services.now.current_condition.0.temp_C | default:–}}`. Bindings are paths and filters only; any arithmetic belongs in SQL.

**Design for the wheel and for voice.** The content area is about 330 × 410 px with big type, about four wheel stops on the first screen. Apps don't need text fields: anything you'd add in your own words is an action with params, which your agent runs when you say it ("add milk").

## Example: a reminder

A nudge to stretch, every hour outside quiet hours. It asks for `schedule` and `notify` the first time it opens.

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

## Example: web data and a widget

The weather where the R1 is, from wttr.in (no key), with a glance card.

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

`create_app` (an agent), `POST /api/create {brief, model?, big?}`, or `tools/r1h create "<brief>"`:

1. **One build at a time.** A second gets 409. Progress streams as `build` events on `/ws`.
2. **A no-tools model turn** in the agent box: the system prompt is `prompts/builder.md` plus the R1 design guide (`prompts/design.md`), the message is `prompts/create.md` plus your brief. The model is the R1's setting (haiku, **sonnet** by default, opus, or **codex** to build through Codex), unless you ask for one.
3. **Writing:** `build {stage: writing, section}` as each top-level section streams.
4. **Compile and check:** r1d takes the first ` ```enee ` block, compiles it, checks what the R1 can't run, and runs its checks.
5. **Check its web data:** each service URL is fetched once; a URL that fails, or a binding to a field the response doesn't have, is a problem (with an outline of what the response does have).
6. **Repair:** problems go back to the model as fresh turns, up to 2 (3 for `big: true` builds, which also get ten-minute turns and more effort).
7. **Install:** `app_installed`, then `build {stage: done, app, cost_usd, seconds}`. A new app never replaces another: a taken id becomes `<id>-2`.

The first time the app opens, if it declares permissions you haven't decided on, a **consent card** asks: allow or don't allow.

A build can be stopped: `POST /api/builds/stop`, the agent's `stop_build`, or two clicks on its card.

## Editing, fixing and undo

- **Edit:** "make the water tracker show this week's total" (the agent's `edit_app`), holding the button on the app's page in Manage apps, or `POST /api/apps/{id}/edit {request}`. The model replies with one past-tense sentence and an `enee-patch` of only what changes:

  ```yaml
  queries:
    week: SELECT coalesce(sum(count), 0) AS total FROM glasses WHERE is_this_week(at)
  views:
    main:
      - stat: { label: This week, value: "{{week.total}}" }
  ```

  Entries replace by name; `name: null` removes one. The id never changes. If an edit removes tables or columns, r1d snapshots the database first, and the chat card names what was deleted.
- **Undo:** `undo_app_change` or `POST /api/apps/{id}/undo` reinstalls the previous version and refills the data from the snapshot. Every version is kept under `apps/<id>/versions/`.
- **Fix:** an app's page reports its runtime errors. **Fix it** in Manage apps, or `fix_app`, starts an edit asked to fix them.
- **Remove:** to the trash for 7 days; `restore_app` brings it back with its data.

## Permissions

| Permission | Needed by |
|---|---|
| `net.http: { hosts: [..], reason }` | Services (web data) and `r1.fetch`; every host listed |
| `schedule` | Triggers, and web data while the app is closed |
| `notify` | A trigger's `notify` step (it wakes the R1) |
| `open.url` | An action's `open_url` step (the R1's link viewer) |
| `clipboard.write` | An action's `copy` step |
| `audio.play` | `play`, `pause`, `resume`, `toggle`, `stop` steps |

Nothing else is granted. Only the shell's own screens decide: an app's page can't, and an agent can only ask (on a card, every time). Decisions are bound to the declaration, so an edit that adds a host asks again.

Web data is fetched only while the app is on screen, or, with `schedule` or a glance card, in the background at most every 15 minutes while the display is on. Nothing fetches with the screen off.

## Widgets

A widget is an app's `glance:` card: one card type (`stats`, `bars`, `line`, `weather`, `headlines`, `steps`, `table`, `link`, `image`) whose fields bind the app's queries, state, settings and services. r1d fills it in while the app is closed; it never binds `logic` or `player`. The card gets about 330 × 170 px. Lists come from rows with `{ each: <query | services.x.items>, take?, …fields }`. Order them with `PUT /api/glance {apps}` or the agent's `glance_widgets`.

## Games and logic

- **`game`** blocks run a small JavaScript game in a sandboxed worker (no network or page), drawing on a canvas, with the wheel and button through an `r1` object.
- **`logic:`** is an app's own code for rules, computation, or requests that build on each other. It publishes `values` that views bind as `{{logic.x}}`, runs the app's own actions, and fetches only its granted hosts through r1d. It's trial-run in the box before install.

## Using apps without opening them

Agents read and act on apps by voice: `describe_app` (actions, queries, settings, last web data), `query_app`, `run_action` (a whole action, undoable for 10 minutes with `undo_action`). System One runs simple logging actions with no agent ("log two glasses of water").

## From the command line

```bash
tools/r1h apps                                 # installed apps
tools/r1h install fixtures/apps/water.enee.yaml   # compile and install (diagnostics if rejected)
tools/r1h create "Count the coffees I drink each day, with a daily limit"   # build on the R1 (uses your subscription)
tools/r1h export water-log [file] [--data]     # save the spec, or with its data and settings (.enee.json)
tools/r1h remove water-log
tools/r1h launch water-log                     # open it on screen
```

`python3 tools/eval-apps.py` builds a suite of briefs on the R1 and reports times, repairs, cost and permissions. Every build costs a little, so it asks first.

The full reference the R1's builder uses is `prompts/builder.md`, with the design guide in `prompts/design.md`; `fixtures/apps/` has working examples.
