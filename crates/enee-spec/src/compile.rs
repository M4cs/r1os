//! The compile pass: EneeSpec YAML → [`CompiledApp`] + diagnostics (docs/specs/enee-spec.md §8).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::blocks::{self, ActionSig, Ctx};
use crate::compiled::{
    AppMeta, CompiledApp, GLANCE_TYPES, Logic, Permission, Query, Service, Table, View, glance_fields, glance_lists,
};
use crate::data::{self, Ty};
use crate::diag::{Diagnostic, Diags, Severity};
use crate::lines::Lines;
use crate::sql::{self, Checker, Info};
use crate::util::{child, duration_ms, index, is_clock_time, is_ident, kind, list, suggest};
use crate::value::{self, Names, Scope};

pub const MAX_SOURCE_BYTES: usize = 256 * 1024;

const SECTIONS: [&str; 15] = [
    "enee",
    "app",
    "permissions",
    "settings",
    "data",
    "queries",
    "state",
    "services",
    "actions",
    "views",
    "triggers",
    "logic",
    "glance",
    "components",
    "checks",
];
const ARCHETYPES: [&str; 12] = [
    "tracker",
    "habit",
    "monitor",
    "notes",
    "list",
    "feed",
    "timer",
    "converter",
    "widget",
    "launcher",
    "form",
    "custom",
];
const APP_KEYS: [&str; 9] = ["id", "name", "icon", "archetype", "design", "accent", "theme", "description", "window"];
const PERMISSIONS: &[&str] = &[
    "app.storage",
    "app.ui",
    "clipboard.write",
    "system.cpu",
    "system.gpu",
    "system.memory",
    "system.disk",
    "system.network",
    "system.battery",
    "notify",
    "schedule",
    "schedule.background",
    "input.hotkey",
    "open.url",
    "apps.launch",
    "media.control",
    "audio.play",
    "net.http",
    "clipboard.read",
    "fs.read",
    "fs.watch",
    "fs.write",
    "net.lan",
    "process.list",
    "window.active",
    "screen.capture",
    "ai.claude",
    "ai.judge",
    "service.call",
    "process.control",
    "input.synthesize",
    "input.capture",
    "net.any",
    "service.native",
    "shell.exec",
];
/// Stream producers usable under `services:` and the permission each needs.
const SERVICES: &[(&str, &str)] = &[
    ("system.cpu", "system.cpu"),
    ("system.memory", "system.memory"),
    ("system.disk", "system.disk"),
    ("system.network", "system.network"),
    ("system.gpu", "system.gpu"),
    ("system.battery", "system.battery"),
    ("system.temps", "system.cpu"),
    ("process.list", "process.list"),
    ("window.active", "window.active"),
    ("media.now_playing", "media.control"),
    ("audio.volume", "media.control"),
    ("clipboard", "clipboard.read"),
    ("http.fetch", "net.http"),
    ("http.rss", "net.http"),
    ("http.json", "net.http"),
    ("fs.watch", "fs.watch"),
];
const STEPS: [&str; 15] = [
    "sql", "toast", "set", "navigate", "notify", "refresh", "open_url", "copy", "launch", "play", "pause", "resume", "stop", "toggle", "run",
];
/// The most a logic script may be (ADR-0041).
const LOGIC_SCRIPT_MAX: usize = 96 * 1024;
/// What a logic script can't use, and what to use instead. Bare `fetch(` too (`r1.fetch(` is the way).
const LOGIC_FORBIDDEN: [(&str, &str); 8] = [
    ("XMLHttpRequest", "can't reach the network: use `r1.fetch(url)`, which goes through the app's `net.http`"),
    ("WebSocket", "can't reach the network: use `r1.fetch(url)`"),
    ("importScripts", "can't load code: the logic is one script"),
    ("eval(", "isn't allowed: write the code itself"),
    ("new Function", "isn't allowed: write the code itself"),
    ("document.", "doesn't exist: the script has no page; publish values with `r1.set` and bind them in views"),
    ("window.", "doesn't exist: the script has no page, only `r1`"),
    ("localStorage", "doesn't exist: keep values in state with `r1.save`"),
];
/// The player's steps (`audio.play`, ADR-0040): `play` starts a stream, the rest act on what this app is playing.
const PLAYER_STEPS: [&str; 4] = ["pause", "resume", "stop", "toggle"];
const LATER_STEPS: [&str; 4] = ["call", "ai", "judge", "if"];
const TRIGGER_STEPS: [&str; 4] = ["sql", "notify", "refresh", "copy"];
const HELPERS: &str = "is_today, is_this_week, is_this_month, local_hour, local_date, local_weekday, days_ago, hours_ago, now_utc, streak";

#[derive(Clone, Debug)]
pub struct Options {
    /// Prepare SQL against a scratch database (needs the `sqlite` feature; skipped otherwise).
    pub check_sql: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { check_sql: true }
    }
}

#[derive(Debug, Serialize)]
pub struct Output {
    /// No errors (warnings allowed).
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Present whenever the YAML parsed and `app` was usable, even with errors (progressive preview).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<CompiledApp>,
}

pub fn compile(src: &str, opts: &Options) -> Output {
    let lines = Lines::new(src);
    let mut diags = Diags::default();
    let app = Pass::run(src, opts, &lines, &mut diags);
    let mut items = diags.items;
    for d in &mut items {
        if d.line.is_none() {
            d.line = lines.get(&d.path);
        }
    }
    let ok = !items.iter().any(|d| d.severity == Severity::Error);
    Output { ok, diagnostics: items, app }
}

/// In a flow map, `{ description: Read, write, repeat }` parses as `description: Read` plus the keys `write` and
/// `repeat` with null values. A null-valued key that reads like prose (a space, closing punctuation, or characters no
/// EneeSpec key has) right after a string value is such a split tail: glue it back.
fn rejoin_comma_splits(v: &mut Value, path: &str, diags: &mut Diags) {
    match v {
        Value::Object(map) => {
            let mut out = Map::new();
            let mut last: Option<String> = None;
            for (k, mut val) in std::mem::take(map) {
                let prose = k.contains(' ')
                    || k.ends_with(['.', '!', '?'])
                    || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
                let tail = val.is_null() && prose;
                if tail && let Some(Value::String(prev)) = last.as_ref().and_then(|l| out.get_mut(l)) {
                    prev.push_str(", ");
                    prev.push_str(&k);
                    let at = child(path, last.as_deref().unwrap_or_default());
                    diags
                        .warn("W_AUTOFIX", &at, "a comma split this value; it was rejoined")
                        .hint("Quote values that contain commas inside `{ … }` maps");
                    continue;
                }
                rejoin_comma_splits(&mut val, &child(path, &k), diags);
                last = Some(k.clone());
                out.insert(k, val);
            }
            *map = out;
        }
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                rejoin_comma_splits(item, &index(path, i), diags);
            }
        }
        _ => {}
    }
}

struct Pass<'a> {
    lines: &'a Lines,
    diags: &'a mut Diags,
    names: Names,
    checker: Option<Checker>,
    tables: Vec<Table>,
    settings: Map<String, Value>,
    state: Map<String, Value>,
    persist: Vec<String>,
    permissions: Vec<Permission>,
    actions: BTreeMap<String, ActionSig>,
    views: Vec<String>,
    /// (permission, YAML path of the use).
    needs: Vec<(&'static str, String)>,
    /// The logic's actions (`run: name` steps), ADR-0041.
    logic_actions: Vec<String>,
}

impl<'a> Pass<'a> {
    fn run(src: &str, opts: &Options, lines: &'a Lines, diags: &'a mut Diags) -> Option<CompiledApp> {
        if src.len() > MAX_SOURCE_BYTES {
            diags.error(
                "E_TOO_LARGE",
                "",
                format!("spec is {} KB; the limit is {} KB", src.len() / 1024, MAX_SOURCE_BYTES / 1024),
            );
            return None;
        }
        // YAML 1.2 core schema: only `true`/`false` are booleans, so `y: n` and a trigger's `on:` stay strings.
        let mut yaml = serde_saphyr::Options::default();
        yaml.strict_booleans = true;
        yaml.emit_comments = false;
        let mut root: Value = match serde_saphyr::from_str_with_options(src, yaml) {
            Ok(v) => v,
            Err(e) => {
                let msg = e.to_string();
                let first = msg.lines().next().unwrap_or("invalid YAML").to_string();
                let line = e.location().map(|l| l.line() as usize);
                let d = diags.error("E_YAML", "", first);
                d.line = line;
                d.hint(if src.contains(": {{") || src.contains("- {{") {
                    "Quote every binding: `value: \"{{today.total}}\"` (an unquoted `{` starts a YAML map)"
                } else {
                    "Quote strings that contain `: `, ` #`, `{{` or start with `+`, `-`, `*`, `&`, `!`, `%` or `@`"
                });
                return None;
            }
        };
        rejoin_comma_splits(&mut root, "", diags);
        let Some(root) = root.as_object() else {
            diags.error("E_SHAPE", "", format!("the spec must be a YAML map of sections, got {}", kind(&root)));
            return None;
        };
        let mut pass = Pass {
            lines,
            diags,
            names: Names::default(),
            checker: None,
            tables: Vec::new(),
            settings: Map::new(),
            state: Map::new(),
            persist: Vec::new(),
            permissions: Vec::new(),
            actions: BTreeMap::new(),
            views: Vec::new(),
            needs: Vec::new(),
            logic_actions: Vec::new(),
        };
        pass.sections(root);
        let app = pass.app(root.get("app"));
        pass.permissions(root.get("permissions"));

        let settings = data::compile_settings(root.get("settings"), pass.diags);
        let tables = data::compile_tables(root.get("data"), pass.diags);
        let mut ddl = tables.ddl;
        ddl.extend(settings.ddl);
        pass.tables = tables.tables;
        pass.names.settings = settings.defs.keys().cloned().collect();
        pass.settings = settings.defs;
        if opts.check_sql {
            match Checker::new(&ddl) {
                Ok(c) => pass.checker = Some(c),
                Err(e) => {
                    pass.diags.error("E_INTERNAL", "data", format!("the generated schema failed to load: {e}"));
                }
            }
        }

        pass.state(root.get("state"));
        let services = pass.services(root.get("services"));
        let logic = pass.logic(root.get("logic"));
        pass.views = match root.get("views") {
            Some(Value::Object(v)) => v.keys().cloned().collect(),
            Some(Value::Array(_)) => vec!["main".into()],
            _ => Vec::new(),
        };
        let queries = pass.queries(root.get("queries"));
        let actions = pass.actions(root.get("actions"));
        let (views, sourcemap) = pass.compile_views(root.get("views"));
        let triggers = pass.triggers(root.get("triggers"));
        let glance = pass.glance(root.get("glance"));
        let checks = pass.checks(root.get("checks"));
        if root.contains_key("components") {
            pass.diags.error("E_UNSUPPORTED", "components", "`components` escape hatches are not supported yet (v0)".to_owned());
        }
        pass.coverage(!services.is_empty());

        Some(CompiledApp {
            enee: 1,
            app: app?,
            permissions: pass.permissions,
            settings: pass.settings,
            tables: pass.tables,
            ddl,
            queries,
            state: pass.state,
            persist: pass.persist,
            services,
            actions,
            views,
            logic,
            glance,
            triggers,
            checks,
            sourcemap,
        })
    }

    /// The app's glance card (ADR-0042): one card of r1d's catalog (ADR-0039) whose fields bind the app's queries,
    /// state, settings and services. `{ each: <query | services.x.list | state.key>, take?, …fields }` builds a list
    /// field from rows (`{{row.x}}`; a table's rows from `cells: [..]`). r1d fills it in and checks it again.
    fn glance(&mut self, glance: Option<&Value>) -> Option<Value> {
        let v = glance?;
        let Some(card) = v.as_object() else {
            let d = self.diags.error("E_SHAPE", "glance", "`glance` is one card: `{ type: stats, items: [..] }`");
            d.hint("e.g. `glance: { type: stats, title: Water, items: [ { label: today, value: \"{{today.0.glasses | default:0}}\" } ] }`");
            return None;
        };
        let kind = card.get("type").and_then(Value::as_str).unwrap_or_default();
        let Some(lists) = glance_lists(kind) else {
            let msg = if kind.is_empty() { "the glance card needs a `type`".to_owned() } else { format!("there's no `{kind}` card") };
            self.diags.error("E_GLANCE_TYPE", "glance.type", msg).hint(format!("Card types: {}", GLANCE_TYPES.join(", ")));
            return None;
        };
        let mut out = Map::new();
        out.insert("type".into(), json!(kind));
        self.glance_keys(kind, "", card.keys().filter(|k| *k != "type"), "glance");
        for (k, field) in card {
            if k == "type" {
                continue;
            }
            let path = child("glance", k);
            // A list written out: its items' fields.
            if let (Some(items), true) = (field.as_array(), kind != "table") {
                for (i, item) in items.iter().enumerate() {
                    if let Some(m) = item.as_object() {
                        self.glance_keys(kind, k, m.keys(), &index(&path, i));
                    }
                }
            }
            let compiled = match field.as_object().filter(|m| m.contains_key("each")) {
                Some(m) if lists.contains(&k.as_str()) => match self.glance_each(m, kind, k, &path) {
                    Some(c) => c,
                    None => continue,
                },
                Some(_) => {
                    let d = self.diags.error("E_GLANCE_EACH", &path, format!("`each` builds a list, and `{k}` isn't one"));
                    d.hint(if lists.is_empty() {
                        format!("A {kind} card has no lists")
                    } else {
                        format!("A {kind} card's lists: {}", lists.join(", "))
                    });
                    continue;
                }
                None => value::compile(field, Scope::QueryParam, &self.names, &path, self.diags),
            };
            out.insert(k.clone(), compiled);
        }
        let out = Value::Object(out);
        // The card shows while the app is closed: its script isn't running, and the player isn't the app's.
        if let Some(root) = page_only(&out) {
            let d = self.diags.error(
                "E_GLANCE_BIND",
                "glance",
                format!("a glance card can't show `{root}.*`: it's on screen while the app is closed"),
            );
            d.hint("Bind the app's queries, state, settings or services");
        }
        Some(out)
    }

    /// Fields a glance card (or its list's items) doesn't have: r1d would drop them.
    fn glance_keys<'k>(&mut self, kind: &str, list: &str, keys: impl Iterator<Item = &'k String>, path: &str) {
        let known = glance_fields(kind, list);
        for k in keys {
            if !known.contains(&k.as_str()) {
                let what = if list.is_empty() { format!("a {kind} card") } else { format!("a {kind} card's {list}") };
                let d = self.diags.warn("W_GLANCE_KEY", child(path, k), format!("{what} has no `{k}`: it won't show"));
                d.hint(format!("Fields: {}", known.join(", ")));
            }
        }
    }

    /// A glance list field built from rows: `{ each, take?, …item }` → `{ each: <Value>, take?, item }`.
    fn glance_each(&mut self, m: &Map<String, Value>, kind: &str, field: &str, path: &str) -> Option<Value> {
        let table = kind == "table";
        if !table {
            self.glance_keys(kind, field, m.keys().filter(|k| !matches!(k.as_str(), "each" | "take")), path);
        }
        let at = child(path, "each");
        let name = m["each"].as_str().unwrap_or_default().trim().trim_start_matches("{{").trim_end_matches("}}").trim().to_owned();
        let (source, cols) = if let Some(field) = name.strip_prefix("services.") {
            let service = field.split('.').next().unwrap_or_default();
            if !self.names.services.contains(service) {
                let d = self.diags.error("E_BIND_UNKNOWN", &at, format!("unknown service `{service}`"));
                d.hint(format!("Declared services: {}", list(self.names.services.iter().map(String::as_str))));
                return None;
            }
            (format!("/services/{}", field.replace('.', "/")), Vec::new())
        } else if let Some(key) = name.strip_prefix("state.") {
            if !self.names.state.contains(key.split('.').next().unwrap_or_default()) {
                let d = self.diags.error("E_BIND_UNKNOWN", &at, format!("unknown state key `{key}`"));
                d.hint(format!("Declared state keys: {}", list(self.names.state.iter().map(String::as_str))));
                return None;
            }
            (format!("/state/{}", key.replace('.', "/")), Vec::new())
        } else if let Some(cols) = self.names.queries.get(&name) {
            (format!("/q/{name}"), cols.clone())
        } else {
            let queries: Vec<&str> = self.names.queries.keys().map(String::as_str).collect();
            let d = self.diags.error(
                "E_REF_QUERY",
                &at,
                format!("`each` takes a query, `services.<name>.<list>` or `state.<key>`; no query `{name}`"),
            );
            d.hint(match suggest(&name, queries.iter().copied()) {
                Some(s) => format!("Did you mean `{s}`? Queries: {}", list(queries)),
                None => format!("Queries: {}", list(queries)),
            });
            return None;
        };
        let take = m.get("take").and_then(|t| match t.as_u64().filter(|n| (1..=50).contains(n)) {
            Some(n) => Some(n),
            None => {
                self.diags.error("E_GLANCE_EACH", child(path, "take"), "`take` is how many rows to show: 1–50");
                None
            }
        });
        let saved = std::mem::replace(&mut self.names.row, cols);
        let item = if table {
            match m.get("cells") {
                Some(c @ Value::Array(_)) => value::compile(c, Scope::Row, &self.names, &child(path, "cells"), self.diags),
                _ => {
                    let d = self.diags.error("E_GLANCE_EACH", path, "a table's `each` names its row's `cells`");
                    d.hint("e.g. `rows: { each: top, cells: [\"{{row.name}}\", \"{{row.score}}\"] }`");
                    Value::Null
                }
            }
        } else {
            let fields = m.iter().filter(|(k, _)| !matches!(k.as_str(), "each" | "take"));
            Value::Object(
                fields.map(|(k, v)| (k.clone(), value::compile(v, Scope::Row, &self.names, &child(path, k), self.diags))).collect(),
            )
        };
        self.names.row = saved;
        let mut out = json!({ "each": { "p": source }, "item": item });
        if let Some(n) = take {
            out["take"] = json!(n);
        }
        Some(out)
    }

    /// The logic lane (ADR-0041): `{ script, values: [..], actions: [..] }`.
    fn logic(&mut self, logic: Option<&Value>) -> Option<Logic> {
        let v = logic?;
        let Some(m) = v.as_object() else {
            let d = self.diags.error("E_SHAPE", "logic", "`logic` is `{ script: |, values: [..], actions: [..] }`");
            d.hint("e.g. `logic: { values: [total], script: \"r1.on('change', () => r1.set('total', r1.q.items.length))\" }`");
            return None;
        };
        for k in m.keys() {
            if !["script", "values", "actions"].contains(&k.as_str()) {
                self.diags.warn("W_LOGIC_KEY", child("logic", k), format!("`logic` takes `script`, `values` and `actions`, not `{k}`"));
            }
        }
        let script = m.get("script").and_then(Value::as_str).unwrap_or_default().to_owned();
        if script.trim().is_empty() {
            self.diags.error("E_LOGIC", "logic.script", "the logic needs its `script`");
            return None;
        }
        if script.len() > LOGIC_SCRIPT_MAX {
            self.diags.error(
                "E_LOGIC",
                "logic.script",
                format!("the script is {} KB; logic may be {} KB", script.len() / 1024, LOGIC_SCRIPT_MAX / 1024),
            );
        }
        for (token, why) in LOGIC_FORBIDDEN {
            if script.contains(token) {
                self.diags.error("E_LOGIC_FORBIDDEN", "logic.script", format!("`{}` {why}", token.trim_end_matches('(')));
            }
        }
        // `fetch(` on its own (not `r1.fetch(`).
        if script.match_indices("fetch(").any(|(at, _)| !script[..at].ends_with('.')) {
            self.diags
                .error("E_LOGIC_FORBIDDEN", "logic.script", "`fetch` can't reach the network on its own: use `r1.fetch(url)`")
                .hint("r1.fetch goes through r1d, to the hosts the app's `net.http` lists");
        }
        if !script.contains("r1.") {
            self.diags.warn("W_LOGIC_R1", "logic.script", "the script never uses `r1`: it can't publish or do anything");
        }
        // `r1.fetch` goes through the app's `net.http` hosts.
        if script.contains("r1.fetch(") {
            self.need("net.http", "logic.script");
        }
        let names = |pass: &mut Self, key: &str| -> Vec<String> {
            let mut out = Vec::new();
            for (n, x) in m.get(key).and_then(Value::as_array).into_iter().flatten().enumerate() {
                match x.as_str() {
                    Some(s) if is_ident(s) => out.push(s.to_owned()),
                    _ => {
                        pass.diags.error("E_NAME", format!("logic.{key}[{n}]"), format!("logic {key} are snake_case names"));
                    }
                }
            }
            out
        };
        let values = names(self, "values");
        let actions = names(self, "actions");
        for a in &actions {
            if !script.contains(a.as_str()) {
                self.diags.warn("W_LOGIC_ACTION", "logic.actions", format!("the script never names `{a}`: `r1.action(\"{a}\", …)` answers it"));
            }
        }
        self.names.logic = values.iter().cloned().collect();
        self.logic_actions = actions.clone();
        Some(Logic { script, values, actions })
    }

    fn sql(&self, stmt: &str) -> Result<Info, sql::Error> {
        match &self.checker {
            Some(c) => c.check(stmt),
            None if stmt.trim().is_empty() => Err(sql::Error::Empty),
            None => Ok(Info { params: sql::named_params(stmt), readonly: true, ..Info::default() }),
        }
    }

    /// Reports a failed SQL check with a hint built from the schema.
    fn sql_error(&mut self, path: &str, err: sql::Error) {
        let (code, msg) = match err {
            sql::Error::Empty => ("E_SQL", "empty SQL".to_string()),
            sql::Error::Syntax(m) => ("E_SQL", m),
            sql::Error::Multiple => ("E_SQL_MULTI", "one statement per `sql` (use several steps)".to_string()),
            sql::Error::Positional(p) => ("E_SQL_PARAM", format!("`{p}`: use named params like `:reps`")),
            sql::Error::Denied(what) => ("E_SQL_DENIED", format!("not allowed in app SQL: {what}")),
        };
        let bare_time = ["no such column: now", "no such column: today", "no such column: current_date"];
        let hint = if bare_time.iter().any(|m| msg.contains(m)) {
            Some("SQL time: now() (UTC timestamp), today() (local date); omit a column declared `= now` to use its default".to_string())
        } else if msg.contains("no such column") || msg.contains("no such table") {
            let mut parts: Vec<String> = self
                .tables
                .iter()
                .map(|t| {
                    format!(
                        "`{}`({})",
                        t.name,
                        t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
                    )
                })
                .collect();
            if !self.settings.is_empty() {
                parts.push(format!("`settings`({})", self.settings.keys().cloned().collect::<Vec<_>>().join(", ")));
            }
            Some(format!("Tables: {}", if parts.is_empty() { "(none)".into() } else { parts.join("; ") }))
        } else if msg.contains("no such function") {
            Some(format!("Host helpers: {HELPERS}; plus SQLite built-ins"))
        } else if code == "E_SQL_DENIED" {
            Some("Apps may read and write their own tables; schema changes go in `data:`".to_string())
        } else {
            None
        };
        let d = self.diags.error(code, path, msg);
        if let Some(h) = hint {
            d.hint(h);
        }
    }

    fn need(&mut self, perm: &'static str, path: &str) {
        self.needs.push((perm, path.to_string()));
    }

    fn sections(&mut self, root: &Map<String, Value>) {
        let mut last = 0;
        for key in root.keys() {
            match SECTIONS.iter().position(|s| s == key) {
                Some(pos) => {
                    if pos < last {
                        let d = self.diags.warn(
                            "W_SECTION_ORDER",
                            key.as_str(),
                            format!("`{key}` should come before `{}`", SECTIONS[last]),
                        );
                        d.hint(format!("Section order: {}", SECTIONS.join(", ")));
                    }
                    last = last.max(pos);
                }
                None => {
                    let d = self.diags.error("E_KEY_UNKNOWN", key.as_str(), format!("unknown section `{key}`"));
                    d.hint(match suggest(key, SECTIONS) {
                        Some(s) => format!("Did you mean `{s}`?"),
                        None => format!("Sections: {}", SECTIONS.join(", ")),
                    });
                }
            }
        }
        match root.get("enee") {
            Some(v) if v.as_i64() == Some(1) => {}
            Some(v) => {
                self.diags.error("E_VERSION", "enee", format!("unsupported spec version `{v}`")).hint("Use `enee: 1`");
            }
            None => {
                self.diags.error("E_VERSION", "", "missing `enee: 1` as the first line");
            }
        }
        if !root.contains_key("views") {
            self.diags.error("E_SHAPE", "", "missing `views` (at least one view with blocks)");
        }
    }

    fn app(&mut self, app: Option<&Value>) -> Option<AppMeta> {
        let Some(app) = app.and_then(Value::as_object) else {
            self.diags.error("E_SHAPE", "app", "missing `app: { id, name, icon, archetype }`");
            return None;
        };
        for k in app.keys() {
            if !APP_KEYS.contains(&k.as_str()) {
                self.diags
                    .warn("W_KEY_UNKNOWN", child("app", k), format!("unknown key `{k}` ignored"))
                    .hint(format!("App keys: {}", APP_KEYS.join(", ")));
            }
        }
        if app.contains_key("window") {
            self.diags.warn("W_UNSUPPORTED", "app.window", "`app.window` is not supported yet (v0) and was ignored");
        }
        let text = |k: &str| app.get(k).and_then(Value::as_str).map(str::to_string);
        let id = text("id").unwrap_or_default();
        let kebab = !id.is_empty()
            && id.len() <= 40
            && id.starts_with(|c: char| c.is_ascii_lowercase())
            && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !id.ends_with('-')
            && !id.contains("--");
        if !kebab {
            self.diags.error(
                "E_APP_ID",
                "app.id",
                format!("app id `{id}` must be kebab-case (a-z, 0-9, -), at most 40 chars"),
            );
        }
        let Some(name) = text("name") else {
            self.diags.error("E_PROP_MISSING", "app", "missing `app.name`");
            return None;
        };
        let archetype = text("archetype").unwrap_or_else(|| "custom".into());
        if !ARCHETYPES.contains(&archetype.as_str()) {
            let d = self.diags.error("E_APP_ARCHETYPE", "app.archetype", format!("unknown archetype `{archetype}`"));
            d.hint(format!("Archetypes: {}", ARCHETYPES.join(", ")));
        }
        let theme = text("theme").unwrap_or_else(|| "dark".into());
        if !["dark", "light", "system"].contains(&theme.as_str()) {
            self.diags
                .error("E_APP_THEME", "app.theme", format!("unknown theme `{theme}`"))
                .hint("Themes: dark, light, system");
        }
        // Missing icons are picked from the app's name and description; unknown names are mapped (with a warning).
        let context = format!("{name} {} {archetype}", text("description").unwrap_or_default());
        let icon = match text("icon") {
            Some(requested) => crate::icons::resolve_prop(&requested, &context, "app.icon", self.diags).to_string(),
            None => crate::icons::resolve(None, &context).0.to_string(),
        };
        Some(AppMeta {
            id,
            name,
            icon,
            archetype,
            design: text("design").unwrap_or_else(|| "enee/default@^1".into()),
            accent: text("accent"),
            theme,
            description: text("description"),
        })
    }

    fn permissions(&mut self, perms: Option<&Value>) {
        let Some(perms) = perms else { return };
        let Some(map) = perms.as_object() else {
            self.diags.error(
                "E_SHAPE",
                "permissions",
                "`permissions` must map ids to reasons, e.g. `notify: Remind you to stretch`",
            );
            return;
        };
        for (id, v) in map {
            let path = child("permissions", id);
            if !PERMISSIONS.contains(&id.as_str()) {
                let d = self.diags.error("E_PERM_UNKNOWN", &path, format!("unknown permission `{id}`"));
                d.hint(match suggest(id, PERMISSIONS.iter().copied()) {
                    Some(s) => format!("Did you mean `{s}`?"),
                    None => format!("Permissions: {}", PERMISSIONS.join(", ")),
                });
                continue;
            }
            if ["app.storage", "app.ui"].contains(&id.as_str()) {
                self.diags.warn("W_PERM_IMPLICIT", &path, format!("`{id}` is always granted; remove it"));
                continue;
            }
            let (reason, scope) = match v {
                Value::String(r) => (r.clone(), None),
                Value::Object(m) => {
                    let reason = m.get("reason").and_then(Value::as_str).unwrap_or_default().to_string();
                    let scope: Map<String, Value> =
                        m.iter().filter(|(k, _)| *k != "reason").map(|(k, v)| (k.clone(), v.clone())).collect();
                    (reason, (!scope.is_empty()).then_some(Value::Object(scope)))
                }
                _ => (String::new(), None),
            };
            if reason.trim().is_empty() {
                self.diags
                    .error("E_PERM_REASON", &path, format!("permission `{id}` needs a reason the user will read"))
                    .hint("e.g. `notify: Remind you to do pushups during the day`");
                continue;
            }
            self.permissions.push(Permission { id: id.clone(), scope, reason });
        }
    }

    fn state(&mut self, state: Option<&Value>) {
        let Some(state) = state else { return };
        let Some(map) = state.as_object() else {
            self.diags.error("E_SHAPE", "state", "`state` must be a map of initial values");
            return;
        };
        for (k, v) in map {
            if k == "_persist" {
                continue;
            }
            if !is_ident(k) {
                self.diags.error("E_NAME", child("state", k), format!("state key `{k}` must be snake_case"));
                continue;
            }
            self.names.state.insert(k.clone());
            self.state.insert(k.clone(), v.clone());
        }
        // Keys kept across restarts: the rest is lost when the app unloads.
        if let Some(persist) = map.get("_persist") {
            let path = child("state", "_persist");
            let Some(keys) = persist.as_array() else {
                self.diags.error("E_SHAPE", &path, "`_persist` lists state keys, like `[ draft, tab ]`");
                return;
            };
            for key in keys {
                match key.as_str() {
                    Some(k) if self.state.contains_key(k) => {
                        if !self.persist.iter().any(|p| p == k) {
                            self.persist.push(k.to_string());
                        }
                    }
                    other => {
                        let names = self.state.keys().map(String::as_str);
                        let key = other.map_or_else(|| key.to_string(), str::to_string);
                        self.diags
                            .error("E_REF_STATE", &path, format!("unknown state key `{key}`"))
                            .hint(format!("Declared state keys: {}", list(names)));
                    }
                }
            }
        }
    }

    fn services(&mut self, services: Option<&Value>) -> Vec<Service> {
        let mut out = Vec::new();
        let Some(services) = services else { return out };
        let Some(map) = services.as_object() else {
            self.diags.error(
                "E_SHAPE",
                "services",
                "`services` must be a map like `cpu: { use: system.cpu, every: 1s }`",
            );
            return out;
        };
        for (name, def) in map {
            let path = child("services", name);
            if !is_ident(name) {
                self.diags.error("E_NAME", &path, format!("service name `{name}` must be snake_case"));
                continue;
            }
            let Some(def) = def.as_object() else {
                self.diags.error("E_SHAPE", &path, "a service is a map like `{ use: system.cpu, every: 1s }`");
                continue;
            };
            let uses = def.get("use").and_then(Value::as_str).unwrap_or_default().to_string();
            if uses.starts_with("svc:") {
                self.diags.error("E_UNSUPPORTED", child(&path, "use"), "custom services are not supported yet (v0)");
                continue;
            }
            let Some((_, perm)) = SERVICES.iter().find(|(s, _)| *s == uses) else {
                let names = SERVICES.iter().map(|(s, _)| *s);
                let d = self.diags.error("E_SERVICE_UNKNOWN", child(&path, "use"), format!("unknown service `{uses}`"));
                d.hint(format!("Services: {}", list(names)));
                continue;
            };
            self.need(perm, &child(&path, "use"));
            let min_every = if uses.starts_with("http.") { 60_000 } else { 200 };
            let clipboard = uses == "clipboard";
            let mut every_ms = None;
            let mut keep_ms = None;
            let mut options = Map::new();
            for (k, v) in def {
                match k.as_str() {
                    "use" => {}
                    "every" | "keep" if clipboard => {
                        let d = self.diags.error(
                            "E_SERVICE",
                            child(&path, k),
                            format!("the clipboard service records each copy as it happens, so it has no `{k}`"),
                        );
                        d.hint("Keep at most so many items with `limit: 500`");
                    }
                    "limit" => match v.as_u64() {
                        _ if !clipboard => {
                            self.diags.error("E_SERVICE", child(&path, k), "only the clipboard service takes `limit`");
                        }
                        Some(n @ 1..=5000) => {
                            options.insert(k.clone(), json!(n));
                        }
                        _ => {
                            let d = self.diags.error(
                                "E_SERVICE",
                                child(&path, k),
                                "`limit` is how many items to keep: a number from 1 to 5000",
                            );
                            if v.is_string() {
                                d.hint("Write the number itself, e.g. `limit: 500`: a setting can't change it");
                            }
                        }
                    },
                    "every" | "keep" => match duration_ms(v) {
                        Some(ms) if k == "every" && ms < min_every => {
                            self.diags.error(
                                "E_SERVICE",
                                child(&path, k),
                                format!("`every` must be at least {} ms for {uses}", min_every),
                            );
                        }
                        Some(ms) if k == "every" => every_ms = Some(ms),
                        Some(ms) => keep_ms = Some(ms),
                        None => {
                            self.diags.error(
                                "E_SERVICE",
                                child(&path, k),
                                format!("`{k}` must be a duration like 1s or 5m"),
                            );
                        }
                    },
                    "into" => {
                        let t = v.as_str().unwrap_or_default();
                        match self.tables.iter().find(|x| x.name == t) {
                            _ if uses != "http.rss" && !clipboard => {
                                let d = self.diags.error(
                                    "E_SERVICE",
                                    child(&path, k),
                                    "only `http.rss` and `clipboard` write into a table",
                                );
                                d.hint("Read other services' values with `{{services.<name>.<field>}}`");
                            }
                            None => {
                                self.diags.error(
                                    "E_REF_TABLE",
                                    child(&path, k),
                                    format!("no table `{t}` to write into"),
                                );
                            }
                            Some(table) if clipboard && !table.columns.iter().any(|c| c.name == "kind") => {
                                let d = self.diags.error(
                                    "E_SERVICE",
                                    child(&path, k),
                                    format!("table `{t}` needs a `kind` column for the clipboard's items"),
                                );
                                d.hint("Each copy is a row; `kind` is text, image or files, and the clipboard also fills `text`, `files`, `image`, `thumb` and `source` when the table has them, e.g. `clips: { kind: enum(text|image|files)!, text: text, files: text, image: text, thumb: text, source: text, pinned: bool = false }`");
                            }
                            Some(_) if clipboard => {}
                            Some(table) if !table.columns.iter().any(|c| c.name == "guid" && c.ty == "text") => {
                                let d = self.diags.error(
                                    "E_SERVICE",
                                    child(&path, k),
                                    format!("table `{t}` needs a `guid: text` column for the feed's items"),
                                );
                                d.hint("New items are added by `guid`; the feed also fills `title`, `link`, `summary`, `author` and `published` when the table has them, e.g. `items: { guid: text!, title: text, link: text, published: datetime, read: bool = false }`");
                            }
                            Some(_) => {}
                        }
                        options.insert(k.clone(), v.clone());
                    }
                    _ => {
                        options.insert(k.clone(), v.clone());
                    }
                }
            }
            if clipboard && !options.contains_key("into") {
                let d =
                    self.diags.error("E_SERVICE", &path, "the clipboard service needs `into: <table>` for its items");
                d.hint("e.g. `history: { use: clipboard, into: clips, limit: 500 }`");
            }
            let deps = if uses.starts_with("http.") {
                self.check_url(options.get("url"), &child(&path, "url"))
            } else {
                vec![]
            };
            self.names.services.insert(name.clone());
            out.push(Service { name: name.clone(), uses, every_ms, keep_ms, options, deps });
        }
        out
    }

    /// HTTP services need an https URL whose host is written out and listed in `net.http.hosts`. The path and query
    /// may bind plain `{{state.x}}` or `{{settings.x}}` values, which the host percent-encodes into the URL; returns
    /// their pointers (`/state/x`).
    fn check_url(&mut self, url: Option<&Value>, path: &str) -> Vec<String> {
        let Some(url) = url.and_then(Value::as_str) else {
            self.diags.error("E_SERVICE", path, "http services need a `url`");
            return Vec::new();
        };
        let Some(rest) = url.strip_prefix("https://") else {
            self.diags.error("E_SERVICE", path, "`url` must start with https://");
            return Vec::new();
        };
        let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
        if authority.contains("{{") {
            let d = self.diags.error("E_SERVICE", path, "the URL's host must be written out");
            d.hint("Bindings can go in the path and query, like https://api.example.com/v1/{{state.symbol}}");
            return Vec::new();
        }
        self.check_host(authority.split(':').next().unwrap_or_default(), path);
        let mut deps = Vec::new();
        let mut tail = rest;
        while let Some(start) = tail.find("{{") {
            let Some(len) = tail[start..].find("}}") else {
                self.diags.error("E_SERVICE", path, "a `{{` in the URL is never closed");
                break;
            };
            let inner = tail[start + 2..start + len].trim();
            tail = &tail[start + len + 2..];
            let parts: Vec<&str> = inner.split('.').collect();
            let known = match parts.as_slice() {
                ["state", key, rest @ ..] => {
                    self.names.state.contains(*key) && rest.iter().all(|p| is_ident(p) || p.parse::<usize>().is_ok())
                }
                ["settings", key] => self.names.settings.contains(*key),
                _ => false,
            };
            if !known {
                let d = self.diags.error("E_SERVICE", path, format!("the URL can't bind `{{{{{inner}}}}}`"));
                d.hint("URL bindings are plain `{{state.<key>}}` or `{{settings.<key>}}` values (no filters), declared in `state:` or `settings:`");
                continue;
            }
            let pointer = format!("/{}", parts.join("/"));
            if !deps.contains(&pointer) {
                deps.push(pointer);
            }
        }
        deps
    }

    /// `host` must be listed in `net.http.hosts`.
    fn check_host(&mut self, host: &str, path: &str) {
        let hosts: Vec<String> = self
            .permissions
            .iter()
            .find(|p| p.id == "net.http")
            .and_then(|p| p.scope.as_ref())
            .and_then(|s| s.get("hosts"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        if !hosts.iter().any(|h| h == host) {
            let d = self.diags.error("E_PERM_SCOPE", path, format!("host `{host}` is not covered by `net.http`"));
            d.hint(format!("Declare `net.http: {{ hosts: [{host}], reason: … }}`"));
        }
    }

    fn queries(&mut self, queries: Option<&Value>) -> Vec<crate::compiled::Query> {
        let mut out = Vec::new();
        let Some(queries) = queries else { return out };
        let Some(map) = queries.as_object() else {
            self.diags.error("E_SHAPE", "queries", "`queries` must map names to SQL");
            return out;
        };
        for name in map.keys() {
            if is_ident(name)
                && !["settings", "state", "services", "params", "row", "app", "now"].contains(&name.as_str())
            {
                self.names.queries.insert(name.clone(), Vec::new());
            } else {
                self.diags.error(
                    "E_NAME",
                    child("queries", name),
                    format!("query name `{name}` must be snake_case and not a binding root"),
                );
            }
        }
        for (name, def) in map {
            if !self.names.queries.contains_key(name) {
                continue;
            }
            let path = child("queries", name);
            let (stmt, params_src, sql_path) = match def {
                Value::String(s) => (s.trim().to_string(), None, path.clone()),
                Value::Object(m) => {
                    for k in m.keys() {
                        if !["sql", "params"].contains(&k.as_str()) {
                            self.diags
                                .warn("W_KEY_UNKNOWN", child(&path, k), format!("unknown key `{k}` ignored"))
                                .hint("Query keys: sql, params");
                        }
                    }
                    let s = m.get("sql").and_then(Value::as_str).unwrap_or_default().trim().to_string();
                    (s, m.get("params").and_then(Value::as_object).cloned(), child(&path, "sql"))
                }
                other => {
                    self.diags.error(
                        "E_SHAPE",
                        &path,
                        format!("a query is SQL text or `{{ sql, params }}`, got {}", kind(other)),
                    );
                    continue;
                }
            };
            let info = match self.sql(&stmt) {
                Ok(info) => info,
                Err(e) => {
                    self.sql_error(&sql_path, e);
                    continue;
                }
            };
            if info.validated && !info.readonly {
                self.diags
                    .error("E_SQL_NOT_READONLY", &sql_path, "queries must be read-only (SELECT)")
                    .hint("Put writes in an action's `sql` step");
                continue;
            }
            let declared = params_src.unwrap_or_default();
            for p in &info.params {
                if !declared.contains_key(p) {
                    let d = self.diags.error(
                        "E_SQL_PARAM",
                        &sql_path,
                        format!("`:{p}` is not declared in the query's `params`"),
                    );
                    d.hint(format!("Add `params: {{ {p}: \"{{{{state.{p}}}}}\" }}` to the query"));
                }
            }
            let mut params = Map::new();
            for (k, v) in &declared {
                let ppath = child(&child(&path, "params"), k);
                if !info.params.contains(k) {
                    self.diags.warn(
                        "W_SQL_PARAM_UNUSED",
                        &ppath,
                        format!("param `{k}` is not used as `:{k}` in the SQL"),
                    );
                }
                let v = value::compile(v, Scope::QueryParam, &self.names, &ppath, self.diags);
                params.insert(k.clone(), v);
            }
            let mut seen = BTreeSet::new();
            for c in &info.columns {
                if !is_ident(c) {
                    let d = self.diags.warn(
                        "W_SQL_COLUMN_NAME",
                        &sql_path,
                        format!("result column `{c}` can't be bound by name"),
                    );
                    d.hint("Alias computed columns, e.g. `count(*) AS n`");
                } else if !seen.insert(c.as_str()) {
                    self.diags.warn("W_SQL_COLUMN_NAME", &sql_path, format!("duplicate result column `{c}`"));
                }
            }
            self.names.queries.insert(name.clone(), info.columns.clone());
            out.push(Query { name: name.clone(), sql: stmt, params, tables: info.reads, columns: info.columns });
        }
        out
    }

    fn actions(&mut self, actions: Option<&Value>) -> Map<String, Value> {
        let mut out = Map::new();
        let Some(actions) = actions else { return out };
        let Some(map) = actions.as_object() else {
            self.diags.error("E_SHAPE", "actions", "`actions` must map names to `{ params, do }`");
            return out;
        };
        // Signatures first: blocks and checks reference actions by name.
        let mut compiled_params: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        for (name, def) in map {
            let path = child("actions", name);
            if !is_ident(name) {
                self.diags.error("E_NAME", &path, format!("action name `{name}` must be snake_case"));
                continue;
            }
            let mut sig = ActionSig { params: BTreeMap::new(), enums: BTreeMap::new() };
            let mut params = Map::new();
            if let Some(ps) = def.get("params") {
                let Some(ps) = ps.as_object() else {
                    self.diags.error(
                        "E_SHAPE",
                        child(&path, "params"),
                        "`params` must map names to types, e.g. `{ reps: int, note: text? }`",
                    );
                    continue;
                };
                for (p, ty) in ps {
                    let ppath = child(&child(&path, "params"), p);
                    if !is_ident(p) {
                        self.diags.error("E_NAME", &ppath, format!("param name `{p}` must be snake_case"));
                        continue;
                    }
                    // `{p}`, `{i}` and `{t}` objects are Value IR forms, so args maps can't use these keys.
                    if ["p", "i", "t"].contains(&p.as_str()) {
                        let d = self.diags.error("E_NAME_RESERVED", &ppath, format!("`{p}` can't name a param"));
                        d.hint("Use a descriptive name, e.g. `page`, `index` or `text`");
                        continue;
                    }
                    let Some(decl) = ty.as_str() else {
                        self.diags.error("E_FIELD_TYPE", &ppath, "param type must be a string like `int` or `text?`");
                        continue;
                    };
                    let (decl, required) = match decl.trim() {
                        d if d.ends_with('?') => (d.trim_end_matches('?'), false),
                        d => (d.trim_end_matches('!'), true),
                    };
                    match Ty::parse(decl) {
                        Ok((t, _)) if !matches!(t, Ty::Ref(_)) => {
                            let mut def = Map::new();
                            def.insert("type".into(), json!(t.name()));
                            def.insert("required".into(), json!(required));
                            if let Ty::Enum(opts) = &t {
                                def.insert("options".into(), json!(opts));
                                sig.enums.insert(p.clone(), opts.clone());
                            }
                            params.insert(p.clone(), Value::Object(def));
                            sig.params.insert(p.clone(), required);
                        }
                        _ => {
                            self.diags.error("E_FIELD_TYPE", &ppath, format!("unknown param type `{decl}`")).hint("Types: int, real, text, bool, datetime, date, time, duration, json, enum(a|b); `?` = optional");
                        }
                    }
                }
            }
            self.actions.insert(name.clone(), sig);
            compiled_params.insert(name.clone(), params);
        }
        for (name, def) in map {
            let Some(params) = compiled_params.remove(name) else { continue };
            let path = child("actions", name);
            let inline = def.as_object().and_then(inline_steps);
            if let Some(m) = def.as_object() {
                for k in m.keys() {
                    let read = inline.is_some() && STEP_KEYS.contains(&k.as_str());
                    if !read && !["params", "confirm", "do"].contains(&k.as_str()) {
                        self.diags
                            .warn("W_KEY_UNKNOWN", child(&path, k), format!("unknown key `{k}` ignored"))
                            .hint("Action keys: params, confirm, do");
                    }
                }
            }
            self.names.params = Some(params.keys().cloned().collect());
            let mut action = Map::new();
            action.insert("params".into(), Value::Object(params));
            if let Some(c) = def.get("confirm") {
                let c = value::compile(c, Scope::Step, &self.names, &child(&path, "confirm"), self.diags);
                action.insert("confirm".into(), c);
            }
            let steps = match def.as_object() {
                Some(m) => self.do_steps(m, inline.as_ref(), &path, false),
                None => self.steps(None, &child(&path, "do"), false),
            };
            action.insert("steps".into(), Value::Array(steps));
            self.names.params = None;
            out.insert(name.clone(), Value::Object(action));
        }
        out
    }

    /// An action's or trigger's steps: its `do`, or the steps written straight under it ([`inline_steps`]), which
    /// is worth a warning.
    fn do_steps(
        &mut self,
        def: &Map<String, Value>,
        inline: Option<&Value>,
        path: &str,
        in_trigger: bool,
    ) -> Vec<Value> {
        let path = child(path, "do");
        if let Some(inline) = inline {
            self.diags
                .warn("W_AUTOFIX", &path, "steps written without `do:` were read as its `do` list")
                .hint("Write steps under `do:`, e.g. `do: [ { sql: … }, { toast: Saved } ]`");
            return self.steps(Some(inline), &path, in_trigger);
        }
        self.steps(def.get("do"), &path, in_trigger)
    }

    fn steps(&mut self, steps: Option<&Value>, path: &str, in_trigger: bool) -> Vec<Value> {
        let items: Vec<(String, &Value)> = match steps {
            Some(Value::Array(a)) => a.iter().enumerate().map(|(i, s)| (index(path, i), s)).collect(),
            Some(single @ Value::Object(_)) => vec![(path.to_string(), single)],
            Some(other) => {
                self.diags.error("E_SHAPE", path, format!("`do` must be a list of steps, got {}", kind(other)));
                return Vec::new();
            }
            None => {
                self.diags.error("E_PROP_MISSING", path, "missing `do:` steps");
                return Vec::new();
            }
        };
        items.into_iter().filter_map(|(p, s)| self.step(s, &p, in_trigger)).collect()
    }

    fn step(&mut self, step: &Value, path: &str, in_trigger: bool) -> Option<Value> {
        let Some(map) = step.as_object().filter(|m| m.len() == 1) else {
            self.diags.error(
                "E_STEP_SHAPE",
                path,
                "each step is a map with one key, e.g. `- sql: INSERT …` or `- toast: Saved`",
            );
            return None;
        };
        let (kind_name, body) = map.iter().next()?;
        let kind_name = kind_name.as_str();
        let bpath = child(path, kind_name);
        if LATER_STEPS.contains(&kind_name) {
            self.diags
                .error("E_UNSUPPORTED", &bpath, format!("`{kind_name}` steps are not supported yet (v0)"))
                .hint(format!("Steps: {}", STEPS.join(", ")));
            return None;
        }
        if !STEPS.contains(&kind_name) {
            let d = self.diags.error("E_STEP_UNKNOWN", &bpath, format!("unknown step `{kind_name}`"));
            d.hint(match suggest(kind_name, STEPS) {
                Some(s) => format!("Did you mean `{s}`? Steps: {}", STEPS.join(", ")),
                None => format!("Steps: {}", STEPS.join(", ")),
            });
            return None;
        }
        if in_trigger && !TRIGGER_STEPS.contains(&kind_name) {
            let d = self.diags.error(
                "E_TRIGGER_STEP",
                &bpath,
                format!("`{kind_name}` can't run in a trigger (the app may be closed)"),
            );
            d.hint(format!("Trigger steps: {}", TRIGGER_STEPS.join(", ")));
            return None;
        }
        let val = |pass: &mut Self, v: &Value, p: &str| value::compile(v, Scope::Step, &pass.names, p, pass.diags);
        let compiled = match kind_name {
            "sql" => {
                let stmt = body.as_str().unwrap_or_default().trim().to_string();
                let info = match self.sql(&stmt) {
                    Ok(i) => i,
                    Err(e) => {
                        self.sql_error(&bpath, e);
                        return None;
                    }
                };
                if info.validated && info.readonly {
                    self.diags
                        .warn("W_SQL_READONLY_STEP", &bpath, "this step only reads; it has no effect")
                        .hint("Read data with a named query instead");
                }
                for p in &info.params {
                    let known = self.names.params.as_ref().is_some_and(|ps| ps.contains(p))
                        || (!in_trigger && self.names.state.contains(p))
                        || self.names.settings.contains(p);
                    if !known {
                        let from = if in_trigger { "settings" } else { "the action's params, state or settings" };
                        self.diags
                            .error("E_SQL_PARAM", &bpath, format!("`:{p}` is not in {from}"))
                            .hint(format!("Declare it, e.g. `params: {{ {p}: int }}`"));
                    }
                }
                json!({ "sql": stmt, "params": info.params })
            }
            "toast" => json!({ "toast": val(self, body, &bpath) }),
            "copy" => {
                // A click's copy is implicit; a trigger's happens while the user does something else.
                if in_trigger {
                    self.need("clipboard.write", &bpath);
                }
                match body.as_object() {
                    // `copy: { text, image, files }`: an item of clipboard history, copied back as what it was: the
                    // first part that isn't empty, files before an image before text (ADR-0022).
                    Some(parts) => {
                        if in_trigger {
                            self.diags.error("E_STEP", &bpath, "a trigger's copy takes text only");
                        }
                        let mut out = Map::new();
                        for (k, v) in parts {
                            if !["text", "image", "files"].contains(&k.as_str()) {
                                let d = self.diags.error(
                                    "E_STEP",
                                    child(&bpath, k),
                                    format!("copy takes `text`, `image` or `files`, not `{k}`"),
                                );
                                d.hint("e.g. `copy: { text: \"{{params.text}}\", image: \"{{params.image}}\", files: \"{{params.files}}\" }`");
                                continue;
                            }
                            out.insert(k.clone(), val(self, v, &child(&bpath, k)));
                        }
                        if out.is_empty() {
                            self.diags.error("E_STEP", &bpath, "copy needs `text`, `image` or `files`");
                        }
                        json!({ "copy_parts": out })
                    }
                    None => json!({ "copy": val(self, body, &bpath) }),
                }
            }
            "open_url" => {
                if let Some(u) = body.as_str().filter(|u| !u.contains("{{"))
                    && !(u.starts_with("https://") || u.starts_with("http://"))
                {
                    self.diags.error("E_STEP", &bpath, "open_url needs an http(s) URL");
                }
                self.need("open.url", &bpath);
                json!({ "open_url": val(self, body, &bpath) })
            }
            "play" => {
                self.need("audio.play", &bpath);
                let literal = |u: &str| !(u.contains("{{") || u.starts_with("https://") || u.starts_with("http://"));
                match body {
                    Value::String(u) => {
                        if literal(u) {
                            self.diags.error("E_STEP", &bpath, "play needs an http(s) stream URL");
                        }
                        json!({ "play": { "url": val(self, body, &bpath) } })
                    }
                    Value::Object(parts) => {
                        let mut out = Map::new();
                        for (k, v) in parts {
                            if !["url", "title", "subtitle"].contains(&k.as_str()) {
                                let d = self.diags.error("E_STEP", child(&bpath, k), format!("play takes `url`, `title` and `subtitle`, not `{k}`"));
                                d.hint("e.g. `play: { url: \"{{params.url}}\", title: \"{{params.name}}\" }`");
                                continue;
                            }
                            if k == "url" && v.as_str().is_some_and(literal) {
                                self.diags.error("E_STEP", child(&bpath, k), "play needs an http(s) stream URL");
                            }
                            out.insert(k.clone(), val(self, v, &child(&bpath, k)));
                        }
                        if !out.contains_key("url") {
                            self.diags.error("E_STEP", &bpath, "play needs a `url`: the stream's address");
                        }
                        json!({ "play": out })
                    }
                    _ => {
                        self.diags.error("E_STEP", &bpath, "play takes a URL, or `{ url, title, subtitle }`");
                        return None;
                    }
                }
            }
            "run" => {
                // One of the app's script's actions (ADR-0041).
                let name = body.as_str().unwrap_or_default().trim().to_owned();
                if !self.logic_actions.contains(&name) {
                    let d = self.diags.error("E_REF_LOGIC", &bpath, format!("the app's logic has no action `{name}`"));
                    d.hint(if self.logic_actions.is_empty() {
                        "Declare it: `logic: { actions: [name], script: ... r1.action(\"name\", (params) => …) }`".to_owned()
                    } else {
                        format!("Logic actions: {}", self.logic_actions.join(", "))
                    });
                }
                json!({ "run": name })
            }
            kind if PLAYER_STEPS.contains(&kind) => {
                self.need("audio.play", &bpath);
                json!({ kind: true })
            }
            "launch" => {
                if body.as_str().is_none_or(|name| name.trim().is_empty()) {
                    let d =
                        self.diags.error("E_STEP", &bpath, "launch needs the app's name, as the Start menu shows it");
                    d.hint("e.g. `- launch: Spotify`");
                }
                self.need("apps.launch", &bpath);
                json!({ "launch": val(self, body, &bpath) })
            }
            "navigate" => {
                let v = body.as_str().unwrap_or_default();
                if !self.views.iter().any(|x| x == v) {
                    let d = self.diags.error("E_REF_VIEW", &bpath, format!("no view `{v}`"));
                    d.hint(format!("Views: {}", list(self.views.iter().map(String::as_str))));
                }
                json!({ "navigate": v })
            }
            "refresh" => {
                let q = body.as_str().unwrap_or_default();
                if !self.names.queries.contains_key(q) {
                    let d = self.diags.error("E_REF_QUERY", &bpath, format!("no query `{q}`"));
                    d.hint(format!("Queries: {}", list(self.names.queries.keys().map(String::as_str))));
                }
                json!({ "refresh": q })
            }
            "notify" => {
                self.need("notify", &bpath);
                let (title, rest) = match body {
                    Value::Object(m) => (m.get("title").cloned(), m.clone()),
                    other => (Some(other.clone()), Map::new()),
                };
                let Some(title) = title else {
                    self.diags.error("E_PROP_MISSING", &bpath, "notify needs a `title`");
                    return None;
                };
                let mut n = Map::new();
                n.insert("title".into(), val(self, &title, &child(&bpath, "title")));
                if let Some(b) = rest.get("body") {
                    n.insert("body".into(), val(self, b, &child(&bpath, "body")));
                }
                if let Some(c) = rest.get("click") {
                    let c = c.as_str().unwrap_or_default();
                    let ok = c == "open" || c.strip_prefix("action:").is_some_and(|a| self.actions.contains_key(a));
                    if !ok {
                        self.diags.error(
                            "E_STEP",
                            child(&bpath, "click"),
                            format!("`click: {c}` must be `open` or `action:<name>`"),
                        );
                    }
                    n.insert("click".into(), json!(c));
                }
                for k in rest.keys() {
                    if !["title", "body", "click"].contains(&k.as_str()) {
                        self.diags
                            .warn("W_KEY_UNKNOWN", child(&bpath, k), format!("unknown key `{k}` ignored"))
                            .hint("notify keys: title, body, click");
                    }
                }
                json!({ "notify": n })
            }
            "set" => {
                let Some(m) = body.as_object() else {
                    self.diags.error("E_SHAPE", &bpath, "set is a map like `{ state.draft.reps: 15 }`");
                    return None;
                };
                let mut out = Map::new();
                for (k, v) in m {
                    let kpath = child(&bpath, k);
                    let Some(target) = value::bind_path(k, &self.names, &kpath, self.diags) else { continue };
                    out.insert(target, val(self, v, &kpath));
                }
                json!({ "set": out })
            }
            _ => unreachable!("checked against STEPS"),
        };
        Some(compiled)
    }

    fn compile_views(&mut self, views: Option<&Value>) -> (Vec<View>, Map<String, Value>) {
        let mut out = Vec::new();
        let mut sourcemap = Map::new();
        let Some(views) = views else { return (out, sourcemap) };
        // (name, definition, YAML path)
        let entries: Vec<(String, Value, String)> = match views {
            Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone(), child("views", k))).collect(),
            Value::Array(_) => {
                self.diags
                    .warn("W_AUTOFIX", "views", "`views` is a list; read as one view named `main`")
                    .hint("Name each view: `views: { main: [ … ] }`");
                vec![("main".into(), views.clone(), "views".into())]
            }
            _ => {
                self.diags.error("E_SHAPE", "views", "`views` must map view names to lists of blocks");
                return (out, sourcemap);
            }
        };
        if entries.is_empty() {
            self.diags.error("E_SHAPE", "views", "add at least one view");
        }
        for (name, def, path) in &entries {
            let path = path.clone();
            if !is_ident(name) {
                self.diags.error("E_NAME", &path, format!("view name `{name}` must be snake_case"));
                continue;
            }
            let (items, ipath, title, icon) = match def {
                Value::Object(m) => {
                    for k in m.keys() {
                        if !["title", "icon", "items"].contains(&k.as_str()) {
                            self.diags
                                .warn("W_KEY_UNKNOWN", child(&path, k), format!("unknown key `{k}` ignored"))
                                .hint("A long-form view has: title, icon, items");
                        }
                    }
                    let text = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
                    let icon = text("icon").map(|i| {
                        crate::icons::resolve_prop(
                            &i,
                            &text("title").unwrap_or_default(),
                            &child(&path, "icon"),
                            self.diags,
                        )
                        .to_string()
                    });
                    (
                        m.get("items").cloned().unwrap_or(Value::Array(Vec::new())),
                        child(&path, "items"),
                        text("title"),
                        icon,
                    )
                }
                other => (other.clone(), path.clone(), None, None),
            };
            let mut cx = Ctx {
                names: &mut self.names,
                actions: &self.actions,
                state: &self.state,
                lines: self.lines,
                diags: self.diags,
            };
            let compiled = blocks::compile_view(name, &items, &ipath, &mut cx);
            sourcemap.extend(compiled.sourcemap);
            out.push(View { name: name.clone(), title, icon, spec: compiled.spec });
        }
        (out, sourcemap)
    }

    fn triggers(&mut self, triggers: Option<&Value>) -> Vec<Value> {
        let mut out = Vec::new();
        let Some(triggers) = triggers else { return out };
        let Some(map) = triggers.as_object() else {
            self.diags.error("E_SHAPE", "triggers", "`triggers` must map names to `{ every|at|on, when?, do }`");
            return out;
        };
        for (name, def) in map {
            let path = child("triggers", name);
            let Some(def) = def.as_object() else {
                self.diags.error("E_SHAPE", &path, "a trigger is a map like `{ every: 1h, do: [...] }`");
                continue;
            };
            if !is_ident(name) {
                self.diags.error("E_NAME", &path, format!("trigger name `{name}` must be snake_case"));
                continue;
            }
            self.need("schedule", &path);
            let mut t = Map::new();
            t.insert("name".into(), json!(name));
            let schedules: Vec<&str> =
                ["every", "at", "on", "cron"].into_iter().filter(|k| def.contains_key(*k)).collect();
            if schedules.len() != 1 {
                self.diags.error("E_TRIGGER", &path, "a trigger needs exactly one of `every`, `at` or `on`");
            }
            let inline = inline_steps(def);
            for (k, v) in def {
                let kpath = child(&path, k);
                match k.as_str() {
                    "every" => {
                        if let Some(every) = self.trigger_every(v, &kpath) {
                            t.insert("every".into(), every);
                        }
                    }
                    "at" => {
                        let ok =
                            v.as_str().is_some_and(is_clock_time) || self.setting_ref(v, &["time"], &kpath).is_some();
                        if ok {
                            t.insert("at".into(), value::compile(v, Scope::Step, &self.names, &kpath, self.diags));
                        } else {
                            self.diags.error(
                                "E_TRIGGER",
                                &kpath,
                                "`at` must be a local time like \"09:00\" or a time setting",
                            );
                        }
                    }
                    "on" => {
                        let on = v.as_str().unwrap_or_default();
                        let ok = match on.strip_prefix("change:") {
                            Some(table) => self.tables.iter().any(|t| t.name == table) || table == "settings",
                            None => ["start", "open", "close"].contains(&on),
                        };
                        if !ok {
                            self.diags
                                .error("E_TRIGGER", &kpath, format!("unknown event `{on}`"))
                                .hint("Events: start, open, close, change:<table>");
                        }
                        t.insert("on".into(), json!(on));
                    }
                    "cron" => {
                        self.diags
                            .error("E_UNSUPPORTED", &kpath, "cron triggers are not supported yet (v0)")
                            .hint("Use `every: 1h` or `at: \"09:00\"`");
                    }
                    "skip" => {
                        let ok = v.as_str().is_some_and(|s| {
                            s.split_once('-').is_some_and(|(a, b)| is_clock_time(a.trim()) && is_clock_time(b.trim()))
                        }) || self.setting_ref(v, &["time_range"], &kpath).is_some();
                        if ok {
                            t.insert("skip".into(), value::compile(v, Scope::Step, &self.names, &kpath, self.diags));
                        } else {
                            self.diags.error(
                                "E_TRIGGER",
                                &kpath,
                                "`skip` must be a range like \"21:30-08:00\" or a time_range setting",
                            );
                        }
                    }
                    "when" => {
                        let stmt = v.as_str().unwrap_or_default().trim().to_string();
                        match self.sql(&stmt) {
                            Ok(info) => {
                                if info.validated && (!info.readonly || info.columns.len() != 1) {
                                    self.diags.error(
                                        "E_TRIGGER",
                                        &kpath,
                                        "`when` must be a read-only SELECT returning one value",
                                    );
                                }
                                for p in &info.params {
                                    if !self.names.settings.contains(p) {
                                        self.diags.error(
                                            "E_SQL_PARAM",
                                            &kpath,
                                            format!("`:{p}`: trigger SQL can only use settings"),
                                        );
                                    }
                                }
                                t.insert("when".into(), json!(stmt));
                            }
                            Err(e) => self.sql_error(&kpath, e),
                        }
                    }
                    "do" => {}
                    step if inline.is_some() && STEP_KEYS.contains(&step) => {}
                    other => {
                        self.diags
                            .warn("W_KEY_UNKNOWN", &kpath, format!("unknown key `{other}` ignored"))
                            .hint("Trigger keys: every, at, on, skip, when, do");
                    }
                }
            }
            let steps = self.do_steps(def, inline.as_ref(), &path, true);
            t.insert("steps".into(), Value::Array(steps));
            out.push(Value::Object(t));
        }
        out
    }

    /// `every`: a literal duration (≥ 60 s, emitted in seconds) or a binding to a duration setting.
    fn trigger_every(&mut self, v: &Value, path: &str) -> Option<Value> {
        if let Some(s) = v.as_str()
            && s.contains("{{")
        {
            self.setting_ref(v, &["duration", "int"], path).or_else(|| {
                self.diags.error("E_TRIGGER", path, "`every` must be a duration like 1h or a duration setting");
                None
            })?;
            return Some(value::compile(v, Scope::Step, &self.names, path, self.diags));
        }
        match duration_ms(v) {
            Some(ms) if ms >= 60_000 => Some(json!(ms / 1000)),
            Some(_) => {
                self.diags.error("E_TRIGGER", path, "triggers run at most once a minute (`every` ≥ 1m)");
                None
            }
            None => {
                self.diags.error("E_TRIGGER", path, "`every` must be a duration like 1h or a duration setting");
                None
            }
        }
    }

    /// `"{{settings.x}}"` where setting x has one of `types`.
    fn setting_ref(&self, v: &Value, types: &[&str], _path: &str) -> Option<String> {
        let s = v.as_str()?.trim();
        let key = s.strip_prefix("{{")?.strip_suffix("}}")?.trim().strip_prefix("settings.")?;
        let ty = self.settings.get(key)?.get("type")?.as_str()?;
        types.contains(&ty).then(|| key.to_string())
    }

    fn checks(&mut self, checks: Option<&Value>) -> Vec<Value> {
        let mut out = Vec::new();
        let Some(checks) = checks else { return out };
        let Some(items) = checks.as_array() else {
            self.diags.error("E_SHAPE", "checks", "`checks` must be a list");
            return out;
        };
        for (i, check) in items.iter().enumerate() {
            let path = index("checks", i);
            let Some(m) = check.as_object() else {
                self.diags.error(
                    "E_SHAPE",
                    &path,
                    "a check is `{ do: { action, params }, expect: { query.column: value } }` or `{ render: <view> }`",
                );
                continue;
            };
            if let Some(view) = m.get("render") {
                let v = view.as_str().unwrap_or_default();
                if !self.views.iter().any(|x| x == v) {
                    self.diags.error("E_REF_VIEW", child(&path, "render"), format!("no view `{v}`"));
                }
                out.push(json!({ "render": v }));
                continue;
            }
            // An app without actions has nothing for a check to run (builds wrote such checks anyway): it goes.
            if self.actions.is_empty() && m.contains_key("do") {
                self.diags
                    .warn("W_AUTOFIX", &path, "the app has no actions, so this check had nothing to run: dropped")
                    .hint("Apps without actions don't need checks");
                continue;
            }
            let mut c = Map::new();
            let dos: Vec<(String, &Value)> = match m.get("do") {
                Some(Value::Array(a)) => {
                    a.iter().enumerate().map(|(j, d)| (index(&child(&path, "do"), j), d)).collect()
                }
                Some(d) => vec![(child(&path, "do"), d)],
                None => Vec::new(),
            };
            let mut compiled_dos = Vec::new();
            for (dpath, d) in &dos {
                let Some(dm) = d.as_object() else {
                    self.diags.error("E_SHAPE", dpath, "`do` is `{ action, params }`");
                    continue;
                };
                let mut cx = Ctx {
                    names: &mut self.names,
                    actions: &self.actions,
                    state: &self.state,
                    lines: self.lines,
                    diags: self.diags,
                };
                if let Some(args) = cx.args(dm, dpath, Scope::Step) {
                    compiled_dos.push(json!({ "action": dm["action"], "args": args }));
                }
            }
            match (m.get("do"), compiled_dos.len()) {
                (Some(Value::Array(_)), _) => {
                    c.insert("do".into(), Value::Array(compiled_dos));
                }
                (Some(_), 1) => {
                    c.insert("do".into(), compiled_dos.remove(0));
                }
                _ => {}
            }
            let Some(expect) = m.get("expect").and_then(Value::as_object) else {
                self.diags.error("E_SHAPE", &path, "a check needs `expect: { query.column: value }`");
                continue;
            };
            let mut e = Map::new();
            for (k, v) in expect {
                let kpath = child(&child(&path, "expect"), k);
                let compiled =
                    value::compile_str(&format!("{{{{{k}}}}}"), Scope::Element, &self.names, &kpath, self.diags);
                match compiled.get("$state").and_then(Value::as_str) {
                    Some(p) => {
                        e.insert(p.to_string(), v.clone());
                    }
                    None if compiled.is_null() => {}
                    None => {
                        self.diags.error("E_CHECK", &kpath, "expect keys are plain paths like `today.total`");
                    }
                }
            }
            c.insert("expect".into(), Value::Object(e));
            out.push(Value::Object(c));
        }
        out
    }

    /// Every capability used must be declared; every declared permission should be used.
    /// Every needed permission is declared, and every declared one is used. `schedule` is used by triggers, and also by
    /// services, which it keeps running in the background (permissions-and-services.md §2).
    fn coverage(&mut self, has_services: bool) {
        let declared: BTreeSet<&str> = self.permissions.iter().map(|p| p.id.as_str()).collect();
        let mut reported = BTreeSet::new();
        for (perm, path) in &self.needs {
            if !declared.contains(perm) && reported.insert(*perm) {
                let d =
                    self.diags.error("E_PERM_MISSING", path.as_str(), format!("this needs the `{perm}` permission"));
                d.hint(format!("Add `{perm}: <reason the user will read>` under `permissions:`"));
            }
        }
        let mut used: BTreeSet<&str> = self.needs.iter().map(|(p, _)| *p).collect();
        if has_services {
            used.insert("schedule");
        }
        for p in &self.permissions {
            if !used.contains(p.id.as_str()) {
                self.diags
                    .warn(
                        "W_PERM_UNUSED",
                        child("permissions", &p.id),
                        format!("`{}` is declared but nothing uses it", p.id),
                    )
                    .hint("Remove it so the consent card stays short");
            }
        }
    }
}

/// The step kinds, for reading steps written straight under an action or trigger.
const STEP_KEYS: [&str; 15] = [
    "sql", "toast", "set", "navigate", "notify", "refresh", "open_url", "copy", "launch", "play", "pause", "resume", "stop", "toggle", "run",
];

/// Steps written straight under an action or trigger, without `do:` (`remove: { params: …, sql: DELETE … }`): builds
/// wrote them so, and each step is one key, so they read as its `do` list, in order. `None` when it has a `do`.
fn inline_steps(def: &Map<String, Value>) -> Option<Value> {
    if def.contains_key("do") {
        return None;
    }
    let steps: Vec<Value> = def
        .iter()
        .filter(|(k, _)| STEP_KEYS.contains(&k.as_str()))
        .map(|(k, v)| Value::Object(Map::from_iter([(k.clone(), v.clone())])))
        .collect();
    (!steps.is_empty()).then_some(Value::Array(steps))
}

/// A binding only the app's open page has (`logic.*`, `player.*`), if a compiled Value reads one.
fn page_only(v: &Value) -> Option<&'static str> {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(p)) = m.get("p") {
                if p.starts_with("/logic/") {
                    return Some("logic");
                }
                if p.starts_with("/player/") {
                    return Some("player");
                }
            }
            m.values().find_map(page_only)
        }
        Value::Array(items) => items.iter().find_map(page_only),
        _ => None,
    }
}
