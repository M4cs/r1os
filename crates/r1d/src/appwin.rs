//! Agents in Android apps (ADR-0061, T-113; Max 2026-10-12: agents that "find you rides on Uber/Lyft. Order you
//! doordash. Search things on spotify"): an app you installed, on a display of its own that the host owns, which an
//! agent reads and taps through while you watch its preview in the chat, and hands to you when it needs you.
//!
//! - **Where:** the host (`AgentApps.kt`) owns an R1-sized virtual display, opens the app on it, taps and types into it,
//!   and reads it with its own accessibility service, on only while an app is open. r1d asks it to do something
//!   (`app_window {id, op, agent, chat, args}`), and the host answers at `POST /api/apps/window/result`; a tool waits
//!   for that ([`call`]). The same shape as the browser (crate::browser).
//! - **Its tools:** `app_open`, `app_read` (the screen as lines with refs like `[a12]`), `app_screenshot`,
//!   `app_click`, `app_type`, `app_scroll`, `app_back`, `app_wait`, `app_ask_user` and `app_close`. Every step's
//!   answer has the screen's picture along with its lines (Max, Q62).
//! - **What asks you** (Q55, Q57):
//!   - **an app agents haven't used:** "let agents use Uber?", once; yes is kept for every agent ([`consented`], core's `agent_apps`);
//!   - **acting:** looking runs; the first tap, typing or key in a turn asks, by the `android` level (ask, allow,
//!     never), and "allow" lets the rest of that turn go ahead ([`end_turn`] clears it);
//!   - **a final step** (placing an order, requesting a ride, paying, subscribing, sending): every time, at every
//!     level, naming the app and what its screen says ([`commit`]).
//! - **Yours alone:** passwords, card numbers and one-time codes. Their fields refuse the agent's typing, and screens
//!   Android marks secure come out black for it; the agent hands you the app (`app_ask_user`). Android's permission
//!   prompts too: the agent may read them, never answer them ([`PROMPT`]).
//! - **Already open, or yours to take** (T-139): an app open on the R1's own screen moves into the window as it is, and
//!   back when you say "let me use it" (`app_close {for_user: true}`, or the full window's button).

use crate::hub::{Event, Hub, Source};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

/// How long a tool waits for the host (an app can take a while to start).
const WAIT: Duration = Duration::from_secs(30);
/// How long `app_ask_user` waits for you.
const ASK_WAIT: Duration = Duration::from_secs(5 * 60);

type Answer = tokio::sync::oneshot::Sender<Result<Value, String>>;

#[derive(Default)]
pub struct AppWindow {
    pending: Mutex<HashMap<String, Answer>>,
    /// Chats allowed to act in an app for the rest of their turn.
    granted: Mutex<HashSet<String>>,
    /// One step at a time, in the order they came (crate::mcp's `call`): an agent sends several at once, and on one
    /// screen they interleaved (typing before the tap it followed, 2026-10-12's Uber Eats order).
    pub turn: tokio::sync::Mutex<()>,
    /// Test chats (`POST /harness/dry_run`, tools/eval-control.py): nothing that adds to a cart or finishes anything
    /// is pressed in them, in the app window or the browser ([`dry_refusal`]).
    dry: Mutex<HashSet<String>>,
    /// The screen each chat last saw (its lines), and its last act on it with how many times in a row: refs go with the
    /// names it saw ([`AppWindow::name_of`]), and an act repeated on an unchanged screen is caught ([`AppWindow::again`]).
    seen: Mutex<HashMap<String, Seen>>,
}

#[derive(Default)]
struct Seen {
    lines: String,
    act: String,
    times: u32,
    /// The last answer said nothing changed on the screen (the host's word for a tap that did nothing).
    nothing: bool,
}

impl AppWindow {
    /// The host's answer to a request.
    pub fn answer(&self, id: &str, result: Result<Value, String>) -> bool {
        match self.pending.lock().unwrap().remove(id) {
            Some(tx) => tx.send(result).is_ok(),
            None => false,
        }
    }

    /// A turn ended: what it was allowed to do in an app goes with it, and the screen it last saw.
    pub fn end_turn(&self, chat: &str) {
        self.granted.lock().unwrap().remove(chat);
        self.forget(chat);
    }
}

/// The chat's turn was stopped (crate::stops): it may no longer act, and an ask of it goes.
pub fn stopped(hub: &Hub, chat: &str) {
    hub.app_window.end_turn(chat);
    hub.emit(Event::AppWindow { id: String::new(), op: "stopped".into(), agent: String::new(), chat: chat.to_owned(), args: Value::Null }, Source::Api);
}

/// Asks the host to do `op` in the app window and waits for its answer.
pub async fn call(hub: &Hub, agent: &str, chat: &str, op: &str, args: Value) -> Result<Value, String> {
    let id = crate::core::random_hex(6);
    let (tx, rx) = tokio::sync::oneshot::channel();
    hub.app_window.pending.lock().unwrap().insert(id.clone(), tx);
    hub.emit(Event::AppWindow { id: id.clone(), op: op.to_owned(), agent: agent.to_owned(), chat: chat.to_owned(), args }, Source::Api);
    let wait = if op == "ask_user" { ASK_WAIT } else { WAIT };
    let started = std::time::Instant::now();
    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(r)) => {
            // How long each step takes, for `r1h perf` (an ask waits on you, not the app), and where an act's time went.
            if op != "ask_user" && r.is_ok() {
                hub.perf.mark(&format!("app.{op}"), started.elapsed());
            }
            if let Ok(v) = &r
                && let Some(t) = v["took"].as_object()
            {
                for part in ["settle", "read"] {
                    if let Some(ms) = t.get(part).and_then(Value::as_u64) {
                        hub.perf.mark(&format!("app.{op}.{part}"), Duration::from_millis(ms));
                    }
                }
            }
            r
        }
        _ => {
            hub.app_window.pending.lock().unwrap().remove(&id);
            Err(if op == "ask_user" { "The user didn't answer in 5 minutes.".into() } else { "The R1's app window didn't answer (is the R1's app running?).".into() })
        }
    }
}

/// A link an app may open in the window: an app's own (spotify:…, uber://…) or a web address one claims; never an
/// intent, a file, content, script, or the R1 itself.
pub fn allowed_link(link: &str) -> Result<(), String> {
    let l = link.trim();
    let scheme = l.split_once(':').map(|(s, _)| s.to_ascii_lowercase()).ok_or("that isn't a link")?;
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return Err("that isn't a link".into());
    }
    if matches!(scheme.as_str(), "intent" | "file" | "content" | "javascript" | "data" | "about" | "android-app" | "jar") {
        return Err(format!("not a {scheme}: link: an app's own link (spotify:…, uber://…) or a web address"));
    }
    if matches!(scheme.as_str(), "http" | "https") {
        crate::browser::allowed_url(l)?;
    }
    Ok(())
}

/// An element named in words (`target`) instead of a ref: System One picks it from the screen as it is now (a plain
/// read, then crate::browser's matcher: words first, then meaning, on the R1), as in the browser. Typing prefers
/// fields. None: the host's own word match tries.
pub async fn resolve(hub: &Hub, agent: &str, chat: &str, op: &str, target: &str) -> Option<String> {
    let screen = call(hub, agent, chat, "read", json!({ "plain": true })).await.ok()?;
    let all = crate::browser::elements(screen["text"].as_str().unwrap_or_default());
    let fit: Vec<crate::browser::El> = if op == "type" { all.into_iter().filter(|e| matches!(e.role.as_str(), "textbox" | "password")).collect() } else { all };
    pick(hub, target, &fit).map(|e| e.r#ref)
}

/// The element an agent's words name: by the words first, by meaning only when very close. An agent copies words off
/// the screen, so a loose meaning match is more likely wrong than right ("See more" found "More options", 2026-10-12).
pub fn pick(hub: &Hub, target: &str, els: &[crate::browser::El]) -> Option<crate::browser::El> {
    let named: Vec<&crate::browser::El> = els.iter().filter(|e| !e.name.is_empty()).collect();
    if let Some((s, e)) = named.iter().map(|e| (crate::browser::word_fit(target, &e.name), *e)).max_by(|a, b| a.0.total_cmp(&b.0))
        && s >= 0.6
    {
        return Some(e.clone());
    }
    // By meaning, those sharing a word with it first: only so many are weighed, and a long item sheet's had put the
    // one meant past them.
    let mut by_words = named.clone();
    by_words.sort_by_key(|e| crate::browser::word_fit(target, &e.name) == 0.0);
    let by_words: Vec<&crate::browser::El> = by_words.into_iter().take(MEANING_MAX).collect();
    let names: Vec<&str> = by_words.iter().map(|e| e.name.as_str()).collect();
    let scores = hub.system1.closeness(target, &names)?;
    let (s, e) = scores.into_iter().zip(by_words.iter()).max_by(|a, b| a.0.total_cmp(&b.0))?;
    (s >= MEANING_CLOSE).then(|| (*e).clone())
}

/// How close in meaning an agent's words must be to an element's, with no words in common.
const MEANING_CLOSE: f32 = 0.85;
/// How many of a screen's elements are weighed by meaning.
const MEANING_MAX: usize = 40;

/// Whether you've let agents use an app.
pub fn consented(hub: &Hub, package: &str) -> bool {
    hub.core.agent_apps().iter().any(|p| p == package)
}

/// The first time agents use an app: the card asks you, every time until you say yes (no level skips it).
pub async fn may_use(hub: &Hub, agent: &str, chat: &str, package: &str, name: &str) -> Result<(), String> {
    if consented(hub, package) {
        return Ok(());
    }
    let mut ask = crate::approvals::Ask::new("android", format!("use {name} for you"), terms_note(package)).every_time().chat(chat);
    if let Some(w) = hub.mail.woken(chat) {
        let from = hub.core.agent(&w.by).map(|a| a.name).unwrap_or_else(|| "another agent".into());
        ask = ask.why(&format!("{from} asked it to"));
    }
    hub.approvals.gate(hub, agent, ask).await?;
    let mut all = hub.core.agent_apps();
    if !all.iter().any(|p| p == package) {
        all.push(package.to_owned());
        hub.core.set_agent_apps(&all).map_err(|e| format!("couldn't keep that: {e}"))?;
    }
    Ok(())
}

/// What the card adds for an app whose terms forbid automated use (research §6): you decide with that in front of you.
pub fn terms_note(package: &str) -> String {
    match package {
        "com.dd.doordash" => "DoorDash's terms forbid using it by automated means; it's your account".into(),
        "me.lyft.android" => "Lyft's terms forbid using it with any automatic process; it's your account".into(),
        _ => "agents tap and type in it while you watch, and hand it to you for passwords, codes and payments".into(),
    }
}

/// Before the first act in a turn (a tap, typing, a key): the `android` level, or the card.
pub async fn may_act(hub: &Hub, agent: &str, chat: &str, what: &str, app: &str) -> Result<(), String> {
    if hub.app_window.granted.lock().unwrap().contains(chat) {
        return Ok(());
    }
    let mut ask = crate::approvals::Ask::new("android", format!("use {app}: {what}"), String::new());
    if let Some(w) = hub.mail.woken(chat) {
        let from = hub.core.agent(&w.by).map(|a| a.name).unwrap_or_else(|| "another agent".into());
        ask = ask.why(&format!("{from} asked it to"));
    }
    hub.approvals.gate(hub, agent, ask.chat(chat)).await?;
    hub.app_window.granted.lock().unwrap().insert(chat.to_owned());
    Ok(())
}

/// A final step (Q57): the card every time, whatever the level, with what the screen says: the order read from it (its
/// items, total, tip, where it goes, how it's paid and when: [`order_of`]), and its words as they are. Refused in a turn
/// another agent's message started (ADR-0012).
pub async fn commit(hub: &Hub, agent: &str, chat: &str, app: &str, step: &str, screen: &str) -> Result<(), String> {
    if hub.mail.woken(chat).is_some() {
        return Err("Not in this turn: another agent's message started it, not the user, and this is a final step (an order, a ride, a payment, a message). Say what you'd do; the user can ask you directly.".into());
    }
    hub.approvals.gate(hub, agent, commit_ask(hub, agent, chat, app, step, screen).await).await
}

/// A final step's card: its step, and the order on the screen as it is now (the whole checkout, what's below the fold
/// too: a plain read).
pub async fn commit_ask<'a>(hub: &Hub, agent: &str, chat: &str, app: &str, step: &str, screen: &str) -> crate::approvals::Ask<'a> {
    let read = call(hub, agent, chat, "read", json!({ "plain": true })).await.ok();
    let order = read.as_ref().and_then(|v| v["text"].as_str()).map(order_of).filter(Order::any);
    let ask = crate::approvals::Ask::new("android", format!("{step} in {app}"), screen.chars().take(400).collect::<String>()).every_time().chat(chat);
    match order {
        Some(o) => ask.order(o.json()),
        None => ask,
    }
}

/// A button whose words say it finishes something you'd pay for or send: it asks every time (Q57). The agent says so
/// too (`final: true`), but the words decide even when it doesn't.
pub fn is_final(label: &str) -> bool {
    let l = label.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    // Whole words in a row, so "Sending options" or "Requests" don't count, starting within the first [within] words.
    let has = |phrase: &[&str], within: usize| words.windows(phrase.len()).take(within + 1).any(|w| w == phrase);
    const PHRASES: &[&[&str]] = &[
        &["place", "order"], &["place", "your", "order"], &["order", "now"], &["submit", "order"], &["complete", "order"],
        &["complete", "purchase"], &["checkout"], &["check", "out"], &["pay"], &["buy"], &["purchase"], &["request"], &["book"],
        &["subscribe"], &["start", "subscription"], &["start", "trial"], &["send"], &["donate"], &["confirm", "ride"],
        &["confirm", "order"], &["confirm", "and", "pay"], &["confirm", "booking"], &["confirm", "payment"],
    ];
    // Near the start, as on a button ("Place order • $25.12", "Go to checkout"): a store's card that mentions "Buy 1,
    // Get 1" far into its words isn't one, and refusing it stopped a test from opening a restaurant (2026-10-13).
    // "Send as a gift" on a cart is a choice, not sending anything: it put its words on a checkout card (2026-10-13).
    let gift = words.iter().take(FINAL_WITHIN + 4).any(|w| w.starts_with("gift"));
    words == ["confirm"] || PHRASES.iter().any(|p| !(gift && p.len() == 1 && p[0] == "send") && has(p, FINAL_WITHIN))
}

/// How many words into a label a final step's words may start.
const FINAL_WITHIN: usize = 3;

/// What a checkout says it'll do (Q57), for a final step's card: read from the screen, never taken from the agent.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Order {
    /// What's in it, each with its price when the screen puts one by it.
    pub items: Vec<(String, Option<String>)>,
    pub total: Option<String>,
    pub tip: Option<String>,
    /// Where it goes: a delivery address, a ride's drop-off, or where it's picked up.
    pub to: Option<String>,
    /// How it's paid: "Visa ••1234".
    pub pay: Option<String>,
    /// When: "10–20 min", "ASAP", "Arrives by 6:40 PM".
    pub when: Option<String>,
}

impl Order {
    /// Anything worth a row on the card.
    pub fn any(&self) -> bool {
        self.total.is_some() || !self.items.is_empty() || self.to.is_some()
    }

    pub fn json(&self) -> Value {
        json!({
            "items": self.items.iter().map(|(n, p)| json!({ "name": n, "price": p })).collect::<Vec<_>>(),
            "total": self.total, "tip": self.tip, "to": self.to, "pay": self.pay, "when": self.when,
        })
    }
}

/// A checkout's order, from a plain read's lines (`text "Total"`, `button "Place order • $25.12" [a40]`). Prices pair
/// with the words they're on or come after; what's called a total, a tip or a fee is that; the rest are its items.
pub fn order_of(lines: &str) -> Order {
    let mut o = Order::default();
    let mut label: Option<String> = None;
    let mut final_price = None;
    for line in lines.lines() {
        let line = line.trim();
        if line.starts_with('(') {
            continue;
        }
        let role = line.split_whitespace().next().unwrap_or_default();
        let Some(words) = line.split_once('"').and_then(|(_, rest)| rest.rsplit_once('"')).map(|(w, _)| w.trim().to_owned()) else { continue };
        if words.is_empty() {
            continue;
        }
        if o.to.is_none() {
            o.to = destination(&words);
        }
        if o.pay.is_none() {
            o.pay = payment(&words);
        }
        if o.when.is_none() {
            o.when = timing(&words);
        }
        let price = price_in(&words).map(|(a, b)| words[a..b].replace(' ', ""));
        // The final step's own button ("Place order • $25.12"): its price is the total when nothing else says.
        if role == "button" && is_final(&words) {
            final_price = final_price.or(price);
            label = None;
            continue;
        }
        let Some(price) = price else {
            // An item whose line a read cut short, its price with it ("Oven-Roasted Turkey (6''), Quantity: 1, Bread: …
            // Cheese: Provolon", Uber Eats' cart, 2026-10-13): still in the order, by its quantity, once.
            if words.to_lowercase().contains("quantity:") && o.items.len() < 12
                && let Some(name) = item_name(&words)
                && !o.items.iter().any(|(n, _)| *n == name)
            {
                o.items.push((name, None));
                label = None;
                continue;
            }
            label = Some(words);
            continue;
        };
        let (a, b) = price_in(&words).unwrap_or_default();
        let own = format!("{} {}", &words[..a], &words[b..]).trim_matches(|c: char| c.is_whitespace() || "•·,:-".contains(c)).to_owned();
        let name = if own.is_empty() { label.take() } else { Some(own) };
        label = None;
        let Some(name) = name else { continue };
        let l = name.to_lowercase();
        if (l.contains("total") && !l.contains("subtotal")) || l.contains("amount due") || l.starts_with("you pay") {
            // "Total: before promotion: $16.61, now: $16.25": what you'd pay is the last price on it.
            o.total = Some(last_price(&words).unwrap_or(price));
        } else if l.contains("tip") {
            o.tip = Some(price);
        } else if !FEES.iter().any(|f| l.contains(f)) && !l.starts_with("add ") && o.items.len() < 12 && let Some(name) = item_name(&name) {
            // The same item read again in full replaces its cut-short line's empty price.
            match o.items.iter_mut().find(|(n, p)| *n == name && p.is_none()) {
                Some(it) => it.1 = Some(price),
                None => o.items.push((name, Some(price))),
            }
        }
    }
    if o.total.is_none() {
        o.total = final_price;
    }
    // No total, nowhere it goes and no way it's paid: not a checkout. An item's sheet lists its extras' prices
    // ("Bacon $1.49"), which a card showed as the order (2026-10-13); the screen's words say more.
    if o.total.is_none() && o.to.is_none() && o.pay.is_none() {
        o.items.clear();
    }
    o
}

/// An item's name from its line, short enough for a card: Uber Eats reads a cart's item as "Oven-Roasted Turkey (6''),
/// Quantity: 1, Bread: Artisan Italian, … Price: $9.19" (2026-10-13), which was too long and left the card with no
/// items. Its choices go; a quantity over one leads ("2× Coke"). None: still no name.
fn item_name(line: &str) -> Option<String> {
    let mut name = line.trim();
    for tail in ["price", "price:"] {
        if name.to_lowercase().ends_with(tail) {
            name = name[..name.len() - tail.len()].trim_end_matches(|c: char| c.is_whitespace() || ".,:;•·-".contains(c));
        }
    }
    let mut count = None;
    // On the line itself, not a lowercased copy: its byte places can differ.
    if let Some(at) = name.find(", Quantity").or_else(|| name.find(", quantity")) {
        count = name[at + ", Quantity".len()..].trim_start_matches([':', ' ']).split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u32>().ok());
        name = &name[..at];
    } else if name.chars().count() > 60 && let Some(at) = name.find(", ") {
        name = &name[..at];
    }
    let name: String = name.trim().chars().take(80).collect();
    if name.is_empty() {
        return None;
    }
    Some(match count {
        Some(n) if n > 1 => format!("{n}× {name}"),
        _ => name,
    })
}

/// Lines that cost something but aren't what you're buying.
const FEES: &[&str] = &[
    "subtotal", "fee", "tax", "discount", "promo", "saving", "credit", "off your order", "uber one", "membership", "bag",
    "estimated", "surcharge", "deposit", "rewards", "you saved", "service", "delivery", "small order", "regulatory",
];

/// The last price in some words, as they'd show it ("$16.25" of "before promotion: $16.61, now: $16.25").
fn last_price(s: &str) -> Option<String> {
    let mut at = 0;
    let mut last = None;
    while let Some((a, b)) = price_in(&s[at..]) {
        last = Some(s[at + a..at + b].replace(' ', ""));
        at += b;
    }
    last
}

/// Where a price is in some words: `$12.39`, `$ 1,204.50`, `€9`, `£4.20`.
fn price_in(s: &str) -> Option<(usize, usize)> {
    for (i, c) in s.char_indices() {
        if !matches!(c, '$' | '€' | '£') {
            continue;
        }
        let rest = &s[i + c.len_utf8()..];
        let skip = rest.len() - rest.trim_start_matches(' ').len();
        let digits = &rest[skip..];
        let n = digits.find(|d: char| !(d.is_ascii_digit() || d == ',' || d == '.')).unwrap_or(digits.len());
        let num = digits[..n].trim_end_matches(['.', ',']);
        if num.starts_with(|d: char| d.is_ascii_digit()) {
            return Some((i, i + c.len_utf8() + skip + num.len()));
        }
    }
    None
}

/// Where it goes: "Deliver to 25 Main St", "Drop-off: Springfield Airport", "Pickup at Chipotle", or a street address.
fn destination(words: &str) -> Option<String> {
    let l = words.to_lowercase();
    for lead in ["deliver to", "delivering to", "delivery address", "drop-off", "dropoff", "destination", "pickup at", "pick up at", "pick up from", "pickup from"] {
        if let Some(rest) = l.strip_prefix(lead) {
            let rest = &words[words.len() - rest.len()..];
            let rest = rest.trim_matches(|c: char| c.is_whitespace() || ":•·,-".contains(c));
            if !rest.is_empty() {
                return Some(rest.chars().take(80).collect());
            }
        }
    }
    // A street address: a number, then a street's name and kind ("12B Example St", "40 Sample Ave").
    let parts: Vec<&str> = words.split_whitespace().collect();
    let numbered = parts.first().is_some_and(|w| w.starts_with(|c: char| c.is_ascii_digit()) && w.len() <= 6);
    const KINDS: &[&str] = &["st", "street", "ave", "avenue", "rd", "road", "blvd", "boulevard", "dr", "drive", "ln", "lane", "way", "ct", "court", "pl", "place", "pkwy", "parkway", "hwy", "highway", "sq", "square", "ter", "terrace", "cir", "circle"];
    let kind = parts.iter().skip(1).take(5).any(|w| KINDS.contains(&w.trim_end_matches([',', '.']).to_lowercase().as_str()));
    (numbered && kind && words.len() <= 80 && price_in(words).is_none()).then(|| words.to_owned())
}

/// How it's paid: a card's kind and last four ("Visa •••• 1234" → "Visa ••1234"), or a wallet. Either starts the line,
/// and a card's name without its digits counts only as the whole line, so "Discover new dishes" isn't one.
fn payment(words: &str) -> Option<String> {
    let l = words.trim().to_lowercase();
    if l.chars().count() > 40 {
        return None;
    }
    const CARDS: &[(&str, &str)] = &[("visa", "Visa"), ("mastercard", "Mastercard"), ("amex", "Amex"), ("american express", "Amex"), ("discover", "Discover")];
    const WALLETS: &[(&str, &str)] = &[("apple pay", "Apple Pay"), ("google pay", "Google Pay"), ("paypal", "PayPal"), ("venmo", "Venmo"), ("uber cash", "Uber Cash"), ("cash app", "Cash App")];
    // Its last four: four digits on their own, after the dots or "ending in".
    let chars: Vec<char> = words.chars().collect();
    let last4 = (0..chars.len().saturating_sub(3)).rev().find_map(|i| {
        let run: String = chars[i..i + 4].iter().collect();
        let before = if i == 0 { ' ' } else { chars[i - 1] };
        let after = chars.get(i + 4).copied().unwrap_or(' ');
        (run.chars().all(|c| c.is_ascii_digit()) && !before.is_ascii_digit() && !after.is_ascii_digit()).then_some(run)
    });
    if let Some((k, name)) = CARDS.iter().find(|(k, _)| l.starts_with(k)) {
        return match last4 {
            Some(d) => Some(format!("{name} ••{d}")),
            None => (l == *k).then(|| (*name).to_owned()),
        };
    }
    WALLETS.iter().find(|(k, _)| l.starts_with(k)).map(|(_, name)| (*name).to_owned())
}

/// When: "10–20 min", "ASAP", "Arrives by 6:40 PM", "Standard · 25–35 min".
fn timing(words: &str) -> Option<String> {
    let l = words.to_lowercase();
    if l.starts_with("arriv") || l.starts_with("deliver by") || l.starts_with("estimated arrival") || l == "asap" || l.starts_with("asap") {
        return Some(words.chars().take(40).collect());
    }
    // A range of minutes: "10–20 min", "25-35 mins".
    let parts: Vec<&str> = l.split_whitespace().collect();
    for (i, w) in parts.iter().enumerate() {
        if w.starts_with("min") && i > 0 {
            let range = parts[i - 1];
            if range.chars().any(|c| c.is_ascii_digit()) && range.chars().all(|c| c.is_ascii_digit() || "–-".contains(c)) {
                return Some(format!("{range} min"));
            }
        }
    }
    None
}

/// Android's permission prompt in the window (the host says so): the user answers it, not the agent (T-139).
pub const PROMPT: &str = "That's Android asking the user's permission (a permission prompt): theirs to answer, never yours. Hand it to them with app_ask_user (say what the app wants and why), then read the screen again.";

impl AppWindow {
    /// A chat becomes a dry run, or stops being one.
    pub fn set_dry(&self, chat: &str, on: bool) {
        let mut d = self.dry.lock().unwrap();
        if on {
            d.insert(chat.to_owned());
        } else {
            d.remove(chat);
        }
    }

    pub fn is_dry(&self, chat: &str) -> bool {
        self.dry.lock().unwrap().contains(chat)
    }

    /// The screen a step answered with, as this chat saw it.
    pub fn saw(&self, chat: &str, screen: &Value) {
        if let Some(t) = screen["text"].as_str() {
            let mut all = self.seen.lock().unwrap();
            let s = all.entry(chat.to_owned()).or_default();
            s.nothing = t.contains("Nothing changed on the screen");
            // A step that didn't read the screen (an app_do's before its last) leaves the screen last read, whose refs
            // the next steps still name.
            if screen["quick"] == true {
                return;
            }
            let lines = lines_of(t);
            if lines != s.lines {
                s.lines = lines;
                s.times = 0;
            }
        }
    }

    /// What a ref was called on the screen this chat last saw.
    pub fn name_of(&self, chat: &str, r#ref: &str) -> Option<String> {
        let all = self.seen.lock().unwrap();
        let s = all.get(chat)?;
        crate::browser::elements(&s.lines).into_iter().find(|e| e.r#ref == r#ref).map(|e| e.name).filter(|n| !n.is_empty())
    }

    /// This act on the screen the chat last saw: how many times in a row now that it changed nothing at all (a tap
    /// that does something each time, like a quantity's +, counts once).
    pub fn again(&self, chat: &str, act: &str) -> u32 {
        let mut all = self.seen.lock().unwrap();
        let s = all.entry(chat.to_owned()).or_default();
        if s.act == act && s.nothing {
            s.times += 1;
        } else {
            s.act = act.to_owned();
            s.times = 1;
        }
        s.times
    }

    /// A chat's gone: what it saw goes too.
    pub fn forget(&self, chat: &str) {
        self.seen.lock().unwrap().remove(chat);
    }
}

/// A screen's lines without the step's own first line(s) of what it did: only lines with an element in them count.
fn lines_of(text: &str) -> String {
    text.lines().filter(|l| l.ends_with(']')).collect::<Vec<_>>().join("\n")
}

impl AppWindow {
}

/// A button that adds to a cart (or a bag, a basket, an order): in a dry run it isn't pressed either. So is one that
/// starts with update, save, add, confirm or continue and shows a price ("Update item • $9.99"). Not "Done • $14.14":
/// on Uber Eats that closes a choice's own sheet (Subway's bread, toasted) and adds nothing; refusing it stopped a
/// sandwich halfway (2026-10-13).
pub fn adds_to_cart(label: &str) -> bool {
    let l = label.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    // Within its first words, as on a button ("Add 1 to cart • $14.14"), not a store's card that says "added" and
    // "order again" somewhere in its description.
    let early = &words[..words.len().min(6)];
    if early.iter().any(|w| w.starts_with("add")) && early.iter().any(|w| ["cart", "bag", "basket", "order"].contains(w)) {
        return true;
    }
    let first = words.first().copied().unwrap_or_default();
    ["update", "save", "add", "confirm", "continue"].contains(&first) && l.contains('$')
}

/// What a tap by place (x,y) must clear. [buttons]: every button's words on the screen; [near]: those the tap could be
/// on, with room for a web page's lag around each. Outside a test, a final step anywhere on the screen asks on the card,
/// since which button is under the finger can't be told when the words lag. In a test, a final step or an add-to-cart
/// the tap could be on refuses; one elsewhere on the screen doesn't, or no tap by place would get through an item
/// sheet, whose add-to-cart bar never leaves it (2026-10-13).
pub fn by_place(buttons: &[String], near: &[String], dry: bool) -> Option<String> {
    if dry {
        return near.iter().find(|b| is_final(b) || adds_to_cart(b)).map(|b| {
            format!("Not pressed: this is a test run, and that's on or right by “{}”. Tap what you meant by its words or ref, or say what you'd do.", b.chars().take(60).collect::<String>())
        });
    }
    buttons.iter().find(|b| is_final(b)).map(|b| format!("a tap at a place on a screen with “{}” on it", b.chars().take(60).collect::<String>()))
}

/// In a dry run (a test), what a tap on [label] would refuse: adding to a cart, or a final step. None: go ahead.
pub fn dry_refusal(hub: &Hub, chat: &str, label: &str, final_: bool) -> Option<String> {
    (hub.app_window.is_dry(chat) && (final_ || is_final(label) || adds_to_cart(label))).then(|| {
        format!("Not pressed: this is a test run, which stops before “{}”. Say exactly what you'd add or do, with the total the screen shows.", label.chars().take(60).collect::<String>())
    })
}

/// What's been learned about using an app, for the agent opening it (T-144: workflow memory raised agents' success by a
/// quarter to a half in published work): the ways that worked, by the words on screen, never by refs.
pub fn tips(package: &str) -> Option<&'static str> {
    Some(match package {
        "com.ubercab.eats" => "Tips for Uber Eats: \
            Search tab, then type into \"Search Uber Eats\" with submit; results take a few seconds (app_wait if the \
            answer still shows suggestions). A restaurant's store page, its menu and item sheets are web pages: their words \
            lag the picture, so go by the picture and tap at x,y when they disagree. In a store, \"Search items\" (the \
            magnifier at the top) finds a menu item fastest; or tap its menu tab (\"Extra Value Meals\", \"Sandwiches\"). \
            An item sheet lists its groups (\"Select Option\", \"Select Drink\", bread, toppings) each marked Required or \
            optional; a choice can open its own page (a meal's drink, a protein's portion, a sandwich's bread and toasting) \
            ending in \"Done • $price\", which closes that page and adds nothing: press it and carry on with the next \
            group. When every required group is chosen, the item sheet's bottom button reads \"Add 1 to cart • $price\": \
            check each choice on the picture before it. Choose a sheet's options in one app_do (each by its words or \
            ref), then look at the picture: the steps before the last don't read the screen, so it's much faster than one \
            call each. A store that's closed still shows its menu.",
        "com.spotify.music" => "Tips for Spotify: \
            Search tab, then type into \"What do you want to listen to?\" with submit; tap a result's row to open it. An \
            artist's page lists Popular songs; an album's page lists its tracks (app_find scrolls to one). Liked Songs is in \
            Your Library. Only play when asked: a tap anywhere on a song's row (its number, picture or title) plays it, as \
            does a play button. To learn a row's title, read the screen, its picture, or app_find; never tap the row. A \
            row that reads \"media slot\" has no words: the picture has its title. \"You're offline\" at the bottom: the \
            words for songs and albums may be missing, so go by the picture.",
        _ => return None,
    })
}

/// How many times `app_find` scrolls looking for its target.
pub const FIND_SCROLLS: usize = 8;

/// A screen's lines without the first (what the step did): to tell whether a scroll changed anything.
/// Scrolls in a row that change nothing before `app_find` decides it's at the end (a collapsing header can take one).
pub const FIND_STILL: usize = 2;

pub fn lines(screen: &Value) -> String {
    screen["text"].as_str().unwrap_or_default().lines().skip(1).collect::<Vec<_>>().join("\n")
}

/// The tools that use the app window, which take its turn one at a time.
pub fn is_tool(name: &str) -> bool {
    matches!(
        name,
        "app_open" | "app_read" | "app_screenshot" | "app_click" | "app_type" | "app_scroll" | "app_back" | "app_wait" | "app_ask_user" | "app_close" | "app_do"
            | "app_find"
    )
}

/// Ops that act in the app, which ask first (scrolling and going back only look around, as in the browser).
pub fn acts(op: &str) -> bool {
    matches!(op, "click" | "type" | "key")
}

/// What an op's answer says, for the agent: the host's own words, or JSON.
pub fn words(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(m) if m.contains_key("text") => {
            let mut out = String::new();
            if let Some(a) = m.get("app").and_then(Value::as_str) {
                out.push_str(&format!("[{a}]\n"));
            }
            out.push_str(m["text"].as_str().unwrap_or_default());
            out
        }
        other => other.to_string(),
    }
}

/// The MCP tool's answer for an op: text, or (a screenshot) an image.
pub fn reply(op: &str, r: Result<Value, String>) -> Value {
    match r {
        Ok(v) if op == "screenshot" => match v["image"].as_str() {
            Some(b64) => json!({ "content": [
                { "type": "image", "data": b64, "mimeType": "image/jpeg" },
                { "type": "text", "text": format!("[{}] {}x{}{}", v["app"].as_str().unwrap_or_default(), v["width"], v["height"],
                    if v["secure"] == true { " (a secure screen: black for you; ask the user to take over)" } else { "" }) }
            ], "isError": false }),
            None => json!({ "content": [{ "type": "text", "text": "No screenshot: no app is open." }], "isError": true }),
        },
        // Every step's answer carries the screen's picture with its lines (Max, Q62).
        Ok(v) => {
            let mut content = vec![json!({ "type": "text", "text": words(&v) })];
            if let Some(b64) = v["image"].as_str() {
                content.push(json!({ "type": "image", "data": b64, "mimeType": "image/jpeg" }));
            }
            json!({ "content": content, "isError": false })
        }
        Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_steps_by_their_words() {
        for yes in ["Place order", "Request UberX", "Confirm ride", "Pay $23.40", "Checkout", "Subscribe", "Send", "Book now", "Confirm"] {
            assert!(is_final(yes), "{yes} is a final step");
        }
        for no in ["Search", "Add to cart", "Home", "Requests", "Sending options", "Play", "Your Library", "Settings"] {
            assert!(!is_final(no), "{no} isn't a final step");
        }
        assert!(is_final("Go to checkout") && is_final("Swipe to pay"));
        // A cart's gift choice isn't sending anything (Uber Eats, 2026-10-13); sending a message or money is.
        assert!(!is_final("Send as a gift. Add a personal message"));
        assert!(is_final("Send $20") && is_final("Send message"));
        assert!(!is_final("Pizza Days,Delivery Fee is $3.49,Delivered in 25 to 45 min,Buy 1, Get 1 Free"));
        assert!(!adds_to_cart("Pizza Days, 4.5 stars, you added it before, see the menu to order again"));
    }

    #[test]
    fn a_checkouts_order() {
        // As a checkout's plain read gives it: prices beside their names or after them, fees, the final step's button.
        let screen = r#"button "Back" [a1]
text "Checkout"
text "Example Kitchen- Springfield"
text "Deliver to 25 Main St"
text "Standard · 25–35 min"
text "1× Classic Fries"
text "$5.50"
text "2× Smash Burger $21.98"
text "Subtotal"
text "$27.48"
text "Delivery Fee $0.49"
text "Taxes & Other Fees"
text "$3.12"
text "Courier tip"
text "$4.00"
text "Visa •••• 4242"
text "Total"
text "$35.09"
button "Place order • $35.09" [a20]"#;
        let o = order_of(screen);
        assert_eq!(o.items, vec![("1× Classic Fries".to_owned(), Some("$5.50".to_owned())), ("2× Smash Burger".to_owned(), Some("$21.98".to_owned()))]);
        assert_eq!(o.total.as_deref(), Some("$35.09"));
        assert_eq!(o.tip.as_deref(), Some("$4.00"));
        assert_eq!(o.to.as_deref(), Some("25 Main St"));
        assert_eq!(o.pay.as_deref(), Some("Visa ••4242"));
        assert_eq!(o.when.as_deref(), Some("25–35 min"));
        // No total line: the final step's own price.
        let o = order_of("text \"Big Mac Meal\"\ntext \"$12.39\"\nbutton \"Place order • $14.02\" [a3]");
        assert_eq!(o.total.as_deref(), Some("$14.02"));
        assert_eq!(o.items.len(), 1);
        // A ride: where to, and the price on its button.
        let o = order_of("text \"Drop-off: Springfield Airport Terminal B\"\ntext \"UberX\"\ntext \"$38.20\"\nbutton \"Confirm UberX\" [a9]");
        assert_eq!(o.to.as_deref(), Some("Springfield Airport Terminal B"));
        assert_eq!(o.items, vec![("UberX".to_owned(), Some("$38.20".to_owned()))]);
        // Nothing an order says: no rows.
        assert!(!order_of("text \"Search\"\nbutton \"Home\" [a1]").any());
        assert_eq!(price_in("about $ 1,204.50 total"), Some((6, 16)));
        assert_eq!(destination("40 Sample Ave").as_deref(), Some("40 Sample Ave"));
        assert_eq!(destination("4.7 stars (9,000+)"), None);
        assert_eq!(payment("Apple Pay").as_deref(), Some("Apple Pay"));
        assert_eq!(payment("Discover new dishes"), None);
        assert_eq!(payment("Mastercard ending in 5555").as_deref(), Some("Mastercard ••5555"));
        // A suggestion to add isn't in the order.
        assert!(order_of("text \"Add a drink? Coke\"\ntext \"$1.99\"").items.is_empty());
        // Uber Eats' cart, as it read on 2026-10-13: an item with all its choices on one line, the gift row, a total.
        let cart = r#"text "Subway"
button "Oven-Roasted Turkey (6''), Quantity: 1, Bread: Artisan Italian, Cheese: Provolone, Veggies: Lettuce, Tomatoes, Red Onions, Sauces: Regular Mayonnaise. Price: $9.19" [a12]
button "Quantity, Decrement" [a13]
text "1"
button "Send as a gift. Add a personal message" [a15]
text "Subtotal: $9.19"
text "Total: $17.32"
button "Go to checkout" [a20]"#;
        let o = order_of(cart);
        assert_eq!(o.items, vec![("Oven-Roasted Turkey (6'')".to_owned(), Some("$9.19".to_owned()))]);
        assert_eq!(o.total.as_deref(), Some("$17.32"));
        assert_eq!(item_name("Medium Coke, Quantity: 2, Price:").as_deref(), Some("2× Medium Coke"));
        // The same cart as a read gives it, long labels cut at 80 characters (their prices with them), and a total
        // before and after a promotion (Uber Eats, 2026-10-13).
        let read = r#"button "Subway" [a4]
text "Subway"
button "Oven-Roasted Turkey (6''), Quantity: 1, " [a5]
button "Oven-Roasted Turkey (6''), Quantity: 1, Bread: Artisan Italian, Cheese: Provolon" [a6]
button "Quantity, Decrement" [a7]
button "Send as a gift. Add a personal message" [a10]
text "Subtotal: $8.99"
button "Delivery Fee: $3.99" [a11]
button "Other Fees: before promotion: $3.00, now: $2.64" [a12]
text "Taxes: $0.63"
text "Total: before promotion: $16.61, now: $16.25"
button "View savings details · 100, Savings progress · Saving $0.36 · with Offers" [a13]
button "Go to checkout" [a14]"#;
        let o = order_of(read);
        assert_eq!(o.items, vec![("Oven-Roasted Turkey (6'')".to_owned(), None)]);
        assert_eq!(o.total.as_deref(), Some("$16.25"));
        assert_eq!(last_price("before promotion: $16.61, now: $16.25").as_deref(), Some("$16.25"));
        // An item's sheet isn't an order: its extras' prices aren't items (Uber Eats, 2026-10-13).
        let sheet = "text \"Oven-Roasted Turkey (6'')\"
text \"$9.19\"
text \"Bacon\"
text \"$1.49\"
button \"Add 1 to cart • $9.19\" [a30]";
        assert!(!order_of(sheet).any());
    }

    #[test]
    fn links_apps_may_open() {
        for ok in ["spotify:search:rabbit%20hole", "uber://?action=setPickup&pickup=my_location", "https://open.spotify.com/track/x"] {
            assert!(allowed_link(ok).is_ok(), "{ok}");
        }
        for bad in ["intent://x#Intent;end", "file:///data/r1os", "content://media/x", "javascript:alert(1)", "http://127.0.0.1:7171/", "no link"] {
            assert!(allowed_link(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn carts_by_their_words() {
        for yes in ["Add 1 to cart • $13.39", "Add to bag", "Add to order", "ADD TO BASKET"] {
            assert!(adds_to_cart(yes), "{yes}");
        }
        for no in ["Cart", "Add to favorites", "Medium Coke®", "Mac Sauce, $0.38, Increment", "Done"] {
            assert!(!adds_to_cart(no), "{no}");
        }
        assert!(adds_to_cart("Update item - $9.99") && !adds_to_cart("Done • $14.14"));
    }

    #[test]
    fn taps_by_place_in_a_test_stop_only_near_a_cart_button() {
        let sheet = ["Italian Herbs & Cheese".to_string(), "Add 1 to cart • $14.14".to_string()];
        let bread = ["Italian Herbs & Cheese".to_string()];
        assert!(by_place(&sheet, &bread, true).is_none());
        assert!(by_place(&sheet, &sheet[1..], true).is_some());
        // Outside a test, an add-to-cart isn't a final step; a place-order button anywhere asks.
        assert!(by_place(&sheet, &sheet[1..], false).is_none());
        let checkout = ["Place order".to_string(), "Change".to_string()];
        assert!(by_place(&checkout, &checkout[1..], false).is_some());
    }

    #[test]
    fn the_window_s_tools() {
        assert!(is_tool("app_click") && is_tool("app_do") && is_tool("app_ask_user"));
        assert!(!is_tool("app_history") && !is_tool("create_app") && !is_tool("undo_app_change"));
    }

    #[test]
    fn acting_ops() {
        assert!(acts("click") && acts("type") && acts("key"));
        assert!(!acts("read") && !acts("screenshot") && !acts("open") && !acts("scroll") && !acts("back"));
    }
}
