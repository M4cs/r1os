//! Signing the agent box into Claude and Codex (ADR-0004; docs/specs/experience.md §2.4).
//!
//! - **Codex:** `codex login --device-auth` in the box prints a URL and a one-time code. The R1
//!   shows them (QR plus big text); the user approves on their phone, and the CLI finishes by itself.
//! - **Claude:** `claude auth login` in the box prints an authorization URL, then reads a code from
//!   stdin. Typing that on a 2.88" screen is miserable, so r1d opens a **pairing page on the LAN**
//!   (`http://<r1-ip>:7180/pair#<token>`) only while pairing. The R1 shows its QR; the page links to
//!   Claude's sign-in and takes the pasted code, which r1d writes to the CLI's stdin. The code is
//!   never logged or stored, and the CLIs keep their own credentials in the agent's home.
//!   The pairing server closes after success, failure or 10 minutes.

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
        if lan_ip.is_some() {
            let app = Router::new()
                .route("/pair", get(pair_page).post(pair_code))
                .route("/pair/info", axum::routing::post(pair_info))
                .with_state((self.clone(), hub.clone()));
            let listener = tokio::net::TcpListener::bind(("0.0.0.0", PAIR_PORT)).await.context("pairing port")?;
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

    async fn auth_url_for(&self, token: &str) -> Option<String> {
        let slot = self.pairing.lock().await;
        slot.as_ref().filter(|p| constant_eq(&p.token, token)).map(|p| p.auth_url.clone())
    }
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

type PairState = (Arc<Accounts>, Arc<Hub>);

#[derive(Deserialize)]
struct TokenReq {
    token: String,
}

async fn pair_info(State((acc, hub)): State<PairState>, Json(req): Json<TokenReq>) -> Response {
    match acc.auth_url_for(&req.token).await {
        Some(url) => Json(json!({ "auth_url": url, "agent": hub.core.focus().name })).into_response(),
        None => (axum::http::StatusCode::GONE, Json(json!({ "error": "This pairing link has expired. Start again on your R1." }))).into_response(),
    }
}

#[derive(Deserialize)]
struct CodeReq {
    token: String,
    code: String,
}

async fn pair_code(State((acc, hub)): State<PairState>, Json(req): Json<CodeReq>) -> Response {
    match acc.submit_code(&hub, &req.token, &req.code).await {
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
const token=location.hash.slice(1);const $=id=>document.getElementById(id);
fetch('/pair/info',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({token})}).then(r=>r.json()).then(j=>{
 if(j.error){$('msg').textContent=j.error;$('msg').className='msg bad';$('go').style.display='none';$('send').disabled=true;return}
 $('go').href=j.auth_url;$('t').textContent='Connect '+j.agent+' to Claude';});
$('send').onclick=async()=>{const code=$('code').value.trim();if(!code)return;$('send').disabled=true;$('msg').textContent='Connecting…';$('msg').className='msg';
 const r=await fetch('/pair',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({token,code})});const j=await r.json().catch(()=>({}));
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
}
