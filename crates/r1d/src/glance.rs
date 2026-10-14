//! Widgets on the glance screen (ADR-0042): an app's `glance:` card, filled in from its data here and drawn by the
//! host on the screen the R1 wakes to, one at a time.
//!
//! - **Which, in order:** the apps listed in core.db (`glance_apps`). An app with a glance card joins the end the first
//!   time it's installed; `PUT /api/glance {apps}` (the agent's `set_glance`) changes the list.
//! - **Filled in here** ([`fill`]): the card's Values read the app's queries (run against its database), its state
//!   (kept or declared), its settings and its services' kept values; `each` makes a list's items from rows. Each field
//!   is then made the type the catalog wants ([`normalize`]: numbers, text, a dash for text there's no data for yet), and
//!   the card is checked as an agent's would be (crate::cards).
//! - **When:** an app's change marks it ([`Glance::dirty`]: its data, its services' values, an edit, its state, a
//!   permission), and the display coming on marks them all (times like "5 min ago", "today"). A task fills what's
//!   marked and, when the list changed, sends it (`glance {widgets}`). No timers: nothing runs while nothing changes.
//! - **Web data** comes from the app's services, which run in the background for listed apps as for `schedule`
//!   (crate::services): only while the display is on, at most every 15 minutes, and on wake when what's kept is older.
//! - **Not allowed yet:** a service the user hasn't allowed (`net.http`, asked when the app first opens) leaves the card
//!   without its data. The widget says so (`note`), and touching it opens the app, which asks.

use crate::hub::{Event, Hub, Source};
use enee_spec::compiled::{CompiledApp, glance_lists};
use enee_spec::eval::{self, Format};
use enee_spec::sql::Clock;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Changes come in bursts (a feed's items, a build installing): one pass for each burst.
const SETTLE: Duration = Duration::from_millis(250);
/// The most widgets listed.
pub const MAX_WIDGETS: usize = 12;

#[derive(Default)]
pub struct Glance {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
}

#[derive(Default)]
struct Inner {
    dirty: HashSet<String>,
    /// Every listed app is to be filled again (the display came on, the list changed, r1d started).
    all: bool,
    /// By app: its card, or why there's none.
    filled: HashMap<String, Filled>,
    /// The widgets as last sent.
    sent: String,
}

#[derive(Clone, Default)]
struct Filled {
    card: Option<Value>,
    note: Option<String>,
}

impl Glance {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts the task that fills marked widgets (once, at startup).
    pub fn start(hub: &Arc<Hub>) {
        hub.glance.lock().all = true;
        hub.glance.wake.notify_one();
        tokio::spawn(run(hub.clone()));
    }

    /// The app's data, services, state, permissions or spec changed: fill its card again.
    pub fn dirty(&self, app: &str) {
        self.lock().dirty.insert(app.to_owned());
        self.wake.notify_one();
    }

    /// Fill every card again (the display came on: relative times and "today" moved on).
    pub fn all(&self) {
        self.lock().all = true;
        self.wake.notify_one();
    }

    /// Whether the app's card is on the glance screen (its services then run in the background).
    pub fn listed(&self, hub: &Hub, app: &str) -> bool {
        hub.core.glance_apps().iter().any(|a| a == app)
    }

    /// An app was installed (or edited): one with a glance card joins the list the first time it's seen.
    pub fn installed(&self, hub: &Hub, app: &str) {
        let has = hub.apps.get(app).is_some_and(|i| i.app.glance.is_some());
        let mut known = hub.core.glance_known();
        if has && !known.iter().any(|k| k == app) {
            known.push(app.to_owned());
            let _ = hub.core.set_glance_known(&known);
            let mut apps = hub.core.glance_apps();
            if apps.len() < MAX_WIDGETS && !apps.iter().any(|a| a == app) {
                apps.push(app.to_owned());
                let _ = hub.core.set_glance_apps(&apps);
                hub.services.background_changed();
            }
        }
        self.dirty(app);
    }

    /// An app was removed: off the list (a restore from the trash lists it again).
    pub fn removed(&self, hub: &Hub, app: &str) {
        let apps: Vec<String> = hub.core.glance_apps().into_iter().filter(|a| a != app).collect();
        let known: Vec<String> = hub.core.glance_known().into_iter().filter(|a| a != app).collect();
        let _ = hub.core.set_glance_apps(&apps);
        let _ = hub.core.set_glance_known(&known);
        self.lock().filled.remove(app);
        self.all();
    }

    /// Lists these apps' cards, in this order (each must be installed and have a glance card).
    pub fn set_apps(&self, hub: &Hub, apps: &[String]) -> anyhow::Result<Vec<String>> {
        let out = checked_apps(hub, apps)?;
        hub.core.set_glance_apps(&out)?;
        hub.services.background_changed();
        self.all();
        Ok(out)
    }

    /// The listed widgets, in order: `{app, name, icon, accent, card?, note?}`. Fills any not filled yet.
    pub fn widgets(&self, hub: &Hub) -> Vec<Value> {
        let listed = listed_apps(hub);
        let missing: Vec<String> = {
            let inner = self.lock();
            listed.iter().filter(|id| !inner.filled.contains_key(*id)).cloned().collect()
        };
        for id in missing {
            let f = fill_app(hub, &id);
            self.lock().filled.insert(id, f);
        }
        let inner = self.lock();
        listed.iter().filter_map(|id| widget(hub, id, inner.filled.get(id)?)).collect()
    }

    /// Fills what's marked, and sends the list if it changed.
    fn pass(&self, hub: &Hub) {
        let (dirty, all) = {
            let mut inner = self.lock();
            (std::mem::take(&mut inner.dirty), std::mem::take(&mut inner.all))
        };
        let listed = listed_apps(hub);
        for id in listed.iter().filter(|id| all || dirty.contains(*id)) {
            let f = fill_app(hub, id);
            self.lock().filled.insert(id.clone(), f);
        }
        self.lock().filled.retain(|id, _| listed.contains(id));
        let widgets = self.widgets(hub);
        let json = serde_json::to_string(&widgets).unwrap_or_default();
        {
            let mut inner = self.lock();
            if inner.sent == json {
                return;
            }
            inner.sent = json;
        }
        hub.emit(Event::Glance { widgets }, Source::Api);
    }
}

async fn run(hub: Arc<Hub>) {
    loop {
        hub.glance.wake.notified().await;
        tokio::time::sleep(SETTLE).await;
        let h = hub.clone();
        let _ = tokio::task::spawn_blocking(move || h.glance.pass(&h)).await;
    }
}

/// The listed apps that are installed and have a glance card, in order.
/// Apps whose cards the glance screen would show, checked: each has a card, no repeats, not too many.
pub fn checked_apps(hub: &Hub, apps: &[String]) -> anyhow::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for id in apps {
        let installed = hub.apps.get(id).ok_or_else(|| anyhow::anyhow!("no app `{id}`"))?;
        anyhow::ensure!(installed.app.glance.is_some(), "`{id}` has no glance card: an edit can give it one (`glance:`)");
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    anyhow::ensure!(out.len() <= MAX_WIDGETS, "the glance screen takes {MAX_WIDGETS} widgets at most");
    Ok(out)
}

fn listed_apps(hub: &Hub) -> Vec<String> {
    hub.core.glance_apps().into_iter().filter(|id| hub.apps.get(id).is_some_and(|i| i.app.glance.is_some())).collect()
}

fn widget(hub: &Hub, id: &str, f: &Filled) -> Option<Value> {
    let app = &hub.apps.get(id)?.app;
    let mut w = json!({ "app": id, "name": app.app.name, "icon": app.app.icon });
    if let Some(accent) = &app.app.accent {
        w["accent"] = json!(accent);
    }
    if let Some(card) = &f.card {
        w["card"] = card.clone();
    }
    if let Some(note) = &f.note {
        w["note"] = json!(note);
    }
    Some(w)
}

/// The app's card, filled in from its data and checked; or, with none, what to say instead.
fn fill_app(hub: &Hub, id: &str) -> Filled {
    let Some(installed) = hub.apps.get(id) else { return Filled::default() };
    let app = &installed.app;
    let Some(template) = &app.glance else { return Filled::default() };
    let services = hub.services.values(id);
    let mut state = app.state.clone();
    state.extend(hub.apps.kept_state(id));
    let state = Value::Object(state);
    let settings = Value::Object(hub.apps.settings(id).unwrap_or_default());
    let meta = serde_json::to_value(&app.app).unwrap_or(Value::Null);
    let clock = Clock::system();
    let offset = |ms: i64| clock.offset_ms(ms);
    let fmt = Format { now_ms: clock.now_ms(), offset_ms: &offset };
    let mut queries: HashMap<String, Value> = HashMap::new();
    let mut read = |path: &str| -> Value {
        let segs: Vec<&str> = path.trim_start_matches('/').splitn(3, '/').collect();
        let at = |v: &Value, rest: &[&str]| eval::read_pointer(v, &format!("/{}", rest.join("/"))).cloned().unwrap_or(Value::Null);
        match segs.as_slice() {
            ["settings", rest @ ..] => at(&settings, rest),
            ["state", rest @ ..] => at(&state, rest),
            ["app", rest @ ..] => at(&meta, rest),
            ["services", name, rest @ ..] => services.get(&format!("/services/{name}")).map(|v| at(v, rest)).unwrap_or(Value::Null),
            ["q", name, rest @ ..] => {
                let rows = queries.entry((*name).to_owned()).or_insert_with(|| query(hub, app, name, &settings, &state, &fmt));
                at(rows, rest)
            }
            _ => Value::Null,
        }
    };
    let card = normalize(&fill(template, &mut read, &fmt), false);
    match crate::cards::check(&card) {
        Ok(card) => Filled { card: Some(card), note: None },
        Err(why) => {
            tracing::debug!(app = id, "glance card not shown: {why}");
            let waiting = app.services.iter().any(|s| s.options.get("url").is_some()) && !crate::broker::allows(&hub.core, app, "net.http");
            let note = if waiting { "Open it once to allow its data" } else { "Nothing to show yet" };
            Filled { card: None, note: Some(note.into()) }
        }
    }
}

/// A named query's rows, its params read from the app's settings and state (a glance card has no page).
fn query(hub: &Hub, app: &CompiledApp, name: &str, settings: &Value, state: &Value, fmt: &Format) -> Value {
    let Some(q) = app.queries.iter().find(|q| q.name == name) else { return json!([]) };
    let mut read = |path: &str| {
        if let Some(rest) = path.strip_prefix("/settings") {
            eval::read_pointer(settings, rest).cloned().unwrap_or(Value::Null)
        } else if let Some(rest) = path.strip_prefix("/state") {
            eval::read_pointer(state, rest).cloned().unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    };
    let params: Map<String, Value> = q.params.iter().map(|(k, v)| (k.clone(), eval::eval(v, &mut read, fmt))).collect();
    match hub.apps.db(&app.app.id).and_then(|(_, db)| db.query(&q.sql, &params)) {
        Ok(rows) => Value::Array(rows.into_iter().map(Value::Object).collect()),
        Err(e) => {
            tracing::warn!(app = %app.app.id, query = name, "glance card: query failed: {e:#}");
            json!([])
        }
    }
}

/// The card's Values evaluated: `each` lists become one item per row.
pub fn fill(template: &Value, read: &mut dyn FnMut(&str) -> Value, fmt: &Format) -> Value {
    let Some(m) = template.as_object() else { return eval::eval(template, read, fmt) };
    let mut out = Map::new();
    for (k, v) in m {
        let filled = match v.as_object().filter(|e| e.contains_key("each") && e.contains_key("item")) {
            Some(e) => {
                let rows = eval::eval(&e["each"], read, fmt);
                let take = e.get("take").and_then(Value::as_u64).map_or(usize::MAX, |n| n as usize);
                let items = rows.as_array().into_iter().flatten().take(take);
                Value::Array(items.map(|row| eval::eval_row(&e["item"], row, read, fmt)).collect())
            }
            None => eval::eval(v, read, fmt),
        };
        out.insert(k.clone(), filled);
    }
    Value::Object(out)
}

/// Fields the catalog wants as numbers, by list ("" for the card's own fields).
fn numeric(kind: &str, list: &str) -> &'static [&'static str] {
    match (kind, list) {
        ("weather", "") => &["temp", "high", "low", "feels"],
        ("weather", "hours") => &["temp"],
        ("weather", "days") => &["high", "low"],
        ("bars", "items") | ("line", "points") => &["value"],
        _ => &[],
    }
}

/// Numbers an item can't be drawn without: an item missing one is left out.
fn needs_number(kind: &str, list: &str) -> &'static [&'static str] {
    match (kind, list) {
        ("weather", "") => &[],
        _ => numeric(kind, list),
    }
}

/// Text the catalog needs: a dash when there's no data for it yet ("0 glasses" is the builder's job, with `default`).
fn needs_text(kind: &str, list: &str) -> &'static [&'static str] {
    match (kind, list) {
        ("headlines", "items") | ("steps", "items") => &["title"],
        ("link", "") => &["title"],
        ("weather", "") => &["place", "condition"],
        ("weather", "hours") => &["time"],
        ("weather", "days") => &["day"],
        ("stats", "items") => &["label", "value"],
        ("bars", "items") | ("line", "points") => &["label"],
        _ => &[],
    }
}

/// Fields holding a link or a picture.
fn links(kind: &str, list: &str) -> &'static [&'static str] {
    match (kind, list) {
        ("headlines", "items") | ("link", "") => &["url", "image"],
        ("image", "") => &["url", "link"],
        _ => &[],
    }
}

/// Each list's most items, as the catalog takes them.
fn most(kind: &str, list: &str) -> usize {
    match (kind, list) {
        ("headlines", "items") | ("steps", "items") | ("weather", "hours") => 8,
        ("stats", "items") => 4,
        ("bars", "items") | ("table", "rows") => 10,
        ("line", "points") => 40,
        ("weather", "days") => 7,
        _ => 50,
    }
}

/// The filled card's fields as the catalog wants them. `stand_in`: a build's check, where any link will do and lists
/// aren't cut (so a list that would be too long is a problem).
pub fn normalize(card: &Value, stand_in: bool) -> Value {
    let Some(m) = card.as_object() else { return card.clone() };
    let kind = m.get("type").and_then(Value::as_str).unwrap_or_default().to_owned();
    let lists = glance_lists(&kind).unwrap_or(&[]);
    let mut out = fields(&kind, "", m, stand_in, lists);
    if kind == "table" {
        if let Some(cols) = m.get("columns").and_then(Value::as_array) {
            out.insert("columns".into(), Value::Array(cols.iter().map(|c| json!(eval::to_text(c))).collect()));
        }
        if let Some(rows) = m.get("rows").and_then(Value::as_array) {
            let rows = rows.iter().take(if stand_in { usize::MAX } else { most(&kind, "rows") });
            let cells = |r: &Value| Value::Array(r.as_array().into_iter().flatten().map(|c| json!(eval::to_text(c))).collect());
            out.insert("rows".into(), Value::Array(rows.map(cells).collect()));
        }
        return Value::Object(out);
    }
    for list in lists {
        let Some(items) = m.get(*list).and_then(Value::as_array) else { continue };
        let need = needs_number(&kind, list);
        let kept: Vec<Value> = items
            .iter()
            .filter_map(Value::as_object)
            .map(|i| fields(&kind, list, i, stand_in, &[]))
            .filter(|i| need.iter().all(|k| i.contains_key(*k)))
            .take(if stand_in { usize::MAX } else { most(&kind, list) })
            .map(Value::Object)
            .collect();
        out.insert((*list).into(), Value::Array(kept));
    }
    Value::Object(out)
}

/// One object's fields (the card's own, or a list item's): numbers as numbers, the rest as text, empty ones left out.
fn fields(kind: &str, list: &str, m: &Map<String, Value>, stand_in: bool, skip: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in m {
        if skip.contains(&k.as_str()) || (kind == "table" && list.is_empty() && matches!(k.as_str(), "columns" | "rows")) {
            continue;
        }
        if numeric(kind, list).contains(&k.as_str()) {
            let n = match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.trim().parse::<f64>().ok(),
                _ => None,
            };
            if let Some(n) = n.filter(|n| n.is_finite()) {
                out.insert(k.clone(), json!(n));
            }
            continue;
        }
        let text = match v {
            Value::Array(_) | Value::Object(_) => continue,
            other => eval::to_text(other).trim().to_owned(),
        };
        if links(kind, list).contains(&k.as_str()) && !(text.starts_with("https://") || text.starts_with("http://")) {
            if stand_in && !text.is_empty() {
                out.insert(k.clone(), json!("https://example.com/"));
            }
            continue;
        }
        if k == "trend" && !matches!(text.as_str(), "up" | "down" | "flat") {
            continue;
        }
        if !text.is_empty() {
            out.insert(k.clone(), json!(text));
        }
    }
    for k in needs_text(kind, list) {
        out.entry(*k).or_insert_with(|| json!("—"));
    }
    out
}

/// What's wrong with the card's shape, found by filling it with stand-ins (a build's check, before any data):
/// `E_GLANCE …` problems the builder repairs.
pub fn shape_problems(app: &CompiledApp) -> Vec<String> {
    let Some(template) = &app.glance else { return Vec::new() };
    let mut card = template.clone();
    // Each list's rows: as many as it takes (3 if it doesn't say), each with the fields its item reads.
    if let Some(m) = card.as_object_mut() {
        for v in m.values_mut() {
            let Some(e) = v.as_object_mut().filter(|e| e.contains_key("each") && e.contains_key("item")) else { continue };
            let n = e.get("take").and_then(Value::as_u64).unwrap_or(3) as usize;
            let mut row = Map::new();
            row_fields(&e["item"], &mut row);
            e.insert("each".into(), json!({ "$x": Value::Array(vec![Value::Object(row); n]) }));
        }
    }
    let clock = Clock::system();
    let offset = |ms: i64| clock.offset_ms(ms);
    let fmt = Format { now_ms: clock.now_ms(), offset_ms: &offset };
    let mut read = |_: &str| json!("1");
    let filled = normalize(&fill(&card, &mut read, &fmt), true);
    match crate::cards::check(&filled) {
        Ok(_) => Vec::new(),
        Err(why) => vec![format!("E_GLANCE glance: {why}")],
    }
}

/// The row fields an item template reads (`{ i: "field" }`), each set to a stand-in.
fn row_fields(v: &Value, row: &mut Map<String, Value>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(f)) = m.get("i") {
                row.insert(f.clone(), json!("1"));
            }
            m.values().for_each(|x| row_fields(x, row));
        }
        Value::Array(items) => items.iter().for_each(|x| row_fields(x, row)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt_now<T>(f: impl FnOnce(&Format) -> T) -> T {
        let offset = |_: i64| 0;
        f(&Format { now_ms: 1_791_600_000_000, offset_ms: &offset })
    }

    fn compiled(src: &str) -> CompiledApp {
        crate::apps::compile(src).expect("compiles")
    }

    const WATER: &str = r#"enee: 1
app: { id: water, name: Water, archetype: widget }
data:
  drinks: { glasses: int! }
queries:
  today: SELECT coalesce(sum(glasses), 0) AS glasses FROM drinks
  week: SELECT 'mon' AS day, sum(glasses) AS glasses FROM drinks
state:
  goal: 8
views:
  main:
    - stat: { label: Today, value: "{{today.0.glasses}}" }
glance:
  type: bars
  title: "{{today.0.glasses | default:0}} of {{state.goal}} glasses"
  items: { each: week, take: 7, label: "{{row.day}}", value: "{{row.glasses}}" }
"#;

    #[test]
    fn the_catalogs_agree() {
        assert_eq!(enee_spec::compiled::GLANCE_TYPES, crate::cards::TYPES);
    }

    #[test]
    fn a_card_fills_from_rows() {
        let app = compiled(WATER);
        let rows = json!([{ "day": "mon", "glasses": 3 }, { "day": "tue", "glasses": "5" }, { "day": "wed", "glasses": null }]);
        let mut read = |p: &str| match p {
            "/q/week" => rows.clone(),
            "/q/today/0/glasses" => json!(5),
            "/state/goal" => json!(8),
            _ => Value::Null,
        };
        let card = fmt_now(|fmt| normalize(&fill(app.glance.as_ref().unwrap(), &mut read, fmt), false));
        let card = crate::cards::check(&card).expect("a good card");
        assert_eq!(card["title"], "5 of 8 glasses");
        // Wednesday has no number: it's left out; Tuesday's text became a number.
        assert_eq!(card["items"], json!([{ "label": "mon", "value": 3.0 }, { "label": "tue", "value": 5.0 }]));
    }

    #[test]
    fn missing_text_is_a_dash_and_bad_links_go() {
        let card = json!({ "type": "headlines", "items": [{ "title": null, "url": "nope", "source": "" }] });
        assert_eq!(normalize(&card, false), json!({ "type": "headlines", "items": [{ "title": "—" }] }));
        let stats = json!({ "type": "stats", "items": [{ "label": "a", "value": 1 }, {}, {}, {}, {}, {}] });
        assert_eq!(normalize(&stats, false)["items"].as_array().unwrap().len(), 4, "cut to the catalog's most");
    }

    #[test]
    fn a_builds_check_finds_bad_shapes() {
        assert_eq!(shape_problems(&compiled(WATER)), Vec::<String>::new());
        let with = |glance: &str| compiled(&format!("{}glance:\n{glance}", WATER.split("glance:").next().unwrap()));
        // One step: the catalog's steps are 2–8 (a long list is only cut).
        let p = shape_problems(&with("  type: steps\n  items: { each: week, take: 1, title: \"{{row.day}}\" }\n"));
        assert!(p.len() == 1 && p[0].starts_with("E_GLANCE"), "{p:?}");
        assert_eq!(shape_problems(&with("  type: stats\n  items: { each: week, take: 6, label: \"{{row.day}}\", value: \"{{row.glasses}}\" }\n")), Vec::<String>::new());
        // A link card needs its link.
        let p = shape_problems(&with("  type: link\n  title: Water\n"));
        assert!(p.len() == 1 && p[0].contains("url"), "{p:?}");
        // Rows' links stand in as links.
        let p = shape_problems(&with("  type: headlines\n  items: { each: week, title: \"{{row.day}}\", url: \"{{row.day}}\" }\n"));
        assert_eq!(p, Vec::<String>::new());
    }
}
