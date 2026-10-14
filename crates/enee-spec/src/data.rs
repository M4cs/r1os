//! `data:` (tables) and `settings:` → table definitions and idempotent DDL (docs/specs/compiled-app.md §6).

use serde_json::{Map, Value, json};

use crate::compiled::{Column, Table};
use crate::diag::Diags;
use crate::util::{child, duration_ms, index, is_clock_time, is_ident, kind, list, sql_str, suggest};

pub const NOW_SQL: &str = "(strftime('%Y-%m-%dT%H:%M:%fZ','now'))";
const TODAY_SQL: &str = "(date('now','localtime'))";
const IMPLICIT: [&str; 3] = ["id", "created_at", "updated_at"];
/// Words SQLite rejects as bare table or column names (derived from SQLite itself; see the `sql` tests).
pub const SQL_RESERVED: [&str; 61] = [
    "add",
    "all",
    "alter",
    "and",
    "as",
    "autoincrement",
    "between",
    "case",
    "cast",
    "check",
    "collate",
    "commit",
    "constraint",
    "create",
    "default",
    "deferrable",
    "delete",
    "distinct",
    "drop",
    "else",
    "escape",
    "except",
    "exists",
    "foreign",
    "from",
    "group",
    "having",
    "if",
    "in",
    "index",
    "insert",
    "intersect",
    "into",
    "is",
    "isnull",
    "join",
    "limit",
    "not",
    "nothing",
    "notnull",
    "null",
    "on",
    "or",
    "order",
    "primary",
    "raise",
    "references",
    "returning",
    "select",
    "set",
    "table",
    "then",
    "to",
    "transaction",
    "union",
    "unique",
    "update",
    "using",
    "values",
    "when",
    "where",
];

/// Snake-case, not an SQL keyword. Reports and returns false otherwise.
fn sql_name(name: &str, what: &str, path: &str, diags: &mut Diags) -> bool {
    if !is_ident(name) {
        diags.error("E_NAME", path, format!("{what} name `{name}` must be snake_case"));
        return false;
    }
    if SQL_RESERVED.contains(&name) {
        let d = diags.error("E_NAME_RESERVED", path, format!("`{name}` is an SQL keyword and can't name a {what}"));
        d.hint("Pick a more specific name, e.g. `order` → `position`, `group` → `group_name`, `on` → `day`");
        return false;
    }
    true
}
const FIELD_TYPES: [&str; 11] =
    ["int", "real", "text", "bool", "datetime", "date", "time", "duration", "json", "enum(a|b)", "ref(table)"];
const SETTING_TYPES: [&str; 11] =
    ["int", "real", "text", "bool", "duration", "time", "time_range", "date", "enum", "color", "hotkey"];

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Int,
    Real,
    Text,
    Bool,
    Datetime,
    Date,
    Time,
    Duration,
    Json,
    Enum(Vec<String>),
    Ref(String),
}

impl Ty {
    pub fn name(&self) -> &'static str {
        match self {
            Ty::Int => "int",
            Ty::Real => "real",
            Ty::Text => "text",
            Ty::Bool => "bool",
            Ty::Datetime => "datetime",
            Ty::Date => "date",
            Ty::Time => "time",
            Ty::Duration => "duration",
            Ty::Json => "json",
            Ty::Enum(_) => "enum",
            Ty::Ref(_) => "ref",
        }
    }

    fn sql(&self) -> &'static str {
        match self {
            Ty::Int | Ty::Bool | Ty::Duration | Ty::Ref(_) => "INTEGER",
            Ty::Real => "REAL",
            _ => "TEXT",
        }
    }

    /// Parses a type name; aliases are accepted with a warning by the caller.
    pub fn parse(s: &str) -> Result<(Ty, Option<&'static str>), String> {
        let s = s.trim();
        if let Some(inner) = s.strip_prefix("enum(").and_then(|r| r.strip_suffix(')')) {
            let opts: Vec<String> = inner.split('|').map(|o| o.trim().to_string()).collect();
            if opts.iter().any(String::is_empty) {
                return Err(format!("empty option in `{s}`"));
            }
            return Ok((Ty::Enum(opts), None));
        }
        if let Some(inner) = s.strip_prefix("ref(").and_then(|r| r.strip_suffix(')')) {
            return Ok((Ty::Ref(inner.trim().to_string()), None));
        }
        let (ty, alias) = match s {
            "int" => (Ty::Int, None),
            "real" => (Ty::Real, None),
            "text" => (Ty::Text, None),
            "bool" => (Ty::Bool, None),
            "datetime" => (Ty::Datetime, None),
            "date" => (Ty::Date, None),
            "time" => (Ty::Time, None),
            "duration" => (Ty::Duration, None),
            "json" => (Ty::Json, None),
            "integer" => (Ty::Int, Some("int")),
            "float" | "number" | "double" => (Ty::Real, Some("real")),
            "string" | "str" => (Ty::Text, Some("text")),
            "boolean" => (Ty::Bool, Some("bool")),
            "timestamp" => (Ty::Datetime, Some("datetime")),
            _ => return Err(format!("unknown type `{s}`")),
        };
        Ok((ty, alias))
    }
}

/// A parsed `type[!|?] [= default]` field.
struct Field {
    ty: Ty,
    not_null: bool,
    /// Canonical JSON default (`"now"`, `3600`, `true`, …) and its SQL expression.
    default: Option<(Value, String)>,
}

pub struct Data {
    pub tables: Vec<Table>,
    pub ddl: Vec<String>,
}

pub fn compile_tables(data: Option<&Value>, diags: &mut Diags) -> Data {
    let mut out = Data { tables: Vec::new(), ddl: Vec::new() };
    let Some(data) = data else { return out };
    let Some(map) = data.as_object() else {
        diags.error("E_SHAPE", "data", format!("`data` must be a map of tables, got {}", kind(data)));
        return out;
    };
    let table_names: Vec<&str> = map.keys().map(String::as_str).filter(|k| *k != "_indexes").collect();
    // (YAML path, spec, table that unqualified entries refer to)
    let mut indexes: Vec<(String, &Value, Option<String>)> = Vec::new();
    for (name, fields) in map {
        let path = child("data", name);
        if name == "_indexes" {
            indexes.push((path, fields, None));
            continue;
        }
        if !sql_name(name, "table", &path, diags) {
            continue;
        }
        if name == "settings" || name.starts_with("sqlite_") {
            diags.error("E_NAME_RESERVED", &path, format!("`{name}` is reserved")).hint(if name == "settings" {
                "User-tunable values go in the top-level `settings:` section; they are readable in SQL as `SELECT x FROM settings`"
            } else {
                "Pick another table name"
            });
            continue;
        }
        let Some(fields) = fields.as_object() else {
            diags.error(
                "E_SHAPE",
                &path,
                format!("table `{name}` must be a map of `column: type`, got {}", kind(fields)),
            );
            continue;
        };
        let mut cols = vec![Column {
            name: "id".into(),
            ty: "int".into(),
            not_null: true,
            primary_key: true,
            ..Column::default()
        }];
        let mut defs = vec!["id INTEGER PRIMARY KEY".to_string()];
        for (col, spec) in fields {
            let cpath = child(&path, col);
            // Indexes declared inside their table are accepted as well.
            if col == "_indexes" {
                indexes.push((cpath, spec, Some(name.clone())));
                continue;
            }
            if IMPLICIT.contains(&col.as_str()) {
                diags
                    .warn("W_IMPLICIT_COLUMN", &cpath, format!("`{col}` is implicit and was ignored"))
                    .hint("Every table already has id, created_at and updated_at");
                continue;
            }
            if !sql_name(col, "column", &cpath, diags) {
                continue;
            }
            let Some(field) = parse_field(spec, &cpath, diags) else { continue };
            if let Ty::Ref(target) = &field.ty
                && !table_names.contains(&target.as_str())
            {
                let d = diags.error("E_REF_TABLE", &cpath, format!("`ref({target})`: no table `{target}`"));
                d.hint(format!("Tables: {}", list(table_names.iter().copied())));
                continue;
            }
            let mut def = format!("{col} {}", field.ty.sql());
            if field.not_null {
                def.push_str(" NOT NULL");
            }
            if let Some((_, sql)) = &field.default {
                def.push_str(&format!(" DEFAULT {sql}"));
            }
            match &field.ty {
                Ty::Enum(opts) => {
                    let opts: Vec<String> = opts.iter().map(|o| sql_str(o)).collect();
                    def.push_str(&format!(" CHECK ({col} IN ({}))", opts.join(",")));
                }
                Ty::Ref(target) => {
                    let on_delete = if field.not_null { "CASCADE" } else { "SET NULL" };
                    def.push_str(&format!(" REFERENCES {target}(id) ON DELETE {on_delete}"));
                }
                _ => {}
            }
            defs.push(def);
            cols.push(Column {
                name: col.clone(),
                ty: field.ty.name().into(),
                not_null: field.not_null,
                primary_key: false,
                default: field.default.map(|(v, _)| v),
                options: match &field.ty {
                    Ty::Enum(opts) => opts.clone(),
                    _ => Vec::new(),
                },
                references: match &field.ty {
                    Ty::Ref(target) => Some(target.clone()),
                    _ => None,
                },
            });
        }
        for stamp in ["created_at", "updated_at"] {
            defs.push(format!("{stamp} TEXT NOT NULL DEFAULT {NOW_SQL}"));
            cols.push(Column {
                name: stamp.into(),
                ty: "datetime".into(),
                not_null: true,
                default: Some(json!("now")),
                ..Column::default()
            });
        }
        out.ddl.push(format!("CREATE TABLE IF NOT EXISTS {name} ({})", defs.join(", ")));
        out.ddl.push(format!(
            "CREATE TRIGGER IF NOT EXISTS {name}_touch AFTER UPDATE ON {name} FOR EACH ROW WHEN NEW.updated_at IS OLD.updated_at \
             BEGIN UPDATE {name} SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END"
        ));
        out.tables.push(Table { name: name.clone(), columns: cols });
    }
    for (path, spec, table) in indexes {
        compile_indexes(spec, &path, table.as_deref(), &out.tables, &mut out.ddl, diags);
    }
    out
}

/// `table(col, …)`, `unique table(col, …)`, or inside a table `(col, …)` / `unique(col, …)`.
fn compile_indexes(
    spec: &Value,
    path: &str,
    own_table: Option<&str>,
    tables: &[Table],
    ddl: &mut Vec<String>,
    diags: &mut Diags,
) {
    let Some(items) = spec.as_array() else {
        diags.error("E_SHAPE", path, "`_indexes` must be a list like `[ sets(at), \"unique ticks(habit_id, day)\" ]`");
        return;
    };
    // An unquoted `[ ticks(habit_id, day) ]` reaches us as two items split at the comma; rejoin them.
    let mut joined: Vec<(String, String)> = Vec::new();
    let mut open: Option<(String, String)> = None;
    for (i, item) in items.iter().enumerate() {
        let text = item.as_str().map(str::trim).unwrap_or_default().to_string();
        match open.take() {
            Some((at, head)) => {
                let merged = format!("{head}, {text}");
                if text.contains(')') {
                    diags
                        .warn("W_AUTOFIX", &at, format!("read as `{merged}`"))
                        .hint("Quote multi-column indexes: \"ticks(habit_id, day)\"");
                    joined.push((at, merged));
                } else {
                    open = Some((at, merged));
                }
            }
            None if text.contains('(') && !text.contains(')') => open = Some((index(path, i), text)),
            None => joined.push((index(path, i), text)),
        }
    }
    if let Some(unclosed) = open {
        joined.push(unclosed);
    }
    for (ipath, text) in joined {
        let (unique, body) = match text.strip_prefix("unique") {
            Some(rest) if rest.starts_with([' ', '(']) => (true, rest.trim()),
            _ => (false, text.trim()),
        };
        let parsed = body.strip_suffix(')').and_then(|b| b.split_once('(')).and_then(|(table, cols)| {
            let table = match table.trim() {
                "" => own_table?.to_string(),
                t => t.to_string(),
            };
            Some((table, cols.split(',').map(|c| c.trim().to_string()).collect::<Vec<_>>()))
        });
        let Some((table, cols)) = parsed else {
            diags.error("E_SHAPE", &ipath, "index must look like `table(col, col)` or `unique table(col, col)`");
            continue;
        };
        let Some(t) = tables.iter().find(|t| t.name == table) else {
            diags.error("E_REF_TABLE", &ipath, format!("no table `{table}`"));
            continue;
        };
        if let Some(bad) = cols.iter().find(|c| !t.columns.iter().any(|tc| &tc.name == *c)) {
            // An expression (`local_date(at)`): index the column it reads; queries still use the expression.
            if bad.contains('(') {
                let inside: Vec<&str> = bad
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .filter(|w| t.columns.iter().any(|tc| tc.name == *w))
                    .collect();
                let instead = inside.first().map_or("the column it reads".to_string(), |c| format!("`{table}({c})`"));
                diags
                    .error("E_REF_COLUMN", &ipath, format!("an index lists columns, not expressions like `{bad})`"))
                    .hint(format!("Index {instead} instead; queries can still filter on the expression"));
                continue;
            }
            let names = t.columns.iter().map(|c| c.name.as_str());
            diags
                .error("E_REF_COLUMN", &ipath, format!("table `{table}` has no column `{bad}`"))
                .hint(format!("Columns: {}", list(names)));
            continue;
        }
        let (kind, suffix) = if unique { ("UNIQUE INDEX", "uniq") } else { ("INDEX", "idx") };
        ddl.push(format!(
            "CREATE {kind} IF NOT EXISTS {table}_{}_{suffix} ON {table}({})",
            cols.join("_"),
            cols.join(", ")
        ));
    }
}

fn parse_field(spec: &Value, path: &str, diags: &mut Diags) -> Option<Field> {
    // Long form: { type: "int!", default: 5, renamed_from: old }.
    let (decl, long_default) = match spec {
        Value::String(s) => (s.clone(), None),
        Value::Object(m) => {
            for k in m.keys() {
                if !["type", "default", "renamed_from"].contains(&k.as_str()) {
                    diags
                        .warn("W_KEY_UNKNOWN", child(path, k), format!("unknown key `{k}` ignored"))
                        .hint("Field keys: type, default, renamed_from");
                }
            }
            let Some(ty) = m.get("type").and_then(Value::as_str) else {
                let d =
                    diags.error("E_FIELD", path, "long-form field needs `type`, e.g. `{ type: \"int!\", default: 0 }`");
                // `{ habit_id: "ref(habits)!", day: date }` is a table nested inside another one.
                if !m.is_empty() && m.values().all(Value::is_string) {
                    d.hint(
                        "This looks like a table nested inside another table. Every table goes directly under `data:`",
                    );
                }
                return None;
            };
            (ty.to_string(), m.get("default").cloned())
        }
        other => {
            diags.error(
                "E_FIELD",
                path,
                format!("field must be a type string like `int!` or `text = 'x'`, got {}", kind(other)),
            );
            return None;
        }
    };
    let (ty_part, default_part) = split_default(&decl);
    let mut ty_str = ty_part.trim();
    let mut not_null = false;
    if let Some(t) = ty_str.strip_suffix('!') {
        not_null = true;
        ty_str = t.trim_end();
    } else if let Some(t) = ty_str.strip_suffix('?') {
        ty_str = t.trim_end();
    }
    let ty = match Ty::parse(ty_str) {
        Ok((ty, alias)) => {
            if let Some(canon) = alias {
                diags.warn("W_AUTOFIX", path, format!("type `{ty_str}` read as `{canon}`"));
            }
            ty
        }
        Err(msg) => {
            let d = diags.error("E_FIELD_TYPE", path, msg);
            d.hint(format!(
                "Field types: {}; suffix ! = NOT NULL, ? = nullable; `= <default>`",
                FIELD_TYPES.join(", ")
            ));
            return None;
        }
    };
    if let Ty::Enum(opts) = &ty
        && let Some(bad) = opts.iter().find(|o| o.contains('\''))
    {
        diags.error("E_FIELD_TYPE", path, format!("enum option `{bad}` may not contain quotes"));
        return None;
    }
    let raw_default = match (default_part, long_default) {
        (Some(d), _) => Some(Value::String(d.trim().to_string())),
        (None, d) => d,
    };
    let default = match raw_default {
        None => None,
        Some(d) => Some(field_default(&ty, &d, path, diags)?),
    };
    Some(Field { ty, not_null, default })
}

/// Splits `enum(a|b) = a` at the first `=` outside parentheses.
fn split_default(s: &str) -> (&str, Option<&str>) {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '=' if depth == 0 => return (&s[..i], Some(&s[i + 1..])),
            _ => {}
        }
    }
    (s, None)
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    for q in ['\'', '"'] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Canonical JSON + SQL literal for a field default. Defaults from the short form arrive as strings.
fn field_default(ty: &Ty, d: &Value, path: &str, diags: &mut Diags) -> Option<(Value, String)> {
    let text = match d {
        Value::String(s) => unquote(s).to_string(),
        other => other.to_string(),
    };
    let bad = |diags: &mut Diags, want: &str| {
        diags.error("E_FIELD_DEFAULT", path, format!("default `{text}` is not a valid {want}"));
        None
    };
    match ty {
        Ty::Int => match text.parse::<i64>() {
            Ok(n) => Some((json!(n), n.to_string())),
            Err(_) => bad(diags, "int"),
        },
        Ty::Real => match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Some((json!(n), n.to_string())),
            _ => bad(diags, "real"),
        },
        Ty::Bool => match text.as_str() {
            "true" | "1" => Some((json!(true), "1".into())),
            "false" | "0" => Some((json!(false), "0".into())),
            _ => bad(diags, "bool (true/false)"),
        },
        Ty::Text | Ty::Time | Ty::Json => {
            if *ty == Ty::Time && !is_clock_time(&text) {
                return bad(diags, "time (HH:MM)");
            }
            if *ty == Ty::Json && serde_json::from_str::<Value>(&text).is_err() {
                return bad(diags, "JSON value");
            }
            Some((Value::String(text.clone()), sql_str(&text)))
        }
        Ty::Datetime => match text.as_str() {
            "now" => Some((json!("now"), NOW_SQL.into())),
            t if t.len() >= 10 && t.as_bytes()[4] == b'-' => Some((json!(t), sql_str(t))),
            _ => bad(diags, "datetime (`now` or an ISO timestamp)"),
        },
        Ty::Date => match text.as_str() {
            "today" => Some((json!("today"), TODAY_SQL.into())),
            t if t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' => Some((json!(t), sql_str(t))),
            _ => bad(diags, "date (`today` or YYYY-MM-DD)"),
        },
        Ty::Duration => match duration_ms(&Value::String(text.clone())) {
            Some(ms) if ms % 1000 == 0 => Some((json!(ms / 1000), (ms / 1000).to_string())),
            _ => bad(diags, "duration in whole seconds (e.g. 90s, 25m, 1h)"),
        },
        Ty::Enum(opts) => {
            if opts.contains(&text) {
                Some((Value::String(text.clone()), sql_str(&text)))
            } else {
                let d =
                    diags.error("E_FIELD_DEFAULT", path, format!("default `{text}` is not one of the enum options"));
                d.hint(format!("Options: {}", opts.join(", ")));
                None
            }
        }
        Ty::Ref(_) => {
            diags.error("E_FIELD_DEFAULT", path, "ref fields cannot have a default");
            None
        }
    }
}

pub struct Settings {
    pub defs: Map<String, Value>,
    pub ddl: Vec<String>,
}

const SETTING_KEYS: [&str; 6] = ["type", "default", "label", "min", "max", "options"];

pub fn compile_settings(settings: Option<&Value>, diags: &mut Diags) -> Settings {
    let mut out = Settings { defs: Map::new(), ddl: Vec::new() };
    let Some(settings) = settings else { return out };
    let Some(map) = settings.as_object() else {
        diags.error("E_SHAPE", "settings", format!("`settings` must be a map, got {}", kind(settings)));
        return out;
    };
    let mut cols = Vec::new();
    let mut values = Vec::new();
    for (key, def) in map {
        let path = child("settings", key);
        if key == "id" {
            diags.error("E_NAME_RESERVED", &path, "`id` can't name a setting");
            continue;
        }
        if !sql_name(key, "setting", &path, diags) {
            continue;
        }
        let Some(def) = def.as_object() else {
            diags.error(
                "E_SHAPE",
                &path,
                "setting must be a map like `{ type: int, default: 100, label: Daily goal }`",
            );
            continue;
        };
        for k in def.keys() {
            if !SETTING_KEYS.contains(&k.as_str()) {
                let d = diags.warn("W_KEY_UNKNOWN", child(&path, k), format!("unknown key `{k}` ignored"));
                d.hint(format!("Setting keys: {}", SETTING_KEYS.join(", ")));
            }
        }
        let ty = def.get("type").and_then(Value::as_str).unwrap_or_default();
        if !SETTING_TYPES.contains(&ty) {
            let d = diags.error("E_SETTING_TYPE", child(&path, "type"), format!("unknown setting type `{ty}`"));
            d.hint(match suggest(ty, SETTING_TYPES) {
                Some(s) => format!("Did you mean `{s}`? Types: {}", SETTING_TYPES.join(", ")),
                None => format!("Types: {}", SETTING_TYPES.join(", ")),
            });
            continue;
        }
        let options: Option<Vec<String>> = def
            .get("options")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|o| o.as_str().map(str::to_string).unwrap_or_else(|| o.to_string())).collect());
        if ty == "enum" && options.as_ref().is_none_or(Vec::is_empty) {
            diags.error("E_SETTING", &path, "enum settings need `options: [a, b]`");
            continue;
        }
        let Some(default) = def.get("default") else {
            diags.error("E_SETTING", &path, "every setting needs a `default`");
            continue;
        };
        let dpath = child(&path, "default");
        let Some((value, sql)) = setting_default(ty, default, options.as_deref(), &dpath, diags) else { continue };
        for bound in ["min", "max"] {
            if let Some(b) = def.get(bound)
                && !b.is_number()
            {
                diags.error("E_SETTING", child(&path, bound), format!("`{bound}` must be a number"));
            }
        }
        let mut canon = Map::new();
        canon.insert("type".into(), json!(ty));
        canon.insert("default".into(), value);
        for k in ["label", "min", "max"] {
            if let Some(v) = def.get(k) {
                canon.insert(k.into(), v.clone());
            }
        }
        if let Some(opts) = options {
            canon.insert("options".into(), json!(opts));
        }
        let sql_ty = match ty {
            "int" | "bool" | "duration" => "INTEGER",
            "real" => "REAL",
            _ => "TEXT",
        };
        cols.push((key.clone(), sql_ty));
        values.push(sql);
        out.defs.insert(key.clone(), Value::Object(canon));
    }
    if !cols.is_empty() {
        let col_defs: Vec<String> = cols.iter().map(|(k, t)| format!("{k} {t}")).collect();
        let names: Vec<&str> = cols.iter().map(|(k, _)| k.as_str()).collect();
        out.ddl.push(format!(
            "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), {})",
            col_defs.join(", ")
        ));
        out.ddl.push(format!(
            "INSERT OR IGNORE INTO settings (id, {}) VALUES (1, {})",
            names.join(", "),
            values.join(", ")
        ));
    }
    out
}

fn setting_default(
    ty: &str,
    d: &Value,
    options: Option<&[String]>,
    path: &str,
    diags: &mut Diags,
) -> Option<(Value, String)> {
    let mismatch = |diags: &mut Diags, want: &str| {
        diags.error("E_SETTING_DEFAULT", path, format!("default must be {want}, got `{d}`"));
        None
    };
    match ty {
        "int" => match d.as_i64() {
            Some(n) => Some((json!(n), n.to_string())),
            None => mismatch(diags, "an integer"),
        },
        "real" => match d.as_f64() {
            Some(n) => Some((json!(n), n.to_string())),
            None => mismatch(diags, "a number"),
        },
        "bool" => match d.as_bool() {
            Some(b) => Some((json!(b), if b { "1" } else { "0" }.into())),
            None => mismatch(diags, "true or false"),
        },
        "duration" => match duration_ms(d) {
            Some(ms) if ms % 1000 == 0 => Some((json!(ms / 1000), (ms / 1000).to_string())),
            _ => mismatch(diags, "a duration in whole seconds (e.g. 90s, 25m, 1h)"),
        },
        "time" => match d.as_str() {
            Some(s) if is_clock_time(s) => Some((json!(s), sql_str(s))),
            _ => mismatch(diags, "a time like \"09:00\""),
        },
        "time_range" => match d.as_str().and_then(|s| s.split_once('-')) {
            Some((a, b)) if is_clock_time(a.trim()) && is_clock_time(b.trim()) => {
                let s = d.as_str().unwrap_or_default();
                Some((json!(s), sql_str(s)))
            }
            _ => mismatch(diags, "a range like \"21:30-08:00\""),
        },
        "enum" => match d.as_str() {
            Some(s) if options.is_some_and(|o| o.iter().any(|x| x == s)) => Some((json!(s), sql_str(s))),
            _ => {
                let d2 = diags.error("E_SETTING_DEFAULT", path, format!("default `{d}` is not one of the options"));
                d2.hint(format!("Options: {}", options.unwrap_or_default().join(", ")));
                None
            }
        },
        _ => match d {
            Value::String(s) => Some((json!(s), sql_str(s))),
            Value::Number(n) => Some((json!(n.to_string()), sql_str(&n.to_string()))),
            _ => mismatch(diags, "a string"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(yaml_json: Value) -> (Data, Diags) {
        let mut d = Diags::default();
        let out = compile_tables(Some(&yaml_json), &mut d);
        (out, d)
    }

    #[test]
    fn field_grammar() {
        let (out, d) = tables(json!({ "t": {
            "a": "int!", "b": "text? = 'hi'", "c": "enum(x|y) = y", "d": "bool = true", "e": "duration = 25m",
            "f": "date = today", "g": { "type": "real", "default": 1.5 }
        }}));
        assert!(!d.has_errors(), "{:?}", d.items);
        let ddl = &out.ddl[0];
        assert!(ddl.contains("a INTEGER NOT NULL"), "{ddl}");
        assert!(ddl.contains("b TEXT DEFAULT 'hi'"), "{ddl}");
        assert!(ddl.contains("c TEXT DEFAULT 'y' CHECK (c IN ('x','y'))"), "{ddl}");
        assert!(ddl.contains("d INTEGER DEFAULT 1"), "{ddl}");
        assert!(ddl.contains("e INTEGER DEFAULT 1500"), "{ddl}");
        assert!(ddl.contains("f TEXT DEFAULT (date('now','localtime'))"), "{ddl}");
        assert!(ddl.contains("g REAL DEFAULT 1.5"), "{ddl}");
        assert_eq!(out.tables[0].columns.len(), 10);
    }

    #[test]
    fn field_errors() {
        let (_, d) =
            tables(json!({ "t": { "a": "integr", "b": "enum(x|y) = z", "id": "int" }, "settings": { "x": "int" } }));
        let codes: Vec<&str> = d.items.iter().map(|i| i.code).collect();
        assert_eq!(codes, ["E_FIELD_TYPE", "E_FIELD_DEFAULT", "W_IMPLICIT_COLUMN", "E_NAME_RESERVED"]);
    }

    #[test]
    fn an_expression_index_names_its_column() {
        let (_, diags) = tables(
            json!({ "moods": { "rating": "int!", "at": "datetime = now" }, "_indexes": ["moods(local_date(at))"] }),
        );
        let d = diags.items.iter().find(|d| d.code == "E_REF_COLUMN").expect("an error");
        assert!(d.message.contains("not expressions"), "{}", d.message);
        assert!(d.hint.as_deref().is_some_and(|h| h.contains("`moods(at)`")), "{:?}", d.hint);
    }

    #[test]
    fn split_and_unique_indexes() {
        let (out, d) = tables(json!({
            "ticks": { "habit_id": "int!", "day": "date = today", "done": "bool = 0" },
            "_indexes": ["ticks(habit_id", "day)", "unique ticks(day)"]
        }));
        assert_eq!(d.items.iter().map(|i| i.code).collect::<Vec<_>>(), ["W_AUTOFIX"]);
        assert!(
            out.ddl.contains(&"CREATE INDEX IF NOT EXISTS ticks_habit_id_day_idx ON ticks(habit_id, day)".to_string())
        );
        assert!(out.ddl.contains(&"CREATE UNIQUE INDEX IF NOT EXISTS ticks_day_uniq ON ticks(day)".to_string()));

        // Inside the table, the table name may be left out.
        let (out, d) = tables(json!({
            "ticks": { "habit_id": "int!", "day": "date = today", "_indexes": ["unique(habit_id", "day)", "ticks(day)"] }
        }));
        assert_eq!(d.items.iter().map(|i| i.code).collect::<Vec<_>>(), ["W_AUTOFIX"]);
        assert!(out.ddl.contains(
            &"CREATE UNIQUE INDEX IF NOT EXISTS ticks_habit_id_day_uniq ON ticks(habit_id, day)".to_string()
        ));
        assert!(out.ddl.contains(&"CREATE INDEX IF NOT EXISTS ticks_day_idx ON ticks(day)".to_string()));
    }

    #[test]
    fn settings_ddl() {
        let mut d = Diags::default();
        let s = compile_settings(
            Some(
                &json!({ "goal": { "type": "int", "default": 100 }, "every": { "type": "duration", "default": "60m" } }),
            ),
            &mut d,
        );
        assert!(!d.has_errors());
        assert_eq!(s.ddl[1], "INSERT OR IGNORE INTO settings (id, goal, every) VALUES (1, 100, 3600)");
        assert_eq!(s.defs["every"]["default"], json!(3600));
    }
}
