use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::*;

/// A fresh directory under the system temp dir (removed by the test).
fn temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("enee-data-{name}-{}-{}", std::process::id(), unix_ms()))
}

fn params(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

const NOW: &str = "(strftime('%Y-%m-%dT%H:%M:%fZ','now'))";

/// Shaped like the compiler's output for a habits app with one setting.
fn v1() -> Vec<String> {
    vec![
        format!(
            "CREATE TABLE IF NOT EXISTS habits (id INTEGER PRIMARY KEY, name TEXT NOT NULL, \
             created_at TEXT NOT NULL DEFAULT {NOW}, updated_at TEXT NOT NULL DEFAULT {NOW})"
        ),
        "CREATE TRIGGER IF NOT EXISTS habits_touch AFTER UPDATE ON habits FOR EACH ROW WHEN NEW.updated_at IS \
         OLD.updated_at BEGIN UPDATE habits SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = NEW.id; END"
            .into(),
        "CREATE TABLE IF NOT EXISTS ticks (id INTEGER PRIMARY KEY, habit_id INTEGER NOT NULL REFERENCES habits(id) \
         ON DELETE CASCADE, day TEXT DEFAULT (date('now','localtime')))"
            .into(),
        "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), goal INTEGER NOT NULL DEFAULT 8)"
            .into(),
        "INSERT OR IGNORE INTO settings (id) VALUES (1)".into(),
    ]
}

fn open(dir: &Path, ddl: &[String]) -> Opened {
    AppDb::open(dir, ddl, Clock::system(), Timeouts::default()).expect("open")
}

#[test]
fn data_persists_and_writes_report_their_tables() {
    let dir = temp_dir("persist");
    {
        let Opened { db, migration } = open(&dir, &v1());
        assert!(migration.is_none(), "a new database needs no migration");
        let added = db.exec("INSERT INTO habits (name) VALUES (:name)", &params(json!({ "name": "Read" }))).unwrap();
        assert_eq!(added, Exec { changed_tables: vec!["habits".into()], rows_affected: 1 });
        db.exec("INSERT INTO ticks (habit_id) VALUES (:habit)", &params(json!({ "habit": 1, "unused": 5 }))).unwrap();
        let none = db.exec("UPDATE habits SET name = 'x' WHERE id = 99", &Map::new()).unwrap();
        assert_eq!(none, Exec::default(), "nothing changed, nothing to refresh");
        assert_eq!(db.settings().unwrap(), params(json!({ "goal": 8 })));
    }
    // Reopened: the rows are still there.
    let Opened { db, .. } = open(&dir, &v1());
    let rows = db.query("SELECT name FROM habits", &Map::new()).unwrap();
    assert_eq!(rows, vec![params(json!({ "name": "Read" }))]);
    assert_eq!(db.query("SELECT count(*) AS n FROM ticks", &Map::new()).unwrap()[0]["n"], json!(1));

    // Deleting a habit cascades to its ticks, and both tables are reported.
    let deleted = db.exec("DELETE FROM habits WHERE id = :id", &params(json!({ "id": 1 }))).unwrap();
    assert_eq!(deleted.rows_affected, 1);
    assert!(deleted.changed_tables.contains(&"ticks".to_owned()), "{deleted:?}");
    assert_eq!(db.query("SELECT count(*) AS n FROM ticks", &Map::new()).unwrap()[0]["n"], json!(0));
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn app_sql_is_hardened() {
    let dir = temp_dir("harden");
    let Opened { db, .. } = open(&dir, &v1());
    for sql in [
        "DROP TABLE habits",
        "CREATE TABLE extra (x)",
        "PRAGMA foreign_keys = OFF",
        "ATTACH DATABASE 'other.db' AS other",
        "BEGIN",
    ] {
        let err = db.exec(sql, &Map::new()).expect_err(sql).to_string();
        assert!(err.contains("not allowed"), "{sql}: {err}");
    }
    // Queries run on the read-only connection.
    assert!(db.query("DELETE FROM habits", &Map::new()).is_err());
    // The contract's helpers are there.
    let today = db.query("SELECT is_today(now_utc()) AS t, today() = date('now','localtime') AS same", &Map::new());
    assert_eq!(today.unwrap(), vec![params(json!({ "t": 1, "same": 1 }))]);
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn long_statements_are_stopped() {
    let dir = temp_dir("timeout");
    let quick = Timeouts { query: Duration::from_millis(50), step: Duration::from_millis(50) };
    let Opened { db, .. } = AppDb::open(&dir, &v1(), Clock::system(), quick).unwrap();
    let forever = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";
    let err = db.query(forever, &Map::new()).expect_err("interrupted").to_string();
    assert!(err.contains("longer than 50 ms"), "{err}");
    // The connection is still usable afterwards.
    assert!(db.query("SELECT 1 AS one", &Map::new()).is_ok());
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_columns_are_added_after_a_snapshot() {
    let dir = temp_dir("migrate");
    let before = {
        let Opened { db, .. } = open(&dir, &v1());
        db.exec("INSERT INTO habits (name) VALUES ('Read')", &Map::new()).unwrap();
        db.query("SELECT updated_at FROM habits", &Map::new()).unwrap()[0]["updated_at"].clone()
    };

    let mut v2 = v1();
    v2[0] = format!(
        "CREATE TABLE IF NOT EXISTS habits (id INTEGER PRIMARY KEY, name TEXT NOT NULL, archived INTEGER NOT NULL \
         DEFAULT 0, seen_at TEXT NOT NULL DEFAULT {NOW}, created_at TEXT NOT NULL DEFAULT {NOW}, updated_at TEXT NOT \
         NULL DEFAULT {NOW})"
    );
    v2[3] = "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), goal INTEGER NOT NULL \
             DEFAULT 8, reminder TEXT DEFAULT '09:00')"
        .into();
    v2.push("CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, body TEXT)".into());
    v2.push("CREATE INDEX IF NOT EXISTS habits_archived_idx ON habits(archived)".into());

    let Opened { db, migration } = open(&dir, &v2);
    let m = migration.expect("a migration");
    assert_eq!(m.added, ["habits.archived", "habits.seen_at", "settings.reminder"]);
    assert_eq!(m.relaxed, ["habits.seen_at"], "SQLite can't add a column with a non-constant default");
    assert!(m.snapshot.exists());

    let row = db.query("SELECT archived, seen_at, updated_at FROM habits", &Map::new()).unwrap().remove(0);
    assert_eq!(row["archived"], json!(0));
    assert!(row["seen_at"].is_string(), "existing rows get the default's value");
    assert_eq!(row["updated_at"], before, "the backfill doesn't count as an update");
    assert_eq!(db.settings().unwrap(), params(json!({ "goal": 8, "reminder": "09:00" })));
    assert!(db.exec("INSERT INTO notes (body) VALUES ('hi')", &Map::new()).is_ok());
    drop(db);

    let Opened { migration, .. } = open(&dir, &v2);
    assert!(migration.is_none(), "the same schema again needs nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

fn open_reshaping(dir: &Path, ddl: &[String], reshape: &Reshape) -> Opened {
    AppDb::open_reshaping(dir, ddl, Clock::system(), Timeouts::default(), reshape).expect("open")
}

/// v1 with `habits.note` (indexed), and a `tags` table.
fn with_notes() -> Vec<String> {
    let mut ddl = v1();
    ddl[0] = format!(
        "CREATE TABLE IF NOT EXISTS habits (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, \
         created_at TEXT NOT NULL DEFAULT {NOW}, updated_at TEXT NOT NULL DEFAULT {NOW})"
    );
    ddl.push("CREATE INDEX IF NOT EXISTS habits_note_idx ON habits(note)".into());
    ddl.push("CREATE TABLE IF NOT EXISTS tags (id INTEGER PRIMARY KEY, label TEXT)".into());
    ddl
}

#[test]
fn an_edit_deletes_what_it_removed_and_an_undo_brings_it_back() {
    let dir = temp_dir("reshape");
    {
        let Opened { db, .. } = open(&dir, &with_notes());
        db.exec("INSERT INTO habits (name, note) VALUES ('Read', 'ten pages'), ('Run', NULL)", &Map::new()).unwrap();
        db.exec("INSERT INTO tags (label) VALUES ('health')", &Map::new()).unwrap();
        db.exec("INSERT INTO ticks (habit_id) VALUES (1)", &Map::new()).unwrap();
    }

    // Without a reshape, nothing goes: v1 lacks `note` and `tags`, and they stay (as after an undo, ADR-0021).
    let Opened { db, migration } = open(&dir, &v1());
    assert!(migration.is_none());
    assert_eq!(db.query("SELECT note FROM habits WHERE id = 1", &Map::new()).unwrap()[0]["note"], json!("ten pages"));
    drop(db);

    // An edit removed them: they're deleted, after a snapshot.
    let reshape = Reshape {
        drop_tables: vec!["tags".into()],
        drop_columns: vec![("habits".into(), "note".into()), ("habits".into(), "gone".into())],
        restore: false,
    };
    let Opened { db, migration } = open_reshaping(&dir, &v1(), &reshape);
    let m = migration.expect("a migration");
    assert_eq!(m.dropped, ["tags", "habits.note"], "only what's there");
    assert!(m.snapshot.exists());
    assert!(db.query("SELECT note FROM habits", &Map::new()).is_err(), "the column is gone");
    assert!(db.query("SELECT * FROM tags", &Map::new()).is_err(), "the table is gone");
    assert_eq!(db.query("SELECT count(*) AS n FROM ticks", &Map::new()).unwrap()[0]["n"], json!(1), "rows stay");
    db.exec("INSERT INTO habits (name) VALUES ('Swim')", &Map::new()).unwrap();
    drop(db);

    // Undo: the version with them comes back, and so do their values.
    let Opened { db, migration } =
        open_reshaping(&dir, &with_notes(), &Reshape { restore: true, ..Reshape::default() });
    let m = migration.expect("a migration");
    assert_eq!(m.added, ["habits.note"]);
    assert_eq!(m.restored, ["habits.note (2)", "tags (1)"]);
    let notes = db.query("SELECT name, note FROM habits ORDER BY id", &Map::new()).unwrap();
    let notes: Vec<(Value, Value)> = notes.into_iter().map(|r| (r["name"].clone(), r["note"].clone())).collect();
    assert_eq!(
        notes,
        [(json!("Read"), json!("ten pages")), (json!("Run"), Value::Null), (json!("Swim"), Value::Null)],
        "rows written since keep theirs"
    );
    assert_eq!(db.query("SELECT label FROM tags", &Map::new()).unwrap()[0]["label"], json!("health"));
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_column_sqlite_wont_drop_goes_with_a_rebuild() {
    let dir = temp_dir("rebuild");
    {
        let Opened { db, .. } = open(&dir, &v1());
        db.exec("INSERT INTO habits (name) VALUES ('Read'), ('Run')", &Map::new()).unwrap();
        db.exec("INSERT INTO ticks (habit_id, day) VALUES (1, '2026-10-06'), (2, '2026-10-07')", &Map::new()).unwrap();
    }
    // `ticks.habit_id` references `habits`: DROP COLUMN refuses it.
    let mut v2 = v1();
    v2[2] =
        "CREATE TABLE IF NOT EXISTS ticks (id INTEGER PRIMARY KEY, day TEXT DEFAULT (date('now','localtime')))".into();
    let reshape = Reshape { drop_columns: vec![("ticks".into(), "habit_id".into())], ..Reshape::default() };
    let Opened { db, migration } = open_reshaping(&dir, &v2, &reshape);
    assert_eq!(migration.expect("a migration").dropped, ["ticks.habit_id"]);
    let days = db.query("SELECT id, day FROM ticks ORDER BY id", &Map::new()).unwrap();
    assert_eq!(days.len(), 2, "every row is kept");
    assert_eq!(days[1]["day"], json!("2026-10-07"));
    assert!(db.exec("DELETE FROM habits WHERE id = 1", &Map::new()).is_ok(), "foreign keys are back on and hold");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn erase_sets_the_database_aside() {
    let dir = temp_dir("erase");
    let Opened { db, .. } = open(&dir, &v1());
    db.exec("INSERT INTO habits (name) VALUES ('Read')", &Map::new()).unwrap();
    db.erase().unwrap();
    assert!(!dir.join("app.db").exists());
    let kept = std::fs::read_dir(dir.join("snapshots")).unwrap().count();
    assert_eq!(kept, 1, "the erased database is kept as a snapshot");
    let Opened { db, .. } = open(&dir, &v1());
    assert!(db.query("SELECT * FROM habits", &Map::new()).unwrap().is_empty());
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every row of `table`, ordered by id, as JSON.
fn table(db: &AppDb, table: &str) -> Value {
    Value::Array(
        db.query(&format!("SELECT * FROM {table} ORDER BY id"), &Map::new())
            .unwrap()
            .into_iter()
            .map(Value::Object)
            .collect(),
    )
}

#[test]
fn a_recorded_run_is_put_back() {
    let dir = temp_dir("revert");
    let Opened { db, .. } = open(&dir, &v1());
    db.exec("INSERT INTO habits (name) VALUES ('Read'), ('Run')", &Map::new()).unwrap();
    db.exec("INSERT INTO ticks (habit_id, day) VALUES (1, '2026-10-01'), (2, '2026-10-01')", &Map::new()).unwrap();
    let (habits, ticks) = (table(&db, "habits"), table(&db, "ticks"));

    // An insert, a rename (which fires the touch trigger), a deleted habit (which cascades to its tick) and a setting.
    let mut run = db.record();
    run.exec("INSERT INTO habits (name) VALUES (:name)", &params(json!({ "name": "Floss" }))).unwrap();
    run.exec("UPDATE habits SET name = 'Reading' WHERE id = 1", &Map::new()).unwrap();
    let deleted = run.exec("DELETE FROM habits WHERE id = 2", &Map::new()).unwrap();
    assert!(deleted.changed_tables.contains(&"ticks".to_owned()));
    run.exec("UPDATE settings SET goal = 3", &Map::new()).unwrap();
    // The new row changes again: its first change (the insert) decides what it was.
    run.exec("UPDATE habits SET name = 'Flossing' WHERE id = 3", &Map::new()).unwrap();
    let changes = run.finish();
    assert!(changes.revertible());
    assert_eq!(changes.tables(), ["habits", "settings", "ticks"]);
    assert_ne!(table(&db, "habits"), habits);

    let undone = db.revert(&changes).unwrap();
    assert!(
        undone.changed_tables.contains(&"habits".to_owned()) && undone.changed_tables.contains(&"ticks".to_owned())
    );
    assert_eq!(table(&db, "habits"), habits, "same rows, ids and timestamps");
    assert_eq!(table(&db, "ticks"), ticks, "the cascaded tick is back too");
    assert_eq!(db.settings().unwrap(), params(json!({ "goal": 8 })));

    // The host's own transaction is over: app SQL still may not use one.
    assert!(db.exec("BEGIN", &Map::new()).unwrap_err().to_string().contains("not allowed"));
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runs_that_cant_be_put_back_say_so() {
    let dir = temp_dir("revert-replace");
    let Opened { db, .. } = open(&dir, &v1());
    db.exec("INSERT INTO habits (name) VALUES ('Read')", &Map::new()).unwrap();
    let mut run = db.record();
    run.exec("INSERT OR REPLACE INTO habits (id, name) VALUES (1, 'Write')", &Map::new()).unwrap();
    let changes = run.finish();
    assert!(!changes.revertible());
    assert!(db.revert(&changes).is_err());
    assert_eq!(db.query("SELECT name FROM habits", &Map::new()).unwrap()[0]["name"], json!("Write"));

    // A run that changed nothing has nothing to put back.
    let mut run = db.record();
    run.exec("UPDATE habits SET name = 'x' WHERE id = 99", &Map::new()).unwrap();
    assert!(run.finish().is_empty());
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}
