//! Ported from eneeware `crates/enee-data` at 61bc7a7 (2026-10-07), now owned by r1os (ADR-0005).
//! Per-app SQLite for enee apps (ADR-0005, docs/specs/enee-apps.md): one database per app, owned by the core.
//!
//! - **Two hardened connections:** a writer for the schema and the app's steps, and a read-only reader for its
//!   queries. Once the schema is in place, an authorizer allows only reading and writing the app's own tables: no
//!   schema changes, PRAGMA, ATTACH or transactions. `SQLITE_DBCONFIG_DEFENSIVE` is on, attached databases are
//!   limited to 0, and a progress handler stops a statement that runs past its deadline.
//! - **The contract's SQL helpers** (`is_today`, `streak`, … from `enee_spec::sql`) in the user's timezone, and
//!   foreign keys on, so the compiled `ON DELETE CASCADE` works.
//! - **Migrations** ([`migrate`]): the app's DDL is idempotent, and columns a newer spec adds to an existing table are
//!   added after a snapshot of the database. What an edit removed is deleted then too, and an undo brings it back from
//!   a snapshot (ADR-0023).
//! - **Changed tables** for query refresh: from the authorizer at prepare time and SQLite's update hook while the
//!   statement runs, so rows removed by `ON DELETE CASCADE` count too.
//! - **Recorded runs** ([`undo`]): the rows a run of steps changes, kept so they can be put back.

pub mod migrate;
pub mod undo;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use enee_spec::sql::{Clock, register_helpers};
use rusqlite::config::DbConfig;
use rusqlite::hooks::{Action, AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OpenFlags, Statement};
use serde_json::{Map, Value, json};

pub use migrate::{Migration, Reshape};
pub use undo::{Changes, Recording};

/// How long one statement may run.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub query: Duration,
    pub step: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts { query: Duration::from_secs(2), step: Duration::from_secs(5) }
    }
}

/// What a step changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exec {
    /// Tables whose rows changed, cascades included (for refreshing the queries that read them).
    pub changed_tables: Vec<String>,
    pub rows_affected: u64,
}

/// One app's database: `<dir>/app.db`, plus `<dir>/snapshots/`.
pub struct AppDb {
    dir: PathBuf,
    writer: Mutex<Guarded>,
    reader: Mutex<Guarded>,
    has_settings: bool,
    timeouts: Timeouts,
}

/// The result of opening: the database, and the migration it needed (if any).
pub struct Opened {
    pub db: AppDb,
    pub migration: Option<Migration>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppDb {
    /// Opens (or creates) `<dir>/app.db` and brings its schema up to `ddl`, the compiled app's DDL. Nothing is
    /// deleted: a table or column the DDL doesn't have stays.
    pub fn open(dir: &Path, ddl: &[String], clock: Clock, timeouts: Timeouts) -> anyhow::Result<Opened> {
        Self::open_reshaping(dir, ddl, clock, timeouts, &Reshape::default())
    }

    /// [`open`](Self::open), deleting what an edit removed, or bringing back what an undo's version has again
    /// ([`Reshape`], ADR-0023).
    pub fn open_reshaping(
        dir: &Path,
        ddl: &[String],
        clock: Clock,
        timeouts: Timeouts,
        reshape: &Reshape,
    ) -> anyhow::Result<Opened> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join("app.db");
        let writer = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening {}", path.display()))?;
        configure(&writer, &clock)?;
        let mode: String = writer.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        anyhow::ensure!(mode.eq_ignore_ascii_case("wal"), "WAL mode unavailable ({mode})");
        writer.pragma_update(None, "synchronous", "NORMAL")?;
        writer.pragma_update(None, "foreign_keys", true)?;
        let migration = migrate::run(&writer, dir, ddl, reshape).context("updating the app's tables")?;
        let has_settings = writer
            .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'settings'", [], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n > 0)?;
        let writer = Guarded::new(writer, Access::ReadWrite)?;

        let reader =
            Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .with_context(|| format!("opening {} read-only", path.display()))?;
        configure(&reader, &clock)?;
        let reader = Guarded::new(reader, Access::ReadOnly)?;
        let db = AppDb {
            dir: dir.to_owned(),
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
            has_settings,
            timeouts,
        };
        Ok(Opened { db, migration })
    }

    /// Runs a (compiled) query on the read-only connection; rows come back as JSON objects.
    pub fn query(&self, sql: &str, params: &Map<String, Value>) -> anyhow::Result<Vec<Map<String, Value>>> {
        let conn = lock(&self.reader);
        conn.run(self.timeouts.query, |db| {
            let mut stmt = db.prepare(sql)?;
            bind(&mut stmt, params)?;
            rows(&mut stmt)
        })
        .map(|(rows, _)| rows)
    }

    /// Runs a (compiled) step on the writer.
    pub fn exec(&self, sql: &str, params: &Map<String, Value>) -> anyhow::Result<Exec> {
        let conn = lock(&self.writer);
        let (rows_affected, written) = conn.run(self.timeouts.step, |db| {
            let mut stmt = db.prepare(sql)?;
            bind(&mut stmt, params)?;
            // A step that returns rows (RETURNING) still runs to the end; its rows are dropped.
            if stmt.column_count() > 0 {
                rows(&mut stmt)?;
                Ok(db.changes())
            } else {
                Ok(stmt.raw_execute()? as u64)
            }
        })?;
        let changed_tables = if rows_affected > 0 || !written.touched.is_empty() {
            written.planned.union(&written.touched).cloned().collect()
        } else {
            Vec::new()
        };
        Ok(Exec { changed_tables, rows_affected })
    }

    /// The single settings row (`/settings/*`), without `id` and unset values. Empty for apps without settings.
    pub fn settings(&self) -> anyhow::Result<Map<String, Value>> {
        if !self.has_settings {
            return Ok(Map::new());
        }
        let mut row =
            self.query("SELECT * FROM settings WHERE id = 1", &Map::new())?.into_iter().next().unwrap_or_default();
        row.remove("id");
        row.retain(|_, v| !v.is_null());
        Ok(row)
    }

    /// Closes the database and moves it into `snapshots/` (kept as a safety net), so the next open starts empty.
    pub fn erase(self) -> anyhow::Result<()> {
        let dir = self.dir.clone();
        drop(self); // closing the last connection checkpoints and removes the WAL
        erase_files(&dir)
    }
}

/// [`AppDb::erase`] for a database that isn't open.
pub fn erase_files(dir: &Path) -> anyhow::Result<()> {
    let db = dir.join("app.db");
    if db.exists() {
        let snapshots = dir.join("snapshots");
        std::fs::create_dir_all(&snapshots)?;
        std::fs::rename(&db, snapshots.join(format!("app-{}-erased.db", unix_ms())))?;
    }
    for leftover in ["app.db-wal", "app.db-shm"] {
        let _ = std::fs::remove_file(dir.join(leftover));
    }
    Ok(())
}

pub(crate) fn unix_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

/// Settings shared by both connections. Runs before the authorizer is installed.
fn configure(conn: &Connection, clock: &Clock) -> anyhow::Result<()> {
    conn.busy_timeout(Duration::from_secs(2))?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, 16 * 1024 * 1024)?;
    conn.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 1024 * 1024)?;
    conn.set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 256)?;
    register_helpers(conn, clock.clone())?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    ReadOnly,
    ReadWrite,
}

/// Tables a statement wrote.
#[derive(Default)]
struct Written {
    /// From the authorizer when the statement was prepared.
    planned: BTreeSet<String>,
    /// From the update hook while it ran (includes foreign-key cascades).
    touched: BTreeSet<String>,
    /// The authorizer saw an update or a delete: the statement may rewrite existing rows.
    rewrites: bool,
}

/// State shared with the connection's hooks.
struct Guard {
    epoch: Instant,
    /// Milliseconds since `epoch` after which the running statement is interrupted; `u64::MAX` when idle.
    deadline_ms: AtomicU64,
    written: Mutex<Written>,
    denied: Mutex<Option<String>>,
    /// While a run is recorded ([`undo`]): each row the update hook reports, with what happened to it.
    rows: Mutex<Option<Vec<(Action, String, i64)>>>,
    /// The host's own statements are running (putting a run back), which may use a transaction and pragmas.
    internal: AtomicBool,
}

impl Guard {
    fn expired(&self) -> bool {
        self.epoch.elapsed().as_millis() as u64 > self.deadline_ms.load(Ordering::Relaxed)
    }
}

/// A connection with the authorizer, update hook and progress handler installed.
struct Guarded {
    db: Connection,
    guard: Arc<Guard>,
}

impl Guarded {
    fn new(db: Connection, access: Access) -> anyhow::Result<Self> {
        let guard = Arc::new(Guard {
            epoch: Instant::now(),
            deadline_ms: AtomicU64::new(u64::MAX),
            written: Mutex::default(),
            denied: Mutex::default(),
            rows: Mutex::default(),
            internal: AtomicBool::new(false),
        });
        let auth = guard.clone();
        db.authorizer(Some(move |ctx: AuthContext<'_>| authorize(&auth, access, ctx)))?;
        let hook = guard.clone();
        db.update_hook(Some(move |action, _db: &str, table: &str, row: i64| {
            lock(&hook.written).touched.insert(table.to_owned());
            if let Some(rows) = lock(&hook.rows).as_mut() {
                rows.push((action, table.to_owned(), row));
            }
        }))?;
        let timer = guard.clone();
        db.progress_handler(1_000, Some(move || timer.expired()))?;
        Ok(Guarded { db, guard })
    }

    /// Runs one statement with a deadline; returns its result and the tables it wrote. A statement the authorizer
    /// refused reports why.
    fn run<T>(
        &self,
        limit: Duration,
        statement: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> anyhow::Result<(T, Written)> {
        *lock(&self.guard.written) = Written::default();
        *lock(&self.guard.denied) = None;
        let deadline = self.guard.epoch.elapsed() + limit;
        self.guard.deadline_ms.store(deadline.as_millis() as u64, Ordering::Relaxed);
        let result = statement(&self.db);
        self.guard.deadline_ms.store(u64::MAX, Ordering::Relaxed);
        let written = std::mem::take(&mut *lock(&self.guard.written));
        match result {
            Ok(value) => Ok((value, written)),
            Err(e) => {
                if let Some(what) = lock(&self.guard.denied).take() {
                    anyhow::bail!("not allowed in app SQL: {what}");
                }
                if matches!(&e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted)
                {
                    anyhow::bail!("the statement took longer than {} ms and was stopped", limit.as_millis());
                }
                Err(e.into())
            }
        }
    }
}

/// App SQL may read, and on the writer write, the app's own tables; everything else is refused.
fn authorize(guard: &Guard, access: Access, ctx: AuthContext<'_>) -> Authorization {
    if guard.internal.load(Ordering::Relaxed) {
        return Authorization::Allow;
    }
    match ctx.action {
        AuthAction::Read { .. } | AuthAction::Select | AuthAction::Function { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Insert { table_name }
        | AuthAction::Delete { table_name }
        | AuthAction::Update { table_name, .. }
            if access == Access::ReadWrite =>
        {
            if !table_name.starts_with("sqlite_") {
                let mut written = lock(&guard.written);
                written.planned.insert(table_name.to_owned());
                written.rewrites |= !matches!(ctx.action, AuthAction::Insert { .. });
            }
            Authorization::Allow
        }
        other => {
            *lock(&guard.denied) = Some(format!("{other:?}"));
            Authorization::Deny
        }
    }
}

/// Binds `:name` parameters the statement uses from `params`; the ones it doesn't use are ignored, and missing
/// ones are NULL (as SQLite leaves unbound parameters).
fn bind(stmt: &mut Statement<'_>, params: &Map<String, Value>) -> rusqlite::Result<()> {
    for i in 1..=stmt.parameter_count() {
        let Some(name) = stmt.parameter_name(i).and_then(|n| n.strip_prefix(':')).map(str::to_owned) else {
            continue;
        };
        if let Some(value) = params.get(&name) {
            stmt.raw_bind_parameter(i, to_sql(value))?;
        }
    }
    Ok(())
}

fn rows(stmt: &mut Statement<'_>) -> rusqlite::Result<Vec<Map<String, Value>>> {
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let mut rows = stmt.raw_query();
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut obj = Map::new();
        for (i, name) in names.iter().enumerate() {
            obj.insert(name.clone(), to_json(row.get::<_, Sql>(i)?));
        }
        out.push(obj);
    }
    Ok(out)
}

/// JSON → SQLite: booleans are 0/1, whole numbers integers; arrays and objects are stored as JSON text.
fn to_sql(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(|| Sql::Real(n.as_f64().unwrap_or_default()), Sql::Integer),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

/// SQLite → JSON. Blobs don't cross the bridge (null).
fn to_json(v: Sql) -> Value {
    match v {
        Sql::Null | Sql::Blob(_) => Value::Null,
        Sql::Integer(i) => json!(i),
        Sql::Real(f) => json!(f),
        Sql::Text(s) => Value::String(s),
    }
}

#[cfg(test)]
mod tests;
