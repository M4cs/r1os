//! An app's reminders that went off (ADR-0024), kept until you've seen them (T-049): the shell shows the newest again
//! when the R1 wakes, as it does an agent's reminder (ADR-0028), and What's up lists them. Seeing one (a touch on its
//! banner, or the wheel or the button while it's up) deletes it. At most a day old, and the newest [`MAX`].

use rusqlite::{Connection, params};
use serde::Serialize;

/// How long one is kept unseen.
const KEEP_MS: i64 = 24 * 60 * 60_000;
/// At most this many (the newest).
const MAX: i64 = 50;

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_notices (
            id    INTEGER PRIMARY KEY,
            app   TEXT NOT NULL,
            title TEXT NOT NULL,
            body  TEXT NOT NULL,
            at    INTEGER NOT NULL
        );",
    )
}

/// One that went off and hasn't been seen.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Notice {
    pub id: i64,
    pub app: String,
    pub title: String,
    pub body: String,
    pub at: i64,
}

/// Keeps one that just went off. Its id.
pub fn add(db: &Connection, app: &str, title: &str, body: &str, now: i64) -> rusqlite::Result<i64> {
    db.execute("INSERT INTO app_notices (app, title, body, at) VALUES (?1, ?2, ?3, ?4)", params![app, title, body, now])?;
    let id = db.last_insert_rowid();
    db.execute("DELETE FROM app_notices WHERE at < ?1", [now - KEEP_MS])?;
    db.execute("DELETE FROM app_notices WHERE id NOT IN (SELECT id FROM app_notices ORDER BY at DESC, id DESC LIMIT ?1)", [MAX])?;
    Ok(id)
}

/// The unseen ones, newest first.
pub fn unseen(db: &Connection, now: i64) -> rusqlite::Result<Vec<Notice>> {
    let mut st = db.prepare("SELECT id, app, title, body, at FROM app_notices WHERE at >= ?1 ORDER BY at DESC, id DESC")?;
    st.query_map([now - KEEP_MS], |r| Ok(Notice { id: r.get(0)?, app: r.get(1)?, title: r.get(2)?, body: r.get(3)?, at: r.get(4)? }))?.collect()
}

/// It was seen: gone. Whether it was there.
pub fn seen(db: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM app_notices WHERE id = ?1", [id])? == 1)
}

/// An app was removed: its notices go with it.
pub fn forget_app(db: &Connection, app: &str) -> rusqlite::Result<()> {
    db.execute("DELETE FROM app_notices WHERE app = ?1", [app])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_until_seen_and_for_a_day() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let day = 24 * 60 * 60_000;
        let old = add(&db, "water", "Time for water", "2/8", 0).unwrap();
        let a = add(&db, "water", "Time for water", "3/8", day).unwrap();
        let b = add(&db, "habits", "Stretch", "", day + 5).unwrap();
        let now = day + 2_000;
        // The first is over a day old by the time the others came: gone.
        assert_eq!(unseen(&db, now).unwrap().iter().map(|n| n.id).collect::<Vec<_>>(), vec![b, a]);
        assert!(!seen(&db, old).unwrap());
        assert!(seen(&db, b).unwrap());
        forget_app(&db, "water").unwrap();
        assert!(unseen(&db, now).unwrap().is_empty());
    }
}
