//! The compiled app artifact (docs/specs/compiled-app.md, v0). Field order and names match the contract;
//! values that the runtime evaluates are kept as JSON (`serde_json::Value`).

use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Clone, Debug, Serialize)]
pub struct CompiledApp {
    pub enee: u32,
    pub app: AppMeta,
    pub permissions: Vec<Permission>,
    pub settings: Map<String, Value>,
    pub tables: Vec<Table>,
    pub ddl: Vec<String>,
    pub queries: Vec<Query>,
    pub state: Map<String, Value>,
    /// State keys kept across restarts (`state._persist`), saved by the host.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub persist: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<Service>,
    pub actions: Map<String, Value>,
    pub views: Vec<View>,
    /// The app's own code (ADR-0041), run in a sandboxed Worker while the app is on screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logic: Option<Logic>,
    /// The app's card on the glance screen (ADR-0042): a card of r1d's catalog whose fields are Values, read by r1d
    /// while the app is closed. A list field may be `{ each: <Value>, take?, item }`: `item` once per row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glance: Option<Value>,
    pub triggers: Vec<Value>,
    pub checks: Vec<Value>,
    pub sourcemap: Map<String, Value>,
}

/// The card types a glance card can be (ADR-0042): r1d's cards in chats (ADR-0039, `cards::TYPES` there).
pub const GLANCE_TYPES: &[&str] = &["headlines", "link", "weather", "image", "stats", "bars", "line", "steps", "table"];

/// A glance card type's list fields: where `each` can build the items from rows. None: not a card type.
pub fn glance_lists(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "headlines" | "stats" | "bars" | "steps" => &["items"],
        "weather" => &["hours", "days"],
        "line" => &["points"],
        "table" => &["rows"],
        "link" | "image" => &[],
        _ => return None,
    })
}

/// A glance card type's fields (`""`), or a list field's items' fields (`"items"`), as r1d's catalog keeps them.
pub fn glance_fields(kind: &str, list: &str) -> &'static [&'static str] {
    match (kind, list) {
        ("headlines", "") => &["title", "items"],
        ("headlines", "items") => &["title", "tagline", "source", "time", "url", "image"],
        ("link", "") => &["title", "url", "source", "summary", "image"],
        ("weather", "") => &["place", "temp", "condition", "unit", "icon", "high", "low", "feels", "note", "hours", "days"],
        ("weather", "hours") => &["time", "temp", "icon", "condition"],
        ("weather", "days") => &["day", "high", "low", "icon", "condition"],
        ("image", "") => &["url", "caption", "source", "link"],
        ("stats", "") => &["title", "items"],
        ("stats", "items") => &["label", "value", "delta", "trend"],
        ("bars", "") => &["title", "unit", "items"],
        ("line", "") => &["title", "unit", "points"],
        ("bars", "items") | ("line", "points") => &["label", "value"],
        ("steps", "") => &["title", "items"],
        ("steps", "items") => &["title", "detail"],
        ("table", "") => &["title", "columns", "rows"],
        _ => &[],
    }
}

/// The logic lane (ADR-0041): the script, the values it publishes (`{{logic.x}}`) and the actions it answers
/// (`run: name` steps).
#[derive(Clone, Debug, Serialize)]
pub struct Logic {
    pub script: String,
    pub values: Vec<String>,
    pub actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AppMeta {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub archetype: String,
    pub design: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    pub theme: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Permission {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Value>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub not_null: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub primary_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// `enum`: its options.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// `ref`: the table it points to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Query {
    pub name: String,
    pub sql: String,
    pub params: Map<String, Value>,
    pub tables: Vec<String>,
    pub columns: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub name: String,
    #[serde(rename = "use")]
    pub uses: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_ms: Option<u64>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub options: Map<String, Value>,
    /// The `/state/…` and `/settings/…` values an `http.*` URL binds (`{{state.ticker}}`). The runtime reports their
    /// values to the host, which builds the URL (compiled-app.md §8).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub spec: Value,
}
