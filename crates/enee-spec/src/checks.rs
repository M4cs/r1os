//! Runs a compiled app's `checks` against a scratch database (native only): the smoke test of
//! docs/specs/enee-spec.md §4.12. `sql` steps execute for real; `set` steps update a state model; other steps
//! (toast, notify, navigate, …) are no-ops here. Value IR is evaluated with the runtime's rules (`value.ts`).
//!
//! [`Sandbox`] gives the same scratch database to other callers: the eval harness's functional checklists run an
//! app's actions and read its queries with it.

use rusqlite::types::Value as Sql;
use rusqlite::{Connection, ToSql};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::compiled::CompiledApp;
use crate::sql::{Clock, register_helpers};

#[derive(Debug, Serialize)]
pub struct CheckResult {
    /// Index into `checks`.
    pub index: usize,
    pub ok: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

/// Runs every `do`/`expect` check on its own fresh database (`render` checks need a UI and are skipped).
pub fn run_checks(app: &CompiledApp) -> Vec<CheckResult> {
    app.checks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.get("expect").is_some())
        .map(|(index, check)| {
            let failures = run(app, check).unwrap_or_else(|e| vec![e]);
            CheckResult { index, ok: failures.is_empty(), failures }
        })
        .collect()
}

fn run(app: &CompiledApp, check: &Value) -> Result<Vec<String>, String> {
    let mut env = Env::fresh(app)?;
    let mut failures = Vec::new();
    let calls = match check.get("do") {
        Some(Value::Array(a)) => a.clone(),
        Some(d) => vec![d.clone()],
        None => Vec::new(),
    };
    for (i, call) in calls.iter().enumerate() {
        // Real calls are seconds apart: keep `= now` timestamps distinct so `ORDER BY at DESC` is deterministic.
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if let Err(e) = env.run_action(call) {
            failures.push(e);
        }
    }
    if let Some(Value::Object(expect)) = check.get("expect") {
        for (path, want) in expect {
            match env.read(path, &Map::new()) {
                Ok(got) if same(&got, want) => {}
                Ok(got) => failures.push(format!("`{path}` is {got}, expected {want}")),
                Err(e) => failures.push(format!("`{path}`: {e}")),
            }
        }
    }
    Ok(failures)
}

/// A compiled app on its own scratch database, with its state: run its actions and read its queries as a check would.
pub struct Sandbox<'a> {
    env: Env<'a>,
}

impl<'a> Sandbox<'a> {
    pub fn new(app: &'a CompiledApp) -> Result<Self, String> {
        Ok(Sandbox { env: Env::fresh(app)? })
    }

    /// Runs `action` with `args` (plain values), as a check's `do` would.
    pub fn run(&mut self, action: &str, args: Value) -> Result<(), String> {
        self.env.run_action(&json!({ "action": action, "args": args }))
    }

    /// A state path's value: `/q/<query>/…`, `/settings/<key>` or `/state/…`.
    pub fn read(&self, path: &str) -> Result<Value, String> {
        self.env.read(path, &Map::new())
    }

    /// Sets a `/state/…` path, as an input bound to it would.
    pub fn set(&mut self, path: &str, value: Value) {
        set_pointer(&mut self.env.state, path.strip_prefix("/state").unwrap_or(path), value);
    }

    /// Runs SQL of the caller's own on the scratch database (what the app's queries don't show: row counts, rows).
    pub fn select(&self, sql: &str) -> Result<Vec<Map<String, Value>>, String> {
        self.env.exec(sql, &[]).map_err(|e| e.to_string())
    }
}

struct Env<'a> {
    app: &'a CompiledApp,
    conn: Connection,
    state: Value,
}

impl<'a> Env<'a> {
    /// The app's schema on a new in-memory database, and its initial state.
    fn fresh(app: &'a CompiledApp) -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        register_helpers(&conn, Clock::system()).map_err(|e| e.to_string())?;
        // As in the app host (enee-data): `ref` columns are enforced, and deletes cascade.
        conn.pragma_update(None, "foreign_keys", true).map_err(|e| e.to_string())?;
        for stmt in &app.ddl {
            conn.execute_batch(stmt).map_err(|e| format!("schema: {e}"))?;
        }
        Ok(Env { app, conn, state: Value::Object(app.state.clone()) })
    }
}

impl Env<'_> {
    fn run_action(&mut self, call: &Value) -> Result<(), String> {
        let name = call.get("action").and_then(Value::as_str).unwrap_or_default();
        let action = self.app.actions.get(name).ok_or_else(|| format!("no action `{name}`"))?;
        let args = match self.eval(call.get("args").unwrap_or(&Value::Null), &Map::new())? {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        if let Some(Value::Object(params)) = action.get("params") {
            for (p, def) in params {
                if def.get("required") == Some(&json!(true)) && args.get(p).is_none_or(Value::is_null) {
                    return Err(format!("action `{name}`: missing param `{p}`"));
                }
            }
        }
        for (i, step) in action.get("steps").and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(sql) = step.get("sql").and_then(Value::as_str) {
                let mut bound: Vec<(String, Sql)> = Vec::new();
                for p in step.get("params").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                    let v = match args.get(p) {
                        Some(v) => v.clone(),
                        None => match self.state.get(p) {
                            Some(v) => v.clone(),
                            None => self.setting(p).unwrap_or(Value::Null),
                        },
                    };
                    bound.push((format!(":{p}"), to_sql(&v)));
                }
                self.exec(sql, &bound).map_err(|e| format!("action `{name}` step {i}: {e}"))?;
            } else if let Some(Value::Object(set)) = step.get("set") {
                for (path, v) in set {
                    let v = self.eval(v, &args)?;
                    set_pointer(&mut self.state, path.strip_prefix("/state").unwrap_or(path), v);
                }
            }
        }
        Ok(())
    }

    fn exec(&self, sql: &str, bound: &[(String, Sql)]) -> rusqlite::Result<Vec<Map<String, Value>>> {
        let mut stmt = self.conn.prepare(sql)?;
        let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
        let params: Vec<(&str, &dyn ToSql)> = bound.iter().map(|(k, v)| (k.as_str(), v as &dyn ToSql)).collect();
        let mut rows = stmt.query(params.as_slice())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut obj = Map::new();
            for (i, n) in names.iter().enumerate() {
                obj.insert(n.clone(), to_json(row.get::<_, Sql>(i)?));
            }
            out.push(obj);
        }
        Ok(out)
    }

    fn setting(&self, key: &str) -> Option<Value> {
        if !self.app.settings.contains_key(key) {
            return None;
        }
        let rows = self.exec(&format!("SELECT {key} FROM settings WHERE id = 1"), &[]).ok()?;
        rows.into_iter().next().and_then(|mut r| r.remove(key))
    }

    fn query(&self, name: &str) -> Result<Value, String> {
        let q = self.app.queries.iter().find(|q| q.name == name).ok_or_else(|| format!("no query `{name}`"))?;
        let mut bound = Vec::new();
        for (k, v) in &q.params {
            bound.push((format!(":{k}"), to_sql(&self.eval(v, &Map::new())?)));
        }
        let rows = self.exec(&q.sql, &bound).map_err(|e| format!("query `{name}`: {e}"))?;
        Ok(Value::Array(rows.into_iter().map(Value::Object).collect()))
    }

    /// Absolute state paths: `/q/<query>/…`, `/settings/<key>`, `/state/…`, `/params/<name>` (inside steps).
    fn read(&self, path: &str, params: &Map<String, Value>) -> Result<Value, String> {
        let segs: Vec<&str> = path.trim_start_matches('/').splitn(3, '/').collect();
        let rest = |from: usize| format!("/{}", segs[from..].join("/"));
        let found = match segs.as_slice() {
            ["q", name, ..] => {
                let rows = self.query(name)?;
                if segs.len() > 2 { rows.pointer(&rest(2)).cloned() } else { Some(rows) }
            }
            ["settings", key, ..] => self.setting(key),
            ["state", ..] => self.state.pointer(&rest(1)).cloned(),
            ["params", key, ..] => params.get(*key).cloned(),
            _ => return Err(format!("unknown root in `{path}`")),
        };
        Ok(found.unwrap_or(Value::Null))
    }

    /// The Value IR (docs/specs/compiled-app.md §3); filters only matter for display and are ignored here.
    fn eval(&self, v: &Value, params: &Map<String, Value>) -> Result<Value, String> {
        let only = |m: &Map<String, Value>, keys: &[&str]| m.keys().all(|k| keys.contains(&k.as_str()));
        Ok(match v {
            Value::Array(items) => Value::Array(items.iter().map(|x| self.eval(x, params)).collect::<Result<_, _>>()?),
            Value::Object(m) => {
                if let Some(Value::String(p)) = m.get("p")
                    && only(m, &["p", "f", "a"])
                {
                    self.read(p, params)?
                } else if m.get("i").is_some_and(Value::is_string) && only(m, &["i", "f", "a"]) {
                    Value::Null
                } else if let Some(Value::Array(parts)) = m.get("t")
                    && only(m, &["t"])
                {
                    let mut s = String::new();
                    for part in parts {
                        s.push_str(&to_text(&self.eval(part, params)?));
                    }
                    Value::String(s)
                } else if let Some(Value::String(p)) = m.get("$state")
                    && only(m, &["$state"])
                {
                    self.read(p, params)?
                } else if let Some(x) = m.get("$x")
                    && only(m, &["$x"])
                {
                    self.eval(x, params)?
                } else {
                    let mut out = Map::new();
                    for (k, x) in m {
                        out.insert(k.clone(), self.eval(x, params)?);
                    }
                    Value::Object(out)
                }
            }
            other => other.clone(),
        })
    }
}

fn set_pointer(root: &mut Value, pointer: &str, v: Value) {
    let mut cur = root;
    let segs: Vec<&str> = pointer.trim_start_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    let Some((last, parents)) = segs.split_last() else { return };
    for seg in parents {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        cur = cur.as_object_mut().expect("object").entry(seg.to_string()).or_insert(Value::Object(Map::new()));
    }
    if !cur.is_object() {
        *cur = Value::Object(Map::new());
    }
    cur.as_object_mut().expect("object").insert(last.to_string(), v);
}

fn to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn to_sql(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(|| Sql::Real(n.as_f64().unwrap_or_default()), Sql::Integer),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

fn to_json(v: Sql) -> Value {
    match v {
        Sql::Null | Sql::Blob(_) => Value::Null,
        Sql::Integer(i) => json!(i),
        Sql::Real(f) => json!(f),
        Sql::Text(s) => Value::String(s),
    }
}

/// Numbers compare within a relative 1e-6 (SQL float math), booleans match SQLite's 0/1, an expected object
/// matches when its keys do (extra columns are fine), and arrays match element by element.
fn same(got: &Value, want: &Value) -> bool {
    let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= 1e-6 * b.abs().max(1.0));
    match (got, want) {
        (Value::Number(a), Value::Number(b)) => close(a.as_f64(), b.as_f64().unwrap_or(f64::NAN)),
        (Value::Number(a), Value::Bool(b)) | (Value::Bool(b), Value::Number(a)) => {
            close(a.as_f64(), f64::from(u8::from(*b)))
        }
        (Value::Object(g), Value::Object(w)) => w.iter().all(|(k, wv)| g.get(k).is_some_and(|gv| same(gv, wv))),
        (Value::Array(g), Value::Array(w)) => g.len() == w.len() && g.iter().zip(w).all(|(a, b)| same(a, b)),
        _ => got == want,
    }
}

#[cfg(test)]
mod tests {
    use super::same;
    use serde_json::json;

    #[test]
    fn expectations_are_lenient_where_sql_is_imprecise() {
        assert!(same(&json!(7.999999992549419), &json!(8)));
        assert!(!same(&json!(7.9), &json!(8)));
        assert!(same(&json!(1), &json!(true)));
        assert!(same(&json!([{ "id": 1, "name": "Read", "streak": 1 }]), &json!([{ "name": "Read", "streak": 1 }])));
        assert!(!same(&json!([{ "name": "Read" }, { "name": "Run" }]), &json!([{ "name": "Read" }])));
    }
}
