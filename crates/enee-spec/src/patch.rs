//! EneeSpec patches (docs/specs/enee-spec.md §7): an edit names only what changes, and the host merges it into the
//! current spec. A patch is a partial spec:
//!
//! - `app` merges by field. `permissions`, `settings`, `queries`, `state`, `services`, `actions`, `views` and
//!   `triggers` merge by entry: an entry replaces the one with its name or is added at the end of its section, and
//!   `name: null` removes it (a section left empty goes too).
//! - `data` merges by table and a table by column, so adding a column takes one line. `table: null` removes a table
//!   (which an edit's data check then refuses).
//! - `checks`, `logic`, `glance` and `components` are replaced whole. A section the spec doesn't have yet is added in section
//!   order, and `section: null` removes one. `enee` is ignored.
//!
//! The merge works on the text, so what a patch doesn't touch keeps its exact lines (formatting and comments), and
//! what it changes takes the patch's own lines, re-indented to fit. A section or table written in flow style
//! (`{ … }`) that a patch merges into is written out again in block style.

use serde_json::{Map, Value};
use serde_saphyr::granit_parser::{Event, Parser, StructureStyle};

/// Sections in the order a spec lists them (enee-spec.md §3).
const ORDER: [&str; 15] = [
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
/// Sections a patch replaces whole.
const WHOLE: [&str; 4] = ["checks", "logic", "glance", "components"];
/// How deep the outline goes: section, entry, and a data table's column.
const DEPTH: usize = 3;

/// Merges `patch` into `spec`, or says why it can't (the patch isn't a YAML map of sections).
pub fn apply(spec: &str, patch: &str) -> Result<String, String> {
    let spec = spec.replace("\r\n", "\n");
    let patch = patch.replace("\r\n", "\n");
    let changes = parse(&patch).map_err(|e| format!("the patch isn't valid YAML: {e}"))?;
    let Value::Object(changes) = changes else {
        return Err("the patch must be a YAML map of sections, like the spec".into());
    };
    let from = Outline::read(&patch);
    let mut text = Text::new(&spec);
    for (section, change) in &changes {
        if section != "enee" {
            text.section(&patch, &from, section, change)?;
        }
    }
    Ok(text.joined())
}

/// The YAML as the compiler reads it (YAML 1.2 core schema).
fn parse(src: &str) -> Result<Value, String> {
    let mut options = serde_saphyr::Options::default();
    options.strict_booleans = true;
    options.emit_comments = false;
    serde_saphyr::from_str_with_options(src, options)
        .map_err(|e| e.to_string().lines().next().unwrap_or("invalid YAML").to_owned())
}

/// A key the outline found: its path, where its lines are, and whether its value is a block mapping (whose own keys
/// are items too, down to [`DEPTH`]).
#[derive(Clone, Debug)]
struct Item {
    path: Vec<String>,
    /// The key's line (0-based) and column.
    start: usize,
    col: usize,
    /// Past the value's last line, leaving out trailing blank and comment lines.
    end: usize,
    block_map: bool,
}

/// Where a text's sections, their entries and the data tables' columns are.
#[derive(Debug, Default)]
struct Outline {
    items: Vec<Item>,
}

enum Frame {
    /// A mapping reached through mappings from the root: its path, and the item whose value comes next.
    Map { path: Vec<String>, key: Option<Option<usize>> },
    /// Anything else (a sequence, a mapping inside one, one too deep): its keys aren't items.
    Other,
}

impl Outline {
    fn read(src: &str) -> Self {
        let mut items: Vec<Item> = Vec::new();
        let mut stack: Vec<Frame> = Vec::new();
        for event in Parser::new_from_str(src) {
            let Ok((event, span)) = event else { break };
            match event {
                Event::Scalar(value, ..) => match stack.last_mut() {
                    Some(Frame::Map { path, key }) if key.is_none() => {
                        *key = Some((path.len() < DEPTH).then(|| {
                            let mut path = path.clone();
                            path.push(value.into_owned());
                            let start = span.start.line().saturating_sub(1);
                            items.push(Item { path, start, col: span.start.col(), end: 0, block_map: false });
                            items.len() - 1
                        }));
                    }
                    _ => advance(&mut stack),
                },
                Event::Alias(_) => advance(&mut stack),
                Event::MappingStart(style, ..) => {
                    let frame = match stack.last() {
                        None => Frame::Map { path: Vec::new(), key: None },
                        Some(Frame::Map { key: Some(Some(item)), .. }) if style == StructureStyle::Block => {
                            items[*item].block_map = true;
                            Frame::Map { path: items[*item].path.clone(), key: None }
                        }
                        _ => Frame::Other,
                    };
                    stack.push(frame);
                }
                Event::SequenceStart(..) => stack.push(Frame::Other),
                Event::MappingEnd | Event::SequenceEnd => {
                    stack.pop();
                    advance(&mut stack);
                }
                _ => {}
            }
        }
        let lines: Vec<&str> = src.split('\n').collect();
        let count = if src.ends_with('\n') { lines.len() - 1 } else { lines.len() };
        for i in 0..items.len() {
            let depth = items[i].path.len();
            let next = items[i + 1..].iter().find(|later| later.path.len() <= depth).map_or(count, |later| later.start);
            let mut end = next.max(items[i].start + 1);
            while end > items[i].start + 1 && {
                let line = lines[end - 1].trim();
                line.is_empty() || line.starts_with('#')
            } {
                end -= 1;
            }
            items[i].end = end;
        }
        Outline { items }
    }

    fn get(&self, path: &[&str]) -> Option<&Item> {
        self.items.iter().find(|item| item.path.iter().map(String::as_str).eq(path.iter().copied()))
    }

    /// The items directly under `path`.
    fn children<'a>(&'a self, path: &'a [&str]) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().filter(move |item| {
            item.path.len() == path.len() + 1
                && item.path.iter().map(String::as_str).zip(path.iter().copied()).all(|(a, b)| a == b)
        })
    }
}

/// A value finished: the map it was in waits for its next key.
fn advance(stack: &mut [Frame]) {
    if let Some(Frame::Map { key, .. }) = stack.last_mut() {
        *key = None;
    }
}

/// The spec being patched: its lines, re-outlined after each change.
struct Text {
    lines: Vec<String>,
    newline: bool,
    outline: Outline,
}

impl Text {
    fn new(src: &str) -> Self {
        let newline = src.ends_with('\n');
        let body = src.strip_suffix('\n').unwrap_or(src);
        let lines = if body.is_empty() { Vec::new() } else { body.split('\n').map(str::to_owned).collect() };
        let outline = Outline::read(src);
        Text { lines, newline, outline }
    }

    fn joined(&self) -> String {
        let mut text = self.lines.join("\n");
        if self.newline || !text.is_empty() {
            text.push('\n');
        }
        text
    }

    /// Replaces lines `from..to` with `new`, and outlines the result again.
    fn splice(&mut self, from: usize, to: usize, new: Vec<String>) {
        self.lines.splice(from..to, new);
        self.outline = Outline::read(&self.joined());
    }

    /// The spec's value at `path` (for merges the text can't do line by line).
    fn value(&self, path: &[&str]) -> Value {
        let root = parse(&self.joined()).unwrap_or(Value::Null);
        path.iter().fold(root, |value, key| value.get(*key).cloned().unwrap_or(Value::Null))
    }

    fn section(&mut self, patch: &str, from: &Outline, name: &str, change: &Value) -> Result<(), String> {
        let path = [name];
        let Some(current) = self.outline.get(&path).cloned() else {
            if !change.is_null() {
                let new = lines_of(patch, from.get(&path), name, change, 0);
                let at = self.section_slot(name);
                self.splice(at, at, new);
            }
            return Ok(());
        };
        if change.is_null() {
            self.splice(current.start, current.end, Vec::new());
            return Ok(());
        }
        let Value::Object(entries) = change else {
            let new = lines_of(patch, from.get(&path), name, change, 0);
            self.splice(current.start, current.end, new);
            return Ok(());
        };
        if WHOLE.contains(&name) {
            let new = lines_of(patch, from.get(&path), name, change, 0);
            self.splice(current.start, current.end, new);
            return Ok(());
        }
        if !current.block_map {
            // A flow-style or empty section: merged as values, written out in block style.
            let merged = merge_section(name, self.value(&path), entries);
            let mut new = Vec::new();
            emit(name, &merged, 0, &mut new);
            self.splice(current.start, current.end, new);
            return Ok(());
        }
        for (entry, change) in entries {
            self.entry(patch, from, name, entry, change);
        }
        Ok(())
    }

    /// One entry of a block-style section.
    fn entry(&mut self, patch: &str, from: &Outline, section: &str, name: &str, change: &Value) {
        let path = [section, name];
        let indent = self.child_indent(&[section]);
        let current = self.outline.get(&path).cloned();
        if change.is_null() {
            if let Some(current) = current {
                self.splice(current.start, current.end, Vec::new());
            }
            // Its last entry gone, the section goes too: an empty `triggers:` doesn't compile.
            let empty = match self.value(&[section]) {
                Value::Null => true,
                Value::Object(entries) => entries.is_empty(),
                _ => false,
            };
            if empty && let Some(left) = self.outline.get(&[section]).cloned() {
                self.splice(left.start, left.end, Vec::new());
            }
            return;
        }
        // A data table merges by column.
        if section == "data"
            && let (Some(current), Value::Object(columns)) = (&current, change)
            && self.value(&path).is_object()
        {
            if current.block_map {
                for (column, change) in columns {
                    self.column(patch, from, name, column, change);
                }
            } else {
                let merged = merge_map(self.value(&path), columns);
                let mut new = Vec::new();
                emit(name, &merged, indent, &mut new);
                self.splice(current.start, current.end, new);
            }
            return;
        }
        let new = lines_of(patch, from.get(&path), name, change, indent);
        match current {
            Some(current) => self.splice(current.start, current.end, new),
            None => {
                let at = self.section_end(&[section]);
                self.splice(at, at, new);
            }
        }
    }

    /// One column of a block-style data table.
    fn column(&mut self, patch: &str, from: &Outline, table: &str, name: &str, change: &Value) {
        let path = ["data", table, name];
        let indent = self.child_indent(&["data", table]);
        let current = self.outline.get(&path).cloned();
        if change.is_null() {
            if let Some(current) = current {
                self.splice(current.start, current.end, Vec::new());
            }
            return;
        }
        let new = lines_of(patch, from.get(&path), name, change, indent);
        match current {
            Some(current) => self.splice(current.start, current.end, new),
            None => {
                let at = self.section_end(&["data", table]);
                self.splice(at, at, new);
            }
        }
    }

    /// The indent of a block mapping's keys (two more than its own key when it has none yet).
    fn child_indent(&self, path: &[&str]) -> usize {
        match self.outline.children(path).next() {
            Some(child) => child.col,
            None => self.outline.get(path).map_or(0, |item| item.col + 2),
        }
    }

    /// The line after a block mapping's last child: where a new one goes.
    fn section_end(&self, path: &[&str]) -> usize {
        let item = self.outline.get(path);
        self.outline
            .children(path)
            .map(|child| child.end)
            .max()
            .or(item.map(|item| item.end))
            .unwrap_or(self.lines.len())
    }

    /// Where a new section goes: before the first section that comes after it, else at the end.
    fn section_slot(&self, name: &str) -> usize {
        let rank = |section: &str| ORDER.iter().position(|s| *s == section).unwrap_or(ORDER.len());
        self.outline
            .children(&[])
            .filter(|section| rank(&section.path[0]) > rank(name))
            .map(|section| section.start)
            .min()
            .unwrap_or(self.lines.len())
    }
}

/// The lines for `name: change` at `indent`: the patch's own lines for it when it has them (re-indented), else
/// written out in block style.
fn lines_of(patch: &str, item: Option<&Item>, name: &str, change: &Value, indent: usize) -> Vec<String> {
    match item {
        Some(item) => {
            let lines: Vec<&str> = patch.split('\n').collect();
            reindent(&lines[item.start..item.end], item.col, indent)
        }
        None => {
            let mut out = Vec::new();
            emit(name, change, indent, &mut out);
            out
        }
    }
}

/// Shifts lines from indent `from` to indent `to`, keeping their relative indentation (block scalars stay intact).
fn reindent(lines: &[&str], from: usize, to: usize) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                return String::new();
            }
            let spaces = line.len() - line.trim_start_matches(' ').len();
            let rest = &line[spaces.min(from)..];
            format!("{}{rest}", " ".repeat(to))
        })
        .collect()
}

/// A section merged as values: `app` and the named sections by entry, `data` by table and column.
fn merge_section(name: &str, current: Value, entries: &Map<String, Value>) -> Value {
    let mut merged = match current {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    for (key, change) in entries {
        match (name, merged.get(key), change) {
            (_, _, Value::Null) => {
                merged.remove(key);
            }
            ("data", Some(Value::Object(table)), Value::Object(columns)) => {
                let table = merge_map(Value::Object(table.clone()), columns);
                merged.insert(key.clone(), table);
            }
            _ => {
                merged.insert(key.clone(), change.clone());
            }
        }
    }
    Value::Object(merged)
}

/// A map with `changes` merged in by key (`null` removes).
fn merge_map(current: Value, changes: &Map<String, Value>) -> Value {
    let mut merged = match current {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    for (key, change) in changes {
        if change.is_null() {
            merged.remove(key);
        } else {
            merged.insert(key.clone(), change.clone());
        }
    }
    Value::Object(merged)
}

/// `key: value` in block style at `indent`. Collections inside sequences are written in flow style (JSON).
fn emit(key: &str, value: &Value, indent: usize, out: &mut Vec<String>) {
    let pad = " ".repeat(indent);
    let key = scalar(key);
    match value {
        Value::Object(map) if !map.is_empty() => {
            out.push(format!("{pad}{key}:"));
            for (k, v) in map {
                emit(k, v, indent + 2, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            out.push(format!("{pad}{key}:"));
            for item in items {
                out.push(format!("{pad}  - {}", inline(item)));
            }
        }
        other => out.push(format!("{pad}{key}: {}", inline(other))),
    }
}

/// A value on one line: a plain or quoted scalar, or a collection in flow style (JSON, which YAML reads).
fn inline(value: &Value) -> String {
    match value {
        Value::String(s) => scalar(s),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
        other => other.to_string(),
    }
}

/// A string as a YAML scalar: plain when that reads back as the same string, else double-quoted.
fn scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.trim() == s
        && !s.contains(['\n', '\r', '\t'])
        && !s.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`',
        ])
        && !s.contains(": ")
        && !s.contains(" #")
        && !s.ends_with(':')
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off" | "y" | "n"
        )
        && s.parse::<f64>().is_err();
    if plain { s.to_owned() } else { serde_json::to_string(s).unwrap_or_default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, compile};

    const WATER: &str = include_str!("../../../fixtures/apps/water.enee.yaml");

    /// The lines of `after` that aren't in `before`, and the reverse.
    fn diff(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
        let (b, a): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
        let added = a.iter().filter(|l| !b.contains(l)).map(|l| (*l).to_owned()).collect();
        let removed = b.iter().filter(|l| !a.contains(l)).map(|l| (*l).to_owned()).collect();
        (added, removed)
    }

    fn compiles(spec: &str) {
        let out = compile(spec, &Options::default());
        assert!(out.ok, "{:?}\n{spec}", out.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn an_entry_replaces_its_namesake_and_nothing_else_moves() {
        let patch = "actions:\n  drink:\n    params: { count: int }\n    do:\n      - sql: INSERT INTO glasses (count) VALUES (:count)\n      - toast: \"+{{params.count}} 💧\"\n";
        let patched = apply(WATER, patch).unwrap();
        let (added, removed) = diff(WATER, &patched);
        assert_eq!(added, ["      - toast: \"+{{params.count}} 💧\""]);
        assert_eq!(removed, ["      - toast: \"+{{params.count}} glass\""]);
        compiles(&patched);
    }

    #[test]
    fn new_entries_go_at_the_end_of_their_section_and_null_removes() {
        let patch = "queries:\n  week: SELECT sum(count) AS total FROM glasses WHERE is_this_week(at)\nactions:\n  undo: null\n";
        let patched = apply(WATER, patch).unwrap();
        let lines: Vec<&str> = patched.lines().collect();
        let last = lines.iter().position(|l| l.starts_with("  last:")).unwrap();
        assert_eq!(lines[last + 1], "  week: SELECT sum(count) AS total FROM glasses WHERE is_this_week(at)");
        assert!(!patched.contains("  undo:\n") && !patched.contains("Undone"));
        // The view's Undo button and a check still use it: the compiler says so, and a repair turn fixes it.
        assert!(!compile(&patched, &Options::default()).ok);
    }

    #[test]
    fn a_section_s_last_entry_takes_the_section_with_it() {
        let patch = "permissions:\n  notify: null\n  schedule: null\ntriggers:\n  reminder: null\n";
        let patched = apply(WATER, patch).unwrap();
        assert!(!patched.contains("permissions:") && !patched.contains("triggers:"), "{patched}");
        assert!(patched.contains("settings:\n") && patched.contains("checks:\n"));
        compiles(&patched);
    }

    #[test]
    fn a_table_merges_by_column() {
        let patch = "data:\n  glasses:\n    size: enum(small|large) = small\n";
        let patched = apply(WATER, patch).unwrap();
        assert!(patched.contains(
            "  glasses:\n    count: int! = 1\n    at: datetime = now\n    size: enum(small|large) = small\n  _indexes"
        ));
        compiles(&patched);
        // A table written in flow style comes back in block style, with its columns.
        let flow = WATER.replace(
            "  glasses:\n    count: int! = 1\n    at: datetime = now\n",
            "  glasses: { count: int! = 1, at: datetime = now }\n",
        );
        let patched = apply(&flow, patch).unwrap();
        assert!(patched.contains(
            "  glasses:\n    count: int! = 1\n    at: datetime = now\n    size: enum(small|large) = small\n"
        ));
        compiles(&patched);
    }

    #[test]
    fn app_merges_by_field_even_in_flow_style() {
        let patched = apply(WATER, "app: { icon: drop-half }\n").unwrap();
        assert!(patched.contains("app:\n  id: water-log\n  name: Water\n  icon: drop-half\n  archetype: tracker\n"));
        assert!(patched.contains("  description: Glasses of water per day.\n"));
        compiles(&patched);
    }

    #[test]
    fn whole_sections_new_sections_and_indentation() {
        // `checks` is replaced whole.
        let patch = "checks:\n  - do: { action: drink, params: { count: 2 } }\n    expect: { today.total: 2 }\n";
        let patched = apply(WATER, patch).unwrap();
        assert!(
            patched.ends_with(
                "checks:\n  - do: { action: drink, params: { count: 2 } }\n    expect: { today.total: 2 }\n"
            )
        );
        compiles(&patched);
        // A missing section goes where the order puts it, as the patch wrote it.
        let no_triggers = WATER.replace(&WATER[WATER.find("triggers:").unwrap()..WATER.find("checks:").unwrap()], "");
        let patch = "triggers:\n    nudge:\n        every: 3h\n        do:\n            - notify: { title: Water? }\n";
        let patched = apply(&no_triggers, patch).unwrap();
        let at = patched.find("triggers:\n    nudge:\n        every: 3h\n").expect("added");
        assert!(at < patched.find("checks:").unwrap(), "in section order");
        compiles(&patched);
        // An entry takes its section's indent; its own nesting stays as the patch wrote it.
        let patch = "actions:\n    refill:\n        do:\n            - sql: INSERT INTO glasses (count) VALUES (1)\n";
        let patched = apply(WATER, patch).unwrap();
        assert!(
            patched.contains("  refill:\n      do:\n          - sql: INSERT INTO glasses (count) VALUES (1)\nviews:")
        );
        compiles(&patched);
    }

    #[test]
    fn views_are_replaced_by_name_and_bad_patches_say_why() {
        let patch = "views:\n  main:\n    - stat: { label: Today, value: \"{{today.total}}\" }\n";
        let patched = apply(WATER, patch).unwrap();
        assert!(
            patched.contains("views:\n  main:\n    - stat: { label: Today, value: \"{{today.total}}\" }\ntriggers:")
        );
        assert!(apply(WATER, "- not: a map").unwrap_err().contains("map of sections"));
        assert!(apply(WATER, "views: { main: [ }").unwrap_err().contains("isn't valid YAML"));
    }

    #[test]
    fn scalars_are_quoted_only_when_they_must_be() {
        assert_eq!(scalar("int! = 1"), "int! = 1");
        assert_eq!(scalar("enum(a|b) = a"), "enum(a|b) = a");
        assert_eq!(scalar("{{today.total}}"), "\"{{today.total}}\"");
        assert_eq!(scalar("yes"), "\"yes\"");
        assert_eq!(scalar("12"), "\"12\"");
        assert_eq!(scalar("a: b"), "\"a: b\"");
    }
}
