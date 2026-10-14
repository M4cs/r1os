//! The Value IR evaluator (docs/specs/compiled-app.md §3), ported from the runtime's `value.ts` for the host, which
//! formats what triggers show (notifications). Same forms and null-safe filters; formatting is English (US), which is
//! what the runtime's `Intl` gives on Max's machine.
//!
//!   { p: "/abs/path", f?, a? }   read a path (through `read`)
//!   { i: "field", f?, a? }       a row field: null outside a row ([`eval_row`] has one: a glance card's `each`)
//!   { t: [ "text", Value, … ] }  concatenate parts into a string
//!   { $state: "/p" }, { $x: Value }, and plain objects and arrays, evaluated field by field

use serde_json::{Map, Number, Value, json};

/// Local time for the date filters.
pub struct Format<'a> {
    /// Now, in ms since the Unix epoch.
    pub now_ms: i64,
    /// The local UTC offset at an instant, in ms (DST-correct, like `sql::Clock`).
    pub offset_ms: &'a dyn Fn(i64) -> i64,
}

impl Format<'_> {
    fn local(&self, ms: i64) -> i64 {
        ms + (self.offset_ms)(ms)
    }
}

const DAY_MS: i64 = 86_400_000;

fn only(m: &Map<String, Value>, keys: &[&str]) -> bool {
    m.keys().all(|k| keys.contains(&k.as_str()))
}

/// Evaluates a Value (or a structure containing Values). Never fails: unknown paths read as null.
pub fn eval(value: &Value, read: &mut dyn FnMut(&str) -> Value, fmt: &Format) -> Value {
    eval_row(value, &Value::Null, read, fmt)
}

/// [`eval`] for a row template: `{ i: "field" }` reads the row's field (a glance card's `each`, ADR-0042).
pub fn eval_row(value: &Value, row: &Value, read: &mut dyn FnMut(&str) -> Value, fmt: &Format) -> Value {
    let eval = |v: &Value, read: &mut dyn FnMut(&str) -> Value| eval_row(v, row, read, fmt);
    match value {
        Value::Array(items) => Value::Array(items.iter().map(|v| eval(v, read)).collect()),
        Value::Object(m) => {
            if let Some(Value::String(p)) = m.get("p")
                && only(m, &["p", "f", "a"])
            {
                return apply_filters(read(p), m.get("f"), m.get("a"), fmt);
            }
            if let Some(Value::String(field)) = m.get("i")
                && only(m, &["i", "f", "a"])
            {
                return apply_filters(read_field(row, field), m.get("f"), m.get("a"), fmt);
            }
            if let Some(Value::Array(parts)) = m.get("t")
                && only(m, &["t"])
            {
                let mut out = String::new();
                for part in parts {
                    out.push_str(&to_text(&eval(part, read)));
                }
                return Value::String(out);
            }
            if let Some(Value::String(p)) = m.get("$state")
                && only(m, &["$state"])
            {
                return read(p);
            }
            if let Some(x) = m.get("$x")
                && only(m, &["$x"])
            {
                return eval(x, read);
            }
            Value::Object(m.iter().map(|(k, v)| (k.clone(), eval(v, read))).collect())
        }
        other => other.clone(),
    }
}

/// A row's field, as the runtime's `readField` reads it: a column, a path inside the row (`hourly.4.temp`, or a pointer
/// `/hourly/4/temp`), or the row itself (`""`).
fn read_field(row: &Value, field: &str) -> Value {
    if let Some(v) = row.get(field) {
        return v.clone();
    }
    let pointer = if field.is_empty() || field.starts_with('/') { field.to_owned() } else { format!("/{}", field.replace('.', "/")) };
    read_pointer(row, &pointer).cloned().unwrap_or(Value::Null)
}

/// Reads a JSON pointer (`/a/0/b`); `~1`/`~0` escapes are honoured.
pub fn read_pointer<'v>(root: &'v Value, path: &str) -> Option<&'v Value> {
    if path.is_empty() || path == "/" {
        return Some(root);
    }
    let mut cur = root;
    for raw in path.strip_prefix('/').unwrap_or(path).split('/') {
        let seg = raw.replace("~1", "/").replace("~0", "~");
        cur = match cur {
            Value::Array(items) => items.get(seg.parse::<usize>().ok()?)?,
            Value::Object(m) => m.get(&seg)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Text for `{t}` concatenation, like the runtime's `toText`: null → "", numbers as JavaScript prints them.
pub fn to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map(js_number).unwrap_or_default(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// `String(n)` in JavaScript, for finite values: integral numbers print without a fraction.
fn js_number(n: f64) -> String {
    if !n.is_finite() {
        String::new()
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn apply_filters(value: Value, f: Option<&Value>, a: Option<&Value>, fmt: &Format) -> Value {
    match f {
        None | Some(Value::Null) => value,
        Some(Value::Array(names)) => {
            let args = a.and_then(Value::as_array);
            names.iter().enumerate().fold(value, |acc, (i, name)| {
                apply_filter(acc, name.as_str().unwrap_or_default(), args.and_then(|a| a.get(i)), fmt)
            })
        }
        Some(Value::String(name)) => apply_filter(value, name, a, fmt),
        Some(_) => value,
    }
}

/// One filter. Null input yields null (except `default`); unknown filters pass the value through.
pub fn apply_filter(value: Value, name: &str, arg: Option<&Value>, fmt: &Format) -> Value {
    if name == "default" {
        return if is_blank(&value) { arg.cloned().unwrap_or(Value::Null) } else { value };
    }
    if value.is_null() {
        return Value::Null;
    }
    let digits = arg.and_then(to_digits);
    let num = |value: Value, f: &dyn Fn(f64) -> Value| match to_number(&value) {
        Some(n) => f(n),
        None => value,
    };
    match name {
        "time" => format_time(&value, fmt).map(Value::String).unwrap_or(value),
        "date" => format_date(&value, fmt).map(Value::String).unwrap_or(value),
        "datetime" => format_datetime(&value, fmt).map(Value::String).unwrap_or(value),
        "relative" => format_relative(&value, fmt).map(Value::String).unwrap_or(value),
        "duration" => num(value, &|n| json!(format_duration(n))),
        "seconds" => num(value, &|n| json!(format_clock(n))),
        "number" => num(value, &|n| json!(format_number(n, digits))),
        "percent" => num(value, &|n| json!(format!("{}%", format_number(n * 100.0, Some(digits.unwrap_or(0)))))),
        "bytes" => num(value, &|n| json!(format_bytes(n))),
        "round" => num(value, &|n| {
            let m = 10f64.powi(digits.unwrap_or(0) as i32);
            Number::from_f64(((n + f64::EPSILON) * m).round() / m).map(Value::Number).unwrap_or(Value::Null)
        }),
        "upper" => json!(to_text(&value).to_uppercase()),
        "lower" => json!(to_text(&value).to_lowercase()),
        "plural" => {
            let word = arg.and_then(Value::as_str).unwrap_or_default().to_string();
            num(value, &|n| json!(pluralize(n, &word)))
        }
        _ => value,
    }
}

fn is_blank(v: &Value) -> bool {
    matches!(v, Value::Null) || v.as_str() == Some("")
}

fn to_digits(arg: &Value) -> Option<usize> {
    let n = match arg {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    n.is_finite().then(|| n.trunc().clamp(0.0, 20.0) as usize)
}

fn to_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|n| n.is_finite()),
        Value::String(s) if !s.trim().is_empty() => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// `Intl.NumberFormat("en-US")`: grouping, at most 2 decimals, or exactly `digits`.
pub fn format_number(n: f64, digits: Option<usize>) -> String {
    let fixed = format!("{:.*}", digits.unwrap_or(2), n.abs());
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let frac = if digits.is_some() { frac } else { frac.trim_end_matches('0') };
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let negative = n < 0.0 && (grouped.chars().any(|c| c != '0' && c != ',') || frac.chars().any(|c| c != '0'));
    let sign = if negative { "-" } else { "" };
    if frac.is_empty() { format!("{sign}{grouped}") } else { format!("{sign}{grouped}.{frac}") }
}

fn format_bytes(n: f64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let sign = if n < 0.0 { "-" } else { "" };
    let (mut v, mut unit) = (n.abs(), 0);
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let digits = if unit == 0 || v >= 10.0 { 0 } else { 1 };
    let text = format_number(v, Some(digits));
    // `maximumFractionDigits` in the runtime: "1.0 KB" prints as "1 KB".
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{sign}{text} {}", UNITS[unit])
}

/// `duration`: seconds → "45s", "5m", "1h 30m", "2d 3h".
pub fn format_duration(total: f64) -> String {
    let sign = if total < 0.0 { "-" } else { "" };
    let mut s = total.abs().round() as i64;
    if s == 0 {
        return "0s".into();
    }
    let d = s / 86_400;
    s -= d * 86_400;
    let h = s / 3600;
    s -= h * 3600;
    let m = s / 60;
    s -= m * 60;
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 && d == 0 {
        parts.push(format!("{m}m"));
    }
    if s > 0 && d == 0 && h == 0 {
        parts.push(format!("{s}s"));
    }
    parts.truncate(2);
    format!("{sign}{}", parts.join(" "))
}

/// `seconds`: seconds → clock text ("4:05", "1:02:03").
pub fn format_clock(total: f64) -> String {
    let sign = if total < 0.0 { "-" } else { "" };
    let s = total.abs().floor() as i64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{sign}{h}:{m:02}:{sec:02}") } else { format!("{sign}{m}:{sec:02}") }
}

/// `plural`: "1 set", "3 sets"; the word may be "person|people".
pub fn pluralize(n: f64, word: &str) -> String {
    let (one, many) = match word.split_once('|') {
        Some((one, many)) => (one.to_string(), many.to_string()),
        None => (word.to_string(), english_plural(word)),
    };
    let num = format_number(n, None);
    if one.is_empty() {
        return num;
    }
    format!("{num} {}", if n.abs() == 1.0 { one } else { many })
}

fn english_plural(w: &str) -> String {
    let lower = w.to_lowercase();
    let vowel_y = lower.ends_with("ay") || lower.ends_with("ey") || lower.ends_with("iy");
    let vowel_y = vowel_y || lower.ends_with("oy") || lower.ends_with("uy");
    if w.is_empty() {
        String::new()
    } else if lower.ends_with('y') && !vowel_y {
        format!("{}ies", &w[..w.len() - 1])
    } else if ["s", "x", "z", "ch", "sh"].iter().any(|end| lower.ends_with(end)) {
        format!("{w}es")
    } else {
        format!("{w}s")
    }
}

// ----- dates ---------------------------------------------------------------------------------------------------

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// An instant to format: UTC ms, or a local date-only value that must not shift through the zone.
enum Instant {
    Utc(i64),
    LocalDate(i64),
}

/// What the host stores (UTC ISO-8601 with ms), SQLite's `YYYY-MM-DD HH:MM:SS` (UTC), date-only strings (local) and
/// epoch numbers (ms, or seconds when < 1e11).
fn parse_instant(v: &Value) -> Option<Instant> {
    match v {
        Value::Number(n) => {
            let n = n.as_f64().filter(|n| n.is_finite())?;
            Some(Instant::Utc(if n.abs() < 1e11 { (n * 1000.0) as i64 } else { n as i64 }))
        }
        Value::String(s) if s.len() == 10 => days_from_date(s).map(|d| Instant::LocalDate(d * DAY_MS)),
        Value::String(s) => parse_utc(s).map(Instant::Utc),
        _ => None,
    }
}

/// `YYYY-MM-DD` → days since the epoch.
fn days_from_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (s[0..4].parse().ok()?, s[5..7].parse().ok()?, s[8..10].parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Days since the epoch → (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// ms → `2026-10-06T04:07:49.959Z`, the form the host stores timestamps in (compiled-app.md §6).
pub fn iso_utc(ms: i64) -> String {
    let (y, mo, d) = civil_from_days(ms.div_euclid(DAY_MS));
    let in_day = ms.rem_euclid(DAY_MS);
    let (h, mi, s, milli) = (in_day / 3_600_000, in_day / 60_000 % 60, in_day / 1000 % 60, in_day % 1000);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// `2026-10-06T04:07:49.959Z`, `2026-10-06 04:07:49` (UTC) and the like → ms.
fn parse_utc(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once(['T', ' '])?;
    let day = days_from_date(date)?;
    let mut parts = time.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next().map(str::parse).transpose().ok()?.unwrap_or(0.0);
    Some(day * DAY_MS + h * 3_600_000 + m * 60_000 + (sec * 1000.0).round() as i64)
}

/// "9:05 AM".
fn clock_12h(h: i64, m: i64) -> String {
    let (h12, half) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {half}")
}

/// Local (year, month, day, hour, minute) of an instant.
fn local_parts(at: &Instant, fmt: &Format) -> (i64, i64, i64, i64, i64) {
    let local = match *at {
        Instant::Utc(ms) => fmt.local(ms),
        Instant::LocalDate(ms) => ms,
    };
    let (y, mo, d) = civil_from_days(local.div_euclid(DAY_MS));
    let in_day = local.rem_euclid(DAY_MS) / 60_000;
    (y, mo, d, in_day / 60, in_day % 60)
}

fn format_time(v: &Value, fmt: &Format) -> Option<String> {
    // A time of day (setting type `time`): no zone conversion.
    if let Some(s) = v.as_str()
        && let Some((h, m)) = s.split_once(':')
        && h.len() <= 2
        && let (Ok(h), Ok(m)) = (h.parse::<i64>(), m.get(..2).unwrap_or(m).parse::<i64>())
        && (0..24).contains(&h)
        && (0..60).contains(&m)
    {
        return Some(clock_12h(h, m));
    }
    let (_, _, _, h, m) = local_parts(&parse_instant(v)?, fmt);
    Some(clock_12h(h, m))
}

fn date_text(at: &Instant, fmt: &Format) -> String {
    let (y, mo, d, _, _) = local_parts(at, fmt);
    let (this_year, _, _) = civil_from_days(fmt.local(fmt.now_ms).div_euclid(DAY_MS));
    let month = MONTHS[(mo - 1) as usize];
    if y == this_year { format!("{month} {d}") } else { format!("{month} {d}, {y}") }
}

fn format_date(v: &Value, fmt: &Format) -> Option<String> {
    Some(date_text(&parse_instant(v)?, fmt))
}

fn format_datetime(v: &Value, fmt: &Format) -> Option<String> {
    let at = parse_instant(v)?;
    let (_, _, _, h, m) = local_parts(&at, fmt);
    Some(format!("{}, {}", date_text(&at, fmt), clock_12h(h, m)))
}

/// `Intl.RelativeTimeFormat("en-US", { numeric: "auto", style: "short" })`, past 30 days a date.
fn format_relative(v: &Value, fmt: &Format) -> Option<String> {
    let at = parse_instant(v)?;
    let ms = match at {
        Instant::Utc(ms) => ms,
        Instant::LocalDate(ms) => ms - (fmt.offset_ms)(ms),
    };
    let diff = (ms - fmt.now_ms) as f64 / 1000.0;
    let abs = diff.abs();
    if abs < 45.0 {
        return Some("just now".into());
    }
    let rel = |n: f64, one_back: &str, one_ahead: &str, unit: &str| {
        let n = n.round() as i64;
        match n {
            -1 if !one_back.is_empty() => one_back.to_string(),
            1 if !one_ahead.is_empty() => one_ahead.to_string(),
            n if n < 0 => format!("{} {unit} ago", -n),
            n => format!("in {n} {unit}"),
        }
    };
    Some(if abs < 3600.0 {
        rel(diff / 60.0, "", "", "min.")
    } else if abs < 86_400.0 {
        rel(diff / 3600.0, "", "", "hr.")
    } else if abs < 604_800.0 {
        let n = (diff / 86_400.0).round();
        if n.abs() == 1.0 { rel(n, "yesterday", "tomorrow", "") } else { rel(n, "", "", "days") }
    } else if abs < 2_592_000.0 {
        rel(diff / 604_800.0, "last wk.", "next wk.", "wk.")
    } else {
        date_text(&at, fmt)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-06 12:00 UTC, in a UTC-4 zone (EDT).
    const NOW: i64 = 1_791_288_000_000;

    fn fmt_at<T>(f: impl FnOnce(&Format) -> T) -> T {
        let offset = |_: i64| -4 * 3_600_000;
        f(&Format { now_ms: NOW, offset_ms: &offset })
    }

    fn eval_with(v: Value, state: Value) -> Value {
        fmt_at(|fmt| eval(&v, &mut |p| read_pointer(&state, p).cloned().unwrap_or(Value::Null), fmt))
    }

    #[test]
    fn the_water_reminder_text() {
        let body =
            json!({ "t": [{ "p": "/q/today/0/total" }, "/", { "p": "/settings/daily_goal" }, " glasses today"] });
        let state = json!({ "q": { "today": [{ "total": 3, "n": 2 }] }, "settings": { "daily_goal": 8 } });
        assert_eq!(eval_with(body, state), json!("3/8 glasses today"));
        // Missing rows read as null, which prints as nothing (like the runtime).
        let body = json!({ "t": [{ "p": "/q/today/0/total" }, " so far"] });
        assert_eq!(eval_with(body, json!({ "q": { "today": [] } })), json!(" so far"));
        assert_eq!(to_text(&json!(3.0)), "3");
        assert_eq!(to_text(&json!(2.5)), "2.5");
    }

    #[test]
    fn filters_match_the_runtime() {
        let f = |v: Value, name: &str, arg: Option<Value>| fmt_at(|fmt| apply_filter(v, name, arg.as_ref(), fmt));
        assert_eq!(f(json!(1234.5), "number", None), json!("1,234.5"));
        assert_eq!(f(json!(1234567), "number", None), json!("1,234,567"));
        assert_eq!(f(json!(2), "number", Some(json!(1))), json!("2.0"));
        assert_eq!(f(json!(-0.004), "number", None), json!("0"), "the runtime's Intl prints -0; plain 0 reads better");
        assert_eq!(f(json!(0.426), "percent", None), json!("43%"));
        assert_eq!(f(json!(1536), "bytes", None), json!("1.5 KB"));
        assert_eq!(f(json!(1024), "bytes", None), json!("1 KB"));
        assert_eq!(f(json!(5400), "duration", None), json!("1h 30m"));
        assert_eq!(f(json!(183_600), "duration", None), json!("2d 3h"));
        assert_eq!(f(json!(245), "seconds", None), json!("4:05"));
        assert_eq!(f(json!(3723), "seconds", None), json!("1:02:03"));
        assert_eq!(f(json!(2.345), "round", Some(json!(2))), json!(2.35));
        assert_eq!(f(json!(1), "plural", Some(json!("set"))), json!("1 set"));
        assert_eq!(f(json!(3), "plural", Some(json!("entry"))), json!("3 entries"));
        assert_eq!(f(json!(2), "plural", Some(json!("person|people"))), json!("2 people"));
        assert_eq!(f(Value::Null, "default", Some(json!(0))), json!(0));
        assert_eq!(f(Value::Null, "number", None), Value::Null);
        assert_eq!(f(json!("n/a"), "number", None), json!("n/a"), "non-numbers pass through");
        assert_eq!(f(json!("Hi"), "upper", None), json!("HI"));
        // Chains: `{{x | number:1 | default:0}}`.
        let chain = json!({ "p": "/x", "f": ["number", "default"], "a": [1, 0] });
        assert_eq!(eval_with(chain.clone(), json!({ "x": 3 })), json!("3.0"));
        assert_eq!(eval_with(chain, json!({})), json!(0));
    }

    #[test]
    fn dates_are_local() {
        let f = |v: Value, name: &str| fmt_at(|fmt| apply_filter(v, name, None, fmt));
        // 13:05 UTC is 9:05 AM in UTC-4.
        assert_eq!(f(json!("2026-10-06T13:05:00.000Z"), "time"), json!("9:05 AM"));
        assert_eq!(f(json!("2026-10-06 13:05:00"), "time"), json!("9:05 AM"));
        assert_eq!(f(json!("21:30"), "time"), json!("9:30 PM"), "a time of day is not shifted");
        assert_eq!(f(json!("2026-10-06T03:00:00.000Z"), "date"), json!("Oct 5"), "11 PM the day before, locally");
        assert_eq!(f(json!("2025-02-01"), "date"), json!("Feb 1, 2025"));
        assert_eq!(f(json!("2026-10-06T16:00:00.000Z"), "datetime"), json!("Oct 6, 12:00 PM"));
        assert_eq!(f(json!(NOW - 10_000), "relative"), json!("just now"));
        assert_eq!(f(json!(NOW - 5 * 60_000), "relative"), json!("5 min. ago"));
        assert_eq!(f(json!(NOW + 2 * 3_600_000), "relative"), json!("in 2 hr."));
        assert_eq!(f(json!(NOW - 86_400_000), "relative"), json!("yesterday"));
        assert_eq!(f(json!(NOW - 3 * 86_400_000), "relative"), json!("3 days ago"));
        assert_eq!(iso_utc(parse_utc("2026-10-06T04:07:49.959Z").unwrap()), "2026-10-06T04:07:49.959Z");
        assert_eq!(days_from_date("1970-01-01"), Some(0));
        assert_eq!(civil_from_days(days_from_date("2024-02-29").unwrap()), (2024, 2, 29));
    }
}
