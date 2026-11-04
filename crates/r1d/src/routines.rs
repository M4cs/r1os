//! Routines (R7): an agent's turn on a schedule, as rabbitOS's proactive rabbit and OS3's background tasks do. "Every
//! weekday at 7:30, tell me the weather and what's on my day." You set one by talking to any agent (`set_routine`), or
//! from the companion; it runs as that agent, in the chat you have with it now (its most recent: a new chat you start is
//! where the next run goes, so routines don't leave a chat each), at a local time of day, once or on chosen weekdays (as
//! alarms do, crate::alarms). Each run shows there under its title and time (T-092).
//!
//! - **Battery first:** nothing runs between runs. The scheduler's one kernel alarm (crate::wake) wakes the R1 for the
//!   next one, and the host holds the CPU awake while the turn runs (`awake` events), at most five minutes.
//! - **What you see:** the reply waits in its chat, as unread (the glance screen's greeting counts it). With `show` on,
//!   the R1 wakes and a note at the top says it's there.
//! - **Nothing to tell:** a run can say nothing (2026-10-17, Max: Newsey sent "—" and Archer that there was nothing to
//!   do). Its agent replies just [nothing] (crate::chat::NOTHING), or with no words at all, or ends its words with
//!   [nothing] (T-206), and r1d keeps nothing of it: no reply, no divider, nothing unread, no note. So a routine can be
//!   "only if …" ("tell me if it'll rain").
//! - **Late:** one more than half an hour late (the R1 was off) doesn't run; it moves on to its next day.
//! - An agent that works on a computer runs it there (crate::remote), and it waits for that computer like any message.

use crate::hub::{Event, Hub, Source};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

const MIN_MS: i64 = 60_000;
/// One this late doesn't run.
const LATE_MS: i64 = 30 * MIN_MS;
/// How long a routine waits for its chat to stop answering you before it gives up on this run.
const BUSY_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// Routines at most.
const MAX: i64 = 30;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Routine {
    pub id: String,
    /// The agent that runs it.
    pub agent: String,
    /// What to call it ("Morning briefing").
    pub title: String,
    /// What the agent is asked each time.
    pub prompt: String,
    /// The local time of day, minutes after midnight.
    pub minute: i64,
    /// The weekdays it runs on, Monday = bit 0; 0 runs once.
    pub days: i64,
    pub on: bool,
    /// When it runs next (ms); None when it's off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    /// When it last ran (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<i64>,
    /// The chat its runs go to (made on the first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    /// Wake the R1 and say so when it's done.
    pub show: bool,
}

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS routines (
           id TEXT PRIMARY KEY, agent TEXT NOT NULL, title TEXT NOT NULL, prompt TEXT NOT NULL, minute INTEGER NOT NULL,
           days INTEGER NOT NULL DEFAULT 0, on_ INTEGER NOT NULL DEFAULT 1, at INTEGER, last INTEGER, chat TEXT,
           show INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL);",
    )
}

/// Runs from before T-092 kept the prompt as a plain message, which the chat showed as yours: marks them as the
/// routine's (its own chat's messages that are exactly its prompt), as new runs are. Safe to run again.
pub fn mark_prompts(db: &Connection) -> rusqlite::Result<usize> {
    db.execute(
        "UPDATE messages SET meta = json_set(COALESCE(meta, '{}'), '$.routine',
           (SELECT title FROM routines r WHERE r.chat = messages.chat AND r.prompt = messages.text LIMIT 1))
         WHERE chat IN (SELECT chat FROM routines WHERE chat IS NOT NULL) AND role = 'user'
           AND json_extract(meta, '$.routine') IS NULL
           AND EXISTS (SELECT 1 FROM routines r WHERE r.chat = messages.chat AND r.prompt = messages.text)",
        [],
    )
}

const COLUMNS: &str = "id, agent, title, prompt, minute, days, on_, at, last, chat, show";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Routine> {
    Ok(Routine {
        id: r.get(0)?,
        agent: r.get(1)?,
        title: r.get(2)?,
        prompt: r.get(3)?,
        minute: r.get(4)?,
        days: r.get(5)?,
        on: r.get::<_, i64>(6)? != 0,
        at: r.get(7)?,
        last: r.get(8)?,
        chat: r.get(9)?,
        show: r.get::<_, i64>(10)? != 0,
    })
}

/// By time of day.
pub fn list(db: &Connection) -> rusqlite::Result<Vec<Routine>> {
    let mut st = db.prepare(&format!("SELECT {COLUMNS} FROM routines ORDER BY minute, created"))?;
    st.query_map([], row)?.collect()
}

pub fn get(db: &Connection, id: &str) -> rusqlite::Result<Option<Routine>> {
    db.query_row(&format!("SELECT {COLUMNS} FROM routines WHERE id = ?1"), [id], row).optional()
}

fn clean(s: &str, max: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// What's passed to make or change one: `time` (7:30), `days`, `title`, `prompt`, `show`; `id` changes that one, and
/// `on` alone turns it on or off.
pub fn from_args(db: &Connection, args: &Value, agent: &str, now: i64, offset: crate::alarms::Offset) -> anyhow::Result<Routine> {
    let id = args["id"].as_str().filter(|s| !s.is_empty());
    if let (Some(id), None, Some(on)) = (id, args["time"].as_str(), args["on"].as_bool()) {
        return turn(db, id, on, now, offset);
    }
    let old = match id {
        Some(id) => Some(get(db, id)?.ok_or_else(|| anyhow::anyhow!("There's no routine `{id}`. Use list_routines."))?),
        None => None,
    };
    let minute = match args["time"].as_str() {
        Some(t) => crate::alarms::parse_time(t).ok_or_else(|| anyhow::anyhow!("time: `{t}` isn't a time of day like 7:30 or 19:05."))?,
        None => old.as_ref().map(|r| r.minute).ok_or_else(|| anyhow::anyhow!("Give a time of day (7:30)."))?,
    };
    let days = if args.get("days").is_some_and(|d| !d.is_null()) {
        crate::alarms::parse_days(&args["days"]).map_err(anyhow::Error::msg)?
    } else {
        old.as_ref().map_or(0, |r| r.days)
    };
    let title = args["title"].as_str().map(|t| clean(t, 60)).filter(|t| !t.is_empty()).or_else(|| old.as_ref().map(|r| r.title.clone()));
    let prompt = args["prompt"].as_str().map(|p| clean(p, 1000)).filter(|p| !p.is_empty()).or_else(|| old.as_ref().map(|r| r.prompt.clone()));
    let prompt = prompt.ok_or_else(|| anyhow::anyhow!("Say what to do each time (prompt)."))?;
    let title = title.unwrap_or_else(|| prompt.chars().take(40).collect());
    let show = args["show"].as_bool().or(old.as_ref().map(|r| r.show)).unwrap_or(false);
    let at = crate::alarms::next_ring(minute, days, now, offset);
    let id = match old {
        Some(r) => {
            db.execute(
                "UPDATE routines SET minute = ?2, days = ?3, title = ?4, prompt = ?5, show = ?6, at = ?7, on_ = 1 WHERE id = ?1",
                params![r.id, minute, days & 0x7f, title, prompt, show as i64, at],
            )?;
            r.id
        }
        None => {
            let n: i64 = db.query_row("SELECT count(*) FROM routines", [], |r| r.get(0))?;
            anyhow::ensure!(n < MAX, "There are already {MAX} routines: remove some first.");
            let id = crate::core::random_hex(4);
            db.execute(
                "INSERT INTO routines (id, agent, title, prompt, minute, days, at, show, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, agent, title, prompt, minute, days & 0x7f, at, show as i64, now],
            )?;
            id
        }
    };
    Ok(get(db, &id)?.expect("just written"))
}

/// On (at its next time) or off.
pub fn turn(db: &Connection, id: &str, on: bool, now: i64, offset: crate::alarms::Offset) -> anyhow::Result<Routine> {
    let r = get(db, id)?.ok_or_else(|| anyhow::anyhow!("There's no routine `{id}`."))?;
    let at = on.then(|| crate::alarms::next_ring(r.minute, r.days, now, offset));
    db.execute("UPDATE routines SET on_ = ?2, at = ?3 WHERE id = ?1", params![id, on as i64, at])?;
    Ok(get(db, id)?.expect("just written"))
}

pub fn remove(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM routines WHERE id = ?1", [id])? == 1)
}

/// What's due at `now` (moved on to their next times already; a one-off turns off), and when the next is.
pub fn due(db: &Connection, now: i64, offset: crate::alarms::Offset) -> rusqlite::Result<(Vec<Routine>, Option<i64>)> {
    let mut st = db.prepare(&format!("SELECT {COLUMNS} FROM routines WHERE on_ = 1 AND at IS NOT NULL AND at <= ?1"))?;
    let due: Vec<Routine> = st.query_map([now], row)?.collect::<rusqlite::Result<_>>()?;
    let mut run = Vec::new();
    for r in due {
        let at = r.at.unwrap_or(now);
        let next = (r.days != 0).then(|| crate::alarms::next_ring(r.minute, r.days, now, offset));
        db.execute("UPDATE routines SET at = ?2, on_ = ?3 WHERE id = ?1", params![r.id, next, next.is_some() as i64])?;
        if now - at > LATE_MS {
            tracing::info!(routine = %r.id, late_min = (now - at) / MIN_MS, "routine: too late to run");
            continue;
        }
        run.push(r);
    }
    let next: Option<i64> = db.query_row("SELECT min(at) FROM routines WHERE on_ = 1 AND at IS NOT NULL", [], |r| r.get(0))?;
    Ok((run, next))
}

/// Runs one: its agent's turn in its own chat, the CPU held awake meanwhile, and a note when it's done (with `show`).
pub async fn run(hub: Arc<Hub>, r: Routine) {
    tracing::info!(routine = %r.id, agent = %r.agent, "routine: running");
    let why = format!("routine:{}", r.id);
    hub.emit(Event::Awake { why: why.clone(), on: true }, Source::Api);
    if let Err(e) = run_turn(&hub, &r).await {
        tracing::warn!(routine = %r.id, error = %format!("{e:#}"), "routine: didn't run");
    }
    hub.emit(Event::Awake { why, on: false }, Source::Api);
}

async fn run_turn(hub: &Arc<Hub>, r: &Routine) -> anyhow::Result<()> {
    let agent = hub.core.agent(&r.agent).ok_or_else(|| anyhow::anyhow!("its agent is gone"))?;
    // The agent's current chat (Max, 2026-10-12: "the same chat session with an agent throughout … if they decide to
    // make a new chat that's when we do it"): the one you've had with it most lately, so routines don't leave a chat
    // each. A new chat you start is the current one from then on; one is made only if the agent has none.
    let chat = match hub.core.chats(Some(&agent.id), "user")?.into_iter().next() {
        Some(c) => c,
        None => {
            let c = hub.core.new_chat(&crate::core::random_hex(8), &agent.id, &agent.brain)?;
            hub.core.set_title(&c.id, &r.title)?;
            c
        }
    };
    // Where it last ran, and when, for what shows it (Tools › Routines' "its chat").
    hub.core.with(|db| db.execute("UPDATE routines SET chat = ?2, last = ?3 WHERE id = ?1", params![r.id, chat.id, crate::core::now()]))?;
    hub.emit(Event::RoutinesChanged, Source::Api);
    // Your chat may be answering you: the routine waits its turn (a while), rather than not running.
    let mut waited = std::time::Duration::ZERO;
    while !hub.chat_busy.lock().unwrap().insert(chat.id.clone()) {
        if waited >= BUSY_WAIT {
            anyhow::bail!("its chat was answering for {} minutes", BUSY_WAIT.as_secs() / 60);
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        waited += std::time::Duration::from_secs(5);
    }
    // Not the prompt as if you'd said it (T-092): its title, which an open chat shows as a divider between runs.
    hub.emit(Event::chat(&agent.id, &chat.id, "routine", Some(r.title.clone())), Source::Api);
    let reply = crate::chat::turn(hub.clone(), chat.id.clone(), r.prompt.clone(), Some(crate::chat::OnScreen::Routine(r.title.clone())), Vec::new()).await;
    hub.chat_busy.lock().unwrap().remove(&chat.id);
    // Nothing to show: it had nothing to tell, so nothing of it was kept, or it waits for its computer.
    let Some(text) = reply? else {
        tracing::info!(routine = %r.id, "routine: nothing to show");
        return Ok(());
    };
    if r.show {
        crate::scheduler::wake_display().await;
        hub.emit(Event::Routine { id: r.id.clone(), agent: agent.id.clone(), chat: chat.id.clone(), title: r.title.clone(), text: text.chars().take(200).collect() }, Source::Api);
    }
    Ok(())
}

/// Routines changed: the scheduler looks again, and what shows them reloads.
pub fn changed(hub: &Hub) {
    hub.scheduler.wake();
    hub.emit(Event::RoutinesChanged, Source::Api);
}

/// For an agent: what's set, with ids.
pub fn summary(list: &[Routine], hub: &Hub, offset: crate::alarms::Offset) -> Value {
    json!(list
        .iter()
        .map(|r| {
            let agent = hub.core.agent(&r.agent).map_or_else(|| r.agent.clone(), |a| a.name);
            json!({
                "id": r.id, "title": r.title, "prompt": r.prompt, "agent": agent, "on": r.on, "show": r.show,
                "time": format!("{:02}:{:02}", r.minute / 60, r.minute % 60),
                "repeats": crate::alarms::describe_days(r.days),
                "next": r.at.map(|t| crate::alarms::describe_time(t, offset)),
            })
        })
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db
    }

    fn edt(_: i64) -> i64 {
        -4 * 3_600_000
    }

    /// Friday 2026-10-09 18:46 EDT.
    const NOW: i64 = 1_791_585_960_000;

    #[test]
    fn routines_run_on_their_days_and_move_on() {
        let db = db();
        let r = from_args(&db, &json!({ "time": "7:30", "days": "weekdays", "title": "Morning", "prompt": "the weather and my day" }), "nova", NOW, &edt).unwrap();
        assert_eq!(crate::alarms::describe_time(r.at.unwrap(), &edt), "Monday 2026-10-12 07:30");
        assert!(from_args(&db, &json!({ "time": "7:30" }), "nova", NOW, &edt).is_err(), "a prompt is needed");
        // Changing only the time keeps the rest.
        let r = from_args(&db, &json!({ "id": r.id, "time": "8:00" }), "nova", NOW, &edt).unwrap();
        assert_eq!((r.minute, r.title.as_str(), r.days), (480, "Morning", 0x1f));
        // Due on Monday at 8: it runs and moves on to Tuesday.
        let monday = r.at.unwrap();
        let (run, next) = due(&db, monday + 1000, &edt).unwrap();
        assert_eq!(run.len(), 1);
        assert_eq!(crate::alarms::describe_time(next.unwrap(), &edt), "Tuesday 2026-10-13 08:00");
        // Off, then on again; too late (the R1 was off) doesn't run.
        assert!(!turn(&db, &r.id, false, NOW, &edt).unwrap().on);
        assert!(due(&db, monday + 7 * 86_400_000, &edt).unwrap().0.is_empty());
        let r = turn(&db, &r.id, true, monday + 1000, &edt).unwrap();
        let (run, _) = due(&db, r.at.unwrap() + LATE_MS + 1, &edt).unwrap();
        assert!(run.is_empty(), "too late");
        // Once: it turns off after.
        let once = from_args(&db, &json!({ "time": "19:00", "prompt": "remind me what I wanted to read" }), "nova", NOW, &edt).unwrap();
        let (run, _) = due(&db, once.at.unwrap(), &edt).unwrap();
        assert_eq!(run.len(), 1);
        assert!(!get(&db, &once.id).unwrap().unwrap().on);
        assert_eq!(get(&db, &once.id).unwrap().unwrap().title, "remind me what I wanted to read");
    }

    #[test]
    fn earlier_runs_prompts_are_marked_as_the_routines() {
        let db = db();
        db.execute_batch("CREATE TABLE messages (id INTEGER PRIMARY KEY, chat TEXT, role TEXT, text TEXT, at INTEGER, meta TEXT)").unwrap();
        let r = from_args(&db, &json!({ "time": "7:30", "title": "Morning", "prompt": "the weather" }), "nova", NOW, &edt).unwrap();
        db.execute("UPDATE routines SET chat = 'c1' WHERE id = ?1", [&r.id]).unwrap();
        db.execute_batch(
            "INSERT INTO messages (chat, role, text, at, meta) VALUES
               ('c1', 'user', 'the weather', 1, NULL), ('c1', 'assistant', 'sunny', 2, NULL),
               ('c1', 'user', 'and tomorrow?', 3, NULL), ('c2', 'user', 'the weather', 4, '{\"images\":[\"p1\"]}')",
        )
        .unwrap();
        assert_eq!(mark_prompts(&db).unwrap(), 1, "only its own chat's, and only the prompt");
        assert_eq!(mark_prompts(&db).unwrap(), 0, "once");
        let meta: Vec<Option<String>> =
            db.prepare("SELECT meta FROM messages ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(meta[0].as_deref(), Some(r#"{"routine":"Morning"}"#));
        assert_eq!((meta[2].as_deref(), meta[3].as_deref()), (None, Some(r#"{"images":["p1"]}"#)), "what you said yourself stays yours");
    }
}
