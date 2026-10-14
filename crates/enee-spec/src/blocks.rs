//! `views:` blocks → json-render flat specs (docs/specs/compiled-app.md §4) plus the sourcemap.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value, json};

use crate::diag::Diags;
use crate::lines::Lines;
use crate::util::{child, humanize, index, is_ident, kind, list, suggest};
use crate::value::{self, FILTERS, Names, Scope};

pub const BLOCKS: [&str; 22] = [
    "stat",
    "timer",
    "gauge",
    "progress",
    "heading",
    "text",
    "button",
    "quick_actions",
    "form",
    "input",
    "toggle",
    "chart",
    "calendar_heatmap",
    "list",
    "empty_state",
    "section",
    "card",
    "row",
    "ui",
    "wheel",
    "game",
    "logic_view",
];

/// Catalog components and their props (`children` is structural, not a prop). `Game` isn't here: a game's script is
/// checked by the `game` block (r1os ADR-0038), so a raw `ui:` element can't carry one.
const CATALOG: [(&str, &[&str]); 19] = [
    ("Stack", &["gap"]),
    ("Row", &["gap", "align", "wrap"]),
    ("Card", &["title"]),
    ("Heading", &["text", "level"]),
    ("Text", &["text", "tone"]),
    ("Stat", &["label", "value", "caption", "format"]),
    ("Timer", &["label", "until", "since", "caption", "done"]),
    ("Gauge", &["label", "value", "max", "unit", "caption"]),
    ("Progress", &["value", "max", "label"]),
    ("Button", &["label", "variant", "icon"]),
    ("Input", &["label", "type", "value", "placeholder", "options", "min", "max", "step"]),
    ("Wheel", &["label", "value", "min", "max", "step", "options", "unit", "wrap"]),
    ("Toggle", &["label", "value"]),
    ("BarChart", &["data", "x", "y", "xDomain", "title"]),
    ("LineChart", &["data", "x", "y", "xDomain", "title"]),
    ("DonutChart", &["data", "x", "y", "title"]),
    ("CalendarHeatmap", &["data", "date", "value", "weeks", "title"]),
    ("List", &["items", "itemKey", "title", "caption", "image", "rowActions", "empty"]),
    ("EmptyState", &["text", "icon"]),
];

/// The longest a game's script may be (r1os ADR-0038).
const GAME_SCRIPT_MAX: usize = 48 * 1024;

/// What a game's script can't use, and why: it runs in a worker with no network and no page (ADR-0038).
const GAME_FORBIDDEN: [(&str, &str); 9] = [
    ("fetch(", "can't reach the network: games run offline"),
    ("XMLHttpRequest", "can't reach the network: games run offline"),
    ("WebSocket", "can't reach the network: games run offline"),
    ("importScripts", "can't load code: a game is one script"),
    ("eval(", "isn't allowed: write the code itself"),
    ("new Function", "isn't allowed: write the code itself"),
    ("document.", "doesn't exist: a game has no page, only `r1.ctx`"),
    ("window.", "doesn't exist: a game has no page, only `r1`"),
    ("localStorage", "doesn't exist: keep values with `r1.save`"),
];

/// What an action accepts, for argument checks.
pub struct ActionSig {
    /// Param name → required.
    pub params: BTreeMap<String, bool>,
    /// Options of `enum(a|b)` params, so form fields bound to them become selects.
    pub enums: BTreeMap<String, Vec<String>>,
}

pub struct Ctx<'a> {
    pub names: &'a mut Names,
    pub actions: &'a BTreeMap<String, ActionSig>,
    pub state: &'a Map<String, Value>,
    pub lines: &'a Lines,
    pub diags: &'a mut Diags,
}

impl Ctx<'_> {
    pub fn val(&mut self, v: &Value, scope: Scope, path: &str) -> Value {
        value::compile(v, scope, self.names, path, self.diags)
    }

    /// Checks `action` + `params` against the action's signature and compiles the args.
    pub fn args(&mut self, p: &Map<String, Value>, path: &str, scope: Scope) -> Option<Value> {
        let apath = child(path, "action");
        let name = match p.get("action") {
            Some(Value::String(s)) => s.clone(),
            _ => {
                self.diags.error("E_PROP_MISSING", path, "missing `action` (the name of an action)");
                return None;
            }
        };
        let Some(sig) = self.actions.get(&name) else {
            let actions: Vec<&str> = self.actions.keys().map(String::as_str).collect();
            let d = self.diags.error("E_REF_ACTION", apath, format!("no action `{name}`"));
            const STEPS: [&str; 14] = [
                "sql", "toast", "set", "navigate", "notify", "refresh", "open_url", "copy", "launch", "play", "pause", "resume",
                "stop", "toggle",
            ];
            d.hint(if STEPS.contains(&name.as_str()) {
                format!(
                    "`{name}` is a step, not an action. Declare an action that runs it, e.g. `open: {{ params: {{ url: text }}, do: [ {{ open_url: \"{{{{params.url}}}}\" }} ] }}`, and call that"
                )
            } else {
                match suggest(&name, actions.iter().copied()) {
                    Some(s) => format!("Did you mean `{s}`? Actions: {}", list(actions)),
                    None => format!("Actions: {}", list(actions)),
                }
            });
            return None;
        };
        let required: Vec<String> = sig.params.iter().filter(|(_, r)| **r).map(|(k, _)| k.clone()).collect();
        let declared: Vec<String> = sig.params.keys().cloned().collect();
        let ppath = child(path, "params");
        let raw = p.get("params").cloned().unwrap_or_else(|| Value::Object(Map::new()));
        if let Value::Object(m) = &raw {
            for k in m.keys() {
                if !declared.contains(k) {
                    let d = self.diags.error(
                        "E_ARG_UNKNOWN",
                        child(&ppath, k),
                        format!("action `{name}` has no param `{k}`"),
                    );
                    d.hint(format!("Params of `{name}`: {}", list(declared.iter().map(String::as_str))));
                }
            }
            for r in &required {
                if !m.contains_key(r) {
                    self.diags.error("E_ARG_MISSING", &ppath, format!("action `{name}` needs param `{r}`"));
                }
            }
        }
        let args = self.val(&raw, scope, &ppath);
        // `params: "{{state.draft}}"`: the initial state object should carry the required params.
        if let Some(Value::String(abs)) = args.get("$state")
            && let Some(key) = abs.strip_prefix("/state/")
            && let Some(Value::Object(init)) = self.state.get(key)
        {
            for r in &required {
                if !init.contains_key(r) {
                    let d = self.diags.warn(
                        "W_ARG_MISSING",
                        &ppath,
                        format!("`state.{key}` has no `{r}`, which action `{name}` requires"),
                    );
                    d.hint(format!("Add `{r}` to `state.{key}`"));
                }
            }
        } else if !args.is_object() && !args.is_null() {
            self.diags.error(
                "E_PROP_VALUE",
                &ppath,
                "params must be a map or a single state binding like \"{{state.draft}}\"",
            );
        }
        Some(args)
    }
}

pub struct ViewOut {
    pub spec: Value,
    pub sourcemap: Vec<(String, Value)>,
}

struct Builder<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    elements: Map<String, Value>,
    sourcemap: Vec<(String, Value)>,
}

pub fn compile_view(name: &str, items: &Value, path: &str, cx: &mut Ctx<'_>) -> ViewOut {
    let mut b = Builder { cx, elements: Map::new(), sourcemap: Vec::new() };
    let children = b.items(name, items, path);
    let mut elements = Map::new();
    elements.insert(name.to_string(), element("Stack", json!({ "gap": "md" }), Some(children), None, None));
    elements.extend(b.elements);
    ViewOut { spec: json!({ "root": name, "elements": elements }), sourcemap: b.sourcemap }
}

/// json-render's React schema expects `children` on every element, leaves included.
fn element(ty: &str, props: Value, children: Option<Vec<String>>, visible: Option<Value>, on: Option<Value>) -> Value {
    let mut e = Map::new();
    e.insert("type".into(), json!(ty));
    e.insert("props".into(), props);
    e.insert("children".into(), json!(children.unwrap_or_default()));
    if let Some(v) = visible {
        e.insert("visible".into(), v);
    }
    if let Some(on) = on {
        e.insert("on".into(), on);
    }
    Value::Object(e)
}

impl Builder<'_, '_> {
    fn diags(&mut self) -> &mut Diags {
        self.cx.diags
    }

    fn add(&mut self, key: &str, el: Value, path: &str, block: &str) {
        let entry = self.entry(path, block);
        self.sourcemap.push((key.to_string(), entry));
        self.elements.insert(key.to_string(), el);
    }

    /// A list of blocks under `parent`; returns the child keys in order.
    fn items(&mut self, parent: &str, items: &Value, path: &str) -> Vec<String> {
        let Some(items) = items.as_array() else {
            self.diags()
                .error("E_SHAPE", path, format!("expected a list of blocks, got {}", kind(items)))
                .hint("Write blocks as `- stat: { … }` list items");
            return Vec::new();
        };
        let mut ordinals: HashMap<String, usize> = HashMap::new();
        let mut keys = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let ipath = index(path, i);
            if let Some(key) = self.block(parent, item, &ipath, &mut ordinals) {
                keys.push(key);
            }
        }
        keys
    }

    fn block(
        &mut self,
        parent: &str,
        item: &Value,
        path: &str,
        ordinals: &mut HashMap<String, usize>,
    ) -> Option<String> {
        let Some(map) = item.as_object() else {
            self.diags().error(
                "E_SHAPE",
                path,
                format!("a block must be a map like `stat: {{ … }}`, got {}", kind(item)),
            );
            return None;
        };
        let names: Vec<&String> = map.keys().filter(|k| !["id", "visible"].contains(&k.as_str())).collect();
        let [block] = names.as_slice() else {
            let d = self.diags().error("E_BLOCK_SHAPE", path, "each list item must hold exactly one block");
            d.hint("Indent block props under the block name: `- list:` then `    data: recent`");
            return None;
        };
        let block = block.as_str();
        if block == "component" {
            self.diags()
                .error("E_UNSUPPORTED", path, "`component:` escapes are not supported yet (v0)")
                .hint("Use blocks or a `ui:` element");
            return None;
        }
        if !BLOCKS.contains(&block) {
            let d = self.diags().error("E_BLOCK_UNKNOWN", path, format!("unknown block `{block}`"));
            d.hint(match suggest(block, BLOCKS) {
                Some(s) => format!("Did you mean `{s}`? Blocks: {}", BLOCKS.join(", ")),
                None => format!("Blocks: {}", BLOCKS.join(", ")),
            });
            return None;
        }
        let body = &map[block];
        let bpath = child(path, block);
        // `id` / `visible` may sit next to the block name or inside its props.
        let field = |k: &str| map.get(k).or_else(|| body.as_object().and_then(|b| b.get(k)));
        let id = field("id").cloned();
        let visible = field("visible").cloned();
        let key = match id {
            Some(Value::String(id)) if is_ident(&id) => format!("{parent}/{id}"),
            Some(other) => {
                self.diags().error("E_BLOCK_ID", path, format!("block id must be a snake_case string, got `{other}`"));
                return None;
            }
            None => {
                let n = ordinals.entry(block.to_string()).or_default();
                *n += 1;
                format!("{parent}/{block}-{n}")
            }
        };
        if self.elements.contains_key(&key) {
            self.diags().error("E_BLOCK_ID", path, format!("duplicate element id `{key}`"));
            return None;
        }
        let visible = visible.and_then(|v| self.visible(&v, &child(path, "visible")));
        self.slot(&key, path, block, |b| b.expand(block, body, &key, &bpath, visible))?;
        Some(key)
    }

    /// Reserves the element's position before building it, so parents precede their children in the output.
    fn slot(
        &mut self,
        key: &str,
        path: &str,
        block: &str,
        build: impl FnOnce(&mut Self) -> Option<Value>,
    ) -> Option<()> {
        self.elements.insert(key.to_string(), Value::Null);
        let at = self.sourcemap.len();
        self.sourcemap.push((key.to_string(), Value::Null));
        match build(self) {
            Some(el) => {
                self.elements.insert(key.to_string(), el);
                self.sourcemap[at].1 = self.entry(path, block);
                Some(())
            }
            None => {
                self.elements.shift_remove(key);
                self.sourcemap.remove(at);
                None
            }
        }
    }

    fn entry(&self, path: &str, block: &str) -> Value {
        let mut entry = Map::new();
        entry.insert("path".into(), json!(path));
        if let Some(line) = self.cx.lines.get(path) {
            entry.insert("line".into(), json!(line));
        }
        entry.insert("block".into(), json!(block));
        Value::Object(entry)
    }

    /// Props of a map-bodied block, warning on unknown keys. A scalar body is shorthand for `shorthand_key`.
    fn props(
        &mut self,
        body: &Value,
        path: &str,
        known: &[&str],
        shorthand_key: Option<&str>,
    ) -> Option<Map<String, Value>> {
        match (body, shorthand_key) {
            (Value::Object(m), _) => {
                for k in m.keys() {
                    if !known.contains(&k.as_str()) && !["id", "visible"].contains(&k.as_str()) {
                        let d =
                            self.diags().warn("W_PROP_UNKNOWN", child(path, k), format!("unknown prop `{k}` ignored"));
                        d.hint(match suggest(k, known.iter().copied()) {
                            Some(s) => format!("Did you mean `{s}`? Props: {}", known.join(", ")),
                            None => format!("Props: {}", known.join(", ")),
                        });
                    }
                }
                Some(m.clone())
            }
            (Value::String(_) | Value::Number(_), Some(k)) => {
                let mut m = Map::new();
                m.insert(k.to_string(), body.clone());
                Some(m)
            }
            _ => {
                self.diags().error(
                    "E_SHAPE",
                    path,
                    format!("expected a map of props ({}), got {}", known.join(", "), kind(body)),
                );
                None
            }
        }
    }

    fn require(&mut self, p: &Map<String, Value>, keys: &[&str], path: &str) -> bool {
        let mut ok = true;
        for k in keys {
            if !p.contains_key(*k) {
                self.diags().error("E_PROP_MISSING", path, format!("missing required prop `{k}`"));
                ok = false;
            }
        }
        ok
    }

    fn val(&mut self, v: &Value, scope: Scope, path: &str) -> Value {
        self.cx.val(v, scope, path)
    }

    /// An `icon` prop as a valid Phosphor name (mapped with a warning when needed). Bindings aren't allowed here.
    fn icon(&mut self, v: &Value, context: &str, path: &str) -> Option<Value> {
        match v.as_str() {
            Some(s) if !s.contains("{{") => Some(json!(crate::icons::resolve_prop(s, context, path, self.cx.diags))),
            _ => {
                self.diags().error("E_PROP_VALUE", path, "`icon` must be a Phosphor icon name, e.g. trash");
                None
            }
        }
    }

    /// Copies optional props, compiling bindings.
    fn copy(&mut self, from: &Map<String, Value>, to: &mut Map<String, Value>, keys: &[(&str, &str)], path: &str) {
        for (src, dst) in keys {
            if let Some(v) = from.get(*src) {
                let v = self.val(v, Scope::Element, &child(path, src));
                to.insert(dst.to_string(), v);
            }
        }
    }

    fn expand(&mut self, block: &str, body: &Value, key: &str, path: &str, visible: Option<Value>) -> Option<Value> {
        let el = |ty: &str, props: Map<String, Value>, children: Option<Vec<String>>, on: Option<Value>| {
            element(ty, Value::Object(props), children, visible.clone(), on)
        };
        match block {
            "stat" => {
                let p = self.props(body, path, &["label", "value", "caption", "format"], None)?;
                if !self.require(&p, &["label", "value"], path) {
                    return None;
                }
                let mut props = Map::new();
                self.copy(&p, &mut props, &[("label", "label"), ("value", "value"), ("caption", "caption")], path);
                if let Some(format) = p.get("format").and_then(Value::as_str) {
                    let (name, arg) = match format.split_once(':') {
                        Some((n, a)) => (n.trim(), Some(a.trim())),
                        None => (format.trim(), None),
                    };
                    if !FILTERS.contains(&name) {
                        self.diags()
                            .error("E_BIND_FILTER", child(path, "format"), format!("unknown format `{name}`"))
                            .hint(format!("Formats: {}", FILTERS.join(", ")));
                    } else if let Some(v) = props.get("value").and_then(|v| with_filter(v, name, arg)) {
                        props.insert("value".into(), v);
                    } else if arg.is_none() {
                        props.insert("format".into(), json!(name));
                    }
                }
                Some(el("Stat", props, None, None))
            }
            "timer" => {
                let mut p = self.props(body, path, &["label", "until", "since", "caption", "done"], None)?;
                if !self.require(&p, &["label"], path) {
                    return None;
                }
                match (p.contains_key("until"), p.contains_key("since")) {
                    // Builds wrote both (the start and the end of a session): it counts down, which is what a
                    // session with an end shows.
                    (true, true) => {
                        p.remove("since");
                        self.diags()
                            .warn("W_AUTOFIX", path, "a timer counts down to `until` or up from `since`: kept `until`")
                            .hint("Give a countdown only `until`, and a stopwatch only `since`");
                    }
                    (false, false) => {
                        self.diags()
                            .error(
                                "E_PROP_MISSING",
                                path,
                                "a timer needs `until` (a countdown) or `since` (a stopwatch)",
                            )
                            .hint("e.g. `until: \"{{active.0.ends_at}}\"`, a time a query works out");
                        return None;
                    }
                    _ => {}
                }
                let mut props = Map::new();
                let keys = [
                    ("label", "label"),
                    ("until", "until"),
                    ("since", "since"),
                    ("caption", "caption"),
                    ("done", "done"),
                ];
                self.copy(&p, &mut props, &keys, path);
                Some(el("Timer", props, None, None))
            }
            "gauge" => {
                let p = self.props(body, path, &["label", "value", "max", "unit", "caption"], None)?;
                if !self.require(&p, &["label", "value"], path) {
                    return None;
                }
                let mut props = Map::new();
                let keys =
                    [("label", "label"), ("value", "value"), ("max", "max"), ("unit", "unit"), ("caption", "caption")];
                self.copy(&p, &mut props, &keys, path);
                Some(el("Gauge", props, None, None))
            }
            "progress" => {
                let p = self.props(body, path, &["value", "max", "label"], None)?;
                if !self.require(&p, &["value", "max"], path) {
                    return None;
                }
                let mut props = Map::new();
                self.copy(&p, &mut props, &[("value", "value"), ("max", "max"), ("label", "label")], path);
                Some(el("Progress", props, None, None))
            }
            "heading" | "text" | "empty_state" => {
                let (ty, extra) = match block {
                    "heading" => ("Heading", "level"),
                    "text" => ("Text", "tone"),
                    _ => ("EmptyState", "icon"),
                };
                let p = self.props(body, path, &["text", extra], Some("text"))?;
                if !self.require(&p, &["text"], path) {
                    return None;
                }
                let mut props = Map::new();
                if extra == "icon" {
                    self.copy(&p, &mut props, &[("text", "text")], path);
                    if let Some(i) = p.get("icon").and_then(|v| self.icon(v, "", &child(path, "icon"))) {
                        props.insert("icon".into(), i);
                    }
                } else {
                    self.copy(&p, &mut props, &[("text", "text"), (extra, extra)], path);
                }
                if ty == "Heading" && !props.contains_key("level") {
                    props.insert("level".into(), json!(2));
                }
                Some(el(ty, props, None, None))
            }
            "button" => {
                let p = self.props(body, path, &["label", "action", "params", "variant", "icon"], None)?;
                let (props, on) = self.button(&p, path, "primary", Scope::Element)?;
                Some(el("Button", props, None, Some(on)))
            }
            "quick_actions" => {
                let (buttons, items_path) = match body {
                    Value::Array(_) => (body, path.to_string()),
                    Value::Object(m) if m.contains_key("items") => (&m["items"], child(path, "items")),
                    _ => {
                        self.diags().error("E_SHAPE", path, "quick_actions is a list of `{ label, action, params }`");
                        return None;
                    }
                };
                let mut keys = Vec::new();
                for (i, b) in buttons.as_array().into_iter().flatten().enumerate() {
                    let bpath = index(&items_path, i);
                    let Some(p) = self.props(b, &bpath, &["label", "action", "params", "variant", "icon"], None) else {
                        continue;
                    };
                    let Some((props, on)) = self.button(&p, &bpath, "secondary", Scope::Element) else { continue };
                    let bkey = format!("{key}/button-{}", i + 1);
                    self.add(&bkey, element("Button", Value::Object(props), None, None, Some(on)), &bpath, block);
                    keys.push(bkey);
                }
                Some(el(
                    "Row",
                    Map::from_iter([("gap".into(), json!("sm")), ("wrap".into(), json!(true))]),
                    Some(keys),
                    None,
                ))
            }
            "form" => self.form(body, key, path, visible.clone()),
            "wheel" => self.wheel(body, path, visible.clone()),
            "game" => self.game(body, path, visible.clone()),
            // A view the app's logic builds (ADR-0041): the tree its script publishes at `logic.<value>`.
            "logic_view" => {
                let name = body.as_str().unwrap_or_default().trim().trim_start_matches("logic.").to_owned();
                if !self.cx.names.logic.contains(&name) {
                    let known = list(self.cx.names.logic.iter().map(String::as_str));
                    let d = self.diags().error("E_BIND_UNKNOWN", path, format!("`logic_view` names a logic value; no `{name}`"));
                    d.hint(format!("Logic values: {known}"));
                    return None;
                }
                Some(element("LogicView", json!({ "value": name }), None, visible.clone(), None))
            }
            "input" | "toggle" => {
                let p = self.props(body, path, &["bind", "label", "type", "placeholder", "options", "min", "max", "step"], None)?;
                let (ty, props) = self.field(&p, path, &BTreeMap::new())?;
                if block == "toggle" && ty != "Toggle" {
                    self.diags().warn("W_AUTOFIX", path, "toggle bound to a non-boolean state value");
                }
                Some(el(if block == "toggle" { "Toggle" } else { ty }, props, None, None))
            }
            "chart" => {
                let p = self.props(body, path, &["kind", "data", "x", "y", "x_domain", "title"], None)?;
                if !self.require(&p, &["data", "x", "y"], path) {
                    return None;
                }
                let ty = match p.get("kind").and_then(Value::as_str).unwrap_or("bar") {
                    "bar" => "BarChart",
                    "line" => "LineChart",
                    // Shares of a whole: `x` names each slice, `y` sizes it.
                    "donut" | "pie" => "DonutChart",
                    other => {
                        self.diags()
                            .error(
                                "E_PROP_VALUE",
                                child(path, "kind"),
                                format!("chart kind `{other}` is not supported yet"),
                            )
                            .hint("Kinds: bar, line, donut");
                        return None;
                    }
                };
                if ty == "DonutChart" && p.contains_key("x_domain") {
                    self.diags()
                        .warn(
                            "W_PROP_IGNORED",
                            child(path, "x_domain"),
                            "a donut has no x axis, so `x_domain` is left out",
                        )
                        .hint("Remove it, or use kind: bar");
                }
                let (data, cols) = self.query_ref(&p["data"], &child(path, "data"))?;
                let mut props = Map::new();
                props.insert("data".into(), data);
                for axis in ["x", "y"] {
                    let Some(col) = p[axis].as_str() else {
                        self.diags().error(
                            "E_PROP_VALUE",
                            child(path, axis),
                            format!("`{axis}` must be a column name"),
                        );
                        return None;
                    };
                    if !cols.is_empty() && !cols.iter().any(|c| c == col) {
                        let d = self.diags().error(
                            "E_BIND_COLUMN",
                            child(path, axis),
                            format!("query has no column `{col}`"),
                        );
                        d.hint(format!("Columns: {}", cols.join(", ")));
                    }
                    props.insert(axis.into(), json!(col));
                }
                if let Some(d) = p.get("x_domain").filter(|_| ty != "DonutChart") {
                    let range = match d {
                        Value::String(s) => s
                            .split_once("..")
                            .and_then(|(a, b)| Some([a.trim().parse::<i64>().ok()?, b.trim().parse::<i64>().ok()?])),
                        Value::Array(a) if a.len() == 2 => a[0].as_i64().zip(a[1].as_i64()).map(|(a, b)| [a, b]),
                        _ => None,
                    };
                    match range {
                        Some([a, b]) if a < b => {
                            props.insert("xDomain".into(), json!([a, b]));
                        }
                        _ => {
                            self.diags().error(
                                "E_PROP_VALUE",
                                child(path, "x_domain"),
                                "x_domain must be an integer range like \"0..23\"",
                            );
                        }
                    }
                }
                self.copy(&p, &mut props, &[("title", "title")], path);
                Some(el(ty, props, None, None))
            }
            "calendar_heatmap" => {
                let p = self.props(body, path, &["data", "date", "value", "weeks", "title"], None)?;
                if !self.require(&p, &["data", "date", "value"], path) {
                    return None;
                }
                let (data, cols) = self.query_ref(&p["data"], &child(path, "data"))?;
                let mut props = Map::new();
                props.insert("data".into(), data);
                for key in ["date", "value"] {
                    let Some(col) = p[key].as_str() else {
                        self.diags().error("E_PROP_VALUE", child(path, key), format!("`{key}` must be a column name"));
                        return None;
                    };
                    if !cols.is_empty() && !cols.iter().any(|c| c == col) {
                        let d = self.diags().error(
                            "E_BIND_COLUMN",
                            child(path, key),
                            format!("query has no column `{col}`"),
                        );
                        d.hint(format!("Columns: {}", cols.join(", ")));
                    }
                    props.insert(key.into(), json!(col));
                }
                if let Some(weeks) = p.get("weeks") {
                    match weeks.as_u64() {
                        Some(n @ 1..=53) => {
                            props.insert("weeks".into(), json!(n));
                        }
                        _ => {
                            self.diags().error(
                                "E_PROP_VALUE",
                                child(path, "weeks"),
                                "`weeks` is a number from 1 to 53",
                            );
                        }
                    }
                }
                self.copy(&p, &mut props, &[("title", "title")], path);
                Some(el("CalendarHeatmap", props, None, None))
            }
            "list" => self.list(body, path, visible.clone()),
            "section" | "card" | "row" => {
                let known: &[&str] = match block {
                    "row" => &["items", "gap", "align", "wrap"],
                    _ => &["items", "title"],
                };
                let (p, items_val, items_path) = match body {
                    Value::Array(_) => (Map::new(), body.clone(), path.to_string()),
                    _ => {
                        let p = self.props(body, path, known, None)?;
                        let items = p.get("items").cloned().unwrap_or(Value::Array(Vec::new()));
                        (p, items, child(path, "items"))
                    }
                };
                let mut children = Vec::new();
                let mut props = Map::new();
                let ty = match block {
                    "row" => {
                        self.row_layout(&items_val, &items_path);
                        props.insert("gap".into(), json!("sm"));
                        self.copy(&p, &mut props, &[("gap", "gap"), ("align", "align"), ("wrap", "wrap")], path);
                        "Row"
                    }
                    "card" => {
                        self.copy(&p, &mut props, &[("title", "title")], path);
                        "Card"
                    }
                    _ => {
                        props.insert("gap".into(), json!("sm"));
                        if let Some(title) = p.get("title") {
                            let text = self.val(title, Scope::Element, &child(path, "title"));
                            let hkey = format!("{key}/heading");
                            self.add(
                                &hkey,
                                element("Heading", json!({ "text": text, "level": 3 }), None, None, None),
                                &child(path, "title"),
                                block,
                            );
                            children.push(hkey);
                        }
                        "Stack"
                    }
                };
                children.extend(self.items(key, &items_val, &items_path));
                Some(el(ty, props, Some(children), None))
            }
            "ui" => self.ui(body, key, path, visible.clone()),
            _ => unreachable!("checked against BLOCKS"),
        }
    }

    /// Layout rules for rows (design-systems.md §5.2): a row of stats is a summary of at most three numbers, and stats
    /// get a row of their own, where the runtime lines them up.
    fn row_layout(&mut self, items: &Value, path: &str) {
        let Some(items) = items.as_array() else { return };
        let kinds: Vec<&str> = items
            .iter()
            .filter_map(|item| item.as_object()?.keys().find(|k| BLOCKS.contains(&k.as_str())).map(String::as_str))
            .collect();
        // A timer is a stat that ticks.
        let stats = kinds.iter().filter(|k| matches!(**k, "stat" | "timer")).count();
        if stats > 3 {
            self.diags()
                .error("E_LAYOUT", path, format!("a row holds at most 3 stats, and this one has {stats}"))
                .hint("Keep the 2–3 numbers that matter most in the row, and show the rest in a second row or a card");
        }
        let gauges = kinds.iter().filter(|k| **k == "gauge").count();
        if gauges > 3 {
            self.diags()
                .error("E_LAYOUT", path, format!("a row holds at most 3 gauges, and this one has {gauges}"))
                .hint("Put the rest in a second row");
        }
        if stats > 0 && stats < kinds.len() {
            self.diags()
                .warn("W_LAYOUT", path, "stats share this row with other blocks, so they won't line up")
                .hint("Give the stats a row of their own");
        }
    }

    /// `{ label, action, params?, variant?, icon? }` → Button props + `on.press`.
    fn button(
        &mut self,
        p: &Map<String, Value>,
        path: &str,
        variant: &str,
        scope: Scope,
    ) -> Option<(Map<String, Value>, Value)> {
        if !self.require(p, &["label", "action"], path) {
            return None;
        }
        let mut props = Map::new();
        self.copy(p, &mut props, &[("label", "label")], path);
        let variant = p.get("variant").cloned().unwrap_or_else(|| json!(variant));
        props.insert("variant".into(), variant);
        if let Some(i) = p.get("icon").and_then(|v| self.icon(v, "", &child(path, "icon"))) {
            props.insert("icon".into(), i);
        }
        let on = self.run(p, path, scope)?;
        Some((props, json!({ "press": on })))
    }

    /// `action` + `params` → `{ action: "run", params: { name, args } }`, checking the action's signature.
    fn run(&mut self, p: &Map<String, Value>, path: &str, scope: Scope) -> Option<Value> {
        let args = self.cx.args(p, path, scope)?;
        let name = p.get("action").and_then(Value::as_str).unwrap_or_default();
        Some(json!({ "action": "run", "params": { "name": name, "args": args } }))
    }

    /// `form: { fields: [ { bind, label?, type?, placeholder? } ], submit?: { label, action, params? }, title? }`.
    /// Without `submit`, the fields edit state as they're typed (a query over state works from them, like a converter's).
    fn form(&mut self, body: &Value, key: &str, path: &str, visible: Option<Value>) -> Option<Value> {
        let p = self.props(body, path, &["fields", "submit", "title"], None)?;
        if !self.require(&p, &["fields"], path) {
            return None;
        }
        let fpath = child(path, "fields");
        let mut children = Vec::new();
        let mut binds: Vec<String> = Vec::new();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        // Enum params of the submit action: a field bound to `state.draft.category` picks from their options.
        let submit_enums = p
            .get("submit")
            .and_then(|s| s.get("action"))
            .and_then(Value::as_str)
            .and_then(|a| self.cx.actions.get(a))
            .map(|sig| sig.enums.clone())
            .unwrap_or_default();
        for (i, f) in p["fields"].as_array().into_iter().flatten().enumerate() {
            let ipath = index(&fpath, i);
            let Some(fp) = self.props(f, &ipath, &["bind", "label", "type", "placeholder", "options", "min", "max", "step"], None) else {
                continue;
            };
            let Some((ty, props)) = self.field(&fp, &ipath, &submit_enums) else { continue };
            if let Some(b) = props.get("value").and_then(|v| v.get("$bindState")).and_then(Value::as_str) {
                binds.push(b.to_string());
            }
            let n = counts.entry(ty).or_default();
            *n += 1;
            let fkey = format!("{key}/{}-{n}", ty.to_ascii_lowercase());
            self.add(&fkey, element(ty, Value::Object(props), None, None, None), &ipath, "form");
            children.push(fkey);
        }
        if let Some(raw) = p.get("submit") {
            let spath = child(path, "submit");
            let mut submit = self.props(raw, &spath, &["label", "action", "params", "variant", "icon"], None)?;
            // Without explicit params, submit the state object the fields edit (`state.draft`).
            if !submit.contains_key("params")
                && let Some(parent) = common_parent(&binds)
            {
                let state_path = parent.trim_start_matches('/').replace('/', ".");
                submit.insert("params".into(), json!(format!("{{{{{state_path}}}}}")));
            }
            let (props, on) = self.button(&submit, &spath, "primary", Scope::Element)?;
            let skey = format!("{key}/submit");
            self.add(&skey, element("Button", Value::Object(props), None, None, Some(on)), &spath, "form");
            children.push(skey);
        }
        let mut props = Map::new();
        self.copy(&p, &mut props, &[("title", "title")], path);
        Some(element("Card", Value::Object(props), Some(children), visible, None))
    }

    /// An input bound to state: `Input` (text/number/time/date) or `Toggle` (booleans).
    fn field(
        &mut self,
        p: &Map<String, Value>,
        path: &str,
        enums: &BTreeMap<String, Vec<String>>,
    ) -> Option<(&'static str, Map<String, Value>)> {
        if !self.require(p, &["bind"], path) {
            return None;
        }
        let bind_src = p["bind"].as_str().unwrap_or_default();
        let abs = value::bind_path(bind_src, self.cx.names, &child(path, "bind"), self.cx.diags)?;
        let initial = abs.strip_prefix("/state/").and_then(|rest| {
            let mut segs = rest.split('/');
            let first = self.cx.state.get(segs.next()?)?;
            segs.try_fold(first, |v, seg| v.get(seg)).cloned()
        });
        let declared = p.get("type").and_then(Value::as_str);
        let field_name = abs.rsplit('/').next().unwrap_or_default();
        let explicit_options: Option<Vec<Value>> = p.get("options").and_then(Value::as_array).cloned();
        let inferred_options = enums.get(field_name).map(|o| o.iter().map(|s| json!(s)).collect::<Vec<_>>());
        let ty = match (declared, &initial) {
            (Some("toggle" | "bool"), _) | (None, Some(Value::Bool(_))) => "toggle",
            (Some("select" | "enum"), _) => "select",
            (Some(t @ ("text" | "number" | "time" | "date")), _) => t,
            (Some(other), _) => {
                self.diags()
                    .error("E_PROP_VALUE", child(path, "type"), format!("unknown input type `{other}`"))
                    .hint("Types: text, number, time, date, select (with options), toggle");
                return None;
            }
            (None, _) if explicit_options.is_some() || inferred_options.is_some() => "select",
            (None, Some(Value::Number(_))) => "number",
            _ => "text",
        };
        let options = match ty {
            "select" => match explicit_options.or(inferred_options) {
                Some(o) if !o.is_empty() => Some(o),
                _ => {
                    self.diags()
                        .error("E_PROP_MISSING", path, "a select needs `options: [a, b]`")
                        .hint("List the choices, or bind the field to an action param declared as `enum(a|b)`");
                    return None;
                }
            },
            _ => None,
        };
        let mut props = Map::new();
        let label = match p.get("label") {
            Some(l) => self.val(l, Scope::Element, &child(path, "label")),
            None => json!(humanize(abs.rsplit('/').next().unwrap_or_default())),
        };
        props.insert("label".into(), label);
        if ty == "toggle" {
            props.insert("value".into(), json!({ "$bindState": abs }));
            return Some(("Toggle", props));
        }
        props.insert("type".into(), json!(ty));
        props.insert("value".into(), json!({ "$bindState": abs }));
        if let Some(o) = options {
            props.insert("options".into(), Value::Array(o));
        }
        self.copy(p, &mut props, &[("placeholder", "placeholder")], path);
        // A number is turned with the wheel (ADR-0038): its ends and step, when given.
        if ty == "number" {
            let (min, max, step) = self.range(p, path, false)?;
            for (k, v) in [("min", min), ("max", max), ("step", step)] {
                if let Some(v) = v {
                    props.insert(k.into(), json!(v));
                }
            }
        }
        Some(("Input", props))
    }

    /// `min`, `max` and `step` as numbers, checked: both ends or neither (`need`: both), min below max, a positive step
    /// that leaves no more than 100 000 values between them.
    fn range(&mut self, p: &Map<String, Value>, path: &str, need: bool) -> Option<(Option<f64>, Option<f64>, Option<f64>)> {
        let mut num = |k: &str| -> Result<Option<f64>, ()> {
            match p.get(k) {
                None => Ok(None),
                Some(v) => match v.as_f64() {
                    Some(n) if n.is_finite() => Ok(Some(n)),
                    _ => {
                        self.cx
                            .diags
                            .error("E_PROP_VALUE", child(path, k), format!("`{k}` must be a number, got {}", kind(v)))
                            .hint("e.g. `min: 0`, `max: 20`, `step: 1` (bindings aren't allowed here)");
                        Err(())
                    }
                },
            }
        };
        let (min, max, step) = (num("min").ok()?, num("max").ok()?, num("step").ok()?);
        match (min, max) {
            (Some(lo), Some(hi)) if lo >= hi => {
                self.diags().error("E_PROP_VALUE", child(path, "max"), format!("`max` ({hi}) must be above `min` ({lo})"));
                return None;
            }
            (None, None) if need => {
                self.diags()
                    .error("E_PROP_MISSING", path, "a number wheel needs `min` and `max`")
                    .hint("e.g. `min: 0`, `max: 20` (or give `options: [small, medium, large]`)");
                return None;
            }
            (Some(_), None) | (None, Some(_)) if need => {
                self.diags().error("E_PROP_MISSING", path, "a number wheel needs both `min` and `max`");
                return None;
            }
            _ => {}
        }
        if let Some(s) = step
            && s <= 0.0
        {
            self.diags().error("E_PROP_VALUE", child(path, "step"), "`step` must be above 0");
            return None;
        }
        if let (Some(lo), Some(hi)) = (min, max)
            && (hi - lo) / step.unwrap_or(1.0) > 100_000.0
        {
            self.diags()
                .error("E_PROP_VALUE", path, "that's more than 100 000 values to turn through")
                .hint("Use a bigger `step`, or narrow `min`–`max`");
            return None;
        }
        Some((min, max, step))
    }

    /// `wheel: { bind, label?, min, max, step?, unit?, wrap? }` or `wheel: { bind, label?, options: [..], wrap? }`: a
    /// value turned with the R1's wheel (ADR-0038). The state changes as it turns, so a query over it follows live.
    fn wheel(&mut self, body: &Value, path: &str, visible: Option<Value>) -> Option<Value> {
        let p = self.props(body, path, &["bind", "label", "min", "max", "step", "options", "unit", "wrap"], None)?;
        if !self.require(&p, &["bind"], path) {
            return None;
        }
        let abs = value::bind_path(p["bind"].as_str().unwrap_or_default(), self.cx.names, &child(path, "bind"), self.cx.diags)?;
        let mut props = Map::new();
        let label = match p.get("label") {
            Some(l) => self.val(l, Scope::Element, &child(path, "label")),
            None => json!(humanize(abs.rsplit('/').next().unwrap_or_default())),
        };
        props.insert("label".into(), label);
        props.insert("value".into(), json!({ "$bindState": abs }));
        match p.get("options") {
            Some(Value::Array(o)) if !o.is_empty() && o.iter().all(Value::is_string) => {
                props.insert("options".into(), Value::Array(o.clone()));
                if p.contains_key("min") || p.contains_key("max") {
                    self.diags().warn("W_PROP_UNKNOWN", path, "a wheel of `options` has no `min`/`max`: ignored");
                }
            }
            Some(other) => {
                self.diags()
                    .error("E_PROP_VALUE", child(path, "options"), format!("`options` must be a list of words, got {}", kind(other)))
                    .hint("e.g. `options: [small, medium, large]`");
                return None;
            }
            None => {
                let (min, max, step) = self.range(&p, path, true)?;
                for (k, v) in [("min", min), ("max", max), ("step", step)] {
                    if let Some(v) = v {
                        props.insert(k.into(), json!(v));
                    }
                }
            }
        }
        if let Some(unit) = p.get("unit") {
            match unit.as_str() {
                Some(u) if u.chars().count() <= 12 => {
                    props.insert("unit".into(), json!(u));
                }
                _ => {
                    self.diags().error("E_PROP_VALUE", child(path, "unit"), "`unit` is a short word, e.g. km");
                    return None;
                }
            }
        }
        if let Some(w) = p.get("wrap") {
            props.insert("wrap".into(), json!(w.as_bool().unwrap_or(false)));
        }
        Some(element("Wheel", Value::Object(props), None, visible, None))
    }

    /// `game: { label, script, actions?: [..], height? }`: a small game in JavaScript (ADR-0038). It runs sandboxed
    /// (a worker in a frame with no network and no DOM), so a script that reaches for those can't work: say so here.
    fn game(&mut self, body: &Value, path: &str, visible: Option<Value>) -> Option<Value> {
        let p = self.props(body, path, &["label", "script", "actions", "height"], None)?;
        if !self.require(&p, &["label", "script"], path) {
            return None;
        }
        let spath = child(path, "script");
        let script = match p["script"].as_str() {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                self.diags().error("E_PROP_VALUE", &spath, "`script` is the game's JavaScript, as a block of text");
                return None;
            }
        };
        if script.len() > GAME_SCRIPT_MAX {
            self.diags()
                .error("E_PROP_VALUE", &spath, format!("the script is {} KB; a game may be {} KB", script.len() / 1024, GAME_SCRIPT_MAX / 1024))
                .hint("Keep the game small: one screen, a few objects");
            return None;
        }
        let mut ok = true;
        for (needle, why) in GAME_FORBIDDEN {
            if script.contains(needle) {
                self.diags()
                    .error("E_GAME_SCRIPT", &spath, format!("`{}` {why}", needle.trim_end_matches('(')))
                    .hint("Games run sandboxed: draw on `r1.ctx`, take input with `r1.on(…)`, keep scores with `r1.save` or an action (`r1.action`)");
                ok = false;
            }
        }
        if !ok {
            return None;
        }
        if !script.contains("r1.") {
            self.diags()
                .warn("W_GAME_SCRIPT", &spath, "the script never uses `r1`, so it can't draw or take input")
                .hint("Draw on `r1.ctx` in `r1.loop((dt) => …)`, and listen with `r1.on(\"wheel\", (d) => …)`");
        }
        let mut props = Map::new();
        props.insert("label".into(), self.val(&p["label"], Scope::Element, &child(path, "label")));
        props.insert("script".into(), json!(script));
        if let Some(raw) = p.get("actions") {
            let apath = child(path, "actions");
            let Some(given) = raw.as_array() else {
                self.diags().error("E_PROP_VALUE", &apath, "`actions` lists the actions the game may run, e.g. `[save_score]`");
                return None;
            };
            let mut names = Vec::new();
            for (i, a) in given.iter().enumerate() {
                let name = a.as_str().unwrap_or_default();
                if !self.cx.actions.contains_key(name) {
                    let known: Vec<&str> = self.cx.actions.keys().map(String::as_str).collect();
                    let d = self.diags().error("E_ACTION_UNKNOWN", index(&apath, i), format!("unknown action `{name}`"));
                    d.hint(format!("Declared actions: {}", list(known.iter().copied())));
                    return None;
                }
                names.push(json!(name));
            }
            props.insert("actions".into(), Value::Array(names));
        }
        if let Some(h) = p.get("height") {
            match h.as_f64() {
                Some(n) if (160.0..=640.0).contains(&n) => {
                    props.insert("height".into(), json!(n));
                }
                _ => {
                    self.diags().error("E_PROP_VALUE", child(path, "height"), "`height` is 160–640 (dp); leave it out to fill most of the screen");
                    return None;
                }
            }
        }
        Some(element("Game", Value::Object(props), None, visible, None))
    }

    /// `data: recent` (or `"{{recent}}"`) → `{"$state": "/q/recent"}` and the query's columns.
    fn query_ref(&mut self, v: &Value, path: &str) -> Option<(Value, Vec<String>)> {
        let name = v.as_str().unwrap_or_default().trim().trim_start_matches("{{").trim_end_matches("}}").trim();
        // A service field holding an array (e.g. `services.cpu.history`); its columns aren't known statically.
        if let Some(field) = name.strip_prefix("services.") {
            let service = field.split('.').next().unwrap_or_default();
            if !self.cx.names.services.contains(service) {
                let d = self.cx.diags.error("E_BIND_UNKNOWN", path, format!("unknown service `{service}`"));
                d.hint(format!("Declared services: {}", list(self.cx.names.services.iter().map(String::as_str))));
                return None;
            }
            return Some((json!({ "$state": format!("/services/{}", field.replace('.', "/")) }), Vec::new()));
        }
        // A list the app's logic publishes (ADR-0041): its columns aren't known statically either.
        if let Some(field) = name.strip_prefix("logic.") {
            let value = field.split('.').next().unwrap_or_default();
            if !self.cx.names.logic.contains(value) {
                let d = self.cx.diags.error("E_BIND_UNKNOWN", path, format!("unknown logic value `{value}`"));
                d.hint(format!("Logic values: {}", list(self.cx.names.logic.iter().map(String::as_str))));
                return None;
            }
            return Some((json!({ "$state": format!("/logic/{}", field.replace('.', "/")) }), Vec::new()));
        }
        match self.cx.names.queries.get(name) {
            Some(cols) => Some((json!({ "$state": format!("/q/{name}") }), cols.clone())),
            None => {
                let queries: Vec<&str> = self.cx.names.queries.keys().map(String::as_str).collect();
                let d =
                    self.cx.diags.error("E_REF_QUERY", path, format!("`data` must name a query; no query `{name}`"));
                let expression = ["?", ":", "(", "||", "&&", "=="].iter().any(|op| name.contains(op));
                d.hint(if expression {
                    "`data` takes one query name, not an expression. Make one query handle both cases, e.g. `WHERE :q = '' OR title LIKE '%' || :q || '%'`".to_string()
                } else {
                    match suggest(name, queries.iter().copied()) {
                        Some(s) => format!("Did you mean `{s}`? Queries: {}", list(queries)),
                        None => format!("Queries: {}", list(queries)),
                    }
                });
                None
            }
        }
    }

    /// `list: { data, title, caption?, image?, row_actions?, empty?, key? }`
    fn list(&mut self, body: &Value, path: &str, visible: Option<Value>) -> Option<Value> {
        let p = self.props(body, path, &["data", "title", "caption", "image", "row_actions", "empty", "key"], None)?;
        if !self.require(&p, &["data", "title"], path) {
            return None;
        }
        let (items, cols) = self.query_ref(&p["data"], &child(path, "data"))?;
        let mut props = Map::new();
        props.insert("items".into(), items);
        let item_key = match p.get("key").and_then(Value::as_str) {
            Some(k) => Some(k.to_string()),
            None if cols.is_empty() || cols.iter().any(|c| c == "id") => Some("id".to_string()),
            None => {
                let d = self.diags().warn(
                    "W_LIST_KEY",
                    path,
                    "the list query has no `id` column, so rows are keyed by position",
                );
                d.hint("Select `id` in the query, or set `key: <unique column>`");
                None
            }
        };
        if let Some(k) = item_key {
            props.insert("itemKey".into(), json!(k));
        }
        // Row templates see the query's columns.
        let saved = std::mem::replace(&mut self.cx.names.row, cols);
        let title = self.val(&p["title"], Scope::Row, &child(path, "title"));
        let caption = p.get("caption").map(|c| self.val(c, Scope::Row, &child(path, "caption")));
        // A thumbnail at the row's start: the app's own image files (`files/…`, clipboard history's), or `data:` images.
        let image = p.get("image").map(|c| self.val(c, Scope::Row, &child(path, "image")));
        let mut actions = Vec::new();
        for (i, a) in p.get("row_actions").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let apath = index(&child(path, "row_actions"), i);
            let Some(ap) = self.props(a, &apath, &["icon", "label", "action", "params"], None) else { continue };
            if !self.require(&ap, &["icon", "action"], &apath) {
                continue;
            }
            let Some(args) = self.cx.args(&ap, &apath, Scope::Row) else { continue };
            let label =
                ap.get("label").cloned().unwrap_or_else(|| json!(humanize(ap["action"].as_str().unwrap_or_default())));
            let icon = self.icon(&ap["icon"], "", &child(&apath, "icon")).unwrap_or(Value::Null);
            actions.push(json!({ "icon": icon, "label": label, "action": ap["action"], "args": args }));
        }
        self.cx.names.row = saved;
        props.insert("title".into(), title);
        if let Some(c) = caption {
            props.insert("caption".into(), c);
        }
        if let Some(i) = image {
            props.insert("image".into(), i);
        }
        if !actions.is_empty() {
            props.insert("rowActions".into(), Value::Array(actions));
        }
        self.copy(&p, &mut props, &[("empty", "empty")], path);
        Some(element("List", Value::Object(props), None, visible, None))
    }

    /// `ui: { type, props?, children?: [ { type, … } ], on?: { press: { action, params } } }`
    fn ui(&mut self, body: &Value, key: &str, path: &str, visible: Option<Value>) -> Option<Value> {
        let p = self.props(body, path, &["type", "props", "children", "on"], None)?;
        let ty = p.get("type").and_then(Value::as_str).unwrap_or_default();
        let Some((_, known)) = CATALOG.iter().find(|(c, _)| *c == ty) else {
            let comps = CATALOG.iter().map(|(c, _)| *c);
            let d = self.diags().error("E_UI_TYPE", child(path, "type"), format!("unknown component `{ty}`"));
            d.hint(format!("Components: {}", list(comps)));
            return None;
        };
        let props_path = child(path, "props");
        let mut props = Map::new();
        if let Some(raw) = p.get("props") {
            let raw = self.props(raw, &props_path, known, None)?;
            for (k, v) in &raw {
                if k == "icon" {
                    if let Some(i) = self.icon(v, "", &child(&props_path, k)) {
                        props.insert(k.clone(), i);
                    }
                } else if known.contains(&k.as_str()) {
                    let v = self.val(v, Scope::Element, &child(&props_path, k));
                    props.insert(k.clone(), v);
                }
            }
        }
        let on = match p.get("on").and_then(|o| o.get("press")).and_then(Value::as_object) {
            Some(press) => Some(json!({ "press": self.run(press, &child(path, "on.press"), Scope::Element)? })),
            None => None,
        };
        let children = match p.get("children").and_then(Value::as_array) {
            Some(kids) => {
                let mut keys = Vec::new();
                for (i, kid) in kids.iter().enumerate() {
                    let kpath = index(&child(path, "children"), i);
                    let kkey = format!("{key}/ui-{}", i + 1);
                    if self.slot(&kkey, &kpath, "ui", |b| b.ui(kid, &kkey, &kpath, None)).is_some() {
                        keys.push(kkey);
                    }
                }
                Some(keys)
            }
            None => matches!(ty, "Stack" | "Row" | "Card").then(Vec::new),
        };
        Some(element(ty, Value::Object(props), children, visible, on))
    }

    /// `visible: { when: today.total, gte: "{{settings.daily_goal}}", not?: true }`, `all: [..]`, `any: [..]`,
    /// `"{{state.show}}"` or a literal bool → json-render visibility conditions.
    fn visible(&mut self, v: &Value, path: &str) -> Option<Value> {
        match v {
            Value::Bool(_) => Some(v.clone()),
            Value::String(s) => self.state_ref(s, path).map(|p| json!({ "$state": p })),
            Value::Object(m) => {
                if let Some(all) = m.get("all").or_else(|| m.get("any")) {
                    let op = if m.contains_key("all") { "$and" } else { "$or" };
                    let conds: Option<Vec<Value>> = all
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .map(|(i, c)| self.visible(c, &index(path, i)))
                        .collect();
                    let mut m = Map::new();
                    m.insert(op.into(), Value::Array(conds?));
                    return Some(Value::Object(m));
                }
                let Some(when) = m.get("when").and_then(Value::as_str) else {
                    self.diags()
                        .error("E_VISIBLE", path, "visible needs `when: <binding path>`")
                        .hint("e.g. `visible: { when: today.total, gte: 100 }`");
                    return None;
                };
                let mut cond = Map::new();
                cond.insert("$state".into(), json!(self.state_ref(when, &child(path, "when"))?));
                for (k, val) in m {
                    match k.as_str() {
                        "when" => {}
                        "eq" | "neq" | "gt" | "gte" | "lt" | "lte" => {
                            let operand = match val {
                                Value::String(s) if s.contains("{{") => {
                                    json!({ "$state": self.state_ref(s, &child(path, k))? })
                                }
                                other => other.clone(),
                            };
                            cond.insert(k.clone(), operand);
                        }
                        "not" => {
                            cond.insert("not".into(), val.clone());
                        }
                        other => {
                            self.diags()
                                .error("E_VISIBLE", child(path, other), format!("unknown visibility key `{other}`"))
                                .hint("Keys: when, eq, neq, gt, gte, lt, lte, not, all, any");
                            return None;
                        }
                    }
                }
                Some(Value::Object(cond))
            }
            other => {
                self.diags().error("E_VISIBLE", path, format!("visible must be a condition map, got {}", kind(other)));
                None
            }
        }
    }

    /// A plain (unfiltered) binding path → absolute state path. Directives don't run inside visibility conditions.
    fn state_ref(&mut self, s: &str, path: &str) -> Option<String> {
        let wrapped = if s.contains("{{") { s.to_string() } else { format!("{{{{{s}}}}}") };
        match value::compile_str(&wrapped, Scope::Element, self.cx.names, path, self.cx.diags) {
            Value::Object(m) if m.contains_key("$state") => m["$state"].as_str().map(str::to_string),
            Value::Null => None,
            _ => {
                self.diags().error(
                    "E_VISIBLE",
                    path,
                    "visibility conditions take a plain binding (no filters or text)",
                );
                None
            }
        }
    }
}

/// Appends a filter to a bound value: `{"$state": p}` becomes `{"$x": {p, f, a?}}`, and a `$x` path read gets the
/// filter added to its chain. Literals and concatenations can't take one (`None`).
fn with_filter(value: &Value, name: &str, arg: Option<&str>) -> Option<Value> {
    let arg = arg.map(|a| a.parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| json!(a)));
    let mut ir = match value {
        Value::Object(m) if m.len() == 1 && m.get("$state").is_some_and(Value::is_string) => {
            let mut ir = Map::new();
            ir.insert("p".into(), m["$state"].clone());
            ir
        }
        Value::Object(m) if m.len() == 1 => match m.get("$x") {
            Some(Value::Object(ir)) if ir.contains_key("p") => ir.clone(),
            _ => return None,
        },
        _ => return None,
    };
    let mut names: Vec<Value> = match ir.remove("f") {
        Some(Value::Array(fs)) => fs,
        Some(f) => vec![f],
        None => Vec::new(),
    };
    let mut args: Vec<Value> = match ir.remove("a") {
        Some(Value::Array(a)) if names.len() > 1 => a,
        Some(a) => vec![a],
        None => vec![Value::Null; names.len()],
    };
    args.resize(names.len(), Value::Null);
    names.push(json!(name));
    args.push(arg.unwrap_or(Value::Null));
    if names.len() == 1 {
        ir.insert("f".into(), names.remove(0));
        if !args[0].is_null() {
            ir.insert("a".into(), args.remove(0));
        }
    } else {
        ir.insert("f".into(), Value::Array(names));
        if args.iter().any(|a| !a.is_null()) {
            ir.insert("a".into(), Value::Array(args));
        }
    }
    Some(json!({ "$x": ir }))
}

/// `/state/draft/reps` + `/state/draft/sets` → `/state/draft`.
fn common_parent(binds: &[String]) -> Option<String> {
    let first = binds.first()?;
    let parent = first.rsplit_once('/')?.0;
    (parent.len() > "/state".len() && binds.iter().all(|b| b.rsplit_once('/').is_some_and(|(p, _)| p == parent)))
        .then(|| parent.to_string())
}
