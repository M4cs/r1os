//! The Journal (ADR-0033): a timeline of notes, photos and recordings, kept in core.db. You add to it by
//! talking to an agent ("journal: ran 5k this morning"), and from a photo's or a recording's page; agents
//! add and search with their tools (`journal_add`, `journal_search`, `journal_list`).
//!
//! An entry points at a photo or recording (crate::media) rather than copying it; deleting the media leaves
//! the entry's words.

use rusqlite::{Connection, params};
use serde::Serialize;

/// The longest entry.
const MAX_TEXT: usize = 4000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub at: i64,
    /// `note`, `photo` or `recording`.
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// The agent that wrote it, if one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS journal (
           id TEXT PRIMARY KEY, at INTEGER NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL DEFAULT '', media TEXT, agent TEXT);
         CREATE INDEX IF NOT EXISTS journal_at ON journal(at);",
    )
}

const COLUMNS: &str = "id, at, kind, text, media, agent";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry { id: r.get(0)?, at: r.get(1)?, kind: r.get(2)?, text: r.get(3)?, media: r.get(4)?, agent: r.get(5)? })
}

/// A new entry: words, and maybe a photo or recording (its kind comes from the media).
pub fn add(db: &Connection, text: &str, media: Option<&str>, agent: Option<&str>, at: i64) -> anyhow::Result<Entry> {
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    let kind = match media {
        Some(m) => crate::media::get(db, m)?.ok_or_else(|| anyhow::anyhow!("There's no photo or recording `{m}`."))?.kind,
        None => {
            anyhow::ensure!(!text.is_empty(), "Say what to write.");
            "note".into()
        }
    };
    let e = Entry { id: crate::core::random_hex(6), at, kind, text, media: media.map(str::to_owned), agent: agent.map(str::to_owned) };
    db.execute(
        &format!("INSERT INTO journal ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"),
        params![e.id, e.at, e.kind, e.text, e.media, e.agent],
    )?;
    Ok(e)
}

/// Newest first, before `before` (ms) if given.
pub fn list(db: &Connection, before: Option<i64>, limit: i64) -> rusqlite::Result<Vec<Entry>> {
    let mut st = db.prepare(&format!("SELECT {COLUMNS} FROM journal WHERE ?1 IS NULL OR at < ?1 ORDER BY at DESC LIMIT ?2"))?;
    st.query_map(params![before, limit.clamp(1, 500)], row)?.collect()
}

/// Entries whose words (or their recording's transcript, or their media's label) have every word of `query`,
/// within `from`..`to` (ms) if given. Newest first.
pub fn search(db: &Connection, query: &str, from: Option<i64>, to: Option<i64>, limit: i64) -> rusqlite::Result<Vec<Entry>> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).filter(|w| w.len() > 1).take(8).collect();
    let mut st = db.prepare(
        "SELECT j.id, j.at, j.kind, j.text, j.media, j.agent,
                lower(j.text || ' ' || coalesce(m.label, '') || ' ' || coalesce(m.transcript, ''))
         FROM journal j LEFT JOIN media m ON m.id = j.media
         WHERE (?1 IS NULL OR j.at >= ?1) AND (?2 IS NULL OR j.at < ?2)
         ORDER BY j.at DESC LIMIT 2000",
    )?;
    let found = st
        .query_map(params![from, to], |r| Ok((row(r)?, r.get::<_, String>(6)?)))?
        .filter_map(Result::ok)
        .filter(|(_, hay)| words.iter().all(|w| hay.contains(w.as_str())))
        .map(|(e, _)| e)
        .take(usize::try_from(limit.clamp(1, 100)).unwrap_or(20))
        .collect();
    Ok(found)
}

pub fn remove(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM journal WHERE id = ?1", [id])? == 1)
}

pub fn set_text(db: &Connection, id: &str, text: &str) -> rusqlite::Result<bool> {
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    Ok(db.execute("UPDATE journal SET text = ?2 WHERE id = ?1", params![id, text])? == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_added_listed_and_found() {
        let db = Connection::open_in_memory().unwrap();
        crate::media::migrate(&db).unwrap();
        migrate(&db).unwrap();
        assert!(add(&db, "  ", None, None, 1).is_err());
        let a = add(&db, "Ran 5k by the river", None, Some("nova"), 1_000).unwrap();
        let b = add(&db, "Dinner with Sam", None, None, 2_000).unwrap();
        assert_eq!(a.kind, "note");
        assert!(add(&db, "", Some("nope"), None, 3).is_err());
        assert_eq!(list(&db, None, 10).unwrap(), vec![b.clone(), a.clone()]);
        assert_eq!(list(&db, Some(2_000), 10).unwrap(), vec![a.clone()]);
        assert_eq!(search(&db, "river ran", None, None, 10).unwrap(), vec![a.clone()]);
        assert!(search(&db, "river sam", None, None, 10).unwrap().is_empty());
        assert_eq!(search(&db, "sam", Some(1_500), None, 10).unwrap(), vec![b.clone()]);
        assert!(set_text(&db, &b.id, "Dinner with Sam and Alex").unwrap());
        assert!(remove(&db, &a.id).unwrap());
        assert_eq!(list(&db, None, 10).unwrap().len(), 1);
    }
}
