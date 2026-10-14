//! The companion's link, as a test program for phase 0 (docs/specs/companion.md §2): the seed of the companion's core.
//!
//! - `r1-link-pc pair [--relay-only]`: shows a pairing QR (a page in the browser) and waits for an R1 to read it; then
//!   serves it.
//! - `r1-link-pc serve [--relay-only]`: serves the R1s it has paired with.
//!
//! It keeps its key and the R1s it trusts in `%APPDATA%\r1os\companion` (`~/.config/r1os/companion` elsewhere), and
//! answers `ping` (with the time) and `hello`. `--relay-only` tries "away from home" at home: no direct path at all.

use anyhow::{Context, Result, anyhow};
use r1_link::{EndpointId, Msg, PairingCode, Reach};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("r1os")
        .join("companion")
}

/// The R1s this computer trusts: key → name.
fn load_r1s() -> BTreeMap<String, String> {
    std::fs::read(dir().join("r1s.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_r1s(r1s: &BTreeMap<String, String>) -> Result<()> {
    std::fs::write(dir().join("r1s.json"), serde_json::to_vec_pretty(r1s)?)?;
    Ok(())
}

struct State {
    name: String,
    /// The QR being shown: its code, until it's used or 5 minutes pass.
    pairing: Mutex<Option<(PairingCode, std::time::Instant)>>,
    r1s: Mutex<BTreeMap<String, String>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into())).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("serve");
    let reach = if args.iter().any(|a| a == "--relay-only") { Reach::RelayOnly } else { Reach::Anywhere };
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "my computer".into());
    let key = r1_link::load_or_create_key(&dir().join("key"))?;
    let ep = r1_link::bind(key, reach, true, r1_link::Dns::System).await?;
    let t = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(15), ep.online()).await.ok();
    let addr = ep.addr();
    println!("this computer: {} ({name}), online in {} ms, reach {reach:?}", ep.id(), t.elapsed().as_millis());
    for a in addr.ip_addrs() {
        println!("  direct: {a}");
    }
    for r in addr.relay_urls() {
        println!("  relay:  {r}");
    }
    let state = Arc::new(State { name: name.clone(), pairing: Mutex::new(None), r1s: Mutex::new(load_r1s()) });
    println!("trusts {} R1(s)", state.r1s.lock().unwrap().len());

    if cmd == "pair" {
        // The LAN addresses go in the QR, so the R1 finds this computer at once at home.
        let lan: Vec<_> = addr.ip_addrs().copied().filter(|a| match a.ip() {
            std::net::IpAddr::V4(v4) => v4.is_private(),
            std::net::IpAddr::V6(_) => false,
        }).collect();
        let code = PairingCode::new(ep.id(), &name, lan)?;
        let uri = code.to_uri();
        show_qr(&uri, &name)?;
        println!("pairing code: {uri}");
        *state.pairing.lock().unwrap() = Some((code, std::time::Instant::now()));
    }

    while let Some(incoming) = ep.accept().await {
        let state = state.clone();
        let me = ep.id();
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(e) => return eprintln!("a connection failed: {e}"),
            };
            if let Err(e) = serve(conn, me, state).await {
                eprintln!("{e:#}");
            }
        });
    }
    Ok(())
}

async fn serve(conn: iroh::endpoint::Connection, me: EndpointId, state: Arc<State>) -> Result<()> {
    let r1 = conn.remote_id();
    let known = state.r1s.lock().unwrap().get(&r1.to_string()).cloned();
    println!("{} connected ({}) via {}", known.as_deref().unwrap_or("an unknown R1"), r1.fmt_short(), r1_link::path_kind(&conn));
    // Path changes, as they happen (a relay first, then direct, usually).
    let watch = conn.clone();
    tokio::spawn(async move {
        let mut last = "";
        while watch.close_reason().is_none() {
            let now = r1_link::path_kind(&watch);
            if now != last {
                println!("  path: {now}{}", r1_link::path_rtt(&watch).map(|r| format!(", rtt {} ms", r.as_millis())).unwrap_or_default());
                last = now;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    loop {
        let (msg, send) = match r1_link::next(&conn).await {
            Ok(m) => m,
            Err(_) => {
                println!("{} went", r1.fmt_short());
                return Ok(());
            }
        };
        let trusted = state.r1s.lock().unwrap().contains_key(&r1.to_string());
        let reply = match msg {
            Msg::Pair { secret, name, version } => pair(&state, me, r1, &secret, &name, version),
            Msg::Call { .. } | Msg::Event { .. } if !trusted => Msg::Refused { why: "this computer doesn't know this R1: pair first".into() },
            Msg::Call { method, params } => Msg::Result { ok: true, value: handle(&state, &method, params) },
            Msg::Event { event } => {
                println!("event: {event}");
                continue;
            }
            other => Msg::Refused { why: format!("unexpected {other:?}") },
        };
        r1_link::answer(send, &reply).await?;
    }
}

fn pair(state: &State, me: EndpointId, r1: EndpointId, secret: &str, name: &str, version: u32) -> Msg {
    let mut slot = state.pairing.lock().unwrap();
    let Some((code, at)) = slot.as_ref() else { return Msg::Refused { why: "this computer isn't showing a code".into() } };
    if at.elapsed() > Duration::from_secs(300) {
        *slot = None;
        return Msg::Refused { why: "the code ran out: show a new one".into() };
    }
    let ok = r1_link::parse_secret(secret).is_some_and(|s| r1_link::same_secret(&s, &code.secret));
    if !ok {
        return Msg::Refused { why: "that isn't this computer's code".into() };
    }
    let shown = r1_link::short_code(&me, &r1, &code.secret);
    *slot = None;
    drop(slot);
    // The phase-0 confirm: the code shows here; the companion's window will ask "Link <name>?".
    println!("\n  paired with {name} (r1d v{version}). Code on both screens: {shown}\n");
    let mut r1s = state.r1s.lock().unwrap();
    r1s.insert(r1.to_string(), name.to_owned());
    if let Err(e) = save_r1s(&r1s) {
        eprintln!("couldn't keep it: {e}");
    }
    Msg::Paired { name: state.name.clone(), code: shown }
}

fn handle(state: &State, method: &str, params: Value) -> Value {
    match method {
        "ping" => json!({ "at_ms": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0), "echo": params }),
        "hello" => json!({ "name": state.name, "os": std::env::consts::OS }),
        _ => json!(format!("no method `{method}`")),
    }
}

/// The QR, big, on a page in the browser (the companion's window will draw it itself).
fn show_qr(uri: &str, name: &str) -> Result<()> {
    let svg = qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| anyhow!("{e}"))?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(480, 480)
        .quiet_zone(true)
        .build();
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>link an R1</title>\
         <body style=\"margin:0;height:100vh;display:grid;place-items:center;background:#0b0b0c;color:#f2f2f2;font:600 18px system-ui\">\
         <div style=\"text-align:center\"><div style=\"background:#fff;padding:18px;border-radius:24px;display:inline-block\">{svg}</div>\
         <p style=\"margin-top:22px\">On the R1: settings › computers › <b>link a computer</b>, and point it here.</p>\
         <p style=\"opacity:.6\">{name} · good for 5 minutes</p></div>"
    );
    // Served on this computer only, from an http address: opening a .html file goes to whatever opens .html files
    // (on Max's PC, VS Code), while an http address goes to the browser.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("the QR page")?;
    let url = format!("http://{}/", listener.local_addr()?);
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let _ = write!(s, "HTTP/1.1 200 OK
Content-Type: text/html; charset=utf-8
Content-Length: {}
Connection: close

{page}", page.len());
        }
    });
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    println!("the QR is at {url} (it opens in your browser)");
    Ok(())
}
