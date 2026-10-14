//! Who may call r1d (ADR-0026, docs/specs/r1d.md §6). r1d listens on 127.0.0.1, which every process on the
//! R1 can reach: Android's apps, the agents' jail, and any web page a WebView loads. So `/api`, `/harness` and
//! `/ws` take a bearer token (`Authorization: Bearer …`):
//!
//! - **The per-boot token**, made fresh each time r1d starts. It allows everything. Root reads it from
//!   `/data/r1os/run/token` (0600; `tools/r1h` over adb). The host app asks for it (`GET /token`), and r1d
//!   answers only a connection whose socket the kernel lists as the host's uid (ADR-0036, which replaced
//!   writing it into the host's own files: AOSP's SELinux refuses that to any daemon, ADR-0035).
//! - **An app's page token** (`POST /api/apps/{id}/session`, with the per-boot token): what a mini-app's
//!   WebView gets. It allows only that app's own runtime endpoints, so a page can't reach other apps, their
//!   permissions, the agents or the harness.
//!
//! Not covered: `/mcp` and `/hooks/pre-tool-use` (each agent process has its own token, crate::mcp), the shell's static files (no
//! data), and the simulator (`--sim` runs on a PC and is open). The mic socket checks the per-boot token in
//! its hello (crate::voice).
//!
//! Before any of that, on the R1 r1d answers only requests addressed to it ([addressed], T-186): a `Host` of
//! `127.0.0.1`, `localhost` or `[::1]`. A web page on another name that resolves to 127.0.0.1 (DNS rebinding) in
//! one of the host's WebViews would otherwise share the host's uid, which `/token` trusts.

use crate::hub::Hub;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The app that gets the token.
const HOST_PACKAGE: &str = "dev.r1os.host";

pub struct Auth {
    enabled: bool,
    master: String,
    /// Page tokens: token → app id.
    apps: Mutex<HashMap<String, String>>,
    /// The host app's uid (from the package manager), and when it was last looked up: it changes only
    /// when the app is installed afresh, so it's looked up again on a mismatch, at most every 10 s.
    host_uid: Mutex<(Option<u32>, Option<Instant>)>,
    root: std::path::PathBuf,
}

impl Auth {
    /// `enabled`: on the R1 (the simulator is open). `root`: r1d's data (`/data/r1os`).
    pub fn new(enabled: bool, root: &Path) -> Self {
        Auth {
            enabled,
            master: crate::core::random_hex(24),
            apps: Mutex::default(),
            host_uid: Mutex::default(),
            root: root.to_path_buf(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The per-boot token (the mic's hello carries it).
    pub fn master(&self) -> &str {
        &self.master
    }

    /// A token for `app`'s page: one per app while r1d runs.
    pub fn app_token(&self, app: &str) -> String {
        let mut apps = self.apps.lock().unwrap();
        if let Some((t, _)) = apps.iter().find(|(_, a)| a.as_str() == app) {
            return t.clone();
        }
        let t = crate::core::random_hex(24);
        apps.insert(t.clone(), app.to_owned());
        t
    }

    /// Whether `token` may make this request.
    pub fn allows(&self, method: &Method, path: &str, token: Option<&str>) -> bool {
        if !self.enabled || !guarded(path) {
            return true;
        }
        let Some(token) = token else { return false };
        if equal(token, &self.master) {
            return true;
        }
        let app = self.apps.lock().unwrap().get(token).cloned();
        app.is_some_and(|app| page_may(method, path, &app))
    }

    /// Writes the token where root can read it (the tools). Best effort: logged on failure.
    pub fn publish(&self) {
        if !self.enabled {
            return;
        }
        let run = self.root.join("run");
        let wrote = std::fs::create_dir_all(&run)
            .and_then(|()| write_private(&run.join("token"), &self.master))
            .and_then(|()| set_mode(&run, 0o700));
        if let Err(e) = wrote {
            tracing::warn!(error = %e, "auth: couldn't write the token for tools");
        }
    }

    /// The token, for the host app only (`GET /token`, ADR-0036): `peer` is the connection's other end,
    /// `port` r1d's. The kernel's socket table says whose that socket is; r1d compares it with the host
    /// app's uid. The simulator is open, so it answers anyone there.
    pub async fn for_host(&self, peer: SocketAddr, port: u16) -> Result<String, &'static str> {
        if !self.enabled {
            return Ok(self.master.clone());
        }
        if !peer.ip().is_loopback() {
            return Err("not from this R1");
        }
        let table = [std::fs::read_to_string("/proc/net/tcp"), std::fs::read_to_string("/proc/net/tcp6")]
            .into_iter()
            .filter_map(Result::ok)
            .collect::<String>();
        let Some(uid) = socket_owner(&table, peer.port(), port) else { return Err("whose connection is this?") };
        if self.host_uid(false).await == Some(uid) || self.host_uid(true).await == Some(uid) {
            Ok(self.master.clone())
        } else {
            tracing::warn!(uid, "auth: refused the token to a process that isn't the host app");
            Err("only the host app gets r1d's token")
        }
    }

    /// The host app's uid, cached; `fresh` looks it up again (at most every 10 s).
    async fn host_uid(&self, fresh: bool) -> Option<u32> {
        {
            let (uid, at) = *self.host_uid.lock().unwrap();
            let recent = at.is_some_and(|t| t.elapsed() < Duration::from_secs(10));
            if (uid.is_some() && !fresh) || (fresh && recent) {
                return uid;
            }
        }
        let out = crate::tool::run("cmd", &["package", "list", "packages", "-U", HOST_PACKAGE], crate::tool::DEADLINE).await.ok()?;
        let uid = package_uid(&out.stdout, HOST_PACKAGE);
        *self.host_uid.lock().unwrap() = (uid, Some(Instant::now()));
        uid
    }
}

/// The uid of the socket whose local port is `peer_port` and whose other end is `server_port`, connected
/// (`/proc/net/tcp` and `tcp6` rows: `sl local rem st … uid …`, ports in hex).
fn socket_owner(table: &str, peer_port: u16, server_port: u16) -> Option<u32> {
    let port_of = |addr: &str| addr.rsplit_once(':').and_then(|(_, p)| u16::from_str_radix(p, 16).ok());
    table.lines().find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 || f[3] != "01" {
            return None;
        }
        if port_of(f[1]) == Some(peer_port) && port_of(f[2]) == Some(server_port) { f[7].parse().ok() } else { None }
    })
}

/// `package:dev.r1os.host uid:10054` → 10054.
fn package_uid(out: &str, package: &str) -> Option<u32> {
    let want = format!("package:{package}");
    out.lines()
        .find(|l| l.split_whitespace().next() == Some(want.as_str()))?
        .split_whitespace()
        .find_map(|w| w.strip_prefix("uid:"))?
        .parse()
        .ok()
}

/// The paths that take a token.
fn guarded(path: &str) -> bool {
    path.starts_with("/api/") || path == "/api" || path.starts_with("/harness/") || path == "/ws"
}

/// What an app's page may call: its own app, and only the runtime's endpoints.
fn page_may(method: &Method, path: &str, app: &str) -> bool {
    let Some(rest) = path.strip_prefix("/api/apps/") else { return false };
    let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if id != app {
        return false;
    }
    matches!(
        (method.as_str(), tail),
        ("GET", "" | "settings" | "services" | "player")
            | ("POST", "init" | "query" | "step" | "state" | "services/vars" | "errors" | "open_url" | "copy" | "media" | "fetch")
    )
}

/// Compares in time independent of where they differ.
fn equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The middleware: refuses guarded paths without a token that allows them (401).
pub async fn layer(State(hub): State<Arc<Hub>>, req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    if hub.auth.allows(req.method(), req.uri().path(), token) {
        return next.run(req).await;
    }
    (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "r1d needs its token" }))).into_response()
}

/// Whether a request's `Host` is r1d's own loopback address, on any port (`tools/r1h` forwards another), or absent
/// (the link's calls through the router, crate::link).
fn ours(host: Option<&str>) -> bool {
    let Some(host) = host else { return true };
    let (name, port) = match host.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((name, rest)) if rest.is_empty() || rest.starts_with(':') => (name, rest.strip_prefix(':')),
            _ => return false,
        },
        None => match host.split_once(':') {
            Some((name, port)) => (name, Some(port)),
            None => (host, None),
        },
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok && (name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost") || name == "::1")
}

/// The outermost middleware on the R1: refuses (421) a request addressed to another name (T-186). The simulator, and
/// an R1 with `/data/r1os/auth-off`, answer any.
pub async fn addressed(State(hub): State<Arc<Hub>>, req: Request, next: Next) -> Response {
    // A `Host` that isn't text is nobody's: refused.
    let host = match req.headers().get(axum::http::header::HOST) {
        Some(v) => Some(v.to_str().unwrap_or("?").to_owned()),
        None => req.uri().authority().map(|a| a.as_str().to_owned()),
    };
    if !hub.auth.enabled() || ours(host.as_deref()) {
        return next.run(req).await;
    }
    let shown: String = host.unwrap_or_default().chars().take(80).collect();
    tracing::warn!(host = %shown, path = req.uri().path(), "refused a request addressed to another host");
    (StatusCode::MISDIRECTED_REQUEST, axum::Json(serde_json::json!({ "error": "r1d answers only 127.0.0.1" }))).into_response()
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    set_mode(&tmp, 0o600)?;
    std::fs::rename(&tmp, path)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_requests_addressed_to_r1d() {
        for host in [None, Some("127.0.0.1:7171"), Some("127.0.0.1:7172"), Some("localhost:5171"), Some("LOCALHOST"), Some("[::1]:7171"), Some("127.0.0.1")] {
            assert!(ours(host), "{host:?}");
        }
        // DNS rebinding: a public name that resolves to 127.0.0.1.
        for host in ["evil.example:7171", "evil.example", "127.0.0.1.evil.example:7171", "localhost.evil.example", "127.0.0.1:", "127.0.0.1:x", "[::1", "[::1]x", "[::2]:7171", "192.168.1.4:7171", "?"] {
            assert!(!ours(Some(host)), "{host}");
        }
    }

    #[test]
    fn tokens_and_their_reach() {
        let a = Auth::new(true, Path::new("/nonexistent"));
        let m = a.master().to_owned();
        let get = Method::GET;
        let post = Method::POST;
        assert!(a.allows(&get, "/app.html", None), "static files are open");
        assert!(a.allows(&post, "/mcp", None), "mcp checks its own tokens");
        assert!(!a.allows(&get, "/api/agents", None));
        assert!(!a.allows(&get, "/api/agents", Some("nope")));
        assert!(a.allows(&get, "/api/agents", Some(&m)));
        assert!(a.allows(&get, "/ws", Some(&m)) && !a.allows(&get, "/ws", None));
        assert!(!a.allows(&post, "/harness/wheel", None));

        let t = a.app_token("water");
        assert_eq!(a.app_token("water"), t, "one per app");
        assert!(a.allows(&post, "/api/apps/water/step", Some(&t)));
        assert!(a.allows(&get, "/api/apps/water", Some(&t)));
        assert!(a.allows(&get, "/api/apps/water/services", Some(&t)));
        assert!(!a.allows(&post, "/api/apps/water/permissions", Some(&t)), "not its permissions");
        assert!(!a.allows(&post, "/api/apps/water/settings", Some(&t)), "nor writing its settings");
        assert!(!a.allows(&post, "/api/apps/notes/step", Some(&t)), "nor another app");
        assert!(!a.allows(&get, "/api/agents", Some(&t)));
        assert!(!a.allows(&post, "/api/apps/water/session", Some(&t)), "nor minting tokens");

        let open = Auth::new(false, Path::new("/nonexistent"));
        assert!(open.allows(&get, "/api/agents", None), "the simulator is open");
    }

    #[test]
    fn the_token_goes_by_the_socket_owner() {
        // r1d listening on 7171 (0x1C03), the host's connection from port 54498 (0xD4E2) as uid 10054, and
        // r1d's own end of it (uid 0).
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1C03 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 127281 1
   1: 0100007F:D4E2 0100007F:1C03 01 00000000:00000000 00:00000000 00000000 10054        0 130001 1
   2: 0100007F:1C03 0100007F:D4E2 01 00000000:00000000 00:00000000 00000000     0        0 130002 1
";
        assert_eq!(socket_owner(table, 54498, 7171), Some(10054));
        assert_eq!(socket_owner(table, 54499, 7171), None, "no such connection");
        assert_eq!(socket_owner(table, 7171, 54498), Some(0), "r1d's own end is r1d's");
        assert_eq!(package_uid("package:dev.r1os.host uid:10054\n", "dev.r1os.host"), Some(10054));
        assert_eq!(package_uid("package:dev.r1os.hostile uid:10099\n", "dev.r1os.host"), None);
    }
}
