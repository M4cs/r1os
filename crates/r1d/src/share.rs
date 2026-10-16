//! Sharing by QR (ADR-0056, T-071): one item on the Wi-Fi while its code shows, and an app installed from another
//! R1's code.
//!
//! - **Out** ([`Shares::start`]): a photo, a recording or an app is served on a port the system picks, on the Wi-Fi
//!   address alone, at one path, `/s/<token>/<name>` (a 128-bit token); anything else is a 404. It's plain HTTP, so
//!   the token and what's shared cross the Wi-Fi as they are: it's something you're showing someone nearby, for ten
//!   minutes at most (T-175 looked, and left it so; the privacy page says it). It stops when the code's
//!   screen closes ([`Shares::stop`]), after [`LASTS`], or when another share starts. A photo or a recording is served as
//!   itself (ranges too, so a phone can seek a recording); an app as a small page to a browser and as its spec
//!   ([`APP_MIME`]) to an R1.
//! - **In** ([`Shares::scan`]): a QR the camera read. A share's URL (http, a private address that isn't loopback, the
//!   `/s/<token>/…` shape) is fetched (10 s, 1 MB, no redirects); an app's spec is compiled and kept until you say yes
//!   ([`Shares::install`]), with its name and what it does to show first.

use crate::hub::Hub;
use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::services::ServeFile;

/// How long a share stays up, its code showing.
pub const LASTS: Duration = Duration::from_secs(10 * 60);
/// An app's spec, as one R1 asks another for it.
pub const APP_MIME: &str = "application/vnd.r1os.app+yaml";
/// The most a shared app's spec may be.
const APP_MAX: u64 = 1 << 20;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a fetched app waits for your yes.
const PENDING_FOR: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
enum Item {
    File { path: PathBuf },
    App { name: String, about: String, source: String },
}

struct Served {
    token: String,
    name: String,
    item: Item,
}

#[derive(Default)]
pub struct Shares {
    /// The share that's up: how to stop it.
    live: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Apps fetched from a code, waiting for your yes: token → (spec, when).
    pending: Mutex<HashMap<String, (String, Instant)>>,
}

impl Shares {
    /// Shares `what` (`{media: id}` or `{app: id}`): `{url, name, expires_s}`. Replaces a share that's up.
    pub async fn start(&self, hub: &Hub, what: &Value) -> Result<Value> {
        let (name, item) = if let Some(id) = what["media"].as_str() {
            let m = hub.core.with(|db| crate::media::get(db, id))?.with_context(|| format!("no media `{id}`"))?;
            let ext = if m.kind == "recording" { "wav" } else { "jpg" };
            let base = Some(slug(&m.label)).filter(|s| !s.is_empty()).unwrap_or_else(|| m.kind.clone());
            (format!("{base}.{ext}"), Item::File { path: crate::media::file(&hub.cfg.data, &m) })
        } else if let Some(id) = what["app"].as_str() {
            let a = hub.apps.get(id).with_context(|| format!("no app `{id}`"))?;
            let about = a.app.app.description.clone().unwrap_or_default();
            (format!("{id}.enee.yaml"), Item::App { name: a.app.app.name.clone(), about, source: a.source.clone() })
        } else {
            bail!("share `media` or `app`");
        };
        let ip = lan_ip().context("sharing needs the Wi-Fi: no address on a local network")?;
        let listener = tokio::net::TcpListener::bind((ip, 0)).await.context("couldn't listen")?;
        let port = listener.local_addr()?.port();
        let token = crate::core::random_hex(16);
        let url = format!("http://{ip}:{port}/s/{token}/{name}");
        let served = Arc::new(Served { token: token.clone(), name: name.clone(), item });
        let app = Router::new().route("/s/{token}/{name}", get(serve)).fallback(|| async { StatusCode::NOT_FOUND }).with_state(served);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        if let Some(old) = self.live.lock().unwrap().replace(stop) {
            let _ = old.send(());
        }
        tokio::spawn(async move {
            let ended = async {
                tokio::select! {
                    _ = stopped => {}
                    () = tokio::time::sleep(LASTS) => {}
                }
            };
            if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(ended).await {
                tracing::warn!(error = %e, "share: the server stopped");
            }
            tracing::info!(port, "share: stopped");
        });
        tracing::info!(port, %name, "share: up");
        Ok(json!({ "url": url, "name": name, "expires_s": LASTS.as_secs() }))
    }

    /// The share that's up goes (its code's screen closed). Nothing listens after.
    pub fn stop(&self) {
        if let Some(stop) = self.live.lock().unwrap().take() {
            let _ = stop.send(());
        }
    }

    /// What a QR the camera read is: `{kind: "link"}` (a computer's code), `{kind: "app", token, app}` (an R1's shared
    /// app, fetched and checked, waiting for [`Shares::install`]), `{kind: "file"}` (a shared photo or recording: for a
    /// phone), or `{kind: "text", text}`.
    pub fn scan(&self, text: &str) -> Result<Value> {
        if text.starts_with("r1os-link:") {
            return Ok(json!({ "kind": "link" }));
        }
        let Ok(url) = share_url(text) else {
            return Ok(json!({ "kind": "text", "text": text.chars().take(500).collect::<String>() }));
        };
        let config = ureq::Agent::config_builder()
            .https_only(false)
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(FETCH_TIMEOUT))
            .user_agent(concat!("r1os/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let mut r = agent.get(url.as_str()).header("accept", APP_MIME).call().map_err(|e| anyhow!("couldn't reach it: {e}"))?;
        match r.status().as_u16() {
            200 => {}
            404 => bail!("that share has ended"),
            s => bail!("the R1 sharing it answered {s}"),
        }
        let is_app = r.headers().get("content-type").and_then(|t| t.to_str().ok()).is_some_and(|t| t.starts_with(APP_MIME));
        if !is_app {
            return Ok(json!({ "kind": "file", "url": url.as_str() }));
        }
        let source = r.body_mut().with_config().limit(APP_MAX).read_to_string().map_err(|e| anyhow!("couldn't read it: {e}"))?;
        let app = crate::apps::compile(&source).map_err(|r| anyhow!("that app doesn't check: {}", r.diagnostics.first().map_or("", |d| d.message.as_str())))?;
        let token = crate::core::random_hex(8);
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, (_, at)| at.elapsed() < PENDING_FOR);
        pending.insert(token.clone(), (source, Instant::now()));
        let a = &app.app;
        Ok(json!({
            "kind": "app", "token": token,
            "app": { "id": a.id, "name": a.name, "icon": a.icon, "accent": a.accent, "description": a.description },
        }))
    }

    /// You said yes to an app a code brought (`token`): its spec, to install.
    pub fn take(&self, token: &str) -> Result<String> {
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, (_, at)| at.elapsed() < PENDING_FOR);
        pending.remove(token).map(|(s, _)| s).context("that app isn't waiting any more: scan its code again")
    }
}

async fn serve(State(s): State<Arc<Served>>, Path((token, name)): Path<(String, String)>, headers: HeaderMap, req: Request) -> Response {
    if !crate::accounts::constant_eq(&token, &s.token) || name != s.name {
        return StatusCode::NOT_FOUND.into_response();
    }
    match &s.item {
        Item::File { path } => match tower::ServiceExt::oneshot(ServeFile::new(path), req).await {
            Ok(r) => {
                let mut r = r.into_response();
                if let Ok(v) = format!("inline; filename=\"{}\"", s.name).parse() {
                    r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
                }
                r
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        Item::App { name, about, source } => {
            let accept = headers.get(header::ACCEPT).and_then(|a| a.to_str().ok()).unwrap_or_default();
            if accept.contains(APP_MIME) || req.uri().query() == Some("raw") {
                return ([(header::CONTENT_TYPE, APP_MIME)], source.clone()).into_response();
            }
            Html(app_page(name, about, &s.name)).into_response()
        }
    }
}

/// What a phone's browser shows for a shared app.
fn app_page(name: &str, about: &str, file: &str) -> String {
    let (name, about, file) = (escape(name), escape(about), escape(file));
    format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
         <title>{name}</title><style>body{{font:17px/1.45 system-ui,sans-serif;max-width:32em;margin:12vh auto;padding:0 20px;\
         background:#0e1013;color:#eceef1}}h1{{font-size:28px;margin:0 0 6px}}p{{color:#9aa1ab}}a{{color:#ff8a3d}}</style>\
         <h1>{name}</h1><p>{about}</p><p>An r1os app. To add it to an R1, point the R1's camera at the code this page \
         came from (Apps › add from a code).</p><p><a href=\"{file}?raw\" download=\"{file}\">Download its file</a></p>"
    )
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A file name from a label: letters, digits and dashes, at most 40.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// The R1's address on the local network: the one its default route leaves from (no packet is sent), if it's a
/// private one (Wi-Fi; a cellular address isn't reachable from a phone beside it).
fn lan_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    local(ip).then_some(ip)
}

/// A private address on a local network: never loopback (r1d's own API), never the internet.
fn local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => is_ula(v6) || is_v6_link_local(v6),
    }
}

fn is_ula(v6: Ipv6Addr) -> bool {
    v6.segments()[0] & 0xfe00 == 0xfc00
}

fn is_v6_link_local(v6: Ipv6Addr) -> bool {
    v6.segments()[0] & 0xffc0 == 0xfe80
}

/// A share's URL from a QR: http, to a private address literal (no names, no user info), the `/s/<token>/<name>`
/// shape. Everything else is refused before anything is fetched.
fn share_url(raw: &str) -> Result<url::Url> {
    let url = url::Url::parse(raw.trim())?;
    anyhow::ensure!(url.scheme() == "http", "not an R1's share");
    anyhow::ensure!(url.username().is_empty() && url.password().is_none(), "not an R1's share");
    let ip = match url.host() {
        Some(url::Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(url::Host::Ipv6(v6)) => IpAddr::V6(v6),
        _ => bail!("not an R1's share"),
    };
    anyhow::ensure!(local(ip), "not an R1's share");
    let parts: Vec<&str> = url.path_segments().map(Iterator::collect).unwrap_or_default();
    anyhow::ensure!(
        parts.len() == 3 && parts[0] == "s" && parts[1].len() == 32 && parts[1].bytes().all(|b| b.is_ascii_hexdigit()) && !parts[2].is_empty(),
        "not an R1's share"
    );
    anyhow::ensure!(url.port().is_some(), "not an R1's share");
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn only_a_shares_shape_on_a_local_address_is_fetched() {
        assert!(share_url(&format!("http://192.168.1.20:41234/s/{T}/water.enee.yaml")).is_ok());
        assert!(share_url(&format!("http://10.0.0.5:5000/s/{T}/photo.jpg")).is_ok());
        assert!(share_url(&format!("http://[fd00::1]:5000/s/{T}/a.jpg")).is_ok());
        for bad in [
            format!("http://127.0.0.1:7171/s/{T}/x"),
            "http://127.0.0.1:7171/api/state".to_owned(),
            format!("http://localhost:5000/s/{T}/x"),
            format!("https://192.168.1.20:5000/s/{T}/x"),
            format!("http://8.8.8.8:5000/s/{T}/x"),
            format!("http://192.168.1.20/s/{T}/x"),
            format!("http://u:p@192.168.1.20:5000/s/{T}/x"),
            "http://192.168.1.20:5000/s/short/x".to_owned(),
            format!("http://192.168.1.20:5000/api/{T}/x"),
            format!("http://[::1]:5000/s/{T}/x"),
            "WIFI:S:home;T:WPA;P:secret;;".to_owned(),
        ] {
            assert!(share_url(&bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn a_label_becomes_a_file_name() {
        assert_eq!(slug("Idea for the garden!"), "idea-for-the-garden");
        assert_eq!(slug("  ..  "), "");
        assert_eq!(escape("<b>\"x\" & y</b>"), "&lt;b&gt;&quot;x&quot; &amp; y&lt;/b&gt;");
    }

    #[tokio::test]
    async fn a_share_serves_its_one_path_and_stops() {
        // The server alone (no hub): what start() wires up.
        let served = Arc::new(Served { token: T.into(), name: "a.enee.yaml".into(), item: Item::App { name: "Water".into(), about: "Glasses a day".into(), source: "app: {}".into() } });
        let app = Router::new().route("/s/{token}/{name}", get(serve)).fallback(|| async { StatusCode::NOT_FOUND }).with_state(served);
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move { axum::serve(listener, app).with_graceful_shutdown(async { let _ = stopped.await; }).await });
        let base = format!("http://127.0.0.1:{port}");
        let get = |path: String, accept: &'static str| {
            tokio::task::spawn_blocking(move || {
                let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).build());
                let mut r = agent.get(&path).header("accept", accept).call().unwrap();
                let ct = r.headers().get("content-type").map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
                (r.status().as_u16(), ct, r.body_mut().read_to_string().unwrap_or_default())
            })
        };
        let (s, ct, body) = get(format!("{base}/s/{T}/a.enee.yaml"), APP_MIME).await.unwrap();
        assert_eq!((s, ct.as_str(), body.as_str()), (200, APP_MIME, "app: {}"));
        let (s, ct, body) = get(format!("{base}/s/{T}/a.enee.yaml"), "text/html").await.unwrap();
        assert!(s == 200 && ct.starts_with("text/html") && body.contains("Water"), "{body}");
        assert_eq!(get(format!("{base}/s/{}/a.enee.yaml", "f".repeat(32)), APP_MIME).await.unwrap().0, 404);
        assert_eq!(get(format!("{base}/s/{T}/other.yaml"), APP_MIME).await.unwrap().0, 404);
        assert_eq!(get(format!("{base}/api/state"), APP_MIME).await.unwrap().0, 404);
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
    }
}
