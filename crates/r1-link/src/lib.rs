//! The link between an R1 and a computer (ADR-0048, docs/specs/companion.md §3–4), shared by r1d and the companion.
//!
//! - **Pairing** ([`PairingCode`]): the computer shows `r1os-link:1?id=…&s=…&n=…&a=…` as a QR; the R1's camera reads it,
//!   dials the computer by its key and proves the one-time secret ([`Msg::Pair`]). Both show the same [`short_code`]
//!   and pin each other's key.
//! - **The transport:** iroh ([`bind`]): QUIC between endpoints dialled by public key, end-to-end encrypted, direct
//!   when it can be and relayed when it can't. The R1 always dials; the computer accepts only keys it has pinned (and
//!   a pairing while its QR is up).
//! - **The protocol:** one JSON [`Msg`] per QUIC stream each way ([`ask`], [`answer`]), versioned by the ALPN.

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::presets;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

pub use iroh::endpoint::{Connection, SendStream};
pub use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, TransportAddr};

/// The protocol, as QUIC's ALPN: a new version is a new ALPN.
pub const ALPN: &[u8] = b"r1os/link/1";
pub const VERSION: u32 = 1;
/// A message's size at most (a turn's events are streamed, not sent whole).
const MAX_MSG: usize = 24 << 20;

// ---- pairing ----------------------------------------------------------------------------------------------

/// What the computer's QR says: its key, a one-time secret, its name, and where it is on its own network.
#[derive(Debug, Clone, PartialEq)]
pub struct PairingCode {
    pub id: EndpointId,
    pub secret: [u8; 16],
    pub name: String,
    pub addrs: Vec<SocketAddr>,
}

impl PairingCode {
    /// A new code for this computer: a fresh secret.
    pub fn new(id: EndpointId, name: &str, addrs: Vec<SocketAddr>) -> Result<Self> {
        let mut secret = [0u8; 16];
        getrandom::fill(&mut secret).map_err(|e| anyhow!("no randomness: {e}"))?;
        Ok(PairingCode { id, secret, name: name.chars().take(40).collect(), addrs })
    }

    /// `r1os-link:1?id=<key>&s=<secret, hex>&n=<name>&a=<addr>,<addr>`.
    pub fn to_uri(&self) -> String {
        let addrs: Vec<String> = self.addrs.iter().map(|a| a.to_string()).collect();
        format!("r1os-link:1?id={}&s={}&n={}&a={}", self.id, hex(&self.secret), escape(&self.name), escape(&addrs.join(",")))
    }

    pub fn parse(s: &str) -> Result<Self> {
        let rest = s.trim().strip_prefix("r1os-link:1?").ok_or_else(|| anyhow!("not an r1os link code"))?;
        let (mut id, mut secret, mut name, mut addrs) = (None, None, String::new(), Vec::new());
        for pair in rest.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = unescape(v);
            match k {
                "id" => id = Some(v.parse::<EndpointId>().map_err(|e| anyhow!("bad key: {e}"))?),
                "s" => secret = Some(unhex(&v).and_then(|b| <[u8; 16]>::try_from(b).ok()).ok_or_else(|| anyhow!("bad secret"))?),
                "n" => name = v.chars().take(40).collect(),
                "a" => addrs = v.split(',').filter_map(|a| a.parse().ok()).collect(),
                _ => {}
            }
        }
        Ok(PairingCode { id: id.context("no key")?, secret: secret.context("no secret")?, name, addrs })
    }

    /// Where to dial it: its key, and its addresses on its network (iroh finds the rest, or a relay).
    pub fn addr(&self) -> EndpointAddr {
        EndpointAddr::from_parts(self.id, self.addrs.iter().copied().map(TransportAddr::Ip))
    }
}

/// The four digits both screens show while pairing: from both keys and the secret, so a different R1 or computer in
/// the middle would show a different code.
pub fn short_code(a: &EndpointId, b: &EndpointId, secret: &[u8; 16]) -> String {
    let (lo, hi) = if a.as_bytes() <= b.as_bytes() { (a, b) } else { (b, a) };
    let mut h = Sha256::new();
    h.update(b"r1os-link short code");
    h.update(lo.as_bytes());
    h.update(hi.as_bytes());
    h.update(secret);
    let d = h.finalize();
    format!("{:04}", u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 10_000)
}

/// The secret, compared in constant time.
pub fn same_secret(a: &[u8; 16], b: &[u8; 16]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---- the endpoint -----------------------------------------------------------------------------------------

/// Where an endpoint looks names up (relays, and peers through n0's DNS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dns {
    /// The system's configuration (the computer).
    System,
    /// These servers, by UDP (the R1: reading Android's configuration needs its JVM, which a daemon hasn't got).
    Servers(Vec<std::net::IpAddr>),
}

/// How an endpoint may reach the other: any way, or only through a relay (to try "away from home" at home).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Anywhere,
    RelayOnly,
}

/// An endpoint with this key. `accept`: it takes connections (the computer); the R1 only dials.
pub async fn bind(key: SecretKey, reach: Reach, accept: bool, dns: Dns) -> Result<Endpoint> {
    bind_near(key, reach, accept, dns, None).await
}

/// [`bind`], with only `relay` in its relay map when given: the one an earlier endpoint settled on ([`home_relay`]).
/// iroh checks the network every 20–26 s while an endpoint's up, with a QUIC handshake (certificates checked) to every
/// relay in its map: four by default, which on the R1 was most of what an idle link cost (T-035). It still reaches a
/// computer homed on another relay through that one.
pub async fn bind_near(key: SecretKey, reach: Reach, accept: bool, dns: Dns, relay: Option<&str>) -> Result<Endpoint> {
    let mut b = Endpoint::builder(presets::N0).secret_key(key);
    if let Some(url) = relay.and_then(|u| u.parse::<iroh::RelayUrl>().ok()) {
        b = b.relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from(url)));
    }
    if let Dns::Servers(servers) = dns {
        let mut r = iroh::dns::DnsResolver::builder();
        for ip in servers {
            r = r.add_nameserver_config(iroh::dns::NameserverConfig::udp(ip));
        }
        b = b.dns_resolver(r.build());
    }
    if accept {
        b = b.alpns(vec![ALPN.to_vec()]);
    }
    if reach == Reach::RelayOnly {
        b = b.clear_ip_transports();
    }
    b.bind().await.map_err(|e| anyhow!("couldn't start the link: {e}"))
}

/// The relay an endpoint settled on as its home (the nearest it found), once its first look at the network is done.
pub fn home_relay(ep: &Endpoint) -> Option<String> {
    ep.addr().relay_urls().next().map(|u| u.to_string())
}

/// What the endpoint has done since it was bound, from iroh's counters: how often it looked at the network (a full
/// look closes its address-discovery connections, so the next costs new handshakes), why, and its relay connections.
pub fn stats(ep: &Endpoint) -> serde_json::Value {
    let m = ep.metrics();
    let (n, s) = (&m.net_report, &m.socket);
    serde_json::json!({
        "relay": home_relay(ep),
        "reports": n.reports.get(), "reports_full": n.reports_full.get(), "portmap_attempts": n.portmap_attempts.get(),
        "re_stun_ticks": s.actor_tick_re_stun.get(), "link_changes": s.actor_link_change.get(),
        "portmap_changes": s.actor_tick_portmap_changed.get(), "holepunch_attempts": s.holepunch_attempts.get(),
        "relay_conns": s.relay_conns_success.get(), "relay_conns_failed": s.relay_conns_failed.get(),
        "relay_home_changes": s.relay_home_change.get(),
    })
}

/// The key in `path` (32 bytes), made the first time (only its owner can read it).
pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    if let Ok(bytes) = std::fs::read(path) {
        let b: [u8; 32] = bytes.try_into().map_err(|_| anyhow!("{} isn't a key", path.display()))?;
        return Ok(SecretKey::from_bytes(&b));
    }
    let key = SecretKey::generate();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, key.to_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

/// How a connection's packets go now: `direct`, `relay`, or `none` (no path chosen yet).
pub fn path_kind(conn: &Connection) -> &'static str {
    for p in conn.paths().iter() {
        if p.is_selected() {
            return if p.is_ip() { "direct" } else { "relay" };
        }
    }
    "none"
}

/// The selected path's round trip, as QUIC measures it.
pub fn path_rtt(conn: &Connection) -> Option<Duration> {
    conn.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt())
}

// ---- messages ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Msg {
    /// The R1 asks to pair, with the secret from the QR.
    Pair { secret: String, name: String, version: u32 },
    /// The computer agrees, and both show `code`.
    Paired { name: String, code: String },
    /// No (a wrong or used secret, an unknown key, a no on the computer).
    Refused { why: String },
    /// A request; the answer is a [`Msg::Result`] on the same stream.
    Call { method: String, #[serde(default)] params: Value },
    Result { ok: bool, #[serde(default)] value: Value },
    /// Something that happened (no answer).
    Event { event: Value },
}

/// Sends `msg` on a new stream and reads the answer.
pub async fn ask(conn: &Connection, msg: &Msg) -> Result<Msg> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| anyhow!("link: {e}"))?;
    send.write_all(&serde_json::to_vec(msg)?).await.map_err(|e| anyhow!("link: {e}"))?;
    send.finish().map_err(|e| anyhow!("link: {e}"))?;
    let bytes = recv.read_to_end(MAX_MSG).await.map_err(|e| anyhow!("link: {e}"))?;
    serde_json::from_slice(&bytes).context("link: a bad answer")
}

/// Takes the next stream's message, and a way to answer it.
pub async fn next(conn: &Connection) -> Result<(Msg, iroh::endpoint::SendStream)> {
    let (send, mut recv) = conn.accept_bi().await.map_err(|e| anyhow!("link closed: {e}"))?;
    let bytes = recv.read_to_end(MAX_MSG).await.map_err(|e| anyhow!("link: {e}"))?;
    let msg = serde_json::from_slice(&bytes).context("link: a bad message")?;
    Ok((msg, send))
}

/// Answers a message [`next`] took.
pub async fn answer(mut send: iroh::endpoint::SendStream, msg: &Msg) -> Result<()> {
    send.write_all(&serde_json::to_vec(msg)?).await.map_err(|e| anyhow!("link: {e}"))?;
    send.finish().map_err(|e| anyhow!("link: {e}"))?;
    Ok(())
}

/// Sends `msg` on a new stream and reads what comes back line by line (one JSON message a line) until the other side
/// finishes: each to `each` as it comes (an agent's reply streaming), the last returned (its [`Msg::Result`]).
pub async fn ask_stream(conn: &Connection, msg: &Msg, mut each: impl FnMut(Msg)) -> Result<Msg> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| anyhow!("link: {e}"))?;
    send.write_all(&serde_json::to_vec(msg)?).await.map_err(|e| anyhow!("link: {e}"))?;
    send.finish().map_err(|e| anyhow!("link: {e}"))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut last = None;
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = recv.read(&mut chunk).await.map_err(|e| anyhow!("link: {e}"))?;
        let Some(n) = n else { break };
        buf.extend_from_slice(&chunk[..n]);
        anyhow::ensure!(buf.len() <= MAX_MSG, "link: a message over {} MB", MAX_MSG >> 20);
        while let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            if line.len() > 1 {
                let m: Msg = serde_json::from_slice(&line).context("link: a bad message")?;
                if let Some(prev) = last.replace(m) {
                    each(prev);
                }
            }
        }
    }
    last.context("link: the stream ended with nothing")
}

/// One message of a streamed answer ([`ask_stream`]): call it for each, then [`finish`].
pub async fn say(send: &mut iroh::endpoint::SendStream, msg: &Msg) -> Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    send.write_all(&line).await.map_err(|e| anyhow!("link: {e}"))
}

/// Ends a streamed answer.
pub fn finish(mut send: iroh::endpoint::SendStream) -> Result<()> {
    send.finish().map_err(|e| anyhow!("link: {e}"))
}

/// A call: `Ok(value)`, or the other side's error.
pub async fn call(conn: &Connection, method: &str, params: Value) -> Result<Value> {
    match ask(conn, &Msg::Call { method: method.into(), params }).await? {
        Msg::Result { ok: true, value } => Ok(value),
        Msg::Result { ok: false, value } => bail!("{}", value.as_str().unwrap_or("it failed")),
        other => bail!("link: unexpected {other:?}"),
    }
}

/// The secret as the QR writes it (for [`Msg::Pair`]).
pub fn secret_hex(secret: &[u8; 16]) -> String {
    hex(secret)
}

/// A secret from [`Msg::Pair`].
pub fn parse_secret(s: &str) -> Option<[u8; 16]> {
    unhex(s).and_then(|b| b.try_into().ok())
}

// ---- text ----------------------------------------------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Percent-escapes all but letters, digits and `-._~:,[]` (enough for names and addresses).
fn escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:,[]".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(x) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(x);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_agree() {
        let pc = SecretKey::generate().public();
        let r1 = SecretKey::generate().public();
        let code = PairingCode::new(pc, "Sam's PC & co", vec!["192.168.1.20:5544".parse().unwrap(), "[fe80::1]:5544".parse().unwrap()]).unwrap();
        let uri = code.to_uri();
        assert!(uri.starts_with("r1os-link:1?id="), "{uri}");
        assert_eq!(PairingCode::parse(&uri).unwrap(), code);
        assert!(PairingCode::parse("https://example.com").is_err());
        assert_eq!(short_code(&pc, &r1, &code.secret), short_code(&r1, &pc, &code.secret));
        assert_eq!(short_code(&pc, &r1, &code.secret).len(), 4);
        assert!(same_secret(&code.secret, &code.secret) && !same_secret(&code.secret, &[0; 16]));
    }

    #[tokio::test]
    async fn pair_and_call_over_a_real_endpoint() {
        // Two endpoints on this machine, no relay needed: the R1 dials the computer by its key and a local address.
        let pc = bind(SecretKey::generate(), Reach::Anywhere, true, Dns::System).await.unwrap();
        let r1 = bind(SecretKey::generate(), Reach::Anywhere, false, Dns::Servers(vec!["9.9.9.9".parse().unwrap()])).await.unwrap();
        let addrs: Vec<SocketAddr> = pc.addr().ip_addrs().copied().collect();
        let code = PairingCode::new(pc.id(), "pc", addrs).unwrap();
        let secret = code.secret;
        let pc_id = pc.id();
        let server = tokio::spawn(async move {
            let conn = pc.accept().await.unwrap().await.unwrap();
            let (msg, send) = next(&conn).await.unwrap();
            let Msg::Pair { secret: s, name, .. } = msg else { panic!("{msg:?}") };
            assert_eq!(name, "r1");
            assert!(same_secret(&unhex(&s).unwrap().try_into().unwrap(), &secret));
            answer(send, &Msg::Paired { name: "pc".into(), code: short_code(&pc_id, &conn.remote_id(), &secret) }).await.unwrap();
            let (msg, send) = next(&conn).await.unwrap();
            assert!(matches!(msg, Msg::Call { ref method, .. } if method == "ping"));
            answer(send, &Msg::Result { ok: true, value: serde_json::json!("pong") }).await.unwrap();
            // A streamed answer: three events, then the result.
            let (msg, mut send) = next(&conn).await.unwrap();
            assert!(matches!(msg, Msg::Call { ref method, .. } if method == "turn"));
            for i in 0..3 {
                say(&mut send, &Msg::Event { event: serde_json::json!({ "n": i }) }).await.unwrap();
            }
            say(&mut send, &Msg::Result { ok: true, value: serde_json::json!("done") }).await.unwrap();
            finish(send).unwrap();
            conn.closed().await;
        });
        let conn = r1.connect(code.addr(), ALPN).await.unwrap();
        let reply = ask(&conn, &Msg::Pair { secret: hex(&code.secret), name: "r1".into(), version: VERSION }).await.unwrap();
        let Msg::Paired { code: shown, .. } = reply else { panic!("{reply:?}") };
        assert_eq!(shown, short_code(&r1.id(), &code.id, &code.secret));
        assert_eq!(call(&conn, "ping", Value::Null).await.unwrap(), "pong");
        let mut seen = Vec::new();
        let last = ask_stream(&conn, &Msg::Call { method: "turn".into(), params: Value::Null }, |m| seen.push(m)).await.unwrap();
        assert_eq!(seen.len(), 3);
        assert!(matches!(last, Msg::Result { ok: true, .. }));
        conn.close(0u32.into(), b"bye");
        let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    }
}
