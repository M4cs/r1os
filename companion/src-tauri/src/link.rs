//! The companion's side of the link (ADR-0048, docs/specs/companion.md §3–4): an iroh endpoint that R1s dial.
//!
//! - **Its key and the R1s it trusts** live in `%APPDATA%\r1os\companion` (`key`, `r1s.json`), where phase 0's test
//!   program kept them, so an R1 paired then still knows this computer.
//! - **Pairing:** [`Link::start_pairing`] makes a one-time code (a QR, 5 minutes). An R1 that read it sends `pair`; the
//!   window asks "Link *R1*?" with the four-digit code both screens show ([`Link::answer`]), and only a yes pins it.
//! - **An R1 connected** (it dials; ADR-0049 keeps it while anything's going on, up to this computer's idle time,
//!   [`Settings`]): the window can call its allowed API ([`Link::api`]), and it sends the events the window follows.
//! - **Its agents that work here** (crate::runner): the R1 sends their turns (`turn`), and their tools and questions go
//!   back to it.
//! - The window hears of changes as `link` events (the whole [`Link::state`]) and of an R1's events as `r1`.

use anyhow::{Context, Result, anyhow, bail};
use r1_link::{Connection, Endpoint, EndpointId, Msg, PairingCode, Reach};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// The companion's settings (`settings.json`): how long an R1's link stays when nothing's going on.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Seconds (0: only while the R1's screen is on).
    pub idle_s: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { idle_s: 5 * 60 }
    }
}

/// How long a QR is good for.
const CODE_LIFE: Duration = Duration::from_secs(300);
/// How long an R1 waits for the window's yes.
const CONFIRM: Duration = Duration::from_secs(90);

pub struct Link {
    app: AppHandle,
    ep: Endpoint,
    name: String,
    dir: PathBuf,
    /// The R1s this computer trusts: key → name.
    r1s: Mutex<BTreeMap<String, String>>,
    conns: Mutex<HashMap<String, Connection>>,
    code: Mutex<Option<(PairingCode, Instant, String)>>,
    asking: Mutex<Option<Asking>>,
    settings: Mutex<Settings>,
    runner: OnceLock<Arc<crate::runner::Runner>>,
}

/// An R1 waiting for the window's answer.
struct Asking {
    r1: String,
    name: String,
    code: String,
    answer: tokio::sync::oneshot::Sender<bool>,
}

#[derive(Serialize)]
pub struct R1 {
    id: String,
    name: String,
    online: bool,
    path: Option<&'static str>,
}

impl Link {
    pub async fn start(app: AppHandle) -> Result<Arc<Link>> {
        let dir = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("r1os").join("companion");
        let key = r1_link::load_or_create_key(&dir.join("key"))?;
        let ep = r1_link::bind(key, Reach::Anywhere, true, r1_link::Dns::System).await?;
        let r1s = std::fs::read(dir.join("r1s.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let settings = std::fs::read(dir.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "this computer".into());
        let link = Arc::new(Link {
            app,
            ep,
            name,
            dir,
            r1s: Mutex::new(r1s),
            conns: Mutex::new(HashMap::new()),
            code: Mutex::new(None),
            asking: Mutex::new(None),
            settings: Mutex::new(settings),
            runner: OnceLock::new(),
        });
        let runner = crate::runner::Runner::start(link.app.clone(), Arc::downgrade(&link)).await?;
        let _ = link.runner.set(runner);
        let me = link.clone();
        tokio::spawn(async move {
            while let Some(incoming) = me.ep.accept().await {
                let me = me.clone();
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(conn) => me.serve(conn).await,
                        Err(e) => tracing::info!(error = %e, "an R1's connection failed"),
                    }
                });
            }
        });
        // Online in the background (the relay); the window shows the state as it comes.
        let me = link.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(20), me.ep.online()).await;
            me.changed();
        });
        Ok(link)
    }

    /// Everything the window shows about the link.
    pub fn state(&self) -> Value {
        let conns = self.conns.lock().unwrap();
        let r1s: Vec<R1> = self
            .r1s
            .lock()
            .unwrap()
            .iter()
            .map(|(id, name)| {
                let c = conns.get(id).filter(|c| c.close_reason().is_none());
                R1 { id: id.clone(), name: name.clone(), online: c.is_some(), path: c.map(r1_link::path_kind) }
            })
            .collect();
        let code = self.code.lock().unwrap().as_ref().and_then(|(c, at, svg)| {
            let left = CODE_LIFE.checked_sub(at.elapsed())?;
            Some(json!({ "uri": c.to_uri(), "svg": svg, "left_s": left.as_secs() }))
        });
        let asking = self.asking.lock().unwrap().as_ref().map(|a| json!({ "r1": a.r1, "name": a.name, "code": a.code }));
        json!({ "me": { "id": self.ep.id().to_string(), "name": self.name }, "r1s": r1s, "pairing": code, "asking": asking })
    }

    fn changed(&self) {
        let _ = self.app.emit("link", self.state());
    }

    /// A QR for an R1 to read: this computer's key, a one-time secret, its name and LAN addresses.
    pub fn start_pairing(&self) -> Result<Value> {
        let lan: Vec<_> = self
            .ep
            .addr()
            .ip_addrs()
            .copied()
            .filter(|a| matches!(a.ip(), std::net::IpAddr::V4(v4) if v4.is_private()))
            .collect();
        let code = PairingCode::new(self.ep.id(), &self.name, lan)?;
        let svg = qrcode::QrCode::with_error_correction_level(code.to_uri().as_bytes(), qrcode::EcLevel::M)
            .map_err(|e| anyhow!("{e}"))?
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(320, 320)
            .quiet_zone(false)
            .dark_color(qrcode::render::svg::Color("#0b0b0c"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build();
        *self.code.lock().unwrap() = Some((code, Instant::now(), svg));
        self.changed();
        Ok(self.state())
    }

    pub fn stop_pairing(&self) {
        *self.code.lock().unwrap() = None;
        self.changed();
    }

    /// The window's answer to an R1 asking to link.
    pub fn answer(&self, yes: bool) {
        if let Some(a) = self.asking.lock().unwrap().take() {
            let _ = a.answer.send(yes);
        }
        self.changed();
    }

    /// Forgets an R1: it can't call this computer again until it pairs again.
    pub fn unlink(&self, id: &str) -> Result<()> {
        let mut r1s = self.r1s.lock().unwrap();
        r1s.remove(id).context("no such R1")?;
        self.save(&r1s)?;
        drop(r1s);
        if let Some(c) = self.conns.lock().unwrap().remove(id) {
            c.close(0u32.into(), b"unlinked");
        }
        self.changed();
        Ok(())
    }

    pub fn runner(&self) -> Option<Arc<crate::runner::Runner>> {
        self.runner.get().cloned()
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Keeps new settings, and tells every connected R1.
    pub async fn set_settings(&self, s: Settings) -> Result<()> {
        *self.settings.lock().unwrap() = s.clone();
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("settings.json"), serde_json::to_vec_pretty(&s)?)?;
        let ids: Vec<String> = self.conns.lock().unwrap().keys().cloned().collect();
        for id in ids {
            let _ = self.call(&id, "settings", json!({ "idle_s": s.idle_s })).await;
        }
        Ok(())
    }

    /// A call to a connected R1: its answer's value.
    pub async fn call(&self, id: &str, method: &str, params: Value) -> Result<Value> {
        let conn = self.conns.lock().unwrap().get(id).filter(|c| c.close_reason().is_none()).cloned();
        let conn = conn.ok_or_else(|| anyhow!("asleep"))?;
        r1_link::call(&conn, method, params).await
    }

    /// One of the R1's allowed routes (r1d checks): `{status, body}`.
    pub async fn api(&self, id: &str, method: &str, path: &str, body: Value) -> Result<Value> {
        self.call_api(id, json!({ "method": method, "path": path, "body": body })).await
    }

    /// The same, with r1d's params as they are (`base64` and `mime` for bytes: a photo dropped in).
    pub async fn call_api(&self, id: &str, params: Value) -> Result<Value> {
        let conn = self.conns.lock().unwrap().get(id).filter(|c| c.close_reason().is_none()).cloned();
        let conn = conn.ok_or_else(|| anyhow!("asleep"))?;
        r1_link::call(&conn, "api", params).await
    }

    fn save(&self, r1s: &BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join("r1s.json"), serde_json::to_vec_pretty(r1s)?)?;
        Ok(())
    }

    /// An R1's connection: its pairing, or (once trusted) its events, for as long as it's open.
    async fn serve(self: Arc<Self>, conn: Connection) {
        let r1 = conn.remote_id().to_string();
        let trusted = self.r1s.lock().unwrap().contains_key(&r1);
        if trusted {
            self.conns.lock().unwrap().insert(r1.clone(), conn.clone());
            tracing::info!(r1 = %&r1[..10], path = r1_link::path_kind(&conn), "an R1 connected");
            self.changed();
            // How long it keeps the link when nothing's going on (ADR-0049).
            let (c, idle_s) = (conn.clone(), self.settings().idle_s);
            tokio::spawn(async move {
                let _ = r1_link::call(&c, "settings", json!({ "idle_s": idle_s })).await;
            });
        }
        // The path changes (a relay first, then direct): the window shows it.
        let watch = conn.clone();
        let me = self.clone();
        tokio::spawn(async move {
            let mut last = "";
            while watch.close_reason().is_none() {
                let now = r1_link::path_kind(&watch);
                if now != last {
                    last = now;
                    me.changed();
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        while let Ok((msg, send)) = r1_link::next(&conn).await {
            let me = self.clone();
            let conn = conn.clone();
            // An agent's turn, streamed back as it runs (crate::runner).
            if let Msg::Call { method, params } = &msg
                && method == "turn"
                && self.r1s.lock().unwrap().contains_key(&r1)
                && let Some(runner) = self.runner()
            {
                tokio::spawn(runner.turn(r1.clone(), params.clone(), send));
                continue;
            }
            tokio::spawn(async move {
                let reply = me.handle(&conn, msg).await;
                if let Some(reply) = reply {
                    let _ = r1_link::answer(send, &reply).await;
                } else {
                    let _ = r1_link::answer(send, &Msg::Result { ok: true, value: Value::Null }).await;
                }
            });
        }
        self.conns.lock().unwrap().retain(|_, c| c.close_reason().is_none());
        tracing::info!(r1 = %&r1[..10], "an R1 went");
        self.changed();
    }

    async fn handle(&self, conn: &Connection, msg: Msg) -> Option<Msg> {
        let r1 = conn.remote_id();
        let trusted = self.r1s.lock().unwrap().contains_key(&r1.to_string());
        match msg {
            Msg::Pair { secret, name, .. } => Some(match self.pair(conn, r1, &secret, &name).await {
                Ok(m) => m,
                Err(e) => Msg::Refused { why: format!("{e:#}") },
            }),
            _ if !trusted => Some(Msg::Refused { why: "this computer doesn't know this R1: link it first".into() }),
            Msg::Event { event } => {
                let _ = self.app.emit("r1", json!({ "r1": r1.to_string(), "event": event }));
                None
            }
            Msg::Call { method, params } => Some(match method.as_str() {
                "hello" => Msg::Result { ok: true, value: json!({ "name": self.name, "os": std::env::consts::OS }) },
                // The R1 stops one of its agents' turns here (r1d's crate::stops).
                "stop" => {
                    let chat = params["chat"].as_str().unwrap_or_default();
                    let stopped = self.runner().is_some_and(|r| r.stop(&r1.to_string(), chat));
                    Msg::Result { ok: true, value: json!({ "stopped": stopped }) }
                }
                "ping" => Msg::Result { ok: true, value: json!("pong") },
                other => Msg::Result { ok: false, value: json!(format!("no method `{other}`")) },
            }),
            other => Some(Msg::Refused { why: format!("unexpected {other:?}") }),
        }
    }

    /// An R1 read the QR: the window asks, and a yes pins it.
    async fn pair(&self, conn: &Connection, r1: EndpointId, secret: &str, name: &str) -> Result<Msg> {
        let shown = {
            let mut slot = self.code.lock().unwrap();
            let Some((code, at, _)) = slot.as_ref() else { bail!("this computer isn't showing a code") };
            if at.elapsed() > CODE_LIFE {
                *slot = None;
                bail!("the code ran out: show a new one");
            }
            let ok = r1_link::parse_secret(secret).is_some_and(|s| r1_link::same_secret(&s, &code.secret));
            anyhow::ensure!(ok, "that isn't this computer's code");
            let shown = r1_link::short_code(&self.ep.id(), &r1, &code.secret);
            *slot = None;
            shown
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.asking.lock().unwrap() = Some(Asking { r1: r1.to_string(), name: name.to_owned(), code: shown.clone(), answer: tx });
        self.changed();
        let yes = tokio::time::timeout(CONFIRM, rx).await.ok().and_then(Result::ok).unwrap_or(false);
        self.asking.lock().unwrap().take();
        if !yes {
            self.changed();
            bail!("not linked on the computer");
        }
        let mut r1s = self.r1s.lock().unwrap();
        r1s.insert(r1.to_string(), name.to_owned());
        self.save(&r1s)?;
        drop(r1s);
        self.conns.lock().unwrap().insert(r1.to_string(), conn.clone());
        self.changed();
        tracing::info!(r1 = %r1.fmt_short(), "linked an R1");
        Ok(Msg::Paired { name: self.name.clone(), code: shown })
    }
}
