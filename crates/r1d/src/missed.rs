//! What you missed (R8, Max 2026-10-11: "agents could see notifications from android apps that also have connections
//! too so they could let you know about things that you missed"). Android apps' notifications that came while the
//! display was off are kept on the R1 for a day (core.db `missed`; never sent anywhere), each matched to the connection
//! that can read more of it (Gmail's to the Gmail connector, crate::connections). An agent's `what_you_missed` has what's
//! new since you last caught up (and the rest of the day's), so it reads the whole email or thread through the
//! connection (reading never asks) and tells you what matters. What's up shows them too ("while you were away").

use crate::hub::Hub;
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};

/// How long they're kept.
const KEEP_MS: i64 = 24 * 60 * 60_000;
/// At most this many (the newest).
const MAX: i64 = 300;

pub fn migrate(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS missed (
            id      INTEGER PRIMARY KEY,
            key     TEXT NOT NULL,
            package TEXT NOT NULL,
            app     TEXT NOT NULL,
            title   TEXT NOT NULL,
            text    TEXT NOT NULL,
            at      INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS missed_at ON missed (at);",
    )?;
    Ok(())
}

/// One that came while you weren't looking.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Missed {
    pub key: String,
    pub package: String,
    pub app: String,
    pub title: String,
    pub text: String,
    pub at: i64,
}

/// Apps whose notifications a connection can read more of: (package or its start, the connection's name in words).
const APPS: &[(&str, &str)] = &[
    ("com.google.android.gm", "Gmail"),
    ("com.google.android.calendar", "Google Calendar"),
    ("com.google.android.apps.docs", "Google Drive"),
    ("com.microsoft.office.outlook", "Outlook"),
    ("com.microsoft.teams", "Teams"),
    ("com.Slack", "Slack"),
    ("notion.id", "Notion"),
    ("com.github.android", "GitHub"),
    ("com.asana.app", "Asana"),
    ("com.atlassian.android.jira", "Jira"),
    ("com.dropbox.android", "Dropbox"),
    ("com.box.android", "Box"),
    ("com.linear", "Linear"),
    ("com.figma.mirror", "Figma"),
];

/// The connection that can read more of an app's notifications, if one's connected: its name ("Gmail") and its tools'
/// prefix (`mcp__claude_ai_Gmail__`). Outlook's is Microsoft 365's on claude.ai.
pub fn connection_for(hub: &Hub, package: &str) -> Option<(String, String)> {
    let (_, word) = APPS.iter().find(|(p, _)| package == *p || package.starts_with(&format!("{p}.")))?;
    let words: &[&str] = if *word == "Outlook" { &["Outlook", "Microsoft 365", "Microsoft"] } else { &[word] };
    crate::connections::servers(hub)
        .into_iter()
        .filter(|s| s.status == "connected")
        .find(|s| words.iter().any(|w| crate::connections::label(&s.name).eq_ignore_ascii_case(w) || crate::connections::label(&s.name).starts_with(w)))
        .map(|s| (crate::connections::label(&s.name).to_owned(), format!("mcp__{}__", crate::connections::slug(&s.name))))
}

/// A notification came with the display off: kept (unless it's the same words again).
pub fn record(db: &Connection, n: &crate::android::Note, now: i64) -> anyhow::Result<()> {
    if n.title.trim().is_empty() && n.text.trim().is_empty() {
        return Ok(());
    }
    let same: i64 = db.query_row(
        "SELECT count(*) FROM missed WHERE key = ?1 AND title = ?2 AND text = ?3 AND at > ?4",
        params![n.key, n.title, n.text, now - KEEP_MS],
        |r| r.get(0),
    )?;
    if same > 0 {
        return Ok(());
    }
    db.execute(
        "INSERT INTO missed (key, package, app, title, text, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![n.key, n.package, n.app, n.title, n.text.chars().take(1000).collect::<String>(), now],
    )?;
    db.execute("DELETE FROM missed WHERE at < ?1", params![now - KEEP_MS])?;
    db.execute("DELETE FROM missed WHERE id NOT IN (SELECT id FROM missed ORDER BY at DESC LIMIT ?1)", params![MAX])?;
    Ok(())
}

/// Forgets them all (`DELETE /api/missed`).
pub fn clear(db: &Connection) -> anyhow::Result<()> {
    db.execute("DELETE FROM missed", [])?;
    Ok(())
}

/// The day's, newest first.
pub fn list(db: &Connection, now: i64) -> anyhow::Result<Vec<Missed>> {
    let mut st = db.prepare("SELECT key, package, app, title, text, at FROM missed WHERE at > ?1 ORDER BY at DESC")?;
    let rows = st.query_map(params![now - KEEP_MS], |r| {
        Ok(Missed { key: r.get(0)?, package: r.get(1)?, app: r.get(2)?, title: r.get(3)?, text: r.get(4)?, at: r.get(5)? })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// When you last caught up (an agent told you, or What's up showed them): what came after is new.
pub fn caught_up(hub: &Hub) -> i64 {
    hub.core.missed_seen()
}

pub fn set_caught_up(hub: &Hub, at: i64) -> anyhow::Result<()> {
    hub.core.set_missed_seen(at)
}

/// For an agent's `what_you_missed`: what's new since you last caught up and the rest of the day's, each with the
/// connection that can read more of it; then you're caught up.
pub fn for_agent(hub: &Hub) -> anyhow::Result<Value> {
    let now = crate::core::now();
    let since = caught_up(hub);
    let all = hub.core.with(|db| list(db, now))?;
    let c = crate::alarms::clock();
    let when = |ms: i64| crate::alarms::describe_time(ms, &|m| c.offset_ms(m));
    let item = |m: &Missed| {
        let conn = connection_for(hub, &m.package);
        json!({
            "app": m.app, "title": m.title, "text": m.text, "at": m.at, "when": when(m.at),
            "connection": conn.as_ref().map(|(n, _)| n), "tools": conn.as_ref().map(|(_, p)| format!("{p}*")),
        })
    };
    let new: Vec<Value> = all.iter().filter(|m| m.at > since).map(item).collect();
    let earlier: Vec<Value> = all.iter().filter(|m| m.at <= since).take(30).map(item).collect();
    if !new.is_empty() {
        set_caught_up(hub, now)?;
    }
    Ok(json!({
        "new": new,
        "earlier_today": earlier,
        "last_caught_up": if since > 0 { Value::from(when(since)) } else { Value::Null },
        "how": "These came while the R1's screen was off. Where an item has a connection, read more through it before you \
                summarise (search its tools for the sender or subject: reading never asks the user). Tell the user what \
                matters and what needs them, briefly; skip the noise.",
    }))
}

/// For What's up: how many came since you last caught up, by app (newest app first), each with its connection.
pub fn summary(hub: &Hub) -> anyhow::Result<Value> {
    let now = crate::core::now();
    let since = caught_up(hub);
    let all: Vec<Missed> = hub.core.with(|db| list(db, now))?.into_iter().filter(|m| m.at > since).collect();
    let mut apps: Vec<(String, String, usize, String)> = Vec::new();
    for m in &all {
        match apps.iter_mut().find(|a| a.0 == m.package) {
            Some(a) => a.2 += 1,
            None => apps.push((m.package.clone(), m.app.clone(), 1, m.title.clone())),
        }
    }
    let apps: Vec<Value> = apps
        .into_iter()
        .map(|(package, app, count, newest)| json!({ "package": package, "app": app, "count": count, "newest": newest, "connection": connection_for(hub, &package).map(|c| c.0) }))
        .collect();
    Ok(json!({ "count": all.len(), "apps": apps }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(key: &str, title: &str, text: &str) -> crate::android::Note {
        crate::android::Note { key: key.into(), package: "com.google.android.gm".into(), app: "Gmail".into(), title: title.into(), text: text.into(), at: 0, can_reply: false }
    }

    #[test]
    fn kept_for_a_day_without_repeats() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let day = KEEP_MS;
        record(&db, &note("k1", "Sam", "dinner at 8?"), 1_000).unwrap();
        record(&db, &note("k1", "Sam", "dinner at 8?"), 2_000).unwrap(); // the same words again
        record(&db, &note("k1", "Sam", "or 9?"), 3_000).unwrap(); // the same thread, new words
        record(&db, &note("k2", "", ""), 4_000).unwrap(); // nothing to say
        let l = list(&db, 5_000).unwrap();
        assert_eq!(l.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["or 9?", "dinner at 8?"]);
        // A day on, they're gone.
        record(&db, &note("k3", "Bank", "statement ready"), day + 3_500).unwrap();
        assert_eq!(list(&db, day + 3_500).unwrap().len(), 1);
    }
}
