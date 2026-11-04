//! Hermes Agent as a brain (ADR-0070): the R1 as the voice and wheel of a gateway its owner already runs.
//!
//! - **Nothing runs here.** r1d is a client of the owner's gateway: its memory, skills, tools and model sign-ins stay
//!   on its machine. A chat is a Hermes API session of its own (`r1os-<chat id>`), and a turn is a run: `POST /v1/runs`,
//!   then its events from `GET /v1/runs/{id}/events` (SSE), with one socket open only while it lasts.
//! - **The key** is the one long-lived credential r1d holds. It's in `<data>/hermes.json`, never `core.db` (a backup
//!   copies that, and promises no credentials), never logged, and never sent back by the API.
//! - **It can't use r1d's tools:** `/mcp` is on localhost. Its own tools run on the gateway's machine, so what they want
//!   to do comes to the R1's card, every time ([`Ui::allow`]), and a turn another agent started is refused outright.
//! - **A dropped stream** reattaches with `Last-Event-ID`: the run carries on at the gateway, which replays what was
//!   missed. One connection lasts at most [`STREAM_LIMIT`], so a blocked read can't hold its thread for long.
//! - **Tidying is Hermes's own:** it compresses its sessions, and an old session id follows the rotation.
//! - **A gateway that restarts mid-run** pauses that session until someone says the lost run won't finish: every turn
//!   after it fails with `unknown_execution` (seen on v0.22.7, even for a clean restart). So a run that didn't end
//!   cleanly is remembered and settled before the chat's next turn ([`settle_run`]). If r1d has lost track of it, the
//!   chat carries on in a fresh session, told what was said lately.
//!
//! Tried against a real Hermes Agent v0.22.7 gateway (with a fake model), 2026-11-04.

use crate::chat::Turn;
use crate::core::{Agent, Chat, Memory};
use crate::hub::{Event, Hub, Source};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, mpsc};

/// The longest a turn may take: real work on the gateway's machine, and waits on the R1's card.
const TURN_LIMIT: Duration = Duration::from_secs(30 * 60);
/// The longest one events connection lives before it's replaced (ureq's budget for a body isn't per read).
const STREAM_LIMIT: Duration = Duration::from_secs(5 * 60);
/// Not a byte for this long means the connection's gone: the gateway sends a keepalive every 10 s.
const SILENCE: Duration = Duration::from_secs(45);
/// Connections lost in a row before the turn gives up, waiting a little longer after each ([`backoff`]).
const LOST_TRIES: u32 = 6;
/// A short request's budget.
const REQUEST_LIMIT: Duration = Duration::from_secs(30);
/// What a turn needs of a gateway (`GET /v1/capabilities`).
const NEEDS: &[&str] = &["runs", "run_status", "run_events", "run_approval", "run_stop"];
/// What the agent hears when it asks a question mid-turn: the R1 has no way to answer during one.
const NO_QUESTIONS: &str = "The user is on a small handheld and can't answer during a task. Make your best judgement and say \
what you assumed; if you really can't go on without them, finish now and ask in your reply.";
const NOT_CONNECTED: &str = "Hermes isn't connected: add your gateway in the companion, under accounts.";

/// The owner's gateway. No `Debug`: the key must never reach a log.
#[derive(Clone, Serialize, Deserialize)]
pub struct Gateway {
    /// With no trailing slash; a profile is part of it (`/p/<profile>`).
    pub url: String,
    key: String,
}

/// The one gateway this R1 talks to.
pub struct Hermes {
    path: PathBuf,
    gateway: Mutex<Option<Gateway>>,
    /// Chat → its last run, until that run has ended cleanly: one that hasn't may be holding the session ([`settle_run`]).
    open: Mutex<HashMap<String, String>>,
}

impl Hermes {
    pub fn new(data: &Path) -> Self {
        let path = data.join("hermes.json");
        let gateway = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        Hermes { path, gateway: Mutex::new(gateway), open: Mutex::default() }
    }

    /// Where it's connected to, for Settings. Never the key.
    pub fn url(&self) -> Option<String> {
        self.gateway.lock().unwrap().as_ref().map(|g| g.url.clone())
    }

    fn gateway(&self) -> Option<Gateway> {
        self.gateway.lock().unwrap().clone()
    }

    fn keep(&self, gateway: Option<Gateway>) -> Result<()> {
        match &gateway {
            Some(g) => write_private(&self.path, &serde_json::to_vec(g)?).context("keep the gateway's key")?,
            None => match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e).context("forget the gateway's key"),
                _ => {}
            },
        }
        *self.gateway.lock().unwrap() = gateway;
        Ok(())
    }

    /// Connects to the gateway at `url`, once it has answered as one with that key.
    pub async fn connect(&self, hub: &Hub, url: &str, key: &str) -> Result<Value> {
        let key = key.trim();
        anyhow::ensure!(!key.is_empty(), "it needs its API_SERVER_KEY");
        anyhow::ensure!(key.len() <= 4096 && key.chars().all(|c| c.is_ascii_graphic()), "that key has characters a key can't have");
        let gateway = Gateway { url: clean_url(url)?, key: key.to_owned() };
        let checking = gateway.clone();
        tokio::task::spawn_blocking(move || check(&checking)).await.context("check the gateway")??;
        let url = gateway.url.clone();
        self.keep(Some(gateway))?;
        tracing::info!(url = %url, "connected to a Hermes gateway");
        hub.emit(Event::Account { brain: "hermes".into(), state: "signed_in".into(), detail: None }, Source::Api);
        Ok(json!({ "url": url }))
    }

    /// Forgets the gateway and its key. Its chats stay, and say so if you talk in them.
    pub fn forget(&self, hub: &Hub) -> Result<()> {
        self.keep(None)?;
        hub.emit(Event::Account { brain: "hermes".into(), state: "signed_out".into(), detail: None }, Source::Api);
        Ok(())
    }
}

/// Written whole and readable only by its owner (root, on the R1).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// A gateway's address as the owner typed it, checked: its key goes in a header to it, so plain `http` is only for
/// addresses and names that stay on a private network.
pub fn clean_url(raw: &str) -> Result<String> {
    let raw = raw.trim().trim_end_matches('/');
    anyhow::ensure!(!raw.is_empty(), "it needs the gateway's address");
    let full = if raw.contains("://") {
        raw.to_owned()
    } else {
        // Without a scheme: private names and addresses are usually plain http on Hermes's port.
        let scheme = url::Url::parse(&format!("http://{raw}")).ok().filter(private_host).map_or("https", |_| "http");
        format!("{scheme}://{raw}")
    };
    let u = url::Url::parse(&full).map_err(|_| anyhow!("that isn't an address"))?;
    anyhow::ensure!(matches!(u.scheme(), "http" | "https"), "the address starts with http or https");
    anyhow::ensure!(u.host_str().is_some_and(|h| !h.is_empty()), "that address has no host");
    anyhow::ensure!(u.username().is_empty() && u.password().is_none() && u.query().is_none() && u.fragment().is_none(), "just the gateway's address, with the key apart");
    anyhow::ensure!(u.scheme() == "https" || private_host(&u), "use https for a gateway on the internet: its key would cross it in the clear");
    let s = u.as_str().trim_end_matches('/');
    Ok(s.strip_suffix("/v1").unwrap_or(s).to_owned())
}

/// Stays on a private network: private, loopback and link-local addresses, Tailscale's range and its names, and local
/// names.
fn private_host(u: &url::Url) -> bool {
    match u.host() {
        Some(url::Host::Ipv4(ip)) => {
            let [a, b, ..] = ip.octets();
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || (a == 100 && (b & 0xC0) == 64)
        }
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        Some(url::Host::Domain(name)) => {
            let name = name.to_ascii_lowercase();
            !name.contains('.') || [".local", ".lan", ".internal", ".home.arpa", ".ts.net", ".localhost"].iter().any(|end| name.ends_with(end))
        }
        None => false,
    }
}

fn agent(limit: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .https_only(false)
        .proxy(None)
        // Never: a redirect would carry the key somewhere else.
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(REQUEST_LIMIT))
        .timeout_global(Some(limit))
        .user_agent(concat!("r1os/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}

impl Gateway {
    fn bearer(&self) -> String {
        format!("Bearer {}", self.key)
    }

    fn get(&self, path: &str) -> Result<(u16, Value)> {
        let mut r = agent(REQUEST_LIMIT)
            .get(format!("{}{path}", self.url))
            .header("authorization", self.bearer())
            .header("accept", "application/json")
            .call()
            .map_err(|e| anyhow!("{e}"))?;
        let status = r.status().as_u16();
        let text = r.body_mut().read_to_string().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }

    fn post(&self, path: &str, body: &Value, idempotency: Option<&str>) -> Result<(u16, Value)> {
        let mut req = agent(REQUEST_LIMIT)
            .post(format!("{}{path}", self.url))
            .header("authorization", self.bearer())
            .header("content-type", "application/json");
        if let Some(k) = idempotency {
            req = req.header("idempotency-key", k);
        }
        let mut r = req.send(body.to_string()).map_err(|e| anyhow!("{e}"))?;
        let status = r.status().as_u16();
        let text = r.body_mut().read_to_string().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }

    /// Answers one of the run's questions, in the background of the turn.
    fn answer(&self, run: &str, path: &str, body: &Value) {
        match self.post(&format!("/v1/runs/{run}/{path}"), body, None) {
            Ok((s, _)) if (200..300).contains(&s) => {}
            // Settled some other way (another viewer, the run ending): nothing to do.
            Ok((s, v)) => tracing::info!(status = s, why = %v["error"]["code"], "Hermes didn't take an answer"),
            Err(e) => tracing::info!(error = %format!("{e:#}"), "couldn't answer Hermes"),
        }
    }
}

/// A run that didn't end cleanly: if the gateway restarted under it, it's `unknown`, and its session takes no more turns
/// until someone says it won't finish. True once nothing of it holds the session.
fn settle_run(gateway: &Gateway, run: &str) -> bool {
    let status = match gateway.get(&format!("/v1/runs/{run}")) {
        Ok((200, v)) => v,
        // Long forgotten: nothing to settle.
        Ok((404, _)) => return true,
        Ok(_) => return false,
        Err(e) => {
            tracing::info!(error = %format!("{e:#}"), "couldn't ask Hermes about an earlier run");
            return false;
        }
    };
    match status["status"].as_str() {
        Some("completed" | "failed" | "cancelled" | "interrupted") => true,
        Some("unknown") => {
            let body = json!({ "admission_id": status["admission_id"], "execution_generation": status["execution_generation"] });
            match gateway.post(&format!("/v1/runs/{run}/resolve-unknown"), &body, None) {
                Ok((s, _)) if (200..300).contains(&s) => {
                    tracing::info!("told Hermes a run lost in a restart won't finish");
                    true
                }
                // Already settled, some other way.
                Ok((409, _)) => true,
                Ok((s, v)) => {
                    tracing::info!(status = s, why = %v["error"]["code"], "Hermes didn't take a lost run's end");
                    false
                }
                Err(e) => {
                    tracing::info!(error = %format!("{e:#}"), "couldn't tell Hermes a run was lost");
                    false
                }
            }
        }
        // Still going: the next turn queues behind it.
        _ => false,
    }
}

/// Is this Hermes saying the session is paused behind a run lost in a restart?
fn paused(why: &str) -> bool {
    why.contains("unknown_execution")
}

/// Is it a Hermes gateway we can drive, and does it take this key? `Err` says why not, for the owner.
fn check(gateway: &Gateway) -> Result<()> {
    let (status, caps) = gateway.get("/v1/capabilities").map_err(|e| anyhow!("couldn't reach it: {e:#}"))?;
    match status {
        200 => {}
        401 | 403 => bail!("the gateway refused that key"),
        404 => bail!("that isn't a Hermes gateway, or its API server is off (API_SERVER_ENABLED)"),
        s => bail!("it answered HTTP {s}"),
    }
    anyhow::ensure!(caps["platform"] == "hermes-agent", "that isn't a Hermes gateway");
    let missing: Vec<&str> = NEEDS.iter().copied().filter(|n| caps["endpoints"][*n].is_null()).collect();
    anyhow::ensure!(missing.is_empty(), "this Hermes has no {}: update it", missing.join(", "));
    Ok(())
}

/// Why the gateway refused a request, in words.
fn refused(status: u16, body: &Value) -> anyhow::Error {
    let why = body["error"]["message"].as_str().or_else(|| body["error"].as_str()).unwrap_or_default();
    match status {
        401 | 403 => anyhow!("Your Hermes gateway refused r1os's key: connect it again in the companion, under accounts."),
        429 => anyhow!("Your Hermes gateway is running as many tasks as it allows. Try again in a moment."),
        503 => anyhow!("Your Hermes gateway is restarting or busy. Try again in a moment."),
        s if why.is_empty() => anyhow!("Your Hermes gateway answered HTTP {s}."),
        s => anyhow!("Your Hermes gateway answered HTTP {s}: {why}"),
    }
}

// ---- reading a run --------------------------------------------------------------------------------------------

/// One thing from the events stream.
#[derive(Debug, PartialEq)]
enum Got {
    Event(Value),
    /// A keepalive: the connection's there.
    Alive,
    /// The stream ended.
    Closed,
}

/// Server-sent events, a line at a time: `data:` lines up to a blank one. The event's name and number are in its JSON.
#[derive(Default)]
struct Sse {
    data: String,
}

impl Sse {
    fn line(&mut self, line: &str) -> Option<Got> {
        if line.is_empty() {
            if self.data.is_empty() {
                return None;
            }
            let data = std::mem::take(&mut self.data);
            return serde_json::from_str(&data).ok().map(Got::Event);
        }
        if line.starts_with(':') {
            return Some(Got::Alive);
        }
        if let Some(d) = line.strip_prefix("data:") {
            self.data.push_str(d.strip_prefix(' ').unwrap_or(d));
        }
        None
    }
}

/// Opens the run's events on a thread of their own (ureq blocks), after event `after`. They stop when the receiver goes.
fn listen(gateway: Gateway, run: String, after: Option<u64>) -> mpsc::Receiver<Got> {
    let (tx, rx) = mpsc::channel(64);
    let spawned = std::thread::Builder::new().name("r1d-hermes".into()).spawn(move || {
        let mut req = agent(STREAM_LIMIT)
            .get(format!("{}/v1/runs/{run}/events", gateway.url))
            .header("authorization", gateway.bearer())
            .header("accept", "text/event-stream");
        if let Some(seq) = after {
            req = req.header("last-event-id", seq.to_string());
        }
        let mut response = match req.call() {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::info!(status = r.status().as_u16(), "Hermes refused a run's events");
                let _ = tx.blocking_send(Got::Closed);
                return;
            }
            Err(e) => {
                tracing::info!(error = %e, "couldn't open a run's events");
                let _ = tx.blocking_send(Got::Closed);
                return;
            }
        };
        let mut sse = Sse::default();
        for line in BufReader::new(response.body_mut().as_reader()).lines() {
            let Ok(line) = line else { break };
            if let Some(got) = sse.line(&line)
                && tx.blocking_send(got).is_err()
            {
                return;
            }
        }
        let _ = tx.blocking_send(Got::Closed);
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start a thread for a run's events");
    }
    rx
}

/// Something a run asks of the user.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    /// What the answer names: `prompt_id` with Hermes's session authority, else `request_id`.
    id: Option<String>,
    /// Which execution it's for, with the session authority.
    generation: Option<i64>,
    pub command: String,
    pub description: String,
}

impl Asked {
    fn of(ev: &Value) -> Asked {
        let text = |k: &str| ev[k].as_str().unwrap_or_default().to_owned();
        Asked {
            id: ev["prompt_id"].as_str().or_else(|| ev["request_id"].as_str()).map(str::to_owned),
            generation: ev["execution_generation"].as_i64(),
            command: text("command"),
            description: text("description"),
        }
    }

    /// The answer's body. With the session authority Hermes takes exactly these fields.
    fn answer(&self, field: &str, value: &str) -> Value {
        let mut body = serde_json::Map::new();
        if let Some(id) = &self.id {
            body.insert("request_id".into(), json!(id));
        }
        if let Some(generation) = self.generation {
            body.insert("execution_generation".into(), json!(generation));
        }
        body.insert(field.into(), json!(value));
        Value::Object(body)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum How {
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

/// How a run ended.
#[derive(Debug, PartialEq)]
struct Ended {
    how: How,
    output: String,
    error: String,
    usage: Value,
    runtime: Value,
}

impl Ended {
    fn of(how: How, ev: &Value) -> Ended {
        let text = |k: &str| ev[k].as_str().unwrap_or_default().to_owned();
        Ended { how, output: text("output"), error: text("error"), usage: ev["usage"].clone(), runtime: ev["runtime"].clone() }
    }

    /// From `GET /v1/runs/{id}`, for a stream that ended without saying: if the run has ended.
    fn of_status(run: &Value) -> Option<Ended> {
        let how = match run["status"].as_str()? {
            "completed" => How::Completed,
            "failed" => How::Failed,
            "cancelled" => How::Cancelled,
            "interrupted" => How::Interrupted,
            // Its gateway restarted mid-turn, and Hermes can't say how that went.
            "unknown" => How::Interrupted,
            _ => return None,
        };
        Some(Ended::of(how, run))
    }
}

/// What one event means for the turn.
#[derive(Debug, PartialEq)]
enum Step {
    Nothing,
    Say(String),
    Tool(String),
    Approve(Asked),
    Clarify(Asked),
    End(Ended),
}

/// Reads one event. Anything it doesn't know is nothing: Hermes adds events often.
fn step_of(ev: &Value) -> Step {
    match ev["event"].as_str().unwrap_or_default() {
        "message.delta" => ev["delta"].as_str().filter(|d| !d.is_empty()).map_or(Step::Nothing, |d| Step::Say(d.to_owned())),
        // Text beside its tool calls. The final answer comes in `run.completed`.
        "message.interim" if ev["already_streamed"] != true => ev["text"].as_str().filter(|t| !t.is_empty()).map_or(Step::Nothing, |t| Step::Say(format!("{t}\n\n"))),
        "tool.started" => Step::Tool(tool_name(ev["tool"].as_str().unwrap_or("tool"))),
        "approval.request" => Step::Approve(Asked::of(ev)),
        "clarify.request" => Step::Clarify(Asked::of(ev)),
        "run.completed" => Step::End(Ended::of(How::Completed, ev)),
        "run.failed" => Step::End(Ended::of(How::Failed, ev)),
        "run.cancelled" => Step::End(Ended::of(How::Cancelled, ev)),
        "run.interrupted" => Step::End(Ended::of(How::Interrupted, ev)),
        _ => Step::Nothing,
    }
}

/// Hermes's tools under the names the screens already show (ChatScreen's TOOL_LABEL, shell/src/tool-labels.ts).
fn tool_name(tool: &str) -> String {
    match tool {
        "terminal" | "execute_code" | "process_manage" => "Bash",
        "web_search" => "WebSearch",
        "web_extract" | "browser_exec" => "WebFetch",
        "read_file" => "Read",
        "write_file" | "patch" => "Edit",
        "search_files" => "Grep",
        other => other,
    }
    .to_owned()
}

/// What a turn shows and asks on the R1 ([`HubUi`]; a stand-in in the tests).
trait Ui: Send + Sync + 'static {
    fn say(&self, text: &str);
    fn tool(&self, name: String);
    /// Whether the user allows what the run asks: it can take as long as the card does.
    fn allow(&self, asked: &Asked) -> Pin<Box<dyn Future<Output = bool> + Send>>;
}

/// How a run went, seen from here.
struct Followed {
    ended: Ended,
    /// What it had said since its last tool.
    said: String,
    /// You stopped it.
    stopped: bool,
}

fn backoff(tries: u32) -> Duration {
    Duration::from_secs(1 << tries.min(3))
}

/// Follows a run to its end: what it says and does goes to `ui`, what it asks is answered as `ui` says, a stop stops it,
/// and a lost connection comes back where it left off.
async fn follow(gateway: &Gateway, run: &str, ui: Arc<dyn Ui>, stop: Option<Arc<Notify>>) -> Result<Followed> {
    let far = tokio::time::Instant::now() + TURN_LIMIT;
    let (mut stop_by, mut stopped) = (None::<tokio::time::Instant>, false);
    let (mut last_seq, mut lost) = (None::<u64>, 0u32);
    let mut said = String::new();
    let mut asked = HashSet::new();
    let mut answers = tokio::task::JoinSet::new();
    let mut heard = tokio::time::Instant::now();
    let mut rx = listen(gateway.clone(), run.to_owned(), None);
    let ended = loop {
        let got = tokio::select! {
            g = rx.recv() => g.unwrap_or(Got::Closed),
            () = tokio::time::sleep_until(heard + SILENCE) => Got::Closed,
            () = tokio::time::sleep_until(far) => {
                let g = gateway.clone();
                let r = run.to_owned();
                let _ = tokio::task::spawn_blocking(move || g.post(&format!("/v1/runs/{r}/stop"), &json!({}), None)).await;
                bail!("Hermes was still going after {} minutes, so r1os stopped it.", TURN_LIMIT.as_secs() / 60);
            }
            () = crate::stops::fired(&stop), if !stopped => {
                stopped = true;
                stop_by = Some(tokio::time::Instant::now() + crate::stops::STOP_WAIT);
                let g = gateway.clone();
                let r = run.to_owned();
                match tokio::task::spawn_blocking(move || g.post(&format!("/v1/runs/{r}/stop"), &json!({}), None)).await {
                    Ok(Ok((s, _))) if (200..300).contains(&s) => {}
                    other => tracing::info!(ok = other.is_ok(), "couldn't ask Hermes to stop"),
                }
                continue;
            }
            () = tokio::time::sleep_until(stop_by.unwrap_or(far)), if stop_by.is_some() => {
                // It didn't end in time. It may yet, at the gateway, but you've stopped waiting.
                break Ended { how: How::Cancelled, output: String::new(), error: String::new(), usage: Value::Null, runtime: Value::Null };
            }
        };
        heard = tokio::time::Instant::now();
        match got {
            Got::Alive => lost = 0,
            Got::Closed => {
                // No end in the stream: has the run ended anyway? Otherwise, back where it left off.
                let (g, r) = (gateway.clone(), run.to_owned());
                let status = tokio::task::spawn_blocking(move || g.get(&format!("/v1/runs/{r}"))).await.context("ask how a run went")?;
                match &status {
                    Ok((200, v)) => {
                        if let Some(e) = Ended::of_status(v) {
                            break e;
                        }
                    }
                    Ok((s @ (401 | 403), v)) => return Err(refused(*s, v)),
                    _ => {}
                }
                lost += 1;
                if lost > LOST_TRIES {
                    bail!("Lost touch with your Hermes gateway partway through: it may still finish there.");
                }
                tracing::info!(lost, "a run's events ended early: reattaching");
                tokio::time::sleep(backoff(lost)).await;
                rx = listen(gateway.clone(), run.to_owned(), last_seq);
                heard = tokio::time::Instant::now();
            }
            Got::Event(ev) => {
                lost = 0;
                // A replay after reattaching can overlap what's been read.
                if let Some(seq) = ev["seq"].as_u64() {
                    if last_seq.is_some_and(|l| seq <= l) {
                        continue;
                    }
                    last_seq = Some(seq);
                }
                match step_of(&ev) {
                    Step::Nothing => {}
                    Step::Say(text) => {
                        said.push_str(&text);
                        ui.say(&text);
                    }
                    Step::Tool(name) => {
                        said.clear();
                        ui.tool(name);
                    }
                    Step::Approve(a) => {
                        if asked.insert(a.id.clone().unwrap_or_else(|| a.command.clone())) {
                            let (ui, g, r) = (ui.clone(), gateway.clone(), run.to_owned());
                            // The turn keeps reading while the card waits.
                            answers.spawn(async move {
                                let choice = if ui.allow(&a).await { "once" } else { "deny" };
                                let body = a.answer("choice", choice);
                                let _ = tokio::task::spawn_blocking(move || g.answer(&r, "approval", &body)).await;
                            });
                        }
                    }
                    Step::Clarify(a) => {
                        if asked.insert(a.id.clone().unwrap_or_default()) {
                            let (g, r) = (gateway.clone(), run.to_owned());
                            let body = a.answer("answer", NO_QUESTIONS);
                            answers.spawn(async move {
                                let _ = tokio::task::spawn_blocking(move || g.answer(&r, "clarify", &body)).await;
                            });
                        }
                    }
                    Step::End(e) => break e,
                }
            }
        }
    };
    Ok(Followed { ended, said, stopped })
}

// ---- a turn -----------------------------------------------------------------------------------------------------

/// What the R1 shows and asks for a turn.
struct HubUi {
    hub: Arc<Hub>,
    agent: String,
    chat: String,
    sent: Instant,
    speaking: AtomicBool,
    spoke: AtomicBool,
}

impl Ui for HubUi {
    fn say(&self, text: &str) {
        if !self.speaking.swap(true, Ordering::Relaxed) {
            self.hub.emit(Event::agent_state(&self.agent, "speaking", &self.chat), Source::Api);
        }
        if !self.spoke.swap(true, Ordering::Relaxed) {
            self.hub.perf.mark("reply.first_word.hermes", self.sent.elapsed());
            self.hub.perf.first_word();
        }
        self.hub.emit(Event::chat(&self.agent, &self.chat, "delta", Some(text.to_owned())), Source::Api);
    }

    fn tool(&self, name: String) {
        self.speaking.store(false, Ordering::Relaxed);
        crate::chat::tool_event(&self.hub, &self.agent, &self.chat, name);
    }

    fn allow(&self, asked: &Asked) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let (hub, agent, chat, asked) = (self.hub.clone(), self.agent.clone(), self.chat.clone(), asked.clone());
        Box::pin(async move {
            // A turn another agent started takes no risky steps (ADR-0012).
            if hub.mail.woken(&chat).is_some() {
                tracing::info!(chat = %chat, "refused what Hermes asked in a turn another agent started");
                return false;
            }
            let (detail, why) = if asked.command.is_empty() { (asked.description.as_str(), "") } else { (asked.command.as_str(), asked.description.as_str()) };
            let ask = crate::approvals::Ask::new("commands", "do this on the machine running Hermes", detail.chars().take(400).collect::<String>())
                .why(why)
                .chat(&chat)
                .every_time();
            hub.approvals.gate(&hub, &agent, ask).await.is_ok()
        })
    }
}

/// A chat's session at the gateway: the one it has, or the one named for it.
fn session_of(chat: &Chat) -> String {
    chat.session.as_deref().filter(|s| s.starts_with("r1os-")).map_or_else(|| format!("r1os-{}", chat.id), str::to_owned)
}

/// A new session for a chat whose old one is stuck behind a run r1d has lost track of.
fn fresh_session(chat: &Chat) -> String {
    format!("r1os-{}-{}", chat.id, crate::core::random_hex(3))
}

/// What a fresh session starts from: the words, with what was said lately (crate::remote::recent).
fn with_recap(said: &str, recent: Option<&str>) -> String {
    let lately = recent.map_or(String::new(), |r| format!("{r}\n"));
    format!("[r1os: your gateway restarted during an earlier turn, so this chat carries on in a fresh session.]\n{lately}{said}")
}

/// What a run starts from: words, or words and the photos sent with them.
fn input_of(said: &str, photos: &[crate::media::Item], data: &Path) -> Result<Value> {
    if photos.is_empty() {
        return Ok(json!(said));
    }
    let mut parts = vec![json!({ "type": "text", "text": said })];
    for p in photos {
        parts.push(json!({ "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", p.mime, crate::media::photo_b64(data, p)?) } }));
    }
    Ok(json!([{ "role": "user", "content": parts }]))
}

/// What Hermes hears on top of its own prompt: who it is here, and what the R1 is. Sent with every run and the same
/// each time, so it doesn't spoil the provider's prompt cache.
fn instructions(agent: &Agent, others: &[Agent], memories: &[Memory], zone: &str) -> String {
    let custom = if agent.custom.is_empty() { String::new() } else { format!(" {}", agent.custom) };
    let role = if agent.role.is_empty() { String::new() } else { format!("What you're for: {}.\n", agent.role) };
    let team = if others.is_empty() {
        String::new()
    } else {
        format!(" The user's other agents on the R1 can use them: {}.", others.iter().map(|o| o.name.as_str()).collect::<Vec<_>>().join(", "))
    };
    let shared: Vec<&str> = memories.iter().filter(|m| m.shared).map(|m| m.text.as_str()).collect();
    let own: Vec<&str> = memories.iter().filter(|m| !m.shared).map(|m| m.text.as_str()).collect();
    format!(
        // Spelled out: on a small local model, "here you're {name}, talking through their R1" got its own name right, and
        // that it isn't the handheld, 4 times in 8 (lfm2.5, 2026-11-04).
        "In this conversation your name is {name}. Use that name, not any other you've been given. You are not a device: you \
         run on the user's own machine, and they are talking to you through their Rabbit R1, a small handheld in their \
         hand, with a 2.88-inch screen, a wheel and a button. {personality}{custom}\n\
         {role}\
         Keep replies short enough to read at a glance: a sentence or two unless they ask for more, in plain text, with no \
         tables. They usually speak to you, so their words come through speech-to-text and may have small mistakes. The \
         user's time zone is {zone}.\n\
         Your tools, memory and skills are your own, on the machine you run on, not on the R1. You can't use the R1's own \
         tools (its apps, camera, alarms or screen): if they ask, say so.{team} When something you do needs approval, the \
         R1 shows the user a card; they answer there.\n\n\
         What every agent on the R1 knows about the user:\n{shared}\n\n\
         What the R1 keeps for you in particular:\n{own}",
        name = agent.name,
        personality = crate::chat::personality(&agent.personality),
        shared = crate::chat::bullets(&shared),
        own = crate::chat::bullets(&own),
    )
}

/// Starts a run: `Err` says why it couldn't, in words. The one retry is safe: the key makes it the same run.
async fn start(gateway: &Gateway, body: Value) -> Result<String> {
    let body = Arc::new(body);
    let once = format!("r1os-{}", crate::core::random_hex(12));
    for attempt in 0..2 {
        let (g, b, k) = (gateway.clone(), body.clone(), once.clone());
        match tokio::task::spawn_blocking(move || g.post("/v1/runs", &b, Some(&k))).await.context("start a run")? {
            Ok((200..=299, v)) => return v["run_id"].as_str().map(str::to_owned).context("Your Hermes gateway didn't say which run it started."),
            Ok((s, v)) => return Err(refused(s, &v)),
            Err(e) if attempt == 0 => {
                tracing::info!(error = %format!("{e:#}"), "couldn't reach Hermes: once more");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => bail!("Couldn't reach your Hermes gateway at {}: {e:#}", gateway.url),
        }
    }
    unreachable!("the second attempt returns")
}

/// What a followed run comes to. `Err` is what the chat shows; a stop is [`crate::stops::Stopped`].
fn finish(run: Followed) -> Result<(String, Value, Value)> {
    let Followed { ended, said, stopped } = run;
    let reply = if ended.output.trim().is_empty() { said.trim().to_owned() } else { ended.output.trim().to_owned() };
    if stopped || ended.how == How::Cancelled {
        return Err(anyhow::Error::new(crate::stops::Stopped { said: reply, killed: false }));
    }
    match ended.how {
        How::Interrupted => bail!("Your Hermes gateway shut down partway through. Ask again once it's back."),
        // A run that ran out of steps still wraps up what it found: that's an answer.
        How::Failed if reply.is_empty() => bail!("Hermes couldn't finish that: {}", if ended.error.is_empty() { "it didn't say why" } else { &ended.error }),
        _ if reply.is_empty() => bail!("Hermes finished without an answer."),
        _ => Ok((reply, ended.usage, ended.runtime)),
    }
}

/// One of `agent`'s turns in `chat`, on the owner's gateway.
pub async fn turn(hub: &Arc<Hub>, chat: &Chat, agent: &Agent, said: &str, photos: &[crate::media::Item]) -> Result<Turn> {
    let gateway = hub.hermes.gateway().context(NOT_CONNECTED)?;
    // A run the last turn didn't see end may be holding the session.
    let earlier = hub.hermes.open.lock().unwrap().get(&chat.id).cloned();
    if let Some(run) = earlier {
        let g = gateway.clone();
        if tokio::task::spawn_blocking(move || settle_run(&g, &run)).await.unwrap_or(false) {
            hub.hermes.open.lock().unwrap().remove(&chat.id);
        }
    }
    let mut session = session_of(chat);
    if chat.session.as_deref() != Some(session.as_str()) {
        hub.core.set_session(&chat.id, &session)?;
    }
    let others = crate::chat::others(hub, &agent.id);
    let memories = crate::chat::memories(hub, &agent.id);
    let told = instructions(agent, &others, &memories, &crate::alarms::zone());
    let sent = Instant::now();
    let ui = Arc::new(HubUi { hub: hub.clone(), agent: agent.id.clone(), chat: chat.id.clone(), sent, speaking: AtomicBool::new(false), spoke: AtomicBool::new(false) });
    let mut input = input_of(said, photos, &hub.cfg.data)?;
    let mut retried = false;
    let followed = loop {
        let body = json!({ "input": input, "session_id": session, "instructions": told });
        match start(&gateway, body).await {
            Ok(run) => {
                hub.hermes.open.lock().unwrap().insert(chat.id.clone(), run.clone());
                let out = follow(&gateway, &run, ui.clone(), hub.stops.get(&chat.id)).await;
                // Whatever was still waiting on a card has nothing to wait for.
                hub.approvals.cancel_chat(hub, &chat.id);
                let out = out?;
                let stuck = out.ended.how == How::Failed && paused(&out.ended.error);
                // Seen to its end, or refused without running: nothing of it holds the session. One that was
                // interrupted or that you stopped gets looked at before the next turn.
                if stuck || (out.ended.how != How::Interrupted && !out.stopped) {
                    hub.hermes.open.lock().unwrap().remove(&chat.id);
                }
                if !stuck || retried {
                    break out;
                }
            }
            Err(e) if !retried && paused(&format!("{e:#}")) => {}
            Err(e) => return Err(e),
        }
        // Its session is stuck behind a run we've lost track of (r1d restarted too): carry on in a fresh one.
        retried = true;
        session = fresh_session(chat);
        hub.core.set_session(&chat.id, &session)?;
        tracing::info!(chat = %chat.id, "a Hermes session is stuck behind a lost run: starting a fresh one");
        input = input_of(&with_recap(said, crate::remote::recent(hub, &chat.id).as_deref()), photos, &hub.cfg.data)?;
    };
    let (text, usage, runtime) = finish(followed)?;
    hub.perf.mark("reply.done.hermes", sent.elapsed());
    // Context 0 is "not known", so r1d never tidies it: Hermes compresses its own sessions.
    Ok(Turn { text, meta: json!({ "brain": "hermes", "usage": usage, "runtime": runtime }), context: 0, window: 200_000, compacted: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::{Path as AxPath, Request, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};

    #[test]
    fn addresses() {
        for (raw, want) in [
            ("https://hermes.example.com", "https://hermes.example.com"),
            ("https://hermes.example.com/", "https://hermes.example.com"),
            ("https://hermes.example.com/v1/", "https://hermes.example.com"),
            ("https://hermes.example.com/p/work", "https://hermes.example.com/p/work"),
            ("hermes.example.com", "https://hermes.example.com"),
            ("192.168.1.20:8642", "http://192.168.1.20:8642"),
            ("http://192.168.1.20:8642", "http://192.168.1.20:8642"),
            ("http://100.101.102.103:8642", "http://100.101.102.103:8642"),
            ("http://box.tail1234.ts.net:8642", "http://box.tail1234.ts.net:8642"),
            ("http://homelab:8642", "http://homelab:8642"),
            ("homelab.local:8642", "http://homelab.local:8642"),
            ("http://[fd00::1]:8642", "http://[fd00::1]:8642"),
        ] {
            assert_eq!(clean_url(raw).unwrap(), want, "{raw}");
        }
        for bad in [
            "",
            "   ",
            "ftp://192.168.1.20",
            "http://hermes.example.com",
            "http://8.8.8.8:8642",
            "http://100.128.0.1:8642",
            "https://user:pass@hermes.example.com",
            "https://hermes.example.com?key=1",
            "https://hermes.example.com#k",
        ] {
            assert!(clean_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn server_sent_events() {
        let mut sse = Sse::default();
        assert_eq!(sse.line(": open"), Some(Got::Alive));
        assert_eq!(sse.line(""), None);
        assert_eq!(sse.line("id: 3"), None);
        assert_eq!(sse.line("data: {\"event\": \"message.delta\","), None);
        assert_eq!(sse.line("data:\"seq\": 3}"), None);
        assert_eq!(sse.line(""), Some(Got::Event(json!({ "event": "message.delta", "seq": 3 }))));
        // Not JSON: passed over.
        assert_eq!(sse.line("data: nope"), None);
        assert_eq!(sse.line(""), None);
    }

    #[test]
    fn what_events_mean() {
        assert_eq!(step_of(&json!({ "event": "message.delta", "delta": "Hi" })), Step::Say("Hi".into()));
        assert_eq!(step_of(&json!({ "event": "message.delta", "delta": "" })), Step::Nothing);
        assert_eq!(step_of(&json!({ "event": "message.interim", "text": "Looking", "already_streamed": true })), Step::Nothing);
        assert_eq!(step_of(&json!({ "event": "message.interim", "text": "Looking", "already_streamed": false })), Step::Say("Looking\n\n".into()));
        assert_eq!(step_of(&json!({ "event": "tool.started", "tool": "terminal", "preview": "ls" })), Step::Tool("Bash".into()));
        assert_eq!(step_of(&json!({ "event": "tool.started", "tool": "todo" })), Step::Tool("todo".into()));
        assert_eq!(step_of(&json!({ "event": "subagent.start" })), Step::Nothing);
        assert_eq!(step_of(&json!({ "event": "something.new" })), Step::Nothing);
        assert_eq!(step_of(&json!({ "no": "name" })), Step::Nothing);
        let Step::End(e) = step_of(&json!({ "event": "run.completed", "output": "Done.", "usage": { "total_tokens": 3 } })) else { panic!() };
        assert_eq!((e.how, e.output.as_str(), e.usage["total_tokens"].as_i64()), (How::Completed, "Done.", Some(3)));
        let Step::End(e) = step_of(&json!({ "event": "run.failed", "error": "no credits" })) else { panic!() };
        assert_eq!((e.how, e.error.as_str()), (How::Failed, "no credits"));
    }

    #[test]
    fn answers_name_what_they_answer() {
        // With the session authority: exactly these fields, or Hermes refuses it.
        let Step::Approve(a) = step_of(&json!({ "event": "approval.request", "kind": "approval", "prompt_id": "p1", "execution_generation": 3, "command": "rm -rf build", "description": "delete", "choices": ["once", "deny"] })) else { panic!() };
        assert_eq!(a.command, "rm -rf build");
        assert_eq!(a.answer("choice", "once"), json!({ "request_id": "p1", "execution_generation": 3, "choice": "once" }));
        // Without it: its request id, or nothing.
        let Step::Approve(a) = step_of(&json!({ "event": "approval.request", "request_id": "r9", "command": "ls" })) else { panic!() };
        assert_eq!(a.answer("choice", "deny"), json!({ "request_id": "r9", "choice": "deny" }));
        let Step::Approve(a) = step_of(&json!({ "event": "approval.request", "command": "ls" })) else { panic!() };
        assert_eq!(a.answer("choice", "deny"), json!({ "choice": "deny" }));
        let Step::Clarify(a) = step_of(&json!({ "event": "clarify.request", "prompt_id": "q1", "execution_generation": 4, "question": "Which?" })) else { panic!() };
        assert_eq!(a.answer("answer", "guess"), json!({ "request_id": "q1", "execution_generation": 4, "answer": "guess" }));
    }

    #[test]
    fn how_a_run_comes_out() {
        let ended = |how, output: &str, error: &str| Ended { how, output: output.into(), error: error.into(), usage: Value::Null, runtime: Value::Null };
        let run = |ended, said: &str, stopped| Followed { ended, said: said.into(), stopped };
        assert_eq!(finish(run(ended(How::Completed, " Done. ", ""), "ignored", false)).unwrap().0, "Done.");
        assert_eq!(finish(run(ended(How::Completed, "", ""), "What it said", false)).unwrap().0, "What it said");
        assert!(finish(run(ended(How::Completed, "", ""), "", false)).is_err());
        // Out of steps: its wrap-up is still an answer.
        assert_eq!(finish(run(ended(How::Failed, "As far as I got", ""), "", false)).unwrap().0, "As far as I got");
        assert!(format!("{:#}", finish(run(ended(How::Failed, "", "no credits"), "", false)).unwrap_err()).contains("no credits"));
        assert!(format!("{:#}", finish(run(ended(How::Interrupted, "", ""), "", false)).unwrap_err()).contains("shut down"));
        for r in [run(ended(How::Cancelled, "", ""), "So far", false), run(ended(How::Completed, "", ""), "So far", true)] {
            let e = finish(r).unwrap_err();
            let s = e.downcast_ref::<crate::stops::Stopped>().expect("a stop");
            assert_eq!(s.said, "So far");
        }
    }

    #[test]
    fn it_hears_who_it_is_and_not_r1ds_tools() {
        let pip = Agent { name: "Pip".into(), role: "the garden".into(), custom: "Say hi in Spanish.".into(), ..Agent::default() };
        let bolt = Agent { name: "Bolt".into(), ..Agent::default() };
        let m = |text: &str, shared| Memory { id: 1, text: text.into(), at: 0, shared, agent: None, by: None };
        let said = instructions(&pip, &[bolt], &[m("Sam likes tea", true), m("Tomatoes in bed two", false)], "Europe/Lisbon");
        for part in ["your name is Pip", "You are not a device", "the garden", "Say hi in Spanish.", "Europe/Lisbon", "other agents on the R1 can use them: Bolt", "- Sam likes tea", "- Tomatoes in bed two"] {
            assert!(said.contains(part), "{part} in {said}");
        }
        for tool in ["show_card", "ask_agent", "mcp__r1__", "remember"] {
            assert!(!said.contains(tool), "{tool}");
        }
    }

    #[test]
    fn the_key_stays_outside_the_database_and_private() {
        let dir = std::env::temp_dir().join(format!("r1d-hermes-{}-{}", std::process::id(), crate::core::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let h = Hermes::new(&dir);
        assert_eq!(h.url(), None);
        h.keep(Some(Gateway { url: "https://h.example".into(), key: "s3cret-s3cret-s3cret".into() })).unwrap();
        assert_eq!(Hermes::new(&dir).url().as_deref(), Some("https://h.example"), "back after a restart");
        assert_eq!(Hermes::new(&dir).gateway().unwrap().key, "s3cret-s3cret-s3cret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("hermes.json")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(crate::backup::read(&dir, "hermes.json", 0, 10).is_err(), "not in a backup");
        h.keep(None).unwrap();
        h.keep(None).unwrap();
        assert_eq!(Hermes::new(&dir).url(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- against a stand-in gateway, over real sockets --------------------------------------------------------

    const KEY: &str = "0123456789abcdef";

    /// A gateway that plays a script: each connection to the events gets the next list of frames, then the stream ends.
    #[derive(Default)]
    struct Script {
        streams: Vec<Vec<Value>>,
        last_ids: Vec<Option<String>>,
        status: Value,
        created: Vec<(Value, Option<String>)>,
        answers: Vec<(String, Value)>,
        stops: u32,
    }

    type Shared = Arc<Mutex<Script>>;

    fn authed(h: &HeaderMap) -> bool {
        h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {KEY}"))
    }

    async fn create(State(s): State<Shared>, h: HeaderMap, body: String) -> Response {
        if !authed(&h) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let idem = h.get("idempotency-key").and_then(|v| v.to_str().ok()).map(str::to_owned);
        s.lock().unwrap().created.push((serde_json::from_str(&body).unwrap(), idem));
        (StatusCode::ACCEPTED, axum::Json(json!({ "run_id": "run_1", "status": "started" }))).into_response()
    }

    async fn events(State(s): State<Shared>, h: HeaderMap) -> Response {
        if !authed(&h) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let mut s = s.lock().unwrap();
        s.last_ids.push(h.get("last-event-id").and_then(|v| v.to_str().ok()).map(str::to_owned));
        let frames = if s.streams.is_empty() { Vec::new() } else { s.streams.remove(0) };
        let mut body = String::from(": open\n\n");
        for f in frames {
            body.push_str(&format!("id: {}\ndata: {f}\n\n", f["seq"]));
        }
        ([("content-type", "text/event-stream")], body).into_response()
    }

    async fn status(State(s): State<Shared>) -> Response {
        axum::Json(s.lock().unwrap().status.clone()).into_response()
    }

    async fn answer(State(s): State<Shared>, AxPath(path): AxPath<(String, String)>, req: Request) -> Response {
        let bytes = axum::body::to_bytes(req.into_body(), 1 << 20).await.unwrap();
        let mut s = s.lock().unwrap();
        if path.1 == "stop" {
            s.stops += 1;
        } else {
            s.answers.push((path.1, serde_json::from_slice(&bytes).unwrap()));
        }
        axum::Json(json!({ "ok": true })).into_response()
    }

    async fn gateway_with(script: Script) -> (Gateway, Shared) {
        let shared: Shared = Arc::new(Mutex::new(script));
        let app = Router::new()
            .route("/v1/runs", post(create))
            .route("/v1/runs/{id}", get(status))
            .route("/v1/runs/{id}/events", get(events))
            .route("/v1/runs/{id}/{action}", post(answer))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (Gateway { url: format!("http://{addr}"), key: KEY.into() }, shared)
    }

    /// Remembers what a turn showed, and answers cards as it's told.
    struct Seen {
        allow: bool,
        said: Mutex<String>,
        tools: Mutex<Vec<String>>,
        cards: Mutex<Vec<Asked>>,
    }

    impl Seen {
        fn new(allow: bool) -> Arc<Self> {
            Arc::new(Seen { allow, said: Mutex::default(), tools: Mutex::default(), cards: Mutex::default() })
        }
    }

    impl Ui for Seen {
        fn say(&self, text: &str) {
            self.said.lock().unwrap().push_str(text);
        }

        fn tool(&self, name: String) {
            self.tools.lock().unwrap().push(name);
        }

        fn allow(&self, asked: &Asked) -> Pin<Box<dyn Future<Output = bool> + Send>> {
            self.cards.lock().unwrap().push(asked.clone());
            let allow = self.allow;
            Box::pin(async move { allow })
        }
    }

    fn ev(seq: u64, name: &str, rest: Value) -> Value {
        let mut v = json!({ "event": name, "run_id": "run_1", "seq": seq });
        v.as_object_mut().unwrap().extend(rest.as_object().unwrap().clone());
        v
    }

    /// Waits for what a background answer posts.
    async fn answered(shared: &Shared) -> Vec<(String, Value)> {
        for _ in 0..100 {
            let got = shared.lock().unwrap().answers.clone();
            if !got.is_empty() {
                return got;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Vec::new()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_streams_to_its_end() {
        let (gw, shared) = gateway_with(Script {
            streams: vec![vec![
                ev(0, "message.delta", json!({ "delta": "Let me " })),
                ev(1, "message.delta", json!({ "delta": "look" })),
                ev(2, "tool.started", json!({ "tool": "terminal", "preview": "ls" })),
                ev(3, "tool.completed", json!({ "tool": "terminal", "error": false })),
                ev(4, "message.delta", json!({ "delta": "Found it" })),
                ev(5, "run.completed", json!({ "output": "Found it.", "usage": { "total_tokens": 9 } })),
            ]],
            ..Script::default()
        })
        .await;
        let run = start(&gw, json!({ "input": "hi", "session_id": "r1os-c1", "instructions": "be brief" })).await.unwrap();
        assert_eq!(run, "run_1");
        let seen = Seen::new(true);
        let out = follow(&gw, &run, seen.clone(), None).await.unwrap();
        assert_eq!(seen.said.lock().unwrap().as_str(), "Let me lookFound it");
        assert_eq!(seen.tools.lock().unwrap().as_slice(), ["Bash"]);
        assert_eq!(out.said, "Found it", "since its last tool");
        let (text, usage, _) = finish(out).unwrap();
        assert_eq!((text.as_str(), usage["total_tokens"].as_i64()), ("Found it.", Some(9)));
        let s = shared.lock().unwrap();
        let (body, idem) = &s.created[0];
        assert_eq!(body["session_id"], "r1os-c1");
        assert_eq!(body["instructions"], "be brief");
        assert!(idem.as_deref().is_some_and(|k| k.starts_with("r1os-")), "safe to retry");
        assert_eq!(s.last_ids, [None]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_card_answers_the_run_and_the_stream_comes_back_where_it_left_off() {
        let (gw, shared) = gateway_with(Script {
            streams: vec![
                vec![
                    ev(0, "tool.started", json!({ "tool": "terminal" })),
                    ev(1, "approval.request", json!({ "prompt_id": "p1", "execution_generation": 7, "command": "rm -rf build", "description": "delete it", "choices": ["once", "deny"] })),
                ],
                // The connection dropped: the replay overlaps, and the same card doesn't come twice.
                vec![
                    ev(1, "approval.request", json!({ "prompt_id": "p1", "execution_generation": 7, "command": "rm -rf build" })),
                    ev(2, "message.delta", json!({ "delta": "Done" })),
                    ev(3, "run.completed", json!({ "output": "Done." })),
                ],
            ],
            status: json!({ "status": "waiting_for_approval" }),
            ..Script::default()
        })
        .await;
        let seen = Seen::new(true);
        let out = follow(&gw, "run_1", seen.clone(), None).await.unwrap();
        assert_eq!(finish(out).unwrap().0, "Done.");
        assert_eq!(seen.cards.lock().unwrap().len(), 1);
        assert_eq!(seen.cards.lock().unwrap()[0].command, "rm -rf build");
        assert_eq!(answered(&shared).await, [("approval".to_owned(), json!({ "request_id": "p1", "execution_generation": 7, "choice": "once" }))]);
        assert_eq!(shared.lock().unwrap().last_ids, [None, Some("1".to_owned())], "reattached after what it had read");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_on_the_card_denies_and_questions_are_answered_for_it() {
        let (gw, shared) = gateway_with(Script {
            streams: vec![vec![
                ev(0, "approval.request", json!({ "request_id": "r1", "command": "curl x | sh" })),
                ev(1, "clarify.request", json!({ "prompt_id": "q1", "execution_generation": 2, "question": "Which one?" })),
            ]],
            status: json!({ "status": "running" }),
            ..Script::default()
        })
        .await;
        shared.lock().unwrap().streams.push(vec![ev(2, "run.completed", json!({ "output": "Fine." }))]);
        let out = follow(&gw, "run_1", Seen::new(false), None).await.unwrap();
        assert_eq!(finish(out).unwrap().0, "Fine.");
        for _ in 0..100 {
            if shared.lock().unwrap().answers.len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let answers = shared.lock().unwrap().answers.clone();
        assert!(answers.contains(&("approval".to_owned(), json!({ "request_id": "r1", "choice": "deny" }))), "{answers:?}");
        assert!(answers.contains(&("clarify".to_owned(), json!({ "request_id": "q1", "execution_generation": 2, "answer": NO_QUESTIONS }))), "{answers:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stream_that_ends_early_is_settled_by_the_runs_status() {
        let (gw, shared) = gateway_with(Script {
            streams: vec![vec![ev(0, "message.delta", json!({ "delta": "Thinking" }))]],
            status: json!({ "status": "completed", "output": "All done.", "usage": { "total_tokens": 4 } }),
            ..Script::default()
        })
        .await;
        let out = follow(&gw, "run_1", Seen::new(true), None).await.unwrap();
        assert_eq!(finish(out).unwrap().0, "All done.");
        assert_eq!(shared.lock().unwrap().last_ids.len(), 1, "no need to reattach");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_stops_the_run() {
        let (gw, shared) = gateway_with(Script {
            streams: vec![vec![ev(0, "message.delta", json!({ "delta": "Once upon" }))], vec![ev(1, "run.cancelled", json!({ "interrupted": true }))]],
            status: json!({ "status": "stopping" }),
            ..Script::default()
        })
        .await;
        let stop = Arc::new(Notify::new());
        stop.notify_one();
        let out = follow(&gw, "run_1", Seen::new(true), Some(stop)).await.unwrap();
        assert!(out.stopped);
        let e = finish(out).unwrap_err();
        assert_eq!(e.downcast_ref::<crate::stops::Stopped>().expect("a stop").said, "Once upon");
        assert_eq!(shared.lock().unwrap().stops, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_key_says_so() {
        let (mut gw, _) = gateway_with(Script::default()).await;
        gw.key = "wrong-wrong-wrong-wrong".into();
        let e = start(&gw, json!({ "input": "hi" })).await.unwrap_err();
        assert!(format!("{e:#}").contains("refused r1os's key"), "{e:#}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_gateway_it_cant_reach_says_so() {
        // Nothing listens on the port this one had.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let gw = Gateway { url: url.clone(), key: KEY.into() };
        let e = start(&gw, json!({ "input": "hi" })).await.unwrap_err();
        assert!(format!("{e:#}").contains(&url), "{e:#}");
        assert!(!format!("{e:#}").contains(KEY), "never the key");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_lost_in_a_restart_is_settled() {
        // As a real v0.22.7 gateway reports it after a restart: not terminal, and it holds the session until resolved.
        let (gw, shared) = gateway_with(Script { status: json!({ "run_id": "run_1", "status": "unknown", "admission_id": "adm_1", "execution_generation": 7 }), ..Script::default() }).await;
        let g = gw.clone();
        assert!(tokio::task::spawn_blocking(move || settle_run(&g, "run_1")).await.unwrap());
        assert_eq!(shared.lock().unwrap().answers, [("resolve-unknown".to_owned(), json!({ "admission_id": "adm_1", "execution_generation": 7 }))]);

        // Ended anyway: nothing to tell it.
        shared.lock().unwrap().answers.clear();
        for status in ["completed", "failed", "cancelled", "interrupted"] {
            shared.lock().unwrap().status = json!({ "status": status });
            let g = gw.clone();
            assert!(tokio::task::spawn_blocking(move || settle_run(&g, "run_1")).await.unwrap(), "{status}");
        }
        assert!(shared.lock().unwrap().answers.is_empty());

        // Still going: the next turn queues behind it, and it's looked at again after.
        shared.lock().unwrap().status = json!({ "status": "running" });
        let g = gw.clone();
        assert!(!tokio::task::spawn_blocking(move || settle_run(&g, "run_1")).await.unwrap());
    }

    #[test]
    fn a_stuck_session_is_told_apart_and_its_chat_carries_on() {
        assert!(paused("unknown_execution"));
        assert!(paused("Your Hermes gateway answered HTTP 409: unknown_execution"));
        assert!(!paused("no credits left"));
        let told = with_recap("hello", Some("The user: hi\nYou: hey"));
        assert!(told.contains("fresh session") && told.ends_with("]\nThe user: hi\nYou: hey\nhello"), "{told}");
        assert!(with_recap("hello", None).ends_with("fresh session.]\nhello"));
    }

    /// A stand-in `/v1/capabilities`.
    async fn with_capabilities(status: StatusCode, caps: Value) -> Gateway {
        let app = Router::new().route("/v1/capabilities", get(move || async move { (status, axum::Json(caps)) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Gateway { url: format!("http://{addr}"), key: KEY.into() }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connecting_checks_its_a_gateway_we_can_drive() {
        let endpoints: serde_json::Map<String, Value> = NEEDS.iter().map(|n| ((*n).to_owned(), json!({ "method": "POST", "path": "/x" }))).collect();
        let good = json!({ "platform": "hermes-agent", "endpoints": endpoints });
        let run = |g: Gateway| async move { tokio::task::spawn_blocking(move || check(&g)).await.unwrap() };
        run(with_capabilities(StatusCode::OK, good.clone()).await).await.unwrap();
        let cases = [
            (StatusCode::UNAUTHORIZED, json!({}), "refused that key"),
            (StatusCode::NOT_FOUND, json!({}), "API_SERVER_ENABLED"),
            (StatusCode::OK, json!({ "platform": "something-else" }), "isn't a Hermes gateway"),
            (StatusCode::OK, json!({ "platform": "hermes-agent", "endpoints": { "runs": {} } }), "update it"),
        ];
        for (status, caps, why) in cases {
            let e = run(with_capabilities(status, caps).await).await.unwrap_err();
            assert!(format!("{e:#}").contains(why), "{e:#}");
        }
    }
}
