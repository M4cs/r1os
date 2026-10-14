//! System One (ADR-0032): a fast, on-device reading of what you're saying, while you say it. Where it's sure,
//! the R1 acts without waiting for an agent: it opens the camera as you ask "what am I looking at", turns it
//! around for "how do I look", opens an app, starts an edit, logs to an app, writes in the journal. Anything
//! else goes to your agent (System Two) as before.
//!
//! - **Rules** catch the clear phrasings ("open water", "change my water app so…", "journal: …").
//! - **A sentence embedder** (all-MiniLM-L6-v2, crate::system1::embed) catches the rest by meaning: what's said is
//!   compared with example phrasings of each target (the built-ins, each app's opening, each app's simple
//!   actions), and the nearest wins if it's near enough and clearly nearer than the runner-up. Without the
//!   model (not fetched, or the simulator) the rules work alone.
//! - **Arguments come from code** (crate::system1::extract, eneeware's): numbers and number words. An action
//!   that needs words of yours (an item to add) goes to the agent, which reads them better.
//! - **While you talk**, partials are read (at most every [`PARTIAL_EVERY`]) and `intent` events say what's
//!   coming, so the shell can get ready (the camera opens behind the words). **When you let go**, the final
//!   decision rides on `voice: final` (`intent`), so the shell acts on the words or hands them to the agent.
//!
//! Ported from eneeware's command bar (crates/enee-judge route.rs and extract.rs, ADR-0020 there): the catalog
//! from the apps' specs, code finding the arguments, and acting only above a confidence gate. eneeware's judge
//! is a hosted model (TypeSafe's Jev); here it's on the R1.

pub mod embed;
pub mod extract;
pub mod tokenizer;

use crate::hub::{Event, Hub, Source};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::path::Path;
use std::sync::mpsc as smpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Partials are read at most this often.
const PARTIAL_EVERY: Duration = Duration::from_millis(250);
/// The model's gates: a target this near (cosine), and this much nearer than the next target.
const NEAR: f32 = 0.62;
const CLEAR: f32 = 0.05;
/// Running an app's action on the model's word alone takes more.
const NEAR_ACTION: f32 = 0.72;
/// ...and a short sentence: "I did twenty pushups and then went for a run" is more than a log.
const ACTION_WORDS: usize = 8;

/// What System One makes of an utterance.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Intent {
    /// `look`, `selfie`, `camera`, `photos`, `journal`, `record`, `stop_record`, `open`, `android` (an Android app,
    /// by package in `app`), `media` (the player: `action` pause, resume or stop), `edit`, `run`, `timer` (`args`:
    /// `seconds`; `text`: its label) and `pomodoro` (`args`: `focus` minutes, if said).
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// An action's arguments.
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
    /// The words that go with it: a journal entry, an edit's request.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// How sure: 1 for a rule, the cosine for the model.
    pub score: f32,
    /// `rule` or `model`.
    pub via: String,
}

impl Intent {
    fn new(kind: &str, via: &str, score: f32) -> Self {
        Intent { kind: kind.into(), app: None, action: None, args: Map::new(), text: String::new(), score, via: via.into() }
    }
}

/// One thing System One can do, with the phrasings that mean it.
#[derive(Clone)]
struct Target {
    kind: String,
    app: Option<String>,
    action: Option<String>,
    /// The action's one numeric param, if it has one, and whether it's whole (`int`).
    number: Option<String>,
    integer: bool,
    /// What "a …" may count one of: the param's own name ("a rep"), or for a plain count the table's ("a glass").
    counts: Vec<String>,
    examples: Vec<String>,
    vectors: Vec<Vec<f32>>,
}

pub struct System1 {
    embedder: RwLock<Option<Arc<embed::Embedder>>>,
    targets: RwLock<Vec<Target>>,
    /// [`VISUALS`]' phrasings, embedded once the model is in: what kind of card an answer would suit.
    visuals: RwLock<Vec<(&'static str, Vec<f32>)>>,
    /// The latest partial for the worker (the newest wins).
    partial: Mutex<Option<smpsc::Sender<String>>>,
    status: Mutex<String>,
}

impl System1 {
    /// How close `phrase` is in meaning to each of `names` (cosine, with the on-device embedder): None until it's in.
    /// For the browser's commands (crate::browser::command), when an element's words don't settle it.
    pub fn closeness(&self, phrase: &str, names: &[&str]) -> Option<Vec<f32>> {
        let e = self.embedder.read().unwrap().clone()?;
        let p = e.embed(phrase).ok()?;
        Some(names.iter().map(|n| e.embed(n).map(|v| embed::cosine(&p, &v)).unwrap_or(0.0)).collect())
    }
}

impl Default for System1 {
    fn default() -> Self {
        System1 {
            embedder: RwLock::default(),
            targets: RwLock::default(),
            visuals: RwLock::default(),
            partial: Mutex::default(),
            status: Mutex::new("loading".into()),
        }
    }
}

impl System1 {
    /// Loads the model (on its own thread) and starts the partial reader. `dir`: `<data>/models/system1`.
    pub fn start(hub: &Arc<Hub>, dir: std::path::PathBuf, lib: Option<std::path::PathBuf>) {
        let h = hub.clone();
        let (tx, rx) = smpsc::channel::<String>();
        *hub.system1.partial.lock().unwrap() = Some(tx);
        std::thread::Builder::new()
            .name("system1".into())
            .spawn(move || {
                let t = Instant::now();
                match embed::Embedder::load(&dir, lib.as_deref()) {
                    Ok(e) => {
                        tracing::info!(model = %e.model, ms = t.elapsed().as_millis(), "system one ready");
                        *h.system1.status.lock().unwrap() = format!("ready: {}", e.model);
                        *h.system1.embedder.write().unwrap() = Some(Arc::new(e));
                    }
                    Err(e) => {
                        tracing::info!(error = %format!("{e:#}"), "system one: no model; rules only");
                        *h.system1.status.lock().unwrap() = format!("rules only: {e}");
                    }
                }
                h.system1.rebuild(&h);
                h.system1.embed_visuals();
                worker(&h, &rx);
            })
            .expect("spawn system one");
        // The apps' targets follow the apps.
        let h = hub.clone();
        tokio::spawn(async move {
            let mut rx = h.subscribe();
            while let Ok(ev) = rx.recv().await {
                if matches!(ev.event, Event::AppInstalled { .. } | Event::AppRemoved { .. }) {
                    let h2 = h.clone();
                    let _ = tokio::task::spawn_blocking(move || h2.system1.rebuild(&h2)).await;
                }
            }
        });
    }

    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    /// A partial transcript: read on the worker (it may emit an `intent` to get ready).
    pub fn partial(&self, text: &str) {
        if let Some(tx) = &*self.partial.lock().unwrap() {
            let _ = tx.send(text.to_owned());
        }
    }

    /// The final words: what to do with them, if System One is sure (else the agent has them).
    pub fn decide(&self, hub: &Hub, text: &str) -> Option<Intent> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        if let Some(i) = rules(hub, text) {
            return Some(i);
        }
        self.nearest(text, true)
    }

    /// What kind of card would suit the answer to `text` (ADR-0039), if any: a hint for the agent before it starts,
    /// so it reaches for `show_card` when it helps. Rules first, then the model's nearest [`VISUALS`] phrasing.
    pub fn visual(&self, text: &str) -> Option<&'static str> {
        let t = normal(text);
        if t.is_empty() {
            return None;
        }
        if let Some(v) = visual_rule(&t) {
            return Some(v);
        }
        let embedder = self.embedder.read().unwrap().clone()?;
        let v = embedder.embed(text).ok()?;
        // Each kind's nearest phrasing; the nearest kind wins if it's near enough and clearly nearer than the next.
        let visuals = self.visuals.read().unwrap();
        let mut by_kind: Vec<(f32, &'static str)> = VISUALS
            .iter()
            .map(|(kind, _)| {
                let s = visuals.iter().filter(|(k, _)| k == kind).map(|(_, e)| embed::cosine(&v, e)).fold(f32::MIN, f32::max);
                (s, *kind)
            })
            .collect();
        by_kind.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (best, kind) = *by_kind.first()?;
        let second = by_kind.get(1).map_or(0.0, |s| s.0);
        (best >= NEAR_VISUAL && best - second >= CLEAR).then_some(kind)
    }

    fn embed_visuals(&self) {
        let Some(embedder) = self.embedder.read().unwrap().clone() else { return };
        let mut out = Vec::new();
        for (kind, examples) in VISUALS {
            for e in *examples {
                if let Ok(v) = embedder.embed(e) {
                    out.push((*kind, v));
                }
            }
        }
        *self.visuals.write().unwrap() = out;
    }

    /// What the model makes of `text` (`last`: the final words, where actions may run).
    fn nearest(&self, text: &str, last: bool) -> Option<Intent> {
        let embedder = self.embedder.read().unwrap().clone()?;
        let v = embedder.embed(text).ok()?;
        let targets = self.targets.read().unwrap();
        let mut scored: Vec<(f32, &Target)> =
            targets.iter().map(|t| (t.vectors.iter().map(|e| embed::cosine(&v, e)).fold(f32::MIN, f32::max), t)).collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (best, target) = *scored.first()?;
        let second = scored.iter().find(|(_, t)| t.kind != target.kind || t.app != target.app || t.action != target.action).map_or(0.0, |s| s.0);
        if target.kind == "open" {
            return None;
        }
        let gate = if target.kind == "run" { NEAR_ACTION } else { NEAR };
        if best < gate || best - second < CLEAR {
            return None;
        }
        let mut i = Intent::new(&target.kind, "model", best);
        i.app.clone_from(&target.app);
        i.action.clone_from(&target.action);
        if target.kind == "run" {
            if text.split_whitespace().count() > ACTION_WORDS {
                return None;
            }
            // Its number from the words: exactly one, or "a"/"an" before what it counts; else the agent decides.
            if let Some(param) = &target.number {
                let mut nums = extract::numbers(text);
                if nums.is_empty() {
                    nums = extract::articles(text)
                        .into_iter()
                        .filter(|c| c.surface.split_whitespace().nth(1).is_some_and(|noun| target.counts.iter().any(|w| same_word(noun, w))))
                        .collect();
                }
                if nums.len() != 1 {
                    return None;
                }
                i.args.insert(param.clone(), extract::json_number(nums[0].value, target.integer));
            }
            if !last {
                return None;
            }
        }
        Some(i)
    }

    /// The targets again, from the built-ins and the installed apps, each phrasing embedded once.
    fn rebuild(&self, hub: &Hub) {
        let mut targets = builtins();
        for s in hub.apps.list() {
            let Some(installed) = hub.apps.get(&s.id) else { continue };
            let app = &installed.app;
            let name = app.app.name.to_lowercase();
            targets.push(Target {
                kind: "open".into(),
                app: Some(app.app.id.clone()),
                action: None,
                number: None,
                integer: false,
                counts: Vec::new(),
                examples: vec![format!("open {name}"), format!("show me {name}"), format!("open my {name} app"), format!("go to {name}")],
                vectors: Vec::new(),
            });
            for (action, def) in &app.actions {
                if def.get("confirm").is_some() {
                    continue;
                }
                let params = def["params"].as_object().cloned().unwrap_or_default();
                let numeric: Vec<&String> =
                    params.iter().filter(|(_, d)| matches!(d["type"].as_str(), Some("int" | "real" | "number"))).map(|(k, _)| k).collect();
                let required_text = params.iter().any(|(_, d)| !matches!(d["type"].as_str(), Some("int" | "real" | "number")) && d["required"] == true);
                // Only actions System One can fill (at most one number, no words of yours) that add something:
                // never one that deletes (an undo, a remove), whatever it's called.
                let (tables, toast) = action_words(def);
                if required_text || numeric.len() > 1 || tables.is_empty() || deletes(def) {
                    continue;
                }
                let verb = action.replace('_', " ");
                let what = numeric.first().map(|p| p.replace('_', " ")).unwrap_or_default();
                let param = numeric.first().map(|p| p.as_str()).unwrap_or_default();
                let counts: Vec<String> = if matches!(param, "count" | "amount" | "number" | "n" | "qty" | "quantity" | "times") {
                    tables.split_whitespace().map(str::to_owned).collect()
                } else {
                    vec![param.to_owned()]
                };
                let mut examples = vec![
                    format!("{verb} {what} {name}"),
                    format!("{verb} in {name}"),
                    format!("{name} {verb}"),
                    format!("log {what} {tables} {name}"),
                    format!("add {tables} to {name}"),
                    format!("i did {what} {tables} {name}"),
                    format!("i {verb} {tables} {name}"),
                ];
                if !toast.is_empty() {
                    examples.push(format!("{tables} {toast} {name}"));
                }
                let examples: Vec<String> = examples.into_iter().map(|e| e.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
                targets.push(Target {
                    kind: "run".into(),
                    app: Some(app.app.id.clone()),
                    action: Some(action.clone()),
                    number: numeric.first().map(|p| (*p).clone()),
                    integer: numeric.first().is_some_and(|p| params[*p]["type"] == "int"),
                    counts,
                    examples,
                    vectors: Vec::new(),
                });
            }
        }
        if let Some(e) = self.embedder.read().unwrap().clone() {
            for t in &mut targets {
                t.vectors = t.examples.iter().filter_map(|x| e.embed(x).ok()).collect();
            }
        }
        tracing::info!(targets = targets.len(), "system one: catalog");
        *self.targets.write().unwrap() = targets;
    }
}

/// Reads the latest partial (dropping older ones) and says when the intent changes: only what's worth
/// getting ready for (the camera).
fn worker(hub: &Hub, rx: &smpsc::Receiver<String>) {
    let mut last_kind: Option<String> = None;
    let mut last_text = String::new();
    while let Ok(mut text) = rx.recv() {
        // Only the newest partial matters.
        while let Ok(newer) = rx.try_recv() {
            text = newer;
        }
        // The utterance ended (its final words, or a cancel): start over without a word to the shell,
        // which the final or the cancel already told. A "nothing" here after the final put the camera
        // away before the photo (2026-10-10).
        if text.is_empty() {
            last_kind = None;
            last_text.clear();
            continue;
        }
        if text == last_text {
            continue;
        }
        last_text.clone_from(&text);
        let started = Instant::now();
        let intent = rules(hub, &text).or_else(|| hub.system1.nearest(&text, false)).filter(|i| matches!(i.kind.as_str(), "look" | "selfie"));
        let kind = intent.as_ref().map(|i| i.kind.clone());
        if kind != last_kind {
            last_kind.clone_from(&kind);
            hub.emit(Event::Intent { state: "partial".into(), intent: intent.map(|i| json!(i)) }, Source::Hw);
        }
        std::thread::sleep(PARTIAL_EVERY.saturating_sub(started.elapsed()));
    }
}

/// The action deletes rows (an undo, a remove): System One never runs it.
fn deletes(def: &Value) -> bool {
    def["steps"].as_array().into_iter().flatten().any(|s| s["sql"].as_str().is_some_and(|q| q.trim_start().to_ascii_uppercase().starts_with("DELETE")))
}

/// Two words that are the same but for a plural ("glass", "glasses"; "rep", "reps").
fn same_word(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let plural = |one: &str, many: &str| many == format!("{one}s") || many == format!("{one}es");
    a == b || plural(&a, &b) || plural(&b, &a)
}

/// What an action is about, in words: the tables its steps write ("glasses"), and its toast's own words
/// ("logged"), as eneeware's catalog described actions by what they do.
fn action_words(def: &Value) -> (String, String) {
    let mut tables = Vec::new();
    let mut toast = Vec::new();
    for step in def["steps"].as_array().into_iter().flatten() {
        if let Some(sql) = step["sql"].as_str() {
            let words: Vec<&str> = sql.split_whitespace().collect();
            for w in words.windows(2) {
                if matches!(w[0].to_ascii_uppercase().as_str(), "INTO" | "UPDATE")
                    && let Some(t) = w[1].split('(').next().filter(|t| *t != "settings" && !t.is_empty())
                {
                    tables.push(t.replace('_', " "));
                }
            }
        }
        // A toast is a template: its literal parts are words.
        if let Some(parts) = step["toast"]["t"].as_array() {
            toast.extend(parts.iter().filter_map(Value::as_str).map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_owned()));
        } else if let Some(t) = step["toast"].as_str() {
            toast.push(t.to_owned());
        }
    }
    tables.dedup();
    (tables.join(" "), toast.join(" ").split_whitespace().collect::<Vec<_>>().join(" "))
}

// ---- rules ----------------------------------------------------------------------------------------

/// The clear phrasings, decided without the model.
fn rules(hub: &Hub, text: &str) -> Option<Intent> {
    let t = normal(text);
    if let Some(rest) = strip_any(&t, &["journal ", "dear diary ", "diary ", "note to self "]) {
        let mut i = Intent::new("journal", "rule", 1.0);
        // The words as said (not lowercased), after the lead.
        i.text = text.trim().split_once(char::is_whitespace).map_or(rest.clone(), |(_, r)| r.trim_start_matches([':', ',', ' ']).to_owned());
        return Some(i);
    }
    // r1os's own screens (crate::screens, R7): "what's up", "translate English to Portuguese", "open bluetooth".
    if let Some(i) = screen_rule(&t) {
        return Some(i);
    }
    if matches_any(&t, LOOK) {
        return Some(Intent::new("look", "rule", 1.0));
    }
    if matches_any(&t, SELFIE) {
        return Some(Intent::new("selfie", "rule", 1.0));
    }
    if starts_any(&t, &["start recording", "start a recording", "record this", "record the meeting", "record my"]) {
        return Some(Intent::new("record", "rule", 1.0));
    }
    // The player, while something plays (ADR-0040), r1os's own or an Android app's (R7): "pause", "next song",
    // "what's playing".
    if (hub.player.busy() || hub.android.media().is_some())
        && t.split_whitespace().count() <= 5
        && let Some(op) = media_op(&t)
    {
        let mut i = Intent::new("media", "rule", 1.0);
        i.action = Some(op.into());
        return Some(i);
    }
    // "Play Daft Punk on Spotify" goes to the agent: it finds the thing to play (play_music), where a search from here
    // would only open Spotify's results (R7).
    if starts_any(&t, &["stop recording", "stop the recording", "end the recording"]) {
        return Some(Intent::new("stop_record", "rule", 1.0));
    }
    // "set a timer for ten minutes", "start a pomodoro" (ADR-0050). Only ever from the final words (partials are
    // read for the camera alone), so "for one…" can still become fifteen.
    if let Some(i) = timer_rule(&t) {
        return Some(i);
    }
    if matches_any(&t, &["take a picture", "take a photo", "open the camera", "open camera", "camera"]) && t.split_whitespace().count() <= 4 {
        return Some(Intent::new("camera", "rule", 1.0));
    }
    if matches_any(&t, &["show my photos", "show me my photos", "open photos", "open my photos", "my photos"]) && t.split_whitespace().count() <= 4 {
        return Some(Intent::new("photos", "rule", 1.0));
    }
    // "change my water app so it…" / "edit the water app: …"
    if let Some(rest) = strip_any(&t, &["change ", "edit ", "update ", "modify ", "tweak "]) {
        let rest = rest.trim_start_matches("my ").trim_start_matches("the ");
        for sep in [" app so ", " app to ", " app and ", " app by ", " app: ", " so ", ": "] {
            if let Some((name, _)) = rest.split_once(sep)
                && let Some(app) = app_named(hub, name)
            {
                let mut i = Intent::new("edit", "rule", 1.0);
                i.app = Some(app);
                i.text = text.trim().to_owned();
                return Some(i);
            }
        }
    }
    // "open water", "show me my shopping list", "launch pushups"
    if let Some(rest) = strip_any(&t, &["open ", "show me ", "launch ", "go to ", "pull up ", "start "]) {
        let name = rest.trim_start_matches("my ").trim_start_matches("the ").trim_end_matches(" app");
        if let Some(app) = app_named(hub, name) {
            let mut i = Intent::new("open", "rule", 1.0);
            i.app = Some(app);
            return Some(i);
        }
        // Else an Android app by its name ("open spotify"), and the store by any of its names.
        let android = if STORE_NAMES.contains(&name) {
            hub.android.get(crate::android::STORE).map(|a| a.package)
        } else {
            hub.android.named(name).map(|a| a.package)
        };
        if let Some(package) = android {
            let mut i = Intent::new("android", "rule", 1.0);
            i.app = Some(package);
            return Some(i);
        }
    }
    None
}

/// A timer or a pomodoro to start (ADR-0050): its length from the number extractor ("ten minutes", "an hour and
/// thirty minutes", a bare "10" is minutes), its label from a last "for the pasta". Asking about one, or changing or
/// stopping one, goes to the agent (it has the tools), as does a timer app to build.
fn timer_rule(t: &str) -> Option<Intent> {
    let words: Vec<&str> = t.split_whitespace().collect();
    let pomodoro = words.iter().any(|w| matches!(*w, "pomodoro" | "pomodoros"));
    if words.len() > 14 || !(pomodoro || words.contains(&"timer")) {
        return None;
    }
    const NOT: &[&str] = &[
        "cancel", "stop", "pause", "resume", "remove", "delete", "skip", "left", "how", "what", "when", "which", "is", "was",
        "did", "does", "went", "change", "edit", "app", "build", "add", "more", "extend", "reset", "restart", "list", "show",
    ];
    if words.iter().any(|w| NOT.contains(w)) {
        return None;
    }
    let mut ms = 0.0;
    let mut bare = Vec::new();
    for c in extract::numbers(t) {
        match c.unit {
            extract::Unit::Seconds => ms += c.value * 1000.0,
            extract::Unit::Minutes => ms += c.value * 60_000.0,
            extract::Unit::Hours => ms += c.value * 3_600_000.0,
            extract::Unit::Bare => bare.push(c.value),
            _ => return None,
        }
    }
    if ms == 0.0 && bare.len() == 1 {
        ms = bare[0] * 60_000.0;
    }
    if pomodoro {
        let mut i = Intent::new("pomodoro", "rule", 1.0);
        if ms > 0.0 {
            i.args.insert("focus".into(), json!(ms / 60_000.0));
        }
        return Some(i);
    }
    if !(1000.0..=86_400_000.0).contains(&ms) {
        return None;
    }
    let mut i = Intent::new("timer", "rule", 1.0);
    i.args.insert("seconds".into(), json!((ms / 1000.0).round() as i64));
    // "…for the pasta": the last "for" that isn't the length.
    if let Some((_, last)) = t.rsplit_once(" for ")
        && extract::numbers(last).is_empty()
        && !matches!(last, "me" | "us" | "you" | "now" | "please")
    {
        let label = last.trim_start_matches("the ").trim_start_matches("my ").trim_start_matches("a ").trim_end_matches(" please");
        i.text = label.to_owned();
    }
    Some(i)
}

/// The player's words: pause, resume, stop, next, previous, or what's on (`now`).
fn media_op(t: &str) -> Option<&'static str> {
    let is = |ws: &[&str]| ws.contains(&t);
    if is(&["pause", "pause it", "pause the music", "pause the radio", "hold the music", "pause the song", "pause this", "pause music"]) {
        Some("pause")
    } else if is(&["resume", "unpause", "keep playing", "resume the music", "resume the radio", "play it again", "play", "resume music", "continue playing"]) {
        Some("resume")
    } else if is(&["stop the music", "stop the radio", "stop playing", "turn off the radio", "turn the music off", "turn off the music", "stop music"]) {
        Some("stop")
    } else if is(&["next", "skip", "skip it", "skip this", "skip this song", "next song", "next track", "skip song", "skip the song", "play the next song", "next one"]) {
        Some("next")
    } else if is(&["previous", "previous song", "previous track", "last song", "go back a song", "back a song", "play the last song", "play the previous song"]) {
        Some("previous")
    } else if is(&["what's playing", "whats playing", "what song is this", "what's this song", "whats this song", "who sings this", "what is playing", "who's singing", "what song is playing", "what's on"]) {
        Some("now")
    } else {
        None
    }
}

/// What's up, said as itself.
/// What's up, at once. "What did I miss?", "catch me up" and "brief me" are the agent's (R8): it reads what came while
/// the R1 slept through your connections (crate::missed).
const WHATS_UP: &[&str] = &["what's up", "whats up", "what's new", "whats new", "anything new"];

/// r1os's own screens: what's up; the Translator on the languages named ("translate English to Portuguese", "open the
/// translator for Spanish"); and any screen by name after "open", "show me", "go to" ("open bluetooth settings").
/// Translating some words ("translate hello into Spanish") is the agent's.
/// Endings a spoken site name has ("amazon dot com" → "amazon com" once normal).
const SITE_ENDINGS: &[&str] = &[
    "com", "org", "net", "io", "ai", "co", "dev", "app", "edu", "gov", "uk", "us", "ca", "de", "fr", "info", "me", "tv", "news", "shop",
    "xyz", "gg", "fm", "ly", "so", "to", "sh",
];

/// "go to amazon dot com", "open news ycombinator com": the site, for the R1's browser (ADR-0054).
fn site(t: &str) -> Option<String> {
    let rest = strip_any(t, &["go to ", "open ", "visit ", "pull up ", "browse to ", "load ", "navigate to "])?;
    let words: Vec<&str> = rest.split_whitespace().filter(|w| *w != "dot").collect();
    if words.len() < 2 || words.len() > 4 || !SITE_ENDINGS.contains(words.last()?) {
        return None;
    }
    Some(words.join("."))
}

fn screen_rule(t: &str) -> Option<Intent> {
    // A website, said by name: the R1's browser on it (ADR-0054).
    if let Some(url) = site(t).and_then(|s| crate::browser::allowed_url(&s).ok()) {
        let mut i = Intent::new("screen", "rule", 1.0);
        i.args.insert("target".into(), json!({ "t": "browser", "url": url }));
        return Some(i);
    }
    let screen = |id: &str, mine: Option<&str>, theirs: Option<&str>| {
        let s = crate::screens::named(id)?;
        let target = crate::screens::target(s, mine, theirs).ok()?;
        let mut i = Intent::new("screen", "rule", 1.0);
        i.args.insert("target".into(), target);
        Some(i)
    };
    if WHATS_UP.contains(&t) {
        return screen("whats_up", None, None);
    }
    const LEADS: &[&str] = &[
        "open the translator", "open translator", "open up the translator", "start the translator", "launch the translator",
        "the translator", "translator", "translate from", "translate",
    ];
    if let Some(lead) = LEADS.iter().find(|l| t == **l || t.starts_with(&format!("{l} "))) {
        let rest = t[lead.len()..].trim();
        let joins = ["for", "from", "to", "into", "in", "and", "between", "with", "please", "mode"];
        let langs: Option<Vec<&str>> = rest.split_whitespace().filter(|w| !joins.contains(w)).map(crate::screens::language).collect();
        let langs = langs?;
        // English is the user's when it's one of two; else the first is theirs.
        let (mine, theirs) = match langs[..] {
            [] => (None, None),
            ["en"] => (Some("en"), None),
            [one] => (None, Some(one)),
            [a, "en"] if a != "en" => (Some("en"), Some(a)),
            [a, b] => (Some(a), Some(b)),
            _ => return None,
        };
        return screen("translator", mine, theirs);
    }
    if let Some(rest) = strip_any(t, &["open up ", "open ", "show me ", "go to ", "take me to ", "pull up "])
        && let Some(s) = crate::screens::named(&rest)
    {
        return screen(s.id, None, None);
    }
    if t.ends_with(" settings") {
        return crate::screens::named(t).and_then(|s| screen(s.id, None, None));
    }
    None
}

/// How near (cosine) a [`VISUALS`] phrasing must be for a card hint.
const NEAR_VISUAL: f32 = 0.6;

/// Phrasings whose answers suit a card, by card type (crate::cards::TYPES), for the model.
const VISUALS: &[(&str, &[&str])] = &[
    ("headlines", &["what's the news", "what's happening in the world today", "latest headlines", "any news about the election", "what's going on in tech", "catch me up on the news"]),
    ("weather", &["what's the weather", "will it rain tomorrow", "is it cold outside", "what's the forecast this week", "do i need an umbrella", "how hot will it get today"]),
    ("stats", &["how's the stock market", "what's the price of bitcoin", "what was the score of the game", "how much is apple stock", "what's the exchange rate for euros"]),
    ("bars", &["compare the population of these countries", "which is bigger", "rank the tallest buildings", "how do they stack up"]),
    ("line", &["how has the price changed this year", "show me the trend", "inflation over the last ten years"]),
    ("steps", &["how do i make pancakes", "how does photosynthesis work", "walk me through setting it up", "what are the steps to change a tire", "give me a recipe for lasagna"]),
    ("image", &["show me a picture of a capybara", "what does the eiffel tower look like", "show me what it looks like"]),
    ("table", &["compare the iphone and the pixel", "make me a table of", "what are the differences between them"]),
];

/// The clear phrasings: a card type for the answer, from the words alone.
fn visual_rule(t: &str) -> Option<&'static str> {
    let words = |ws: &[&str]| matches_any(t, ws);
    if words(&["news", "headlines", "headline", "what's happening", "whats happening", "going on in the world", "latest on"]) {
        Some("headlines")
    } else if words(&["weather", "forecast", "temperature", "raining", "rain today", "rain tomorrow", "snowing", "umbrella", "humidity", "how hot", "how cold", "chilly", "windy"]) {
        Some("weather")
    } else if words(&["stock", "stocks", "price of", "bitcoin", "crypto", "exchange rate", "score of", "the score", "scores", "stock market"]) {
        Some("stats")
    } else if words(&["over time", "trend", "over the last", "over the past"]) {
        Some("line")
    } else if words(&["compare", "vs", "versus", "difference between", "differences between"]) {
        Some("table")
    } else if starts_any(t, &["how do i ", "how to ", "how can i ", "how does ", "walk me through", "steps to ", "recipe for", "give me a recipe"]) {
        Some("steps")
    } else if words(&["picture of", "photo of", "look like", "pictures of", "photos of"]) {
        Some("image")
    } else {
        None
    }
}

/// What "open …" calls Aurora Store, the R1's app store for Android apps.
const STORE_NAMES: &[&str] = &["app store", "aurora", "aurora store", "play store", "the store", "store", "android apps"];

/// "What am I looking at?" and friends: about what's in front of the camera.
const LOOK: &[&str] = &[
    "what am i looking at",
    "what am i pointing at",
    "what is this",
    "what's this",
    "whats this",
    "what is that",
    "what's that",
    "what are these",
    "what are those",
    "what do you see",
    "can you see this",
    "look at this",
    "read this",
    "what does this say",
    "what does it say",
    "identify this",
    "describe what you see",
    "what kind of",
    "how much is this",
];

/// "How do I look?" and friends: the camera turns around to you.
const SELFIE: &[&str] = &[
    "how do i look",
    "do i look",
    "look okay on me",
    "look ok on me",
    "look good on me",
    "look alright on me",
    "look fine on me",
    "rate my outfit",
    "how is my hair",
    "how's my hair",
    "check my outfit",
    "look at me",
    "take a selfie",
];

/// The built-in targets, for the model (the rules above catch their usual phrasings).
fn builtins() -> Vec<Target> {
    let t = |kind: &str, examples: &[&str]| Target {
        kind: kind.into(),
        app: None,
        action: None,
        number: None,
        integer: false,
        counts: Vec::new(),
        examples: examples.iter().map(|s| (*s).to_owned()).collect(),
        vectors: Vec::new(),
    };
    vec![
        t("look", &[
            "what am i looking at",
            "what is this thing in front of me",
            "can you tell me what this is",
            "what kind of plant is this",
            "read what this sign says",
            "what does this label say",
            "is this food still good",
            "describe what the camera sees",
        ]),
        t("selfie", &["how do i look", "does this shirt look okay on me", "is my outfit good", "what do you think of my hair", "do i look tired"]),
        t("camera", &["take a picture", "open the camera"]),
        t("photos", &["show me my photos"]),
        t("record", &["start recording", "record this meeting"]),
    ]
}

/// The installed app with this name (or near it: a word off, a plural), its id.
fn app_named(hub: &Hub, name: &str) -> Option<String> {
    let want = normal(name);
    if want.is_empty() {
        return None;
    }
    let mut best: Option<(f64, String)> = None;
    for s in hub.apps.list() {
        let have = normal(&s.name);
        let score = if have == want || have.trim_end_matches('s') == want.trim_end_matches('s') {
            1.0
        } else if have.split_whitespace().any(|w| w == want) || want.split_whitespace().any(|w| w.len() > 3 && have.contains(w)) {
            0.85
        } else {
            similarity(&have, &want)
        };
        if score >= 0.8 && best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, s.id.clone()));
        }
    }
    best.map(|(_, id)| id)
}

/// Lowercase words, punctuation gone.
fn normal(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn matches_any(t: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| t == *p || t.starts_with(&format!("{p} ")) || t.contains(&format!(" {p} ")) || t.ends_with(&format!(" {p}")) || t.contains(p) && p.contains(' '))
}

fn starts_any(t: &str, leads: &[&str]) -> bool {
    leads.iter().any(|l| t.starts_with(l))
}

fn strip_any(t: &str, leads: &[&str]) -> Option<String> {
    leads.iter().find_map(|l| t.strip_prefix(l)).map(|r| r.trim().to_owned()).filter(|r| !r.is_empty())
}

/// 1 − normalised edit distance.
fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    #[allow(clippy::cast_precision_loss)]
    let d = prev[b.len()] as f64;
    1.0 - d / a.len().max(b.len()).max(1) as f64
}

/// The model's location on the R1, and where ONNX Runtime is.
/// ONNX Runtime for r1d's own models (System One's embedder, the Translator's models), loaded once per
/// process from `lib` (`None`: whatever the system finds).
pub fn ort_ready(lib: Option<&Path>) -> anyhow::Result<()> {
    static READY: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    READY
        .get_or_init(|| {
            if let Some(lib) = lib {
                ort::init_from(lib).map_err(|e| format!("ONNX Runtime at {}: {e}", lib.display()))?.with_name("r1d").commit();
            }
            Ok(())
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

pub fn paths(data: &Path) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
    let dir = data.join("models").join("system1");
    let lib = if cfg!(target_os = "android") {
        Some(data.join("lib").join("libonnxruntime.so"))
    } else {
        std::env::var_os("ORT_DYLIB_PATH").map(std::path::PathBuf::from)
    };
    (dir, lib)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_hints() {
        for (said, kind) in [
            ("What's the news today?", Some("headlines")),
            ("any headlines about the launch", Some("headlines")),
            ("what's the weather like in Brooklyn", Some("weather")),
            ("Do I need an umbrella?", Some("weather")),
            ("what's the price of bitcoin", Some("stats")),
            ("compare the pixel vs the iphone", Some("table")),
            ("how do I make cold brew", Some("steps")),
            ("show me a picture of a capybara", Some("image")),
            ("remind me to call mom at 5", None),
            ("thanks!", None),
        ] {
            assert_eq!(visual_rule(&normal(said)), kind, "{said}");
        }
    }

    #[test]
    fn phrasings_and_names() {
        assert!(matches_any(&normal("Hey, what am I looking at?"), LOOK));
        assert!(matches_any(&normal("What's this"), LOOK));
        assert!(matches_any(&normal("does this look okay on me"), SELFIE));
        assert!(!matches_any(&normal("what's the weather"), LOOK));
        assert!(!matches_any(&normal("look up the news"), SELFIE));
        assert_eq!(strip_any("journal ran 5k", &["journal "]).as_deref(), Some("ran 5k"));
        assert!(similarity("pushups", "push ups") > 0.8);
        assert!(similarity("water", "weather") < 0.8);
        assert!(same_word("glass", "glasses") && same_word("reps", "rep") && same_word("set", "set"));
        assert!(!same_word("set", "reps"));
    }

    #[test]
    fn screens_from_what_is_said() {
        let target = |said: &str| screen_rule(&normal(said)).map(|i| i.args["target"].clone());
        assert_eq!(target("Open translator for English to Portuguese"), Some(json!({ "t": "tool", "id": "translator", "mine": "en", "theirs": "pt" })));
        // As Max said it, and as speech-to-text can spell it.
        assert_eq!(target("Open translator for english to portugese"), Some(json!({ "t": "tool", "id": "translator", "mine": "en", "theirs": "pt" })));
        assert_eq!(target("translate portuguese to english"), Some(json!({ "t": "tool", "id": "translator", "mine": "en", "theirs": "pt" })));
        assert_eq!(target("open the translator for spanish"), Some(json!({ "t": "tool", "id": "translator", "theirs": "es" })));
        assert_eq!(target("translator"), Some(json!({ "t": "tool", "id": "translator" })));
        assert_eq!(target("translate hello into spanish"), None, "words to translate are the agent's");
        assert_eq!(target("open bluetooth settings"), Some(json!({ "t": "settings", "page": "bluetooth" })));
        assert_eq!(target("Bluetooth settings"), Some(json!({ "t": "settings", "page": "bluetooth" })));
        assert_eq!(target("show me my routines"), Some(json!({ "t": "tool", "id": "routines" })));
        assert_eq!(target("What's up?"), Some(json!({ "t": "tool", "id": "whats_up" })));
        assert_eq!(target("open spotify"), None);
        assert_eq!(target("go to amazon dot com"), Some(json!({ "t": "browser", "url": "https://amazon.com" })));
        assert_eq!(target("Open news.ycombinator.com"), Some(json!({ "t": "browser", "url": "https://news.ycombinator.com" })));
        assert_eq!(target("open the browser"), Some(json!({ "t": "browser" })));
    }

    #[test]
    fn the_players_words() {
        assert_eq!(media_op(&normal("Next song.")), Some("next"));
        assert_eq!(media_op(&normal("skip this")), Some("next"));
        assert_eq!(media_op(&normal("go back a song")), Some("previous"));
        assert_eq!(media_op(&normal("What's playing?")), Some("now"));
        assert_eq!(media_op(&normal("pause")), Some("pause"));
        assert_eq!(media_op(&normal("play")), Some("resume"));
        assert_eq!(media_op(&normal("next week's weather")), None);
    }

    #[test]
    fn timers_from_what_is_said() {
        let timer = |said: &str| {
            timer_rule(&normal(said)).filter(|i| i.kind == "timer").map(|i| (i.args["seconds"].as_i64().unwrap_or(0), i.text))
        };
        assert_eq!(timer("Set a timer for ten minutes."), Some((600, String::new())));
        assert_eq!(timer("set a 5-minute timer for the pasta"), Some((300, "pasta".into())));
        assert_eq!(timer("timer 90 seconds"), Some((90, String::new())));
        assert_eq!(timer("set a timer for 1 hour and 30 minutes for my bread"), Some((5400, "bread".into())));
        assert_eq!(timer("start a timer for 10"), Some((600, String::new())));
        assert_eq!(timer("set a timer for me for twenty five minutes please"), Some((1500, String::new())));
        assert_eq!(timer("set a timer for half an hour"), Some((1800, String::new())));
        for not in [
            "cancel the timer",
            "how long is left on my timer",
            "is my 10 minute timer done",
            "add 5 minutes to the timer",
            "make me a timer app with 3 presets",
            "set a timer",
            "set a timer at 5 pm",
            "what's the weather in 10 minutes",
        ] {
            assert_eq!(timer(not), None, "{not}");
        }
        let p = timer_rule(&normal("Start a pomodoro")).unwrap();
        assert_eq!((p.kind.as_str(), p.args.get("focus")), ("pomodoro", None));
        let p = timer_rule(&normal("let's do a 50 minute pomodoro")).unwrap();
        assert_eq!((p.kind.as_str(), p.args["focus"].as_f64()), ("pomodoro", Some(50.0)));
        assert!(timer_rule(&normal("skip this pomodoro")).is_none());
    }
}
