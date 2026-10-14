//! Timers and pomodoros (ADR-0050), beside alarms (crate::alarms, ADR-0028). You start them by talking (an agent's
//! `set_timer`, or System One on its own: "set a timer for ten minutes"), on Tools › Timer, or from a linked computer.
//! They live in core.db, and the scheduler wakes for their ends with its kernel alarm (crate::wake), so they ring with
//! the R1 asleep.
//!
//! - **A timer** counts down from a length (1 s to 24 h): running (it `ends` at a time) or paused (time `left`). When
//!   it ends it rings on the alarm screen until a click stops it, and it's gone; a double click gives it a minute more.
//! - **A pomodoro** cycles: focus, a break, focus again (25, 5, and a long 15 after every 4th round; 4 rounds in a set;
//!   each can be changed). Each phase's end wakes the R1 with a note and one chime, and the next phase starts at once.
//!   The end of the set rings. There's one at a time.
//!
//! Lengths are durations, not times of day: the time zone doesn't touch them. One that ended more than half an hour
//! ago (the R1 was off) doesn't ring: a timer is gone, and a pomodoro catches up silently.

use crate::hub::{Event, Hub, Source};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

const SEC_MS: i64 = 1000;
const MIN_MS: i64 = 60_000;
/// The longest timer, and pomodoro phase.
const LONGEST_MS: i64 = 24 * 60 * MIN_MS;
/// Timers at most (an agent in a loop can't fill the database).
const MAX: i64 = 20;
/// A pomodoro's rounds at most: a forgotten one stops waking the R1.
const MAX_ROUNDS: i64 = 12;
/// One that ended this long ago doesn't ring.
const LATE_MS: i64 = 30 * MIN_MS;
/// A double click on a ringing timer (or snoozing it): this much more.
pub const MORE_MS: i64 = MIN_MS;

/// A pomodoro's lengths (ms) and rounds.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Cycle {
    pub focus: i64,
    #[serde(rename = "break")]
    pub short: i64,
    /// A long break, after every [`Cycle::every`]th round.
    pub long: i64,
    pub every: i64,
    /// Focus rounds in the set.
    pub rounds: i64,
}

impl Default for Cycle {
    fn default() -> Self {
        Cycle { focus: 25 * MIN_MS, short: 5 * MIN_MS, long: 15 * MIN_MS, every: 4, rounds: 4 }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Timer {
    pub id: String,
    /// `timer` or `pomodoro`.
    pub kind: String,
    /// Its label, as set ("pasta"); may be empty.
    pub label: String,
    /// What to call it on screen: its label, or "10 min timer", "pomodoro"; a set that's done says so.
    pub title: String,
    /// This run's length (a pomodoro's: its phase's), ms.
    pub length: i64,
    /// Running: when it ends (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends: Option<i64>,
    /// Paused: what's left (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<i64>,
    /// Ringing: since when (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ringing: Option<i64>,
    /// The agent that set it (the shell shows its face).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// A pomodoro's phase: `focus`, `break` or `long` (a long break).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// A pomodoro's focus round, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle: Option<Cycle>,
}

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS timers (
           id TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT NOT NULL DEFAULT '', length INTEGER NOT NULL,
           ends INTEGER, left_ms INTEGER, ringing INTEGER, agent TEXT, phase TEXT, round_no INTEGER,
           focus_ms INTEGER, short_ms INTEGER, long_ms INTEGER, every INTEGER, rounds INTEGER, created INTEGER NOT NULL);",
    )
}

const COLUMNS: &str = "id, kind, label, length, ends, left_ms, ringing, agent, phase, round_no, focus_ms, short_ms, long_ms, every, rounds";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Timer> {
    let cycle = match r.get::<_, Option<i64>>(10)? {
        Some(focus) => {
            let d = Cycle::default();
            Some(Cycle {
                focus,
                short: r.get::<_, Option<i64>>(11)?.unwrap_or(d.short),
                long: r.get::<_, Option<i64>>(12)?.unwrap_or(d.long),
                every: r.get::<_, Option<i64>>(13)?.unwrap_or(d.every),
                rounds: r.get::<_, Option<i64>>(14)?.unwrap_or(d.rounds),
            })
        }
        None => None,
    };
    let mut t = Timer {
        id: r.get(0)?,
        kind: r.get(1)?,
        label: r.get(2)?,
        title: String::new(),
        length: r.get(3)?,
        ends: r.get(4)?,
        left: r.get(5)?,
        ringing: r.get(6)?,
        agent: r.get(7)?,
        phase: r.get(8)?,
        round: r.get(9)?,
        cycle,
    };
    t.title = title(&t);
    Ok(t)
}

/// "pasta", "10 min timer", "pomodoro"; a pomodoro ringing at the end of its set: "pomodoro · 4 rounds done".
fn title(t: &Timer) -> String {
    let name = if !t.label.is_empty() {
        t.label.clone()
    } else if t.kind == "pomodoro" {
        "pomodoro".into()
    } else {
        format!("{} timer", describe(t.length))
    };
    match (&t.cycle, t.ringing) {
        (Some(c), Some(_)) => format!("{name} · {} done", rounds(c.rounds)),
        _ => name,
    }
}

fn rounds(n: i64) -> String {
    if n == 1 { "1 round".into() } else { format!("{n} rounds") }
}

/// Ringing first, then the soonest to end, then paused ones.
pub fn list(db: &Connection) -> rusqlite::Result<Vec<Timer>> {
    let mut st = db.prepare(&format!("SELECT {COLUMNS} FROM timers ORDER BY ringing IS NULL, ends IS NULL, ends, created"))?;
    st.query_map([], row)?.collect()
}

pub fn get(db: &Connection, id: &str) -> rusqlite::Result<Option<Timer>> {
    db.query_row(&format!("SELECT {COLUMNS} FROM timers WHERE id = ?1"), [id], row).optional()
}

fn must(db: &Connection, id: &str) -> anyhow::Result<Timer> {
    get(db, id)?.ok_or_else(|| anyhow::anyhow!("There's no timer `{id}`. Use list_alarms."))
}

fn room(db: &Connection) -> anyhow::Result<()> {
    let n: i64 = db.query_row("SELECT count(*) FROM timers", [], |r| r.get(0))?;
    anyhow::ensure!(n < MAX, "There are already {MAX} timers: cancel some first.");
    Ok(())
}

fn length_ok(ms: i64) -> anyhow::Result<()> {
    anyhow::ensure!((SEC_MS..=LONGEST_MS).contains(&ms), "A timer is 1 second to 24 hours long.");
    Ok(())
}

/// A timer of `length` (ms), running from `now`.
pub fn start(db: &Connection, length: i64, label: &str, agent: Option<&str>, now: i64) -> anyhow::Result<Timer> {
    length_ok(length)?;
    room(db)?;
    let id = crate::core::random_hex(4);
    db.execute(
        "INSERT INTO timers (id, kind, label, length, ends, agent, created) VALUES (?1, 'timer', ?2, ?3, ?4, ?5, ?6)",
        params![id, clean(label), length, now + length, agent, now],
    )?;
    Ok(get(db, &id)?.expect("just written"))
}

/// A pomodoro, its first focus round running from `now`. It replaces one that's running: there's one at a time.
pub fn pomodoro(db: &Connection, c: Cycle, label: &str, agent: Option<&str>, now: i64) -> anyhow::Result<Timer> {
    for ms in [c.focus, c.short, c.long] {
        length_ok(ms)?;
    }
    anyhow::ensure!((1..=MAX_ROUNDS).contains(&c.rounds), "A pomodoro has 1 to {MAX_ROUNDS} rounds.");
    anyhow::ensure!(c.every >= 1, "every: a long break after every 1 or more rounds.");
    db.execute("DELETE FROM timers WHERE kind = 'pomodoro'", [])?;
    room(db)?;
    let id = crate::core::random_hex(4);
    db.execute(
        "INSERT INTO timers (id, kind, label, length, ends, agent, phase, round_no, focus_ms, short_ms, long_ms, every, rounds, created)
         VALUES (?1, 'pomodoro', ?2, ?3, ?4, ?5, 'focus', 1, ?3, ?6, ?7, ?8, ?9, ?10)",
        params![id, clean(label), c.focus, now + c.focus, agent, c.short, c.long, c.every, c.rounds, now],
    )?;
    Ok(get(db, &id)?.expect("just written"))
}

/// What comes after a pomodoro's `phase` in `round`: (phase, round, length), or None at the end of the set.
fn next_phase(phase: &str, round: i64, c: &Cycle) -> Option<(&'static str, i64, i64)> {
    if phase != "focus" {
        return Some(("focus", round + 1, c.focus));
    }
    if round >= c.rounds {
        None
    } else if round % c.every.max(1) == 0 {
        Some(("long", round, c.long))
    } else {
        Some(("break", round, c.short))
    }
}

/// Paused, with what's left kept.
pub fn pause(db: &Connection, id: &str, now: i64) -> anyhow::Result<Timer> {
    let t = must(db, id)?;
    anyhow::ensure!(t.ringing.is_none(), "It's ringing: stop it instead.");
    if let Some(ends) = t.ends {
        db.execute("UPDATE timers SET ends = NULL, left_ms = ?2 WHERE id = ?1", params![id, (ends - now).max(SEC_MS)])?;
    }
    Ok(get(db, id)?.expect("still there"))
}

/// Running again, for what was left.
pub fn resume(db: &Connection, id: &str, now: i64) -> anyhow::Result<Timer> {
    let t = must(db, id)?;
    if let Some(left) = t.left {
        db.execute("UPDATE timers SET ends = ?2, left_ms = NULL WHERE id = ?1", params![id, now + left])?;
    }
    Ok(get(db, id)?.expect("still there"))
}

/// `ms` more (or less, negative), down to a second left and up to a day. A ringing one runs again for `ms`.
pub fn add(db: &Connection, id: &str, ms: i64, now: i64) -> anyhow::Result<Timer> {
    let t = must(db, id)?;
    match (t.ringing, t.ends, t.left) {
        (Some(_), ..) => {
            anyhow::ensure!(ms > 0, "It's ringing: add time, or stop it.");
            let ms = ms.min(LONGEST_MS);
            db.execute("UPDATE timers SET ringing = NULL, ends = ?2, length = ?3 WHERE id = ?1", params![id, now + ms, ms])?;
        }
        (None, Some(ends), _) => {
            let left = (ends - now + ms).clamp(SEC_MS, LONGEST_MS);
            db.execute("UPDATE timers SET ends = ?2, length = max(?3, length + ?4) WHERE id = ?1", params![id, now + left, left, ms])?;
        }
        (None, None, Some(left)) => {
            let left = (left + ms).clamp(SEC_MS, LONGEST_MS);
            db.execute("UPDATE timers SET left_ms = ?2, length = max(?2, length + ?3) WHERE id = ?1", params![id, left, ms])?;
        }
        (None, None, None) => anyhow::bail!("Timer `{id}` isn't running."),
    }
    Ok(get(db, id)?.expect("still there"))
}

/// A pomodoro's next phase now (running). At the end of its set it's done and gone: None.
pub fn skip(db: &Connection, id: &str, now: i64) -> anyhow::Result<Option<Timer>> {
    let t = must(db, id)?;
    let (Some(c), Some(phase), Some(round)) = (t.cycle, t.phase.as_deref(), t.round) else {
        anyhow::bail!("Only a pomodoro has phases to skip.");
    };
    match next_phase(phase, round, &c) {
        Some((p, r, len)) => {
            db.execute(
                "UPDATE timers SET phase = ?2, round_no = ?3, length = ?4, ends = ?5, left_ms = NULL, ringing = NULL WHERE id = ?1",
                params![id, p, r, len, now + len],
            )?;
            Ok(get(db, id)?)
        }
        None => {
            remove(db, id)?;
            Ok(None)
        }
    }
}

/// A ringing timer stopped: it's done, and gone. Whether it was ringing.
pub fn stop(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM timers WHERE id = ?1 AND ringing IS NOT NULL", [id])? == 1)
}

/// Cancelled. Whether it was there.
pub fn remove(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM timers WHERE id = ?1", [id])? == 1)
}

/// The timer ringing now, if one is.
pub fn ringing(db: &Connection) -> rusqlite::Result<Option<Timer>> {
    db.query_row(&format!("SELECT {COLUMNS} FROM timers WHERE ringing IS NOT NULL ORDER BY ringing DESC LIMIT 1"), [], row).optional()
}

/// What happened to a timer at its end.
#[derive(Debug, Clone, PartialEq)]
pub enum Went {
    /// It rang: a timer, or a pomodoro's set done.
    Rang(Timer),
    /// A pomodoro moved on to its next phase (running).
    Phase(Timer),
}

/// What ended by `now` (already recorded: ringing, or a pomodoro's next phase), and when the next one ends.
pub fn due(db: &Connection, now: i64) -> rusqlite::Result<(Vec<Went>, Option<i64>)> {
    let mut st = db.prepare(&format!("SELECT {COLUMNS} FROM timers WHERE ends IS NOT NULL AND ends <= ?1"))?;
    let ended: Vec<Timer> = st.query_map([now], row)?.collect::<rusqlite::Result<_>>()?;
    let mut went = Vec::new();
    for t in ended {
        let ends = t.ends.unwrap_or(now);
        let (Some(c), Some(phase), Some(round)) = (t.cycle, t.phase.clone(), t.round) else {
            if now - ends > LATE_MS {
                tracing::info!(timer = %t.id, late_min = (now - ends) / MIN_MS, "timer: too late to ring");
                remove(db, &t.id)?;
                continue;
            }
            db.execute("UPDATE timers SET ringing = ?2, ends = NULL WHERE id = ?1", params![t.id, now])?;
            went.extend(get(db, &t.id)?.map(Went::Rang));
            continue;
        };
        // From the phase that ended, through any that went by meanwhile (the R1 was off), to the one running now.
        let (mut phase, mut round, mut at) = (phase, round, ends);
        loop {
            match next_phase(&phase, round, &c) {
                None => {
                    if now - at > LATE_MS {
                        remove(db, &t.id)?;
                    } else {
                        db.execute(
                            "UPDATE timers SET ringing = ?2, ends = NULL, phase = ?3, round_no = ?4 WHERE id = ?1",
                            params![t.id, now, phase, round],
                        )?;
                        went.extend(get(db, &t.id)?.map(Went::Rang));
                    }
                    break;
                }
                Some((p, r, len)) if at + len <= now => (phase, round, at) = (p.to_owned(), r, at + len),
                Some((p, r, len)) => {
                    // Timed from when the last one ended, so a late wake doesn't stretch the next phase.
                    db.execute(
                        "UPDATE timers SET phase = ?2, round_no = ?3, length = ?4, ends = ?5 WHERE id = ?1",
                        params![t.id, p, r, len, at + len],
                    )?;
                    if now - at <= LATE_MS {
                        went.extend(get(db, &t.id)?.map(Went::Phase));
                    }
                    break;
                }
            }
        }
    }
    let next: Option<i64> = db.query_row("SELECT min(ends) FROM timers WHERE ends IS NOT NULL", [], |r| r.get(0))?;
    Ok((went, next))
}

/// Tells the shell: the display wakes (on the R1), and a timer rings or a pomodoro's note shows.
pub async fn went_off(hub: &Hub, w: Went) {
    crate::scheduler::wake_display().await;
    match w {
        Went::Rang(t) => {
            tracing::info!(id = %t.id, "timer rang");
            hub.emit(Event::Timer { id: t.id, label: t.title, agent: t.agent, ringing: true }, Source::Api);
        }
        Went::Phase(t) => {
            tracing::info!(id = %t.id, phase = ?t.phase, round = ?t.round, "pomodoro: next phase");
            hub.emit(
                Event::TimerPhase {
                    id: t.id,
                    label: t.title,
                    phase: t.phase.unwrap_or_default(),
                    round: t.round.unwrap_or(1),
                    rounds: t.cycle.map_or(1, |c| c.rounds),
                    length: t.length,
                    ends: t.ends.unwrap_or_default(),
                    agent: t.agent,
                },
                Source::Api,
            );
        }
    }
    hub.emit(Event::TimersChanged, Source::Api);
}

/// Stops a ringing timer, or gives it [`MORE_MS`] (`more`), and tells the shell (its alarm screen goes). Whether it
/// was ringing.
pub fn end_ring(hub: &Hub, id: &str, more: bool) -> anyhow::Result<bool> {
    let now = crate::alarms::clock().now_ms();
    let t = hub.core.with(|db| get(db, id))?;
    let Some(t) = t.filter(|t| t.ringing.is_some()) else { return Ok(false) };
    hub.core.with(|db| -> anyhow::Result<()> {
        if more {
            add(db, id, MORE_MS, now)?;
        } else {
            stop(db, id)?;
        }
        Ok(())
    })?;
    hub.emit(Event::Timer { id: t.id, label: t.title, agent: t.agent, ringing: false }, Source::Api);
    changed(hub);
    Ok(true)
}

/// Timers changed: the scheduler looks again at what's due, and what shows them reloads.
pub fn changed(hub: &Hub) {
    hub.scheduler.wake();
    hub.emit(Event::TimersChanged, Source::Api);
}

// ---- what agents, the shell and the companion pass -----------------------------------------------------

/// A timer or a pomodoro from what's passed: `minutes` and/or `seconds`, `label`; or `pomodoro: true` with `focus`,
/// `short_break`, `long_break` (minutes), `every` and `rounds`.
pub fn from_args(db: &Connection, args: &Value, agent: Option<&str>, now: i64) -> anyhow::Result<Timer> {
    let label = args["label"].as_str().unwrap_or_default();
    let minutes = |k: &str| -> anyhow::Result<Option<i64>> {
        match args[k].as_f64() {
            Some(m) if m > 0.0 && m <= 24.0 * 60.0 => Ok(Some((m * MIN_MS as f64).round() as i64)),
            Some(_) => anyhow::bail!("{k}: more than 0 minutes, at most 24 hours."),
            None => Ok(None),
        }
    };
    if args["pomodoro"].as_bool() == Some(true) || args["kind"] == "pomodoro" {
        let d = Cycle::default();
        let c = Cycle {
            focus: minutes("focus")?.unwrap_or(d.focus),
            short: minutes("short_break")?.unwrap_or(d.short),
            long: minutes("long_break")?.unwrap_or(d.long),
            every: args["every"].as_i64().unwrap_or(d.every),
            rounds: args["rounds"].as_i64().unwrap_or(d.rounds),
        };
        return pomodoro(db, c, label, agent, now);
    }
    let seconds = args["seconds"].as_f64().unwrap_or(0.0);
    anyhow::ensure!((0.0..=86_400.0).contains(&seconds), "seconds: 0 to a day's worth.");
    let length = minutes("minutes")?.unwrap_or(0) + (seconds * 1000.0).round() as i64;
    anyhow::ensure!(length > 0, "Give minutes or seconds.");
    start(db, length, label, agent, now)
}

/// `op` on timer `id`: `pause`, `resume`, `add` (`seconds` or `minutes`, a minute if neither; negative takes off),
/// `skip` (a pomodoro's phase) or `stop` (the ringing one). The timer after it, unless it's gone.
pub fn op(db: &Connection, id: &str, op: &str, args: &Value, now: i64) -> anyhow::Result<Option<Timer>> {
    match op {
        "pause" => pause(db, id, now).map(Some),
        "resume" => resume(db, id, now).map(Some),
        "add" => {
            let ms = match (args["seconds"].as_f64(), args["minutes"].as_f64()) {
                (Some(s), _) => s * 1000.0,
                (None, Some(m)) => m * MIN_MS as f64,
                (None, None) => MORE_MS as f64,
            };
            add(db, id, ms.round() as i64, now).map(Some)
        }
        "skip" => skip(db, id, now),
        "stop" => {
            must(db, id)?;
            stop(db, id)?;
            Ok(None)
        }
        _ => anyhow::bail!("action: pause, resume, add or skip."),
    }
}

/// "10 min", "1 h 30 min", "45 s", "2 min 30 s".
pub fn describe(ms: i64) -> String {
    let s = (ms.max(0) + 500) / 1000;
    let (h, m, sec) = (s / 3600, s % 3600 / 60, s % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h} h"));
    }
    if m > 0 {
        parts.push(format!("{m} min"));
    }
    if sec > 0 || parts.is_empty() {
        parts.push(format!("{sec} s"));
    }
    parts.join(" ")
}

/// How it stands, in a sentence for an agent.
pub fn said(t: &Timer, now: i64) -> String {
    let c = crate::alarms::clock();
    let state = match (t.ringing, t.ends, t.left) {
        (Some(_), ..) => "ringing now".to_owned(),
        (None, Some(ends), _) => format!(
            "{} left, ends {} (the R1's time)",
            describe(ends - now),
            crate::alarms::describe_time(ends, &|ms| c.offset_ms(ms))
        ),
        (None, None, Some(left)) => format!("paused with {} left", describe(left)),
        _ => "stopped".to_owned(),
    };
    match (&t.cycle, t.phase.as_deref(), t.round) {
        (Some(cy), Some(phase), Some(round)) => format!(
            "Pomodoro `{}`: {} of round {round} of {} ({} focus, {} breaks, a {} long break after every {}): {state}.",
            t.id,
            if phase == "focus" { "focus" } else if phase == "long" { "the long break" } else { "the break" },
            cy.rounds,
            describe(cy.focus),
            describe(cy.short),
            describe(cy.long),
            if cy.every == 1 { "round".to_owned() } else { format!("{} rounds", cy.every) },
        ),
        _ => format!("Timer `{}` ({}): {state}.", t.id, t.title),
    }
}

/// For an agent's list_alarms: what's running, with ids.
pub fn summary(list: &[Timer], now: i64) -> Value {
    json!(list.iter().map(|t| json!({ "id": t.id, "kind": t.kind, "about": said(t, now) })).collect::<Vec<_>>())
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db
    }

    const NOW: i64 = 1_791_585_960_000;

    #[test]
    fn a_timer_runs_pauses_rings_and_goes() {
        let db = db();
        assert!(start(&db, 0, "", None, NOW).is_err());
        assert!(start(&db, LONGEST_MS + 1, "", None, NOW).is_err());
        let t = start(&db, 10 * MIN_MS, "  the   pasta ", Some("nova"), NOW).unwrap();
        assert_eq!((t.label.as_str(), t.title.as_str(), t.ends), ("the pasta", "the pasta", Some(NOW + 10 * MIN_MS)));
        let short = start(&db, 90 * SEC_MS, "", None, NOW).unwrap();
        assert_eq!(short.title, "1 min 30 s timer");
        assert!(remove(&db, &short.id).unwrap());
        // Paused 4 minutes in, then resumed 2 minutes later: it ends 2 minutes late.
        let p = pause(&db, &t.id, NOW + 4 * MIN_MS).unwrap();
        assert_eq!((p.ends, p.left), (None, Some(6 * MIN_MS)));
        let (went, next) = due(&db, NOW + 10 * MIN_MS).unwrap();
        assert!(went.is_empty(), "a paused timer doesn't end");
        assert_eq!(next, None);
        let r = resume(&db, &t.id, NOW + 6 * MIN_MS).unwrap();
        assert_eq!(r.ends, Some(NOW + 12 * MIN_MS));
        // A minute more, then two less.
        assert_eq!(add(&db, &t.id, MIN_MS, NOW + 6 * MIN_MS).unwrap().ends, Some(NOW + 13 * MIN_MS));
        assert_eq!(add(&db, &t.id, -2 * MIN_MS, NOW + 6 * MIN_MS).unwrap().ends, Some(NOW + 11 * MIN_MS));
        // It rings, then a double click gives it a minute; stopped, it's gone.
        let (went, _) = due(&db, NOW + 11 * MIN_MS).unwrap();
        let rang = went.iter().find_map(|w| match w { Went::Rang(t) => Some(t.clone()), Went::Phase(_) => None }).unwrap();
        assert_eq!((rang.id.as_str(), rang.ringing, rang.ends), (t.id.as_str(), Some(NOW + 11 * MIN_MS), None));
        assert_eq!(ringing(&db).unwrap().map(|r| r.id), Some(t.id.clone()));
        assert!(pause(&db, &t.id, NOW).is_err(), "a ringing one isn't paused");
        let more = add(&db, &t.id, MORE_MS, NOW + 11 * MIN_MS).unwrap();
        assert_eq!((more.ringing, more.ends, more.title.as_str()), (None, Some(NOW + 12 * MIN_MS), "the pasta"));
        due(&db, NOW + 12 * MIN_MS).unwrap();
        assert!(stop(&db, &t.id).unwrap());
        assert!(get(&db, &t.id).unwrap().is_none());
        // One that ended long ago (the R1 was off) doesn't ring.
        let late = start(&db, MIN_MS, "", None, NOW).unwrap();
        let (went, _) = due(&db, NOW + MIN_MS + LATE_MS + 1).unwrap();
        assert!(went.iter().all(|w| !matches!(w, Went::Rang(t) if t.id == late.id)));
        assert!(get(&db, &late.id).unwrap().is_none());
    }

    #[test]
    fn a_pomodoro_cycles_and_rings_at_the_end_of_its_set() {
        let db = db();
        let c = Cycle { focus: 25 * MIN_MS, short: 5 * MIN_MS, long: 15 * MIN_MS, every: 2, rounds: 3 };
        let p = pomodoro(&db, c, "", Some("nova"), NOW).unwrap();
        assert_eq!((p.phase.as_deref(), p.round, p.title.as_str()), (Some("focus"), Some(1), "pomodoro"));
        let phase = |at: i64| match due(&db, at).unwrap().0.pop() {
            Some(Went::Phase(t)) => (t.phase.unwrap(), t.round.unwrap(), t.ends.unwrap() - at),
            other => panic!("{other:?}"),
        };
        // focus 1 → break → focus 2 → (every 2nd) long → focus 3 → the set's done: it rings.
        assert_eq!(phase(NOW + 25 * MIN_MS), ("break".into(), 1, 5 * MIN_MS));
        assert_eq!(phase(NOW + 30 * MIN_MS), ("focus".into(), 2, 25 * MIN_MS));
        assert_eq!(phase(NOW + 55 * MIN_MS), ("long".into(), 2, 15 * MIN_MS));
        assert_eq!(phase(NOW + 70 * MIN_MS), ("focus".into(), 3, 25 * MIN_MS));
        match due(&db, NOW + 95 * MIN_MS).unwrap().0.pop() {
            Some(Went::Rang(t)) => assert_eq!(t.title, "pomodoro · 3 rounds done"),
            other => panic!("{other:?}"),
        }
        assert!(stop(&db, &p.id).unwrap());
        // Skipping moves on now; skipping the last focus ends it. A new one replaces the old.
        let p = pomodoro(&db, Cycle { rounds: 2, ..Cycle::default() }, "deep work", None, NOW).unwrap();
        let s = skip(&db, &p.id, NOW + MIN_MS).unwrap().unwrap();
        assert_eq!((s.phase.as_deref(), s.ends), (Some("break"), Some(NOW + 6 * MIN_MS)));
        let again = pomodoro(&db, Cycle::default(), "", None, NOW).unwrap();
        assert!(get(&db, &p.id).unwrap().is_none(), "one at a time");
        skip(&db, &again.id, NOW).unwrap();
        assert!(skip(&db, &start(&db, MIN_MS, "", None, NOW).unwrap().id, NOW).is_err(), "a timer has no phases");
        // Away most of an hour (the R1 off): it catches up to the phase running now, timed as if it had been on,
        // and says so once.
        let p = pomodoro(&db, Cycle::default(), "", None, NOW).unwrap();
        assert_eq!(phase(NOW + 84 * MIN_MS), ("focus".into(), 3, MIN_MS));
        assert_eq!(get(&db, &p.id).unwrap().unwrap().ends, Some(NOW + 85 * MIN_MS));
        // A set that ended long ago is just gone.
        let p = pomodoro(&db, Cycle { rounds: 1, ..Cycle::default() }, "", None, NOW).unwrap();
        assert!(due(&db, NOW + 25 * MIN_MS + LATE_MS + 1).unwrap().0.is_empty());
        assert!(get(&db, &p.id).unwrap().is_none());
        assert!(pomodoro(&db, Cycle { rounds: 13, ..Cycle::default() }, "", None, NOW).is_err());
    }

    #[test]
    fn from_what_agents_pass() {
        let db = db();
        let t = from_args(&db, &json!({ "minutes": 1.5, "seconds": 15 }), None, NOW).unwrap();
        assert_eq!(t.length, 105 * SEC_MS);
        assert!(from_args(&db, &json!({}), None, NOW).is_err());
        assert!(from_args(&db, &json!({ "minutes": -3 }), None, NOW).is_err());
        let p = from_args(&db, &json!({ "pomodoro": true, "focus": 50, "short_break": 10 }), Some("nova"), NOW).unwrap();
        assert_eq!(p.cycle, Some(Cycle { focus: 50 * MIN_MS, short: 10 * MIN_MS, ..Cycle::default() }));
        assert_eq!(op(&db, &t.id, "add", &json!({ "minutes": -1 }), NOW).unwrap().unwrap().ends, Some(NOW + 45 * SEC_MS));
        assert_eq!(op(&db, &t.id, "pause", &json!({}), NOW).unwrap().unwrap().left, Some(45 * SEC_MS));
        assert!(op(&db, &t.id, "explode", &json!({}), NOW).is_err());
        assert_eq!(describe(3_600_000 + 30 * MIN_MS), "1 h 30 min");
        assert_eq!(describe(45 * SEC_MS), "45 s");
        assert_eq!(describe(0), "0 s");
        assert!(said(&p, NOW).starts_with("Pomodoro"), "{}", said(&p, NOW));
    }
}
