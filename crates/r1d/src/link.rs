//! Linked computers (ADR-0048, docs/specs/companion.md §3–4).
//!
//! - **The R1 only dials.** Its endpoint takes no connections (no ALPN). Its key is `<data>/link/key` (root-only, like
//!   the rest of `/data/r1os`); the computers it trusts are in core.db (`computers`): their keys and names.
//! - **When it's up** (ADR-0049, which replaces ADR-0048's screen-only rule): while the screen's on, r1d dials each
//!   computer it trusts and keeps the connection ([`Link::display`]), looking again every [`KEEP`] for one that went
//!   (the companion restarted, the network changed); a computer that didn't answer isn't tried again for a minute.
//!   With the screen off a connection stays as long as something's going on with that computer: its calls, the R1's
//!   events it follows (not widgets refreshing), an agent's turn on it ([`Link::busy`]). After that computer's idle
//!   time (its companion sets it: [`DEFAULT_IDLE`] until it has; 0 is "only while the screen's on", gone [`LINGER`]
//!   after) the R1 lets it go, and with none left the endpoint too: an idle link keeps Wi-Fi awake and costs ~0.8% of
//!   a core, mostly iroh's network checks (~2% before an endpoint kept to one relay, [`Link::endpoint`]; T-035).
//! - **Pairing, in two steps** ([`Link::begin_pair`]): a QR the camera read ([`decode_qr`]) dials the computer, and
//!   the four-digit code shows at once (both sides work it out). The computer asks "Link this R1?", and the answer
//!   comes back as `link {state}`.
//! - **The companion's calls** (`api {method, path, body}`): run through r1d's own router with the per-boot token, for
//!   an allowlist of routes ([`allowed`]): the R1's Settings powers, no more. Never the harness, the motor, Android,
//!   answering approvals, tokens or the link itself. Only a computer this R1 trusts gets an answer.
//!   - A body is JSON, or (`base64`, `mime`) bytes: a photo dropped into the companion.
//!   - Granting an app a permission waits for your tap on the R1's approval card, as an agent's request does
//!     (ADR-0045): it's still the R1's own screen that grants (ADR-0023). Taking one away doesn't ask.
//!   - A change made from a computer is noted on the R1 (`changed_from {computer, what}`, a word at the top), at most
//!     once every [`NOTE_EVERY`] for the same thing.
//! - **Agents that work on a computer** (crate::remote): r1d sends each turn (`turn`) and reads the CLI's output as it
//!   streams back; the agent's tool calls come back as `mcp {token, body}` (answered as that agent, only for an agent
//!   on this computer with a live turn's token), and its questions as `approve {token, area, title, detail}` (the R1's
//!   approval card, as its levels say).
//! - **Events** the companion follows ([`FORWARD`]) go to every connected computer ([`forward_events`]).

use crate::hub::{Event, Hub, Source};
use anyhow::{Context, Result, anyhow};
use r1_link::{Connection, Endpoint, EndpointAddr, EndpointId, Msg, PairingCode, Reach};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A computer this R1 trusts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Computer {
    pub id: String,
    pub name: String,
    pub added: i64,
    /// How long its link stays with nothing going on (seconds; its companion's setting). None: [`DEFAULT_IDLE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_s: Option<u64>,
}

/// The least a link stays after the screen goes off.
const LINGER: Duration = Duration::from_secs(30);
/// How long the relay an endpoint settled on is used without looking at every relay again (the R1 may have moved).
const NEAR_FOR: Duration = Duration::from_secs(12 * 3600);
/// How long a link stays with nothing going on, until the computer's companion has said.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(5 * 60);
/// Events that don't count as something going on: widgets refreshing themselves in the background.
const QUIET: &[&str] = &["glance"];
/// A computer that didn't answer isn't dialled again before this.
const RETRY: Duration = Duration::from_secs(60);
/// While the screen's on, how often r1d looks for a computer that isn't connected.
const KEEP: Duration = Duration::from_secs(20);
/// DNS servers until the host has said the network's (Quad9, Cloudflare).
const FALLBACK_DNS: &[&str] = &["9.9.9.9", "1.1.1.1"];
/// The events a linked computer follows.
const FORWARD: &[&str] = &[
    "agents_changed", "agent_state", "unread", "look", "home", "glance", "alarms_changed", "timers_changed", "app_installed", "app_removed",
    "approval_asked", "approval_done", "device_set", "account", "media_changed", "journal_changed",
    // Chats, live in the companion (ADR-0051): replies as they stream, tool steps, cards, builds.
    "chat", "build", "routines_changed", "connections_changed",
];
/// The header r1d's routes see on a linked computer's calls: its name (a message typed there says so to the agent).
pub const FROM_COMPUTER: &str = "x-r1os-computer";
/// A computer's changes to the same thing are noted on the R1 at most this often (a level being dragged).
const NOTE_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Link {
    ep: tokio::sync::Mutex<Option<(Reach, Endpoint)>>,
    conns: Mutex<HashMap<String, Connection>>,
    /// r1d's own router, for the companion's calls (set as the server starts).
    router: OnceLock<axum::Router>,
    /// When the screen went off; none while it's on.
    screen_off: Mutex<Option<Instant>>,
    /// The keeper's running (one at a time): it dials while the screen's on and lets idle links go.
    keeping: std::sync::atomic::AtomicBool,
    /// Per computer: when something last went on with it, and how many agents' turns it's running.
    activity: Mutex<HashMap<String, (Instant, usize)>>,
    failed: Mutex<HashMap<String, Instant>>,
    /// When each computer's change to each thing was last noted on the R1.
    noted: Mutex<HashMap<(String, &'static str), Instant>>,
    /// The relay an endpoint with every relay settled on, and when: the next binds with only it (T-035).
    near: Arc<Mutex<Option<(String, Instant)>>>,
}

impl Link {
    pub fn set_router(&self, router: axum::Router) {
        let _ = self.router.set(router);
    }

    /// The screen went on (the computers are dialled) or off (each link goes once nothing's going on with it).
    pub fn display(hub: &Arc<Hub>, on: bool) {
        if hub.core.computers().is_empty() {
            return;
        }
        *hub.link.screen_off.lock().unwrap() = if on { None } else { Some(Instant::now()) };
        Self::keep(hub);
    }

    /// Starts the keeper, unless it's running: while the screen's on it dials what isn't connected; it lets a link go
    /// once nothing's gone on with it for its idle time, and stops (letting the endpoint go) when none are left.
    fn keep(hub: &Arc<Hub>) {
        use std::sync::atomic::Ordering;
        if hub.link.keeping.swap(true, Ordering::AcqRel) {
            return;
        }
        let h = hub.clone();
        tokio::spawn(async move {
            loop {
                let on = h.link.screen_off.lock().unwrap().is_none();
                if on {
                    h.link.connect_all(&h).await;
                }
                for c in h.core.computers() {
                    if h.link.live(&c.id).is_some() && h.link.idle_out(&c) {
                        if let Some(conn) = h.link.conns.lock().unwrap().remove(&c.id) {
                            conn.close(0u32.into(), b"idle");
                        }
                        tracing::info!(computer = %c.name, "link: let go (nothing going on)");
                    }
                }
                if !on {
                    // Under the endpoint's lock, so a turn dialling out now waits and binds a new one after.
                    let mut slot = h.link.ep.lock().await;
                    let off = h.link.screen_off.lock().unwrap().is_some();
                    let none = h.link.conns.lock().unwrap().values().all(|c| c.close_reason().is_some());
                    if off && none {
                        h.link.conns.lock().unwrap().clear();
                        if let Some((_, ep)) = slot.take() {
                            ep.close().await;
                        }
                        h.link.keeping.store(false, Ordering::Release);
                        tracing::info!("link: let go (the screen's off and nothing's going on)");
                        return;
                    }
                }
                tokio::time::sleep(KEEP.min(LINGER) / 2).await;
            }
        });
    }

    /// Whether a computer's link has been idle long enough to let go (never while the screen's on or a turn runs).
    fn idle_out(&self, c: &Computer) -> bool {
        let Some(off) = *self.screen_off.lock().unwrap() else { return false };
        let (last, busy) = self.activity.lock().unwrap().get(&c.id).copied().unwrap_or((off, 0));
        if busy > 0 {
            return false;
        }
        match c.idle_s.map(Duration::from_secs).unwrap_or(DEFAULT_IDLE) {
            Duration::ZERO => off.elapsed() >= LINGER,
            idle => off.max(last).elapsed() >= idle.max(LINGER),
        }
    }

    /// Something went on with a computer (its call, an event it follows, a turn's progress).
    pub fn touch(&self, computer: &str) {
        let mut a = self.activity.lock().unwrap();
        let e = a.entry(computer.to_owned()).or_insert((Instant::now(), 0));
        e.0 = Instant::now();
    }

    /// An agent's turn on a computer started (`true`) or ended: its link stays meanwhile, whatever the screen.
    pub fn busy(&self, computer: &str, started: bool) {
        let mut a = self.activity.lock().unwrap();
        let e = a.entry(computer.to_owned()).or_insert((Instant::now(), 0));
        e.0 = Instant::now();
        e.1 = if started { e.1 + 1 } else { e.1.saturating_sub(1) };
    }

    /// A connection to a computer this R1 trusts, dialled now if it isn't up (an agent's turn there, with the screen
    /// off): kept by the keeper from then on.
    pub async fn reach(&self, hub: &Arc<Hub>, computer: &str) -> Result<Connection> {
        let conn = self.connect(hub, computer, None, Reach::Anywhere).await?;
        self.touch(computer);
        Self::keep(hub);
        Ok(conn)
    }

    /// The companion's settings for its link: `{idle_s}` (0: only while the screen's on).
    fn set(&self, hub: &Hub, computer: &str, p: &Value) -> Result<Value> {
        let mut all = hub.core.computers();
        let c = all.iter_mut().find(|c| c.id == computer).context("this R1 doesn't trust that computer")?;
        if let Some(s) = p["idle_s"].as_u64() {
            c.idle_s = Some(s.min(24 * 3600));
        }
        let idle_s = c.idle_s;
        hub.core.set_computers(&all)?;
        Ok(json!({ "idle_s": idle_s }))
    }

    async fn connect_all(&self, hub: &Arc<Hub>) {
        for c in hub.core.computers() {
            if self.live(&c.id).is_some() || self.failed.lock().unwrap().get(&c.id).is_some_and(|t| t.elapsed() < RETRY) {
                continue;
            }
            match self.connect(hub, &c.id, None, Reach::Anywhere).await {
                Ok(conn) => tracing::info!(computer = %c.name, path = r1_link::path_kind(&conn), "link: connected"),
                Err(e) => {
                    self.failed.lock().unwrap().insert(c.id.clone(), Instant::now());
                    tracing::info!(computer = %c.name, error = %format!("{e:#}"), "link: not reachable");
                }
            }
        }
    }

    fn live(&self, id: &str) -> Option<Connection> {
        self.conns.lock().unwrap().get(id).filter(|c| c.close_reason().is_none()).cloned()
    }

    /// Lets go of a computer's connection that turned out to be gone (the next call dials again).
    pub fn forget(&self, id: &str) {
        if let Some(c) = self.conns.lock().unwrap().remove(id) {
            c.close(0u32.into(), b"gone");
        }
    }

    /// Whether a computer is connected now.
    pub fn connected(&self, id: &str) -> bool {
        self.live(id).is_some()
    }

    /// The endpoint, bound now if it isn't (a different reach binds it again).
    async fn endpoint(&self, hub: &Hub, reach: Reach) -> Result<Endpoint> {
        let mut slot = self.ep.lock().await;
        if let Some((r, ep)) = slot.as_ref()
            && *r == reach
            && !ep.is_closed()
        {
            return Ok(ep.clone());
        }
        if let Some((_, old)) = slot.take() {
            self.conns.lock().unwrap().clear();
            old.close().await;
        }
        let key = r1_link::load_or_create_key(&hub.cfg.data.join("link").join("key"))?;
        // The network's DNS servers, as the host reports them; public ones until it has.
        let mut dns = hub.device_settings.dns();
        if dns.is_empty() {
            dns = FALLBACK_DNS.iter().filter_map(|s| s.parse().ok()).collect();
        }
        // Only the relay the last endpoint with every relay settled on, if that was recent (T-035): an idle link's
        // network checks then cost one handshake, not four. Otherwise every relay, and remember the one it picks.
        let near = self.near.lock().unwrap().clone().filter(|(_, at)| at.elapsed() < NEAR_FOR).map(|(url, _)| url);
        let ep = r1_link::bind_near(key, reach, false, r1_link::Dns::Servers(dns), near.as_deref()).await?;
        tracing::info!(id = %ep.id(), ?reach, relay = ?near, "link: endpoint up");
        if near.is_none() {
            let (e, remember) = (ep.clone(), self.near.clone());
            tokio::spawn(async move {
                for _ in 0..60 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if e.is_closed() {
                        return;
                    }
                    if let Some(url) = r1_link::home_relay(&e) {
                        tracing::info!(%url, "link: the nearest relay");
                        *remember.lock().unwrap() = Some((url, Instant::now()));
                        return;
                    }
                }
            });
        }
        *slot = Some((reach, ep.clone()));
        Ok(ep)
    }

    /// A connection to `id`, open already or dialled now (by key: iroh finds it, or a relay carries it). A new one is
    /// served: the computer's calls are answered on it.
    async fn connect(&self, hub: &Arc<Hub>, id: &str, addr: Option<EndpointAddr>, reach: Reach) -> Result<Connection> {
        // The endpoint first: another reach binds a new one, which drops the old one's connections.
        let ep = self.endpoint(hub, reach).await?;
        if let Some(c) = self.live(id) {
            return Ok(c);
        }
        let key: EndpointId = id.parse().map_err(|e| anyhow!("bad key: {e}"))?;
        let addr = addr.unwrap_or_else(|| EndpointAddr::from_parts(key, std::iter::empty()));
        let conn = tokio::time::timeout(Duration::from_secs(20), ep.connect(addr, r1_link::ALPN))
            .await
            .map_err(|_| anyhow!("the computer didn't answer in 20 s (is the companion running?)"))?
            .map_err(|e| anyhow!("couldn't reach the computer: {e}"))?;
        self.conns.lock().unwrap().insert(id.to_owned(), conn.clone());
        self.failed.lock().unwrap().remove(id);
        self.touch(id);
        serve(hub.clone(), id.to_owned(), conn.clone());
        presence(hub, id);
        // What waited for this computer's agents goes now.
        crate::remote::flush(hub, id);
        Ok(conn)
    }

    /// Pairing, step one: dials the computer whose QR the camera read and works out the code both screens show. The
    /// computer asks; its answer comes as `link {state: paired | refused | failed}`.
    pub async fn begin_pair(&self, hub: &Arc<Hub>, uri: &str) -> Result<Value> {
        let code = PairingCode::parse(uri)?;
        let id = code.id.to_string();
        let conn = self.connect(hub, &id, Some(code.addr()), Reach::Anywhere).await?;
        let shown = r1_link::short_code(&conn.remote_id(), &self.endpoint(hub, Reach::Anywhere).await?.id(), &code.secret);
        let (h, name, ours, path) = (hub.clone(), code.name.clone(), shown.clone(), r1_link::path_kind(&conn));
        let named = name.clone();
        tokio::spawn(async move {
            let msg = Msg::Pair { secret: r1_link::secret_hex(&code.secret), name: r1_name(), version: r1_link::VERSION };
            let (state, why, computer) = match tokio::time::timeout(Duration::from_secs(120), r1_link::ask(&conn, &msg)).await {
                Ok(Ok(Msg::Paired { name: n, code: c })) if c == ours => ("paired", None, if n.is_empty() { name.clone() } else { n }),
                Ok(Ok(Msg::Paired { code: c, .. })) => ("refused", Some(format!("the codes don't match ({ours} here, {c} there)")), name.clone()),
                Ok(Ok(Msg::Refused { why })) => ("refused", Some(why), name.clone()),
                Ok(Ok(other)) => ("failed", Some(format!("unexpected {other:?}")), name.clone()),
                Ok(Err(e)) => ("failed", Some(format!("{e:#}")), name.clone()),
                Err(_) => ("failed", Some("nobody answered on the computer".into()), name.clone()),
            };
            if state == "paired" {
                let mut all = h.core.computers();
                all.retain(|c| c.id != code.id.to_string());
                all.push(Computer { id: code.id.to_string(), name: computer.clone(), added: crate::core::now(), idle_s: None });
                if let Err(e) = h.core.set_computers(&all) {
                    tracing::warn!(error = %e, "link: couldn't keep the computer");
                }
            } else if let Some(c) = h.link.conns.lock().unwrap().remove(&code.id.to_string()) {
                c.close(1u32.into(), b"not paired");
            }
            tracing::info!(computer = %computer, state, "link: pairing");
            h.emit(Event::Link { state: state.into(), name: computer, code: ours, why }, Source::Api);
        });
        Ok(json!({ "id": id, "name": named, "code": shown, "state": "confirming", "path": path }))
    }

    /// `n` round trips to a computer (the first one it trusts, if not named): the times, and the path they took.
    pub async fn ping(&self, hub: &Arc<Hub>, id: Option<&str>, n: usize, reach: Reach) -> Result<Value> {
        let all = hub.core.computers();
        let c = match id {
            Some(id) => all.iter().find(|c| c.id == id || c.name.eq_ignore_ascii_case(id)),
            None => all.first(),
        }
        .ok_or_else(|| anyhow!("no linked computer"))?
        .clone();
        let t = Instant::now();
        let conn = self.connect(hub, &c.id, None, reach).await?;
        let connect_ms = t.elapsed().as_millis();
        let mut rtts = Vec::new();
        let mut paths = Vec::new();
        for i in 0..n.clamp(1, 50) {
            let t = Instant::now();
            r1_link::call(&conn, "ping", json!(i)).await?;
            rtts.push(t.elapsed().as_secs_f64() * 1000.0);
            paths.push(r1_link::path_kind(&conn));
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let hello = r1_link::call(&conn, "hello", Value::Null).await.unwrap_or(Value::Null);
        Ok(json!({
            "computer": c.name, "reach": format!("{reach:?}"), "connect_ms": connect_ms,
            "rtt_ms": rtts.iter().map(|r| (r * 10.0).round() / 10.0).collect::<Vec<_>>(),
            "paths": paths, "quic_rtt_ms": r1_link::path_rtt(&conn).map(|d| d.as_millis()), "hello": hello,
        }))
    }

    /// The link's state: this R1's key, the computers it trusts, and which are connected now (and how).
    pub async fn status(&self, hub: &Hub) -> Value {
        let ep = self.ep.lock().await.as_ref().map(|(r, ep)| json!({ "id": ep.id().to_string(), "reach": format!("{r:?}"), "stats": r1_link::stats(ep) }));
        let conns = self.conns.lock().unwrap();
        let computers: Vec<Value> = hub
            .core
            .computers()
            .into_iter()
            .map(|c| {
                let conn = conns.get(&c.id).filter(|x| x.close_reason().is_none());
                json!({ "id": c.id, "name": c.name, "added": c.added, "connected": conn.is_some(), "path": conn.map(r1_link::path_kind) })
            })
            .collect();
        json!({ "endpoint": ep, "computers": computers })
    }

    /// Lets the endpoint go: no keepalives, nothing listening.
    pub async fn close(&self) {
        self.conns.lock().unwrap().clear();
        if let Some((_, ep)) = self.ep.lock().await.take() {
            ep.close().await;
        }
    }

    /// Forgets a computer.
    pub async fn unlink(&self, hub: &Hub, id: &str) -> Result<()> {
        let mut all = hub.core.computers();
        let before = all.len();
        all.retain(|c| c.id != id);
        anyhow::ensure!(all.len() < before, "no computer `{id}`");
        hub.core.set_computers(&all)?;
        if let Some(c) = self.conns.lock().unwrap().remove(id) {
            c.close(0u32.into(), b"unlinked");
        }
        Ok(())
    }

    /// A computer's call: `api {method, path, body}` (an allowed route of r1d's own API), or `hello`.
    async fn call(&self, hub: &Arc<Hub>, computer: &str, method: &str, params: Value) -> Msg {
        self.touch(computer);
        let r = match method {
            "api" => self.api(hub, computer, &params).await,
            "settings" => self.set(hub, computer, &params),
            "mcp" => Self::mcp(hub, computer, &params).await,
            "approve" => Self::approve(hub, computer, &params).await,
            "hello" => Ok(json!({ "name": r1_name(), "version": env!("CARGO_PKG_VERSION") })),
            _ => Err(anyhow!("no method `{method}`")),
        };
        match r {
            Ok(value) => Msg::Result { ok: true, value },
            Err(e) => Msg::Result { ok: false, value: json!(format!("{e:#}")) },
        }
    }

    /// One of r1d's routes, as the R1's own Settings would call it: `{status, body}` (JSON), or `{status, mime,
    /// base64}` (a photo).
    async fn api(&self, hub: &Hub, computer: &str, p: &Value) -> Result<Value> {
        use axum::http::header;
        use base64::Engine;
        use tower::ServiceExt;
        let method = p["method"].as_str().unwrap_or("GET").to_ascii_uppercase();
        let path = p["path"].as_str().unwrap_or_default();
        anyhow::ensure!(allowed(&method, path), "a computer can't call {method} {path}");
        let router = self.router.get().cloned().context("r1d isn't serving yet")?;
        let (body, mime) = match (p.get("body"), p["base64"].as_str()) {
            (_, Some(b64)) => (base64::engine::general_purpose::STANDARD.decode(b64)?, p["mime"].as_str().unwrap_or("application/octet-stream")),
            (Some(b), None) if !b.is_null() => (serde_json::to_vec(b)?, "application/json"),
            _ => (Vec::new(), "application/json"),
        };
        let name = hub.core.computers().into_iter().find(|c| c.id == computer).map(|c| c.name).unwrap_or_else(|| "a computer".into());
        if let Some(why) = self.grant_refused(hub, &name, &method, path, p.get("body")).await {
            return Ok(json!({ "status": 403, "body": { "error": why } }));
        }
        let req = axum::http::Request::builder()
            .method(method.as_str())
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {}", hub.auth.master()))
            .header(header::CONTENT_TYPE, mime)
            .header(FROM_COMPUTER, name.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').collect::<String>())
            .body(axum::body::Body::from(body))?;
        let resp = router.oneshot(req).await.map_err(|e| anyhow!("{e}"))?;
        let status = resp.status().as_u16();
        // Talking isn't changing anything: no note on the R1 for a message, a new chat or a photo sent with one.
        if method != "GET" && resp.status().is_success() && !talking(&method, path) {
            self.note(hub, computer, &name, path);
        }
        let mime = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
        let bytes = axum::body::to_bytes(resp.into_body(), 16 << 20).await?;
        if mime.starts_with("application/json") || mime.is_empty() {
            let body: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
            return Ok(json!({ "status": status, "body": body }));
        }
        Ok(json!({ "status": status, "mime": mime, "base64": base64::engine::general_purpose::STANDARD.encode(&bytes) }))
    }

    /// The agent a turn's tool token belongs to, if it works on `computer`: no other computer gets its tools.
    fn caller(hub: &Hub, computer: &str, p: &Value) -> Result<(crate::mcp::Caller, crate::core::Agent)> {
        let caller = p["token"].as_str().and_then(|t| hub.tool_tokens.caller(t)).context("that turn is over")?;
        let agent = hub.core.agent(&caller.agent).context("the agent is gone")?;
        anyhow::ensure!(agent.computer() == Some(computer), "that agent doesn't work on this computer");
        Ok((caller, agent))
    }

    /// One of r1d's tools, called by an agent working on this computer: the JSON-RPC answer (null for a notification).
    async fn mcp(hub: &Arc<Hub>, computer: &str, p: &Value) -> Result<Value> {
        let (caller, _) = Self::caller(hub, computer, p)?;
        Ok(crate::mcp::answer(hub, &caller, &p["body"]).await.unwrap_or(Value::Null))
    }

    /// An agent on this computer asks before it does something (crate::approvals, its levels for files, commands and
    /// what's outside its folders): `{allow, why?}` once you've answered on the R1 (or its level answered for you).
    async fn approve(hub: &Arc<Hub>, computer: &str, p: &Value) -> Result<Value> {
        let (caller, agent) = Self::caller(hub, computer, p)?;
        let area = p["area"].as_str().unwrap_or_default();
        anyhow::ensure!(crate::remote::PC_AREAS.contains(&area), "unknown area `{area}`");
        let title: String = p["title"].as_str().unwrap_or_default().chars().take(140).collect();
        let detail: String = p["detail"].as_str().unwrap_or_default().chars().take(600).collect();
        let ask = crate::approvals::Ask::new(area, title, detail).why(p["why"].as_str().unwrap_or_default()).chat(&caller.chat);
        Ok(match hub.approvals.gate(hub, &agent.id, ask).await {
            Ok(()) => json!({ "allow": true }),
            Err(why) => json!({ "allow": false, "why": why }),
        })
    }

    /// Granting an app permissions from a computer: the R1's approval card asks you, and this says why not if you
    /// didn't allow it (None: go ahead, or nothing's being granted).
    async fn grant_refused(&self, hub: &Hub, computer: &str, method: &str, path: &str, body: Option<&Value>) -> Option<String> {
        let app = path.strip_prefix("/api/apps/")?.strip_suffix("/permissions")?;
        if method != "POST" || app.contains('/') {
            return None;
        }
        let granted: Vec<&str> = body?["decisions"].as_object()?.iter().filter(|(_, v)| v.as_bool() == Some(true)).map(|(k, _)| k.as_str()).collect();
        if granted.is_empty() {
            return None;
        }
        let installed = hub.apps.get(app);
        let name = installed.as_ref().map(|i| i.app.app.name.clone()).unwrap_or_else(|| app.to_owned());
        // In the words the R1's own consent card uses ("send you reminders"), not the permissions' ids.
        let states = installed.map(|i| crate::broker::states(&hub.core, &i.app)).unwrap_or_default();
        let words: Vec<String> = granted.iter().map(|g| states.iter().find(|s| s.id == *g).map_or_else(|| (*g).to_owned(), |s| s.label.clone())).collect();
        let ask = crate::approvals::Ask::new("apps", format!("let {name} {}", words.join(" and ")), "asked from the companion").every_time();
        hub.approvals.confirm(hub, computer, ask).await.err()
    }

    /// Notes a change made from a computer on the R1 (a word at the top), unless that was noted a moment ago.
    fn note(&self, hub: &Hub, computer: &str, name: &str, path: &str) {
        let what = changed_what(path);
        let mut noted = self.noted.lock().unwrap();
        let key = (computer.to_owned(), what);
        if noted.get(&key).is_some_and(|at| at.elapsed() < NOTE_EVERY) {
            return;
        }
        noted.insert(key, Instant::now());
        drop(noted);
        hub.emit(Event::ChangedFrom { computer: name.to_owned(), what: what.into() }, Source::Api);
    }
}

/// A computer talking to an agent (ADR-0051): a new chat, a message, the chat seen, a session warmed, a photo for it.
fn talking(method: &str, path: &str) -> bool {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    method == "POST" && (route.starts_with("/api/chats") || (route == "/api/media" && query.contains("source=chat")))
}

/// What a computer changed, in a word or two for the R1's note.
fn changed_what(path: &str) -> &'static str {
    let path = path.split('?').next().unwrap_or_default();
    const WHAT: &[(&str, &str)] = &[
        ("/api/look", "the look"), ("/api/home", "home"), ("/api/glance", "widgets"), ("/api/agents", "agents"),
        ("/api/alarms", "alarms"), ("/api/timers", "timers"), ("/api/routines", "routines"), ("/api/connections", "connections"), ("/api/skills", "skills"), ("/api/memories", "memories"), ("/api/chats", "chats"), ("/api/media", "photos"),
        ("/api/journal", "the journal"), ("/api/apps", "apps"), ("/api/accounts", "accounts"), ("/api/wifi", "wi-fi"),
    ];
    WHAT.iter().find(|(p, _)| path.starts_with(p)).map(|(_, w)| *w).unwrap_or("settings")
}

/// Answers a computer's calls on its connection, as long as it's open, and only once this R1 trusts it.
fn serve(hub: Arc<Hub>, id: String, conn: Connection) {
    tokio::spawn(async move {
        while let Ok((msg, send)) = r1_link::next(&conn).await {
            let (h, id) = (hub.clone(), id.clone());
            tokio::spawn(async move {
                let trusted = h.core.computers().iter().any(|c| c.id == id);
                let reply = match msg {
                    Msg::Call { .. } if !trusted => Msg::Refused { why: "this R1 doesn't trust this computer yet".into() },
                    Msg::Call { method, params } => h.link.call(&h, &id, &method, params).await,
                    _ => Msg::Refused { why: "the R1 answers calls only".into() },
                };
                let _ = r1_link::answer(send, &reply).await;
            });
        }
        hub.link.conns.lock().unwrap().retain(|_, c| c.close_reason().is_none());
        tracing::info!(computer = %id.get(..10).unwrap_or(&id), "link: connection ended");
        presence(&hub, &id);
    });
}

/// Sends the events a computer follows to every connected one (each on its own stream; nothing comes back).
pub fn forward_events(hub: Arc<Hub>) {
    let mut rx = hub.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            let conns: Vec<Connection> = hub.link.conns.lock().unwrap().values().filter(|c| c.close_reason().is_none()).cloned().collect();
            if conns.is_empty() {
                continue;
            }
            let Ok(event) = serde_json::to_value(&ev) else { continue };
            let kind = event["type"].as_str().unwrap_or_default();
            if !FORWARD.contains(&kind) {
                continue;
            }
            // Something going on (ADR-0049): the links stay a while longer.
            if !QUIET.contains(&kind) {
                let ids: Vec<String> = hub.link.conns.lock().unwrap().keys().cloned().collect();
                ids.iter().for_each(|id| hub.link.touch(id));
            }
            for c in conns {
                let event = event.clone();
                tokio::spawn(async move {
                    let _ = r1_link::ask(&c, &Msg::Event { event }).await;
                });
            }
        }
    });
}

/// What a linked computer may call: what the R1's own Settings can do. Reading agents, apps, the look, settings,
/// alarms, memories, chats and media; changing agents, the look, home, widgets, the R1's settings, the time, app builds'
/// model, alarms and memories, and signing its CLIs in and out.
pub fn allowed(method: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or_default();
    if path.contains("..") || path.contains("//") {
        return false;
    }
    const READ: &[&str] = &[
        "/api/state", "/api/agents", "/api/look", "/api/home", "/api/glance", "/api/device", "/api/apps", "/api/time",
        "/api/builds/model", "/api/alarms", "/api/timers", "/api/routines", "/api/whatsup", "/api/missed", "/api/connections", "/api/memories", "/api/chats", "/api/media", "/api/journal", "/api/accounts",
        "/api/approvals", "/api/wifi", "/api/trash", "/api/backup", "/api/android/apps", "/api/skills",
    ];
    /// (methods, route and what's under it).
    const WRITE: &[(&[&str], &str)] = &[
        (&["PUT", "POST", "DELETE"], "/api/agents"),
        (&["PUT"], "/api/look"),
        (&["PUT"], "/api/home"),
        (&["PUT"], "/api/glance"),
        (&["PUT"], "/api/device/change"),
        (&["POST"], "/api/time"),
        (&["PUT"], "/api/builds/model"),
        (&["POST", "DELETE"], "/api/alarms"),
        (&["POST", "DELETE"], "/api/timers"),
        (&["POST", "DELETE"], "/api/routines"),
        // Skills (T-086): added, turned on or off, removed.
        (&["POST", "DELETE"], "/api/skills"),
        // Connections (ADR-0052): looking again, sign-in links, an agent's rules. "Always" from the computer too.
        (&["POST"], "/api/connections"),
        (&["POST", "DELETE"], "/api/memories"),
        (&["POST"], "/api/accounts"),
    ];
    /// (method, the route itself: `*` is one part of it).
    const EXACT: &[(&str, &str)] = &[
        // Talking to agents from the companion (ADR-0051): a new chat, a message, seen, a session warmed.
        ("POST", "/api/chats"),
        ("POST", "/api/chats/warm"),
        ("POST", "/api/chats/*/messages"),
        ("POST", "/api/chats/*/seen"),
        ("POST", "/api/apps"),
        ("DELETE", "/api/apps/*"),
        ("POST", "/api/apps/*/restore"),
        ("POST", "/api/apps/*/settings"),
        ("POST", "/api/apps/*/permissions"),
        ("POST", "/api/media"),
        ("PUT", "/api/media/*"),
        ("DELETE", "/api/media/*"),
        ("PUT", "/api/journal/*"),
        ("DELETE", "/api/journal/*"),
        ("DELETE", "/api/chats/*"),
        ("POST", "/api/wifi/scan"),
        ("POST", "/api/wifi/connect"),
    ];
    /// Routes a `*` above would match that aren't the thing's own: the Journal's push-to-talk.
    const NOT: &[&str] = &["/api/journal/listen"];
    let under = |p: &str| path == p || path.strip_prefix(p).is_some_and(|rest| rest.starts_with('/'));
    if method == "GET" {
        return READ.iter().any(|p| under(p));
    }
    if NOT.contains(&path) {
        return false;
    }
    let is = |pattern: &str| {
        let (a, b): (Vec<&str>, Vec<&str>) = (pattern.split('/').collect(), path.split('/').collect());
        a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x == y || (*x == "*" && !y.is_empty()))
    };
    WRITE.iter().any(|(methods, p)| methods.contains(&method) && under(p)) || EXACT.iter().any(|(m, p)| *m == method && is(p))
}

/// A computer came or went: if an agent works on it, the R1 shows whether it's there.
fn presence(hub: &Hub, computer: &str) {
    if hub.core.agents().unwrap_or_default().iter().any(|a| a.computer() == Some(computer)) {
        hub.emit(Event::AgentsChanged, Source::Api);
    }
}

/// What the R1 calls itself to a computer ("R1" for now; its name will be the user's to set).
fn r1_name() -> String {
    "R1".into()
}

/// A QR code in a camera frame (JPEG or PNG): its text, or `None`.
pub fn decode_qr(image: &[u8]) -> Result<Option<String>> {
    let img = image::load_from_memory(image).context("not a picture")?.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(img);
    for grid in prepared.detect_grids() {
        if let Ok((_, text)) = grid.decode() {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_link_code_from_a_picture() {
        // A QR the way the computer shows one, drawn 6 px a module with a quiet zone, as a camera frame would hold it.
        let key = r1_link::SecretKey::generate().public();
        let uri = PairingCode::new(key, "Sam's PC", vec!["192.168.1.20:5544".parse().unwrap()]).unwrap().to_uri();
        let qr = qrcode::QrCode::new(uri.as_bytes()).unwrap();
        let (w, px, quiet) = (qr.width(), 6u32, 4u32);
        let side = (w as u32 + quiet * 2) * px;
        let colors = qr.to_colors();
        let img = image::GrayImage::from_fn(side, side, |x, y| {
            let (mx, my) = ((x / px) as i64 - quiet as i64, (y / px) as i64 - quiet as i64);
            let dark = mx >= 0 && my >= 0 && (mx as usize) < w && (my as usize) < w && colors[my as usize * w + mx as usize] == qrcode::Color::Dark;
            image::Luma([if dark { 0 } else { 255 }])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(decode_qr(png.get_ref()).unwrap().as_deref(), Some(uri.as_str()));
        let blank = image::GrayImage::from_pixel(64, 64, image::Luma([255]));
        let mut png = std::io::Cursor::new(Vec::new());
        blank.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(decode_qr(png.get_ref()).unwrap(), None);
    }

    #[test]
    fn computers_get_the_settings_powers_only() {
        assert!(allowed("GET", "/api/agents"));
        assert!(allowed("PUT", "/api/agents/1234abcd/can"));
        assert!(allowed("PUT", "/api/look"));
        assert!(allowed("GET", "/api/media?kind=photo"));
        // The companion's routines (R7).
        assert!(allowed("POST", "/api/routines/ab12cd34/run"));
        assert!(allowed("POST", "/api/routines/ab12cd34/on"));
        // Skills (T-086): added, switched and removed from the companion.
        assert!(allowed("GET", "/api/skills"));
        assert!(allowed("POST", "/api/skills"));
        assert!(allowed("POST", "/api/skills/on"));
        assert!(allowed("DELETE", "/api/skills/frontend-design"));
        assert!(!allowed("PUT", "/api/skills"));
        assert!(allowed("DELETE", "/api/routines/ab12cd34"));
        assert!(allowed("GET", "/api/whatsup"));
        assert!(allowed("GET", "/api/connections"));
        assert!(allowed("POST", "/api/connections/login"));
        assert!(!allowed("POST", "/api/whatsup"));
        assert!(!allowed("GET", "/harness/state"));
        assert!(!allowed("POST", "/api/motor"));
        assert!(!allowed("POST", "/api/approvals/abc"));
        assert!(!allowed("POST", "/api/android/open"));
        assert!(!allowed("GET", "/api/link"));
        assert!(!allowed("POST", "/api/link/pair"));
        assert!(!allowed("GET", "/token"));
        assert!(!allowed("GET", "/api/agentsX"));
        assert!(!allowed("GET", "/api/agents/../motor"));
        assert!(!allowed("PATCH", "/api/look"));
        assert!(allowed("PUT", "/api/device/change"));
        assert!(!allowed("POST", "/api/device"));
        assert!(allowed("POST", "/api/accounts/claude/login"));
        assert!(allowed("POST", "/api/accounts/claude/code"));
        assert!(!allowed("DELETE", "/api/look"));
        // Apps, files and Wi-Fi: only these routes, not what's under them.
        assert!(allowed("POST", "/api/apps"));
        assert!(allowed("DELETE", "/api/apps/water-log"));
        assert!(allowed("POST", "/api/apps/water-log/settings"));
        assert!(allowed("POST", "/api/apps/water-log/permissions"));
        assert!(!allowed("POST", "/api/apps/water-log/run"));
        assert!(!allowed("POST", "/api/apps/water-log/erase"));
        assert!(!allowed("DELETE", "/api/apps/"));
        assert!(!allowed("DELETE", "/api/apps/water-log/settings"));
        assert!(allowed("POST", "/api/media?source=computer"));
        assert!(allowed("DELETE", "/api/media/abc"));
        assert!(allowed("DELETE", "/api/chats/abc"));
        // Talking from the companion (ADR-0051), and not noted on the R1 as a change.
        assert!(allowed("POST", "/api/chats"));
        assert!(allowed("POST", "/api/chats/abc/messages"));
        assert!(allowed("POST", "/api/chats/abc/seen"));
        assert!(!allowed("POST", "/api/chats/abc/messages/x"));
        assert!(talking("POST", "/api/chats/abc/messages") && talking("POST", "/api/media?source=chat"));
        assert!(!talking("DELETE", "/api/chats/abc") && !talking("POST", "/api/media?source=computer"));
        // The PIN lock is the R1's own (ADR-0016).
        assert!(!allowed("GET", "/api/lock") && !allowed("POST", "/api/lock/unlock") && !allowed("POST", "/api/lock/grace"));
        assert!(allowed("POST", "/api/wifi/connect"));
        assert!(allowed("GET", "/api/backup/file?path=core.db&offset=0"));
        assert!(!allowed("POST", "/api/journal/listen"));
        assert!(!allowed("DELETE", "/api/journal/listen"));
        assert!(allowed("DELETE", "/api/journal/abc"));
        assert_eq!(changed_what("/api/look"), "the look");
        assert_eq!(changed_what("/api/device/change"), "settings");
        assert_eq!(changed_what("/api/apps/water-log/settings"), "apps");
    }
}
