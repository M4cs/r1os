//! Alarms and reminders (ADR-0028). You set them by talking to an agent (its tools, crate::mcp), and they
//! live in core.db. The scheduler wakes for them with its kernel alarm (crate::wake, crate::scheduler), so
//! they go off with the R1 asleep.
//!
//! - **An alarm** rings at a local time of day, once or on chosen weekdays. The shell's alarm screen takes
//!   over, with a looping tone, until a click stops it. A double click snoozes it ten minutes; after ten
//!   minutes of ringing the shell gives up (stops it). A one-off alarm turns itself off once stopped.
//! - **A reminder** goes off at a local date and time: the agent's note at the top of the screen. It stays
//!   pending until you've seen it (the shell shows it again when the R1 wakes), then it's gone. One that
//!   repeats (T-069: a time of day on chosen weekdays, like an alarm) moves on to its next time instead.
//! - **A to-do** (T-069) is a reminder with no time: it's on the list until you tick it off (`done`), and a
//!   ticked one goes a day later (or when you clear them).
//!
//! Times are local to the R1's time zone (Android's, `persist.sys.timezone`). It follows the R1's network
//! (ADR-0030: looked up when a network comes up), unless the user picked one (`set_time_zone`). An alarm more
//! than half an hour late (the R1 was off) doesn't ring; it moves on to its next day. A late reminder still
//! goes off.

use crate::hub::{Event, Hub, Source};
use std::sync::Arc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

const MIN_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;
/// Snoozing puts it off this long.
pub const SNOOZE_MS: i64 = 10 * MIN_MS;
/// An alarm this late doesn't ring.
const LATE_MS: i64 = 30 * MIN_MS;
/// Alarms plus reminders, at most (an agent in a loop can't fill the database).
const MAX: i64 = 100;
const DAY_NAMES: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// The local UTC offset at an instant (ms).
pub type Offset<'a> = &'a dyn Fn(i64) -> i64;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Alarm {
    pub id: String,
    /// `alarm` or `reminder`.
    pub kind: String,
    /// The alarm's label, or what to be reminded of.
    pub label: String,
    /// An alarm's local time of day, minutes after midnight.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minute: Option<i64>,
    /// The weekdays an alarm repeats on, Monday = bit 0; 0 rings once.
    pub days: i64,
    /// When it next goes off (ms); None when it's off, or a reminder that has gone off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    pub on: bool,
    /// The agent that set it (the shell shows its face).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// An alarm ringing now: since when (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ringing: Option<i64>,
    /// A reminder that went off (ms), not yet seen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fired: Option<i64>,
    /// A to-do ticked off (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<i64>,
}

impl Alarm {
    /// A reminder with no time: one on the to-do list (T-069).
    pub fn is_todo(&self) -> bool {
        self.kind == "reminder" && self.minute.is_none() && self.at.is_none() && self.fired.is_none()
    }
}

/// When a reminder goes off (T-069): never (a to-do), once, or at a time of day on some weekdays.
pub enum When {
    Todo,
    At(i64),
    /// `next`: its first time.
    Repeats { minute: i64, days: i64, next: i64 },
}

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS alarms (
           id TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT NOT NULL DEFAULT '', minute INTEGER,
           days INTEGER NOT NULL DEFAULT 0, at INTEGER, on_ INTEGER NOT NULL DEFAULT 1, agent TEXT,
           ringing INTEGER, fired INTEGER, created INTEGER NOT NULL, done INTEGER);",
    )?;
    // To-dos ticked off (T-069), in a table from before them.
    if db.prepare("SELECT done FROM alarms LIMIT 0").is_err() {
        db.execute_batch("ALTER TABLE alarms ADD COLUMN done INTEGER;")?;
    }
    Ok(())
}

const COLUMNS: &str = "id, kind, label, minute, days, at, on_, agent, ringing, fired, done";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Alarm> {
    Ok(Alarm {
        id: r.get(0)?,
        kind: r.get(1)?,
        label: r.get(2)?,
        minute: r.get(3)?,
        days: r.get(4)?,
        at: r.get(5)?,
        on: r.get::<_, i64>(6)? != 0,
        agent: r.get(7)?,
        ringing: r.get(8)?,
        fired: r.get(9)?,
        done: r.get(10)?,
    })
}

/// Alarms (by time of day) then reminders (soonest first).
pub fn list(db: &Connection) -> rusqlite::Result<Vec<Alarm>> {
    let mut st = db.prepare(&format!(
        "SELECT {COLUMNS} FROM alarms ORDER BY kind = 'reminder', coalesce(minute, 0), coalesce(at, fired), created"
    ))?;
    st.query_map([], row)?.collect()
}

pub fn get(db: &Connection, id: &str) -> rusqlite::Result<Option<Alarm>> {
    db.query_row(&format!("SELECT {COLUMNS} FROM alarms WHERE id = ?1"), [id], row).optional()
}

fn room(db: &Connection) -> anyhow::Result<()> {
    let n: i64 = db.query_row("SELECT count(*) FROM alarms", [], |r| r.get(0))?;
    anyhow::ensure!(n < MAX, "There are already {MAX} alarms and reminders: remove some first.");
    Ok(())
}

/// A new alarm, or changes to one (`id`). It's on, and due at its next time after `now`.
#[allow(clippy::too_many_arguments)]
pub fn set_alarm(
    db: &Connection,
    id: Option<&str>,
    minute: i64,
    days: i64,
    label: &str,
    agent: Option<&str>,
    now: i64,
    offset: Offset,
) -> anyhow::Result<Alarm> {
    anyhow::ensure!((0..DAY_MS / MIN_MS).contains(&minute), "That isn't a time of day.");
    let at = next_ring(minute, days, now, offset);
    let label = clean(label);
    let id = match id {
        Some(id) => {
            let n = db.execute(
                "UPDATE alarms SET minute = ?2, days = ?3, label = ?4, at = ?5, on_ = 1, ringing = NULL WHERE id = ?1 AND kind = 'alarm'",
                params![id, minute, days & 0x7f, label, at],
            )?;
            anyhow::ensure!(n == 1, "There's no alarm `{id}`. Use list_alarms.");
            id.to_owned()
        }
        None => {
            room(db)?;
            let id = crate::core::random_hex(4);
            db.execute(
                "INSERT INTO alarms (id, kind, label, minute, days, at, agent, created) VALUES (?1, 'alarm', ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, label, minute, days & 0x7f, at, agent, now],
            )?;
            id
        }
    };
    Ok(get(db, &id)?.expect("just written"))
}

/// A reminder: once at a time, repeating, or a to-do (no time).
pub fn set_reminder(db: &Connection, when: When, label: &str, agent: Option<&str>, now: i64) -> anyhow::Result<Alarm> {
    let label = clean(label);
    anyhow::ensure!(!label.is_empty(), "Say what to remind them of.");
    let (at, minute, days) = match when {
        When::Todo => (None, None, 0),
        When::At(at) => {
            anyhow::ensure!(at > now - MIN_MS, "That time has passed.");
            (Some(at), None, 0)
        }
        When::Repeats { minute, days, next } => {
            anyhow::ensure!((0..DAY_MS / MIN_MS).contains(&minute), "That isn't a time of day.");
            (Some(next), Some(minute), days & 0x7f)
        }
    };
    room(db)?;
    let id = crate::core::random_hex(4);
    db.execute(
        "INSERT INTO alarms (id, kind, label, minute, days, at, agent, created) VALUES (?1, 'reminder', ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, label, minute, days, at, agent, now],
    )?;
    Ok(get(db, &id)?.expect("just written"))
}

/// A to-do ticked off, or back on the list.
pub fn tick(db: &Connection, id: &str, done: bool, now: i64) -> anyhow::Result<Alarm> {
    let a = get(db, id)?.filter(Alarm::is_todo).ok_or_else(|| anyhow::anyhow!("There's no to-do `{id}` (a reminder with a time is cancelled instead)."))?;
    db.execute("UPDATE alarms SET done = ?2 WHERE id = ?1", params![a.id, done.then_some(now)])?;
    Ok(get(db, id)?.expect("just written"))
}

/// Takes ticked to-dos off the list: all of them, or those ticked before `before`. How many.
pub fn clear_done(db: &Connection, before: Option<i64>) -> rusqlite::Result<usize> {
    db.execute(
        "DELETE FROM alarms WHERE kind = 'reminder' AND done IS NOT NULL AND at IS NULL AND minute IS NULL AND done < ?1",
        [before.unwrap_or(i64::MAX)],
    )
}

/// Removes one. Whether it was there.
pub fn remove(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM alarms WHERE id = ?1", [id])? == 1)
}

/// An alarm on (due at its next time) or off.
pub fn turn(db: &Connection, id: &str, on: bool, now: i64, offset: Offset) -> anyhow::Result<Alarm> {
    let a = get(db, id)?.filter(|a| a.kind == "alarm").ok_or_else(|| anyhow::anyhow!("There's no alarm `{id}`."))?;
    let at = if on { Some(next_ring(a.minute.unwrap_or(0), a.days, now, offset)) } else { None };
    db.execute("UPDATE alarms SET on_ = ?2, at = ?3, ringing = NULL WHERE id = ?1", params![id, on as i64, at])?;
    Ok(get(db, id)?.expect("just written"))
}

/// The ringing alarm stopped. A one-off alarm turns off (unless it was snoozed meanwhile).
pub fn stop(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    let n = db.execute(
        "UPDATE alarms SET ringing = NULL, on_ = CASE WHEN days = 0 AND at IS NULL THEN 0 ELSE on_ END WHERE id = ?1 AND kind = 'alarm'",
        [id],
    )?;
    Ok(n == 1)
}

/// The ringing alarm goes off again in [`SNOOZE_MS`]. For a repeating alarm the snooze comes first; its
/// next day is worked out again when the snooze rings.
pub fn snooze(db: &Connection, id: &str, now: i64) -> rusqlite::Result<bool> {
    let n = db.execute("UPDATE alarms SET ringing = NULL, on_ = 1, at = ?2 WHERE id = ?1 AND kind = 'alarm'", params![id, now + SNOOZE_MS])?;
    Ok(n == 1)
}

/// A reminder that went off was seen: it's done, or (one that repeats) waits for its next time.
pub fn seen(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    if db.execute("DELETE FROM alarms WHERE id = ?1 AND kind = 'reminder' AND fired IS NOT NULL AND days = 0", [id])? == 1 {
        return Ok(true);
    }
    Ok(db.execute("UPDATE alarms SET fired = NULL WHERE id = ?1 AND kind = 'reminder' AND fired IS NOT NULL", [id])? == 1)
}

/// What went off at `now`: alarms that ring and reminders that fire (already recorded as such), and when
/// the next one is due.
pub fn due(db: &Connection, now: i64, offset: Offset) -> rusqlite::Result<(Vec<Alarm>, Option<i64>)> {
    let mut st = db.prepare(&format!("SELECT {COLUMNS} FROM alarms WHERE on_ = 1 AND at IS NOT NULL AND at <= ?1"))?;
    let due: Vec<Alarm> = st.query_map([now], row)?.collect::<rusqlite::Result<_>>()?;
    let mut went = Vec::new();
    for a in due {
        let at = a.at.unwrap_or(now);
        if a.kind == "reminder" {
            // One that repeats moves on to its next time (late or not); a one-off one is done once seen.
            let next = (a.days != 0).then(|| next_ring(a.minute.unwrap_or(0), a.days, now, offset));
            db.execute("UPDATE alarms SET at = ?2, fired = ?3 WHERE id = ?1", params![a.id, next, now])?;
        } else {
            // A repeating alarm moves on to its next day now; a one-off one has no next time.
            let next = (a.days != 0).then(|| next_ring(a.minute.unwrap_or(0), a.days, now, offset));
            if now - at > LATE_MS {
                tracing::info!(alarm = %a.id, late_min = (now - at) / MIN_MS, "alarm: too late to ring");
                db.execute("UPDATE alarms SET at = ?2, on_ = ?3 WHERE id = ?1", params![a.id, next, next.is_some() as i64])?;
                continue;
            }
            db.execute("UPDATE alarms SET at = ?2, ringing = ?3 WHERE id = ?1", params![a.id, next, now])?;
        }
        if let Some(a) = get(db, &a.id)? {
            went.push(a);
        }
    }
    let next: Option<i64> = db.query_row("SELECT min(at) FROM alarms WHERE on_ = 1 AND at IS NOT NULL", [], |r| r.get(0))?;
    Ok((went, next))
}

/// Tells the shell: the display wakes (on the R1), and an alarm rings or a reminder shows.
pub async fn went_off(hub: &Hub, a: Alarm) {
    tracing::info!(id = %a.id, kind = %a.kind, "alarm went off");
    crate::scheduler::wake_display().await;
    if a.kind == "reminder" {
        hub.emit(Event::Reminder { id: a.id, text: a.label, agent: a.agent, at: a.fired.unwrap_or_default() }, Source::Api);
    } else {
        hub.emit(Event::Alarm { id: a.id, label: a.label, agent: a.agent, ringing: true }, Source::Api);
    }
    hub.emit(Event::AlarmsChanged, Source::Api);
}

/// The next time an alarm at local `minute` rings after `after` (ms): on one of `days` (Monday = bit 0), or
/// any day if none.
pub fn next_ring(minute: i64, days: i64, after: i64, offset: Offset) -> i64 {
    let local_after = after + offset(after);
    let day0 = local_after.div_euclid(DAY_MS);
    for d in 0..=7 {
        let day = day0 + d;
        if days & 0x7f != 0 && days & (1 << weekday(day)) == 0 {
            continue;
        }
        let at = to_utc(day * DAY_MS + minute * MIN_MS, offset);
        if at > after {
            return at;
        }
    }
    // Unreachable (a week always has the day), but never panic over a clock.
    after + DAY_MS
}

/// Monday = 0 for days since 1970-01-01 (a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// The UTC instant whose local time is `local` (the offset of a nearby instant, then of the result).
fn to_utc(local: i64, offset: Offset) -> i64 {
    let guess = local - offset(local);
    local - offset(guess)
}

// ---- setting them --------------------------------------------------------------------------------

/// The R1's clock and its local offset (DST-correct per instant).
pub fn clock() -> enee_spec::sql::Clock {
    enee_spec::sql::Clock::system()
}

/// An alarm from what an agent (or the API) passes: `time`, `days`, `label`; `id` changes that one.
pub fn alarm_from(db: &Connection, args: &Value, agent: Option<&str>) -> anyhow::Result<Alarm> {
    let time = args["time"].as_str().unwrap_or_default();
    let minute = parse_time(time).ok_or_else(|| anyhow::anyhow!("time: `{time}` isn't a time of day like 7:30 or 19:05."))?;
    let days = parse_days(&args["days"]).map_err(anyhow::Error::msg)?;
    let id = args["id"].as_str().filter(|s| !s.is_empty());
    let c = clock();
    set_alarm(db, id, minute, days, args["label"].as_str().unwrap_or_default(), agent, c.now_ms(), &|ms| c.offset_ms(ms))
}

/// A reminder from what an agent (or the API) passes: `text`, and `at` (local) or `in_minutes` (once), `time`
/// with `days` (repeating; `time` alone is its next one), or none of them (a to-do).
pub fn reminder_from(db: &Connection, args: &Value, agent: Option<&str>) -> anyhow::Result<Alarm> {
    let c = clock();
    let now = c.now_ms();
    let offset = |ms| c.offset_ms(ms);
    let when = when_from(args, now, &offset)?;
    set_reminder(db, when, args["text"].as_str().unwrap_or_default(), agent, now)
}

fn when_from(args: &Value, now: i64, offset: Offset) -> anyhow::Result<When> {
    let blank = |k: &str| args[k].is_null() || args[k].as_str().is_some_and(|s| s.trim().is_empty());
    Ok(match (args["in_minutes"].as_f64(), args["at"].as_str().filter(|_| !blank("at")), args["time"].as_str().filter(|_| !blank("time"))) {
        (Some(m), ..) if m > 0.0 && m <= 60.0 * 24.0 * 366.0 => When::At(now + (m * MIN_MS as f64) as i64),
        (Some(_), ..) => anyhow::bail!("in_minutes: more than 0, at most a year."),
        (None, Some(at), _) => When::At(parse_when(at, now, offset).ok_or_else(|| anyhow::anyhow!("at: `{at}` isn't a local time like 2026-10-10 17:00 or 17:00."))?),
        (None, None, Some(time)) => {
            let minute = parse_time(time).ok_or_else(|| anyhow::anyhow!("time: `{time}` isn't a time of day like 9:00."))?;
            let days = parse_days(&args["days"]).map_err(anyhow::Error::msg)?;
            let next = next_ring(minute, days, now, offset);
            if days == 0 { When::At(next) } else { When::Repeats { minute, days, next } }
        }
        (None, None, None) => When::Todo,
    })
}

/// The alarm ringing now, if one is.
pub fn ringing(db: &Connection) -> rusqlite::Result<Option<Alarm>> {
    db.query_row(&format!("SELECT {COLUMNS} FROM alarms WHERE ringing IS NOT NULL ORDER BY ringing DESC LIMIT 1"), [], row).optional()
}

/// Stops a ringing alarm, or snoozes it, and tells the shell (its alarm screen goes). Whether it was there.
pub fn end_ring(hub: &Hub, id: &str, snooze_it: bool) -> anyhow::Result<bool> {
    let now = clock().now_ms();
    let (done, a) = hub.core.with(|db| -> rusqlite::Result<_> {
        let done = if snooze_it { snooze(db, id, now)? } else { stop(db, id)? };
        Ok((done, get(db, id)?))
    })?;
    if let Some(a) = a.filter(|_| done) {
        hub.emit(Event::Alarm { id: a.id, label: a.label, agent: a.agent, ringing: false }, Source::Api);
    }
    changed(hub);
    Ok(done)
}

/// Alarms changed: the scheduler looks again at what's due, and the shell reloads them.
pub fn changed(hub: &Hub) {
    hub.scheduler.wake();
    hub.emit(Event::AlarmsChanged, Source::Api);
}

// ---- what agents say ------------------------------------------------------------------------------

/// "7", "7:30", "07:30", "7:30 am", "7pm", "19:05" → minutes after midnight.
pub fn parse_time(s: &str) -> Option<i64> {
    let t = s.trim().to_ascii_lowercase().replace([' ', '.'], "");
    let (t, half) = if let Some(t) = t.strip_suffix("am") {
        (t, Some(false))
    } else if let Some(t) = t.strip_suffix("pm") {
        (t, Some(true))
    } else {
        (t.as_str(), None)
    };
    let (h, m) = t.split_once(':').unwrap_or((t, "0"));
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    if !(0..60).contains(&m) {
        return None;
    }
    let h = match half {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if (0..24).contains(&h) => h,
        None => return None,
    };
    Some(h * 60 + m)
}

/// Weekdays from what an agent passes: names ("mon", "Tuesday"), "weekdays", "weekends", "daily"; as a list
/// or one string. Empty → once.
pub fn parse_days(v: &Value) -> Result<i64, String> {
    let words: Vec<String> = match v {
        Value::Null => return Ok(0),
        Value::String(s) => s.split([',', ' ', '/']).map(str::to_owned).collect(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => return Err("days: a list of weekday names".into()),
    };
    let mut days = 0;
    for w in words.iter().map(|w| w.trim().to_ascii_lowercase()).filter(|w| !w.is_empty() && w != "and") {
        days |= match w.as_str() {
            "weekdays" | "weekday" => 0x1f,
            "weekends" | "weekend" => 0x60,
            "daily" | "everyday" | "every" | "day" | "days" | "all" => 0x7f,
            w => match DAY_NAMES.iter().position(|d| w.len() >= 2 && d.starts_with(w.trim_end_matches('s'))) {
                Some(i) => 1 << i,
                None => return Err(format!("days: `{w}` isn't a weekday")),
            },
        };
    }
    Ok(days)
}

/// "YYYY-MM-DD HH:MM" (or "…THH:MM"), or "HH:MM" (the next one), local → ms.
pub fn parse_when(s: &str, now: i64, offset: Offset) -> Option<i64> {
    let s = s.trim();
    if let Some((date, time)) = s.split_once([' ', 'T']).filter(|(d, _)| d.len() == 10 && d.as_bytes()[4] == b'-') {
        let mut it = date.split('-').map(str::parse::<i64>);
        let (y, m, d) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        let minute = parse_time(time.trim_end_matches('Z'))?;
        return Some(to_utc(days_from_civil(y, m, d) * DAY_MS + minute * MIN_MS, offset));
    }
    Some(next_ring(parse_time(s)?, 0, now, offset))
}

/// "Saturday 2026-10-10 07:30", in the R1's local time.
pub fn describe_time(ms: i64, offset: Offset) -> String {
    let local = ms + offset(ms);
    let day = local.div_euclid(DAY_MS);
    let (y, m, d) = civil_from_days(day);
    let minute = local.rem_euclid(DAY_MS) / MIN_MS;
    let name = DAY_NAMES[usize::try_from(weekday(day)).unwrap_or(0)];
    let name = format!("{}{}", name[..1].to_ascii_uppercase(), &name[1..]);
    format!("{name} {y:04}-{m:02}-{d:02} {:02}:{:02}", minute / 60, minute % 60)
}

/// "every day", "weekdays", "Mon, Wed", "once".
pub fn describe_days(days: i64) -> String {
    match days & 0x7f {
        0 => "once".into(),
        0x7f => "every day".into(),
        0x1f => "weekdays".into(),
        0x60 => "weekends".into(),
        d => (0..7).filter(|i| d & (1 << i) != 0).map(|i| format!("{}{}", DAY_NAMES[i][..1].to_ascii_uppercase(), &DAY_NAMES[i][1..3])).collect::<Vec<_>>().join(", "),
    }
}

/// For an agent: what's set, with ids.
pub fn summary(list: &[Alarm], offset: Offset) -> Value {
    let alarms: Vec<Value> = list
        .iter()
        .map(|a| {
            let mut v = json!({ "id": a.id, "kind": a.kind, "label": a.label, "on": a.on });
            if let Some(m) = a.minute {
                v["time"] = json!(format!("{:02}:{:02}", m / 60, m % 60));
                v["repeats"] = json!(describe_days(a.days));
            }
            if let Some(at) = a.at {
                v["next"] = json!(describe_time(at, offset));
            }
            if a.ringing.is_some() {
                v["ringing"] = json!(true);
            }
            if let Some(f) = a.fired {
                v["went_off"] = json!(describe_time(f, offset));
            }
            if a.is_todo() {
                v["todo"] = json!(true);
                v["done"] = json!(a.done.is_some());
            }
            v
        })
        .collect();
    json!(alarms)
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect()
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

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

/// The R1's time zone (an Olson id, Android's `persist.sys.timezone`); "UTC" elsewhere.
pub fn zone() -> String {
    #[cfg(target_os = "android")]
    {
        let mut buf = [0u8; 92];
        // SAFETY: the name is a C string and the buffer is PROP_VALUE_MAX (92) bytes.
        let n = unsafe { libc::__system_property_get(c"persist.sys.timezone".as_ptr(), buf.as_mut_ptr().cast()) };
        if let Ok(n @ 1..) = usize::try_from(n) {
            return String::from_utf8_lossy(&buf[..n]).into_owned();
        }
    }
    std::env::var("TZ").unwrap_or_else(|_| "UTC".into())
}

/// The user picked the R1's time zone (an Olson id like `America/New_York`): it stops following the network.
/// Alarms keep their local times.
pub async fn set_zone(hub: &Arc<Hub>, zone: &str) -> anyhow::Result<()> {
    apply_zone(hub, zone).await?;
    hub.core.set_zone_auto(false)?;
    Ok(())
}

/// The time zone follows the R1's network again (`on`; it's looked up now), or stays as it is.
pub async fn set_zone_auto(hub: &Arc<Hub>, on: bool) -> anyhow::Result<()> {
    hub.core.set_zone_auto(on)?;
    if on {
        auto_zone(hub, true).await?;
    }
    Ok(())
}

/// When the network last told us the zone: it's asked at most this often, unless a lookup is forced.
static LAST_LOOKUP: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
const LOOKUP_EVERY: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Sets the time zone from where the R1's network is (ADR-0030), when it's automatic. Called once after r1d
/// starts and when a network comes up (the shell says); `force` skips the 10-minute spacing. The zone it set,
/// if it changed.
pub async fn auto_zone(hub: &Arc<Hub>, force: bool) -> anyhow::Result<Option<String>> {
    if !hub.core.zone_auto() || !cfg!(target_os = "android") {
        return Ok(None);
    }
    {
        let mut last = LAST_LOOKUP.lock().unwrap();
        if !force && last.is_some_and(|t| t.elapsed() < LOOKUP_EVERY) {
            return Ok(None);
        }
        *last = Some(std::time::Instant::now());
    }
    let fetcher = hub.services.fetcher();
    let Some(found) = tokio::task::spawn_blocking(move || lookup_zone(&fetcher)).await? else {
        tracing::info!("time zone: the network didn't say");
        return Ok(None);
    };
    if found == zone() {
        return Ok(None);
    }
    apply_zone(hub, &found).await?;
    tracing::info!(zone = %found, "time zone set from the network");
    Ok(Some(found))
}

/// The zone of the R1's public address: ipinfo.io's answer, else wttr.in's (which r1os already asks for the
/// weather). Each sees only the R1's address, as any server it talks to does.
fn lookup_zone(fetcher: &crate::fetch::Fetcher) -> Option<String> {
    if let Ok(f) = fetcher.json("https://ipinfo.io/json", &|h: &str| h == "ipinfo.io")
        && f.status == 200
        && let Some(z) = f.body["timezone"].as_str().filter(|z| zone_ok(z))
    {
        return Some(z.to_owned());
    }
    let r = fetcher.get("https://wttr.in/?format=%Z", crate::fetch::Format::Text, &|h: &str| h == "wttr.in").ok()?;
    let z = String::from_utf8_lossy(&r.body).trim().to_owned();
    (r.status == 200 && zone_ok(&z)).then_some(z)
}

/// An Olson id's shape: `Area/Place` parts of letters, digits, `_`, `+` and `-`.
pub fn zone_ok(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= 64
        && zone.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c)))
}

/// Sets Android's time zone. Alarms keep their local times.
async fn apply_zone(hub: &Arc<Hub>, zone: &str) -> anyhow::Result<()> {
    anyhow::ensure!(zone_ok(zone), "`{zone}` isn't a time zone id like America/New_York.");
    anyhow::ensure!(cfg!(target_os = "android"), "Only the R1 has a time zone to set.");
    let out = crate::tool::run("cmd", &["alarm", "set-timezone", zone], crate::tool::DEADLINE).await?;
    anyhow::ensure!(out.ok && !out.stdout.contains("Exception"), "Android didn't take `{zone}`: {}", out.stderr.trim());
    // Alarms ring at local times: their next times move with the zone.
    let h = hub.clone();
    tokio::task::spawn_blocking(move || h.core.with(rezone)).await??;
    hub.scheduler.wake();
    hub.emit(Event::AlarmsChanged, Source::Api);
    Ok(())
}

/// Works out each alarm's next time again (the zone changed).
fn rezone(db: &Connection) -> anyhow::Result<()> {
    let clock = enee_spec::sql::Clock::system();
    let now = clock.now_ms();
    let offset = |ms: i64| clock.offset_ms(ms);
    for a in list(db)?.into_iter().filter(|a| a.kind == "alarm" && a.on && a.ringing.is_none()) {
        let at = next_ring(a.minute.unwrap_or(0), a.days, now, &offset);
        db.execute("UPDATE alarms SET at = ?2 WHERE id = ?1", params![a.id, at])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db
    }

    /// New York in October: UTC-4.
    fn edt(_: i64) -> i64 {
        -4 * 3_600_000
    }

    /// Friday 2026-10-09 18:46 EDT.
    const NOW: i64 = 1_791_585_960_000;

    #[test]
    fn times_and_days_from_agents() {
        assert_eq!(parse_time("7"), Some(420));
        assert_eq!(parse_time("7:30 am"), Some(450));
        assert_eq!(parse_time("12am"), Some(0));
        assert_eq!(parse_time("12:15 pm"), Some(735));
        assert_eq!(parse_time("7pm"), Some(1140));
        assert_eq!(parse_time("19:05"), Some(1145));
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("13pm"), None);
        assert_eq!(parse_days(&json!(["mon", "Wednesday"])), Ok(0b101));
        assert_eq!(parse_days(&json!("weekdays")), Ok(0x1f));
        assert_eq!(parse_days(&json!("Saturdays and Sundays")), Ok(0x60));
        assert_eq!(parse_days(&json!(null)), Ok(0));
        assert!(parse_days(&json!(["someday"])).is_err());
        assert_eq!(describe_days(0b101), "Mon, Wed");
        assert_eq!(describe_time(NOW, &edt), "Friday 2026-10-09 18:46");
        assert_eq!(parse_when("2026-10-10 07:30", NOW, &edt).map(|t| describe_time(t, &edt)).as_deref(), Some("Saturday 2026-10-10 07:30"));
        assert_eq!(parse_when("17:00", NOW, &edt).map(|t| describe_time(t, &edt)).as_deref(), Some("Saturday 2026-10-10 17:00"));
        assert_eq!(parse_when("19:00", NOW, &edt).map(|t| describe_time(t, &edt)).as_deref(), Some("Friday 2026-10-09 19:00"));
    }

    #[test]
    fn alarms_ring_on_their_days_and_move_on() {
        let db = db();
        // Weekdays at 7: Friday evening now, so Monday.
        let a = set_alarm(&db, None, 420, 0x1f, " wake  up ", Some("nova"), NOW, &edt).unwrap();
        assert_eq!(a.label, "wake up");
        assert_eq!(describe_time(a.at.unwrap(), &edt), "Monday 2026-10-12 07:00");
        // Once, at 19:00 today.
        let once = set_alarm(&db, None, 1140, 0, "", None, NOW, &edt).unwrap();
        let (went, next) = due(&db, NOW, &edt).unwrap();
        assert!(went.is_empty());
        assert_eq!(next, once.at);
        let at = once.at.unwrap();
        let (went, next) = due(&db, at + 1000, &edt).unwrap();
        assert_eq!(went.len(), 1);
        assert_eq!(went[0].ringing, Some(at + 1000));
        assert_eq!(went[0].at, None, "a one-off alarm has no next time");
        assert_eq!(next, a.at);
        // Snoozed, it rings again in ten minutes; stopped then, it turns off.
        assert!(snooze(&db, &once.id, at + 5000).unwrap());
        let (went, _) = due(&db, at + 5000 + SNOOZE_MS, &edt).unwrap();
        assert_eq!(went.len(), 1);
        assert!(stop(&db, &once.id).unwrap());
        assert!(!get(&db, &once.id).unwrap().unwrap().on);
        // A repeating alarm stays on, on its next day.
        let monday = a.at.unwrap();
        let (went, _) = due(&db, monday, &edt).unwrap();
        assert_eq!(describe_time(went[0].at.unwrap(), &edt), "Tuesday 2026-10-13 07:00");
        assert!(stop(&db, &a.id).unwrap());
        assert!(get(&db, &a.id).unwrap().unwrap().on);
        // Too late to ring (the R1 was off): Tuesday's moves on to Wednesday, silently.
        let tuesday = get(&db, &a.id).unwrap().unwrap().at.unwrap();
        let (went, _) = due(&db, tuesday + 2 * 3_600_000, &edt).unwrap();
        assert!(went.is_empty());
        assert_eq!(describe_time(get(&db, &a.id).unwrap().unwrap().at.unwrap(), &edt), "Wednesday 2026-10-14 07:00");
    }

    #[test]
    fn repeating_reminders_and_todos() {
        let db = db();
        // "Every weekday at 9: take vitamins": Monday first (Friday evening now), then on to Tuesday.
        let w = when_from(&json!({ "time": "9:00", "days": "weekdays" }), NOW, &edt).unwrap();
        let r = set_reminder(&db, w, "take vitamins", None, NOW).unwrap();
        assert_eq!(describe_time(r.at.unwrap(), &edt), "Monday 2026-10-12 09:00");
        let (went, _) = due(&db, r.at.unwrap(), &edt).unwrap();
        assert_eq!(describe_time(went[0].at.unwrap(), &edt), "Tuesday 2026-10-13 09:00");
        assert!(seen(&db, &r.id).unwrap());
        let r = get(&db, &r.id).unwrap().expect("a repeating one stays");
        assert_eq!(r.fired, None);
        // A to-do: no time, never goes off; ticked, it goes a day later (or when cleared).
        let t = set_reminder(&db, when_from(&json!({ "text": "buy milk" }), NOW, &edt).unwrap(), "buy milk", None, NOW).unwrap();
        assert!(t.is_todo() && t.at.is_none());
        assert!(due(&db, NOW + 10 * DAY_MS, &edt).unwrap().0.iter().all(|a| a.id != t.id));
        assert!(tick(&db, &r.id, true, NOW).is_err(), "only to-dos are ticked");
        assert_eq!(tick(&db, &t.id, true, NOW).unwrap().done, Some(NOW));
        assert_eq!(clear_done(&db, Some(NOW)).unwrap(), 0, "not a day old");
        assert_eq!(tick(&db, &t.id, false, NOW).unwrap().done, None);
        tick(&db, &t.id, true, NOW).unwrap();
        assert_eq!(clear_done(&db, Some(NOW + DAY_MS)).unwrap(), 1);
        // "9:00" alone is the next 9:00, once.
        assert!(matches!(when_from(&json!({ "time": "9:00" }), NOW, &edt).unwrap(), When::At(_)));
        assert!(migrate(&db).is_ok(), "again, with the column there");
    }

    #[test]
    fn reminders_go_off_once_until_seen() {
        let db = db();
        assert!(set_reminder(&db, When::At(NOW - 3_600_000), "late", None, NOW).is_err());
        let r = set_reminder(&db, When::At(NOW + 20 * MIN_MS), "call mom", Some("nova"), NOW).unwrap();
        let (went, _) = due(&db, NOW + 21 * MIN_MS, &edt).unwrap();
        assert_eq!(went[0].fired, Some(NOW + 21 * MIN_MS));
        assert_eq!(went[0].at, None);
        assert!(due(&db, NOW + 30 * MIN_MS, &edt).unwrap().0.is_empty(), "once");
        assert!(seen(&db, &r.id).unwrap());
        assert!(list(&db).unwrap().is_empty());
    }
}
