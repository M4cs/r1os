//! Live data for apps (docs/specs/apps.md §6, ADR-0023): an app's `http.json`, `http.rss` and `http.fetch`
//! services, fetched by r1d (crate::fetch) and pushed to its page as `/services/<name>` values
//! (`app_services` events; the runtime's host-written paths).
//!
//! - **While the app is open.** The native shell says when an app's screen opens and closes
//!   (`POST /api/apps/{id}/open|close`); going to sleep closes it. With no apps open the loop waits on a
//!   notify with no timer (battery first, architecture.md §7).
//! - **In the background, for apps allowed `schedule`** and apps whose card is on the glance screen (ADR-0042):
//!   their services keep running while they're closed, at most every 15 minutes, so the app opens fresh, a feed
//!   fills its table, its triggers see new data, and its widget is current. Only while the display is on (the
//!   shell reports it, `POST /api/display`): nothing fetches with the screen off, and nothing wakes the R1 to
//!   fetch. When the display comes back on, a value older than 15 minutes by the clock is fetched at once (the
//!   monotonic clock the intervals use may have stopped while the R1 slept).
//! - **Only what the user allowed:** a service runs if its app holds `net.http` for its URL's host
//!   (crate::broker). A decision made while the app is open applies at once.
//! - **Kept between opens.** Each service's last value (and its `keep` history) is saved in
//!   `data/<id>/services.json`. A reopened app shows it at once and fetches only when it's due (`every`,
//!   5 minutes by default, at least 1 minute).
//! - **URLs that bind values** (`{{state.ticker}}`, `{{settings.town}}`): the page reports those values
//!   when they change (`POST /api/apps/{id}/services/vars`), and r1d builds the URL, percent-encoding each
//!   one (crate::fetch::fill_url). Until the page reports (up to 2 s), the kept state and the settings fill
//!   it. A changed URL fetches again after at least 2 s, and its old value is cleared until the new one
//!   arrives.
//! - **Errors** keep the last good value and add `error` ("HTTP 404", "no answer in 10 s").
//! - **Feeds with `into`** add their new items to the app's table by `guid`, which refreshes the page
//!   (`app_changed`).
//!
//! Ported from eneeware apps/desktop/src-tauri/src/services.rs at 61bc7a7: the http half (`Http`,
//! `set_vars`, `fetched`, `store_items`, the samples), on tokio instead of a thread, without the system
//! samplers, the clipboard and background running (`schedule`).

use crate::apps::Installed;
use crate::feed::{self, Feed};
use crate::fetch::{FetchError, Fetcher, Format};
use crate::hub::{Event, Hub, Source};
use enee_spec::compiled::{CompiledApp, Service};
use enee_spec::eval;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How often an http service fetches without `every`.
const HTTP_EVERY: Duration = Duration::from_secs(300);
/// A closed app's services, running in the background (`schedule`), fetch at most this often.
const BACKGROUND_EVERY: Duration = Duration::from_secs(15 * 60);
/// The least time between a service's fetches when its URL changes (typing a ticker).
const HTTP_GAP: Duration = Duration::from_secs(2);
/// How long the first fetch waits for the page to report the values its URL binds.
const VARS_WAIT: Duration = Duration::from_secs(2);
/// After a failed fetch, the next try comes this soon (or at `every`, if that's sooner).
const ERROR_RETRY: Duration = Duration::from_secs(60);
const MAX_HISTORY: usize = 1440;
/// A sample larger than this isn't kept in a history.
const MAX_HISTORY_SAMPLE: usize = 16 * 1024;
/// The most history saved per service.
const MAX_SAVED_HISTORY: usize = 2 * 1024 * 1024;
/// A feed's value lists this many items.
const FEED_ITEMS: usize = 50;
/// An `http.fetch` value's text is cut to this many bytes.
const MAX_TEXT: usize = 64 * 1024;
/// The columns a feed fills in an `into` table, when the table has them.
const FEED_COLUMNS: [&str; 6] = ["guid", "title", "link", "summary", "author", "published"];
const SAVED: &str = "services.json";

/// One `http.*` service of an open app.
struct Sub {
    name: String,
    format: Format,
    /// `http.rss`: the table its items go into.
    into: Option<String>,
    template: String,
    /// The app's `net.http` hosts (lowercase).
    hosts: Arc<[String]>,
    /// The URL with the current values; `None` while one is missing or empty.
    url: Option<String>,
    every: Duration,
    keep_ms: Option<i64>,
    next: Instant,
    /// Waiting for the page's values before the first fetch.
    waits_for_vars: bool,
    in_flight: bool,
    last_fetch: Option<Instant>,
}

/// An app whose services run: the compiled app they were set up from (an edit sets them up again), the
/// services, and whether it's on screen (else it runs in the background, `schedule`).
struct Running {
    installed: Arc<Installed>,
    subs: Vec<Sub>,
    shown: bool,
}

/// A service's last value and history, kept between opens (and saved).
#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    url: String,
    value: Value,
    #[serde(default)]
    history: VecDeque<Value>,
    /// When it was last fetched (ms since the epoch).
    at_ms: i64,
}

#[derive(Default)]
struct Inner {
    open: HashMap<String, Running>,
    /// The values the apps' URLs bind (`/state/…`, `/settings/…`), and whether the page reported them yet.
    vars: HashMap<String, Map<String, Value>>,
    reported: HashSet<String>,
    /// By app, then service name. Loaded from `services.json` on first open.
    cache: HashMap<String, HashMap<String, Cached>>,
    /// Which apps run in the background needs working out again (an install, a permission, the shell came).
    background_dirty: bool,
}

pub struct Services {
    fetcher: Arc<Fetcher>,
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
    /// The display is on (the shell reports it). Background services wait while it's off.
    display_on: std::sync::atomic::AtomicBool,
    /// The display came back on: background values older than 15 minutes by the clock are due.
    woke: std::sync::atomic::AtomicBool,
    /// `<data>/data`: each app's folder, where its `services.json` goes.
    data: PathBuf,
}

/// One fetch to make.
struct Job {
    app: String,
    name: String,
    url: String,
    format: Format,
    hosts: Arc<[String]>,
    into: Option<String>,
    keep_ms: Option<i64>,
}

impl Services {
    pub fn new(root: &std::path::Path) -> Self {
        Services {
            fetcher: Arc::new(Fetcher::new()),
            inner: Mutex::new(Inner { background_dirty: true, ..Inner::default() }),
            wake: tokio::sync::Notify::new(),
            display_on: std::sync::atomic::AtomicBool::new(true),
            woke: std::sync::atomic::AtomicBool::new(false),
            data: root.join("data"),
        }
    }

    /// The shared fetcher (build checks use it too, crate::create).
    pub fn fetcher(&self) -> Arc<Fetcher> {
        self.fetcher.clone()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts the loop that fetches what's due (once, at startup).
    pub fn start(hub: &Arc<Hub>) {
        tokio::spawn(run(hub.clone()));
    }

    /// The app's screen opened: its services start (those it's allowed), and its kept values are sent at once.
    pub fn open(&self, hub: &Hub, id: &str) {
        let Some(installed) = hub.apps.get(id) else { return };
        let values = {
            let mut inner = self.lock();
            self.load(&mut inner, id);
            if !inner.vars.contains_key(id) {
                let vars = default_vars(hub, &installed.app);
                inner.vars.insert(id.to_owned(), vars);
            }
            let subs = self.subscriptions(hub, &inner, &installed.app);
            inner.open.insert(id.to_owned(), Running { installed, subs, shown: true });
            current_values(&inner, id)
        };
        self.wake.notify_one();
        if !values.is_empty() {
            emit(hub, id, values);
        }
    }

    /// The app's screen closed: its services stop, or go on in the background (`schedule`). Their values stay
    /// for the next open.
    pub fn close(&self, id: &str) {
        let mut inner = self.lock();
        inner.open.remove(id);
        inner.vars.remove(id);
        inner.reported.remove(id);
        inner.background_dirty = true;
        drop(inner);
        self.wake.notify_one();
    }

    /// Which apps run in the background may have changed (an install or removal, a permission decision).
    pub fn background_changed(&self) {
        self.lock().background_dirty = true;
        self.wake.notify_one();
    }

    /// Whether the display is on (the shell says): live meters stop sending while it's off.
    pub fn display_on(&self) -> bool {
        self.display_on.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The display went on or off (the shell says): background services run only while it's on.
    pub fn set_display(&self, on: bool) {
        let was = self.display_on.swap(on, std::sync::atomic::Ordering::Relaxed);
        if on && !was {
            self.woke.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.wake.notify_one();
    }

    /// Starts the background services of closed apps allowed `schedule` or on the glance screen, and stops those no
    /// longer so.
    fn sync_background(&self, hub: &Hub) {
        let wanted: Vec<Arc<Installed>> = hub
            .apps
            .list()
            .into_iter()
            .filter_map(|s| hub.apps.get(&s.id))
            .filter(|i| {
                i.app.services.iter().any(|s| Format::of(&s.uses).is_some())
                    && (crate::broker::allows(&hub.core, &i.app, "schedule") || hub.glance.listed(hub, &i.app.app.id))
            })
            .collect();
        let mut inner = self.lock();
        inner.background_dirty = false;
        let ids: HashSet<String> = wanted.iter().map(|i| i.app.app.id.clone()).collect();
        inner.open.retain(|id, r| r.shown || ids.contains(id));
        for installed in wanted {
            let id = installed.app.app.id.clone();
            if inner.open.contains_key(&id) {
                continue;
            }
            self.load(&mut inner, &id);
            let vars = default_vars(hub, &installed.app);
            inner.vars.insert(id.clone(), vars);
            // Nobody reports values in the background: the kept state and settings fill the URLs.
            inner.reported.insert(id.clone());
            let subs = self.subscriptions(hub, &inner, &installed.app);
            if !subs.is_empty() {
                tracing::info!(app = %id, services = subs.len(), "services: running in the background");
                inner.open.insert(id, Running { installed, subs, shown: false });
            }
        }
    }

    /// The app is open: set its services up again (a permission decision, an edit), keeping its values.
    pub fn refresh(&self, hub: &Hub, id: &str) {
        let open = self.lock().open.contains_key(id);
        if open {
            self.open(hub, id);
        }
    }

    /// The app was removed: forget its values (its folder went to the trash with them).
    pub fn forget(&self, id: &str) {
        let mut inner = self.lock();
        inner.open.remove(id);
        inner.vars.remove(id);
        inner.reported.remove(id);
        inner.cache.remove(id);
    }

    /// Each service's current value (with its history), for a page that's just loaded.
    pub fn values(&self, id: &str) -> Map<String, Value> {
        let mut inner = self.lock();
        self.load(&mut inner, id);
        current_values(&inner, id)
    }

    /// The page reported the values its service URLs bind. A URL that changed fetches again.
    pub fn set_vars(&self, hub: &Hub, id: &str, values: Map<String, Value>) {
        let mut cleared = Map::new();
        {
            let mut inner = self.lock();
            let Inner { open, vars, reported, cache, .. } = &mut *inner;
            let Some(running) = open.get_mut(id) else { return };
            reported.insert(id.to_owned());
            let vars = vars.entry(id.to_owned()).or_default();
            // Only the paths the app's URLs bind.
            let deps: HashSet<&str> = running.installed.app.services.iter().flat_map(|s| s.deps.iter().map(String::as_str)).collect();
            for (k, v) in values {
                if deps.contains(k.as_str()) {
                    vars.insert(k, v);
                }
            }
            let now = Instant::now();
            for sub in &mut running.subs {
                let url = crate::fetch::fill_url(&sub.template, &|p: &str| vars.get(p).cloned());
                let waited = std::mem::take(&mut sub.waits_for_vars);
                if url == sub.url {
                    // The kept values were right: if it was only waiting for these, it fetches now.
                    if waited && !cached_fresh(cache.get(id).and_then(|c| c.get(&sub.name)), sub) {
                        sub.next = now;
                    }
                    continue;
                }
                sub.url = url;
                sub.next = sub.last_fetch.map_or(now, |at| (at + HTTP_GAP).max(now));
                // The old value was for another URL: the app shows an empty one until the new one arrives.
                cleared.insert(format!("/services/{}", sub.name), json!({ "t": eval::iso_utc(now_ms()) }));
            }
        }
        self.wake.notify_one();
        if !cleared.is_empty() {
            emit(hub, id, cleared);
        }
    }

    /// The app's http services it's allowed to run, due now or when their kept value gets old.
    fn subscriptions(&self, hub: &Hub, inner: &Inner, app: &CompiledApp) -> Vec<Sub> {
        let id = &app.app.id;
        let vars = inner.vars.get(id).cloned().unwrap_or_default();
        let reported = inner.reported.contains(id);
        let hosts = http_hosts(app);
        let now = Instant::now();
        let mut subs = Vec::new();
        for s in &app.services {
            let Some(format) = Format::of(&s.uses) else { continue };
            let Some(template) = s.options.get("url").and_then(Value::as_str) else { continue };
            if !crate::broker::allows_host(&hub.core, app, &template_host(template)) {
                continue;
            }
            let url = crate::fetch::fill_url(template, &|p: &str| vars.get(p).cloned());
            let into = s.options.get("into").and_then(Value::as_str).filter(|_| format == Format::Feed).map(str::to_owned);
            let waits_for_vars = !reported && !s.deps.is_empty();
            let mut sub = Sub {
                name: s.name.clone(),
                format,
                into,
                template: template.to_owned(),
                hosts: hosts.clone(),
                url,
                every: s.every_ms.map_or(HTTP_EVERY, Duration::from_millis),
                keep_ms: s.keep_ms.map(|ms| ms as i64),
                next: now,
                waits_for_vars,
                in_flight: false,
                last_fetch: None,
            };
            let cached = inner.cache.get(id).and_then(|c| c.get(&s.name));
            sub.next = match cached {
                // Fetched recently for this URL: due when it gets old.
                Some(c) if Some(&c.url) == sub.url.as_ref() && cached_fresh(Some(c), &sub) => {
                    now + Duration::from_millis(u64::try_from(c.at_ms + sub.every.as_millis() as i64 - now_ms()).unwrap_or(0))
                }
                _ if waits_for_vars => now + VARS_WAIT,
                _ => now,
            };
            subs.push(sub);
        }
        subs
    }

    /// Loads the app's saved values, once.
    fn load(&self, inner: &mut Inner, id: &str) {
        if inner.cache.contains_key(id) {
            return;
        }
        let saved: HashMap<String, Cached> = std::fs::read_to_string(self.data.join(id).join(SAVED))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        inner.cache.insert(id.to_owned(), saved);
    }

    /// Saves the app's values (histories trimmed to what fits).
    fn save(&self, id: &str, cache: &HashMap<String, Cached>) {
        let trimmed: HashMap<&String, Cached> = cache
            .iter()
            .map(|(name, c)| (name, Cached { history: fit_history(&c.history, MAX_SAVED_HISTORY).into(), ..c.clone() }))
            .collect();
        let dir = self.data.join(id);
        let write = || -> anyhow::Result<()> {
            std::fs::create_dir_all(&dir)?;
            let tmp = dir.join(format!("{SAVED}.tmp"));
            std::fs::write(&tmp, serde_json::to_vec(&trimmed)?)?;
            std::fs::rename(&tmp, dir.join(SAVED))?;
            Ok(())
        };
        if let Err(e) = write() {
            tracing::warn!(app = id, error = %e, "couldn't save the app's service values");
        }
    }

    /// The fetches due now (marked in flight), and how long until the next one.
    fn due(&self, hub: &Hub) -> (Vec<Job>, Option<Duration>) {
        // Nobody to show it to (the host app isn't connected): stop everything until an app opens again.
        if hub.clients() == 0 {
            let mut inner = self.lock();
            if !inner.open.is_empty() {
                tracing::info!("services: no shell connected; closing the open apps' services");
                inner.open.clear();
            }
            // When it comes back, the background ones start again.
            inner.background_dirty = true;
            return (Vec::new(), None);
        }
        if self.lock().background_dirty {
            self.sync_background(hub);
        }
        let display_on = self.display_on.load(std::sync::atomic::Ordering::Relaxed);
        // An app edited or removed while open: set it up again, or drop it.
        let stale: Vec<String> = {
            let inner = self.lock();
            inner
                .open
                .iter()
                .filter(|(id, r)| hub.apps.get(id).is_none_or(|now| !Arc::ptr_eq(&now, &r.installed)))
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in stale {
            let shown = self.lock().open.get(&id).is_some_and(|r| r.shown);
            if hub.apps.get(&id).is_some() && shown {
                self.open(hub, &id);
            } else {
                // Gone, or in the background: worked out again with the others.
                self.lock().open.remove(&id);
                if hub.apps.get(&id).is_none() {
                    self.forget(&id);
                }
                self.lock().background_dirty = true;
            }
        }
        if self.lock().background_dirty {
            self.sync_background(hub);
        }
        let woke = display_on && self.woke.swap(false, std::sync::atomic::Ordering::Relaxed);
        let mut inner = self.lock();
        let Inner { open, cache, .. } = &mut *inner;
        let now = Instant::now();
        let mut jobs = Vec::new();
        let mut wait: Option<Duration> = None;
        for (id, running) in open.iter_mut() {
            // In the background: only with the display on, and at most every 15 minutes.
            if !running.shown && !display_on {
                continue;
            }
            let every_floor = if running.shown { Duration::ZERO } else { BACKGROUND_EVERY };
            for sub in &mut running.subs {
                if sub.in_flight {
                    continue;
                }
                // Back from sleep: judged by when it was fetched, as the monotonic clock may have stopped meanwhile.
                if woke && !running.shown {
                    let at = cache.get(id).and_then(|c| c.get(&sub.name)).map_or(0, |c| c.at_ms);
                    if now_ms() - at >= BACKGROUND_EVERY.as_millis() as i64 {
                        sub.last_fetch = None;
                        sub.next = now;
                    }
                }
                let Some(url) = sub.url.clone() else { continue };
                if !running.shown && sub.next < sub.last_fetch.map_or(now, |at| at + every_floor) {
                    sub.next = sub.last_fetch.map_or(now, |at| at + every_floor);
                }
                if sub.next <= now {
                    sub.in_flight = true;
                    sub.waits_for_vars = false;
                    sub.last_fetch = Some(now);
                    sub.next = now + sub.every.max(every_floor);
                    jobs.push(Job {
                        app: id.clone(),
                        name: sub.name.clone(),
                        url,
                        format: sub.format,
                        hosts: sub.hosts.clone(),
                        into: sub.into.clone(),
                        keep_ms: sub.keep_ms,
                    });
                } else {
                    let d = sub.next - now;
                    wait = Some(wait.map_or(d, |w| w.min(d)));
                }
            }
        }
        (jobs, wait)
    }

    /// A fetch finished: keep its value (or its error on the last good one), save, and send it.
    fn fetched(&self, hub: &Hub, job: &Job, result: Result<Value, FetchError>) {
        let now = now_ms();
        let t = eval::iso_utc(now);
        let ok = result.is_ok();
        let (value, snapshot) = {
            let mut inner = self.lock();
            // A result for a URL the app has moved on from is dropped.
            let mut current = true;
            if let Some(sub) = inner.open.get_mut(&job.app).and_then(|r| r.subs.iter_mut().find(|s| s.name == job.name)) {
                sub.in_flight = false;
                current = sub.url.as_deref() == Some(job.url.as_str());
                // A failure tries again soon rather than at `every`.
                if !ok {
                    sub.next = Instant::now() + sub.every.min(ERROR_RETRY);
                }
            }
            if !current {
                return;
            }
            let cache = inner.cache.entry(job.app.clone()).or_default();
            let entry = cache
                .entry(job.name.clone())
                .or_insert_with(|| Cached { url: job.url.clone(), value: json!({}), history: VecDeque::new(), at_ms: now });
            if entry.url != job.url {
                *entry = Cached { url: job.url.clone(), value: json!({}), history: VecDeque::new(), at_ms: now };
            }
            entry.at_ms = now;
            match result {
                Ok(body) => {
                    let sample = sample_of(body, &t);
                    if let Some(keep) = job.keep_ms {
                        if sample.to_string().len() <= MAX_HISTORY_SAMPLE {
                            entry.history.push_back(sample.clone());
                        }
                        trim_history(&mut entry.history, keep, now);
                    }
                    entry.value = sample;
                }
                Err(error) => {
                    // The last good value stays (a new URL has none), with what went wrong.
                    if let Value::Object(fields) = &mut entry.value {
                        fields.insert("error".into(), json!(error.to_string()));
                        fields.entry("t").or_insert_with(|| json!(t));
                    }
                }
            }
            let value = with_history(&entry.value, job.keep_ms.map(|_| &entry.history));
            (value, cache.clone())
        };
        self.save(&job.app, &snapshot);
        if ok {
            hub.core.grant_used(&job.app, "net.http");
        }
        self.wake.notify_one();
        emit(hub, &job.app, Map::from_iter([(format!("/services/{}", job.name), value)]));
    }

    /// Adds a feed's new items to the app's `table` (by `guid`), oldest first, filling the [`FEED_COLUMNS`] it
    /// has. New rows refresh the open page.
    fn store_items(&self, hub: &Hub, app: &str, table: &str, items: &[feed::Item]) {
        let Ok((installed, db)) = hub.apps.db(app) else { return };
        let Some((sql, columns)) = installed.app.tables.iter().find(|t| t.name == table).and_then(feed_insert) else {
            return tracing::warn!(app, table, "feed: the table has no guid column");
        };
        let mut added = 0;
        for item in items.iter().rev() {
            match db.exec(&sql, &feed_params(item, &columns)) {
                Ok(exec) => added += exec.rows_affected,
                Err(e) => {
                    tracing::warn!(app, table, "feed: items not stored: {e:#}");
                    break;
                }
            }
        }
        if added > 0 {
            tracing::info!(app, table, added, "feed: new items");
            hub.scheduler.changed(app, &[table.to_owned()]);
            hub.emit(Event::AppChanged { app: app.to_owned(), tables: vec![table.to_owned()] }, Source::Api);
        }
    }
}

/// The loop: fetches what's due, then sleeps until the next one is due or something changes. With no app
/// open, it only waits for a notify (no timer).
async fn run(hub: Arc<Hub>) {
    loop {
        let (jobs, wait) = hub.services.due(&hub);
        for job in jobs {
            let h = hub.clone();
            tokio::spawn(async move {
                let fetcher = h.services.fetcher();
                let (format, url, hosts) = (job.format, job.url.clone(), job.hosts.clone());
                let started = Instant::now();
                let fetched = tokio::task::spawn_blocking(move || fetch(&fetcher, format, &url, &hosts)).await;
                let (result, items) = fetched.unwrap_or_else(|e| (Err(FetchError::Failed(format!("r1d: {e}"))), None));
                let ms = started.elapsed().as_millis() as u64;
                match &result {
                    Ok(_) => tracing::debug!(app = %job.app, service = %job.name, ms, "fetched"),
                    Err(e) => tracing::warn!(app = %job.app, service = %job.name, url = %job.url, error = %e, ms, "fetch failed"),
                }
                if let (Some(items), Some(table)) = (&items, &job.into) {
                    let (hb, app, table, items) = (h.clone(), job.app.clone(), table.clone(), items.clone());
                    let _ = tokio::task::spawn_blocking(move || hb.services.store_items(&hb, &app, &table, &items)).await;
                }
                h.services.fetched(&h, &job, result);
            });
        }
        match wait {
            Some(d) => {
                let _ = tokio::time::timeout(d, hub.services.wake.notified()).await;
            }
            None => hub.services.wake.notified().await,
        }
    }
}

/// One request, read as the service's format: its value, and a feed's items.
pub fn fetch(fetcher: &Fetcher, format: Format, url: &str, hosts: &[String]) -> (Result<Value, FetchError>, Option<Vec<feed::Item>>) {
    let allowed = |host: &str| hosts.iter().any(|h| h.eq_ignore_ascii_case(host));
    match format {
        Format::Json => (fetcher.json(url, &allowed).and_then(|f| ok_body(f.status, f.body)), None),
        Format::Feed => {
            let mut items = None;
            let value = fetcher.get(url, Format::Feed, &allowed).and_then(ok_status).and_then(|r| {
                let feed = feed::parse(&String::from_utf8_lossy(&r.body)).map_err(FetchError::Failed)?;
                let value = feed_sample(&feed);
                items = Some(feed.items);
                Ok(value)
            });
            (value, items)
        }
        Format::Text => (fetcher.get(url, Format::Text, &allowed).and_then(ok_status).map(|r| text_sample(&r)), None),
        // Not an app service's format (`Format::of` never gives it): chats' cards only.
        Format::Image => (Err(FetchError::Failed("pictures aren't app data".into())), None),
    }
}

fn emit(hub: &Hub, app: &str, values: Map<String, Value>) {
    hub.emit(Event::AppServices { app: app.to_owned(), values }, Source::Api);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A good kept value for this service's URL that isn't due yet (one with an error is due at once).
fn cached_fresh(cached: Option<&Cached>, sub: &Sub) -> bool {
    cached.is_some_and(|c| {
        Some(&c.url) == sub.url.as_ref() && c.value.get("error").is_none() && now_ms() - c.at_ms < sub.every.as_millis() as i64
    })
}

/// What a page that opens is shown at once: each service's kept value (with its history).
fn current_values(inner: &Inner, id: &str) -> Map<String, Value> {
    let mut values = Map::new();
    let Some(cache) = inner.cache.get(id) else { return values };
    let keeps: HashMap<&str, bool> = inner
        .open
        .get(id)
        .map(|r| r.installed.app.services.iter().map(|s| (s.name.as_str(), s.keep_ms.is_some())).collect())
        .unwrap_or_default();
    for (name, c) in cache {
        let keep = keeps.get(name.as_str()).copied().unwrap_or(!c.history.is_empty());
        values.insert(format!("/services/{name}"), with_history(&c.value, keep.then_some(&c.history)));
    }
    values
}

/// The value pushed for a service: its sample, with `history` when it keeps one.
fn with_history(value: &Value, history: Option<&VecDeque<Value>>) -> Value {
    let mut value = value.clone();
    if let (Value::Object(fields), Some(history)) = (&mut value, history) {
        fields.insert("history".into(), Value::Array(history.iter().cloned().collect()));
    }
    value
}

/// The newest samples whose JSON fits in `max` bytes, oldest first.
fn fit_history(history: &VecDeque<Value>, max: usize) -> Vec<Value> {
    let mut size = 0;
    let mut kept: Vec<Value> = history
        .iter()
        .rev()
        .take_while(|sample| {
            size += sample.to_string().len() + 1;
            size <= max
        })
        .cloned()
        .collect();
    kept.reverse();
    kept
}

/// Drops history entries older than `keep_ms` before `now_ms`.
fn trim_history(history: &mut VecDeque<Value>, keep_ms: i64, now_ms: i64) {
    let oldest = eval::iso_utc(now_ms - keep_ms);
    while history.front().and_then(|v| v.get("t")?.as_str().map(|t| t < oldest.as_str())) == Some(true)
        || history.len() > MAX_HISTORY
    {
        history.pop_front();
    }
}

/// The statement that adds a feed item to `table` unless its `guid` is there, and the columns it fills; `None`
/// without a `guid` column.
fn feed_insert(table: &enee_spec::compiled::Table) -> Option<(String, Vec<&'static str>)> {
    let columns: Vec<&str> = FEED_COLUMNS.into_iter().filter(|c| table.columns.iter().any(|x| x.name == *c)).collect();
    if !columns.contains(&"guid") {
        return None;
    }
    let names = columns.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
    let values = columns
        .iter()
        .map(|c| if *c == "published" { "coalesce(:published, now())".to_string() } else { format!(":{c}") })
        .collect::<Vec<_>>()
        .join(", ");
    let t = &table.name;
    Some((format!("INSERT INTO \"{t}\" ({names}) SELECT {values} WHERE NOT EXISTS (SELECT 1 FROM \"{t}\" WHERE guid = :guid)"), columns))
}

/// An item's values for [`feed_insert`]'s columns.
fn feed_params(item: &feed::Item, columns: &[&str]) -> Map<String, Value> {
    let fields = serde_json::to_value(item).unwrap_or_default();
    columns.iter().map(|c| (c.to_string(), fields.get(*c).cloned().unwrap_or(Value::Null))).collect()
}

/// A feed's value: its title and link, and its first [`FEED_ITEMS`] items.
fn feed_sample(feed: &Feed) -> Value {
    let items: Vec<&feed::Item> = feed.items.iter().take(FEED_ITEMS).collect();
    json!({ "title": feed.title, "link": feed.link, "items": items })
}

/// An `http.fetch` value: the status, the content type and the text (cut to [`MAX_TEXT`]).
fn text_sample(response: &crate::fetch::Response) -> Value {
    let text = String::from_utf8_lossy(&response.body);
    let mut cut = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    json!({ "status": response.status, "type": response.content_type, "text": &text[..cut] })
}

/// A 2xx response; any other status is an error.
fn ok_status(response: crate::fetch::Response) -> Result<crate::fetch::Response, FetchError> {
    if (200..300).contains(&response.status) {
        Ok(response)
    } else {
        Err(FetchError::Failed(format!("HTTP {}", response.status)))
    }
}

/// A 2xx body; any other status is an error that quotes the API's own message when it gives one.
fn ok_body(status: u16, body: Value) -> Result<Value, FetchError> {
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let message = ["message", "msg", "error", "detail"].iter().find_map(|k| body.get(*k)?.as_str().map(str::to_owned));
    Err(FetchError::Failed(match message {
        Some(m) => format!("HTTP {status}: {}", m.chars().take(200).collect::<String>()),
        None => format!("HTTP {status}"),
    }))
}

/// A body as a sample: a JSON object's own fields, an array as `items`, anything else as `value` (a feed or
/// a page is already an object); plus `t`.
pub fn sample_of(body: Value, t: &str) -> Value {
    let mut fields = match body {
        Value::Object(fields) => fields,
        Value::Array(items) => Map::from_iter([("items".to_string(), Value::Array(items))]),
        other => Map::from_iter([("value".to_string(), other)]),
    };
    fields.insert("t".into(), json!(t));
    Value::Object(fields)
}

/// The app's `net.http` hosts, lowercase.
fn http_hosts(app: &CompiledApp) -> Arc<[String]> {
    app.permissions.iter().filter(|p| p.id == "net.http").flat_map(crate::broker::hosts).collect()
}

/// A URL template's host (the compiler requires it written out).
pub fn template_host(template: &str) -> String {
    url::Url::parse(&template.replace("{{", "x").replace("}}", "x"))
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

/// The values the app's URLs bind before its page reports them: its kept state (else the initial state), and
/// its settings (else their defaults).
fn default_vars(hub: &Hub, app: &CompiledApp) -> Map<String, Value> {
    let settings = hub.apps.settings(&app.app.id).unwrap_or_default();
    let kept = hub.apps.kept_state(&app.app.id);
    vars_from(app, &settings, &kept)
}

pub fn vars_from(app: &CompiledApp, settings: &Map<String, Value>, kept: &Map<String, Value>) -> Map<String, Value> {
    let mut vars = Map::new();
    for pointer in app.services.iter().flat_map(|s: &Service| &s.deps) {
        let mut parts = pointer.trim_start_matches('/').split('/');
        let value = match (parts.next(), parts.next()) {
            (Some("state"), Some(key)) => {
                let mut value = kept.get(key).or_else(|| app.state.get(key));
                for part in parts {
                    value = value.and_then(|v| v.get(part).or_else(|| v.get(part.parse::<usize>().ok()?)));
                }
                value
            }
            (Some("settings"), Some(key)) => {
                settings.get(key).filter(|v| !v.is_null()).or_else(|| app.settings.get(key).and_then(|d| d.get("default")))
            }
            _ => None,
        };
        if let Some(value) = value {
            vars.insert(pointer.clone(), value.clone());
        }
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(yaml: &str) -> CompiledApp {
        crate::apps::compile(yaml).unwrap_or_else(|r| panic!("{:?}", r.diagnostics))
    }

    #[test]
    fn json_bodies_become_samples() {
        assert_eq!(sample_of(json!({"price": 1.5}), "T"), json!({"price": 1.5, "t": "T"}));
        assert_eq!(sample_of(json!([1, 2]), "T"), json!({"items": [1, 2], "t": "T"}));
        assert_eq!(sample_of(json!(7), "T"), json!({"value": 7, "t": "T"}));
        assert_eq!(ok_body(404, json!({"message": "No such ticker"})).unwrap_err().to_string(), "HTTP 404: No such ticker");
    }

    #[test]
    fn urls_start_from_kept_state_and_settings() {
        let a = app(r#"enee: 1
app: { id: quote, name: Quote }
permissions:
  net.http: { hosts: [api.example.com], reason: Prices }
settings:
  currency: { type: text, default: USD, label: Currency }
state:
  ticker: AAPL
services:
  q: { use: http.json, url: "https://api.example.com/q/{{state.ticker}}?in={{settings.currency}}", every: 5m }
views:
  main:
    - stat: { label: Price, value: "{{services.q.price}}" }
"#);
        let none = Map::new();
        let vars = vars_from(&a, &none, &none);
        let url = crate::fetch::fill_url(a.services[0].options["url"].as_str().unwrap(), &|p| vars.get(p).cloned());
        assert_eq!(url.as_deref(), Some("https://api.example.com/q/AAPL?in=USD"));
        let kept = Map::from_iter([("ticker".to_string(), json!("MSFT"))]);
        let settings = Map::from_iter([("currency".to_string(), json!("EUR"))]);
        let vars = vars_from(&a, &settings, &kept);
        assert_eq!(vars["/state/ticker"], json!("MSFT"));
        assert_eq!(vars["/settings/currency"], json!("EUR"));
        assert_eq!(template_host("https://API.example.com/q/{{state.ticker}}"), "api.example.com");
    }

    #[test]
    fn feed_items_are_added_once() {
        let a = app(include_str!("../../../fixtures/apps/news.enee.yaml"));
        let table = a.tables.iter().find(|t| t.name == "stories").unwrap();
        let (sql, columns) = feed_insert(table).unwrap();
        assert!(sql.contains("WHERE NOT EXISTS") && columns.contains(&"published"), "{sql}");
        let item = feed::Item { guid: "g1".into(), title: "T".into(), ..Default::default() };
        let params = feed_params(&item, &columns);
        assert_eq!(params["guid"], json!("g1"));
        assert_eq!(params["published"], Value::Null, "falls back to now() in SQL");
    }

    #[test]
    fn histories_keep_their_newest_samples() {
        let h: VecDeque<Value> = (0..10).map(|i| json!({"v": i})).collect();
        let fit = fit_history(&h, 30);
        assert_eq!(fit.last(), Some(&json!({"v": 9})));
        assert!(fit.len() < 10);
    }
}
