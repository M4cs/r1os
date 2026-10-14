//! The R1's browser (ADR-0054, R8; Max 2026-10-11: "some mini webview browser that the agent can control on-device
//! accessible with tooling that mimics what browser use claude would have access to on a computer").
//!
//! - **Where:** a WebView in the host. r1d asks it to do something (`browser {id, op, agent, chat, args}`), and the host
//!   answers at `POST /api/browser/result {id, ok, result | error}`; a tool waits for that ([`call`]).
//! - **Its tools** are like Claude in Chrome's: `browser_open`, `browser_read` (the page as lines with element refs),
//!   `browser_text`, `browser_find`, `browser_click`, `browser_type`, `browser_key`, `browser_scroll`,
//!   `browser_screenshot` (an image), `browser_back`, `browser_close`, and `browser_ask_user` (you take over:
//!   "log me in to this site", then "done").
//! - **Reading runs; acting asks once per task** (ADR-0053's rule): the first click, typing or key in a turn shows the
//!   approval card (the `browser` level: ask, allow, never), and "allow" lets the rest of that turn go ahead
//!   ([`end_turn`] clears it).
//! - **Where it may go:** web pages only (http and https), never the R1 itself or the local network ([`allowed_url`]);
//!   the host refuses them too, for every request a page makes.
//! - **Its sign-ins** are the WebView's own cookies, on the R1. r1os never sees passwords: you type them yourself after
//!   `browser_ask_user`.

use crate::hub::{Event, Hub, Source};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

/// How long a tool waits for the host (a page can take a while to load).
const WAIT: Duration = Duration::from_secs(45);
/// How long `browser_ask_user` waits for you.
const ASK_WAIT: Duration = Duration::from_secs(5 * 60);

type Answer = tokio::sync::oneshot::Sender<Result<Value, String>>;

#[derive(Default)]
pub struct Browser {
    pending: Mutex<HashMap<String, Answer>>,
    /// Chats allowed to act in the browser for the rest of their turn.
    granted: Mutex<HashSet<String>>,
    /// One step at a time, in the order they came (crate::mcp's `call`), as in the app window.
    pub turn: tokio::sync::Mutex<()>,
}

impl Browser {
    /// The host's answer to a request.
    pub fn answer(&self, id: &str, result: Result<Value, String>) -> bool {
        match self.pending.lock().unwrap().remove(id) {
            Some(tx) => tx.send(result).is_ok(),
            None => false,
        }
    }

    /// A turn ended: what it was allowed to do in the browser goes with it.
    pub fn end_turn(&self, chat: &str) {
        self.granted.lock().unwrap().remove(chat);
    }
}

/// The chat's turn was stopped (crate::stops): it may no longer act in the browser, and if it was asking you to take
/// over, the host lets that go (`browser {op: "stopped", chat}`).
pub fn stopped(hub: &Hub, chat: &str) {
    hub.browser.end_turn(chat);
    hub.emit(Event::Browser { id: String::new(), op: "stopped".into(), agent: String::new(), chat: chat.to_owned(), args: Value::Null }, Source::Api);
}

/// Asks the host to do `op` in the browser and waits for its answer.
pub async fn call(hub: &Hub, agent: &str, chat: &str, op: &str, args: Value) -> Result<Value, String> {
    let id = crate::core::random_hex(6);
    let (tx, rx) = tokio::sync::oneshot::channel();
    hub.browser.pending.lock().unwrap().insert(id.clone(), tx);
    hub.emit(Event::Browser { id: id.clone(), op: op.to_owned(), agent: agent.to_owned(), chat: chat.to_owned(), args }, Source::Api);
    let wait = if op == "ask_user" { ASK_WAIT } else { WAIT };
    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(r)) => r,
        _ => {
            hub.browser.pending.lock().unwrap().remove(&id);
            Err(if op == "ask_user" { "The user didn't answer in 5 minutes.".into() } else { "The R1's browser didn't answer (is the R1's app running?).".into() })
        }
    }
}

/// A web page's address, or why not: http(s) only, never the R1 itself or the local network.
pub fn allowed_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let url = if url.contains("://") { url.to_owned() } else { format!("https://{url}") };
    let (scheme, rest) = url.split_once("://").ok_or("that isn't a web address")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return Err(format!("only web pages (http and https), not {scheme}:"));
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or_default();
    let host = if host.starts_with('[') { host.split(']').next().unwrap_or_default().trim_start_matches('[') } else { host.split(':').next().unwrap_or_default() };
    if host.is_empty() || private_host(host) {
        return Err("not the R1 itself or the local network: only public web pages".into());
    }
    Ok(url)
}

/// The R1, the local network, or link-local: what the browser never reaches.
pub fn private_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h.ends_with(".lan") || h.ends_with(".internal") || !h.contains(['.', ':']) {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast() || v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

/// Before the first act in a turn (a click, typing, a key): the `browser` level, or the card. `what` is the act in
/// words ("click “Add to cart”"), `site` where.
pub async fn may_act(hub: &Hub, agent: &str, chat: &str, what: &str, site: &str) -> Result<(), String> {
    if hub.browser.granted.lock().unwrap().contains(chat) {
        return Ok(());
    }
    let mut ask = crate::approvals::Ask::new("browser", format!("use the R1's browser: {what}"), if site.is_empty() { String::new() } else { format!("on {site}") });
    // Another agent's message started this turn: the card says so.
    if let Some(w) = hub.mail.woken(chat) {
        let from = hub.core.agent(&w.by).map(|a| a.name).unwrap_or_else(|| "another agent".into());
        ask = ask.why(&format!("{from} asked it to"));
    }
    hub.approvals.gate(hub, agent, ask.chat(chat)).await?;
    hub.browser.granted.lock().unwrap().insert(chat.to_owned());
    Ok(())
}

/// What an op's answer says, for the agent (its page, what happened): the host's own words, or JSON.
pub fn words(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(m) if m.contains_key("text") && m.len() <= 3 => {
            let mut out = String::new();
            if let Some(u) = m.get("url").and_then(Value::as_str) {
                out.push_str(&format!("[{u}]\n"));
            }
            out.push_str(m["text"].as_str().unwrap_or_default());
            out
        }
        other => other.to_string(),
    }
}

/// Ops that act on a page, which ask first.
pub fn acts(op: &str) -> bool {
    matches!(op, "click" | "type" | "key" | "select")
}

/// The MCP tool's answer for an op: text, or (a screenshot) an image.
pub fn reply(op: &str, r: Result<Value, String>) -> Value {
    match r {
        Ok(v) if op == "screenshot" => match v["image"].as_str() {
            Some(b64) => json!({ "content": [
                { "type": "image", "data": b64, "mimeType": "image/jpeg" },
                { "type": "text", "text": format!("[{}] {}x{}", v["url"].as_str().unwrap_or_default(), v["width"], v["height"]) }
            ], "isError": false }),
            None => json!({ "content": [{ "type": "text", "text": "No screenshot: nothing's open." }], "isError": true }),
        },
        Ok(v) => json!({ "content": [{ "type": "text", "text": words(&v) }], "isError": false }),
        Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
    }
}

// ---- System One in the browser (R8; Max, after browser-use's jev-ultrafast) --------------------------------------

/// What you said while the R1's browser is on screen, as System One reads it: done on the R1 at once, with no model.
#[derive(Debug, PartialEq)]
pub enum Said {
    Scroll(&'static str),
    Back,
    Reload,
    Close,
    Open(String),
    Search(String),
    /// The nth (1-based; 0 is the last) of a kind: `result` (a link with a title), `link`, `button`, or any.
    Nth(usize, &'static str),
    Click(String),
}

const ORDINALS: &[(&str, usize)] = &[
    ("first", 1), ("second", 2), ("third", 3), ("fourth", 4), ("fifth", 5), ("sixth", 6), ("seventh", 7), ("eighth", 8), ("ninth", 9),
    ("tenth", 10), ("1st", 1), ("2nd", 2), ("3rd", 3), ("4th", 4), ("5th", 5), ("top", 1), ("last", 0),
];

/// The most words a search said on the page takes (more is a request for the agent).
const SEARCH_WORDS: usize = 8;

/// Words to a command, or none (it goes to the agent).
pub fn parse(text: &str) -> Option<Said> {
    // Speech-to-text's commas and the sentence's last mark go; a dot inside ("amazon.com") stays.
    let mut t: String = text.to_lowercase().trim().trim_end_matches(['.', '!', '?', ',']).replace(',', "");
    for lead in ["please ", "can you ", "could you ", "now ", "okay ", "ok "] {
        if let Some(rest) = t.strip_prefix(lead) {
            t = rest.to_owned();
        }
    }
    let t = t.trim().trim_end_matches(" please").trim().to_owned();
    match t.as_str() {
        "down" | "scroll down" | "scroll" | "page down" | "scroll down a bit" | "keep scrolling" | "more" => return Some(Said::Scroll("down")),
        "up" | "scroll up" | "page up" | "scroll up a bit" => return Some(Said::Scroll("up")),
        "go to the top" | "scroll to the top" | "top of the page" | "back to the top" => return Some(Said::Scroll("top")),
        "go to the bottom" | "scroll to the bottom" | "bottom of the page" => return Some(Said::Scroll("bottom")),
        "back" | "go back" | "previous page" | "back a page" => return Some(Said::Back),
        "reload" | "refresh" | "reload the page" | "refresh the page" => return Some(Said::Reload),
        "close" | "close it" | "close the browser" | "close the page" | "close this" | "done" | "i'm done" | "im done" | "exit" => return Some(Said::Close),
        _ => {}
    }
    // "go to amazon.com", "open wikipedia.org"
    for lead in ["go to ", "open ", "visit ", "load ", "navigate to "] {
        if let Some(rest) = t.strip_prefix(lead) {
            let r = rest.trim().replace(" dot ", ".");
            if r.contains('.') && !r.contains(' ') {
                return Some(Said::Open(r));
            }
        }
    }
    // One short search, not a request with steps ("search DuckDuckGo for reviews, then read the top three"): that's the
    // agent's.
    let steps = [" then ", " and tell ", " and read ", " and give ", " and summari", " and open ", " in the r1 browser", " in the browser"];
    for lead in ["search for ", "search ", "look up ", "find "] {
        if let Some(rest) = t.strip_prefix(lead) {
            let q = rest.trim();
            if !q.is_empty() && q.split_whitespace().count() <= SEARCH_WORDS && !steps.iter().any(|s| t.contains(s)) {
                return Some(Said::Search(q.to_owned()));
            }
        }
    }
    // "open the second one", "click the first result", "the last link"
    let words: Vec<&str> = t.split_whitespace().collect();
    let body: Vec<&str> = words.iter().copied().skip_while(|w| matches!(*w, "click" | "tap" | "press" | "open" | "select" | "choose" | "pick" | "hit" | "on" | "go" | "to")).collect();
    let body: Vec<&str> = body.into_iter().skip_while(|w| *w == "the").collect();
    if let Some(first) = body.first()
        && let Some((_, n)) = ORDINALS.iter().find(|(w, _)| w == first)
        && body.len() <= 3
    {
        let kind = match body.get(1).copied() {
            Some("link") => "link",
            Some("button") => "button",
            None | Some("one" | "result" | "story" | "article" | "video" | "item" | "post" | "option") => "result",
            _ => return None,
        };
        return Some(Said::Nth(*n, kind));
    }
    // "click sign in", "tap the login button", "open settings"
    let verb = words.first().copied().unwrap_or_default();
    if matches!(verb, "click" | "tap" | "press" | "open" | "select" | "choose" | "hit") && body.len() <= 8 && !body.is_empty() {
        let mut target: Vec<&str> = body.clone();
        if matches!(target.last().copied(), Some("button" | "link" | "tab")) && target.len() > 1 {
            target.pop();
        }
        return Some(Said::Click(target.join(" ")));
    }
    None
}

/// One element the page offered (`browser_read`'s lines: `role "name" [ref]`).
#[derive(Debug, Clone)]
pub struct El {
    pub role: String,
    pub name: String,
    pub r#ref: String,
}

pub fn elements(lines: &str) -> Vec<El> {
    lines
        .lines()
        .filter_map(|l| {
            let r#ref = l.rsplit_once('[')?.1.strip_suffix(']')?.to_owned();
            let role = l.split_whitespace().next()?.to_owned();
            let name = l.split_once('"').and_then(|(_, rest)| rest.split_once('"')).map(|(n, _)| n.to_owned()).unwrap_or_default();
            Some(El { role, name, r#ref })
        })
        .collect()
}

/// The nth of a kind on the page.
pub fn nth(els: &[El], n: usize, kind: &str) -> Option<El> {
    let of: Vec<&El> = els
        .iter()
        .filter(|e| match kind {
            "link" => e.role == "link",
            "button" => e.role == "button",
            // A result: a link with a title (not "new" or "login", nor "49 minutes ago", "19 comments", "47 points").
            _ => e.role == "link" && title_like(&e.name),
        })
        .collect();
    let i = if n == 0 { of.len().checked_sub(1)? } else { n - 1 };
    of.get(i).map(|e| (*e).clone())
}

/// A story's or a result's title: four words or 20 letters, and not a count or a time ("49 minutes ago").
fn title_like(name: &str) -> bool {
    let words: Vec<&str> = name.split_whitespace().collect();
    let starts_with_number = words.first().is_some_and(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()));
    (words.len() >= 4 || name.chars().count() >= 20) && !starts_with_number
}

/// How well an element's name fits what you said, by its words alone (1 the same, 0 nothing in common).
pub fn word_fit(target: &str, name: &str) -> f32 {
    let (t, n) = (target.trim().to_lowercase(), name.trim().to_lowercase());
    if t.is_empty() || n.is_empty() {
        return 0.0;
    }
    if t == n {
        return 1.0;
    }
    if n.starts_with(&t) || (t.len() >= 3 && n.contains(&t)) {
        return 0.9 - (n.len().saturating_sub(t.len()) as f32 / 200.0).min(0.2);
    }
    // Words, not what's between them: "Side of Guacamole,$3.85, Increment" has "guacamole" in it.
    let words = |s: &str| -> HashSet<String> { s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect() };
    let (tw, nw) = (words(&t), words(&n));
    let both = tw.intersection(&nw).count();
    if both == 0 {
        return 0.0;
    }
    // Every word asked for is there, in any order ("Guacamole side"): a fit, the less so the more else the name says.
    // Scored against all the name's words it fell short (2026-10-13: app_find scrolled past it eight times).
    if both == tw.len() && tw.iter().any(|w| w.len() > 2) {
        return (0.8 - nw.len().saturating_sub(tw.len()) as f32 * 0.02).max(0.65);
    }
    0.75 * both as f32 / tw.union(&nw).count() as f32 + 0.1
}

/// What you said, done in the browser if System One can (Some: its words for the R1), else None for the agent.
pub async fn command(hub: &Hub, text: &str) -> Result<Option<String>, String> {
    let Some(said) = parse(text) else { return Ok(None) };
    let go = |op: &'static str, args: Value| async move { call(hub, "", "", op, args).await };
    let named = |v: &Value| v["text"].as_str().unwrap_or_default().to_owned();
    Ok(Some(match said {
        Said::Scroll(d) => {
            go("scroll", json!({ "direction": d })).await?;
            format!("scrolled {d}")
        }
        Said::Back => {
            go("back", json!({})).await?;
            "went back".into()
        }
        Said::Reload => {
            go("reload", json!({})).await?;
            "reloaded".into()
        }
        Said::Close => {
            go("close", json!({})).await?;
            "closed the browser".into()
        }
        Said::Open(u) => {
            let url = allowed_url(&u)?;
            go("open", json!({ "url": url })).await?;
            format!("opened {}", u)
        }
        Said::Search(q) => {
            let page = go("read", json!({ "filter": "interactive" })).await?;
            let els = elements(&named(&page));
            // The page's search box, or a field named for searching: never just its first field (a sign-in page's
            // username took a search on 2026-10-11). None here: the agent gets the words.
            let field = els
                .iter()
                .find(|e| matches!(e.role.as_str(), "searchbox" | "textbox" | "combobox") && e.name.to_lowercase().contains("search"))
                .or_else(|| els.iter().find(|e| e.role == "searchbox"));
            let Some(f) = field else { return Ok(None) };
            go("type", json!({ "ref": f.r#ref, "text": q, "submit": true })).await?;
            format!("searched for {q}")
        }
        Said::Nth(n, kind) => {
            let page = go("read", json!({ "filter": "interactive" })).await?;
            let Some(e) = nth(&elements(&named(&page)), n, kind) else { return Ok(None) };
            go("click", json!({ "ref": e.r#ref })).await?;
            format!("opened “{}”", e.name.chars().take(60).collect::<String>())
        }
        Said::Click(target) => {
            let page = go("read", json!({ "filter": "interactive" })).await?;
            let els = elements(&named(&page));
            let Some(e) = best_match(hub, &target, &els) else { return Ok(None) };
            go("click", json!({ "ref": e.r#ref })).await?;
            format!("clicked “{}”", e.name.chars().take(60).collect::<String>())
        }
    }))
}

/// The element that best fits some words (System One): by its name's words first, then by meaning with the on-device
/// embedder (only when the words don't settle it). None when nothing fits well enough.
pub fn best_match(hub: &Hub, target: &str, els: &[El]) -> Option<El> {
    let els: Vec<&El> = els.iter().filter(|e| !e.name.is_empty()).collect();
    let mut best: Option<(f32, &El)> = els.iter().map(|e| (word_fit(target, &e.name), *e)).max_by(|a, b| a.0.total_cmp(&b.0));
    if best.is_none_or(|(s, _)| s < 0.6) {
        let names: Vec<&str> = els.iter().take(MEANING_MAX).map(|e| e.name.as_str()).collect();
        if let Some(scores) = hub.system1.closeness(target, &names)
            && let Some((s, e)) = scores.into_iter().zip(els.iter()).max_by(|a, b| a.0.total_cmp(&b.0))
            && s >= MEANING_MIN
            && best.is_none_or(|(b, _)| s > b)
        {
            best = Some((s, *e));
        }
    }
    let (score, e) = best.filter(|(s, _)| *s >= MEANING_MIN)?;
    tracing::info!(target = %target, chose = %e.name, score, "browser: system one");
    Some(e.clone())
}

/// An agent's step that names its element in words (`target`) instead of a ref: System One finds it on the page.
/// Typing prefers fields; a list prefers lists.
pub async fn resolve(hub: &Hub, agent: &str, chat: &str, op: &str, target: &str) -> Option<String> {
    let page = call(hub, agent, chat, "read", json!({ "filter": "interactive" })).await.ok()?;
    let all = elements(page["text"].as_str().unwrap_or_default());
    let want: &[&str] = match op {
        "type" => &["searchbox", "textbox", "combobox", "password"],
        "select" => &["combobox", "select", "listbox"],
        _ => &[],
    };
    let fit: Vec<El> = if want.is_empty() { all.clone() } else { all.iter().filter(|e| want.contains(&e.role.as_str())).cloned().collect() };
    if let Some(e) = best_match(hub, target, &fit) {
        return Some(e.r#ref);
    }
    // Nothing named like the words: for typing, the page's only searchbox (when the words say search), else its only
    // text box; for anything else, its only one of the kind.
    let only = |roles: &[&str]| -> Option<El> {
        let of: Vec<&El> = fit.iter().filter(|e| roles.contains(&e.role.as_str())).collect();
        (of.len() == 1).then(|| of[0].clone())
    };
    let pick = match op {
        "type" if target.to_lowercase().contains("search") => only(&["searchbox"]).or_else(|| only(&["textbox", "searchbox"])),
        "type" => only(&["textbox", "searchbox"]),
        _ if !want.is_empty() => (fit.len() == 1).then(|| fit[0].clone()),
        _ => None,
    };
    pick.map(|e| e.r#ref)
}

/// At most this many of a page's elements are compared by meaning (each is an embedding: a few ms on the R1).
const MEANING_MAX: usize = 40;
/// How close in meaning an element must be to count.
const MEANING_MIN: f32 = 0.6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_you_say_in_the_browser() {
        assert_eq!(parse("Scroll down."), Some(Said::Scroll("down")));
        assert_eq!(parse("please go back"), Some(Said::Back));
        assert_eq!(parse("go to amazon.com"), Some(Said::Open("amazon.com".into())));
        assert_eq!(parse("search for running shoes"), Some(Said::Search("running shoes".into())));
        // A request with steps is the agent's.
        assert_eq!(parse("search DuckDuckGo in the R1 browser for rabbit r1 reviews, then read the top three results"), None);
        assert_eq!(parse("look up the weather and tell me if I need a coat"), None);
        assert_eq!(parse("open the second one"), Some(Said::Nth(2, "result")));
        assert_eq!(parse("click the last link"), Some(Said::Nth(0, "link")));
        assert_eq!(parse("click sign in"), Some(Said::Click("sign in".into())));
        assert_eq!(parse("tap the login button"), Some(Said::Click("login".into())));
        assert_eq!(parse("what's the cheapest one here"), None);
        assert_eq!(parse("summarise this page"), None);
    }

    #[test]
    fn a_page_s_elements() {
        let lines = "link \"new\" [r1]\nlink \"Show HN: A meta-search for fashion\" [r2]\nlink \"49 minutes ago\" [r5]\nlink \"19 comments\" [r6]\nbutton \"Sign in\" [r3]\nlink \"The tilde in your PATH may not be your HOME\" [r4]";
        let els = elements(lines);
        assert_eq!(els.len(), 6);
        assert_eq!(nth(&els, 1, "result").map(|e| e.r#ref), Some("r2".into()));
        assert_eq!(nth(&els, 2, "result").map(|e| e.r#ref), Some("r4".into()));
        assert_eq!(nth(&els, 0, "link").map(|e| e.r#ref), Some("r4".into()));
        assert!(word_fit("sign in", "Sign in") > 0.99);
        assert!(word_fit("login", "Sign in") < 0.1);
        assert!(word_fit("tilde", "The tilde in your PATH may not be your HOME") > 0.6);
        assert!(word_fit("Guacamole side", "Side of Guacamole,$3.85, Increment") >= 0.6);
        assert!(word_fit("medium coke", "Medium Coke®") > word_fit("medium coke", "Medium Coke Zero® Sugar"));
        assert!(word_fit("extra cheese", "Cheese,110 Cal.") < 0.6);
    }

    #[test]
    fn only_public_web_pages() {
        assert_eq!(allowed_url("example.com").unwrap(), "https://example.com");
        assert!(allowed_url("https://www.amazon.com/gp/cart").is_ok());
        for bad in [
            "http://127.0.0.1:7171/api/state", "http://localhost/x", "http://192.168.1.4", "http://10.0.0.2", "http://172.20.1.1",
            "http://[::1]/", "http://169.254.1.1", "file:///data/r1os", "intent://x", "javascript:alert(1)", "http://r1.local/", "http://router/",
            "http://100.100.1.1/", "http://user@127.0.0.1/",
        ] {
            assert!(allowed_url(bad).is_err(), "{bad} should be refused");
        }
    }
}
