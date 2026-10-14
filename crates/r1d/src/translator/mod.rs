//! The Translator (ADR-0034): hold the button and speak either language; the R1 shows and says it in the
//! other.
//!
//! While its screen is open (a lease the host renews, [`Translator::start`]), push-to-talk keeps the
//! utterance's audio for it instead of the English recognizer (crate::voice). Then, on one worker thread:
//!
//! 1. **Hear:** Whisper transcribes and names the language (speech.rs).
//! 2. **Direction:** your language into theirs; theirs, or any other, into yours.
//! 3. **Translate on the device** when OPUS-MT models cover it (mt.rs): directly, through en-ROMANCE's
//!    target token, or through English. Otherwise **the agent** (one no-tools Claude turn, Haiku).
//! 4. **Say it** in the target language's Piper voice, if there is one. The WAV is kept in memory for
//!    the host to play (the last few turns).
//!
//! Events: `translation` with `state` listening → hearing → heard → translated → spoken, or error /
//! cancelled. The models load when the screen opens and are dropped 5 minutes after it hides (or when a
//! forgotten lease runs out): they take a few hundred MB of the R1's 4 GB.
//!
//! Models live in `<data>/models/translate` (tools/fetch-models.sh): `asr/` (Whisper), `mt/<from>-<to>/`,
//! `tts/<lang>/` and the voices' shared `tts/espeak-ng-data/`.

pub mod mt;
pub mod speech;

use crate::hub::{Event, Hub, Source};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as smpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The screen renews it each time it opens, and each turn extends it.
const LEASE: Duration = Duration::from_secs(15 * 60);
/// After the screen hides (or the display sleeps), the models stay this long: coming back is instant.
const KEEP_LOADED: Duration = Duration::from_secs(5 * 60);
/// Translation models kept loaded (each ~150 MB in memory).
const KEEP_MODELS: usize = 2;
const KEEP_VOICES: usize = 2;
/// Spoken turns kept for the host to play.
const KEEP_SPEECH: usize = 6;
/// The translation models and voices on all eight of the R1's cores: a short sentence in 0.8 s warm,
/// against 1.2 s on four (2026-10-10).
const THREADS: usize = 8;
/// Whisper too: 3.5 s for a 3.8 s clip, against 4.9 s on four.
const HEAR_THREADS: i32 = 8;

/// What it offers, by name. Whisper hears all of them; on-device translation and voices cover the ones
/// tools/fetch-models.sh installs, and the agent the rest.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("zh", "Chinese"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("ru", "Russian"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
    ("id", "Indonesian"),
    ("th", "Thai"),
    ("sv", "Swedish"),
    ("el", "Greek"),
    ("he", "Hebrew"),
    ("ro", "Romanian"),
    ("cs", "Czech"),
];

/// The languages OPUS-MT's en-ROMANCE and ROMANCE-en cover (en-ROMANCE takes `>>xx<<` first).
const ROMANCE: &[&str] = &["es", "fr", "it", "pt", "ro", "ca", "gl"];

pub fn name_of(code: &str) -> &str {
    LANGUAGES.iter().find(|(c, _)| *c == code).map_or(code, |(_, n)| n)
}

/// One turn, as the `translation` event carries it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Turn {
    pub id: u64,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// `device` or `agent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Its spoken translation: `GET /api/translator/speech/{speech}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Time taken so far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
}

struct Lease {
    mine: String,
    theirs: String,
    until: Instant,
}

enum Whisper {
    None,
    Loading,
    Ready(Arc<speech::Whisper>),
    Failed(String),
}

#[derive(Default)]
struct Loaded {
    /// The pair warmed up last (`mine-theirs`): opening the screen again doesn't repeat it.
    warmed: Option<String>,
    models: VecDeque<(String, Arc<mt::Marian>)>,
    voices: VecDeque<(String, Arc<speech::Speaker>)>,
}

enum Job {
    Heard { id: u64, samples: Vec<f32> },
    /// English, already heard by push-to-talk's recognizer (no Whisper).
    English { id: u64, text: String, started: Instant },
}

/// Push-to-talk's recognizer (Kroko, streaming and always loaded) is this sure of what it heard, or surer:
/// it was English, and Whisper's ~3.5 s (it always encodes a 30 s window) can be skipped. Its mean token
/// log-probability on the R1's own voices: English −0.25 to −0.33; Italian −0.84, Portuguese −1.24,
/// Russian −1.35, French −1.42, Spanish −1.49 (2026-10-10).
pub const ENGLISH_SURE: f32 = -0.6;
/// Fewer words than this are too short to judge: Whisper decides.
pub const ENGLISH_MIN_TOKENS: usize = 3;

pub struct Translator {
    /// Held while a model or voice loads: the worker and the warm-up don't load the same one twice.
    loading: Mutex<()>,
    /// How ready it is: off · loading · english · ready (`translator_stage` events).
    stage: Mutex<String>,
    root: Mutex<PathBuf>,
    lib: Mutex<Option<PathBuf>>,
    lease: Mutex<Option<Lease>>,
    whisper: Arc<Mutex<Whisper>>,
    loaded: Mutex<Loaded>,
    speech: Mutex<VecDeque<(u64, Arc<Vec<u8>>)>>,
    next: AtomicU64,
    jobs: Mutex<Option<smpsc::Sender<Job>>>,
    rt: Mutex<Option<tokio::runtime::Handle>>,
}

impl Default for Translator {
    fn default() -> Self {
        Translator {
            loading: Mutex::default(),
            stage: Mutex::new("off".into()),
            root: Mutex::default(),
            lib: Mutex::default(),
            lease: Mutex::default(),
            whisper: Arc::new(Mutex::new(Whisper::None)),
            loaded: Mutex::default(),
            speech: Mutex::default(),
            next: AtomicU64::new(1),
            jobs: Mutex::default(),
            rt: Mutex::default(),
        }
    }
}

/// Where the models are, and ONNX Runtime for the on-device translation (System One's).
pub fn paths(data: &Path) -> (PathBuf, Option<PathBuf>) {
    (data.join("models").join("translate"), crate::system1::paths(data).1)
}

impl Translator {
    /// Starts its worker. Loads nothing until the screen opens.
    pub fn init(hub: &Arc<Hub>, root: PathBuf, lib: Option<PathBuf>) {
        let t = &hub.translator;
        *t.root.lock().unwrap() = root;
        *t.lib.lock().unwrap() = lib;
        *t.rt.lock().unwrap() = Some(tokio::runtime::Handle::current());
        let (tx, rx) = smpsc::channel();
        *t.jobs.lock().unwrap() = Some(tx);
        let h = hub.clone();
        std::thread::Builder::new()
            .name("translator".into())
            .spawn(move || {
                for job in rx {
                    match job {
                        Job::Heard { id, samples } => h.translator.turn(&h, id, &samples),
                        Job::English { id, text, started } => h.translator.turn_heard(&h, id, started, &text, "en"),
                    }
                }
            })
            .expect("spawn the translator");
    }

    fn root(&self) -> PathBuf {
        self.root.lock().unwrap().clone()
    }

    /// The screen is open: push-to-talk is the Translator's.
    pub fn active(&self) -> bool {
        self.lease.lock().unwrap().as_ref().is_some_and(|l| l.until > Instant::now())
    }

    /// One of the two languages is English: push-to-talk's recognizer listens too, and if it's sure it
    /// heard English, its words are used at once (ADR-0034's update).
    pub fn english_side(&self) -> bool {
        self.lease.lock().unwrap().as_ref().is_some_and(|l| l.mine == "en" || l.theirs == "en")
    }

    /// Opens (or renews) the lease for `mine` ⇄ `theirs` and starts loading Whisper.
    pub fn start(&self, hub: &Arc<Hub>, mine: &str, theirs: &str) -> anyhow::Result<()> {
        anyhow::ensure!(LANGUAGES.iter().any(|(c, _)| *c == mine), "unknown language `{mine}`");
        anyhow::ensure!(LANGUAGES.iter().any(|(c, _)| *c == theirs), "unknown language `{theirs}`");
        anyhow::ensure!(mine != theirs, "pick two different languages");
        let fresh = {
            let mut l = self.lease.lock().unwrap();
            let fresh = l.as_ref().is_none_or(|l| l.until <= Instant::now());
            *l = Some(Lease { mine: mine.into(), theirs: theirs.into(), until: Instant::now() + LEASE });
            fresh
        };
        self.warm_up(hub, mine, theirs);
        if fresh {
            let h = hub.clone();
            tokio::spawn(async move { h.translator.expire().await });
        }
        Ok(())
    }

    /// The screen hid: push-to-talk is back at once; the models go a little later, unless it comes back.
    pub fn stop(&self, hub: &Arc<Hub>) {
        *self.lease.lock().unwrap() = None;
        let h = hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(KEEP_LOADED).await;
            if !h.translator.active() {
                h.translator.unload();
            }
        });
    }

    async fn expire(&self) {
        loop {
            let until = match &*self.lease.lock().unwrap() {
                Some(l) => l.until,
                None => return,
            };
            let now = Instant::now();
            if until <= now {
                break;
            }
            tokio::time::sleep(until - now).await;
        }
        tracing::info!("translator lease over; models dropped");
        *self.lease.lock().unwrap() = None;
        self.unload();
    }

    /// Loads and runs, once, what this pair will need, one thing at a time and in the order a first turn
    /// needs them (they compete for the same cores, and a turn waits for what it needs):
    ///
    /// - one side English: English into theirs and its voice (heard at once through push-to-talk's
    ///   recognizer), then Whisper, then the other way round;
    /// - neither: Whisper first (every turn needs it), then both ways.
    ///
    /// A model's first run on the R1 is several times slower than the next ones, so each is run once.
    fn warm_up(&self, hub: &Arc<Hub>, mine: &str, theirs: &str) {
        let pair = format!("{mine}-{theirs}");
        {
            let mut l = self.loaded.lock().unwrap();
            if l.warmed.as_deref() == Some(pair.as_str()) {
                return;
            }
            l.warmed = Some(pair);
        }
        let ears = self.claim_whisper();
        self.set_stage(hub, "loading");
        let (h, mine, theirs) = (hub.clone(), mine.to_owned(), theirs.to_owned());
        std::thread::Builder::new()
            .name("translator-warm".into())
            .spawn(move || {
                let t = Instant::now();
                let me = &h.translator;
                let direction = |from: &str, to: &str| {
                    // Pivots through English need four models: they load as they're used instead.
                    if let Some([hop]) = me.route(from, to).as_deref()
                        && let Ok(m) = me.model(&hop.model)
                    {
                        let _ = m.translate("Hello.", hop.target.as_deref());
                    }
                    if let Some(v) = me.voice(to) {
                        let _ = v.say("Hi.");
                    }
                };
                let other = if mine == "en" { theirs.as_str() } else { mine.as_str() };
                if mine == "en" || theirs == "en" {
                    direction("en", other);
                    tracing::info!(ms = elapsed_ms(t), "translator: english ready");
                    me.set_stage(&h, "english");
                    if ears {
                        me.load_whisper();
                    }
                    direction(other, "en");
                } else {
                    if ears {
                        me.load_whisper();
                    }
                    direction(&mine, &theirs);
                    direction(&theirs, &mine);
                }
                tracing::info!(pair = %format!("{mine}-{theirs}"), ms = elapsed_ms(t), "translator warmed up");
                // Unless it was closed meanwhile (the models are gone).
                if me.loaded.lock().unwrap().warmed.is_some() {
                    me.set_stage(&h, "ready");
                }
            })
            .expect("spawn the translator's warm-up");
    }

    /// Marks Whisper as loading (the screen says it's getting ready), if it isn't loaded or loading already.
    /// True when the caller should load it.
    fn claim_whisper(&self) -> bool {
        let mut w = self.whisper.lock().unwrap();
        if matches!(*w, Whisper::Loading | Whisper::Ready(_)) {
            return false;
        }
        *w = Whisper::Loading;
        true
    }

    fn extend(&self) {
        if let Some(l) = self.lease.lock().unwrap().as_mut() {
            l.until = Instant::now() + LEASE;
        }
    }

    fn set_stage(&self, hub: &Hub, stage: &str) {
        let mut s = self.stage.lock().unwrap();
        if *s != stage {
            stage.clone_into(&mut s);
            hub.emit(Event::TranslatorStage { stage: stage.to_owned() }, Source::Api);
        }
    }

    fn unload(&self) {
        let was = std::mem::replace(&mut *self.whisper.lock().unwrap(), Whisper::None);
        *self.loaded.lock().unwrap() = Loaded::default();
        // Nobody's watching (the screen is gone): no event, the next open starts from "loading".
        "off".clone_into(&mut self.stage.lock().unwrap());
        if !matches!(was, Whisper::None) {
            tracing::info!("translator's models dropped");
        }
    }

    /// Loads Whisper (claimed by [`Translator::claim_whisper`]), on the calling thread.
    fn load_whisper(&self) {
        let t = Instant::now();
        let loaded = speech::Whisper::load(&self.root().join("asr"), HEAR_THREADS);
        // Its first run is slow: get it out of the way on a second of silence.
        if let Ok(w) = &loaded {
            let _ = w.hear(&[0.0; 16_000]);
        }
        let mut w = self.whisper.lock().unwrap();
        // Stopped while loading: don't keep it.
        if !matches!(*w, Whisper::Loading) {
            return;
        }
        *w = match loaded {
            Ok(m) => {
                tracing::info!(model = %m.model, ms = t.elapsed().as_millis(), "translator's whisper ready");
                Whisper::Ready(Arc::new(m))
            }
            Err(e) => {
                tracing::warn!(error = %e, "translator: no whisper");
                Whisper::Failed(e.to_string())
            }
        };
    }

    /// Whisper, once it's loaded: a turn in the other language waits for it (up to 40 s) rather than fail.
    fn ears(&self) -> Result<Arc<speech::Whisper>, String> {
        let until = Instant::now() + Duration::from_secs(40);
        loop {
            match &*self.whisper.lock().unwrap() {
                Whisper::Ready(w) => return Ok(w.clone()),
                Whisper::Failed(e) => return Err(format!("The translator can't hear: {e}")),
                Whisper::None => return Err("The translator isn't open.".into()),
                Whisper::Loading if Instant::now() > until => return Err("The translator's ears are still loading.".into()),
                Whisper::Loading => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// `GET /api/translator`.
    pub fn status(&self) -> Value {
        let root = self.root();
        let lease = self.lease.lock().unwrap();
        let active = lease.as_ref().filter(|l| l.until > Instant::now());
        let hearing = match &*self.whisper.lock().unwrap() {
            Whisper::None => json!("off"),
            Whisper::Loading => json!("loading"),
            Whisper::Ready(_) => json!("ready"),
            Whisper::Failed(e) => json!({ "error": e }),
        };
        let can_hear = root.join("asr").join("encoder.int8.onnx").is_file();
        let languages: Vec<Value> = LANGUAGES
            .iter()
            .map(|(code, name)| {
                let on_device: Vec<&str> =
                    LANGUAGES.iter().map(|(c, _)| *c).filter(|to| to != code && self.route(code, to).is_some()).collect();
                json!({
                    "code": code,
                    "name": name,
                    "voice": root.join("tts").join(code).join("model.onnx").is_file(),
                    "on_device": on_device,
                })
            })
            .collect();
        json!({
            "active": active.is_some(),
            "stage": self.stage.lock().unwrap().clone(),
            "mine": active.map(|l| l.mine.clone()),
            "theirs": active.map(|l| l.theirs.clone()),
            "hearing": hearing,
            "installed": can_hear,
            "languages": languages,
        })
    }

    /// A finished utterance from push-to-talk (crate::voice): translate it on the worker.
    pub fn heard(&self, hub: &Hub, samples: Vec<f32>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        hub.emit(Event::Translation(Turn { id, state: "hearing".into(), ..Turn::default() }), Source::Hw);
        if let Some(tx) = &*self.jobs.lock().unwrap() {
            let _ = tx.send(Job::Heard { id, samples });
        }
    }

    pub fn listening(&self, hub: &Hub) {
        hub.emit(Event::Translation(Turn { state: "listening".into(), ..Turn::default() }), Source::Hw);
    }

    /// Words while you talk, once they sound like English (push-to-talk's recognizer).
    pub fn partial(&self, hub: &Hub, text: &str) {
        hub.emit(Event::Translation(Turn { state: "partial".into(), heard: Some(text.to_owned()), from: Some("en".into()), ..Turn::default() }), Source::Hw);
    }

    /// A finished utterance push-to-talk's recognizer is sure was English: straight to translation.
    pub fn heard_english(&self, hub: &Hub, text: String) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if let Some(tx) = &*self.jobs.lock().unwrap() {
            let _ = tx.send(Job::English { id, text, started: Instant::now() });
        }
        hub.emit(Event::Translation(Turn { id, state: "hearing".into(), ..Turn::default() }), Source::Hw);
    }

    pub fn cancelled(&self, hub: &Hub) {
        hub.emit(Event::Translation(Turn { state: "cancelled".into(), ..Turn::default() }), Source::Hw);
    }

    /// One spoken turn, start to finish (the worker).
    fn turn(&self, hub: &Hub, id: u64, samples: &[f32]) {
        let started = Instant::now();
        let emit = |t: Turn| hub.emit(Event::Translation(Turn { id, ms: Some(elapsed_ms(started)), ..t }), Source::Hw);
        let fail = |msg: String| emit(Turn { state: "error".into(), error: Some(msg), ..Turn::default() });
        self.extend();
        if self.lease.lock().unwrap().is_none() {
            return fail("The translator isn't open.".into());
        }
        let whisper = match self.ears() {
            Ok(w) => w,
            Err(e) => return fail(e),
        };
        #[allow(clippy::cast_precision_loss)]
        let secs = samples.len() as f32 / 16_000.0;
        let heard = whisper.hear(samples);
        let Some(heard) = heard.filter(|h| !h.text.is_empty()) else {
            return fail("I didn't catch that.".into());
        };
        tracing::info!(audio_s = secs, from = %heard.lang, ms = elapsed_ms(started), "translator heard (whisper)");
        self.turn_heard(hub, id, started, &heard.text, &heard.lang);
    }

    /// The rest of a turn, once the words and their language are known.
    fn turn_heard(&self, hub: &Hub, id: u64, started: Instant, heard: &str, from: &str) {
        let emit = |t: Turn| hub.emit(Event::Translation(Turn { id, ms: Some(elapsed_ms(started)), ..t }), Source::Hw);
        self.extend();
        let Some((mine, theirs)) = self.lease.lock().unwrap().as_ref().map(|l| (l.mine.clone(), l.theirs.clone())) else {
            return emit(Turn { state: "error".into(), error: Some("The translator isn't open.".into()), ..Turn::default() });
        };
        let to = if from == mine { theirs } else { mine };
        emit(Turn { state: "heard".into(), heard: Some(heard.to_owned()), from: Some(from.to_owned()), to: Some(to.clone()), ..Turn::default() });
        let base = Turn { heard: Some(heard.to_owned()), from: Some(from.to_owned()), to: Some(to.clone()), ..Turn::default() };
        match self.translate(heard, from, &to) {
            Ok((text, via)) => {
                emit(Turn { state: "translated".into(), text: Some(text.clone()), via: Some(via.clone()), ..base.clone() });
                let speech = self.speak(&to, &text);
                emit(Turn { state: "spoken".into(), text: Some(text), via: Some(via), speech, ..base });
            }
            Err(e) => emit(Turn { state: "error".into(), error: Some(format!("{e:#}")), ..base }),
        }
    }

    /// `text` from `from` into `to`: (translation, `device` | `agent`).
    pub fn translate(&self, text: &str, from: &str, to: &str) -> anyhow::Result<(String, String)> {
        if from == to {
            return Ok((text.to_owned(), "device".into()));
        }
        if let Some(hops) = self.route(from, to) {
            let mut s = text.to_owned();
            let mut ok = true;
            for hop in hops {
                match self.model(&hop.model).and_then(|m| m.translate(&s, hop.target.as_deref())) {
                    Ok(t) => s = t,
                    Err(e) => {
                        tracing::warn!(model = %hop.model, error = %format!("{e:#}"), "on-device translation failed; asking the agent");
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return Ok((s, "device".into()));
            }
        }
        self.by_agent(text, from, to).map(|t| (t, "agent".into()))
    }

    /// The models that take `from` to `to` on the device, if any.
    fn route(&self, from: &str, to: &str) -> Option<Vec<Hop>> {
        let root = self.root().join("mt");
        let has = |m: &str| root.join(m).join("decoder.onnx").is_file();
        let hop = |from: &str, to: &str| -> Option<Hop> {
            let direct = format!("{from}-{to}");
            if has(&direct) {
                Some(Hop { model: direct, target: None })
            } else if from == "en" && ROMANCE.contains(&to) && has("en-ROMANCE") {
                // Brazilian Portuguese, as the Portuguese voice is.
                let target = if to == "pt" { ">>pt_BR<<".to_owned() } else { format!(">>{to}<<") };
                Some(Hop { model: "en-ROMANCE".into(), target: Some(target) })
            } else if to == "en" && ROMANCE.contains(&from) && has("ROMANCE-en") {
                Some(Hop { model: "ROMANCE-en".into(), target: None })
            } else {
                None
            }
        };
        if let Some(h) = hop(from, to) {
            return Some(vec![h]);
        }
        if from != "en" && to != "en" {
            return Some(vec![hop(from, "en")?, hop("en", to)?]);
        }
        None
    }

    fn model(&self, name: &str) -> anyhow::Result<Arc<mt::Marian>> {
        let cached = || self.loaded.lock().unwrap().models.iter().find(|(n, _)| n == name).map(|(_, m)| m.clone());
        if let Some(m) = cached() {
            return Ok(m);
        }
        let _one = self.loading.lock().unwrap();
        if let Some(m) = cached() {
            return Ok(m);
        }
        crate::system1::ort_ready(self.lib.lock().unwrap().as_deref())?;
        let t = Instant::now();
        let m = Arc::new(mt::Marian::load(&self.root().join("mt").join(name), THREADS)?);
        tracing::info!(model = name, ms = elapsed_ms(t), "translation model loaded");
        let mut l = self.loaded.lock().unwrap();
        l.models.push_front((name.to_owned(), m.clone()));
        l.models.truncate(KEEP_MODELS);
        Ok(m)
    }

    /// One no-tools Claude turn (Haiku): for the pairs the device has no model for.
    fn by_agent(&self, text: &str, from: &str, to: &str) -> anyhow::Result<String> {
        let rt = self.rt.lock().unwrap().clone().ok_or_else(|| anyhow::anyhow!("no runtime"))?;
        let req = crate::agent::Request {
            model: "haiku".into(),
            effort: None,
            system_prompt: format!(
                "You are a translator on a handheld device. Translate the user's message from {} into {}. \
                 Reply with the translation only: no quotes, notes, romanization or explanations. Keep names, \
                 numbers and the tone. If the message is already in {}, reply with it unchanged.",
                name_of(from),
                name_of(to),
                name_of(to)
            ),
            prompt: text.to_owned(),
            timeout: Duration::from_secs(45),
        };
        let out = rt
            .block_on(crate::agent::claude_no_tools(&req, &mut |_: &str| {}))
            .map_err(|e| anyhow::anyhow!("no on-device model for {} to {}, and the agent couldn't help: {e:#}", name_of(from), name_of(to)))?;
        Ok(out.text.trim().trim_matches('"').trim().to_owned())
    }

    /// `text` in `lang`'s voice, kept for the host to play; None without a voice.
    fn speak(&self, lang: &str, text: &str) -> Option<u64> {
        let voice = self.voice(lang)?;
        let t = Instant::now();
        let wav = voice.say(text)?;
        tracing::info!(lang, ms = elapsed_ms(t), bytes = wav.len(), "translator spoke");
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut s = self.speech.lock().unwrap();
        s.push_back((id, Arc::new(wav)));
        while s.len() > KEEP_SPEECH {
            s.pop_front();
        }
        Some(id)
    }

    fn voice(&self, lang: &str) -> Option<Arc<speech::Speaker>> {
        let cached = || self.loaded.lock().unwrap().voices.iter().find(|(l, _)| l == lang).map(|(_, v)| v.clone());
        if let Some(v) = cached() {
            return Some(v);
        }
        let _one = self.loading.lock().unwrap();
        if let Some(v) = cached() {
            return Some(v);
        }
        let root = self.root().join("tts");
        let dir = root.join(lang);
        if !dir.join("model.onnx").is_file() {
            return None;
        }
        match speech::Speaker::load(&dir, &root.join("espeak-ng-data"), i32::try_from(THREADS).unwrap_or(4)) {
            Ok(v) => {
                let v = Arc::new(v);
                let mut l = self.loaded.lock().unwrap();
                l.voices.push_front((lang.to_owned(), v.clone()));
                l.voices.truncate(KEEP_VOICES);
                Some(v)
            }
            Err(e) => {
                tracing::warn!(lang, error = %e, "translator: voice didn't load");
                None
            }
        }
    }

    /// A spoken turn's WAV.
    pub fn speech(&self, id: u64) -> Option<Arc<Vec<u8>>> {
        self.speech.lock().unwrap().iter().find(|(i, _)| *i == id).map(|(_, w)| w.clone())
    }

    /// Typed text (`POST /api/translator/text`), optionally spoken: the same pipeline without the ears.
    pub fn text(&self, text: &str, from: &str, to: &str, speak: bool) -> anyhow::Result<Turn> {
        let started = Instant::now();
        let (out, via) = self.translate(text, from, to)?;
        let speech = if speak { self.speak(to, &out) } else { None };
        Ok(Turn {
            id: self.next.fetch_add(1, Ordering::Relaxed),
            state: "spoken".into(),
            heard: Some(text.to_owned()),
            from: Some(from.into()),
            text: Some(out),
            to: Some(to.into()),
            via: Some(via),
            speech,
            ms: Some(elapsed_ms(started)),
            ..Turn::default()
        })
    }

    /// The harness: a WAV on the device as if it had just been said into the open Translator (its screen
    /// shows the turn as usual).
    pub fn hear_file(&self, hub: &Hub, wav: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(self.active(), "the Translator isn't open");
        let (samples, rate) = read_wav(wav)?;
        anyhow::ensure!(rate == 16_000, "it wants 16 kHz audio (got {rate} Hz)");
        self.listening(hub);
        // As a hold decides it (crate::voice): English push-to-talk's recognizer is sure of goes straight on.
        if self.english_side()
            && let Some(stt) = hub.voice.get().and_then(|v| v.stt())
        {
            let stream = stt.stream();
            stt.feed_stream(&stream, &samples);
            let text = stt.finish_stream(&stream);
            let sure = stt.confidence(&stream);
            tracing::info!(confidence = ?sure, "translator (harness): push-to-talk's recognizer");
            if !text.trim().is_empty() && sure.is_some_and(|(p, n)| p >= ENGLISH_SURE && n >= ENGLISH_MIN_TOKENS) {
                self.heard_english(hub, text);
                return Ok(());
            }
        }
        self.heard(hub, samples);
        Ok(())
    }

    /// The harness's benchmark: a WAV on the device through the whole pipeline, timed by stage.
    pub fn bench(&self, wav: &Path, mine: &str, theirs: &str, threads: Option<i32>) -> anyhow::Result<Value> {
        let (samples, rate) = read_wav(wav)?;
        anyhow::ensure!(rate == 16_000, "the benchmark wants 16 kHz audio (got {rate} Hz)");
        let t = Instant::now();
        let w = speech::Whisper::load(&self.root().join("asr"), threads.unwrap_or(i32::try_from(THREADS).unwrap_or(4)))?;
        let load_ms = elapsed_ms(t);
        let t = Instant::now();
        let heard = w.hear(&samples).ok_or_else(|| anyhow::anyhow!("whisper heard nothing"))?;
        let hear_ms = elapsed_ms(t);
        // Again, warm (as on the screen, where it's loaded and warmed when it opens).
        let t = Instant::now();
        let _ = w.hear(&samples);
        let hear_warm_ms = elapsed_ms(t);
        let to = if heard.lang == mine { theirs } else { mine };
        let t = Instant::now();
        let (text, via) = self.translate(&heard.text, &heard.lang, to)?;
        let translate_ms = elapsed_ms(t);
        let t = Instant::now();
        let speech = self.speak(to, &text);
        let speak_ms = elapsed_ms(t);
        #[allow(clippy::cast_precision_loss)]
        let audio_s = samples.len() as f32 / 16_000.0;
        Ok(json!({
            "audio_s": audio_s, "heard": heard.text, "from": heard.lang, "to": to, "text": text, "via": via,
            "speech": speech, "ms": { "load_whisper": load_ms, "hear": hear_ms, "hear_warm": hear_warm_ms, "translate": translate_ms, "speak": speak_ms },
        }))
    }
}

struct Hop {
    model: String,
    target: Option<String>,
}

fn elapsed_ms(t: Instant) -> u64 {
    u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A 16-bit PCM WAV's samples (the first channel) and rate.
fn read_wav(path: &Path) -> anyhow::Result<(Vec<f32>, u32)> {
    let b = std::fs::read(path)?;
    anyhow::ensure!(b.len() > 44 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE", "not a WAV file");
    let (mut rate, mut channels, mut bits) = (16_000u32, 1u16, 16u16);
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32::from_le_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]) as usize;
        let body = &b[at + 8..(at + 8 + len).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            anyhow::ensure!(bits == 16, "the benchmark wants 16-bit audio");
            let step = 2 * usize::from(channels.max(1));
            let samples = body.chunks_exact(step).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect();
            return Ok((samples, rate));
        }
        at += 8 + len + (len & 1);
    }
    anyhow::bail!("the WAV has no data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_go_direct_through_romance_or_through_english() {
        let dir = std::env::temp_dir().join(format!("r1d-translate-{}", std::process::id()));
        for m in ["en-ROMANCE", "ROMANCE-en", "en-de", "de-en", "ja-en"] {
            std::fs::create_dir_all(dir.join("mt").join(m)).unwrap();
            std::fs::write(dir.join("mt").join(m).join("decoder.onnx"), b"").unwrap();
        }
        let t = Translator::default();
        *t.root.lock().unwrap() = dir.clone();
        let names = |r: Option<Vec<Hop>>| r.map(|h| h.iter().map(|h| format!("{}{}", h.model, h.target.clone().unwrap_or_default())).collect::<Vec<_>>());
        assert_eq!(names(t.route("en", "de")), Some(vec!["en-de".to_owned()]));
        assert_eq!(names(t.route("en", "es")), Some(vec!["en-ROMANCE>>es<<".to_owned()]));
        assert_eq!(names(t.route("fr", "en")), Some(vec!["ROMANCE-en".to_owned()]));
        assert_eq!(names(t.route("de", "it")), Some(vec!["de-en".to_owned(), "en-ROMANCE>>it<<".to_owned()]));
        assert_eq!(names(t.route("ja", "de")), Some(vec!["ja-en".to_owned(), "en-de".to_owned()]));
        assert_eq!(names(t.route("en", "ja")), None, "English into Japanese is the agent's");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn wavs_round_trip() {
        let dir = std::env::temp_dir().join(format!("r1d-wav-{}.wav", std::process::id()));
        let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 100.0).sin() * 0.5).collect();
        std::fs::write(&dir, speech::wav(&samples, 16_000)).unwrap();
        let (back, rate) = read_wav(&dir).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(back.len(), samples.len());
        assert!((back[100] - samples[100]).abs() < 0.001);
        std::fs::remove_file(dir).ok();
    }
}
