//! Small helpers shared by the compiler passes.

use serde_json::Value;

/// `snake_case` names: tables, columns, queries, actions, settings, state keys, views, block ids.
pub fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && s.len() <= 64
}

pub fn child(path: &str, key: &str) -> String {
    if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

pub fn index(path: &str, i: usize) -> String {
    format!("{path}[{i}]")
}

pub fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "a map",
    }
}

/// `90s`, `25m`, `1h30m`, `1.5h`, `500ms`, `2d`; a bare number means seconds.
pub fn duration_ms(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|s| *s >= 0.0).map(|s| (s * 1000.0).round() as u64),
        Value::String(s) => parse_duration_ms(s),
        _ => None,
    }
}

pub fn parse_duration_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(secs) = s.parse::<f64>() {
        return (secs >= 0.0).then(|| (secs * 1000.0).round() as u64);
    }
    let mut total = 0f64;
    let mut rest = s;
    while !rest.is_empty() {
        let num_len = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
        if num_len == 0 {
            return None;
        }
        let n: f64 = rest[..num_len].parse().ok()?;
        rest = &rest[num_len..];
        let unit_len = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
        let factor = match &rest[..unit_len] {
            "ms" => 1.0,
            "s" => 1_000.0,
            "m" => 60_000.0,
            "h" => 3_600_000.0,
            "d" => 86_400_000.0,
            _ => return None,
        };
        total += n * factor;
        rest = rest[unit_len..].trim_start();
    }
    Some(total.round() as u64)
}

/// `HH:MM` (24 h).
pub fn is_clock_time(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else { return false };
    h.len() <= 2 && m.len() == 2 && h.parse::<u8>().is_ok_and(|h| h < 24) && m.parse::<u8>().is_ok_and(|m| m < 60)
}

/// Turns `daily_goal` into `Daily goal`.
pub fn humanize(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The closest candidate within a small edit distance, for "did you mean" hints.
pub fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (name.len() / 3).max(1) + 1;
    candidates
        .into_iter()
        .map(|c| (levenshtein(name, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != *cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// SQL string literal.
pub fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Joins names for hints: `a, b, c` or `(none)`.
pub fn list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let v: Vec<&str> = items.into_iter().collect();
    if v.is_empty() { "(none)".into() } else { v.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration_ms("60m"), Some(3_600_000));
        assert_eq!(parse_duration_ms("1h30m"), Some(5_400_000));
        assert_eq!(parse_duration_ms("1.5h"), Some(5_400_000));
        assert_eq!(parse_duration_ms("500ms"), Some(500));
        assert_eq!(parse_duration_ms("90"), Some(90_000));
        assert_eq!(parse_duration_ms("5 min"), None);
        assert_eq!(parse_duration_ms("h"), None);
    }

    #[test]
    fn names_and_suggestions() {
        assert!(is_ident("per_hour") && is_ident("_x") && !is_ident("perHour") && !is_ident("1x") && !is_ident(""));
        assert_eq!(suggest("totl", ["total", "n"]), Some("total"));
        assert_eq!(suggest("zzz", ["total", "n"]), None);
        assert_eq!(humanize("daily_goal"), "Daily goal");
        assert!(is_clock_time("9:05") && is_clock_time("21:30") && !is_clock_time("24:00") && !is_clock_time("9"));
    }
}
