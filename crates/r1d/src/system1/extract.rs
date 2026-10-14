//! Ported from eneeware's crates/enee-judge/src/extract.rs at 61bc7a7 (ADR-0032): System One's arguments come
//! from code, not the model. Unchanged apart from this note; eneeware's docs below speak of Jev, its
//! hosted judge, where r1os has an on-device embedder. Kept whole: times, dates and text spans are for what System
//! One takes on next (alarms, notes), so not all of it is used yet.
#![allow(dead_code)]
//!
//! Command arguments that code finds (research/jev-typesafe.md, spike S9). Jev can't write text or do arithmetic, so
//! code lists every value a command could mean, and a judgment only *picks* one when more than one fits:
//!
//! - [`numbers`]: digits and number words, with their units ("500ml", "two hundred and fifty", "half an hour").
//! - [`articles`]: "a glass", which may count one; only a judgment can tell it from "a set of pushups".
//! - [`times`]: times of day ("7am", "11:30 pm", "23:15", "noon").
//! - [`dates`]: dates ("yesterday", "3 days ago", "next friday", "dec 25", "2026-10-12"), from today's.
//! - [`spans`]: runs of the command's words as typed, for text arguments.
//! - [`note_text`]: what follows a colon or a leading note phrase.

use serde::Serialize;

/// The unit a number was written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Bare,
    Seconds,
    Minutes,
    Hours,
    Ml,
    Liters,
    Money,
    /// A time of day ("7am", "3:30", "at 7"); never a number argument.
    Clock,
    Percent,
    Other,
}

/// What a numeric argument measures, so code can convert the unit it was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    Count,
    Seconds,
    Minutes,
    Hours,
    Ml,
    Liters,
    Money,
}

/// A number found in a command.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    /// The words it was read from, e.g. `1.5 liters` or `twenty-five`.
    pub surface: String,
    pub value: f64,
    pub unit: Unit,
}

/// Every number in `command`, in order, deduplicated by surface text.
pub fn numbers(command: &str) -> Vec<Candidate> {
    let tokens = tokenize(command);
    let mut found: Vec<Candidate> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let Some((value, used, mut unit)) = number_at(&tokens, index) else {
            index += 1;
            continue;
        };
        let mut end = index + used;
        if unit == Unit::Bare
            && let Some(word_unit) = tokens.get(end).and_then(|token| unit_word(token))
        {
            unit = word_unit;
            end += 1;
        }
        if unit == Unit::Bare && index > 0 && tokens[index - 1] == "at" {
            unit = Unit::Clock;
        }
        let surface = tokens[index..end].join(" ");
        if !found.iter().any(|candidate| candidate.surface == surface) {
            found.push(Candidate { surface, value, unit });
        }
        index = end;
    }
    found
}

/// Words an article may count one of: "a glass", "an apple". Weak candidates, never taken without a judgment: "a
/// glass of water" counts one glass, but "a set of pushups" doesn't count the pushups. An article before a unit or a
/// number word is part of a number already ("an hour", "a dozen").
pub fn articles(command: &str) -> Vec<Candidate> {
    let tokens = tokenize(command);
    let mut found: Vec<Candidate> = Vec::new();
    for pair in tokens.windows(2) {
        let [article, noun] = pair else { continue };
        if !matches!(article.as_str(), "a" | "an")
            || unit_word(noun).is_some()
            || word(noun).is_some()
            || noun.chars().next().is_none_or(|c| !c.is_alphabetic())
            || ["few", "couple", "lot", "bit", "little", "half"].contains(&noun.as_str())
        {
            continue;
        }
        let surface = format!("{article} {noun}");
        if !found.iter().any(|candidate| candidate.surface == surface) {
            found.push(Candidate { surface, value: 1.0, unit: Unit::Bare });
        }
    }
    found
}

/// The value `candidate` gives an argument measuring `measure`, after unit conversion, or `None` if its unit doesn't
/// fit, it isn't positive, or an `integer` argument would get a fraction.
pub fn value(candidate: &Candidate, measure: Measure, integer: bool) -> Option<f64> {
    use Measure as M;
    use Unit as U;
    let v = candidate.value;
    let value = match (measure, candidate.unit) {
        (_, U::Bare) => v,
        (M::Seconds, U::Seconds)
        | (M::Minutes, U::Minutes)
        | (M::Hours, U::Hours)
        | (M::Ml, U::Ml)
        | (M::Liters, U::Liters)
        | (M::Money, U::Money) => v,
        (M::Seconds, U::Minutes) => v * 60.0,
        (M::Seconds, U::Hours) => v * 3600.0,
        (M::Minutes, U::Hours) => v * 60.0,
        (M::Minutes, U::Seconds) => v / 60.0,
        (M::Hours, U::Minutes) => v / 60.0,
        (M::Ml, U::Liters) => v * 1000.0,
        (M::Liters, U::Ml) => v / 1000.0,
        _ => return None,
    };
    let integral = (value - value.round()).abs() < 1e-9;
    (value > 0.0 && (!integer || integral)).then_some(value)
}

/// The candidates that fit an argument, with their converted values.
pub fn eligible(candidates: &[Candidate], measure: Measure, integer: bool) -> Vec<(&Candidate, f64)> {
    candidates.iter().filter_map(|candidate| value(candidate, measure, integer).map(|v| (candidate, v))).collect()
}

/// A JSON number for an argument (integers stay integers).
pub fn json_number(value: f64, integer: bool) -> serde_json::Value {
    if integer { serde_json::json!(value.round() as i64) } else { serde_json::json!(value) }
}

/// A time of day found in a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Time {
    /// The words it was read from, e.g. `11 pm`.
    pub surface: String,
    /// 24-hour `HH:MM`.
    pub hhmm: String,
}

/// Every time of day in `command`, in order: `7am`, `7:30 pm`, `23:15`, `noon`, `midnight`. A bare number ("at 7")
/// isn't one: it could be morning or evening.
pub fn times(command: &str) -> Vec<Time> {
    let tokens = tokenize(command);
    let mut found: Vec<Time> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        let next = tokens.get(index + 1).map(String::as_str);
        let (clock, used) = match token {
            "noon" | "midday" => (Some((12, 0)), 1),
            "midnight" => (Some((0, 0)), 1),
            _ => match clock_parts(token) {
                Some((hour, minute, "")) if next.and_then(meridiem).is_some() => {
                    (to_24h(hour, minute, next.and_then(meridiem)), 2)
                }
                Some((hour, minute, "")) if token.contains(':') => (to_24h(hour, minute, None), 1),
                Some((hour, minute, rest)) if meridiem(rest).is_some() => (to_24h(hour, minute, meridiem(rest)), 1),
                _ => (None, 1),
            },
        };
        if let Some((hour, minute)) = clock {
            let surface = tokens[index..index + used].join(" ");
            if !found.iter().any(|time| time.surface == surface) {
                found.push(Time { surface, hhmm: format!("{hour:02}:{minute:02}") });
            }
        }
        index += used;
    }
    found
}

/// A date found in a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Date {
    /// The words it was read from, e.g. `next friday`.
    pub surface: String,
    /// `YYYY-MM-DD`.
    pub iso: String,
}

const WEEKDAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Every date in `command`, in order, reckoned from `today` (`YYYY-MM-DD`, the user's local date):
/// - `today`, `tonight`, `yesterday`, `tomorrow`;
/// - `3 days ago`, `in 2 weeks`;
/// - a weekday: the last one (today counts), or the coming one after `next`;
/// - a month and day (`dec 25`, `25th of december`): the nearest one;
/// - `2026-10-12`, and `10/12` or `10/12/2026` (month first).
pub fn dates(command: &str, today: &str) -> Vec<Date> {
    let Some(today) = day_number(today) else { return Vec::new() };
    // Slashes stay: "12/25".
    let tokens = tokenize_with(command, "$€%.:'-/");
    let word = |i: usize| tokens.get(i).map(String::as_str).unwrap_or("");
    let mut found: Vec<Date> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let (day, used) = match word(index) {
            "today" | "tonight" => (Some(today), 1),
            "yesterday" => (Some(today - 1), 1),
            "tomorrow" => (Some(today + 1), 1),
            "next" | "last" | "this" if weekday(word(index + 1)).is_some() => {
                let wanted = weekday(word(index + 1)).unwrap_or_default();
                let day = match word(index) {
                    "next" => coming(today, wanted),
                    _ => previous(today, wanted),
                };
                (Some(day), 2)
            }
            w if weekday(w).is_some() => (Some(previous(today, weekday(w).unwrap_or_default())), 1),
            "in" => match (count(word(index + 1)), period(word(index + 2))) {
                (Some(n), Some(days)) => (Some(today + n * days), 3),
                _ => (None, 1),
            },
            w => match (count(w), period(word(index + 1)), word(index + 2)) {
                (Some(n), Some(days), "ago") => (Some(today - n * days), 3),
                _ => match month_day(&tokens, index) {
                    Some((month, day, year, used)) => (nearest(today, month, day, year), used),
                    None => (iso_or_slashed(w, today), 1),
                },
            },
        };
        if let Some(day) = day {
            let surface = tokens[index..index + used].join(" ");
            if !found.iter().any(|d| d.surface == surface) {
                found.push(Date { surface, iso: iso(day) });
            }
        }
        index += used;
    }
    found
}

fn weekday(word: &str) -> Option<i64> {
    let word = word.trim_end_matches('s'); // "mondays"
    WEEKDAYS
        .iter()
        .position(|d| *d == word || (word.len() >= 3 && d.starts_with(word) && word.len() <= 4))
        .map(|i| i as i64)
}

fn month(word: &str) -> Option<u32> {
    let word = word.trim_end_matches('.');
    MONTHS.iter().position(|m| *m == word || (word.len() >= 3 && m.starts_with(word))).map(|i| i as u32 + 1)
}

/// A small count, in digits or words ("3", "two", "a").
fn count(word: &str) -> Option<i64> {
    const SMALL: [&str; 13] =
        ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
    match word {
        "a" | "an" => Some(1),
        _ => word
            .parse::<i64>()
            .ok()
            .filter(|n| (0..=400).contains(n))
            .or_else(|| SMALL.iter().position(|w| *w == word).map(|n| n as i64)),
    }
}

/// Days in a `day`/`week` period word.
fn period(word: &str) -> Option<i64> {
    match word {
        "day" | "days" => Some(1),
        "week" | "weeks" => Some(7),
        _ => None,
    }
}

/// A day of the month: `25`, `25th`, `1st`.
fn day_of_month(word: &str) -> Option<u32> {
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &word[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
        return None;
    }
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

/// `dec 25`, `december 25th, 2026`, `25 dec`, `25th of december` → (month, day, year, tokens used).
fn month_day(tokens: &[String], index: usize) -> Option<(u32, u32, Option<i64>, usize)> {
    let word = |i: usize| tokens.get(i).map(String::as_str).unwrap_or("");
    let year = |i: usize| word(i).parse::<i64>().ok().filter(|y| (1900..=2200).contains(y));
    if let (Some(month), Some(day)) = (month(word(index)), day_of_month(word(index + 1))) {
        return Some(match year(index + 2) {
            Some(y) => (month, day, Some(y), 3),
            None => (month, day, None, 2),
        });
    }
    let day = day_of_month(word(index))?;
    let (month, used) = match (month(word(index + 1)), word(index + 1), month(word(index + 2))) {
        (Some(m), _, _) => (m, 2),
        (None, "of", Some(m)) => (m, 3),
        _ => return None,
    };
    Some(match year(index + used) {
        Some(y) => (month, day, Some(y), used + 1),
        None => (month, day, None, used),
    })
}

/// `2026-10-12`, `10/12`, `10/12/2026`.
fn iso_or_slashed(word: &str, today: i64) -> Option<i64> {
    let parts: Vec<&str> = word.split(['-', '/']).collect();
    let n = |s: &str| s.parse::<i64>().ok();
    match parts.as_slice() {
        [y, m, d] if word.contains('-') && y.len() == 4 => from_parts(n(y)?, n(m)?, n(d)?),
        [m, d] if word.contains('/') => {
            let (month, day) = (u32::try_from(n(m)?).ok()?, u32::try_from(n(d)?).ok()?);
            nearest(today, month, day, None)
        }
        [m, d, y] if word.contains('/') => {
            let y = n(y)?;
            from_parts(if y < 100 { 2000 + y } else { y }, n(m)?, n(d)?)
        }
        _ => None,
    }
}

fn from_parts(year: i64, month: i64, day: i64) -> Option<i64> {
    let (month, day) = (u32::try_from(month).ok()?, u32::try_from(day).ok()?);
    valid(year, month, day).then(|| days_from_civil(year, month, day))
}

fn valid(year: i64, month: u32, day: u32) -> bool {
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// The occurrence of `month`/`day` nearest to `today` (this year's, last year's or next year's), or in `year`.
fn nearest(today: i64, month: u32, day: u32, year: Option<i64>) -> Option<i64> {
    let this = civil_from_days(today).0;
    let years: Vec<i64> = match year {
        Some(y) => vec![y],
        None => vec![this - 1, this, this + 1],
    };
    years
        .into_iter()
        .filter(|y| valid(*y, month, day))
        .map(|y| days_from_civil(y, month, day))
        .min_by_key(|d| (d - today).abs())
}

/// The last `weekday` (0 = Monday) on or before `today`.
fn previous(today: i64, weekday: i64) -> i64 {
    today - (day_of_week(today) - weekday).rem_euclid(7)
}

/// The next `weekday` after `today`.
fn coming(today: i64, weekday: i64) -> i64 {
    let ahead = (weekday - day_of_week(today)).rem_euclid(7);
    today + if ahead == 0 { 7 } else { ahead }
}

/// 0 = Monday … 6 = Sunday (day 0 is 1970-01-01, a Thursday).
fn day_of_week(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// `YYYY-MM-DD` → days since 1970-01-01.
fn day_number(iso: &str) -> Option<i64> {
    let parts: Vec<i64> = iso.get(..10)?.split('-').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts.as_slice() {
        [y, m, d] => from_parts(*y, *m, *d),
        _ => None,
    }
}

fn iso(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `7`, `7:30`, `11pm`, `7:30am` → (hour, minute, what follows).
fn clock_parts(token: &str) -> Option<(u32, u32, &str)> {
    let split = token.find(|c: char| !c.is_ascii_digit()).unwrap_or(token.len());
    let (hour, rest) = token.split_at(split);
    if hour.is_empty() || hour.len() > 2 {
        return None;
    }
    let hour: u32 = hour.parse().ok()?;
    match rest.strip_prefix(':') {
        Some(rest) => {
            let minute = rest.get(..2).filter(|m| m.chars().all(|c| c.is_ascii_digit()))?;
            Some((hour, minute.parse().ok()?, &rest[2..]))
        }
        None => Some((hour, 0, rest)),
    }
}

/// `am` → false, `pm` → true.
fn meridiem(word: &str) -> Option<bool> {
    match word {
        "am" | "a.m" | "a.m." => Some(false),
        "pm" | "p.m" | "p.m." => Some(true),
        _ => None,
    }
}

fn to_24h(hour: u32, minute: u32, pm: Option<bool>) -> Option<(u32, u32)> {
    if minute > 59 {
        return None;
    }
    match pm {
        Some(pm) => (1..=12).contains(&hour).then(|| (hour % 12 + if pm { 12 } else { 0 }, minute)),
        None => (hour <= 23).then_some((hour, minute)),
    }
}

/// Spans past this many offered for one text argument: the shortest, the note text and the whole command stay.
pub const MAX_SPANS: usize = 48;
/// A run doesn't start with one of these: they join a value to the rest of the command ("add milk *to* groceries").
const LEADING_JOINERS: [&str; 14] =
    ["to", "into", "in", "on", "onto", "for", "from", "by", "with", "at", "of", "and", "or", "as"];
/// Nor end with one of these.
const TRAILING_JOINERS: [&str; 20] = [
    "to", "into", "in", "on", "onto", "for", "from", "by", "with", "at", "of", "and", "or", "as", "the", "a", "an",
    "my", "our", "your",
];

/// Runs of `command`'s words for a judgment to pick a text argument from (the value-extraction pattern: code finds
/// candidates, Jev picks one). A run is the command's own text from its first word to its last, so casing and inner
/// punctuation survive ("Dune, Part Two", a URL). A run doesn't cross a colon after a word ("note: call mom" gives
/// "note" and "call mom") or the end of a leading note phrase ("jot down that …"), nor start or end on a joining word;
/// all but the [`MAX_SPANS`] shortest of a long command are left out (its note text and the whole command always
/// stay). They come in the command's order.
pub fn spans(command: &str) -> Vec<String> {
    let words = words(command);
    let lead = lead_end(command);
    let lower: Vec<String> = words.iter().map(|&(start, end)| command[start..end].to_lowercase()).collect();
    // (start word, end word, text)
    let mut runs: Vec<(usize, usize, String)> = Vec::new();
    for start in 0..words.len() {
        if LEADING_JOINERS.contains(&lower[start].as_str()) {
            continue;
        }
        for end in start..words.len() {
            if end > start && command[words[end - 1].1..words[end].0].contains(':') {
                break; // a colon ends the run: "note:" leads in what follows
            }
            if lead.is_some_and(|lead| words[start].0 < lead && words[end].1 > lead) {
                break; // a note phrase leads in what follows
            }
            if TRAILING_JOINERS.contains(&lower[end].as_str()) {
                continue;
            }
            let text = command[words[start].0..words[end].1].to_owned();
            if !runs.iter().any(|(_, _, seen)| *seen == text) {
                runs.push((start, end, text));
            }
        }
    }
    let note = note_text(command);
    let whole = command.trim();
    if runs.len() > MAX_SPANS {
        let keep = |text: &str| Some(text) == note.as_deref() || text == whole;
        let mut kept: Vec<(usize, usize, String)> = runs.iter().filter(|(_, _, text)| keep(text)).cloned().collect();
        let mut rest: Vec<(usize, usize, String)> = runs.into_iter().filter(|(_, _, text)| !keep(text)).collect();
        rest.sort_by_key(|(start, end, _)| (end - start, *start));
        rest.truncate(MAX_SPANS.saturating_sub(kept.len()));
        kept.extend(rest);
        kept.sort_by_key(|(start, end, _)| (*start, *end));
        runs = kept;
    }
    let mut out: Vec<String> = runs.into_iter().map(|(_, _, text)| text).collect();
    if let Some(note) = note
        && !out.contains(&note)
    {
        out.push(note);
    }
    out
}

/// The command's words as byte ranges, without the punctuation around them.
fn words(command: &str) -> Vec<(usize, usize)> {
    const EDGE: &[char] = &[',', '.', ';', ':', '!', '?', '"', '\'', '(', ')', '[', ']', '{', '}', '“', '”', '‘', '’'];
    let mut out = Vec::new();
    let mut offset = 0;
    for piece in command.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end();
        let trimmed = word.trim_start_matches(EDGE);
        let start = offset + (word.len() - trimmed.len());
        let trimmed = trimmed.trim_end_matches(EDGE);
        if !trimmed.is_empty() {
            out.push((start, start + trimmed.len()));
        }
        offset += piece.len();
    }
    out
}

/// Command phrases that introduce a note, longest first.
const NOTE_LEADS: [&str; 20] = [
    "add a note that",
    "add a note to",
    "make a note that",
    "make a note to",
    "write down that",
    "jot down that",
    "remember that",
    "note to self",
    "remember to",
    "save a note",
    "take a note",
    "add a note",
    "make a note",
    "write down",
    "note that",
    "jot down",
    "new note",
    "add note",
    "note down",
    "note",
];

/// Where a leading note phrase ("remember to", "jot down that") ends in `command`, as a byte offset.
fn lead_end(command: &str) -> Option<usize> {
    let trimmed = command.trim_start();
    // ASCII lowercasing keeps byte offsets.
    let lower = trimmed.to_ascii_lowercase();
    let lead = NOTE_LEADS.iter().find(|lead| lower.starts_with(&format!("{lead} ")))?;
    Some(command.len() - trimmed.len() + lead.len())
}

/// The text of a note: what follows the first colon, else the command without a leading note phrase. `None` when
/// neither is there.
pub fn note_text(command: &str) -> Option<String> {
    let command = command.trim();
    let text = if let Some((_, rest)) = command.split_once(':').filter(|(_, rest)| !rest.trim().is_empty()) {
        rest
    } else {
        // ASCII lowercasing keeps byte offsets, so `lower`'s prefix lengths slice `command` correctly.
        let lower = command.to_ascii_lowercase();
        let lead = NOTE_LEADS.iter().find(|lead| lower.starts_with(&format!("{lead} ")))?;
        &command[lead.len()..]
    };
    let text = text.trim().trim_end_matches(['.', '!', '?']).trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn tokenize(command: &str) -> Vec<String> {
    tokenize_with(command, "$€%.:'-")
}

/// Lowercase words, keeping the characters in `keep` (and digits' thousands separators dropped).
fn tokenize_with(command: &str, keep: &str) -> Vec<String> {
    let lower = command.to_lowercase();
    // Drop thousands separators ("1,200") before splitting on punctuation.
    let chars: Vec<char> = lower.chars().collect();
    let mut cleaned = String::with_capacity(lower.len());
    for (index, &c) in chars.iter().enumerate() {
        let between_digits = index > 0
            && chars[index - 1].is_ascii_digit()
            && chars.get(index + 1).is_some_and(|next| next.is_ascii_digit());
        if c == ',' && between_digits {
            continue;
        }
        cleaned.push(if c.is_alphanumeric() || keep.contains(c) { c } else { ' ' });
    }
    cleaned
        .split_whitespace()
        .map(|token| token.trim_matches(['.', ':', '\'', '-']).to_owned())
        .filter(|token| !token.is_empty())
        .collect()
}

/// A number starting at `tokens[index]`: its value, how many tokens it spans, and a unit when one is attached
/// ("500ml", "$40").
fn number_at(tokens: &[String], index: usize) -> Option<(f64, usize, Unit)> {
    let token = tokens[index].as_str();
    let next = tokens.get(index + 1).map(String::as_str);
    if token.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '$' || c == '€') {
        return digits(token).map(|(value, unit)| (value, 1, unit));
    }
    match token {
        // "an hour", "a litre"; "a hundred" is handled as a number word.
        "a" | "an" if next.is_some_and(|word| unit_word(word).is_some()) => Some((1.0, 1, Unit::Bare)),
        // "half a litre", "half an hour", "half hour".
        "half" => match next {
            Some("a" | "an") if tokens.get(index + 2).is_some_and(|word| unit_word(word).is_some()) => {
                Some((0.5, 2, Unit::Bare))
            }
            Some(word) if unit_word(word).is_some() => Some((0.5, 1, Unit::Bare)),
            _ => None,
        },
        _ => number_words(tokens, index),
    }
}

/// `40`, `12.50`, `$40`, `500ml`, `25m`, `7am`, `3:30`, `15%`.
fn digits(token: &str) -> Option<(f64, Unit)> {
    let (money, rest) = match token.strip_prefix(['$', '€']) {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    let split = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
    let (number, suffix) = rest.split_at(split);
    let value: f64 = number.trim_end_matches('.').parse().ok()?;
    let unit = match suffix {
        "" if money => Unit::Money,
        "" => Unit::Bare,
        "m" => Unit::Minutes,
        "k" => Unit::Other,
        _ if suffix.starts_with(':') => Unit::Clock,
        _ => unit_word(suffix).unwrap_or(Unit::Other),
    };
    Some((value, unit))
}

fn unit_word(word: &str) -> Option<Unit> {
    Some(match word {
        "sec" | "secs" | "second" | "seconds" => Unit::Seconds,
        "ml" | "milliliter" | "milliliters" | "millilitre" | "millilitres" => Unit::Ml,
        "l" | "liter" | "liters" | "litre" | "litres" => Unit::Liters,
        "min" | "mins" | "minute" | "minutes" => Unit::Minutes,
        "h" | "hr" | "hrs" | "hour" | "hours" => Unit::Hours,
        "dollar" | "dollars" | "bucks" | "usd" | "euro" | "euros" | "eur" => Unit::Money,
        "am" | "pm" => Unit::Clock,
        "%" | "percent" => Unit::Percent,
        _ => return None,
    })
}

/// A run of number words ("twenty-five", "two hundred and fifty", "a dozen").
fn number_words(tokens: &[String], start: usize) -> Option<(f64, usize, Unit)> {
    let mut total = 0u64;
    let mut current = 0u64;
    let mut used = 0;
    let mut seen_number = false;
    let mut index = start;
    while let Some(token) = tokens.get(index) {
        let parts: Vec<&str> = token.split('-').collect();
        let values: Option<Vec<Word>> = parts.iter().map(|part| word(part)).collect();
        match values {
            Some(words) => {
                for word in words {
                    match word {
                        Word::Plain(n) => current += n,
                        Word::Hundred => current = current.max(1) * 100,
                        Word::Thousand => {
                            total += current.max(1) * 1000;
                            current = 0;
                        }
                        Word::Dozen => current = current.max(1) * 12,
                    }
                }
                seen_number = true;
            }
            None => {
                let is_joiner = matches!(token.as_str(), "and" | "a" | "an");
                let continues = tokens.get(index + 1).is_some_and(|next| word(next).is_some());
                // "and" only joins number words; "a"/"an" may only start "a hundred", "a dozen".
                let allowed = match token.as_str() {
                    "and" => seen_number,
                    _ => !seen_number,
                };
                if !(is_joiner && continues && allowed) {
                    break;
                }
            }
        }
        index += 1;
        used = index - start;
    }
    seen_number.then(|| ((total + current) as f64, used, Unit::Bare))
}

enum Word {
    Plain(u64),
    Hundred,
    Thousand,
    Dozen,
}

fn word(text: &str) -> Option<Word> {
    const SMALL: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 8] = ["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    if let Some(n) = SMALL.iter().position(|small| *small == text) {
        return Some(Word::Plain(n as u64));
    }
    if let Some(n) = TENS.iter().position(|tens| *tens == text) {
        return Some(Word::Plain(20 + 10 * n as u64));
    }
    match text {
        "hundred" => Some(Word::Hundred),
        "thousand" => Some(Word::Thousand),
        "dozen" => Some(Word::Dozen),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(command: &str) -> Vec<(String, f64, Unit)> {
        numbers(command).into_iter().map(|c| (c.surface, c.value, c.unit)).collect()
    }

    #[test]
    fn digits_with_units() {
        assert_eq!(found("log 500ml water"), [("500ml".into(), 500.0, Unit::Ml)]);
        assert_eq!(found("had 1.5 liters of water today"), [("1.5 liters".into(), 1.5, Unit::Liters)]);
        assert_eq!(found("add expense $40 gas"), [("$40".into(), 40.0, Unit::Money)]);
        assert_eq!(found("spent 12.50 on lunch."), [("12.50".into(), 12.5, Unit::Bare)]);
        assert_eq!(found("set an alarm for 7am"), [("7am".into(), 7.0, Unit::Clock)]);
        assert_eq!(found("log 20 pushups at 7"), [("20".into(), 20.0, Unit::Bare), ("7".into(), 7.0, Unit::Clock)]);
        assert_eq!(found("what's 15% of 80"), [("15%".into(), 15.0, Unit::Percent), ("80".into(), 80.0, Unit::Bare)]);
        assert_eq!(found("paid 1,200 dollars"), [("1200 dollars".into(), 1200.0, Unit::Money)]);
        assert_eq!(found("plank for 45 seconds"), [("45 seconds".into(), 45.0, Unit::Seconds)]);
    }

    #[test]
    fn number_words_and_articles() {
        assert_eq!(found("did twenty pushups"), [("twenty".into(), 20.0, Unit::Bare)]);
        assert_eq!(found("log twenty-five pushps"), [("twenty-five".into(), 25.0, Unit::Bare)]);
        assert_eq!(
            found("log two hundred and fifty ml of water"),
            [("two hundred and fifty ml".into(), 250.0, Unit::Ml)]
        );
        assert_eq!(found("start a pomodoro for an hour"), [("an hour".into(), 1.0, Unit::Hours)]);
        assert_eq!(found("half a litre of water"), [("half a litre".into(), 0.5, Unit::Liters)]);
        assert_eq!(found("a dozen pushups"), [("a dozen".into(), 12.0, Unit::Bare)]);
        assert!(found("record a set of pushups").is_empty());
        assert!(found("add a note and a reminder").is_empty());
    }

    #[test]
    fn articles_may_count_one() {
        let surfaces = |command: &str| articles(command).into_iter().map(|c| c.surface).collect::<Vec<_>>();
        assert_eq!(surfaces("had a glass of water"), ["a glass"]);
        assert_eq!(surfaces("log a set of pushups and an apple"), ["a set", "an apple"]);
        assert!(surfaces("an hour of reading, a dozen eggs, a few things, a 5k").is_empty());
    }

    #[test]
    fn units_convert_to_what_an_argument_measures() {
        let hour = &numbers("an hour")[0];
        assert_eq!(value(hour, Measure::Minutes, true), Some(60.0));
        assert_eq!(value(hour, Measure::Seconds, true), Some(3600.0));
        assert_eq!(value(hour, Measure::Count, true), None);
        let litre = &numbers("half a litre")[0];
        assert_eq!(value(litre, Measure::Ml, true), Some(500.0));
        let bare = &numbers("2.5")[0];
        assert_eq!(value(bare, Measure::Count, true), None, "a whole number");
        assert_eq!(value(bare, Measure::Money, false), Some(2.5));
        let dollars = &numbers("$12")[0];
        assert_eq!(value(dollars, Measure::Count, true), None);
        assert_eq!(json_number(20.0, true), serde_json::json!(20));
    }

    #[test]
    fn times_of_day() {
        let hhmm = |command: &str| times(command).into_iter().map(|t| (t.surface, t.hhmm)).collect::<Vec<_>>();
        assert_eq!(
            hhmm("slept 11pm to 7:30 am"),
            [("11pm".into(), "23:00".into()), ("7:30 am".into(), "07:30".into())]
        );
        assert_eq!(
            hhmm("bed at 23:15, up at 6:45"),
            [("23:15".into(), "23:15".into()), ("6:45".into(), "06:45".into())]
        );
        assert_eq!(hhmm("lunch at noon"), [("noon".into(), "12:00".into())]);
        assert_eq!(hhmm("12am"), [("12am".into(), "00:00".into())]);
        assert!(hhmm("log 20 pushups at 7").is_empty(), "morning or evening?");
        assert!(hhmm("13pm 25:00").is_empty());
    }

    #[test]
    fn dates_from_today() {
        // 2026-10-06 is a Tuesday.
        let iso =
            |command: &str| dates(command, "2026-10-06").into_iter().map(|d| (d.surface, d.iso)).collect::<Vec<_>>();
        assert_eq!(iso("slept badly yesterday"), [("yesterday".into(), "2026-10-05".into())]);
        assert_eq!(iso("3 days ago"), [("3 days ago".into(), "2026-10-03".into())]);
        assert_eq!(iso("in two weeks"), [("in two weeks".into(), "2026-10-20".into())]);
        assert_eq!(iso("on monday"), [("monday".into(), "2026-10-05".into())]);
        assert_eq!(iso("tuesday"), [("tuesday".into(), "2026-10-06".into())], "today counts");
        assert_eq!(iso("next tuesday"), [("next tuesday".into(), "2026-10-13".into())]);
        assert_eq!(iso("until dec 25"), [("dec 25".into(), "2026-12-25".into())]);
        assert_eq!(iso("on the 25th of december"), [("25th of december".into(), "2026-12-25".into())]);
        assert_eq!(iso("jan 15th"), [("jan 15th".into(), "2027-01-15".into())], "the nearest one");
        assert_eq!(iso("oct 1"), [("oct 1".into(), "2026-10-01".into())]);
        assert_eq!(iso("2026-02-29 or 2028-02-29"), [("2028-02-29".into(), "2028-02-29".into())]);
        assert_eq!(iso("12/25"), [("12/25".into(), "2026-12-25".into())]);
        assert!(iso("log 20 pushups at 7").is_empty());
        assert!(iso("a set of 20").is_empty());
        assert_eq!(iso("x"), []);
    }

    #[test]
    fn spans_keep_the_words_as_typed() {
        let runs = spans("add Milk to groceries");
        assert!(runs.contains(&"Milk".to_owned()));
        assert!(runs.contains(&"add Milk".to_owned()));
        assert!(runs.contains(&"add Milk to groceries".to_owned()));
        assert!(!runs.iter().any(|r| r.starts_with("to ") || r.ends_with(" to")), "{runs:?}");
        let runs = spans("jot down that the key is under the mat");
        assert!(runs.contains(&"the key is under the mat".to_owned()), "{runs:?}");
        assert!(!runs.iter().any(|r| r.starts_with("jot down that the")), "no run crosses the lead: {runs:?}");
        let runs = spans("save https://example.com/a?b=1 as Reading list");
        assert!(runs.contains(&"https://example.com/a?b=1".to_owned()), "{runs:?}");
        assert!(runs.contains(&"Reading list".to_owned()));
        let runs = spans("note: call Mom, then the bank.");
        assert!(runs.contains(&"call Mom, then the bank".to_owned()), "{runs:?}");
        assert_eq!(runs.first().map(String::as_str), Some("note"));
        assert!(!runs.iter().any(|r| r.starts_with("note:")), "no run crosses the colon: {runs:?}");
    }

    #[test]
    fn long_commands_keep_short_runs_and_the_note() {
        let command = "note: remember that the spare key for the shed is under the blue flower pot by the back door \
                       next to the hose";
        let runs = spans(command);
        assert!(runs.len() <= MAX_SPANS + 1, "{}", runs.len());
        assert!(runs.contains(&note_text(command).unwrap()));
        assert!(runs.contains(&"spare key".to_owned()));
    }

    #[test]
    fn note_text_after_colon_or_lead_phrase() {
        assert_eq!(note_text("note: call mom on sunday").as_deref(), Some("call mom on sunday"));
        assert_eq!(note_text("add a note to buy more coffee filters").as_deref(), Some("buy more coffee filters"));
        assert_eq!(
            note_text("Remember to email Sam about the invoice.").as_deref(),
            Some("email Sam about the invoice")
        );
        assert_eq!(
            note_text("jot down that the wifi password is on the fridge").as_deref(),
            Some("the wifi password is on the fridge")
        );
        assert_eq!(note_text("log 20 pushups"), None);
    }
}
