//! Building and editing apps on the R1 (docs/specs/apps.md §4–5).
//!
//! One build is a no-tools Claude turn in the agent box (crate::agent): the R1 builder prompt as the
//! system prompt, and as the message either the create instructions plus a brief, or the edit
//! instructions plus a request and the app's current spec.
//! - **Create:** the reply's ```enee block is compiled and its checks run.
//! - **Edit:** the reply is one sentence for the user, then an ```enee-patch block (a full ```enee
//!   block for a rewrite) applied to the current spec. It must keep the app's id. A clean result is
//!   installed with crate::apps::Apps::install_edit, which records the version and deletes the
//!   data the edit removed (an undo brings it back).
//!
//! Problems go back for up to two repairs, each a fresh no-tools turn carrying the brief or request,
//! the spec so far and the problems (no session to resume). Besides the compiler's and the checks',
//! problems are what the R1 can't run (crate::broker::r1_problems) and, once the spec is clean, what
//! its web data turned out to be ([`probe`]): each http service's URL fetched once, as the app would,
//! so a wrong endpoint or a field the response doesn't have is fixed before you see the app.
//!
//! Ported from eneeware crates/enee-create/src/build.rs at 61bc7a7: extract_spec / fenced_block,
//! fenced, repair_message, problem, edit_message, the edit rules of `Turns`. Dropped: clarify, the
//! Jev draft, plugins, the streaming preview.

use crate::agent;
use crate::hub::{Event, Hub, Source};
use enee_spec::diag::{Diagnostic, Severity};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BUILDER: &str = include_str!("../../../prompts/builder.md");
const DESIGN: &str = include_str!("../../../prompts/design.md");
const CREATE: &str = include_str!("../../../prompts/create.md");
const EDIT: &str = include_str!("../../../prompts/edit.md");
/// Repairs after the first reply (eneeware's MAX_REPAIRS).
const MAX_REPAIRS: usize = 2;
const TURN_TIMEOUT: Duration = Duration::from_secs(180);
/// A big build (`big: true`, ADR-0041): an app that needs more work gets longer turns, more effort and a repair more.
const BIG_TURN_TIMEOUT: Duration = Duration::from_secs(600);

pub fn system_prompt() -> String {
    format!("{BUILDER}\n\n{DESIGN}")
}

pub fn create_message(brief: &str) -> String {
    format!("{CREATE}\n{}", brief.trim())
}

/// The spec in a reply: the first ```enee (or ```yaml / ```yml) block; an unterminated block runs to the end.
pub fn extract_spec(text: &str) -> Option<String> {
    ["enee", "yaml", "yml"].into_iter().find_map(|tag| fenced_block(text, tag))
}

/// The body of the first ```enee-patch block.
pub fn extract_patch(text: &str) -> Option<String> {
    fenced_block(text, "enee-patch")
}

/// The first block fenced as exactly `tag` (```` or ```), where the fence starts a line.
fn fenced_block(text: &str, tag: &str) -> Option<String> {
    for fence in ["````", "```"] {
        let open = format!("{fence}{tag}");
        let mut from = 0;
        while let Some(at) = text[from..].find(&open).map(|i| from + i) {
            from = at + open.len();
            // A fence starts a line (up to 3 spaces in), and the tag ends its info: ```enee isn't ```enee-patch.
            let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
            let starts_line = at - line_start <= 3 && text[line_start..at].bytes().all(|b| b == b' ');
            let after = &text[from..];
            if !starts_line || !after.starts_with(['\n', '\r', ' ']) {
                continue;
            }
            let body = &after[after.find('\n')? + 1..];
            let end = body.find(&format!("\n{fence}")).map_or(body.len(), |i| i + 1);
            return Some(body[..end].to_string());
        }
    }
    None
}

/// `spec` in an ```enee block (a longer fence when it has a ``` of its own).
pub fn fenced(spec: &str) -> String {
    let fence = if spec.contains("```") { "````" } else { "```" };
    format!("{fence}enee\n{}\n{fence}", spec.trim_end())
}

/// An edit's first message: the request and the app's current spec.
pub fn edit_message(request: &str, current: &str) -> String {
    format!("{EDIT}\nRequest: {}\n\nCurrent spec:\n{}", request.trim(), fenced(current))
}

/// An edit's repair turn: the edit again, the spec as the reply left it, and the problems.
pub fn edit_repair_message(request: &str, current: &str, spec_now: &str, problems: &[String]) -> String {
    format!(
        "{}\n\nYou already changed it. The spec now:\n{}\n\nThe host found problems in it. Fix them with an \
         ```enee-patch block of only what changes (against the spec now), or reply with the complete corrected \
         ```enee block. Nothing else.\n\n{}",
        edit_message(request, current),
        fenced(spec_now),
        problems.join("\n")
    )
}

/// The sentence an edit's reply starts with: the text before its first fence, on one line.
pub fn said(text: &str) -> String {
    let before = text.split("```").next().unwrap_or("");
    let one = before.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(240).collect()
}

/// A repair turn's message. Each turn is fresh, so it carries the brief and the spec too.
pub fn repair_message(brief: &str, problems: &[String], spec: &str) -> String {
    format!(
        "{}\n\nYou already wrote this spec for it:\n{}\n\nThe host found problems in it. Fix them with an ```enee-patch \
         block of only what changes, or reply with the complete corrected ```enee block. Nothing else.\n\n{}",
        create_message(brief),
        fenced(spec),
        problems.join("\n")
    )
}

/// A diagnostic as a repair turn shows it: code, path, line, message and hint.
pub fn problem(d: &Diagnostic) -> String {
    format!(
        "{} {}{}: {}{}",
        d.code,
        d.path,
        d.line.map(|l| format!(" (line {l})")).unwrap_or_default(),
        d.message,
        d.hint.as_ref().map(|h| format!(" Hint: {h}")).unwrap_or_default()
    )
}

/// Compiles `spec` and runs its checks: the problems a repair should fix (empty when it's good). Includes
/// what the R1 can't run yet.
pub fn problems(spec: &str) -> Vec<String> {
    let out = enee_spec::compile(spec, &enee_spec::Options::default());
    let mut problems: Vec<String> =
        out.diagnostics.iter().filter(|d| d.severity == Severity::Error).map(problem).collect();
    if let Some(app) = &out.app {
        problems.extend(crate::broker::r1_problems(app).iter().map(problem));
    }
    if problems.is_empty()
        && let Some(app) = &out.app
    {
        for r in enee_spec::run_checks(app).into_iter().filter(|r| !r.ok) {
            problems.push(format!("E_CHECK checks[{}]: {}", r.index, r.failures.join("; ")));
        }
    }
    if problems.is_empty() && !out.ok {
        problems.push("E_COMPILE the spec didn't compile".into());
    }
    problems
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Writing,
    /// Fetching the app's web data once, to check it ([`probe`]).
    Checking,
    Repairing,
    Installing,
    Done,
    Failed,
}

/// What a build's progress events carry (`Event::Build`).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Progress {
    /// `edit` for an edit (absent for a new app).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    /// An edit's sentence about what changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
    /// Tables and columns an edit deleted with their data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// The app's name, when it's done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problems: Option<Vec<String>>,
    /// The user stopped it (with `failed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

/// One build at a time: the R1 has the RAM for one agent CLI, and builds are short.
#[derive(Default)]
pub struct Builds {
    next: std::sync::atomic::AtomicU64,
    busy: std::sync::atomic::AtomicBool,
    /// Builds an agent started from a chat, while they run: chat → (build, the app an edit changes). A
    /// chat opened mid-build shows its card from this (the result is stored in the chat when it's done).
    in_chats: std::sync::Mutex<std::collections::HashMap<String, (u64, Option<String>)>>,
    /// The build running now, and the one the user asked to stop.
    current: std::sync::atomic::AtomicU64,
    stop_asked: std::sync::atomic::AtomicU64,
    stop: tokio::sync::Notify,
}

impl Builds {
    /// Claims the builder, or None if a build is running.
    pub fn start(&self) -> Option<u64> {
        use std::sync::atomic::Ordering;
        self.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).ok()?;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.current.store(id, Ordering::Release);
        Some(id)
    }

    /// Asks the running build (`id`, or whichever is running) to stop. False when there's none.
    pub fn stop(&self, id: Option<u64>) -> bool {
        use std::sync::atomic::Ordering;
        let current = self.current.load(Ordering::Acquire);
        if !self.busy.load(Ordering::Acquire) || current == 0 || id.is_some_and(|i| i != current) {
            return false;
        }
        self.stop_asked.store(current, Ordering::Release);
        self.stop.notify_waiters();
        true
    }

    /// The user asked build `id` to stop.
    pub fn stop_asked(&self, id: u64) -> bool {
        self.stop_asked.load(std::sync::atomic::Ordering::Acquire) == id
    }

    /// Resolves when build `id` is asked to stop.
    pub async fn stopped(&self, id: u64) {
        loop {
            let notified = self.stop.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.stop_asked(id) {
                return;
            }
            notified.await;
        }
    }

    pub fn finish(&self) {
        self.busy.store(false, std::sync::atomic::Ordering::Release);
    }

    /// A build started from `chat`.
    pub fn in_chat(&self, chat: &str, build: u64, edit_of: Option<String>) {
        self.in_chats.lock().unwrap().insert(chat.to_owned(), (build, edit_of));
    }

    /// The build from `chat` finished (its result is in the chat now).
    pub fn left_chat(&self, chat: &str, build: u64) {
        let mut chats = self.in_chats.lock().unwrap();
        if chats.get(chat).is_some_and(|(b, _)| *b == build) {
            chats.remove(chat);
        }
    }

    /// The build running from `chat`, if one is.
    pub fn of_chat(&self, chat: &str) -> Option<(u64, Option<String>)> {
        self.in_chats.lock().unwrap().get(chat).cloned()
    }
}

/// What a build makes.
pub enum Task {
    Create { brief: String },
    /// Change installed app `app` as `request` asks.
    Edit { app: String, request: String },
}

/// The outcome of a finished build.
pub struct Built {
    pub app: String,
    /// An edit's sentence.
    pub said: String,
    pub deleted: Vec<String>,
}

/// Runs one new-app build to the end. Returns the installed app's id.
pub async fn run(hub: Arc<Hub>, id: u64, brief: String, model: String, big: bool) -> Option<String> {
    build(hub, id, Task::Create { brief }, model, big).await.map(|b| b.app)
}

/// Runs one build (create or edit) to the end, reporting progress as `Event::Build`.
pub async fn build(hub: Arc<Hub>, id: u64, task: Task, model: String, big: bool) -> Option<Built> {
    let started = Instant::now();
    let (repairs, turn_timeout, effort) = if big { (MAX_REPAIRS + 1, BIG_TURN_TIMEOUT, "medium") } else { (MAX_REPAIRS, TURN_TIMEOUT, "low") };
    let kind = matches!(task, Task::Edit { .. }).then_some("edit");
    let emit = |stage: Stage, p: Progress| hub.emit(Event::Build { id, stage, progress: Progress { kind, ..p } }, Source::Api);
    // An edit's starting point: the app as it is now.
    let current = match &task {
        Task::Edit { app, .. } => match hub.apps.get(app) {
            Some(i) => Some(i),
            None => {
                emit(Stage::Failed, Progress { error: Some(format!("there's no app `{app}`")), ..Default::default() });
                return None;
            }
        },
        Task::Create { .. } => None,
    };
    let mut cost = 0.0;
    let mut spec: Option<String> = None;
    let mut said_text = String::new();
    let mut last_problems: Vec<String> = Vec::new();

    for attempt in 0..=repairs {
        let stage = if attempt == 0 { Stage::Writing } else { Stage::Repairing };
        let message = match (&task, &current) {
            (Task::Create { brief }, _) => match &spec {
                None => create_message(brief),
                Some(s) => repair_message(brief, &last_problems, s),
            },
            (Task::Edit { request, .. }, Some(c)) => {
                if attempt == 0 {
                    edit_message(request, &c.source)
                } else {
                    edit_repair_message(request, &c.source, spec.as_deref().unwrap_or(&c.source), &last_problems)
                }
            }
            (Task::Edit { .. }, None) => unreachable!("an edit has its current app"),
        };
        emit(stage.clone(), Progress { problems: (attempt > 0).then(|| last_problems.clone()), ..Default::default() });

        // Stream: report which top-level section the model is writing, at most every 300 ms.
        let mut text = String::new();
        let mut last_report = Instant::now();
        let mut section = String::new();
        let mut on_text = |delta: &str| {
            text.push_str(delta);
            if last_report.elapsed() < Duration::from_millis(300) {
                return;
            }
            last_report = Instant::now();
            if let Some(s) = current_section(&text)
                && s != section
            {
                section = s;
            }
            emit(stage.clone(), Progress { section: Some(section.clone()), chars: Some(text.len()), ..Default::default() });
        };
        let req = agent::Request {
            model: model.clone(),
            effort: Some(effort.into()),
            system_prompt: system_prompt(),
            prompt: message,
            timeout: turn_timeout,
        };
        // A place in the box's process budget, shared with the agents (crate::session). Asked to stop
        // meanwhile, it stops here: dropping the turn kills its CLI (crate::agent).
        let turn = async {
            let room = hub.sessions.room(&hub, || {}).await;
            let outcome = if agent::is_codex(&req.model) {
                agent::codex_no_tools(&req, &mut on_text).await
            } else {
                agent::claude_no_tools(&req, &mut on_text).await
            };
            drop(room);
            outcome
        };
        let outcome = tokio::select! {
            o = turn => o,
            () = hub.builds.stopped(id) => {
                emit(Stage::Failed, Progress { error: Some("stopped".into()), stopped: Some(true), cost_usd: Some(cost), seconds: Some(secs(started)), ..Default::default() });
                return None;
            }
        };
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                emit(Stage::Failed, Progress { error: Some(format!("{e:#}")), ..Default::default() });
                return None;
            }
        };
        cost += outcome.cost_usd.unwrap_or(0.0);

        if current.is_some() && said_text.is_empty() {
            said_text = said(&outcome.text);
        }
        // A patch applies to the spec so far: the last reply's, or an edit's current app.
        let base = spec.as_deref().or(current.as_deref().map(|c| c.source.as_str()));
        let next = match (base, extract_patch(&outcome.text)) {
            (Some(base), Some(patch)) => enee_spec::patch::apply(base, &patch).map_err(|e| vec![format!("E_PATCH {e}")]),
            _ => extract_spec(&outcome.text).ok_or_else(|| vec!["E_NO_SPEC the reply had no ```enee block".to_owned()]),
        };
        let candidate = match next {
            Ok(s) => s,
            Err(p) => {
                // No usable spec in the reply: repair the last one, or (none yet) start over from the brief.
                last_problems = p;
                continue;
            }
        };
        last_problems = problems(&candidate);
        if let Some(c) = &current {
            let id = &c.app.app.id;
            if let Ok(app) = crate::apps::compile(&candidate)
                && &app.app.id != id
            {
                last_problems.push(format!("E_ID app.id must stay `{id}` (an edit keeps the app's id)"));
            }
        }
        // Clean, with a glance card: its shape, filled with stand-ins (ADR-0042). Repaired while repairs are left;
        // after that the app installs, and its widget says it has nothing to show.
        if last_problems.is_empty()
            && let Ok(app) = crate::apps::compile(&candidate)
        {
            let found = crate::glance::shape_problems(&app);
            if !found.is_empty() {
                tracing::info!(build = id, problems = ?found, "the app's glance card doesn't fit the catalog");
                if attempt < repairs {
                    last_problems = found;
                }
            }
        }
        // Clean: check its web data. A failure there is repaired while repairs are left; after the last
        // one the app installs anyway (the network may be what failed), and shows the error itself.
        if last_problems.is_empty() && has_http(&candidate) {
            emit(Stage::Checking, Progress::default());
            let found = probe(&hub, &candidate).await;
            if !found.is_empty() {
                tracing::info!(build = id, problems = ?found, "the app's web data didn't check out");
                if attempt < repairs {
                    last_problems = found;
                }
            }
        }
        // Clean, with logic: a trial run of its script in the box (ADR-0041). Like the web data's check, a problem
        // there is repaired while repairs are left; the stand-in can't know everything a real run would.
        if last_problems.is_empty()
            && let Ok(app) = crate::apps::compile(&candidate)
            && app.logic.is_some()
        {
            emit(Stage::Checking, Progress::default());
            let found = crate::logic_check::run(&app).await;
            if !found.is_empty() {
                tracing::info!(build = id, problems = ?found, "the app's logic didn't run cleanly");
                if attempt < repairs {
                    last_problems = found;
                }
            }
        }
        spec = Some(candidate);
        if last_problems.is_empty() {
            break;
        }
    }

    let fail = |error: String, problems: Vec<String>| {
        emit(
            Stage::Failed,
            Progress { error: Some(error), problems: Some(problems), cost_usd: Some(cost), seconds: Some(secs(started)), ..Default::default() },
        );
        None
    };
    let Some(spec) = spec else { return fail("the agent never produced a spec".into(), last_problems) };
    if !last_problems.is_empty() {
        return fail(format!("still broken after {repairs} repairs"), last_problems);
    }
    if let Some(c) = &current
        && spec.trim() == c.source.trim()
    {
        return fail("the reply didn't change anything".into(), vec![]);
    }

    if hub.builds.stop_asked(id) {
        emit(Stage::Failed, Progress { error: Some("stopped".into()), stopped: Some(true), cost_usd: Some(cost), seconds: Some(secs(started)), ..Default::default() });
        return None;
    }
    emit(Stage::Installing, Progress::default());
    let h = hub.clone();
    let said_for_install = said_text.clone();
    let installed = tokio::task::spawn_blocking(move || install(&h, &task, &spec, &said_for_install)).await;
    match installed {
        Ok(Ok((summary, deleted))) => {
            hub.emit(Event::AppInstalled { app: summary.id.clone() }, Source::Api);
            emit(
                Stage::Done,
                Progress {
                    app: Some(summary.id.clone()),
                    name: Some(summary.name.clone()),
                    said: (!said_text.is_empty()).then(|| said_text.clone()),
                    deleted: (!deleted.is_empty()).then(|| deleted.clone()),
                    cost_usd: Some(cost),
                    seconds: Some(secs(started)),
                    ..Default::default()
                },
            );
            Some(Built { app: summary.id, said: said_text, deleted })
        }
        Ok(Err(e)) => fail(format!("{e:#}"), vec![]),
        Err(e) => fail(e.to_string(), vec![]),
    }
}

/// Installs a finished build and records its version. Returns the summary and what an edit deleted.
fn install(hub: &Hub, task: &Task, spec: &str, said: &str) -> anyhow::Result<(crate::apps::Summary, Vec<String>)> {
    match task {
        Task::Create { brief } => {
            // A new app never replaces another: an id that's taken (installed, or in the trash) gets a number.
            let fresh = fresh_id(hub, spec);
            let spec = fresh.as_deref().unwrap_or(spec);
            let (summary, _warnings) = hub
                .apps
                .install(spec)
                .map_err(|r| anyhow::anyhow!("install rejected the spec: {}", r.diagnostics.iter().map(problem).collect::<Vec<_>>().join("; ")))?;
            if let Err(e) = hub.apps.record(&summary.id, "create", brief, "", Vec::new(), spec) {
                tracing::warn!(app = %summary.id, error = %e, "couldn't record the version");
            }
            Ok((summary, Vec::new()))
        }
        Task::Edit { app, request } => {
            let (summary, migration) = hub.apps.install_edit(app, spec, "edit", request, said)?;
            Ok((summary, migration.map(|m| m.dropped).unwrap_or_default()))
        }
    }
}

/// The spec has web data to check.
fn has_http(spec: &str) -> bool {
    crate::apps::compile(spec).is_ok_and(|a| a.services.iter().any(|s| s.uses.starts_with("http.")))
}

/// Fetches each http service once, as the app would with its starting state and setting defaults, and
/// returns problems for a repair: a URL that doesn't answer with what the service expects, or bindings to
/// fields its response doesn't have (with an outline of what it does have). A URL that times out isn't a
/// problem (the network may be slow); one still waiting for a value the user types is skipped.
///
/// These are the build's own requests, made before the user decides anything: only GETs of the URLs the
/// spec declares, to the hosts it declares, with no user data (crate::fetch's rules).
pub async fn probe(hub: &Arc<Hub>, spec: &str) -> Vec<String> {
    let Ok(app) = crate::apps::compile(spec) else { return Vec::new() };
    let hosts: Vec<String> = app.permissions.iter().filter(|p| p.id == "net.http").flat_map(crate::broker::hosts).collect();
    let vars = crate::services::vars_from(&app, &serde_json::Map::new(), &serde_json::Map::new());
    let mut out = Vec::new();
    for s in &app.services {
        let Some(format) = crate::fetch::Format::of(&s.uses) else { continue };
        let Some(template) = s.options.get("url").and_then(serde_json::Value::as_str) else { continue };
        let Some(url) = crate::fetch::fill_url(template, &|p| vars.get(p).cloned()) else { continue };
        let fetcher = hub.services.fetcher();
        let (u, h) = (url.clone(), hosts.clone());
        let fetched = tokio::task::spawn_blocking(move || crate::services::fetch(&fetcher, format, &u, &h).0).await;
        let path = format!("services.{}.url", s.name);
        let value = match fetched {
            Ok(Ok(v)) => v,
            Ok(Err(e)) if e.to_string().starts_with("no answer") => continue,
            Ok(Err(e)) => {
                out.push(format!(
                    "E_FETCH {path}: fetching {url} failed: {e}. Hint: use a public https API that needs no key or \
                     sign-up, and check the URL's path and query against what it documents."
                ));
                continue;
            }
            Err(_) => continue,
        };
        if format == crate::fetch::Format::Text {
            continue;
        }
        let sample = crate::services::sample_of(value, "t");
        let missing: Vec<String> =
            bound_paths(spec, &s.name).into_iter().filter(|p| resolve(&sample, p).is_none()).map(|p| p.join(".")).collect();
        if !missing.is_empty() {
            out.push(format!(
                "E_FETCH services.{}: {url} answered, but without {}. Bind fields it has. Its response (a sample, \
                 as `services.{}`):\n{}",
                s.name,
                missing.iter().map(|m| format!("`services.{}.{m}`", s.name)).collect::<Vec<_>>().join(", "),
                s.name,
                outline(&sample)
            ));
        }
    }
    out
}

/// The field paths the spec binds under `services.<name>`, stopping at a filter or the end of the
/// binding (`{{services.w.current_condition.0.temp_C | round}}` → `current_condition.0.temp_C`). Paths into
/// `history`, `error` and `t` are left out: they're the host's.
fn bound_paths(spec: &str, name: &str) -> Vec<Vec<String>> {
    let needle = format!("services.{name}.");
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut rest = spec;
    while let Some(at) = rest.find(&needle) {
        let before = rest[..at].chars().next_back();
        rest = &rest[at + needle.len()..];
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).unwrap_or(rest.len());
        let path: Vec<String> = rest[..end].split('.').filter(|p| !p.is_empty()).map(str::to_owned).collect();
        if path.is_empty() || ["history", "error", "t"].contains(&path[0].as_str()) || out.contains(&path) {
            continue;
        }
        out.push(path);
    }
    out
}

/// The value at `path` (object keys and array indexes), if it's there.
fn resolve<'a>(v: &'a serde_json::Value, path: &[String]) -> Option<&'a serde_json::Value> {
    path.iter().try_fold(v, |v, seg| match v {
        serde_json::Value::Object(m) => m.get(seg),
        serde_json::Value::Array(a) => a.get(seg.parse::<usize>().ok()?),
        _ => None,
    })
}

/// A short outline of a JSON value for the model: keys with an example value, arrays as their length and
/// first item, at most 4 levels and 60 lines.
fn outline(v: &serde_json::Value) -> String {
    fn walk(v: &serde_json::Value, depth: usize, indent: &str, lines: &mut Vec<String>) {
        use serde_json::Value;
        let Value::Object(m) = v else { return };
        for (k, child) in m {
            if lines.len() >= 60 {
                return;
            }
            match child {
                Value::Object(_) if depth < 4 => {
                    lines.push(format!("{indent}{k}:"));
                    walk(child, depth + 1, &format!("{indent}  "), lines);
                }
                Value::Array(a) => {
                    lines.push(format!("{indent}{k}: [{} items]{}", a.len(), if a.is_empty() { "" } else { ", the first:" }));
                    match a.first() {
                        Some(first @ Value::Object(_)) if depth < 4 => walk(first, depth + 1, &format!("{indent}  0."), lines),
                        Some(first) if !first.is_object() && !first.is_array() => {
                            lines.push(format!("{indent}  0: {}", example(first)));
                        }
                        _ => {}
                    }
                }
                other => lines.push(format!("{indent}{k}: {}", example(other))),
            }
        }
    }
    fn example(v: &serde_json::Value) -> String {
        let s = v.to_string();
        if s.chars().count() > 40 { format!("{}…", s.chars().take(40).collect::<String>()) } else { s }
    }
    let mut lines = Vec::new();
    walk(v, 0, "", &mut lines);
    lines.join("\n")
}

fn secs(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 10.0).round() / 10.0
}

/// The last top-level YAML key in the streamed text (`queries`, `views`, …), for progress.
fn current_section(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .find_map(|l| {
            let key = l.strip_suffix(':').or_else(|| l.split_once(": ").map(|(k, _)| k))?;
            (!l.starts_with([' ', '-', '`']) && !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
                .then(|| key.to_owned())
        })
}

/// The spec with a free `app.id`, if its own is taken: `radio` → `radio-2`, `radio-3`… (None: it's free).
fn fresh_id(hub: &Hub, spec: &str) -> Option<String> {
    let id = crate::apps::compile(spec).ok()?.app.id;
    let taken = |x: &str| hub.apps.get(x).is_some() || hub.apps.trashed().iter().any(|(t, _, _)| t == x);
    if !taken(&id) {
        return None;
    }
    let free = (2..100).map(|n| format!("{id}-{n}")).find(|c| c.len() <= 40 && !taken(c))?;
    enee_spec::patch::apply(spec, &format!("app: {{ id: {free} }}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builder_example_compiles_and_passes_its_checks() {
        let example = include_str!("../../../prompts/example.enee.yaml");
        assert_eq!(problems(example), Vec::<String>::new());
        // And it's the one inside builder.md.
        let in_prompt = extract_spec(BUILDER.split("## Example").nth(1).unwrap()).unwrap();
        assert_eq!(in_prompt.trim(), example.trim());
    }

    #[test]
    fn extract_spec_prefers_enee_and_respects_line_starts() {
        let t = "Here:\n```enee\nenee: 1\n```\nmention ```enee-patch inline\n";
        assert_eq!(extract_spec(t).as_deref(), Some("enee: 1\n"));
        assert_eq!(extract_patch(t), None);
        assert_eq!(extract_spec("```yaml\napp: x\n").as_deref(), Some("app: x\n"));
    }

    #[test]
    fn web_data_checks_find_bound_paths_and_outline_responses() {
        let spec = "views:\n  main:\n    - stat: { value: \"{{services.w.current_condition.0.temp_C | round}}\" }\n    - text: \"{{services.w.error}}\"\n    - list: { data: services.w.weather }\n    - text: \"{{xservices.w.nope}}\"\n";
        let paths = bound_paths(spec, "w");
        assert_eq!(paths, vec![vec!["current_condition".to_string(), "0".into(), "temp_C".into()], vec!["weather".to_string()]]);
        let sample = serde_json::json!({ "current_condition": [ { "temp_C": "12", "weatherDesc": [ { "value": "Sunny" } ] } ], "t": "x" });
        assert!(resolve(&sample, &paths[0]).is_some());
        assert!(resolve(&sample, &paths[1]).is_none());
        let o = outline(&sample);
        assert!(o.contains("current_condition: [1 items], the first:") && o.contains("0.temp_C: \"12\""), "{o}");
    }

    #[test]
    fn builds_cant_use_what_the_r1_doesnt_run() {
        let news = include_str!("../../../fixtures/apps/news.enee.yaml").replace("open.url", "apps.launch").replace("open_url:", "launch:");
        let p = problems(&news);
        assert!(p.iter().any(|p| p.starts_with("E_R1 permissions.apps.launch")), "{p:?}");
    }

    #[test]
    fn current_section_tracks_top_level_keys() {
        assert_eq!(current_section("enee: 1\napp: { id: x }\nqueries:\n  today: SELECT 1\n").as_deref(), Some("queries"));
        assert_eq!(current_section("```enee\nenee: 1\n").as_deref(), Some("enee"));
    }
}
