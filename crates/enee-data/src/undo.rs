//! Recorded runs and putting them back (the command bar's Undo, overlay.md §4): every row a run of steps changes is
//! kept as it was before, so [`AppDb::revert`] can restore it.
//!
//! - The writer is held for the whole run, so nothing else changes rows in between.
//! - SQLite's update hook reports each changed row, cascades included. A row's first change in the run decides what
//!   it was: absent for an insert; for an update or a delete, its values in a copy of the tables taken before the
//!   run's first statement that could rewrite rows.
//! - A run can't be put back if a statement replaces rows (`OR REPLACE` deletes rows the hook never reports), or if
//!   it rewrites a table too big to copy.
//! - Putting back is one transaction, with foreign keys checked at commit, newest change first: inserted rows are
//!   deleted, changed rows get their values back, and deleted rows are inserted again with their ids.

use std::collections::{BTreeSet, HashMap};
use std::sync::MutexGuard;
use std::sync::atomic::Ordering;

use rusqlite::Connection;
use rusqlite::hooks::Action;
use serde_json::{Map, Value};

use crate::migrate::quote;
use crate::{AppDb, Exec, Guarded, bind, lock, rows, to_sql};

/// Tables with more rows than this aren't copied, so a run that rewrites one can't be put back.
const MAX_COPIED_ROWS: i64 = 20_000;
/// The copy's name for each row's rowid.
const ROWID: &str = "_enee_rowid";

/// A table's rows by rowid, or `None` when it was too big to copy.
type Copy = HashMap<String, Option<HashMap<i64, Map<String, Value>>>>;

/// Every row a recorded run changed, as it was before, in the order of their first changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Changes {
    rows: Vec<Changed>,
    revertible: bool,
}

/// One row a run changed.
#[derive(Clone, Debug, PartialEq)]
struct Changed {
    table: String,
    rowid: i64,
    /// The row before its first change; `None` when the run inserted it.
    before: Option<Map<String, Value>>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// False when the run changed rows that can't be put back.
    pub fn revertible(&self) -> bool {
        self.revertible
    }

    /// The tables whose rows changed.
    pub fn tables(&self) -> Vec<String> {
        self.rows.iter().map(|row| row.table.clone()).collect::<BTreeSet<_>>().into_iter().collect()
    }
}

/// A run of steps on the writer whose row changes are recorded ([`AppDb::record`]).
pub struct Recording<'a> {
    db: &'a AppDb,
    conn: MutexGuard<'a, Guarded>,
    /// The tables before the run's first statement that could rewrite rows; `None` until then.
    copy: Option<Copy>,
    changes: Changes,
}

impl AppDb {
    /// Starts a recorded run of steps. It holds the writer until [`Recording::finish`].
    pub fn record(&self) -> Recording<'_> {
        Recording {
            db: self,
            conn: lock(&self.writer),
            copy: None,
            changes: Changes { rows: Vec::new(), revertible: true },
        }
    }

    /// Puts back the rows a recorded run changed, in one transaction: on any failure nothing changes.
    pub fn revert(&self, changes: &Changes) -> anyhow::Result<Exec> {
        anyhow::ensure!(changes.revertible, "this change can't be put back");
        let conn = lock(&self.writer);
        let guard = conn.guard.clone();
        // The host's own statements: a transaction and a pragma, which app SQL may not use.
        guard.internal.store(true, Ordering::Relaxed);
        let result = conn.run(self.timeouts.step, |db| {
            db.execute_batch("BEGIN IMMEDIATE; PRAGMA defer_foreign_keys = ON;")?;
            let restored = restore(db, changes).and_then(|n| db.execute_batch("COMMIT").map(|()| n));
            if restored.is_err() {
                let _ = db.execute_batch("ROLLBACK");
            }
            restored
        });
        guard.internal.store(false, Ordering::Relaxed);
        let (rows_affected, written) = result?;
        Ok(Exec { changed_tables: written.touched.into_iter().collect(), rows_affected })
    }
}

impl Recording<'_> {
    /// Runs one step, like [`AppDb::exec`], recording the rows it changes.
    pub fn exec(&mut self, sql: &str, params: &Map<String, Value>) -> anyhow::Result<Exec> {
        if replaces(sql) {
            self.changes.revertible = false;
        }
        let guard = self.conn.guard.clone();
        *lock(&guard.rows) = Some(Vec::new());
        let copy = &mut self.copy;
        let result = self.conn.run(self.db.timeouts.step, |db| {
            let mut stmt = db.prepare(sql)?;
            // The authorizer has seen the statement: copy the tables before its first update or delete.
            if copy.is_none() && lock(&guard.written).rewrites {
                *copy = Some(copy_tables(db)?);
            }
            bind(&mut stmt, params)?;
            if stmt.column_count() > 0 {
                rows(&mut stmt)?;
                Ok(db.changes())
            } else {
                Ok(stmt.raw_execute()? as u64)
            }
        });
        let changed = lock(&guard.rows).take().unwrap_or_default();
        let (rows_affected, written) = result?;
        for (action, table, rowid) in changed {
            if self.changes.rows.iter().any(|row| row.table == table && row.rowid == rowid) {
                continue; // its first change decides what it was
            }
            let before = match action {
                Action::SQLITE_INSERT => None,
                Action::SQLITE_UPDATE | Action::SQLITE_DELETE => {
                    let old = self.copy.as_ref().and_then(|copy| copy.get(&table)).and_then(Option::as_ref);
                    match old.and_then(|rows| rows.get(&rowid)) {
                        Some(row) => Some(row.clone()),
                        None => {
                            self.changes.revertible = false;
                            None
                        }
                    }
                }
                _ => {
                    self.changes.revertible = false;
                    None
                }
            };
            self.changes.rows.push(Changed { table, rowid, before });
        }
        let changed_tables = if rows_affected > 0 || !written.touched.is_empty() {
            written.planned.union(&written.touched).cloned().collect()
        } else {
            Vec::new()
        };
        Ok(Exec { changed_tables, rows_affected })
    }

    /// Ends the run, releasing the writer.
    pub fn finish(self) -> Changes {
        self.changes
    }
}

/// `INSERT OR REPLACE`, `REPLACE INTO`, `UPDATE OR REPLACE`: the rows they replace go without the update hook.
fn replaces(sql: &str) -> bool {
    let upper = sql.to_ascii_uppercase();
    let words: Vec<&str> =
        upper.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    words.first() == Some(&"REPLACE") || words.windows(2).any(|pair| pair == ["OR", "REPLACE"])
}

/// Every table's rows by rowid; tables past [`MAX_COPIED_ROWS`] are left out (`None`).
fn copy_tables(db: &Connection) -> rusqlite::Result<Copy> {
    let names: Vec<String> = {
        let mut stmt = db.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite%'")?;
        stmt.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let mut copy = Copy::new();
    for name in names {
        let table = quote(&name);
        let count: i64 = db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))?;
        if count > MAX_COPIED_ROWS {
            copy.insert(name, None);
            continue;
        }
        let mut stmt = db.prepare(&format!("SELECT rowid AS {ROWID}, * FROM {table}"))?;
        let mut kept = HashMap::new();
        for mut row in rows(&mut stmt)? {
            if let Some(rowid) = row.remove(ROWID).and_then(|v| v.as_i64()) {
                kept.insert(rowid, row);
            }
        }
        copy.insert(name, Some(kept));
    }
    Ok(copy)
}

/// Undoes `changes`, newest first. Runs inside the revert's transaction.
fn restore(db: &Connection, changes: &Changes) -> rusqlite::Result<u64> {
    let mut restored = 0;
    for Changed { table, rowid, before } in changes.rows.iter().rev() {
        let t = quote(table);
        restored += match before {
            None => db.execute(&format!("DELETE FROM {t} WHERE rowid = ?1"), [rowid])? as u64,
            Some(row) => {
                let exists: i64 =
                    db.query_row(&format!("SELECT count(*) FROM {t} WHERE rowid = ?1"), [rowid], |r| r.get(0))?;
                let columns: Vec<String> = row.keys().map(|c| quote(c)).collect();
                let mut values: Vec<rusqlite::types::Value> = row.values().map(to_sql).collect();
                if exists > 0 {
                    let set: Vec<String> =
                        columns.iter().enumerate().map(|(i, c)| format!("{c} = ?{}", i + 1)).collect();
                    values.push(rusqlite::types::Value::Integer(*rowid));
                    let sql = format!("UPDATE {t} SET {} WHERE rowid = ?{}", set.join(", "), values.len());
                    db.execute(&sql, rusqlite::params_from_iter(values))? as u64
                } else {
                    let marks: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
                    let sql = format!("INSERT INTO {t} ({}) VALUES ({})", columns.join(", "), marks.join(", "));
                    db.execute(&sql, rusqlite::params_from_iter(values))? as u64
                }
            }
        };
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_statements_are_spotted() {
        assert!(replaces("INSERT OR REPLACE INTO t (a) VALUES (1)"));
        assert!(replaces("replace into t (a) values (1)"));
        assert!(!replaces("UPDATE t SET name = replace(name, 'a', 'b')"));
        assert!(!replaces("INSERT INTO t (a) VALUES (1) ON CONFLICT(a) DO UPDATE SET b = 2"));
    }
}
