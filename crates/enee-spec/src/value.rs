//! `{{path | filter:arg | …}}` bindings → the Value IR and json-render props (docs/specs/compiled-app.md §3).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::diag::Diags;

/// What `{{player.…}}` reads (ADR-0040): whether it's playing, its state (`playing`, `paused`, `loading`, `stopped`,
/// `error`), what the app asked to play (`title`, `subtitle`, `url`), the song a station says is on (`song`), and why
/// it stopped (`error`).
pub const PLAYER_FIELDS: &[&str] = &["playing", "state", "title", "subtitle", "song", "url", "error"];

pub const FILTERS: &[&str] = &[
    "time", "date", "datetime", "relative", "duration", "seconds", "number", "percent", "bytes", "round", "upper",
    "lower", "plural", "default",
];

/// Where a value is used decides which roots are legal and how plain bindings are emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// json-render element props: plain → `{"$state"}`, otherwise `{"$x": IR}`.
    Element,
    /// Row templates (List title/caption/rowActions): IR with `row.*` → `{"i"}`.
    Row,
    /// Action/trigger steps: IR with `params.*` allowed.
    Step,
    /// Query params bound from state/settings: IR.
    QueryParam,
}

/// Names a binding may refer to.
#[derive(Default)]
pub struct Names {
    /// Query name → known result columns (empty when SQL checking is unavailable).
    pub queries: BTreeMap<String, Vec<String>>,
    pub settings: BTreeSet<String>,
    pub state: BTreeSet<String>,
    pub services: BTreeSet<String>,
    /// What the app's script publishes (`logic.values`, ADR-0041).
    pub logic: BTreeSet<String>,
    /// Action params (only set while compiling an action).
    pub params: Option<BTreeSet<String>>,
    /// Columns of the current list row (empty = unknown), set while compiling row templates.
    pub row: Vec<String>,
}

#[derive(Debug, PartialEq)]
enum Part {
    Text(String),
    Expr { path: Vec<String>, filters: Vec<(String, Option<Value>)> },
}

enum Target {
    State(String),
    Row(String),
}

/// Compiles any JSON value: strings are parsed as templates, arrays/objects recursively.
pub fn compile(value: &Value, scope: Scope, names: &Names, path: &str, diags: &mut Diags) -> Value {
    match value {
        Value::String(s) => compile_str(s, scope, names, path, diags),
        Value::Array(items) => Value::Array(
            items.iter().enumerate().map(|(i, v)| compile(v, scope, names, &format!("{path}[{i}]"), diags)).collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter().map(|(k, v)| (k.clone(), compile(v, scope, names, &format!("{path}.{k}"), diags))).collect(),
        ),
        other => other.clone(),
    }
}

pub fn compile_str(s: &str, scope: Scope, names: &Names, path: &str, diags: &mut Diags) -> Value {
    if !s.contains("{{") {
        return Value::String(s.to_string());
    }
    let parts = match parse(s) {
        Ok(parts) => parts,
        Err(msg) => {
            let expression =
                ["?", "==", "!=", ">", "<", "+", "*", "(", "&&", "||", " - ", "'"].iter().any(|op| s.contains(op));
            diags.error("E_BIND_SYNTAX", path, msg).hint(if expression {
                "Bindings are paths with filters, not expressions. Compute the value in the query, e.g. `CASE WHEN done THEN 'check' ELSE 'circle' END AS icon`, and bind {{row.icon}}"
            } else {
                "Write bindings as {{query.column}}, {{settings.key}}, {{state.key}} or {{row.field | filter}}"
            });
            return Value::Null;
        }
    };
    let mut ir_parts: Vec<Value> = Vec::with_capacity(parts.len());
    for part in &parts {
        match part {
            Part::Text(t) => ir_parts.push(Value::String(t.clone())),
            Part::Expr { path: p, filters } => {
                let Some(target) = resolve(p, scope, names, path, diags) else {
                    return Value::Null;
                };
                if let Some((f, _)) = filters.iter().find(|(f, _)| !FILTERS.contains(&f.as_str())) {
                    diags
                        .error("E_BIND_FILTER", path, format!("unknown filter `{f}`"))
                        .hint(format!("Filters: {}", FILTERS.join(", ")));
                    return Value::Null;
                }
                ir_parts.push(ir_expr(&target, filters));
            }
        }
    }
    // Exactly one binding, no surrounding text.
    if let [Part::Expr { filters, path: p }] = parts.as_slice() {
        let single = ir_parts.pop().expect("one part");
        return match scope {
            Scope::Element if filters.is_empty() => match resolve_quiet(p, scope, names) {
                Some(Target::State(abs)) => json!({ "$state": abs }),
                _ => json!({ "$x": single }),
            },
            Scope::Element => json!({ "$x": single }),
            _ => single,
        };
    }
    let ir = json!({ "t": ir_parts });
    if scope == Scope::Element { json!({ "$x": ir }) } else { ir }
}

/// One filter → `f`/`a` scalars; a chain → `f: [..]` and, when any filter has an argument, `a: [..]` (null = none).
fn ir_expr(target: &Target, filters: &[(String, Option<Value>)]) -> Value {
    let mut obj = Map::new();
    match target {
        Target::State(p) => obj.insert("p".into(), Value::String(p.clone())),
        Target::Row(f) => obj.insert("i".into(), Value::String(f.clone())),
    };
    match filters {
        [] => {}
        [(f, arg)] => {
            obj.insert("f".into(), Value::String(f.clone()));
            if let Some(a) = arg {
                obj.insert("a".into(), a.clone());
            }
        }
        chain => {
            obj.insert("f".into(), chain.iter().map(|(f, _)| Value::String(f.clone())).collect());
            if chain.iter().any(|(_, a)| a.is_some()) {
                obj.insert("a".into(), chain.iter().map(|(_, a)| a.clone().unwrap_or(Value::Null)).collect());
            }
        }
    }
    Value::Object(obj)
}

/// A state path for bind targets such as `state.draft.reps` (form fields). `draft.reps` is accepted when `draft`
/// is a declared state key.
pub fn bind_path(s: &str, names: &Names, path: &str, diags: &mut Diags) -> Option<String> {
    let mut segs: Vec<String> =
        s.trim().trim_start_matches("{{").trim_end_matches("}}").trim().split('.').map(str::to_string).collect();
    if segs.first().is_some_and(|f| f != "state" && names.state.contains(f)) {
        segs.insert(0, "state".into());
    }
    if segs.first().map(String::as_str) != Some("state") || segs.len() < 2 {
        diags
            .error("E_BIND_TARGET", path, format!("`{s}` is not a state path"))
            .hint("Inputs bind to state, e.g. `bind: state.draft.reps`, and `state:` must declare `draft`");
        return None;
    }
    match resolve(&segs, Scope::Element, names, path, diags)? {
        Target::State(p) => Some(p),
        Target::Row(_) => None,
    }
}

fn resolve_quiet(path: &[String], scope: Scope, names: &Names) -> Option<Target> {
    resolve(path, scope, names, "", &mut Diags::default())
}

fn resolve(path: &[String], scope: Scope, names: &Names, at: &str, diags: &mut Diags) -> Option<Target> {
    let (root, rest) = path.split_first()?;
    let rest_path = rest.join("/");
    let need_key = |what: &str, set: &BTreeSet<String>, diags: &mut Diags| -> bool {
        match rest.first() {
            Some(k) if set.contains(k) => true,
            Some(k) => {
                diags
                    .error("E_BIND_UNKNOWN", at, format!("unknown {what} `{k}`"))
                    .hint(format!("Declared {what}s: {}", join_or_none(set.iter())));
                false
            }
            None => {
                diags.error("E_BIND_UNKNOWN", at, format!("`{root}` needs a key, e.g. {{{{{root}.<key>}}}}"));
                false
            }
        }
    };
    match root.as_str() {
        "row" => {
            if scope != Scope::Row {
                diags.error(
                    "E_BIND_ROW",
                    at,
                    "`row.*` is only available inside list item templates (title, caption, row_actions)",
                );
                return None;
            }
            match rest {
                [field] if !names.row.is_empty() && !names.row.contains(field) => {
                    let d = diags.error("E_BIND_COLUMN", at, format!("list rows have no column `{field}`"));
                    d.hint(format!("Row columns: {}", names.row.join(", ")));
                    None
                }
                [field] => Some(Target::Row(field.clone())),
                // Rows whose fields aren't known here (a service's, a logic value's): a path inside one,
                // `row.hourly.4.weatherDesc.0.value`, which the runtime reads as it reads `{{services.x.a.0.b}}`.
                [_, _, ..] if names.row.is_empty() => Some(Target::Row(rest.join("."))),
                _ => {
                    diags.error("E_BIND_ROW", at, "use `row.<column>`");
                    None
                }
            }
        }
        "settings" => {
            need_key("setting", &names.settings, diags).then(|| Target::State(format!("/settings/{rest_path}")))
        }
        "state" => need_key("state key", &names.state, diags).then(|| Target::State(format!("/state/{rest_path}"))),
        "services" => {
            need_key("service", &names.services, diags).then(|| Target::State(format!("/services/{rest_path}")))
        }
        "params" => match &names.params {
            Some(params) => {
                need_key("action param", params, diags).then(|| Target::State(format!("/params/{rest_path}")))
            }
            None => {
                diags.error("E_BIND_PARAMS", at, "`params.*` is only available inside action steps");
                None
            }
        },
        "app" => Some(Target::State(format!("/app/{rest_path}"))),
        // What the app's script publishes (ADR-0041): `logic.<value>`, and paths inside it (`logic.rows.0.name`).
        "logic" => need_key("logic value", &names.logic, diags).then(|| Target::State(format!("/logic/{rest_path}"))),
        // What this app is playing (ADR-0040): the R1's player, as r1d tells the page.
        "player" => match rest {
            [field] if PLAYER_FIELDS.contains(&field.as_str()) => Some(Target::State(format!("/player/{field}"))),
            _ => {
                let d = diags.error("E_BIND_PLAYER", at, "use `player.<field>`");
                d.hint(format!("Player fields: {}", PLAYER_FIELDS.join(", ")));
                None
            }
        },
        query if names.queries.contains_key(query) => {
            let cols = &names.queries[query];
            let column = |col: &String, diags: &mut Diags| {
                if !cols.is_empty() && !cols.contains(col) {
                    let counting = ["n", "count", "length", "len", "size", "total"].contains(&col.as_str());
                    diags.error("E_BIND_COLUMN", at, format!("query `{query}` has no column `{col}`")).hint(if counting {
                        format!(
                            "Columns of `{query}`: {}. To count its rows, add a query such as `{query}_count: SELECT count(*) AS n FROM …` and use {{{{{query}_count.n}}}}",
                            cols.join(", ")
                        )
                    } else {
                        format!("Columns of `{query}`: {} (alias computed columns with AS)", cols.join(", "))
                    });
                    return false;
                }
                true
            };
            let row = |s: &String| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
            match rest {
                [] => Some(Target::State(format!("/q/{query}"))),
                [n] if row(n) => Some(Target::State(format!("/q/{query}/{n}"))),
                [col] => column(col, diags).then(|| Target::State(format!("/q/{query}/0/{col}"))),
                [n, col] if row(n) => column(col, diags).then(|| Target::State(format!("/q/{query}/{n}/{col}"))),
                _ => {
                    diags.error(
                        "E_BIND_COLUMN",
                        at,
                        format!(
                            "use `{query}.<column>` (first row), `{query}.<n>.<column>` (row n, from 0) or `data: {query}` (all rows)"
                        ),
                    );
                    None
                }
            }
        }
        other => {
            diags.error("E_BIND_UNKNOWN", at, format!("unknown binding root `{other}`")).hint(format!(
                "Roots: a query name ({}), settings, state, services, params (actions), row (list items)",
                join_or_none(names.queries.keys())
            ));
            None
        }
    }
}

fn join_or_none<'a>(items: impl Iterator<Item = &'a String>) -> String {
    let v: Vec<&str> = items.map(String::as_str).collect();
    if v.is_empty() { "(none)".into() } else { v.join(", ") }
}

fn parse(s: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            parts.push(Part::Text(rest[..start].to_string()));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| format!("unclosed `{{{{` in `{s}`"))?;
        parts.push(parse_expr(after[..end].trim()).map_err(|e| format!("{e} in `{s}`"))?);
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest.to_string()));
    }
    Ok(parts)
}

fn parse_expr(expr: &str) -> Result<Part, String> {
    let mut pipes = expr.split('|').map(str::trim);
    let path_str = pipes.next().unwrap_or_default();
    if path_str.is_empty() {
        return Err("empty binding".into());
    }
    // `recent[1].title` is `recent.1.title`.
    let normalized = path_str.replace('[', ".").replace(']', "");
    let path: Vec<String> = normalized.split('.').map(|p| p.trim().to_string()).collect();
    if path.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        return Err(format!("invalid path `{path_str}`"));
    }
    let mut filters = Vec::new();
    for f in pipes {
        let (name, arg) = match f.split_once(':') {
            Some((n, a)) => (n.trim(), Some(filter_arg(a.trim()))),
            None => (f, None),
        };
        if name.is_empty() {
            return Err("empty filter".into());
        }
        filters.push((name.to_string(), arg));
    }
    Ok(Part::Expr { path, filters })
}

/// `number:1` → 1, `round:0.5` → 0.5, `default:'n/a'` → "n/a", `plural:set` → "set".
fn filter_arg(a: &str) -> Value {
    if let Ok(n) = a.parse::<i64>() {
        return json!(n);
    }
    if let Ok(n) = a.parse::<f64>() {
        return json!(n);
    }
    let unquoted = a.strip_prefix(['\'', '"']).and_then(|r| r.strip_suffix(['\'', '"'])).unwrap_or(a);
    Value::String(unquoted.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        let mut n = Names::default();
        n.queries.insert("today".into(), vec!["total".into(), "n".into()]);
        n.queries.insert("recent".into(), vec![]);
        n.settings.insert("daily_goal".into());
        n.state.insert("draft".into());
        n
    }

    fn c(s: &str, scope: Scope) -> (Value, Diags) {
        let mut d = Diags::default();
        let v = compile_str(s, scope, &names(), "x", &mut d);
        (v, d)
    }

    #[test]
    fn plain_element_binding_is_native_state() {
        assert_eq!(c("{{today.total}}", Scope::Element).0, json!({ "$state": "/q/today/0/total" }));
        assert_eq!(c("{{state.draft}}", Scope::Element).0, json!({ "$state": "/state/draft" }));
        assert_eq!(c("{{recent}}", Scope::Element).0, json!({ "$state": "/q/recent" }));
    }

    #[test]
    fn templates_and_filters() {
        assert_eq!(
            c("{{today.n}} sets · goal {{settings.daily_goal}}", Scope::Element).0,
            json!({ "$x": { "t": [{ "p": "/q/today/0/n" }, " sets · goal ", { "p": "/settings/daily_goal" }] } })
        );
        assert_eq!(c("{{row.at | time}}", Scope::Row).0, json!({ "i": "at", "f": "time" }));
        assert_eq!(
            c("{{today.total | number:1}}", Scope::Element).0,
            json!({ "$x": { "p": "/q/today/0/total", "f": "number", "a": 1 } })
        );
        assert_eq!(c("plain text", Scope::Element).0, json!("plain text"));
        assert_eq!(
            c("{{row.at | time | default:'—'}}", Scope::Row).0,
            json!({ "i": "at", "f": ["time", "default"], "a": [null, "—"] })
        );
        assert_eq!(
            c("{{today.n | round | plural}}", Scope::Step).0,
            json!({ "p": "/q/today/0/n", "f": ["round", "plural"] })
        );
        assert_eq!(c("{{today.1.total}}", Scope::Element).0, json!({ "$state": "/q/today/1/total" }));
        assert_eq!(c("{{today[2].n}}", Scope::Step).0, json!({ "p": "/q/today/2/n" }));
        assert_eq!(c("{{today.0}}", Scope::Step).0, json!({ "p": "/q/today/0" }));
    }

    #[test]
    fn errors_are_located_and_hinted() {
        let (_, d) = c("{{today.totl}}", Scope::Element);
        assert_eq!(d.items[0].code, "E_BIND_COLUMN");
        assert!(d.items[0].hint.as_deref().unwrap().contains("total"));
        let (_, d) = c("{{row.reps}}", Scope::Element);
        assert_eq!(d.items[0].code, "E_BIND_ROW");
        let (_, d) = c("{{nope.x}}", Scope::Element);
        assert_eq!(d.items[0].code, "E_BIND_UNKNOWN");
        let (_, d) = c("{{today.total | sparkle}}", Scope::Element);
        assert_eq!(d.items[0].code, "E_BIND_FILTER");
        let (_, d) = c("{{ today.total > 5 ? 'a' : 'b' }}", Scope::Element);
        assert!(d.items[0].hint.as_deref().unwrap().starts_with("Bindings are paths"));
    }
}
