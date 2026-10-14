//! Migrations (ADR-0005, ADR-0023). The compiled DDL is idempotent (`CREATE … IF NOT EXISTS`, `INSERT OR IGNORE`), so
//! new tables, indexes and triggers simply appear. Columns a newer spec adds to a table that already exists are added
//! with `ALTER TABLE … ADD COLUMN`, after a snapshot of the database.
//!
//! SQLite refuses some column definitions in `ADD COLUMN`: `NOT NULL` without a constant default, and non-constant
//! defaults such as `= now`. Those columns are added without the constraint and the default, and existing rows get
//! the default's value once (with triggers paused, so `updated_at` isn't touched).
//!
//! Nothing is dropped unless an edit removed it ([`Reshape`]): a column or table that's only missing from the spec, as
//! after an undo, stays. What an edit removed is dropped after the snapshot, and an undo that puts it back gets its
//! values from the newest snapshot that has them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::config::DbConfig;
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, MAIN_DB, OpenFlags};

/// What a migration did.
#[derive(Clone, Debug, PartialEq)]
pub struct Migration {
    /// `table.column` of every added column.
    pub added: Vec<String>,
    /// The added columns that went in without `NOT NULL` or their default.
    pub relaxed: Vec<String>,
    /// `table` and `table.column` of what an edit removed, deleted with its data.
    pub dropped: Vec<String>,
    /// `table` and `table.column` an undo brought back with their values from an earlier snapshot.
    pub restored: Vec<String>,
    /// The copy of the database taken before the migration.
    pub snapshot: PathBuf,
}

/// What a migration may do besides adding (ADR-0023).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reshape {
    /// Tables an edit removed: dropped with their rows.
    pub drop_tables: Vec<String>,
    /// Columns an edit removed, `(table, column)`: dropped with their values.
    pub drop_columns: Vec<(String, String)>,
    /// An undo put back a version with tables or columns an edit deleted since: the ones this migration re-creates
    /// get their rows and values back from the newest snapshot that has them.
    pub restore: bool,
}

/// What an edit removed that the database still has.
#[derive(Debug, PartialEq)]
enum Drop {
    Table(String),
    Column(String, String),
}

/// Snapshots kept per app (the newest ones).
const KEEP_SNAPSHOTS: usize = 5;

/// A column the spec has and the existing table lacks.
#[derive(Debug, PartialEq)]
struct NewColumn {
    table: String,
    column: String,
    definition: String,
}

/// Brings the database to `ddl`: what an edit removed goes first, then added columns (an index in the DDL may need
/// them), then the DDL, all in one transaction after a snapshot. An undo's restore runs after it.
pub(crate) fn run(
    conn: &Connection,
    dir: &Path,
    ddl: &[String],
    reshape: &Reshape,
) -> anyhow::Result<Option<Migration>> {
    let plan = plan(conn, ddl)?;
    let drops = drops(conn, reshape)?;
    let existing = tables(conn)?;
    let created: Vec<String> =
        ddl.iter().filter_map(|stmt| create_table(stmt)).map(|(t, _)| t).filter(|t| !existing.contains(t)).collect();
    let restoring = reshape.restore && (!plan.is_empty() || !created.is_empty());
    let snapshot = if plan.is_empty() && drops.is_empty() && !restoring { None } else { Some(snapshot(conn, dir)?) };
    // A table rebuild needs foreign keys off, and SQLite switches them only outside a transaction.
    if !drops.is_empty() {
        conn.pragma_update(None, "foreign_keys", false)?;
    }
    let applied = apply(conn, ddl, &plan, &drops);
    if !drops.is_empty() {
        conn.pragma_update(None, "foreign_keys", true)?;
    }
    let (added, relaxed, dropped) = applied?;
    let Some(snapshot) = snapshot else { return Ok(None) };
    let restored = if reshape.restore { restore(conn, dir, &snapshot, &plan, &created) } else { Vec::new() };
    let migration = Migration { added, relaxed, dropped, restored, snapshot };
    tracing::info!(
        added = ?migration.added,
        relaxed = ?migration.relaxed,
        dropped = ?migration.dropped,
        restored = ?migration.restored,
        "app database migrated"
    );
    Ok(Some(migration))
}

type Applied = (Vec<String>, Vec<String>, Vec<String>);

/// The transaction: drops, added columns and the DDL. With drops, every foreign key must still hold at the end.
fn apply(conn: &Connection, ddl: &[String], plan: &[NewColumn], drops: &[Drop]) -> anyhow::Result<Applied> {
    let tx = conn.unchecked_transaction()?;
    let mut dropped = Vec::new();
    for drop in drops {
        match drop {
            Drop::Table(table) => {
                tx.execute_batch(&format!("DROP TABLE {}", quote(table)))
                    .with_context(|| format!("deleting {table}"))?;
                dropped.push(table.clone());
            }
            Drop::Column(table, column) => {
                drop_column(&tx, table, column).with_context(|| format!("deleting {table}.{column}"))?;
                dropped.push(format!("{table}.{column}"));
            }
        }
    }
    let mut added = Vec::new();
    let mut relaxed = Vec::new();
    for column in plan {
        let name = format!("{}.{}", column.table, column.column);
        if add_column(&tx, column).with_context(|| format!("adding {name}"))? {
            relaxed.push(name.clone());
        }
        added.push(name);
    }
    for stmt in ddl {
        tx.execute_batch(stmt).with_context(|| format!("running `{stmt}`"))?;
    }
    if !drops.is_empty() {
        let broken: i64 = tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))?;
        anyhow::ensure!(broken == 0, "deleting it would leave {broken} rows pointing at rows that are gone");
    }
    tx.commit()?;
    Ok((added, relaxed, dropped))
}

/// The tables in the database (not SQLite's own).
fn tables(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")?;
    let names = stmt.query_map([], |row| row.get::<_, String>(0))?;
    names.collect()
}

/// What `reshape` drops that the database has: tables first, then columns of the tables that stay.
fn drops(conn: &Connection, reshape: &Reshape) -> rusqlite::Result<Vec<Drop>> {
    let existing = tables(conn)?;
    let has = |t: &str| existing.contains(t);
    let mut out: Vec<Drop> = reshape.drop_tables.iter().filter(|t| has(t)).map(|t| Drop::Table(t.clone())).collect();
    for (table, column) in &reshape.drop_columns {
        if !has(table) || reshape.drop_tables.contains(table) {
            continue;
        }
        if columns(conn, table)?.iter().any(|c| c.eq_ignore_ascii_case(column)) {
            out.push(Drop::Column(table.clone(), column.clone()));
        }
    }
    Ok(out)
}

/// Drops a column with its values. Indexes on it go first (SQLite won't drop an indexed column); a column SQLite
/// won't drop at all (a foreign key, a UNIQUE constraint) goes with a rebuild of its table.
fn drop_column(conn: &Connection, table: &str, column: &str) -> anyhow::Result<()> {
    for index in indexes_on(conn, table, column)? {
        conn.execute_batch(&format!("DROP INDEX {}", quote(&index)))?;
    }
    if conn.execute_batch(&format!("ALTER TABLE {} DROP COLUMN {}", quote(table), quote(column))).is_ok() {
        return Ok(());
    }
    rebuild_without(conn, table, column)
}

/// The `CREATE INDEX` indexes of `table` that cover `column` (a UNIQUE or PRIMARY KEY one goes with a rebuild).
fn indexes_on(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<Vec<String>> {
    let mut list = conn.prepare(&format!("PRAGMA index_list({})", quote(table)))?;
    let indexes = list.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(3)?)))?;
    let mut out = Vec::new();
    for index in indexes {
        let (name, origin) = index?;
        if origin != "c" {
            continue;
        }
        let mut info = conn.prepare(&format!("PRAGMA index_info({})", quote(&name)))?;
        let covered = info.query_map([], |r| r.get::<_, Option<String>>(2))?;
        if covered.flatten().flatten().any(|c| c.eq_ignore_ascii_case(column)) {
            out.push(name);
        }
    }
    Ok(out)
}

/// SQLite's way to drop what `DROP COLUMN` can't: a copy of the table without the column (and without a table
/// constraint that names it), with its rows. Its indexes and triggers come back with the DDL that follows.
fn rebuild_without(conn: &Connection, table: &str, column: &str) -> anyhow::Result<()> {
    let sql: String =
        conn.query_row("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |r| r.get(0))?;
    let (open, close) = (sql.find('(').context("no columns")?, sql.rfind(')').context("no columns")?);
    let kept: Vec<String> = split_top_level(&sql[open + 1..close])
        .into_iter()
        .filter(|def| match column_name(def) {
            Some(name) => !name.eq_ignore_ascii_case(column),
            None => !names(def, column),
        })
        .collect();
    let rebuilt = format!("{table}__enee_rebuild");
    conn.execute_batch(&format!("CREATE TABLE {} ({})", quote(&rebuilt), kept.join(", ")))?;
    let list = columns(conn, &rebuilt)?.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
    conn.execute_batch(&format!("INSERT INTO {} ({list}) SELECT {list} FROM {}", quote(&rebuilt), quote(table)))?;
    conn.execute_batch(&format!("DROP TABLE {}", quote(table)))?;
    conn.execute_batch(&format!("ALTER TABLE {} RENAME TO {}", quote(&rebuilt), quote(table)))?;
    Ok(())
}

/// A table constraint names `column` (`UNIQUE (habit_id, day)`), as a whole word.
fn names(def: &str, column: &str) -> bool {
    def.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|word| word.eq_ignore_ascii_case(column))
}

/// The migration snapshots, newest first (an erased database isn't one: its data was meant to go).
fn snapshots_newest_first(dir: &Path, skip: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("snapshots"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.as_path() != skip)
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            name.starts_with("app-") && name.ends_with(".db") && !name.ends_with("-erased.db")
        })
        .collect();
    files.sort();
    files.reverse();
    files
}

/// Brings back, from the newest snapshot that has them, what this migration re-created after an edit had deleted it:
/// a re-added column's values (by row `id`, for the rows that still exist) and a re-created table's rows. A row that
/// can't come back (a foreign key to a row that's gone) is skipped.
fn restore(conn: &Connection, dir: &Path, taken: &Path, added: &[NewColumn], created: &[String]) -> Vec<String> {
    let snapshots: Vec<(PathBuf, Connection)> = snapshots_newest_first(dir, taken)
        .into_iter()
        .filter_map(|path| {
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            Connection::open_with_flags(&path, flags).ok().map(|c| (path, c))
        })
        .collect();
    let has = |snapshot: &Connection, table: &str, column: Option<&str>| {
        columns(snapshot, table).is_ok_and(|cols| column.is_none_or(|c| cols.iter().any(|x| x.eq_ignore_ascii_case(c))))
            && tables(snapshot).is_ok_and(|t| t.contains(table))
    };
    let mut restored = Vec::new();
    let mut back = |what: String, copied: anyhow::Result<usize>| match copied {
        Ok(rows) => restored.push(format!("{what} ({rows})")),
        Err(e) => tracing::warn!(what, "couldn't bring deleted data back: {e:#}"),
    };
    for column in added {
        if let Some((_, old)) = snapshots.iter().find(|(_, s)| has(s, &column.table, Some(&column.column))) {
            back(format!("{}.{}", column.table, column.column), copy_column(conn, old, &column.table, &column.column));
        }
    }
    for table in created {
        if let Some((_, old)) = snapshots.iter().find(|(_, s)| has(s, table, None)) {
            back(table.clone(), copy_rows(conn, old, table));
        }
    }
    restored
}

/// Each row's old value of `column`, by `id`, for the rows that still exist. Triggers are paused, so `updated_at`
/// isn't touched.
fn copy_column(conn: &Connection, old: &Connection, table: &str, column: &str) -> anyhow::Result<usize> {
    let mut read = old.prepare(&format!("SELECT id, {} FROM {}", quote(column), quote(table)))?;
    let rows: Vec<(i64, Sql)> = read.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
    let copied = (|| {
        let tx = conn.unchecked_transaction()?;
        let mut write = tx.prepare(&format!("UPDATE {} SET {} = ?1 WHERE id = ?2", quote(table), quote(column)))?;
        let mut copied = 0;
        for (id, value) in rows {
            copied += write.execute(rusqlite::params![value, id]).unwrap_or(0);
        }
        drop(write);
        tx.commit()?;
        anyhow::Ok(copied)
    })();
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)?;
    copied
}

/// The old rows of a re-created table: the columns both versions have.
fn copy_rows(conn: &Connection, old: &Connection, table: &str) -> anyhow::Result<usize> {
    let now = columns(conn, table)?;
    let shared: Vec<String> = columns(old, table)?.into_iter().filter(|c| now.contains(c)).collect();
    anyhow::ensure!(!shared.is_empty(), "nothing in common");
    let list = shared.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
    let marks = (1..=shared.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
    let mut read = old.prepare(&format!("SELECT {list} FROM {}", quote(table)))?;
    let rows: Vec<Vec<Sql>> = read
        .query_map([], |r| (0..shared.len()).map(|i| r.get::<_, Sql>(i)).collect::<Result<Vec<_>, _>>())?
        .collect::<Result<_, _>>()?;
    let tx = conn.unchecked_transaction()?;
    let mut write = tx.prepare(&format!("INSERT OR IGNORE INTO {} ({list}) VALUES ({marks})", quote(table)))?;
    let mut copied = 0;
    for row in rows {
        copied += write.execute(rusqlite::params_from_iter(row)).unwrap_or(0);
    }
    drop(write);
    tx.commit()?;
    Ok(copied)
}

fn plan(conn: &Connection, ddl: &[String]) -> rusqlite::Result<Vec<NewColumn>> {
    let mut out = Vec::new();
    for stmt in ddl {
        let Some((table, body)) = create_table(stmt) else { continue };
        let existing = columns(conn, &table)?;
        if existing.is_empty() {
            continue; // a new table: the DDL creates it whole
        }
        for definition in split_top_level(body) {
            let Some(column) = column_name(&definition) else { continue };
            if !existing.iter().any(|c| c.eq_ignore_ascii_case(&column)) {
                out.push(NewColumn { table: table.clone(), column, definition });
            }
        }
    }
    Ok(out)
}

/// `CREATE TABLE IF NOT EXISTS <name> (<body>)` → (name, body).
fn create_table(stmt: &str) -> Option<(String, &str)> {
    let s = stmt.trim();
    const HEAD: &str = "CREATE TABLE IF NOT EXISTS ";
    if s.len() < HEAD.len() || !s[..HEAD.len()].eq_ignore_ascii_case(HEAD) {
        return None;
    }
    let rest = &s[HEAD.len()..];
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    (open < close).then(|| (unquote(rest[..open].trim()), &rest[open + 1..close]))
}

fn columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", quote(table)))?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    names.collect()
}

/// Splits on commas outside parentheses and quotes.
fn split_top_level(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut current = String::new();
    for c in body.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, ',') if depth == 0 => {
                parts.push(current.trim().to_owned());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }
    parts
}

/// Whitespace-separated tokens, keeping parenthesized groups and quoted strings whole.
fn tokens(def: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut current = String::new();
    for c in def.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, c) if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The column a definition names, or `None` for a table constraint.
fn column_name(def: &str) -> Option<String> {
    let first = tokens(def).into_iter().next()?;
    let keyword = first.to_ascii_uppercase();
    let constraint = matches!(keyword.as_str(), "PRIMARY" | "UNIQUE" | "CHECK" | "FOREIGN" | "CONSTRAINT");
    (!constraint).then(|| unquote(&first))
}

/// The definition without `NOT NULL`, `UNIQUE`, `PRIMARY KEY` and `DEFAULT …`, and the default's expression.
fn relax(def: &str) -> (String, Option<String>) {
    let tokens = tokens(def);
    let mut kept = Vec::new();
    let mut default = None;
    let mut i = 0;
    while i < tokens.len() {
        let upper = tokens[i].to_ascii_uppercase();
        let next = tokens.get(i + 1).map(|t| t.to_ascii_uppercase());
        match (upper.as_str(), next.as_deref()) {
            ("NOT", Some("NULL")) | ("PRIMARY", Some("KEY")) => i += 2,
            ("UNIQUE", _) => i += 1,
            ("DEFAULT", Some(_)) => {
                default = Some(tokens[i + 1].clone());
                i += 2;
            }
            _ => {
                kept.push(tokens[i].clone());
                i += 1;
            }
        }
    }
    (kept.join(" "), default)
}

/// Adds the column; returns whether it had to be relaxed.
fn add_column(conn: &Connection, column: &NewColumn) -> anyhow::Result<bool> {
    let table = quote(&column.table);
    if conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {}", column.definition)).is_ok() {
        return Ok(false);
    }
    let (relaxed, default) = relax(&column.definition);
    conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {relaxed}"))?;
    if let Some(expr) = default {
        let name = quote(&column.column);
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
        let filled = conn.execute_batch(&format!("UPDATE {table} SET {name} = {expr} WHERE {name} IS NULL"));
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)?;
        filled?;
    }
    Ok(true)
}

/// Copies the database (SQLite's online backup) into `snapshots/`, keeping the newest few.
fn snapshot(conn: &Connection, dir: &Path) -> anyhow::Result<PathBuf> {
    let snapshots = dir.join("snapshots");
    std::fs::create_dir_all(&snapshots)?;
    let path = snapshots.join(format!("app-{}.db", crate::unix_ms()));
    conn.backup(MAIN_DB, &path, None).context("snapshot before migrating")?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(&snapshots)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "db"))
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(KEEP_SNAPSHOTS);
    for old in &files[..excess] {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

pub(crate) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn unquote(name: &str) -> String {
    let n = name.trim();
    for (open, close) in [('"', '"'), ('`', '`'), ('[', ']')] {
        if n.len() >= 2 && n.starts_with(open) && n.ends_with(close) {
            return n[1..n.len() - 1].replace("\"\"", "\"");
        }
    }
    n.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_compiled_create_table_statements() {
        let ddl = "CREATE TABLE IF NOT EXISTS ticks (id INTEGER PRIMARY KEY, habit_id INTEGER NOT NULL REFERENCES \
                   habits(id) ON DELETE CASCADE, day TEXT DEFAULT (date('now','localtime')), note TEXT DEFAULT 'a, b')";
        let (table, body) = create_table(ddl).unwrap();
        assert_eq!(table, "ticks");
        let defs = split_top_level(body);
        assert_eq!(defs.len(), 4, "{defs:?}");
        assert_eq!(column_name(&defs[1]).as_deref(), Some("habit_id"));
        assert_eq!(column_name("UNIQUE (a, b)"), None);
        assert!(create_table("CREATE TRIGGER IF NOT EXISTS t_touch …").is_none());
    }

    #[test]
    fn relaxing_drops_what_add_column_refuses() {
        let (def, default) = relax("created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))");
        assert_eq!(def, "created_at TEXT");
        assert_eq!(default.as_deref(), Some("(strftime('%Y-%m-%dT%H:%M:%fZ','now'))"));
        let (def, default) = relax("habit_id INTEGER NOT NULL REFERENCES habits(id) ON DELETE CASCADE");
        assert_eq!(def, "habit_id INTEGER REFERENCES habits(id) ON DELETE CASCADE");
        assert_eq!(default, None);
    }
}
