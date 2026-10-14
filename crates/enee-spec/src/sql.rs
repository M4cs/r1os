//! SQL checks. With the `sqlite` feature every statement is prepared against a scratch in-memory database built
//! from the compiled DDL plus the host helpers; the authorizer reports which tables a statement reads and writes
//! and denies schema changes. Without it (the WASM preview build) only `:params` are extracted.

#[derive(Debug, Default)]
pub struct Info {
    /// False when no database was available to prepare against (columns/tables are then unknown).
    pub validated: bool,
    pub columns: Vec<String>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    /// Named params without the `:` prefix, in order of first use.
    pub params: Vec<String>,
    pub readonly: bool,
}

#[derive(Debug)]
pub enum Error {
    Empty,
    Syntax(String),
    Multiple,
    Positional(String),
    Denied(String),
}

/// `:name` params outside literals and comments, in order of first use.
pub fn named_params(sql: &str) -> Vec<String> {
    let b = sql.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        match b[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < b.len() {
                    if b[i] == q {
                        if b.get(i + 1) == Some(&q) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'[' => i = sql[i..].find(']').map_or(b.len(), |j| i + j + 1),
            b'-' if b.get(i + 1) == Some(&b'-') => i = sql[i..].find('\n').map_or(b.len(), |j| i + j + 1),
            b'/' if b.get(i + 1) == Some(&b'*') => i = sql[i + 2..].find("*/").map_or(b.len(), |j| i + j + 4),
            b':' if b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') => {
                let start = i + 1;
                i = start;
                while i < b.len() && ident(b[i]) {
                    i += 1;
                }
                let name = &sql[start..i];
                if !out.iter().any(|p| p == name) {
                    out.push(name.to_string());
                }
            }
            _ => i += 1,
        }
    }
    out
}

#[cfg(not(feature = "sqlite"))]
pub struct Checker;

#[cfg(not(feature = "sqlite"))]
impl Checker {
    pub fn new(_ddl: &[String]) -> Result<Self, String> {
        Ok(Checker)
    }

    pub fn check(&self, sql: &str) -> Result<Info, Error> {
        if sql.trim().is_empty() {
            return Err(Error::Empty);
        }
        let first = sql.trim_start().split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or_default();
        let readonly = matches!(first.to_ascii_uppercase().as_str(), "SELECT" | "VALUES" | "WITH");
        Ok(Info { validated: false, params: named_params(sql), readonly, ..Info::default() })
    }
}

#[cfg(feature = "sqlite")]
pub use native::{Checker, Clock, register_helpers};

#[cfg(feature = "sqlite")]
mod native {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;
    use rusqlite::functions::{Aggregate, Context, FunctionFlags};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

    use super::{Error, Info};

    #[derive(Default)]
    struct Log {
        reads: Vec<String>,
        writes: Vec<String>,
        denied: Option<String>,
    }

    pub struct Checker {
        conn: Connection,
        log: Arc<Mutex<Log>>,
    }

    impl Checker {
        /// Builds the scratch database. An error here means the compiler emitted bad DDL.
        pub fn new(ddl: &[String]) -> Result<Self, String> {
            let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
            register_helpers(&conn, Clock::utc()).map_err(|e| e.to_string())?;
            for stmt in ddl {
                conn.execute_batch(stmt).map_err(|e| format!("{e} in `{stmt}`"))?;
            }
            let log = Arc::new(Mutex::new(Log::default()));
            let hook_log = log.clone();
            conn.authorizer(Some(move |ctx: AuthContext<'_>| authorize(&hook_log, ctx))).map_err(|e| e.to_string())?;
            Ok(Checker { conn, log })
        }

        pub fn check(&self, sql: &str) -> Result<Info, Error> {
            if sql.trim().is_empty() {
                return Err(Error::Empty);
            }
            *self.log.lock().expect("log") = Log::default();
            let stmt = self.conn.prepare(sql).map_err(|e| {
                if let Some(denied) = self.log.lock().expect("log").denied.take() {
                    return Error::Denied(denied);
                }
                match e {
                    rusqlite::Error::MultipleStatement => Error::Multiple,
                    rusqlite::Error::SqliteFailure(_, Some(msg)) => Error::Syntax(msg),
                    other => Error::Syntax(other.to_string()),
                }
            })?;
            let mut params = Vec::new();
            for i in 1..=stmt.parameter_count() {
                match stmt.parameter_name(i) {
                    Some(name) if name.starts_with(':') => params.push(name[1..].to_string()),
                    Some(name) => return Err(Error::Positional(name.to_string())),
                    None => return Err(Error::Positional("?".into())),
                }
            }
            let log = std::mem::take(&mut *self.log.lock().expect("log"));
            Ok(Info {
                validated: true,
                columns: stmt.column_names().into_iter().map(str::to_string).collect(),
                reads: log.reads,
                writes: log.writes,
                params,
                readonly: stmt.readonly(),
            })
        }
    }

    fn authorize(log: &Mutex<Log>, ctx: AuthContext<'_>) -> Authorization {
        let mut log = log.lock().expect("log");
        let push = |list: &mut Vec<String>, t: &str| {
            if !t.starts_with("sqlite_") && !list.iter().any(|x| x == t) {
                list.push(t.to_string());
            }
        };
        match ctx.action {
            AuthAction::Read { table_name, .. } => push(&mut log.reads, table_name),
            AuthAction::Insert { table_name }
            | AuthAction::Delete { table_name }
            | AuthAction::Update { table_name, .. } => {
                // Writes made by the `<table>_touch` trigger are an implementation detail.
                if ctx.accessor.is_none() {
                    push(&mut log.writes, table_name);
                }
            }
            AuthAction::Select | AuthAction::Function { .. } | AuthAction::Recursive => {}
            other => {
                log.denied = Some(format!("{other:?}"));
                return Authorization::Deny;
            }
        }
        Authorization::Allow
    }

    const DAY_MS: i64 = 86_400_000;
    const HOUR_MS: i64 = 3_600_000;

    type NowFn = dyn Fn() -> i64 + Send + Sync;
    type OffsetFn = dyn Fn(i64) -> i64 + Send + Sync;
    /// A helper over the local day number of its argument.
    type DayFn = fn(&Clock, i64) -> i64;

    /// Wall clock and the local UTC offset at a given instant.
    #[derive(Clone)]
    pub struct Clock {
        now_ms: Arc<NowFn>,
        offset_ms: Arc<OffsetFn>,
    }

    impl Clock {
        pub fn new(
            now_ms: impl Fn() -> i64 + Send + Sync + 'static,
            offset_ms: impl Fn(i64) -> i64 + Send + Sync + 'static,
        ) -> Self {
            Clock { now_ms: Arc::new(now_ms), offset_ms: Arc::new(offset_ms) }
        }

        pub fn utc() -> Self {
            Clock::new(system_now_ms, |_| 0)
        }

        /// The OS timezone, through SQLite's `localtime` modifier (DST-correct per instant). This is the zone
        /// `DEFAULT (date('now','localtime'))` uses, so checks see one consistent "today".
        pub fn system() -> Self {
            let conn = Mutex::new(Connection::open_in_memory().expect("in-memory database"));
            Clock::new(system_now_ms, move |ms| {
                let conn = conn.lock().expect("clock connection");
                conn.query_row(
                    "SELECT CAST(round((julianday(?1, 'unixepoch', 'localtime') - julianday(?1, 'unixepoch')) * 86400000) AS INTEGER)",
                    [ms as f64 / 1000.0],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            })
        }

        fn now(&self) -> i64 {
            (self.now_ms)()
        }

        /// Now, in ms since the Unix epoch.
        pub fn now_ms(&self) -> i64 {
            self.now()
        }

        /// The local UTC offset at `ms`, in ms (for `eval::Format` and the scheduler's local times).
        pub fn offset_ms(&self, ms: i64) -> i64 {
            (self.offset_ms)(ms)
        }

        fn local(&self, ms: i64) -> i64 {
            ms + (self.offset_ms)(ms)
        }

        fn local_day(&self, ts: &str) -> Option<i64> {
            if ts.len() == 10 {
                return parse_date(ts);
            }
            Some(self.local(parse_ts(ts)?).div_euclid(DAY_MS))
        }

        fn today(&self) -> i64 {
            self.local(self.now()).div_euclid(DAY_MS)
        }
    }

    fn system_now_ms() -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default()
    }

    /// The host SQL helpers from docs/specs/compiled-app.md §6.
    pub fn register_helpers(conn: &Connection, clock: Clock) -> rusqlite::Result<()> {
        let f = FunctionFlags::SQLITE_UTF8;
        // One-argument helpers over the local day of a timestamp or date.
        let day_fns: [(&str, DayFn); 4] = [
            ("is_today", |c, d| i64::from(d == c.today())),
            ("is_this_week", |c, d| i64::from(week_start(d) == week_start(c.today()))),
            ("is_this_month", |c, d| {
                let ((y, m, _), (ty, tm, _)) = (civil(d), civil(c.today()));
                i64::from(y == ty && m == tm)
            }),
            ("local_weekday", |_, d| (d + 4).rem_euclid(7)),
        ];
        for (name, g) in day_fns {
            let c = clock.clone();
            conn.create_scalar_function(name, 1, f, move |ctx| {
                Ok(ctx.get::<Option<String>>(0)?.and_then(|ts| c.local_day(&ts)).map(|d| g(&c, d)))
            })?;
        }
        let c = clock.clone();
        conn.create_scalar_function("local_date", 1, f, move |ctx| {
            Ok(ctx.get::<Option<String>>(0)?.and_then(|ts| c.local_day(&ts)).map(format_date))
        })?;
        let c = clock.clone();
        conn.create_scalar_function("local_hour", 1, f, move |ctx| {
            Ok(ctx
                .get::<Option<String>>(0)?
                .and_then(|ts| parse_ts(&ts))
                .map(|ms| c.local(ms).rem_euclid(DAY_MS) / HOUR_MS))
        })?;
        for (name, unit) in [("days_ago", DAY_MS), ("hours_ago", HOUR_MS)] {
            let c = clock.clone();
            conn.create_scalar_function(name, 1, f, move |ctx| {
                let n: f64 = ctx.get(0)?;
                Ok(format_ts(c.now() - (n * unit as f64) as i64))
            })?;
        }
        // `now()` is the spelling models reach for; it is the same as `now_utc()`.
        for name in ["now_utc", "now"] {
            let c = clock.clone();
            conn.create_scalar_function(name, 0, f, move |_| Ok(format_ts(c.now())))?;
        }
        let c = clock.clone();
        conn.create_scalar_function("today", 0, f, move |_| Ok(format_date(c.today())))?;
        conn.create_aggregate_function("streak", 1, f, Streak(clock))?;
        Ok(())
    }

    /// `streak(date_or_ts)`: consecutive local days with at least one row, ending today (or yesterday when
    /// today has none yet).
    struct Streak(Clock);

    impl Aggregate<BTreeSet<i64>, i64> for Streak {
        fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<BTreeSet<i64>> {
            Ok(BTreeSet::new())
        }

        fn step(&self, ctx: &mut Context<'_>, days: &mut BTreeSet<i64>) -> rusqlite::Result<()> {
            if let Some(day) = ctx.get::<Option<String>>(0)?.and_then(|ts| self.0.local_day(&ts)) {
                days.insert(day);
            }
            Ok(())
        }

        fn finalize(&self, _: &mut Context<'_>, days: Option<BTreeSet<i64>>) -> rusqlite::Result<i64> {
            let days = days.unwrap_or_default();
            let today = self.0.today();
            let mut day = if days.contains(&today) { today } else { today - 1 };
            let mut n = 0;
            while days.contains(&day) {
                n += 1;
                day -= 1;
            }
            Ok(n)
        }
    }

    /// Monday-based week.
    fn week_start(day: i64) -> i64 {
        day - (day + 3).rem_euclid(7)
    }

    /// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
    fn civil(days: i64) -> (i64, i64, i64) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + i64::from(m <= 2), m, d)
    }

    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn num(s: &str, range: std::ops::Range<usize>) -> Option<i64> {
        s.get(range).filter(|p| p.bytes().all(|b| b.is_ascii_digit())).and_then(|p| p.parse().ok())
    }

    fn parse_date(s: &str) -> Option<i64> {
        let b = s.as_bytes();
        if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let (y, m, d) = (num(s, 0..4)?, num(s, 5..7)?, num(s, 8..10)?);
        ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
    }

    /// `YYYY-MM-DD[T| ]HH:MM[:SS[.fff]][Z|±HH:MM]` → Unix ms (no zone = UTC).
    fn parse_ts(s: &str) -> Option<i64> {
        let day = parse_date(s)?;
        let mut ms = day * DAY_MS;
        let rest = &s[10..];
        if rest.is_empty() {
            return Some(ms);
        }
        let t = rest.strip_prefix(['T', ' '])?;
        ms += num(t, 0..2)? * HOUR_MS + num(t, 3..5)? * 60_000;
        let mut tail = &t[5..];
        if let Some(sec) = tail.strip_prefix(':') {
            ms += num(sec, 0..2)? * 1000;
            tail = &sec[2..];
            if let Some(frac) = tail.strip_prefix('.') {
                let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
                let f = &frac[..digits.min(3)];
                ms += f.parse::<i64>().unwrap_or(0) * 10i64.pow(3 - f.len() as u32);
                tail = &frac[digits..];
            }
        }
        match tail.as_bytes().first() {
            None | Some(b'Z') => Some(ms),
            Some(sign @ (b'+' | b'-')) => {
                let off = num(tail, 1..3)? * HOUR_MS + num(tail, 4..6)? * 60_000;
                Some(if *sign == b'+' { ms - off } else { ms + off })
            }
            _ => None,
        }
    }

    fn format_date(day: i64) -> String {
        let (y, m, d) = civil(day);
        format!("{y:04}-{m:02}-{d:02}")
    }

    fn format_ts(ms: i64) -> String {
        let day = ms.div_euclid(DAY_MS);
        let rem = ms.rem_euclid(DAY_MS);
        format!(
            "{}T{:02}:{:02}:{:02}.{:03}Z",
            format_date(day),
            rem / HOUR_MS,
            rem % HOUR_MS / 60_000,
            rem % 60_000 / 1000,
            rem % 1000
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn system_clock_matches_sqlite_localtime() {
            let conn = Connection::open_in_memory().unwrap();
            register_helpers(&conn, Clock::system()).unwrap();
            let (ours, sqlite): (String, String) =
                conn.query_row("SELECT today(), date('now','localtime')", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!(ours, sqlite);
        }

        #[test]
        fn dates_round_trip() {
            for day in [-1, 0, 1, 11_016, 20_366, 60_000] {
                let (y, m, d) = civil(day);
                assert_eq!(days_from_civil(y, m, d), day);
            }
            assert_eq!(civil(0), (1970, 1, 1));
            let ms = parse_ts("2026-10-05T14:03:09.120Z").unwrap();
            assert_eq!(format_ts(ms), "2026-10-05T14:03:09.120Z");
            assert_eq!(parse_ts("2026-10-05 14:03:09"), Some(ms - 120));
            assert_eq!(parse_ts("2026-10-05T16:03:09.120+02:00"), Some(ms));
            assert_eq!(parse_ts("2026-13-05"), None);
        }

        #[test]
        fn helpers_in_sql() {
            let clock = Clock::new(|| parse_ts("2026-10-05T23:30:00.000Z").unwrap(), |_| 2 * HOUR_MS);
            let conn = Connection::open_in_memory().unwrap();
            register_helpers(&conn, clock).unwrap();
            let q = |sql: &str| -> String {
                conn.query_row(sql, [], |r| r.get::<_, rusqlite::types::Value>(0)).map(|v| format!("{v:?}")).unwrap()
            };
            // 23:30Z is 01:30 on the 6th in UTC+2, so the 5th at 23:30Z is "today" locally and the 5th at 12:00Z is not.
            assert_eq!(q("SELECT is_today('2026-10-05T23:00:00.000Z')"), "Integer(1)");
            assert_eq!(q("SELECT is_today('2026-10-05T12:00:00.000Z')"), "Integer(0)");
            assert_eq!(q("SELECT local_hour('2026-10-05T23:00:00.000Z')"), "Integer(1)");
            assert_eq!(q("SELECT local_date('2026-10-05T23:00:00.000Z')"), "Text(\"2026-10-06\")");
            assert_eq!(q("SELECT local_weekday('2026-10-04')"), "Integer(0)");
            assert_eq!(q("SELECT is_this_week('2026-10-05')"), "Integer(1)");
            assert_eq!(q("SELECT is_this_week('2026-10-04')"), "Integer(0)");
            assert_eq!(q("SELECT days_ago(1)"), "Text(\"2026-10-04T23:30:00.000Z\")");
            assert_eq!(q("SELECT is_today(NULL)"), "Null");
            assert_eq!(q("SELECT now()"), "Text(\"2026-10-05T23:30:00.000Z\")");
            assert_eq!(q("SELECT today()"), "Text(\"2026-10-06\")");
            let streak = "SELECT streak(d) FROM (SELECT '2026-10-06' d UNION ALL SELECT '2026-10-05' UNION ALL SELECT '2026-10-03')";
            assert_eq!(q(streak), "Integer(2)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_skip_literals_and_comments() {
        let sql = "SELECT * FROM t WHERE a = :a AND b = ':x' -- :y\n AND c = :c /* :z */ AND d = :a";
        assert_eq!(named_params(sql), ["a", "c"]);
    }

    /// `data::SQL_RESERVED` must match what this SQLite build rejects as a table name, column name or column
    /// reference; a few keywords SQLite accepts as identifiers stay allowed.
    #[cfg(feature = "sqlite")]
    #[test]
    fn reserved_words_match_sqlite() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let usable = |w: &str| {
            conn.execute_batch(&format!(
                "CREATE TABLE {w}_t ({w} INTEGER); CREATE TABLE {w} (x INTEGER); DROP TABLE {w}"
            ))
            .is_ok()
                && conn.prepare(&format!("SELECT t.{w}, {w} FROM {w}_t t WHERE {w} = 1 ORDER BY {w}")).is_ok()
        };
        for w in crate::data::SQL_RESERVED {
            assert!(!usable(w), "`{w}` is usable as an identifier; drop it from SQL_RESERVED");
        }
        for w in [
            "key", "action", "first", "last", "row", "plan", "query", "temp", "do", "no", "date", "time", "left",
            "match",
        ] {
            assert!(usable(w), "`{w}` should be usable");
        }
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn checker_reports_reads_writes_and_denials() {
        let ddl = vec!["CREATE TABLE sets (id INTEGER PRIMARY KEY, reps INTEGER, at TEXT)".to_string()];
        let c = Checker::new(&ddl).unwrap();
        let info = c.check("SELECT count(*) AS n FROM sets WHERE is_today(at)").unwrap();
        assert!(info.readonly && info.validated);
        assert_eq!(info.columns, ["n"]);
        assert_eq!(info.reads, ["sets"]);
        let info = c.check("INSERT INTO sets (reps) VALUES (:reps)").unwrap();
        assert!(!info.readonly);
        assert_eq!(
            (info.writes.as_slice(), info.params.as_slice()),
            (&["sets".to_string()][..], &["reps".to_string()][..])
        );
        assert!(matches!(c.check("DROP TABLE sets"), Err(Error::Denied(_))));
        assert!(matches!(c.check("PRAGMA table_info(sets)"), Err(Error::Denied(_))));
        assert!(matches!(c.check("SELECT 1; SELECT 2"), Err(Error::Multiple)));
        assert!(matches!(c.check("SELECT * FROM sets WHERE id = ?"), Err(Error::Positional(_))));
        assert!(matches!(c.check("SELECT rep FROM sets"), Err(Error::Syntax(m)) if m.contains("no such column")));
    }
}
