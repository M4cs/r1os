//! Web requests for apps (docs/specs/apps.md §6: `http.json`, `http.rss`, `http.fetch`). r1d makes them,
//! never an app's page, for an app the user let reach the URL's host (`net.http`, crate::broker). They are
//! SSRF-safe:
//! - https only, to a host name the app declared (no IP literals, no user info), and never through a proxy;
//! - every address the name resolves to must be public. Loopback, private, link-local, shared (CGNAT), multicast,
//!   documentation and other special ranges are dropped after DNS, so a public name can't point into the network
//!   (r1d's own port on loopback included);
//! - redirects are followed here, at most 3, each to a declared host and through the same checks;
//! - 10 s per request; a body of at most 1 MB (4 MB for a feed).
//!
//! Ported from eneeware apps/desktop/src-tauri/src/fetch.rs at 61bc7a7.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use serde_json::Value;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::{Host, Url};

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BODY: u64 = 1 << 20;
const MAX_FEED: u64 = 4 << 20;
const MAX_IMAGE: u64 = 5 << 20;
const MAX_REDIRECTS: usize = 3;

/// What an http service asks for: how it reads the response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `http.json`: a JSON body.
    Json,
    /// `http.rss`: an RSS or Atom feed (`feed.rs`).
    Feed,
    /// `http.fetch`: the body as text.
    Text,
    /// A picture on a chat's card (crate::cards).
    Image,
}

impl Format {
    pub fn of(uses: &str) -> Option<Format> {
        match uses {
            "http.json" => Some(Format::Json),
            "http.rss" => Some(Format::Feed),
            "http.fetch" => Some(Format::Text),
            _ => None,
        }
    }

    fn accept(self) -> &'static str {
        match self {
            Format::Json => "application/json",
            Format::Feed => {
                "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/xml;q=0.8, */*;q=0.5"
            }
            Format::Text => "text/html, text/plain;q=0.9, */*;q=0.5",
            Format::Image => "image/webp, image/png, image/jpeg, image/*;q=0.8",
        }
    }

    fn limit(self) -> u64 {
        match self {
            Format::Feed => MAX_FEED,
            Format::Image => MAX_IMAGE,
            Format::Json | Format::Text => MAX_BODY,
        }
    }
}

/// A response: its status, its body and its content type.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

/// Why a request didn't produce a value.
#[derive(Debug, PartialEq)]
pub enum FetchError {
    /// The URL, or a redirect, isn't allowed.
    Refused(String),
    /// The request failed, or the body isn't JSON.
    Failed(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Refused(why) | FetchError::Failed(why) => f.write_str(why),
        }
    }
}

fn refused(why: impl Into<String>) -> FetchError {
    FetchError::Refused(why.into())
}

/// A JSON response: its status (an API's 404 often explains itself in JSON) and body.
#[derive(Debug)]
pub struct Fetched {
    pub status: u16,
    pub body: Value,
}

/// One HTTP agent for every app (connections are pooled per host).
pub struct Fetcher {
    agent: ureq::Agent,
}

impl Fetcher {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("r1os/", env!("CARGO_PKG_VERSION")))
            .accept("application/json")
            .build();
        Fetcher { agent: ureq::Agent::with_parts(config, DefaultConnector::default(), PublicOnly::default()) }
    }

    /// GETs `url` and parses the body as JSON. `allowed` decides each host: the first one and every redirect's.
    pub fn json(&self, url: &str, allowed: &dyn Fn(&str) -> bool) -> Result<Fetched, FetchError> {
        let response = self.get(url, Format::Json, allowed)?;
        let status = response.status;
        let body = serde_json::from_slice(&response.body)
            .map_err(|_| FetchError::Failed(format!("the response isn't JSON (HTTP {status})")))?;
        Ok(Fetched { status, body })
    }

    /// GETs `url` for `format`, following redirects to allowed hosts.
    pub fn get(&self, url: &str, format: Format, allowed: &dyn Fn(&str) -> bool) -> Result<Response, FetchError> {
        let failed = |e: ureq::Error| FetchError::Failed(describe(&e));
        let mut url = check_url(url, allowed)?;
        for _ in 0..=MAX_REDIRECTS {
            let mut response = self.agent.get(url.as_str()).header("accept", format.accept()).call().map_err(failed)?;
            let status = response.status().as_u16();
            if (300..400).contains(&status) {
                let location = response.headers().get("location").and_then(|l| l.to_str().ok());
                let next = location.and_then(|l| url.join(l).ok()).ok_or_else(|| refused("a redirect went nowhere"))?;
                url = check_url(next.as_str(), allowed)?;
                continue;
            }
            let content_type = response.headers().get("content-type").and_then(|t| t.to_str().ok()).map(str::to_string);
            let body = response.body_mut().with_config().limit(format.limit()).read_to_vec().map_err(failed)?;
            return Ok(Response { status, body, content_type });
        }
        Err(refused(format!("more than {MAX_REDIRECTS} redirects")))
    }
}

/// An agent for a station's stream (crate::player): http or https, public addresses only (every hop: the resolver
/// keeps only public ones), a few redirects, no time limit on the body, which never ends.
pub fn stream_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .https_only(false)
        .proxy(None)
        .max_redirects(3)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .user_agent(concat!("r1os/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::with_parts(config, DefaultConnector::default(), PublicOnly::default())
}

/// A short, user-facing reason for a failed request.
fn describe(e: &ureq::Error) -> String {
    match e {
        ureq::Error::HostNotFound => "the host couldn't be found at a public address".into(),
        ureq::Error::Timeout(_) => format!("no answer in {} s", TIMEOUT.as_secs()),
        ureq::Error::BodyExceedsLimit(limit) => format!("the response is over {} KB", limit / 1024),
        other => other.to_string(),
    }
}

/// The URL, if it's https to a host `allowed` accepts (a name, without user info).
fn check_url(raw: &str, allowed: &dyn Fn(&str) -> bool) -> Result<Url, FetchError> {
    let url = Url::parse(raw).map_err(|_| refused("not a valid URL"))?;
    if url.scheme() != "https" {
        return Err(refused("only https URLs can be fetched"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refused("URLs can't carry a user name or password"));
    }
    match url.host() {
        Some(Host::Domain(host)) if allowed(host) => Ok(url),
        Some(Host::Domain(host)) => Err(refused(format!("{host} isn't one of the app's hosts"))),
        _ => Err(refused("the URL needs a host name")),
    }
}

/// A link an app may open in the R1's link viewer (`open_url`): http(s), no user info, and a host on the web
/// (a name that isn't local, or a public address). The viewer checks each page it loads the same way.
pub fn link_ok(raw: &str) -> Result<String, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "not a valid link".to_owned())?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err("only http(s) links open".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("links can't carry a user name or password".into());
    }
    let public = match url.host() {
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d.contains('.') && !d.ends_with(".local") && !d.ends_with(".localhost") && !d.ends_with(".internal")
        }
        Some(Host::Ipv4(ip)) => public_v4(ip),
        Some(Host::Ipv6(ip)) => public_v6(ip),
        None => false,
    };
    if !public {
        return Err("only links to the web open".into());
    }
    Ok(url.into())
}

/// The system resolver, keeping only public addresses (none left: the host isn't found).
#[derive(Debug, Default)]
struct PublicOnly(DefaultResolver);

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let resolved = self.0.resolve(uri, config, timeout)?;
        let mut public = self.empty();
        for addr in resolved.iter().filter(|a| is_public(a.ip())) {
            public.push(*addr);
        }
        if public.is_empty() {
            tracing::warn!(host = ?uri.host(), "fetch: refused, the host has no public address");
            return Err(ureq::Error::HostNotFound);
        }
        Ok(public)
    }
}

/// A globally routable address: not loopback, private, link-local, shared, multicast, documentation, benchmarking,
/// reserved or unspecified. IPv6 forms that embed an IPv4 address are judged by it; 6to4 and Teredo are refused.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (b & 0xc0) == 64) // shared address space (CGNAT), 100.64.0.0/10
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments, 192.0.0.0/24
        || (a == 198 && (b & 0xfe) == 18) // benchmarking, 198.18.0.0/15
        || a >= 240) // reserved, 240.0.0.0/4
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let s = ip.segments();
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        // NAT64, 64:ff9b::/96
        return public_v4(Ipv4Addr::from((u32::from(s[6]) << 16) | u32::from(s[7])));
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || s[0..6] == [0, 0, 0, 0, 0, 0] // IPv4-compatible (deprecated)
        || (s[0] & 0xfe00) == 0xfc00 // unique local, fc00::/7
        || (s[0] & 0xffc0) == 0xfe80 // link-local, fe80::/10
        || (s[0] & 0xffc0) == 0xfec0 // site-local (deprecated), fec0::/10
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation, 2001:db8::/32
        || (s[0] == 0x2001 && s[1] == 0) // Teredo, 2001::/32
        || s[0] == 0x2002 // 6to4, 2002::/16
        || (s[0] == 0x0100 && s[1..4] == [0, 0, 0])) // discard-only, 100::/64
}

/// Builds a service URL from its template: each `{{state.x}}` / `{{settings.x}}` becomes its value, percent-encoded
/// (so a value can't change the host, path or query structure). `None` when a value is missing or empty.
pub fn fill_url(template: &str, value: &dyn Fn(&str) -> Option<Value>) -> Option<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let len = rest[start..].find("}}")?;
        out.push_str(&rest[..start]);
        let pointer = format!("/{}", rest[start + 2..start + len].trim().replace('.', "/"));
        let text = match value(&pointer)? {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => return None,
        };
        // Nothing to ask for yet; `.` and `..` would move up the path, even encoded.
        if matches!(text.trim(), "" | "." | "..") {
            return None;
        }
        out.extend(url::form_urlencoded::byte_serialize(text.trim().as_bytes()).map(|s| s.replace('+', "%20")));
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_go_only_to_the_web() {
        assert_eq!(link_ok("https://example.com/a?b=1").as_deref(), Ok("https://example.com/a?b=1"));
        assert!(link_ok("http://news.ycombinator.com").is_ok());
        for bad in [
            "javascript:alert(1)",
            "file:///data/r1os/run/token",
            "http://127.0.0.1:7171/api/agents",
            "http://localhost:7180/",
            "http://[::1]/",
            "http://192.168.1.10/",
            "http://printer.local/",
            "https://user:pw@example.com/",
            "intent://scan#Intent;end",
        ] {
            assert!(link_ok(bad).is_err(), "{bad}");
        }
    }
    use serde_json::json;

    #[test]
    fn only_public_addresses() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.10",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "2002:c0a8:101::1",
            "2001:db8::1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private} must be refused");
        }
        for public in ["1.1.1.1", "142.250.80.46", "2606:4700:4700::1111", "::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(is_public(public.parse().unwrap()), "{public} is public");
        }
    }

    #[test]
    fn urls_must_be_https_to_a_declared_name() {
        let allowed = |h: &str| h == "api.example.com";
        assert!(check_url("https://api.example.com/v1?q=1", &allowed).is_ok());
        assert!(check_url("https://API.Example.com/v1", &allowed).is_ok(), "hosts compare lowercased");
        for bad in [
            "http://api.example.com/",
            "https://other.example.com/",
            "https://user:pw@api.example.com/",
            "https://127.0.0.1/",
            "https://[::1]/",
            "file:///C:/Windows/win.ini",
            "not a url",
        ] {
            assert!(matches!(check_url(bad, &allowed), Err(FetchError::Refused(_))), "{bad} must be refused");
        }
    }

    /// Real requests: `cargo test -p r1d fetch -- --ignored`.
    #[test]
    #[ignore = "needs the network"]
    fn live_public_json() {
        let fetcher = Fetcher::new();
        let any = |_: &str| true;
        // Binance answers in JSON either way; from the US it's 451 (geo-blocked).
        let binance = fetcher.json("https://api.binance.com/api/v3/ticker/price?symbol=BTCUSDT", &any).unwrap();
        assert!(binance.status == 451 || binance.body["symbol"] == "BTCUSDT", "{binance:?}");
        let yahoo = fetcher.json("https://query1.finance.yahoo.com/v8/finance/chart/AAPL?range=1d&interval=5m", &any);
        let yahoo = yahoo.unwrap();
        assert!(yahoo.body["chart"]["result"][0]["meta"]["regularMarketPrice"].is_number(), "{}", yahoo.status);
        let missing = fetcher.json("https://query1.finance.yahoo.com/v8/finance/chart/NOPE-NOT-REAL", &any).unwrap();
        assert_eq!(missing.status, 404, "an API's own error JSON comes through");
        let only_binance = |h: &str| h == "api.binance.com";
        assert!(matches!(fetcher.json("https://example.com/", &only_binance), Err(FetchError::Refused(_))));
        let local = fetcher.json("https://localhost/", &any);
        assert!(matches!(local, Err(FetchError::Failed(ref e)) if e.contains("public address")), "{local:?}");
    }

    /// A real feed: `cargo test -p r1d live_feed -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_feed() {
        let fetcher = Fetcher::new();
        let hn = |h: &str| h == "hnrss.org";
        let response = fetcher.get("https://hnrss.org/frontpage", Format::Feed, &hn).unwrap();
        assert_eq!(response.status, 200);
        let feed = crate::feed::parse(&String::from_utf8_lossy(&response.body)).unwrap();
        println!("{} ({} items): {:?}", feed.title, feed.items.len(), feed.items.first());
        assert!(!feed.items.is_empty() && feed.items.iter().all(|i| !i.guid.is_empty() && i.published.is_some()));
        let page = fetcher.get("https://example.com/", Format::Text, &|h: &str| h == "example.com").unwrap();
        assert!(String::from_utf8_lossy(&page.body).contains("Example Domain"));
    }

    #[test]
    fn templates_fill_in_encoded_values() {
        let vars = |p: &str| match p {
            "/state/ticker" => Some(json!("BRK B/../x?y")),
            "/settings/days" => Some(json!(7)),
            "/state/empty" => Some(json!("  ")),
            _ => None,
        };
        assert_eq!(
            fill_url("https://api.example.com/q/{{state.ticker}}?days={{ settings.days }}", &vars).as_deref(),
            Some("https://api.example.com/q/BRK%20B%2F..%2Fx%3Fy?days=7")
        );
        assert_eq!(fill_url("https://api.example.com/{{state.empty}}", &vars), None, "an empty value waits");
        let dots = |_: &str| Some(json!(".."));
        assert_eq!(fill_url("https://api.example.com/v1/{{state.x}}", &dots), None, "can't climb the path");
        assert_eq!(fill_url("https://api.example.com/{{state.gone}}", &vars), None);
        assert_eq!(fill_url("https://api.example.com/plain", &vars).as_deref(), Some("https://api.example.com/plain"));
    }
}
