//! Installed enee apps and their data (ADR-0005, ADR-0006; docs/specs/apps.md).
//!
//! On disk (`--data`, /data/r1os on the R1):
//!   apps/<id>/enee.yaml       the spec (source of truth), compiled at startup and on install
//!   apps/<id>/versions/       every version installed: `<ms>.json` (kind, request, the agent's
//!                             sentence, the spec), for history and undo (ported from eneeware's
//!                             per-app git history, ADR-0005 there, as plain files)
//!   data/<id>/app.db          the app's SQLite (enee-data: hardened, own tables only), plus snapshots/
//!   data/<id>/state.json      the app's `persist` state keys
//!   data/<id>/errors.json     what went wrong on its page since its last version (for a fix)
//!   trash/<id>-<ms>/          a removed app (`app/` and `data/`), restorable for TRASH_KEEP, then
//!                             purged (eneeware's .trash and 7-day purge)
//!
//! The shell renders apps itself and asks r1d to run *named* queries and *indexed* action steps. r1d
//! looks up the SQL in the compiled app and binds only declared scalar parameters, so neither a
//! spec nor the shell can run arbitrary SQL. Ported from eneeware's app_host.rs (`run_query`,
//! `run_step`, `check_params`) at 61bc7a7.

use anyhow::{Context, anyhow, bail};
use enee_data::{AppDb, Changes, Exec, Migration, Reshape, Timeouts};
use enee_spec::compiled::CompiledApp;
use enee_spec::diag::{Diagnostic, Severity};
use enee_spec::sql::Clock;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

/// A problem an app's page hit while it ran (the runtime's errors): what a fix build repairs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppError {
    pub code: String,
    pub message: String,
    /// Where: the action or view, if the page said.
    #[serde(default)]
    pub place: String,
    pub count: u32,
    pub first: i64,
    pub last: i64,
}

/// Distinct problems kept per app (the newest).
const MAX_ERRORS: usize = 10;

/// What a fix build is asked to do, from the problems the app's page reported.
pub fn fix_request(errors: &[AppError]) -> String {
    let mut out = String::from("Fix what went wrong when this app ran on the R1. Its page reported:\n");
    for e in errors {
        let place = if e.place.is_empty() { String::new() } else { format!(" in {}", e.place) };
        let times = if e.count > 1 { format!(" ({} times)", e.count) } else { String::new() };
        out.push_str(&format!("- {}{place}: {}{times}\n", e.code, e.message));
    }
    out.push_str("Keep everything else as it is.");
    out
}

/// Longest text value a query or step may bind (eneeware's MAX_TEXT).
const MAX_TEXT: usize = 64 * 1024;
/// The most an app keeps of its `persist` state, as JSON.
const KEPT_STATE_MAX: usize = 64 * 1024;
/// How long a removed app stays restorable in the trash.
pub const TRASH_KEEP: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
/// How long an action run (the agent's `run_action`) can still be undone.
const UNDO_FOR: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub struct Installed {
    pub app: CompiledApp,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub icon: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// It has a glance card, so it can be a widget (ADR-0042).
    pub glance: bool,
}

impl Summary {
    fn of(app: &CompiledApp) -> Self {
        Summary {
            id: app.app.id.clone(),
            name: app.app.name.clone(),
            icon: app.app.icon.clone(),
            accent: app.app.accent.clone(),
            description: app.app.description.clone(),
            glance: app.glance.is_some(),
        }
    }
}

/// One installed version of an app (`apps/<id>/versions/<at>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    /// When it was installed (ms since the epoch); also its id.
    pub at: i64,
    /// create · edit · undo · restore
    pub kind: String,
    /// What was asked for (the brief or the edit request).
    pub request: String,
    /// The agent's one sentence about the change (edits).
    #[serde(default)]
    pub said: String,
    /// Tables and columns this version's edit deleted, with their data.
    #[serde(default)]
    pub deleted: Vec<String>,
    pub source: String,
}

/// Tables and columns a new version no longer has: an edit deletes them with their data (ADR-0023 in
/// eneeware). Ported from enee-create's `removed_data`.
#[derive(Debug, Default)]
pub struct RemovedData {
    pub tables: Vec<String>,
    pub columns: Vec<(String, String)>,
}

pub fn removed_data(before: &CompiledApp, after: &CompiledApp) -> RemovedData {
    let mut removed = RemovedData::default();
    for table in &before.tables {
        let Some(now) = after.tables.iter().find(|t| t.name == table.name) else {
            removed.tables.push(table.name.clone());
            continue;
        };
        for column in table.columns.iter().filter(|c| !now.columns.iter().any(|n| n.name == c.name)) {
            removed.columns.push((table.name.clone(), column.name.clone()));
        }
    }
    removed
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A rejected install: the compiler's diagnostics, for the model (repair) or the person.
#[derive(Debug, Serialize)]
pub struct Rejected {
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Apps {
    specs: PathBuf,
    data: PathBuf,
    trash: PathBuf,
    installed: RwLock<BTreeMap<String, Arc<Installed>>>,
    dbs: Mutex<HashMap<String, Arc<AppDb>>>,
    /// Each app's last whole-action run, for `undo_run`.
    last_runs: Mutex<HashMap<String, LastRun>>,
}

struct LastRun {
    action: String,
    at: std::time::Instant,
    changes: Changes,
}

/// What a whole-action run did.
#[derive(Debug, Serialize)]
pub struct ActionRun {
    pub changed_tables: Vec<String>,
    pub rows_affected: u64,
    /// The action's toasts, with `{{params.x}}` filled in (others are skipped).
    pub said: Vec<String>,
    /// Whether `undo_run` can put it back.
    pub undoable: bool,
    /// Its player steps (ADR-0040), `{{params.x}}` filled in, for the caller to check and do (crate::player::perform).
    pub media: Vec<MediaStep>,
    /// Steps only the app's page runs (a link, a copy, its logic's `run`), skipped here.
    pub skipped: Vec<String>,
}

/// An action's player step, run without the app's page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaStep {
    pub op: String,
    pub url: String,
    pub title: String,
    pub subtitle: String,
}

impl Apps {
    /// Opens the app store under `root` and compiles every installed spec.
    pub fn open(root: &Path) -> anyhow::Result<Self> {
        let specs = root.join("apps");
        let data = root.join("data");
        std::fs::create_dir_all(&specs).with_context(|| format!("create {}", specs.display()))?;
        std::fs::create_dir_all(&data)?;
        let trash = root.join("trash");
        std::fs::create_dir_all(&trash)?;
        let apps = Apps { specs, data, trash, installed: RwLock::default(), dbs: Mutex::default(), last_runs: Mutex::default() };
        apps.load();
        Ok(apps)
    }

    fn load(&self) {
        let Ok(entries) = std::fs::read_dir(&self.specs) else { return };
        let mut installed = self.installed.write().unwrap();
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path().join("enee.yaml");
            let Ok(source) = std::fs::read_to_string(&path) else { continue };
            match compile(&source) {
                Ok(app) if app.app.id == id => {
                    installed.insert(id, Arc::new(Installed { app, source }));
                }
                Ok(app) => tracing::warn!(dir = %id, spec_id = %app.app.id, "app id doesn't match its directory; skipped"),
                Err(r) => tracing::warn!(app = %id, errors = ?first_errors(&r.diagnostics), "app doesn't compile; skipped"),
            }
        }
        tracing::info!(count = installed.len(), "apps loaded");
    }

    /// Installed apps, most recently created or changed first.
    pub fn list(&self) -> Vec<Summary> {
        let mut apps: Vec<(std::time::SystemTime, Summary)> = self
            .installed
            .read()
            .unwrap()
            .values()
            .map(|i| {
                let changed = std::fs::metadata(self.specs.join(&i.app.app.id).join("enee.yaml"))
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (changed, Summary::of(&i.app))
            })
            .collect();
        apps.sort_by_key(|a| std::cmp::Reverse(a.0));
        apps.into_iter().map(|a| a.1).collect()
    }

    /// The id `source` declares, if it compiles and that app is installed.
    pub fn get_source_id(&self, source: &str) -> Option<String> {
        let id = compile(source).ok()?.app.id;
        self.get(&id).map(|_| id)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Installed>> {
        self.installed.read().unwrap().get(id).cloned()
    }

    /// Compiles `source` and, if it has no errors, installs it (replacing an app with the same id).
    /// Returns the summary plus any warnings.
    pub fn install(&self, source: &str) -> Result<(Summary, Vec<Diagnostic>), Rejected> {
        let out = enee_spec::compile(source, &enee_spec::Options::default());
        let warnings: Vec<Diagnostic> = out.diagnostics.iter().filter(|d| !is_error(d)).cloned().collect();
        let app = match (out.ok, out.app) {
            (true, Some(app)) => app,
            _ => return Err(Rejected { diagnostics: out.diagnostics }),
        };
        let id = app.app.id.clone();
        if !valid_app_id(&id) {
            return Err(Rejected { diagnostics: vec![] });
        }
        let dir = self.specs.join(&id);
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            let tmp = dir.join("enee.yaml.tmp");
            std::fs::write(&tmp, source)?;
            std::fs::rename(&tmp, dir.join("enee.yaml"))
        };
        if let Err(e) = write() {
            tracing::error!(app = %id, error = %e, "can't write the spec");
            return Err(Rejected { diagnostics: vec![] });
        }
        let summary = Summary::of(&app);
        // A new version may change the schema: drop the open database so the next use migrates it.
        self.dbs.lock().unwrap().remove(&id);
        // Its problems were the old version's.
        self.clear_errors(&id);
        self.installed.write().unwrap().insert(id, Arc::new(Installed { app, source: source.to_owned() }));
        Ok((summary, warnings))
    }

    /// Removes the app: its spec, versions and data move to the trash, restorable for TRASH_KEEP.
    pub fn remove(&self, id: &str) -> anyhow::Result<()> {
        if self.installed.write().unwrap().remove(id).is_none() {
            bail!("no app `{id}`");
        }
        self.dbs.lock().unwrap().remove(id);
        self.last_runs.lock().unwrap().remove(id);
        let to = self.trash.join(format!("{id}-{}", now_ms()));
        std::fs::create_dir_all(&to)?;
        std::fs::rename(self.specs.join(id), to.join("app")).context("move the app to the trash")?;
        if self.data.join(id).exists() {
            std::fs::rename(self.data.join(id), to.join("data")).context("move the app's data to the trash")?;
        }
        Ok(())
    }

    /// Removed apps still in the trash, newest first: (id, removed at ms, name).
    pub fn trashed(&self) -> Vec<(String, i64, String)> {
        let Ok(entries) = std::fs::read_dir(&self.trash) else { return Vec::new() };
        let mut out: Vec<(String, i64, String)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let (id, at) = name.rsplit_once('-')?;
                let at: i64 = at.parse().ok()?;
                let source = std::fs::read_to_string(e.path().join("app").join("enee.yaml")).ok()?;
                let app_name = compile(&source).map(|a| a.app.name).unwrap_or_else(|_| id.to_owned());
                Some((id.to_owned(), at, app_name))
            })
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.1));
        out
    }

    /// Brings back the newest removed copy of `id`, data and versions too.
    pub fn restore_trashed(&self, id: &str) -> anyhow::Result<Summary> {
        if self.get(id).is_some() {
            bail!("`{id}` is installed already");
        }
        let (_, at, _) = self.trashed().into_iter().find(|t| t.0 == id).ok_or_else(|| anyhow!("`{id}` isn't in the trash"))?;
        let from = self.trash.join(format!("{id}-{at}"));
        let source = std::fs::read_to_string(from.join("app").join("enee.yaml"))?;
        let app = compile(&source).map_err(|r| anyhow!("the removed app doesn't compile any more: {:?}", first_errors(&r.diagnostics)))?;
        std::fs::rename(from.join("app"), self.specs.join(id))?;
        if from.join("data").exists() {
            std::fs::rename(from.join("data"), self.data.join(id))?;
        }
        let _ = std::fs::remove_dir_all(&from);
        let summary = Summary::of(&app);
        self.installed.write().unwrap().insert(id.to_owned(), Arc::new(Installed { app, source }));
        Ok(summary)
    }

    /// Deletes trash entries older than TRASH_KEEP, data and all. Returns the ids purged.
    pub fn purge_trash(&self) -> Vec<String> {
        let keep_ms = i64::try_from(TRASH_KEEP.as_millis()).unwrap_or(i64::MAX);
        let mut purged = Vec::new();
        for (id, at, _) in self.trashed() {
            if now_ms() - at < keep_ms {
                continue;
            }
            let _ = std::fs::remove_dir_all(self.trash.join(format!("{id}-{at}")));
            tracing::info!(app = %id, "purged from the trash");
            if self.get(&id).is_none() {
                purged.push(id);
            }
        }
        purged
    }

    /// The app's versions, oldest first.
    pub fn versions(&self, id: &str) -> Vec<Version> {
        let Ok(entries) = std::fs::read_dir(self.specs.join(id).join("versions")) else { return Vec::new() };
        let mut out: Vec<Version> = entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
            .collect();
        out.sort_by_key(|v| v.at);
        out
    }

    /// Records a version (the newest becomes the current one).
    pub fn record(&self, id: &str, kind: &str, request: &str, said: &str, deleted: Vec<String>, source: &str) -> anyhow::Result<Version> {
        let dir = self.specs.join(id).join("versions");
        std::fs::create_dir_all(&dir)?;
        // Strictly increasing, even for two records in the same millisecond.
        let at = now_ms().max(self.versions(id).last().map_or(0, |v| v.at + 1));
        let v = Version { at, kind: kind.into(), request: request.into(), said: said.into(), deleted, source: source.into() };
        std::fs::write(dir.join(format!("{at}.json")), serde_json::to_string(&v)?)?;
        Ok(v)
    }

    /// Installs a spec that came whole (a file, or one an agent was given): a version of its own, `how` it came
    /// (a replacement, if the app has history already).
    pub fn install_recorded(&self, source: &str, how: &str) -> Result<(Summary, Vec<Diagnostic>), Rejected> {
        let had = self.get_source_id(source).is_some_and(|id| !self.versions(&id).is_empty());
        self.install(source).inspect(|(summary, _)| {
            let kind = if had { "edit" } else { "create" };
            if let Err(e) = self.record(&summary.id, kind, how, "", Vec::new(), source) {
                tracing::warn!(app = %summary.id, error = %e, "couldn't record the version");
            }
        })
    }

    /// Installs an edit of an installed app: records it, swaps the spec, and migrates the data, deleting
    /// the tables and columns the edit removed (an undo brings them back from the snapshot taken first).
    /// Apps from before history existed get their current spec recorded as the first version.
    pub fn install_edit(&self, id: &str, source: &str, kind: &str, request: &str, said: &str) -> anyhow::Result<(Summary, Option<Migration>)> {
        let before = self.get(id).ok_or_else(|| anyhow!("no app `{id}`"))?;
        if self.versions(id).is_empty() {
            self.record(id, "create", "", "", Vec::new(), &before.source)?;
        }
        let after = compile(source).map_err(|r| anyhow!("the new version doesn't compile: {:?}", first_errors(&r.diagnostics)))?;
        if after.app.id != id {
            bail!("an edit can't change the app's id (`{id}` → `{}`)", after.app.id);
        }
        let removed = removed_data(&before.app, &after);
        let (summary, _) = self.install(source).map_err(|r| anyhow!("install rejected the spec: {:?}", first_errors(&r.diagnostics)))?;
        let reshape = Reshape { drop_tables: removed.tables, drop_columns: removed.columns, restore: false };
        let migration = self.reshape(id, &reshape)?;
        let deleted = migration.as_ref().map(|m| m.dropped.clone()).unwrap_or_default();
        self.record(id, kind, request, said, deleted, source)?;
        Ok((summary, migration))
    }

    /// Goes back to the version before the current one, bringing back data the undone edit deleted.
    /// The undone version moves to `versions/undone/` (kept, not listed).
    pub fn undo(&self, id: &str) -> anyhow::Result<(Summary, Version, Option<Migration>)> {
        let versions = self.versions(id);
        let [.., previous, current] = versions.as_slice() else { bail!("there's nothing to undo") };
        let (summary, _) = self.install(&previous.source).map_err(|r| anyhow!("the earlier version doesn't install: {:?}", first_errors(&r.diagnostics)))?;
        let migration = self.reshape(id, &Reshape { restore: true, ..Reshape::default() })?;
        let dir = self.specs.join(id).join("versions");
        std::fs::create_dir_all(dir.join("undone"))?;
        let name = format!("{}.json", current.at);
        std::fs::rename(dir.join(&name), dir.join("undone").join(&name))?;
        Ok((summary, current.clone(), migration))
    }

    /// Reopens the app's database with `reshape` (enee-data: snapshot first, then drop or restore).
    fn reshape(&self, id: &str, reshape: &Reshape) -> anyhow::Result<Option<Migration>> {
        let installed = self.get(id).ok_or_else(|| anyhow!("no app `{id}`"))?;
        let mut dbs = self.dbs.lock().unwrap();
        dbs.remove(id);
        let opened = AppDb::open_reshaping(&self.data.join(id), &installed.app.ddl, Clock::system(), Timeouts::default(), reshape)?;
        if let Some(m) = &opened.migration {
            tracing::info!(app = %id, migration = ?m, "migrated");
        }
        dbs.insert(id.to_owned(), Arc::new(opened.db));
        Ok(opened.migration)
    }

    /// The app's database, opened (and migrated to its DDL) on first use.
    pub fn db(&self, id: &str) -> anyhow::Result<(Arc<Installed>, Arc<AppDb>)> {
        let installed = self.get(id).ok_or_else(|| anyhow!("no app `{id}`"))?;
        let mut dbs = self.dbs.lock().unwrap();
        if let Some(db) = dbs.get(id) {
            return Ok((installed, db.clone()));
        }
        let opened = AppDb::open(&self.data.join(id), &installed.app.ddl, Clock::system(), Timeouts::default())?;
        if let Some(m) = &opened.migration {
            tracing::info!(app = %id, migration = ?m, "migrated");
        }
        let db = Arc::new(opened.db);
        dbs.insert(id.to_owned(), db.clone());
        Ok((installed, db))
    }

    /// `query.run`: the app's query by name, with declared scalar params.
    pub fn query(&self, id: &str, name: &str, params: &Map<String, Value>) -> anyhow::Result<Vec<Map<String, Value>>> {
        let (installed, db) = self.db(id)?;
        let query = installed.app.queries.iter().find(|q| q.name == name).ok_or_else(|| anyhow!("no query `{name}`"))?;
        let allowed: Vec<String> = query.params.keys().cloned().collect();
        check_params(params, &allowed, &format!("query `{name}`"))?;
        db.query(&query.sql, params)
    }

    /// `action.step`: one of the app's `sql` steps, with declared scalar params.
    pub fn step(&self, id: &str, action: &str, index: usize, params: &Map<String, Value>) -> anyhow::Result<Exec> {
        let (installed, db) = self.db(id)?;
        let step = installed
            .app
            .actions
            .get(action)
            .and_then(|a| a.get("steps"))
            .and_then(Value::as_array)
            .and_then(|steps| steps.get(index))
            .ok_or_else(|| anyhow!("no step {index} in action `{action}`"))?;
        let sql = step.get("sql").and_then(Value::as_str).ok_or_else(|| anyhow!("step {index} of `{action}` is not a sql step"))?;
        let allowed: Vec<String> = match step.get("params").and_then(Value::as_array) {
            Some(names) => names.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
            None => enee_spec::sql::named_params(sql),
        };
        check_params(params, &allowed, &format!("step {index} of `{action}`"))?;
        db.exec(sql, params)
    }

    /// Runs a whole action outside the app's screen (the agent's `run_action`): its `sql` steps in
    /// order, as one recorded run (rolled back if a step fails; `undo_run` puts it back later).
    /// Step parameters bind from `params`, then the app's kept state, then its settings, as
    /// eneeware's command runner did. Screen-only steps (`set`, `navigate`, …) are skipped; `toast`
    /// text is returned so the agent can say it.
    pub fn run_action(&self, id: &str, action: &str, params: &Map<String, Value>) -> anyhow::Result<ActionRun> {
        let (installed, db) = self.db(id)?;
        let def = installed.app.actions.get(action).ok_or_else(|| anyhow!("`{id}` has no action `{action}`"))?;
        let params = coerce_params(def.get("params").and_then(Value::as_object), params, action)?;
        let mut sources = self.settings(id).unwrap_or_default();
        sources.extend(self.kept_state(id));
        sources.extend(params.clone());
        // An optional param (`text?`) left out is null, not a step that can't run (T-045's edit evals found it).
        for (name, d) in def.get("params").and_then(Value::as_object).into_iter().flatten() {
            if d.get("required").and_then(Value::as_bool) != Some(true) && !sources.contains_key(name) {
                sources.insert(name.clone(), Value::Null);
            }
        }
        let steps = def.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();

        let mut rec = db.record();
        let mut result =
            ActionRun { changed_tables: Vec::new(), rows_affected: 0, said: Vec::new(), undoable: false, media: Vec::new(), skipped: Vec::new() };
        let mut failed: Option<anyhow::Error> = None;
        for (i, step) in steps.iter().enumerate() {
            if let Some(sql) = step.get("sql").and_then(Value::as_str) {
                let names: Vec<String> = match step.get("params").and_then(Value::as_array) {
                    Some(n) => n.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
                    None => enee_spec::sql::named_params(sql),
                };
                let mut bound = Map::new();
                for n in &names {
                    match sources.get(n) {
                        Some(v) => {
                            bound.insert(n.clone(), v.clone());
                        }
                        None => {
                            failed = Some(anyhow!("step {i} of `{action}` needs :{n}, which wasn't given"));
                            break;
                        }
                    }
                }
                if failed.is_some() {
                    break;
                }
                if let Err(e) = check_params(&bound, &names, &format!("step {i} of `{action}`")) {
                    failed = Some(e);
                    break;
                }
                match rec.exec(sql, &bound) {
                    Ok(exec) => {
                        result.rows_affected += exec.rows_affected;
                        for t in exec.changed_tables {
                            if !result.changed_tables.contains(&t) {
                                result.changed_tables.push(t);
                            }
                        }
                    }
                    Err(e) => {
                        failed = Some(e.context(format!("step {i} of `{action}`")));
                        break;
                    }
                }
            } else if let Some(t) = step.get("toast").and_then(|t| toast_text(t, &params)) {
                result.said.push(t);
            } else if let Some(p) = step.get("play") {
                let part = |k: &str| p.get(k).and_then(|v| toast_text(v, &params)).unwrap_or_default();
                result.media.push(MediaStep { op: "play".into(), url: part("url"), title: part("title"), subtitle: part("subtitle") });
            } else if let Some(op) = ["pause", "resume", "stop", "toggle"].into_iter().find(|k| step.get(*k).is_some()) {
                result.media.push(MediaStep { op: op.into(), url: String::new(), title: String::new(), subtitle: String::new() });
            } else if let Some(kind) = step.as_object().and_then(|m| m.keys().next()).filter(|k| ["open_url", "copy", "run"].contains(&k.as_str())) {
                result.skipped.push(kind.clone());
            }
        }
        let changes = rec.finish();
        if let Some(e) = failed {
            if !changes.is_empty() {
                db.revert(&changes)?;
            }
            return Err(e);
        }
        result.undoable = !changes.is_empty() && changes.revertible();
        if result.undoable {
            self.last_runs.lock().unwrap().insert(id.to_owned(), LastRun { action: action.to_owned(), at: std::time::Instant::now(), changes });
        }
        Ok(result)
    }

    /// Puts back the app's last `run_action`, if it's less than UNDO_FOR old. Returns the action's
    /// name and what changed.
    pub fn undo_run(&self, id: &str) -> anyhow::Result<(String, Exec)> {
        let last = self.last_runs.lock().unwrap().remove(id).ok_or_else(|| anyhow!("nothing to undo in `{id}`"))?;
        if last.at.elapsed() > UNDO_FOR {
            bail!("the last change in `{id}` is too old to undo");
        }
        let (_, db) = self.db(id)?;
        Ok((last.action, db.revert(&last.changes)?))
    }

    pub fn settings(&self, id: &str) -> anyhow::Result<Map<String, Value>> {
        self.db(id)?.1.settings()
    }

    /// Changes the app's settings (`values`: key → new value), each checked against its definition
    /// (type, options, min and max). Returns them all, as the app reads them.
    /// Checks new values for an app's settings without saving them (an agent asks before it changes them).
    pub fn check_settings(&self, id: &str, values: &Map<String, Value>) -> anyhow::Result<()> {
        let installed = self.get(id).ok_or_else(|| anyhow!("no app `{id}`"))?;
        for (key, v) in values {
            let def = installed.app.settings.get(key).ok_or_else(|| anyhow!("`{id}` has no setting `{key}`"))?;
            setting_value(key, def, v)?;
        }
        Ok(())
    }

    pub fn set_settings(&self, id: &str, values: &Map<String, Value>) -> anyhow::Result<Map<String, Value>> {
        let (installed, db) = self.db(id)?;
        let mut checked = Vec::new();
        for (key, v) in values {
            let def = installed.app.settings.get(key).ok_or_else(|| anyhow!("`{id}` has no setting `{key}`"))?;
            checked.push((key, setting_value(key, def, v)?));
        }
        for (key, value) in checked {
            // `key` is one of the app's settings: a column the compiler checked is a plain SQL name.
            db.exec(&format!("UPDATE settings SET \"{key}\" = :v WHERE id = 1"), &Map::from_iter([("v".to_owned(), value)]))?;
        }
        db.settings()
    }

    /// An app with its data (T-048): its spec, every table's rows and its settings, as `r1h export --data` and the
    /// companion save it (`<id>.enee.json`).
    pub fn export(&self, id: &str) -> anyhow::Result<Value> {
        let (installed, db) = self.db(id)?;
        let mut data = Map::new();
        for t in &installed.app.tables {
            // A table's name is one the compiler checked: a plain SQL name.
            let rows = db.query(&format!("SELECT * FROM \"{}\"", t.name), &Map::new())?;
            data.insert(t.name.clone(), Value::Array(rows.into_iter().map(Value::Object).collect()));
        }
        Ok(json!({ "enee_export": 1, "source": installed.source, "data": data, "settings": self.settings(id).unwrap_or_default() }))
    }

    /// Puts an export's rows and settings into an app that has no data yet (T-048), so nothing is doubled: the rows
    /// keep their ids (references between tables still hold), and columns the app doesn't have are left out, as
    /// are settings it doesn't declare. The rows put in.
    pub fn import_data(&self, id: &str, data: &Map<String, Value>, settings: &Map<String, Value>) -> anyhow::Result<usize> {
        let (installed, db) = self.db(id)?;
        let count_of = |t: &str| -> anyhow::Result<i64> {
            Ok(db.query(&format!("SELECT count(*) AS n FROM \"{t}\""), &Map::new())?.first().and_then(|r| r["n"].as_i64()).unwrap_or(0))
        };
        let tables: Vec<&enee_spec::compiled::Table> = installed.app.tables.iter().filter(|t| data.contains_key(&t.name)).collect();
        for t in &tables {
            anyhow::ensure!(count_of(&t.name)? == 0, "`{id}` already has data in `{}`, so none was imported", t.name);
        }
        let mut put = 0;
        for table in tables {
            let t = table.name.as_str();
            // Its columns as compiled (app SQL takes no pragmas): plain SQL names, so they can go in the statement.
            let columns: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
            for row in data[t].as_array().into_iter().flatten().filter_map(Value::as_object) {
                let keep: Vec<&String> = row.keys().filter(|k| columns.contains(&k.as_str())).collect();
                if keep.is_empty() {
                    continue;
                }
                let names = keep.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", ");
                let values = keep.iter().map(|k| format!(":{k}")).collect::<Vec<_>>().join(", ");
                let params: Map<String, Value> = keep.iter().map(|k| ((*k).clone(), row[k.as_str()].clone())).collect();
                db.exec(&format!("INSERT INTO \"{t}\" ({names}) VALUES ({values})"), &params)?;
                put += 1;
            }
        }
        let known: Map<String, Value> = settings.iter().filter(|(k, _)| installed.app.settings.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        if !known.is_empty() {
            self.set_settings(id, &known)?;
        }
        Ok(put)
    }

    /// Starts the app over: its database (data and settings) and kept state are erased. The database is
    /// kept in its `snapshots/` as a safety net (enee-data). Returns its tables, for refreshing.
    pub fn erase_data(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let installed = self.get(id).ok_or_else(|| anyhow!("no app `{id}`"))?;
        self.dbs.lock().unwrap().remove(id);
        self.last_runs.lock().unwrap().remove(id);
        let dir = self.data.join(id);
        enee_data::erase_files(&dir)?;
        let _ = std::fs::remove_file(dir.join("state.json"));
        let mut tables: Vec<String> = installed.app.tables.iter().map(|t| t.name.clone()).collect();
        if !installed.app.settings.is_empty() {
            tables.push("settings".into());
        }
        Ok(tables)
    }

    /// Records a problem the app's page hit (a repeat counts up). Returns how many distinct problems it has.
    pub fn record_error(&self, id: &str, code: &str, message: &str, place: &str) -> anyhow::Result<usize> {
        self.get(id).ok_or_else(|| anyhow!("no app `{id}`"))?;
        let code: String = code.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect();
        let message: String = message.chars().take(500).collect();
        let mut list = self.errors(id);
        let now = now_ms();
        match list.iter_mut().find(|e| e.code == code && e.message == message) {
            Some(e) => {
                e.count += 1;
                e.last = now;
            }
            None => list.push(AppError { code, message, place: place.chars().take(120).collect(), count: 1, first: now, last: now }),
        }
        if list.len() > MAX_ERRORS {
            list.remove(0);
        }
        let dir = self.data.join(id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("errors.json"), serde_json::to_string(&list)?)?;
        Ok(list.len())
    }

    /// The problems the app's page reported since its last version.
    pub fn errors(&self, id: &str) -> Vec<AppError> {
        std::fs::read_to_string(self.data.join(id).join("errors.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Its problems are forgotten (a new version, or they were dealt with).
    pub fn clear_errors(&self, id: &str) {
        let _ = std::fs::remove_file(self.data.join(id).join("errors.json"));
    }

    /// The app's saved `persist` state keys (empty if none).
    pub fn kept_state(&self, id: &str) -> Map<String, Value> {
        std::fs::read_to_string(self.data.join(id).join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Saves the app's `persist` state keys (only the keys it declares).
    pub fn save_state(&self, id: &str, values: &Map<String, Value>) -> anyhow::Result<()> {
        let installed = self.get(id).ok_or_else(|| anyhow!("no app `{id}`"))?;
        let kept: Map<String, Value> =
            values.iter().filter(|(k, _)| installed.app.persist.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        let json = serde_json::to_string(&kept)?;
        if json.len() > KEPT_STATE_MAX {
            bail!("state too large ({} bytes)", json.len());
        }
        let dir = self.data.join(id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("state.json"), json)?;
        Ok(())
    }
}

/// A setting's new value, as its column holds it: checked against the setting's type, options and bounds.
fn setting_value(key: &str, def: &Value, v: &Value) -> anyhow::Result<Value> {
    let ty = def["type"].as_str().unwrap_or("text");
    let number = v.as_f64().or_else(|| v.as_str()?.trim().parse().ok());
    let bounded = |n: f64| -> anyhow::Result<f64> {
        if let Some(min) = def["min"].as_f64()
            && n < min
        {
            bail!("`{key}` can't be less than {min}");
        }
        if let Some(max) = def["max"].as_f64()
            && n > max
        {
            bail!("`{key}` can't be more than {max}");
        }
        Ok(n)
    };
    let text = |v: &Value| v.as_str().map(str::trim).map(str::to_owned);
    Ok(match ty {
        "int" => match number {
            Some(n) if n.fract() == 0.0 => json!(bounded(n)? as i64),
            _ => bail!("`{key}` is a whole number"),
        },
        "real" => json!(bounded(number.ok_or_else(|| anyhow!("`{key}` is a number"))?)?),
        "bool" => match v {
            Value::Bool(b) => json!(b),
            Value::String(s) if s == "true" || s == "false" => json!(s == "true"),
            _ => bail!("`{key}` is true or false"),
        },
        // Seconds: a number, or a duration like 25m.
        "duration" => {
            let secs = number.or_else(|| enee_spec::duration_ms(v).map(|ms| ms as f64 / 1000.0));
            match secs {
                Some(s) if s >= 0.0 => json!(bounded(s.round())? as i64),
                _ => bail!("`{key}` is a duration, like 90s, 25m or 1h"),
            }
        }
        "enum" => {
            let s = text(v).unwrap_or_default();
            let options: Vec<&str> = def["options"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if !options.contains(&s.as_str()) {
                bail!("`{key}` is one of: {}", options.join(", "));
            }
            json!(s)
        }
        "time" => match text(v) {
            Some(s) if is_clock(&s) => json!(s),
            _ => bail!("`{key}` is a time like 09:00"),
        },
        _ => match text(v).or_else(|| number.map(|n| n.to_string())) {
            Some(s) if s.chars().count() <= 200 => json!(s),
            Some(_) => bail!("`{key}` is at most 200 characters"),
            None => bail!("`{key}` is text"),
        },
    })
}

/// `HH:MM`, 24-hour.
fn is_clock(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else { return false };
    h.len() == 2 && m.len() == 2 && h.parse::<u8>().is_ok_and(|h| h < 24) && m.parse::<u8>().is_ok_and(|m| m < 60)
}

/// Compiles a spec, failing on errors (warnings are fine).
pub fn compile(source: &str) -> Result<CompiledApp, Rejected> {
    let out = enee_spec::compile(source, &enee_spec::Options::default());
    match (out.ok, out.app) {
        (true, Some(app)) => Ok(app),
        _ => Err(Rejected { diagnostics: out.diagnostics }),
    }
}

fn is_error(d: &Diagnostic) -> bool {
    d.severity == Severity::Error
}

fn first_errors(diags: &[Diagnostic]) -> Vec<String> {
    diags.iter().filter(|d| is_error(d)).take(3).map(|d| format!("{} {} ({})", d.code, d.message, d.path)).collect()
}

/// App ids name directories: lowercase letters, digits and dashes.
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Bound values are SQL scalars, and only names the statement declares.
/// The agent's action params, checked against the action's declared ones and coerced to their types
/// (a model may send `"2"` for an int).
fn coerce_params(declared: Option<&Map<String, Value>>, given: &Map<String, Value>, action: &str) -> anyhow::Result<Map<String, Value>> {
    let empty = Map::new();
    let declared = declared.unwrap_or(&empty);
    let mut out = Map::new();
    for (name, value) in given {
        let Some(def) = declared.get(name) else {
            bail!("`{action}` has no parameter `{name}` (it takes: {})", declared.keys().cloned().collect::<Vec<_>>().join(", "));
        };
        let ty = def.get("type").and_then(Value::as_str).unwrap_or("text");
        let v = match (ty, value) {
            ("int", Value::String(s)) => s.trim().parse::<i64>().map(Value::from).map_err(|_| anyhow!("`{name}` must be a whole number"))?,
            ("real", Value::String(s)) => s.trim().parse::<f64>().ok().and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)).ok_or_else(|| anyhow!("`{name}` must be a number"))?,
            ("bool", Value::String(s)) => Value::Bool(matches!(s.trim(), "true" | "yes" | "1")),
            ("enum", Value::String(s)) => {
                let ok = def.get("options").and_then(Value::as_array).is_none_or(|o| o.iter().any(|x| x.as_str() == Some(s.as_str())));
                if !ok {
                    bail!("`{name}` must be one of {}", def.get("options").map(Value::to_string).unwrap_or_default());
                }
                value.clone()
            }
            _ => value.clone(),
        };
        out.insert(name.clone(), v);
    }
    for (name, def) in declared {
        if def.get("required").and_then(Value::as_bool) == Some(true) && !out.contains_key(name) {
            bail!("`{action}` needs `{name}`");
        }
    }
    Ok(out)
}

/// A toast's text: a literal, or its compiled template (enee-spec's value IR) when it binds only
/// action params (`/params/x`, no filters). Anything else is the screen's to render: None.
fn toast_text(v: &Value, params: &Map<String, Value>) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => {
            if let Some(x) = o.get("$x") {
                return toast_text(x, params);
            }
            if let Some(Value::Array(parts)) = o.get("t") {
                return parts.iter().map(|p| toast_text(p, params)).collect::<Option<Vec<_>>>().map(|v| v.concat());
            }
            let path = o.get("p").and_then(Value::as_str)?;
            if o.contains_key("f") {
                return None;
            }
            match params.get(path.strip_prefix("/params/")?)? {
                Value::String(s) => Some(s.clone()),
                v => Some(v.to_string()),
            }
        }
        _ => None,
    }
}

fn check_params(params: &Map<String, Value>, allowed: &[String], what: &str) -> anyhow::Result<()> {
    for (name, value) in params {
        if !allowed.iter().any(|a| a == name) {
            bail!("{what} has no parameter :{name}");
        }
        let scalar = match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => true,
            Value::String(s) => s.len() <= MAX_TEXT,
            Value::Array(_) | Value::Object(_) => false,
        };
        if !scalar {
            bail!("{what}: :{name} must be text, a number, a boolean or null");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WATER: &str = include_str!("../../../fixtures/apps/water.enee.yaml");

    fn store() -> (Apps, PathBuf) {
        let dir = std::env::temp_dir().join(format!("r1d-apps-test-{}-{}", std::process::id(), rand_id()));
        (Apps::open(&dir).unwrap(), dir)
    }

    /// The time and a counter: on Windows two tests can start in the same tick, and shared a folder (2026-10-11).
    fn rand_id() -> String {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        format!("{t}-{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    #[test]
    fn install_query_and_step() {
        let (apps, dir) = store();
        let (summary, _) = apps.install(WATER).map_err(|r| format!("{:?}", r.diagnostics)).unwrap();
        assert_eq!(apps.list().len(), 1);
        let app = apps.get(&summary.id).unwrap();

        // Every query runs on a fresh database.
        for q in &app.app.queries {
            let params: Map<String, Value> = q.params.keys().map(|k| (k.clone(), Value::Null)).collect();
            apps.query(&summary.id, &q.name, &params).unwrap_or_else(|e| panic!("query {}: {e}", q.name));
        }
        // Undeclared parameters and unknown queries are refused.
        let q = &app.app.queries[0];
        let mut bad = Map::new();
        bad.insert("nope".into(), Value::from(1));
        assert!(apps.query(&summary.id, &q.name, &bad).is_err());
        assert!(apps.query(&summary.id, "drop_everything", &Map::new()).is_err());

        // Reopening finds the installed app again.
        drop(apps);
        let again = Apps::open(&dir).unwrap();
        assert!(again.get(&summary.id).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_are_checked_and_erase_starts_over() {
        let (apps, dir) = store();
        let (summary, _) = apps.install(WATER).map_err(|r| format!("{:?}", r.diagnostics)).unwrap();
        let id = summary.id.as_str();
        let set = |k: &str, v: Value| apps.set_settings(id, &Map::from_iter([(k.to_owned(), v)]));
        assert_eq!(set("daily_goal", json!("10")).unwrap()["daily_goal"], json!(10), "a number sent as text");
        assert!(set("daily_goal", json!(0)).is_err(), "below its min");
        assert!(set("daily_goal", json!(2.5)).is_err(), "not whole");
        assert_eq!(set("remind_every", json!("30m")).unwrap()["remind_every"], json!(1800), "a duration, in seconds");
        assert!(set("nope", json!(1)).is_err(), "not one of its settings");
        apps.run_action(id, "drink", &Map::new()).ok();
        assert!(total(&apps, id) >= 0);
        apps.erase_data(id).unwrap();
        assert_eq!(total(&apps, id), 0);
        assert_eq!(apps.settings(id).unwrap()["daily_goal"], json!(8), "back to its default");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn total(apps: &Apps, id: &str) -> i64 {
        let (_, db) = apps.db(id).unwrap();
        db.query("SELECT coalesce(sum(count), 0) AS n FROM glasses", &Map::new()).unwrap()[0]["n"].as_i64().unwrap()
    }

    fn args(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
    }

    #[test]
    fn an_export_brings_its_data_and_settings() {
        let (from, dir) = store();
        let (s, _) = from.install(WATER).map_err(|r| format!("{:?}", r.diagnostics)).unwrap();
        from.run_action(&s.id, "drink", &args(&[("count", json!(2))])).unwrap();
        from.run_action(&s.id, "drink", &args(&[("count", json!(3))])).unwrap();
        from.set_settings(&s.id, &args(&[("daily_goal", json!(10))])).unwrap();
        let export = from.export(&s.id).unwrap();
        assert_eq!(export["data"]["glasses"].as_array().unwrap().len(), 2);
        let (to, dir2) = store();
        let (s2, _) = to.install(export["source"].as_str().unwrap()).unwrap();
        let put = to.import_data(&s2.id, export["data"].as_object().unwrap(), export["settings"].as_object().unwrap()).unwrap();
        assert_eq!(put, 2);
        assert_eq!(total(&to, &s2.id), 5);
        assert_eq!(to.settings(&s2.id).unwrap()["daily_goal"], json!(10));
        // Never twice: an app with data already takes none.
        assert!(to.import_data(&s2.id, export["data"].as_object().unwrap(), &Map::new()).is_err());
        assert_eq!(total(&to, &s2.id), 5);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn an_optional_param_left_out_is_null() {
        let (apps, dir) = store();
        let spec = r"enee: 1
app: { id: spend, name: Spend, icon: wallet }
data:
  spent:
    amount: real!
    note: text
actions:
  add:
    params: { amount: real, note: text? }
    do:
      - sql: INSERT INTO spent (amount, note) VALUES (:amount, :note)
views:
  main:
    - text: { text: Spend }
";
        let (s, _) = apps.install(spec).map_err(|r| format!("{:?}", r.diagnostics)).unwrap();
        apps.run_action(&s.id, "add", &args(&[("amount", json!(12.5))])).unwrap();
        apps.run_action(&s.id, "add", &args(&[("amount", json!(30)), ("note", json!("phone"))])).unwrap();
        let (_, db) = apps.db(&s.id).unwrap();
        let rows = db.query("SELECT amount, note FROM spent ORDER BY amount", &Map::new()).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0]["note"].is_null());
        assert_eq!(rows[1]["note"], json!("phone"));
        assert!(apps.run_action(&s.id, "add", &Map::new()).is_err(), "amount is still needed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn run_action_coerces_params_and_undo_puts_it_back() {
        let (apps, dir) = store();
        let (s, _) = apps.install(WATER).map_err(|r| format!("{:?}", r.diagnostics)).unwrap();
        // A model may send "2" for an int.
        let run = apps.run_action(&s.id, "drink", &args(&[("count", Value::from("2"))])).unwrap();
        assert_eq!(run.rows_affected, 1);
        assert_eq!(run.said, vec!["+2 glass".to_owned()]);
        assert!(run.undoable);
        assert_eq!(total(&apps, &s.id), 2);
        assert!(apps.run_action(&s.id, "drink", &args(&[("cups", Value::from(1))])).is_err(), "unknown param");
        assert!(apps.run_action(&s.id, "drink", &Map::new()).is_err(), "missing param");
        assert!(apps.run_action(&s.id, "nope", &Map::new()).is_err(), "unknown action");
        let (action, _) = apps.undo_run(&s.id).unwrap();
        assert_eq!(action, "drink");
        assert_eq!(total(&apps, &s.id), 0);
        assert!(apps.undo_run(&s.id).is_err(), "only the last run");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_edit_deletes_what_it_removed_and_undo_brings_it_back() {
        let (apps, dir) = store();
        let (s, _) = apps.install(WATER).map_err(|r| format!("{:?}", r.diagnostics)).unwrap();
        let with_note = WATER.replacen("    at: datetime = now
", "    at: datetime = now
    note: text?
", 1);
        assert_ne!(with_note, WATER);
        apps.install_edit(&s.id, &with_note, "edit", "add a note", "Added a note.").unwrap();
        let (_, db) = apps.db(&s.id).unwrap();
        db.exec("INSERT INTO glasses (count, note) VALUES (1, 'after lunch')", &Map::new()).unwrap();
        drop(db);

        // Removing the column deletes its values; the history keeps both versions plus the first.
        let (_, m) = apps.install_edit(&s.id, WATER, "edit", "drop the note", "Removed the note.").unwrap();
        assert_eq!(m.unwrap().dropped, vec!["glasses.note".to_owned()]);
        let versions = apps.versions(&s.id);
        assert_eq!(versions.iter().map(|v| v.kind.as_str()).collect::<Vec<_>>(), ["create", "edit", "edit"]);
        assert_eq!(versions[2].deleted, vec!["glasses.note".to_owned()]);

        // Undo: the note column and its value come back.
        let (_, undone, m) = apps.undo(&s.id).unwrap();
        assert_eq!(undone.said, "Removed the note.");
        assert!(m.unwrap().restored.iter().any(|r| r.starts_with("glasses.note")));
        let (_, db) = apps.db(&s.id).unwrap();
        let rows = db.query("SELECT note FROM glasses", &Map::new()).unwrap();
        assert_eq!(rows[0]["note"], Value::from("after lunch"));
        assert_eq!(apps.versions(&s.id).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn removed_apps_wait_in_the_trash_with_their_data() {
        let (apps, dir) = store();
        let (s, _) = apps.install(WATER).map_err(|r| format!("{:?}", r.diagnostics)).unwrap();
        apps.run_action(&s.id, "drink", &args(&[("count", Value::from(3))])).unwrap();
        apps.remove(&s.id).unwrap();
        assert!(apps.list().is_empty());
        assert_eq!(apps.trashed().len(), 1);
        apps.restore_trashed(&s.id).unwrap();
        assert_eq!(apps.list().len(), 1);
        assert_eq!(total(&apps, &s.id), 3);
        assert!(apps.trashed().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_spec_with_errors_is_rejected() {
        let (apps, dir) = store();
        let r = apps.install("enee: 1\napp:\n  id: Bad Id\n").unwrap_err();
        assert!(!r.diagnostics.is_empty());
        assert!(apps.list().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
