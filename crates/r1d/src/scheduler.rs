//! Reminders and other app triggers on the R1 (ADR-0024, docs/specs/apps.md §6): an app's `triggers` run in
//! r1d, for apps you let run in the background (`schedule`, crate::broker), whether or not they're open.
//!
//! - **Timed** (`every`, `at`): due times count from the last run, kept in `core.db`, so restarts and sleep
//!   neither reset nor repeat them. A trigger seen for the first time starts counting; it doesn't fire at
//!   once. A time missed while the R1 was off fires once.
//! - **Events** (`on`): `start` (r1d starting), `open`/`close` (the app's screen), and `change:<table>` after
//!   the app's own writes (a step, the agent's `run_action`, a feed's new items, its settings). Writes by
//!   triggers themselves don't fire `change:` triggers.
//! - `skip` (a local time range) and `when` (a SQL guard) hold a run back; a held-back timed trigger still
//!   counts as run, so it tries again one period later.
//! - Steps: `sql` (params from settings), `notify` (needs `notify`: the display wakes and the shell shows it)
//!   and `refresh`.
//! - **Waking:** the loop sleeps until the next due trigger, or until something changes, with no timer when
//!   nothing is due. With the screen off the CPU sleeps too, so crate::wake sets a kernel alarm for that time.
//!
//! Ported from eneeware apps/desktop/src-tauri/src/scheduler.rs at 61bc7a7 (`run_due`, `fire`, `timing`,
//! `next_due`, `time_range`, `truthy`), on tokio with crate::wake instead of a thread with a one-minute check.

use crate::hub::{Event, Hub, Source};
use enee_spec::compiled::CompiledApp;
use enee_spec::eval::{self, Format};
use enee_spec::sql::Clock;
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DAY_MS: i64 = 86_400_000;
/// Triggers run at most once a minute (the compiler holds literal periods to this too).
const MIN_EVERY_MS: i64 = 60_000;

#[derive(Debug)]
enum Happened {
    Start,
    View { app: String, open: bool },
    Changed { app: String, tables: Vec<String> },
}

/// When a timed trigger is due.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Timing {
    /// A period, in ms.
    Every(i64),
    /// A local time of day, in minutes after midnight.
    At(i64),
}

/// A notification a trigger made.
#[derive(Debug, Clone)]
pub struct Notice {
    pub app: String,
    pub title: String,
    pub body: String,
}

pub struct Scheduler {
    clock: Clock,
    queue: Mutex<VecDeque<Happened>>,
    wake: tokio::sync::Notify,
    alarm: Arc<crate::wake::Alarm>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler { clock: Clock::system(), queue: Mutex::default(), wake: tokio::sync::Notify::new(), alarm: crate::wake::Alarm::start() }
    }

    /// Starts the loop (once, at startup) and runs `on: start` triggers.
    pub fn start(hub: &Arc<Hub>) {
        hub.scheduler.push(Happened::Start);
        tokio::spawn(run(hub.clone()));
    }

    fn push(&self, h: Happened) {
        self.queue.lock().unwrap().push_back(h);
        self.wake.notify_one();
    }

    /// The app's screen opened or closed.
    pub fn view(&self, app: &str, open: bool) {
        self.push(Happened::View { app: app.to_owned(), open });
    }

    /// The app (or the agent for it) wrote these tables.
    pub fn changed(&self, app: &str, tables: &[String]) {
        if !tables.is_empty() {
            self.push(Happened::Changed { app: app.to_owned(), tables: tables.to_vec() });
        }
    }

    /// Something changed what's due (an install, a permission, a setting): look again.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Apps with triggers that may run them (`schedule` granted).
    fn scheduled(hub: &Hub) -> Vec<Arc<crate::apps::Installed>> {
        hub.apps
            .list()
            .into_iter()
            .filter_map(|s| hub.apps.get(&s.id))
            .filter(|i| !i.app.triggers.is_empty() && crate::broker::allows(&hub.core, &i.app, "schedule"))
            .collect()
    }

    fn on_event(&self, hub: &Hub, happened: Happened) -> Vec<Notice> {
        let wanted = |app: &str, on: &str| -> Vec<(Arc<crate::apps::Installed>, Value)> {
            Self::scheduled(hub)
                .into_iter()
                .filter(|i| app.is_empty() || i.app.app.id == app)
                .flat_map(|i| {
                    let triggers: Vec<Value> =
                        i.app.triggers.iter().filter(|t| t.get("on").and_then(Value::as_str) == Some(on)).cloned().collect();
                    triggers.into_iter().map(move |t| (i.clone(), t))
                })
                .collect()
        };
        let runs = match &happened {
            Happened::Start => wanted("", "start"),
            Happened::View { app, open } => wanted(app, if *open { "open" } else { "close" }),
            Happened::Changed { app, tables } => tables.iter().flat_map(|t| wanted(app, &format!("change:{t}"))).collect(),
        };
        let mut notices = Vec::new();
        for (installed, trigger) in runs {
            notices.extend(self.fire(hub, &installed.app, &trigger, "event"));
        }
        notices
    }

    /// Runs the timed triggers that are due. Returns their notices and when the next one is due (ms).
    fn run_due(&self, hub: &Hub) -> (Vec<Notice>, Option<i64>) {
        let now = self.clock.now_ms();
        let offset = |ms: i64| self.clock.offset_ms(ms);
        let mut next: Option<i64> = None;
        let mut soonest = |t: i64| next = Some(next.map_or(t, |n: i64| n.min(t)));
        let mut notices = Vec::new();
        for installed in Self::scheduled(hub) {
            let app = &installed.app;
            let id = app.app.id.as_str();
            let mut settings: Option<Map<String, Value>> = None;
            for trigger in &app.triggers {
                let Some(name) = trigger.get("name").and_then(Value::as_str) else { continue };
                if trigger.get("every").is_none() && trigger.get("at").is_none() {
                    continue;
                }
                let settings = settings.get_or_insert_with(|| hub.apps.settings(id).unwrap_or_default());
                let Some(timing) = timing(trigger, &with_defaults(app, settings)) else {
                    tracing::warn!(app = id, trigger = name, "scheduler: can't read the trigger's time");
                    continue;
                };
                let Some(last) = hub.core.last_run(id, name) else {
                    // First sight: start counting from now.
                    hub.core.set_last_run(id, name, now);
                    soonest(next_due(timing, now, &offset));
                    continue;
                };
                let due = next_due(timing, last, &offset);
                if due <= now {
                    hub.core.set_last_run(id, name, now);
                    notices.extend(self.fire(hub, app, trigger, "due"));
                    soonest(next_due(timing, now, &offset));
                } else {
                    soonest(due);
                }
            }
        }
        (notices, next)
    }

    /// Runs one trigger: `skip`, then `when`, then its steps. Returns its notifications.
    fn fire(&self, hub: &Hub, app: &CompiledApp, trigger: &Value, why: &str) -> Vec<Notice> {
        let id = app.app.id.as_str();
        let name = trigger.get("name").and_then(Value::as_str).unwrap_or_default();
        let Ok((_, db)) = hub.apps.db(id) else { return Vec::new() };
        let settings = with_defaults(app, &db.settings().unwrap_or_default());
        if let Some(range) = trigger.get("skip").and_then(|v| time_range(v, &settings)) {
            let now = self.clock.now_ms();
            let minute = (now + self.clock.offset_ms(now)).rem_euclid(DAY_MS) / 60_000;
            if in_range(minute, range) {
                tracing::info!(app = id, trigger = name, "trigger held back (skip)");
                return Vec::new();
            }
        }
        if let Some(sql) = trigger.get("when").and_then(Value::as_str) {
            match db.query(sql, &settings_params(sql, None, &settings)) {
                Ok(rows) if truthy(rows.first().and_then(|r| r.values().next())) => {}
                Ok(_) => {
                    tracing::info!(app = id, trigger = name, "trigger held back (when)");
                    return Vec::new();
                }
                Err(e) => {
                    tracing::warn!(app = id, trigger = name, "trigger's `when` failed: {e:#}");
                    return Vec::new();
                }
            }
        }
        tracing::info!(app = id, trigger = name, why, "trigger fired");
        let mut notices = Vec::new();
        let mut queries: HashMap<String, Value> = HashMap::new();
        for step in trigger.get("steps").and_then(Value::as_array).into_iter().flatten() {
            if let Some(sql) = step.get("sql").and_then(Value::as_str) {
                let names = step.get("params").and_then(Value::as_array);
                match db.exec(sql, &settings_params(sql, names, &settings)) {
                    Ok(exec) => {
                        queries.clear();
                        if !exec.changed_tables.is_empty() {
                            hub.emit(Event::AppChanged { app: id.to_owned(), tables: exec.changed_tables }, Source::Api);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(app = id, trigger = name, "trigger's sql step failed: {e:#}");
                        return notices;
                    }
                }
            } else if let Some(notify) = step.get("notify") {
                if !crate::broker::allows(&hub.core, app, "notify") {
                    tracing::info!(app = id, trigger = name, "notification not shown: `notify` isn't allowed");
                    continue;
                }
                let title = self.text(hub, app, notify.get("title"), &settings, &mut queries);
                let body = self.text(hub, app, notify.get("body"), &settings, &mut queries);
                hub.core.grant_used(id, "notify");
                notices.push(Notice { app: id.to_owned(), title: if title.is_empty() { app.app.name.clone() } else { title }, body });
            } else if step.get("refresh").is_some() {
                let mut tables: Vec<String> = app.queries.iter().flat_map(|q| q.tables.clone()).collect();
                tables.sort();
                tables.dedup();
                hub.emit(Event::AppChanged { app: id.to_owned(), tables }, Source::Api);
            }
        }
        hub.core.grant_used(id, "schedule");
        notices
    }

    /// A notification's text: its value evaluated against the app's settings and queries.
    fn text(&self, hub: &Hub, app: &CompiledApp, value: Option<&Value>, settings: &Map<String, Value>, queries: &mut HashMap<String, Value>) -> String {
        let Some(value) = value else { return String::new() };
        let settings = Value::Object(settings.clone());
        let offset = |ms: i64| self.clock.offset_ms(ms);
        let fmt = Format { now_ms: self.clock.now_ms(), offset_ms: &offset };
        let mut read = |path: &str| -> Value {
            let segs: Vec<&str> = path.trim_start_matches('/').splitn(3, '/').collect();
            match segs.as_slice() {
                ["settings", rest @ ..] => eval::read_pointer(&settings, &format!("/{}", rest.join("/"))).cloned().unwrap_or(Value::Null),
                ["q", name, rest @ ..] => {
                    let rows = queries.entry((*name).to_string()).or_insert_with(|| self.query(hub, app, name, &settings));
                    eval::read_pointer(rows, &format!("/{}", rest.join("/"))).cloned().unwrap_or(Value::Null)
                }
                _ => Value::Null,
            }
        };
        eval::to_text(&eval::eval(value, &mut read, &fmt))
    }

    /// A named query's rows, with its params read from settings (triggers have no screen state).
    fn query(&self, hub: &Hub, app: &CompiledApp, name: &str, settings: &Value) -> Value {
        let Some(q) = app.queries.iter().find(|q| q.name == name) else { return Value::Array(Vec::new()) };
        let offset = |ms: i64| self.clock.offset_ms(ms);
        let fmt = Format { now_ms: self.clock.now_ms(), offset_ms: &offset };
        let mut read = |path: &str| match path.strip_prefix("/settings") {
            Some(rest) => eval::read_pointer(settings, rest).cloned().unwrap_or(Value::Null),
            None => Value::Null,
        };
        let params: Map<String, Value> = q.params.iter().map(|(k, v)| (k.clone(), eval::eval(v, &mut read, &fmt))).collect();
        match hub.apps.db(&app.app.id).and_then(|(_, db)| db.query(&q.sql, &params)) {
            Ok(rows) => Value::Array(rows.into_iter().map(Value::Object).collect()),
            Err(e) => {
                tracing::warn!(app = %app.app.id, query = name, "trigger text: query failed: {e:#}");
                Value::Array(Vec::new())
            }
        }
    }

    /// Runs one of an app's triggers now, whatever its timing (the harness: test a reminder without waiting).
    pub fn fire_now(&self, hub: &Hub, app: &str, trigger: &str) -> anyhow::Result<Vec<Notice>> {
        let installed = hub.apps.get(app).ok_or_else(|| anyhow::anyhow!("no app `{app}`"))?;
        let t = installed
            .app
            .triggers
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str) == Some(trigger))
            .ok_or_else(|| anyhow::anyhow!("`{app}` has no trigger `{trigger}`"))?;
        anyhow::ensure!(crate::broker::allows(&hub.core, &installed.app, "schedule"), "`{app}` isn't allowed to run in the background");
        Ok(self.fire(hub, &installed.app, t, "harness"))
    }
}

/// The loop: events, then what's due; then it sleeps until the next due trigger, an event, or the wake alarm.
async fn run(hub: Arc<Hub>) {
    loop {
        let h = hub.clone();
        let worked = tokio::task::spawn_blocking(move || {
            let s = &h.scheduler;
            let mut notices = Vec::new();
            loop {
                let Some(happened) = s.queue.lock().unwrap().pop_front() else { break };
                notices.extend(s.on_event(&h, happened));
            }
            let (due, next) = s.run_due(&h);
            notices.extend(due);
            // Alarms and reminders (crate::alarms) and timers (crate::timers) share the wake alarm.
            let now = s.clock.now_ms();
            let offset = |ms: i64| s.clock.offset_ms(ms);
            let (rang, alarm_next) = h.core.with(|db| crate::alarms::due(db, now, &offset)).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "alarms: couldn't check what's due");
                (Vec::new(), None)
            });
            let (ended, timer_next) = h.core.with(|db| crate::timers::due(db, now)).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "timers: couldn't check what's due");
                (Vec::new(), None)
            });
            // Routines (crate::routines): an agent's turn at a time of day.
            let (routines, routine_next) = h.core.with(|db| crate::routines::due(db, now, &offset)).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "routines: couldn't check what's due");
                (Vec::new(), None)
            });
            let next = [next, alarm_next, timer_next, routine_next].into_iter().flatten().min();
            (notices, rang, ended, routines, next)
        })
        .await;
        let (notices, rang, ended, routines, next) = worked.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "scheduler: a run panicked");
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), None)
        });
        for r in routines {
            tokio::spawn(crate::routines::run(hub.clone(), r));
        }
        for n in notices {
            notify(&hub, n).await;
        }
        for a in rang {
            crate::alarms::went_off(&hub, a).await;
        }
        for t in ended {
            crate::timers::went_off(&hub, t).await;
        }
        let s = &hub.scheduler;
        s.alarm.set(next);
        s.alarm.done();
        let now = s.clock.now_ms();
        match next {
            Some(at) => {
                let wait = Duration::from_millis(u64::try_from(at - now).unwrap_or(0).max(250));
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = s.wake.notified() => {}
                    () = s.alarm.fired.notified() => {}
                }
            }
            None => tokio::select! {
                () = s.wake.notified() => {}
                () = s.alarm.fired.notified() => {}
            },
        }
    }
}

/// Turns the display on (KEYCODE_WAKEUP), on the R1.
pub async fn wake_display() {
    if !cfg!(target_os = "android") {
        return;
    }
    // The R1's own screen, asked of the power service: Android's wake key is ignored while any display counts as on (an
    // app window's, T-149), and the R1 then stayed dark.
    if crate::tool::run("cmd", &["power", "wakeup"], crate::tool::DEADLINE).await.is_ok_and(|o| o.ok) {
        return;
    }
    if let Err(e) = crate::tool::run("cmd", &["input", "keyevent", "224"], crate::tool::DEADLINE).await {
        tracing::warn!(error = %e, "couldn't wake the display");
    }
}

/// A notification: the display wakes (on the R1), and the shell shows it.
pub async fn notify(hub: &Hub, n: Notice) {
    tracing::info!(app = %n.app, title = %n.title, "notify");
    wake_display().await;
    let (name, icon, accent) = hub
        .apps
        .get(&n.app)
        .map(|i| (i.app.app.name.clone(), i.app.app.icon.clone(), i.app.app.accent.clone()))
        .unwrap_or_else(|| (n.app.clone(), "bell".into(), None));
    // Kept until you've seen it (T-049): shown again when the R1 wakes, and in What's up.
    let id = hub.core.with(|db| crate::notices::add(db, &n.app, &n.title, &n.body, crate::core::now())).ok();
    hub.emit(Event::Notify { id, app: n.app, name, icon, accent, title: n.title, body: n.body }, Source::Api);
}

/// The app's settings, with each unset one at its default.
fn with_defaults(app: &CompiledApp, settings: &Map<String, Value>) -> Map<String, Value> {
    let mut out = settings.clone();
    for (k, def) in &app.settings {
        if !out.contains_key(k)
            && let Some(d) = def.get("default")
        {
            out.insert(k.clone(), d.clone());
        }
    }
    out
}

/// A setting a trigger binds to (`{ "p": "/settings/<key>" }`), or a literal.
fn resolve<'a>(v: &'a Value, settings: &'a Map<String, Value>) -> Option<&'a Value> {
    match v.get("p").and_then(Value::as_str).and_then(|p| p.strip_prefix("/settings/")) {
        Some(key) => settings.get(key),
        None if v.is_object() => None,
        None => Some(v),
    }
}

fn timing(trigger: &Value, settings: &Map<String, Value>) -> Option<Timing> {
    if let Some(every) = trigger.get("every") {
        let secs = resolve(every, settings)?.as_f64()?;
        return Some(Timing::Every(((secs * 1000.0) as i64).max(MIN_EVERY_MS)));
    }
    let at = resolve(trigger.get("at")?, settings)?.as_str()?;
    clock_minute(at).map(Timing::At)
}

/// "HH:MM" → minutes after midnight.
fn clock_minute(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.get(..2).unwrap_or(m).parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// `"21:30-08:00"` (or a time_range setting) → minutes after midnight.
fn time_range(v: &Value, settings: &Map<String, Value>) -> Option<(i64, i64)> {
    let (from, to) = resolve(v, settings)?.as_str()?.split_once('-')?;
    Some((clock_minute(from)?, clock_minute(to)?))
}

/// The range may wrap past midnight.
fn in_range(minute: i64, (from, to): (i64, i64)) -> bool {
    if from <= to { (from..to).contains(&minute) } else { minute >= from || minute < to }
}

/// The next time a timed trigger is due after `last` (ms). `offset` is the local UTC offset at an instant.
fn next_due(timing: Timing, last: i64, offset: &dyn Fn(i64) -> i64) -> i64 {
    match timing {
        Timing::Every(ms) => last + ms,
        Timing::At(minute) => {
            let local_last = last + offset(last);
            let mut local = local_last.div_euclid(DAY_MS) * DAY_MS + minute * 60_000;
            if local <= local_last {
                local += DAY_MS;
            }
            // The UTC instant whose local time that is (the offset of a nearby instant, then of the result).
            let guess = local - offset(local);
            local - offset(guess)
        }
    }
}

/// Trigger SQL params come from settings only (`names`: a step's declared params, else the statement's).
fn settings_params(sql: &str, names: Option<&Vec<Value>>, settings: &Map<String, Value>) -> Map<String, Value> {
    let names: Vec<String> = match names {
        Some(names) => names.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        None => enee_spec::sql::named_params(sql),
    };
    names.into_iter().map(|n| (n.clone(), settings.get(&n).cloned().unwrap_or(Value::Null))).collect()
}

/// SQLite truth: null, 0 and "" are false.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty() && s != "0",
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edt(_: i64) -> i64 {
        -4 * 3_600_000
    }

    #[test]
    fn periods_count_from_the_last_run() {
        assert_eq!(next_due(Timing::Every(3_600_000), 1_000, &edt), 3_601_000);
        let s = Map::from_iter([("gap".to_string(), json!(30))]);
        assert_eq!(timing(&json!({"every": {"p": "/settings/gap"}}), &s), Some(Timing::Every(MIN_EVERY_MS)), "at most once a minute");
        assert_eq!(timing(&json!({"every": 7200}), &s), Some(Timing::Every(7_200_000)));
    }

    #[test]
    fn times_of_day_are_local() {
        // 2026-10-09 12:00 UTC is 08:00 EDT: 09:00 is an hour later, 07:00 is tomorrow.
        let noon = 1_791_547_200_000;
        assert_eq!(next_due(Timing::At(9 * 60), noon, &edt), noon + 3_600_000);
        assert_eq!(next_due(Timing::At(7 * 60), noon, &edt), noon + 23 * 3_600_000);
        assert_eq!(clock_minute("09:30"), Some(570));
        assert_eq!(clock_minute("25:00"), None);
    }

    #[test]
    fn skip_ranges_wrap_midnight_and_guards_read_sqlite_truth() {
        assert!(in_range(23 * 60, (21 * 60 + 30, 8 * 60)));
        assert!(in_range(7 * 60, (21 * 60 + 30, 8 * 60)));
        assert!(!in_range(12 * 60, (21 * 60 + 30, 8 * 60)));
        assert!(!truthy(Some(&json!(0))) && truthy(Some(&json!(2))) && !truthy(Some(&json!(""))) && !truthy(None));
        let s = Map::from_iter([("goal".to_string(), json!(8))]);
        assert_eq!(settings_params("SELECT count(*) < :goal FROM t", None, &s)["goal"], json!(8));
    }
}
