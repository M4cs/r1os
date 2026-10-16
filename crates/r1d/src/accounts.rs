//! Signing the agent box into Claude and Codex (ADR-0004; docs/specs/experience.md §2.4).
//!
//! - **Codex:** `codex login --device-auth` in the box prints a URL and a one-time code. The R1
//!   shows them (QR plus big text); the user approves on their phone, and the CLI finishes by itself.
//! - **Claude:** `claude auth login` in the box prints an authorization URL, then reads a code from
//!   stdin. Typing that on a 2.88" screen is miserable, so r1d opens a **pairing page on the LAN**
//!   (`http://<r1-ip>:7180/pair#<token>`) only while pairing. The R1 shows its QR; the page links to
//!   Claude's sign-in and takes the pasted code, which r1d writes to the CLI's stdin. The code is
//!   never logged or stored, and the CLIs keep their own credentials in the agent's home.
//!   The pairing server closes after success, failure or 10 minutes, and listens only on the Wi-Fi address.
//! - **Sealed on the way (T-175):** the page is plain HTTP, so nothing secret crosses the LAN as it is. The token stays
//!   in the URL's fragment, which the browser never sends. The page sends an id made from it, and the code XORed with a
//!   keystream made from it, with a MAC: all HMAC-SHA256 keyed by the token, in the page's own JavaScript (a page on a
//!   plain-HTTP LAN address gets no WebCrypto). Someone watching the Wi-Fi sees neither the code nor anything that would
//!   let them send their own. Someone who can change the page as it goes by could still read the fragment: only HTTPS
//!   the phone trusts would stop that.

use crate::core::random_hex;
use crate::hub::{Event, Hub, Source};
use crate::jail::{CLAUDE, CODEX, Jail};
use anyhow::{Context, bail};
use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};

pub const PAIR_PORT: u16 = 7180;
const PAIR_WINDOW: Duration = Duration::from_secs(600);
const STATUS_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub claude: bool,
    pub codex: bool,
}

struct Pairing {
    token: String,
    auth_url: String,
    stdin: Option<ChildStdin>,
    child: Child,
    started: Instant,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

#[derive(Default)]
pub struct Accounts {
    jail: Jail,
    status: Mutex<Option<(Instant, Status)>>,
    pairing: tokio::sync::Mutex<Option<Pairing>>,
    /// Cancels the running Codex device login (its waiter kills the CLI).
    codex: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The first `https://` URL in `text` that starts with `prefix`.
fn find_url(text: &str, prefix: &str) -> Option<String> {
    let at = text.find(prefix)?;
    Some(text[at..].split(|c: char| c.is_whitespace()).next()?.to_owned())
}

/// A Codex device code like `M48X-1QWFQ`.
fn find_code(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|w| {
        let (a, b) = w.split_once('-')?;
        let ok = |p: &str| (4..=6).contains(&p.len()) && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        (ok(a) && ok(b) && w.chars().any(|c| c.is_ascii_digit())).then(|| w.to_owned())
    })
}

impl Accounts {
    fn emit(hub: &Hub, brain: &str, state: &str, detail: Option<String>) {
        hub.emit(Event::Account { brain: brain.into(), state: state.into(), detail }, Source::Api);
    }

    fn invalidate(&self) {
        *self.status.lock().unwrap() = None;
    }

    /// Which CLIs are signed in (cached for 30 s; refreshed after logins and logouts).
    pub async fn status(&self) -> Status {
        if let Some((at, s)) = self.status.lock().unwrap().clone()
            && at.elapsed() < STATUS_TTL
        {
            return s;
        }
        let claude = self.run(CLAUDE, &["auth", "status"]).await.map(|o| o.contains("\"loggedIn\": true")).unwrap_or(false);
        let codex = self.run(CODEX, &["login", "status"]).await.map(|o| o.contains("Logged in")).unwrap_or(false);
        let s = Status { claude, codex };
        *self.status.lock().unwrap() = Some((Instant::now(), s.clone()));
        s
    }

    async fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<String> {
        self.cli(program, args, Duration::from_secs(20)).await
    }

    /// A CLI command in the box as the agent user, with no stdin: what it printed (crate::connections' `mcp list`).
    pub async fn cli(&self, program: &str, args: &[&str], limit: Duration) -> anyhow::Result<String> {
        let mut cmd = self.jail.command(program, args, "/home/agent", &[("NO_COLOR", "1")])?;
        let out = tokio::time::timeout(limit, cmd.stdin(Stdio::null()).kill_on_drop(true).output()).await??;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr))
    }

    pub async fn logout(&self, hub: Arc<Hub>, brain: &str) -> anyhow::Result<()> {
        let args: &[&str] = match brain {
            "claude" => &["auth", "logout"],
            "codex" => &["logout"],
            other => bail!("unknown brain `{other}`"),
        };
        self.run(if brain == "claude" { CLAUDE } else { CODEX }, args).await?;
        self.invalidate();
        Self::emit(&hub, brain, "signed_out", None);
        Ok(())
    }

    /// Starts Codex's device-code login; returns the URL and code to show.
    pub async fn codex_login(self: &Arc<Self>, hub: Arc<Hub>) -> anyhow::Result<serde_json::Value> {
        if let Some(cancel) = self.codex.lock().unwrap().take() {
            let _ = cancel.send(());
        }
        let mut cmd = self.jail.command(CODEX, &["login", "--device-auth"], "/home/agent", &[("NO_COLOR", "1")])?;
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn()?;
        let mut out = child.stdout.take().context("stdout")?;
        let mut err = child.stderr.take().context("stderr")?;
        let mut text = String::new();
        let found = tokio::time::timeout(Duration::from_secs(30), async {
            let (mut a, mut b) = ([0u8; 2048], [0u8; 2048]);
            loop {
                let n = tokio::select! {
                    r = out.read(&mut a) => { let n = r?; text.push_str(&String::from_utf8_lossy(&a[..n])); n }
                    r = err.read(&mut b) => { let n = r?; text.push_str(&String::from_utf8_lossy(&b[..n])); n }
                };
                let clean = strip_ansi(&text);
                if let (Some(url), Some(code)) = (find_url(&clean, "https://auth.openai.com/"), find_code(&clean)) {
                    return anyhow::Ok((url, code));
                }
                if n == 0 && clean.contains("rror") {
                    bail!("{}", clean.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("codex login failed"));
                }
            }
        })
        .await
        .context("Codex didn't show a device code")??;

        // Keep draining its output, and announce the result when it exits.
        tokio::spawn(async move {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            let _ = tokio::join!(out.read_to_end(&mut a), err.read_to_end(&mut b));
        });
        Self::emit(&hub, "codex", "waiting", None);
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        *self.codex.lock().unwrap() = Some(cancel_tx);
        let me = self.clone();
        tokio::spawn(async move {
            let outcome = tokio::select! {
                s = child.wait() => Some(s.map(|s| s.success()).unwrap_or(false)),
                _ = cancel_rx => None,
                () = tokio::time::sleep(Duration::from_secs(900)) => Some(false),
            };
            let _ = child.kill().await;
            me.invalidate();
            match outcome {
                Some(true) => Self::emit(&hub, "codex", "signed_in", None),
                Some(false) => Self::emit(&hub, "codex", "failed", None),
                None => {} // replaced by a newer login
            }
        });
        Ok(json!({ "url": found.0, "code": found.1, "expires_in": 900 }))
    }

    /// Starts Claude's login and opens the LAN pairing page; returns its URL (for the QR).
    /// `lan_ip`: open the LAN pairing page there (a phone pastes the code). None: a linked computer signs it in
    /// (ADR-0048): no page; it opens the link itself and passes the code back ([`Self::code_from_computer`]).
    pub async fn claude_login(self: &Arc<Self>, hub: Arc<Hub>, lan_ip: Option<&str>) -> anyhow::Result<serde_json::Value> {
        let mut slot = self.pairing.lock().await;
        if let Some(mut old) = slot.take() {
            let _ = old.child.kill().await;
            if let Some(stop) = old.stop.take() {
                let _ = stop.send(());
            }
        }
        let mut cmd = self.jail.command(CLAUDE, &["auth", "login"], "/home/agent", &[("NO_COLOR", "1")])?;
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().context("stdout")?;
        let mut lines = BufReader::new(stdout).lines();
        let auth_url = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(l) = lines.next_line().await? {
                if let Some(u) = find_url(&l, "https://") {
                    return anyhow::Ok(u);
                }
            }
            bail!("claude exited without a sign-in link")
        })
        .await
        .context("Claude didn't show a sign-in link")??;
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

        let token = random_hex(16);
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        if let Some(ip) = lan_ip {
            let app = Router::new()
                .route("/pair", get(pair_page).post(pair_code))
                .route("/pair/info", axum::routing::post(pair_info))
                .with_state((self.clone(), hub.clone()));
            // The Wi-Fi address alone, not every interface (T-175).
            let ip: std::net::IpAddr = ip.parse().context("the Wi-Fi address")?;
            let listener = tokio::net::TcpListener::bind((ip, PAIR_PORT)).await.context("pairing port")?;
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).with_graceful_shutdown(async { let _ = stop_rx.await; }).await;
            });
        }
        *slot = Some(Pairing { token: token.clone(), auth_url: auth_url.clone(), stdin, child, started: Instant::now(), stop: Some(stop_tx) });
        drop(slot);

        // Close the window after 10 minutes if nothing happened.
        let me = self.clone();
        let h = hub.clone();
        let t = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(PAIR_WINDOW).await;
            let mut slot = me.pairing.lock().await;
            if slot.as_ref().is_some_and(|p| p.token == t) {
                if let Some(mut p) = slot.take() {
                    let _ = p.child.kill().await;
                    if let Some(stop) = p.stop.take() {
                        let _ = stop.send(());
                    }
                }
                Self::emit(&h, "claude", "expired", None);
            }
        });
        Self::emit(&hub, "claude", "waiting", None);
        Ok(match lan_ip {
            Some(ip) => json!({ "pair_url": format!("http://{ip}:{PAIR_PORT}/pair#{token}"), "auth_url": auth_url }),
            None => json!({ "auth_url": auth_url, "token": token }),
        })
    }

    /// The code a linked computer's user pasted (in memory only: it goes to the CLI and nowhere else).
    pub async fn code_from_computer(&self, hub: &Hub, token: &str, code: &str) -> anyhow::Result<()> {
        self.submit_code(hub, token, code).await
    }

    /// The pasted code from the pairing page: hand it to `claude auth login` and wait for the result.
    async fn submit_code(&self, hub: &Hub, token: &str, code: &str) -> anyhow::Result<()> {
        let mut slot = self.pairing.lock().await;
        let p = slot.as_mut().filter(|p| constant_eq(&p.token, token) && p.started.elapsed() < PAIR_WINDOW).context("this pairing link has expired")?;
        let code = code.trim();
        anyhow::ensure!(!code.is_empty() && code.len() < 4096 && !code.contains('\n'), "that doesn't look like a code");
        let mut stdin = p.stdin.take().context("already submitted")?;
        stdin.write_all(code.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        drop(stdin);
        Self::emit(hub, "claude", "code_entered", None);
        let status = tokio::time::timeout(Duration::from_secs(60), p.child.wait()).await;
        let mut p = slot.take().unwrap();
        if let Some(stop) = p.stop.take() {
            let _ = stop.send(());
        }
        drop(slot);
        self.invalidate();
        match status {
            Ok(Ok(s)) if s.success() => {
                Self::emit(hub, "claude", "signed_in", None);
                Ok(())
            }
            _ => {
                let _ = p.child.kill().await;
                Self::emit(hub, "claude", "failed", None);
                bail!("Claude didn't accept that code")
            }
        }
    }

    /// The running pairing's token, if [id] is the page's id for it ([pair_id]).
    async fn token_for(&self, id: &str) -> Option<String> {
        let slot = self.pairing.lock().await;
        slot.as_ref().filter(|p| constant_eq(&pair_id(&p.token), id)).map(|p| p.token.clone())
    }

    async fn auth_url_for(&self, id: &str) -> Option<String> {
        let slot = self.pairing.lock().await;
        slot.as_ref().filter(|p| constant_eq(&pair_id(&p.token), id)).map(|p| p.auth_url.clone())
    }
}

/// Whether [a] and [b] are the same, in a time that doesn't depend on where they differ (for tokens).
pub(crate) fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---- the page's seal (T-175): the same HMAC-SHA256 steps as PAIR_HTML's `seal` ---------------------------------------

fn hmac_hex(key: &str, msg: &str) -> String {
    hex(&hmac_bytes(key, msg))
}

fn hmac_bytes(key: &str, msg: &str) -> [u8; 32] {
    use hmac::Mac;
    let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).expect("HMAC takes any key");
    m.update(msg.as_bytes());
    m.finalize().into_bytes().into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// What the page sends in place of the token: an id made from it, which doesn't give the token away.
fn pair_id(token: &str) -> String {
    hmac_hex(token, "r1os pair id")
}

/// The code from the page's `ct` (hex, the code's UTF-8 XORed with the token's keystream) once its `mac` checks out.
fn unseal(token: &str, ct: &str, mac: &str) -> Option<String> {
    if !ct.len().is_multiple_of(2) || ct.len() > 8192 || !constant_eq(&hmac_hex(token, &format!("r1os pair mac {ct}")), mac) {
        return None;
    }
    let bytes: Vec<u8> = (0..ct.len()).step_by(2).map(|i| u8::from_str_radix(ct.get(i..i + 2)?, 16).ok()).collect::<Option<_>>()?;
    let pad: Vec<u8> = (0..bytes.len().div_ceil(32)).flat_map(|i| hmac_bytes(token, &format!("r1os pair pad {i}"))).collect();
    String::from_utf8(bytes.iter().zip(pad).map(|(c, p)| c ^ p).collect()).ok()
}

type PairState = (Arc<Accounts>, Arc<Hub>);

#[derive(Deserialize)]
struct IdReq {
    id: String,
}

async fn pair_info(State((acc, hub)): State<PairState>, Json(req): Json<IdReq>) -> Response {
    match acc.auth_url_for(&req.id).await {
        Some(url) => Json(json!({ "auth_url": url, "agent": hub.core.focus().name })).into_response(),
        None => expired(),
    }
}

fn expired() -> Response {
    (axum::http::StatusCode::GONE, Json(json!({ "error": "This pairing link has expired. Start again on your R1." }))).into_response()
}

#[derive(Deserialize)]
struct SealedReq {
    id: String,
    ct: String,
    mac: String,
}

async fn pair_code(State((acc, hub)): State<PairState>, Json(req): Json<SealedReq>) -> Response {
    let Some(token) = acc.token_for(&req.id).await else { return expired() };
    // A code that doesn't open is turned away before it reaches the CLI, so it can't use up the one try.
    let Some(code) = unseal(&token, &req.ct, &req.mac) else {
        return (axum::http::StatusCode::BAD_REQUEST, Json(json!({ "error": "That didn't come through intact. Try again." }))).into_response();
    };
    match acc.submit_code(&hub, &token, &code).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{e:#}") }))).into_response(),
    }
}

async fn pair_page() -> Html<&'static str> {
    Html(PAIR_HTML)
}

const PAIR_HTML: &str = r##"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="dark light">
<title>Connect your R1 to Claude</title>
<style>
:root{--bg:#0b0c0e;--card:#16181c;--fg:#f2f2f2;--muted:#9aa0a8;--accent:#ff8a3d;--ok:#3ddc84;--bad:#ff6b6b}
@media (prefers-color-scheme:light){:root{--bg:#f6f6f4;--card:#fff;--fg:#141414;--muted:#5d6168}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif;display:flex;min-height:100vh;align-items:center;justify-content:center;padding:20px}
main{width:100%;max-width:420px}h1{font-size:24px;margin:0 0 6px}p{color:var(--muted);margin:0 0 18px}
.step{background:var(--card);border-radius:18px;padding:18px;margin:12px 0}.n{display:inline-grid;place-items:center;width:26px;height:26px;border-radius:50%;background:var(--accent);color:#1d0d02;font-weight:700;margin-right:8px}
a.btn,button{display:block;width:100%;text-align:center;border:0;border-radius:14px;padding:14px;font:600 17px system-ui;background:var(--accent);color:#1d0d02;text-decoration:none;margin-top:12px;cursor:pointer}
textarea{width:100%;margin-top:12px;border-radius:12px;border:1px solid #333;background:transparent;color:var(--fg);font:15px ui-monospace,monospace;padding:12px;min-height:84px}
.msg{margin-top:10px;font-weight:600}.ok{color:var(--ok)}.bad{color:var(--bad)}
</style></head><body><main>
<h1 id="t">Connect your R1</h1><p id="sub">Two steps on this phone. Your R1 does the rest.</p>
<div class="step"><span class="n">1</span><b>Sign in to Claude</b><a class="btn" id="go" href="#" target="_blank" rel="noopener">Open Claude sign-in</a></div>
<div class="step"><span class="n">2</span><b>Paste the code Claude shows you</b>
<textarea id="code" placeholder="Paste the code here" autocomplete="off" autocapitalize="off" spellcheck="false"></textarea>
<button id="send">Connect</button><div class="msg" id="msg"></div></div>
</main><script>
// The token never leaves this phone: it stays in the URL's fragment, and only what's made from it is sent (T-175).
// SHA-256 and HMAC by hand, since a plain-HTTP page on a LAN address gets no WebCrypto.
const K=[0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2];
const rr=(x,n)=>(x>>>n)|(x<<(32-n));
function sha256(b){const l=b.length,n=((l+72)>>6)<<6,m=new Uint8Array(n);m.set(b);m[l]=0x80;const v=new DataView(m.buffer);v.setUint32(n-8,Math.floor(l/0x20000000));v.setUint32(n-4,(l*8)>>>0);
 let h=[0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];const w=new Uint32Array(64);
 for(let o=0;o<n;o+=64){for(let i=0;i<16;i++)w[i]=v.getUint32(o+i*4);
  for(let i=16;i<64;i++){const a=w[i-15],c=w[i-2];w[i]=(w[i-16]+(rr(a,7)^rr(a,18)^(a>>>3))+w[i-7]+(rr(c,17)^rr(c,19)^(c>>>10)))|0}
  let[A,B,C,D,E,F,G,H]=h;
  for(let i=0;i<64;i++){const t1=(H+(rr(E,6)^rr(E,11)^rr(E,25))+((E&F)^(~E&G))+K[i]+w[i])|0,t2=((rr(A,2)^rr(A,13)^rr(A,22))+((A&B)^(A&C)^(B&C)))|0;H=G;G=F;F=E;E=(D+t1)|0;D=C;C=B;B=A;A=(t1+t2)|0}
  const s=[A,B,C,D,E,F,G,H];h=h.map((x,i)=>(x+s[i])|0)}
 const out=new Uint8Array(32),d=new DataView(out.buffer);h.forEach((x,i)=>d.setUint32(i*4,x>>>0));return out}
const enc=s=>new TextEncoder().encode(s),hex=b=>Array.from(b,x=>x.toString(16).padStart(2,'0')).join('');
const cat=(a,b)=>{const c=new Uint8Array(a.length+b.length);c.set(a);c.set(b,a.length);return c};
function hmac(key,msg){let k=enc(key);if(k.length>64)k=sha256(k);const p=new Uint8Array(64);p.set(k);
 return sha256(cat(p.map(x=>x^0x5c),sha256(cat(p.map(x=>x^0x36),enc(msg)))))}
function seal(token,code){const c=enc(code),pad=new Uint8Array(Math.ceil(c.length/32)*32);
 for(let i=0;i*32<c.length;i++)pad.set(hmac(token,'r1os pair pad '+i),i*32);
 const ct=hex(c.map((x,i)=>x^pad[i]));return{id:hex(hmac(token,'r1os pair id')),ct,mac:hex(hmac(token,'r1os pair mac '+ct))}}
const token=location.hash.slice(1);const id=hex(hmac(token,'r1os pair id'));const $=id=>document.getElementById(id);
fetch('/pair/info',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id})}).then(r=>r.json()).then(j=>{
 if(j.error){$('msg').textContent=j.error;$('msg').className='msg bad';$('go').style.display='none';$('send').disabled=true;return}
 $('go').href=j.auth_url;$('t').textContent='Connect '+j.agent+' to Claude';});
$('send').onclick=async()=>{const code=$('code').value.trim();if(!code)return;$('send').disabled=true;$('msg').textContent='Connecting…';$('msg').className='msg';
 const r=await fetch('/pair',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(seal(token,code))});const j=await r.json().catch(()=>({}));
 if(r.ok){$('msg').textContent='Connected. Look at your R1.';$('msg').className='msg ok'}else{$('msg').textContent=j.error||'That didn’t work.';$('msg').className='msg bad';$('send').disabled=false}};
</script></body></html>"##;

/// An SVG QR code for `text` (shown by the shell during onboarding).
pub fn qr_svg(text: &str) -> anyhow::Result<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes())?;
    Ok(code.render::<svg::Color>().min_dimensions(220, 220).quiet_zone(true).dark_color(svg::Color("#000000")).light_color(svg::Color("#ffffff")).build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_device_output() {
        let raw = "Welcome to Codex [v\u{1b}[90m0.162.0\u{1b}[0m]\n1. Open this link\n   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m\n\n2. Enter this one-time code \u{1b}[90m(expires in 15 minutes)\u{1b}[0m\n   \u{1b}[94mM48X-1QWFQ\u{1b}[0m\n";
        let clean = strip_ansi(raw);
        assert_eq!(find_url(&clean, "https://auth.openai.com/").as_deref(), Some("https://auth.openai.com/codex/device"));
        assert_eq!(find_code(&clean).as_deref(), Some("M48X-1QWFQ"));
        assert_eq!(find_code("v0.162.0 expires-in 15"), None);
    }

    #[test]
    fn finds_claudes_link_and_renders_qr() {
        let l = "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&state=abc";
        assert_eq!(find_url(l, "https://").as_deref(), Some("https://claude.com/cai/oauth/authorize?code=true&state=abc"));
        assert!(qr_svg("http://192.168.1.2:7180/pair#00ff").unwrap().starts_with("<?xml"));
        assert!(constant_eq("abc", "abc") && !constant_eq("abc", "abd"));
    }

    #[test]
    fn opens_what_the_page_sealed() {
        // From PAIR_HTML's own `seal`, run in Node (its SHA-256 and HMAC checked against Node's at every padding edge).
        let token = "00112233445566778899aabbccddeeff";
        assert_eq!(pair_id(token), "d46a5411675b5fba7258b1e02840cb62209c2af5db2386468848ca1c3c6e940d");
        let mac = "a29953ddff21298d13c129f471a7ccaae0ed67f7091209cbb9f9935931f59ff2";
        assert_eq!(unseal(token, "a29542faadefce", mac).as_deref(), Some("abc#def"));
        // Four blocks of keystream, and a letter that's two bytes in UTF-8.
        let long = format!("{}é", "Abc123_-#".repeat(12));
        let ct = "829542e8fbb9f7627ef9b2e76b52da90042ca3438944ab79d09a81abea05c29e27d95b1789bf69f81117ac3ee7b5645870e1d006b020a2827011fe4c851cb9d7e369ea5889f82fd6871014cc6c248da83fadd36cd2b92c377c8dc3126e2cc7f0486093c8e1bbdaa24de31babadc7";
        assert_eq!(unseal(token, ct, "182842441a4236e73ac3eb17dc1cb9ea63241992e081b139bfe2252e1aa09b4e"), Some(long));
        // Changed on the way, or sealed with another token: not opened.
        assert_eq!(unseal(token, "a29542faadefcf", mac), None);
        assert_eq!(unseal("ffeeddccbbaa99887766554433221100", "a29542faadefce", mac), None);
        assert_eq!(unseal(token, "a29542faadefc", mac), None);
    }
}
