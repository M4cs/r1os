//! Agents change the R1 with your permission (ADR-0045). Each agent has a level for each area it may change: the
//! look (the R1's look, the glance screen's widgets, the apps on home, agents' looks), the device (brightness, the
//! screen's timeout, the volume, sounds, the time zone, app builds' model, pausing agents) and apps (their settings
//! and permissions):
//!
//! - **ask** (the default): the R1 shows an approval card (`approval_asked`), waking the display if it's off, and the
//!   tool waits for your answer (`POST /api/approvals/{id}`), up to [`WAIT`]. "Always" sets the area to allow.
//! - **allow**: it goes ahead, and the change shows as it's made.
//! - **never**: the tool says it isn't allowed, and where you'd change that.
//!
//! Granting an app a permission asks every time, whatever the level, and offers no "always": it's still your tap on
//! the R1's own screen that grants (ADR-0023). Taking one away follows the level.
//!
//! The agent's tools say all this, so it tells you to look at the R1 when it's waiting.

use crate::hub::{Event, Hub, Source};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// What an agent may change, and how it may: on the R1 (the look, its settings, apps) and, for an agent working on a
/// computer (docs/specs/companion.md §7.2), what it does there: changing files in its folders, running commands, and
/// anything outside its folders.
pub const AREAS: &[&str] = &["look", "device", "apps", "files", "commands", "outside", "browser", "android"];
pub const LEVELS: &[&str] = &["ask", "allow", "never"];
/// How long a tool waits for your answer; the agents' CLIs are given longer (crate::session, crate::chat).
pub const WAIT: Duration = Duration::from_secs(120);

/// An area in words: "the R1's look", for the card and the tools.
pub fn area_words(area: &str) -> &'static str {
    match area {
        "look" => "the R1's look and home",
        "device" => "the R1's settings",
        "files" => "the files in its folders",
        "commands" => "running commands on the computer",
        "outside" => "anything outside its folders",
        "connections" => "your connected apps and skills",
        "browser" => "using a browser",
        "android" => "using your Android apps",
        _ => "apps' settings and permissions",
    }
}

/// An area's level until you set one: changing files in its own folders is allowed; everything else asks.
pub fn default_level(area: &str) -> &'static str {
    if area == "files" { "allow" } else { "ask" }
}

/// What an agent asks to do, for the card.
pub struct Ask<'a> {
    pub area: &'a str,
    /// What it wants to do, after "<name> wants to": "change the R1's look".
    pub title: String,
    /// The change in words.
    pub detail: String,
    /// Its reason, if it gave one.
    pub why: String,
    /// Asks even when the area is allowed, and offers no "always" (granting an app a permission).
    pub every_time: bool,
    /// What "always" allows, when it isn't the area: a connection's tool (crate::connections), and it in words.
    pub rule: Option<(String, String)>,
    /// The chat whose turn asks: stopping it takes the card away (crate::stops).
    pub chat: Option<String>,
    /// A final step's order, read from the app's screen (crate::appwin::Order): the card shows it as rows.
    pub order: Option<Value>,
}

impl<'a> Ask<'a> {
    pub fn new(area: &'a str, title: impl Into<String>, detail: impl Into<String>) -> Self {
        Ask { area, title: title.into(), detail: detail.into(), why: String::new(), every_time: false, rule: None, chat: None, order: None }
    }

    /// The chat whose turn asks (crate::stops).
    pub fn chat(mut self, chat: &str) -> Self {
        self.chat = (!chat.is_empty()).then(|| chat.to_owned());
        self
    }

    /// "Always" allows this tool for the agent, not the area (crate::connections): `words` says it on the card.
    pub fn rule(mut self, tool: String, words: String) -> Self {
        self.rule = Some((tool, words));
        self
    }

    pub fn why(mut self, why: &str) -> Self {
        self.why = why.trim().chars().take(200).collect();
        self
    }

    pub fn every_time(mut self) -> Self {
        self.every_time = true;
        self
    }

    /// What a final step would order (crate::appwin::Order), for the card's rows.
    pub fn order(mut self, order: Value) -> Self {
        self.order = Some(order);
        self
    }
}

/// A change (JSON) in a few words for the card: `{"accent": "purple", "glance": {"clock": "dial"}}` →
/// "accent purple · glance clock dial".
pub fn words(v: &Value) -> String {
    fn walk(prefix: &str, v: &Value, out: &mut Vec<String>) {
        let key = prefix.replace('_', " ");
        match v {
            Value::Object(m) if m.contains_key("kind") => {
                let mut parts = vec![m["kind"].as_str().unwrap_or_default().to_owned()];
                for (k, x) in m.iter().filter(|(k, _)| *k != "kind") {
                    parts.push(match x {
                        Value::Bool(true) => k.clone(),
                        Value::Bool(false) => format!("no {k}"),
                        _ => format!("{k} {}", scalar(x)),
                    });
                }
                out.push(format!("{key} {}", parts.join(", ")).trim().to_owned());
            }
            Value::Object(m) => {
                for (k, x) in m {
                    walk(format!("{prefix} {k}").trim(), x, out);
                }
            }
            _ => out.push(format!("{key} {}", scalar(v)).trim().to_owned()),
        }
    }
    fn scalar(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Bool(b) => (if *b { "on" } else { "off" }).into(),
            Value::Array(a) if a.is_empty() => "none".into(),
            Value::Array(a) => a.iter().map(scalar).collect::<Vec<_>>().join(", "),
            other => other.to_string(),
        }
    }
    let mut out = Vec::new();
    walk("", v, &mut out);
    out.join(" · ")
}

struct Pending {
    approval: Value,
    answer: tokio::sync::oneshot::Sender<bool>,
}

#[derive(Default)]
pub struct Approvals {
    pending: Mutex<HashMap<String, Pending>>,
}

impl Approvals {
    /// Asks, as the agent's level for the area says: `Ok` to go ahead, `Err` with why not (in words for the agent).
    pub async fn gate(&self, hub: &Hub, agent: &str, ask: Ask<'_>) -> Result<(), String> {
        let area = ask.area;
        let name = hub.core.agent(agent).map(|a| a.name).unwrap_or_else(|| "An agent".into());
        match hub.core.permission(agent, area).as_str() {
            "allow" if !ask.every_time => return Ok(()),
            "never" => {
                return Err(format!(
                    "You aren't allowed to change {}: the user can allow it in your space on the R1 (What it can change).",
                    area_words(area)
                ));
            }
            _ => {}
        }
        tracing::info!(agent, area, "asked for approval");
        match self.wait(hub, agent, &name, ask).await {
            Some(true) => Ok(()),
            Some(false) => Err("The user said no on the R1: nothing changed.".into()),
            None => Err("The user didn't answer on the R1 within 2 minutes: nothing changed. Ask again if they still want it.".into()),
        }
    }

    /// A connection's tool asks (crate::connections): always the card, whose "always" allows that tool for the agent.
    pub async fn gate_rule(&self, hub: &Hub, agent: &str, ask: Ask<'_>) -> Result<(), String> {
        let name = hub.core.agent(agent).map(|a| a.name).unwrap_or_else(|| "An agent".into());
        tracing::info!(agent, tool = ask.rule.as_ref().map(|r| r.0.as_str()).unwrap_or_default(), "asked for a connection's approval");
        match self.wait(hub, agent, &name, ask).await {
            Some(true) => Ok(()),
            Some(false) => Err("The user said no on the R1: it didn't happen.".into()),
            None => Err("The user didn't answer on the R1 within 2 minutes: it didn't happen. Ask again if they still want it.".into()),
        }
    }

    /// A linked computer asks (granting an app a permission from the companion): the R1's card asks you every time,
    /// whatever any agent's level. `Err` says why not, for the companion.
    pub async fn confirm(&self, hub: &Hub, computer: &str, ask: Ask<'_>) -> Result<(), String> {
        tracing::info!(area = ask.area, "a computer asked for approval");
        match self.wait(hub, "", computer, ask.every_time()).await {
            Some(true) => Ok(()),
            Some(false) => Err("you said no on the r1: nothing changed".into()),
            None => Err("nobody answered on the r1 within 2 minutes: nothing changed".into()),
        }
    }

    /// Shows the card (waking the display) and waits for your answer: `None` if there was none in [`WAIT`].
    async fn wait(&self, hub: &Hub, agent: &str, name: &str, ask: Ask<'_>) -> Option<bool> {
        let area = ask.area;
        let id = crate::core::random_hex(6);
        let words = ask.rule.as_ref().map_or_else(|| area_words(area).to_owned(), |r| r.1.clone());
        let approval = json!({
            "id": id, "agent": agent, "name": name, "area": area, "area_words": words,
            "title": ask.title, "detail": ask.detail, "why": ask.why, "always": !ask.every_time, "at": crate::core::now(),
            "rule": ask.rule.as_ref().map(|r| r.0.clone()), "chat": ask.chat, "order": ask.order,
        });
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), Pending { approval: approval.clone(), answer: tx });
        crate::scheduler::wake_display().await;
        hub.emit(Event::ApprovalAsked { approval }, Source::Api);
        match tokio::time::timeout(WAIT, rx).await {
            Ok(Ok(allowed)) => Some(allowed),
            _ => {
                self.pending.lock().unwrap().remove(&id);
                hub.emit(Event::ApprovalDone { id, allowed: false }, Source::Api);
                None
            }
        }
    }

    /// Your answer on the R1. `always`: the agent may change that area without asking from now on.
    pub fn answer(&self, hub: &Hub, id: &str, allow: bool, always: bool) -> anyhow::Result<()> {
        let p = self.pending.lock().unwrap().remove(id).ok_or_else(|| anyhow::anyhow!("no approval `{id}` waiting"))?;
        if allow && always && p.approval["always"] == true {
            let agent = p.approval["agent"].as_str().unwrap_or_default();
            if let Some(tool) = p.approval["rule"].as_str() {
                // A connection's tool: that one, for that agent (crate::connections).
                hub.core.with(|db| crate::connections::set_always(db, agent, tool, true))?;
                hub.emit(Event::ConnectionsChanged, Source::Api);
            } else {
                let area = p.approval["area"].as_str().unwrap_or_default();
                hub.core.set_permission(agent, area, "allow")?;
                hub.emit(Event::AgentsChanged, Source::Api);
            }
        }
        let _ = p.answer.send(allow);
        hub.emit(Event::ApprovalDone { id: id.to_owned(), allowed: allow }, Source::Api);
        Ok(())
    }

    /// The chat's turn was stopped (crate::stops): its cards go, as a no.
    pub fn cancel_chat(&self, hub: &Hub, chat: &str) {
        let gone: Vec<(String, Pending)> = {
            let mut pending = self.pending.lock().unwrap();
            let ids: Vec<String> = pending.iter().filter(|(_, p)| p.approval["chat"] == chat).map(|(id, _)| id.clone()).collect();
            ids.into_iter().filter_map(|id| pending.remove(&id).map(|p| (id, p))).collect()
        };
        for (id, p) in gone {
            let _ = p.answer.send(false);
            hub.emit(Event::ApprovalDone { id, allowed: false }, Source::Api);
        }
    }

    /// What's waiting for an answer (the shell shows the oldest).
    pub fn pending(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self.pending.lock().unwrap().values().map(|p| p.approval.clone()).collect();
        out.sort_by_key(|a| a["at"].as_i64().unwrap_or(0));
        out
    }
}

/// What you said on an approval card (T-055), as its answer: `allow`, `always` or `deny`, or None (the card asks
/// again). Its words first; then their meaning, by System One's embedder (`closeness`: the phrase against each
/// answer's examples, or None while it isn't loaded).
pub fn heard(text: &str, closeness: impl FnOnce(&str, &[&str]) -> Option<Vec<f32>>) -> Option<&'static str> {
    // Speech-to-text's curly apostrophe first ("don’t"), then the rest of the punctuation goes.
    let t: String = text.to_lowercase().replace('\u{2019}', "'").chars().map(|c| if c.is_alphanumeric() || c == '\'' || c == ' ' { c } else { ' ' }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return None;
    }
    let first = t.split(' ').next().unwrap_or_default();
    let has = |ws: &[&str]| ws.iter().any(|w| t == *w || t.starts_with(&format!("{w} ")) || t.contains(&format!(" {w} ")) || t.ends_with(&format!(" {w}")));
    // "Always", and "don't ask (me) again", before any no: that "don't" is a yes.
    if has(&["always", "always allow", "from now on", "every time", "for good", "permanently", "don't ask again", "dont ask again", "don't ask me again", "dont ask me again", "stop asking"]) {
        return Some("always");
    }
    // A no that's a yes, before the noes.
    if has(&["why not", "no problem", "no worries", "not a problem", "don't mind", "dont mind", "i don't mind", "i dont mind"]) {
        return Some("allow");
    }
    // Any no: its first word, or a "not" or "-n't" anywhere ("absolutely not", "I'd rather you didn't").
    let negated = t.split(' ').any(|w| w == "not" || w.ends_with("n't") || matches!(w, "dont" | "didnt" | "wouldnt" | "shouldnt" | "cant" | "wont"));
    if matches!(first, "no" | "nope" | "nah" | "never" | "deny" | "decline" | "reject" | "cancel" | "stop")
        || negated
        || has(&["never mind", "nevermind", "no thanks", "no way", "rather not"])
    {
        return Some("deny");
    }
    if matches!(first, "yes" | "yeah" | "yep" | "yup" | "sure" | "ok" | "okay" | "allow" | "fine" | "approve" | "approved" | "alright" | "accept" | "confirm" | "absolutely" | "definitely")
        || has(&["go ahead", "do it", "all right", "sounds good", "please do", "go for it", "that's fine", "thats fine", "of course", "just this once", "this time", "let it", "it's fine", "its fine"])
    {
        return Some("allow");
    }
    // Its meaning: the closest answer, if it's clearly closest.
    const EXAMPLES: [(&str, &str); 6] = [
        ("allow", "yes, go ahead and do it"),
        ("allow", "that's okay with me"),
        ("always", "always allow it, you don't need to ask me"),
        ("always", "you can do that whenever you want"),
        ("deny", "no, don't do that"),
        ("deny", "I don't want that"),
    ];
    let scores = closeness(&t, &EXAMPLES.map(|(_, e)| e))?;
    let mut best: Vec<(&str, f32)> = Vec::new();
    for ((answer, _), s) in EXAMPLES.iter().zip(scores) {
        match best.iter_mut().find(|(a, _)| a == answer) {
            Some(b) => b.1 = b.1.max(s),
            None => best.push((answer, s)),
        }
    }
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    match best.as_slice() {
        [(a, s1), (_, s2), ..] if *s1 >= HEARD_MIN && s1 - s2 >= HEARD_MARGIN => Some(a),
        _ => None,
    }
}

/// How close (cosine) words must be to an answer's example, and how much closer than the next answer's, for System One
/// to take them as that answer.
const HEARD_MIN: f32 = 0.55;
const HEARD_MARGIN: f32 = 0.06;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_in_words() {
        let none = |_: &str, _: &[&str]| None;
        for (said, answer) in [
            ("Allow", Some("allow")),
            ("yes", Some("allow")),
            ("Yeah, go ahead.", Some("allow")),
            ("okay do it", Some("allow")),
            ("just this once", Some("allow")),
            ("always", Some("always")),
            ("Always allow.", Some("always")),
            ("yes and don't ask me again", Some("always")),
            ("don't allow", Some("deny")),
            ("No.", Some("deny")),
            ("nope", Some("deny")),
            ("not now", Some("deny")),
            ("don't", Some("deny")),
            ("Don’t do that", Some("deny")),
            ("absolutely not", Some("deny")),
            ("of course not", Some("deny")),
            ("I'd rather you didn't", Some("deny")),
            ("why not", Some("allow")),
            ("no problem", Some("allow")),
            ("I don't mind", Some("allow")),
            ("what's the weather", None),
            ("", None),
        ] {
            assert_eq!(heard(said, none), answer, "{said:?}");
        }
    }

    #[test]
    fn answers_by_meaning() {
        // Clearly closest to an answer: it; too close to call: none.
        let leaning = |_: &str, _: &[&str]| Some(vec![0.7, 0.5, 0.3, 0.2, 0.1, 0.1]);
        assert_eq!(heard("I reckon that works", leaning), Some("allow"));
        let torn = |_: &str, _: &[&str]| Some(vec![0.62, 0.4, 0.6, 0.2, 0.1, 0.1]);
        assert_eq!(heard("hmm maybe", torn), None);
        let far = |_: &str, _: &[&str]| Some(vec![0.3, 0.2, 0.2, 0.1, 0.1, 0.1]);
        assert_eq!(heard("tell me a joke", far), None);
    }

    #[test]
    fn changes_in_words() {
        assert_eq!(
            words(&json!({ "accent": "purple", "glance": { "clock": "dial", "hide": ["date", "battery"] } })),
            "accent purple · glance clock dial · glance hide date, battery"
        );
        assert_eq!(
            words(&json!({ "home": { "background": { "kind": "mesh", "colors": ["#112233", "#445566"], "flow": true } } })),
            "home background mesh, colors #112233, #445566, flow"
        );
    }
}
