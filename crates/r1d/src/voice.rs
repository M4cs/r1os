//! Push-to-talk (ADR-0009, docs/specs/r1d.md §voice): hold the side button, speak, let go.
//!
//! - **Capture starts on the button's down edge**, so the first syllable isn't lost while the
//!   gesture engine waits `hold_ms` to call it a hold. A tap or double tap throws that audio away.
//!   Audio is never written anywhere: it lives in the recognizer's buffers for one utterance.
//! - **`HoldStart`** sends the shell `voice: listening`. Partial transcripts follow whenever they
//!   change, plus the mic level (`voice_level`, ~15/s, kept out of the event log).
//! - **`HoldEnd`** finishes the utterance, after a short tail: people let go on their last word, so
//!   the mic keeps going for `TAIL`, then r1d stops it and waits `DRAIN` for audio still on its
//!   way over the socket. `voice: final` carries the text (empty if nothing was heard).
//! - **The wheel while the button is down** (ADR-0014): one tick is forgiven (a thumb brushing the
//!   wheel), the second cancels the utterance (`voice: cancelled`) and makes the press a volume
//!   change (`volume: on`, until the button comes up: `volume: off`). Max, 2026-10-09: "not close
//!   out of the PTT if we hold down and only scroll one time".
//!
//! While the Translator's screen is open (crate::translator, ADR-0034), a hold is its own: the audio is
//! kept for its multilingual recognizer, this one listens too when a side is English (its fast path), and
//! `translation` events take the place of `voice` ones. While the Journal's is (crate::notes, ADR-0037), a
//! hold is a voice note: audio kept, words recognised, `note` events. System One (crate::system1) reads the
//! partials otherwise.
//!
//! The mic belongs to the host app, since Android only lets a foreground app record. The host
//! connects to r1d on 127.0.0.1:7173, sends `r1os-mic 1 16000\n`, then streams 16 kHz mono s16le
//! while r1d has sent `S` (start), until `E` (end). The simulator's page does the same over the
//! `/api/mic` WebSocket ("start"/"stop" text, PCM as binary).
//!
//! Speech-to-text is sherpa-onnx's streaming transducer, loaded once at startup from
//! `<data>/models/stt` (encoder, decoder, joiner `.onnx` and `tokens.txt`; `tools/fetch-speech.sh`).
//! Loading takes ~35 s on the R1, on its own thread; a hold meanwhile gets an immediate "warming
//! up" error rather than waiting. One worker thread runs utterances; the async side only sends it
//! commands.

use crate::gesture::GestureKind;
use crate::hub::{Event, Hub, Source};
use serde_json::{Value, json};
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc as smpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc};

pub const MIC_ADDR: &str = "127.0.0.1:7173";
pub const RATE: i32 = 16_000;
const MIC_HELLO: &str = "r1os-mic 1 16000";
/// A hold longer than this is finished for you (a stuck button shouldn't record forever).
const MAX_UTTERANCE_S: usize = 60;
const LEVEL_EVERY: Duration = Duration::from_millis(66);
/// Keep listening this long after the button comes up (letting go on the last word cut it off).
const TAIL: Duration = Duration::from_millis(400);
/// Unless you'd finished: quiet this long when the button came up, and it listens only [`SHORT_TAIL`] more (T-042).
const FINISHED: Duration = Duration::from_millis(350);
const SHORT_TAIL: Duration = Duration::from_millis(60);
/// Audio this loud (on [`level`]'s scale, −55..−10 dBFS → 0..1) is taken as speech: about −44 dBFS.
const SPEAKING: f32 = 0.25;
/// Then wait for audio already on its way from the mic before finishing.
const DRAIN: Duration = Duration::from_millis(80);
const THREADS: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicCmd {
    Start,
    Stop,
}

/// Who wants the mic on: push-to-talk, the Recorder (crate::recorder). It's on while anyone does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    Talk = 1,
    Recorder = 2,
}

enum Cmd {
    /// The button went down: open a stream and buffer (the gesture isn't known yet).
    Begin,
    Audio(Vec<f32>),
    /// It's a hold: tell the shell, and stream partials from here on.
    Listen,
    Finish,
    Cancel,
}

/// A whole-file transcription (the harness's benchmark).
pub struct Transcript {
    pub text: String,
    pub audio_s: f32,
    pub compute: Duration,
    /// Each partial with the audio time it appeared at: how far the words lag the voice.
    pub partials: Vec<(f32, String)>,
}

/// The loaded recognizer.
pub struct Stt {
    rec: OnlineRecognizer,
    pub model: String,
}

impl Stt {
    pub fn load(dir: &Path, threads: i32) -> anyhow::Result<Self> {
        let pick = |stem: &str| -> anyhow::Result<String> {
            // Prefer ONNX Runtime's pre-optimized format (no graph optimization at load), then an
            // int8 export (smaller, faster on the A53), then the plain model.
            for name in [format!("{stem}.int8.ort"), format!("{stem}.ort"), format!("{stem}.int8.onnx"), format!("{stem}.onnx")] {
                let p = dir.join(&name);
                if p.is_file() {
                    return Ok(p.to_string_lossy().into_owned());
                }
            }
            anyhow::bail!("no {stem}.onnx in {}", dir.display())
        };
        let tokens = dir.join("tokens.txt");
        anyhow::ensure!(tokens.is_file(), "no tokens.txt in {}", dir.display());
        let mut cfg = OnlineRecognizerConfig::default();
        cfg.model_config.transducer.encoder = Some(pick("encoder")?);
        cfg.model_config.transducer.decoder = Some(pick("decoder")?);
        cfg.model_config.transducer.joiner = Some(pick("joiner")?);
        cfg.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
        cfg.model_config.num_threads = threads;
        cfg.decoding_method = Some("greedy_search".into());
        let rec = OnlineRecognizer::create(&cfg).ok_or_else(|| anyhow::anyhow!("sherpa-onnx couldn't load {}", dir.display()))?;
        let model = std::fs::read_to_string(dir.join("MODEL")).map(|s| s.trim().to_owned()).unwrap_or_else(|_| {
            dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        });
        Ok(Self { rec, model })
    }

    /// A stream of its own (the Recorder's), decoded alongside push-to-talk's.
    pub fn stream(&self) -> OnlineStream {
        self.rec.create_stream()
    }

    pub fn feed_stream(&self, stream: &OnlineStream, samples: &[f32]) {
        self.feed(stream, RATE, samples);
    }

    pub fn text_of(&self, stream: &OnlineStream) -> String {
        self.text(stream)
    }

    pub fn finish_stream(&self, stream: &OnlineStream) -> String {
        self.finish(stream, RATE)
    }

    fn text(&self, stream: &OnlineStream) -> String {
        self.rec.get_result(stream).map(|r| tidy(&r.text)).unwrap_or_default()
    }

    /// How sure the recognizer is of what it heard: the mean log-probability of its tokens (`ys_probs`, 0 is
    /// certain), and how many there were. English sounds high; another language heard as English words,
    /// low: the Translator's test for skipping Whisper (crate::translator, ADR-0034). The crate's result
    /// type drops it, so it's read from the C library's JSON.
    pub fn confidence(&self, stream: &OnlineStream) -> Option<(f32, usize)> {
        // SAFETY: both crate types are a single raw pointer to their C object (checked at compile time
        // below), and the JSON string is freed by the library's own call.
        const _: () = assert!(std::mem::size_of::<OnlineRecognizer>() == std::mem::size_of::<*const u8>());
        const _: () = assert!(std::mem::size_of::<OnlineStream>() == std::mem::size_of::<*const u8>());
        let json = unsafe {
            let rec = *std::ptr::from_ref(&self.rec).cast::<*const sherpa_onnx_sys::OnlineRecognizer>();
            let st = *std::ptr::from_ref(stream).cast::<*const sherpa_onnx_sys::OnlineStream>();
            let c = sherpa_onnx_sys::SherpaOnnxGetOnlineStreamResultAsJson(rec, st);
            if c.is_null() {
                return None;
            }
            let s = std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned();
            sherpa_onnx_sys::SherpaOnnxDestroyOnlineStreamResultJson(c);
            s
        };
        let v: Value = serde_json::from_str(&json).ok()?;
        let probs: Vec<f32> = v["ys_probs"].as_array()?.iter().filter_map(Value::as_f64).map(|p| p as f32).collect();
        if probs.is_empty() {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        Some((probs.iter().sum::<f32>() / probs.len() as f32, probs.len()))
    }

    fn feed(&self, stream: &OnlineStream, rate: i32, samples: &[f32]) {
        stream.accept_waveform(rate, samples); // sherpa-onnx resamples anything that isn't 16 kHz
        while self.rec.is_ready(stream) {
            self.rec.decode(stream);
        }
    }

    fn finish(&self, stream: &OnlineStream, rate: i32) -> String {
        // A little trailing silence flushes the encoder's right context, so the last word lands.
        stream.accept_waveform(rate, &vec![0.0; usize::try_from(rate).unwrap_or(16_000) * 3 / 10]);
        stream.input_finished();
        while self.rec.is_ready(stream) {
            self.rec.decode(stream);
        }
        self.text(stream)
    }

    /// Transcribes a whole WAV file, chunk by chunk as if it were live.
    pub fn transcribe(&self, path: &str) -> anyhow::Result<Transcript> {
        let wave = sherpa_onnx::Wave::read(path).ok_or_else(|| anyhow::anyhow!("can't read {path}"))?;
        let rate = wave.sample_rate();
        let t = Instant::now();
        let stream = self.rec.create_stream();
        let step = usize::try_from(rate / 10).unwrap_or(1600).max(1);
        let mut partials: Vec<(f32, String)> = Vec::new();
        for (i, chunk) in wave.samples().chunks(step).enumerate() {
            self.feed(&stream, rate, chunk);
            let text = self.text(&stream);
            if partials.last().is_none_or(|(_, last)| *last != text) && !text.is_empty() {
                #[allow(clippy::cast_precision_loss)]
                partials.push((((i + 1) * step) as f32 / rate as f32, text));
            }
        }
        let text = self.finish(&stream, rate);
        #[allow(clippy::cast_precision_loss)]
        let secs = wave.num_samples() as f32 / rate as f32;
        Ok(Transcript { text, audio_s: secs, compute: t.elapsed(), partials })
    }
}

/// Models emit all caps without punctuation; make it read like a message.
fn tidy(raw: &str) -> String {
    let s = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().any(char::is_lowercase) {
        return s;
    }
    let lower = s.to_lowercase();
    let words: Vec<String> = lower
        .split(' ')
        .map(|w| match w {
            "i" | "i'm" | "i'll" | "i've" | "i'd" => {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
            _ => w.to_owned(),
        })
        .collect();
    let mut out = words.join(" ");
    if let Some(f) = out.chars().next() {
        out.replace_range(..f.len_utf8(), &f.to_uppercase().to_string());
    }
    out
}

/// A meter's level for a room, not a mouth at the mic (the Recorder, recordings' peaks): RMS → dBFS,
/// −66..−18 dB mapped to 0..1. Push-to-talk's −55 dB floor left a voice across a room at zero (2026-10-10).
pub fn meter(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ms = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    let db = 10.0 * (ms + 1e-12).log10();
    ((db + 66.0) / 48.0).clamp(0.0, 1.0)
}

/// mic level for the shell's animation: RMS → dBFS, −55..−10 dB mapped to 0..1.
pub fn level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ms = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    let db = 10.0 * (ms + 1e-10).log10();
    ((db + 55.0) / 45.0).clamp(0.0, 1.0)
}

pub struct Voice {
    worker: Mutex<smpsc::Sender<Cmd>>,
    /// Whoever owns the mic right now (the host app, or the simulator page).
    mic: Mutex<Option<mpsc::UnboundedSender<MicCmd>>>,
    /// Who wants it on ([`Holder`] bits).
    holders: Mutex<u8>,
    /// The Recorder's copy of the audio, while it records.
    tap: Mutex<Option<smpsc::Sender<Vec<f32>>>>,
    stt: Arc<Mutex<SttState>>,
    /// When the mic last heard something as loud as speech (ms after `born`), as it arrives: before the recognizer's
    /// queue, which can lag.
    born: Instant,
    loud: std::sync::atomic::AtomicU64,
}

enum SttState {
    Loading,
    Ready(Arc<Stt>),
    Missing(String),
}

impl Voice {
    /// Starts loading the model, the worker and the button controller.
    pub fn start(hub: Arc<Hub>, model_dir: PathBuf) -> Arc<Self> {
        let (tx, rx) = smpsc::channel();
        let stt = Arc::new(Mutex::new(SttState::Loading));
        let voice = Arc::new(Self {
            worker: Mutex::new(tx),
            mic: Mutex::default(),
            holders: Mutex::default(),
            tap: Mutex::default(),
            stt: stt.clone(),
            born: Instant::now(),
            loud: std::sync::atomic::AtomicU64::new(0),
        });
        let state = stt.clone();
        let loading = hub.clone();
        std::thread::Builder::new()
            .name("stt-load".into())
            .spawn(move || {
                let t = Instant::now();
                *state.lock().unwrap() = match Stt::load(&model_dir, THREADS) {
                    Ok(s) => {
                        tracing::info!(model = %s.model, ms = t.elapsed().as_millis(), "speech-to-text ready");
                        loading.perf.mark("stt.load", t.elapsed());
                        SttState::Ready(Arc::new(s))
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "no speech-to-text; push-to-talk will say so");
                        SttState::Missing(e.to_string())
                    }
                };
            })
            .expect("spawn the stt loader");
        let h = hub.clone();
        std::thread::Builder::new()
            .name("stt".into())
            .spawn(move || worker(&h, &stt, &rx))
            .expect("spawn the stt worker");
        tokio::spawn(controller(hub, voice.clone()));
        voice
    }

    pub fn status(&self) -> Value {
        let mic = self.mic.lock().unwrap().is_some();
        match &*self.stt.lock().unwrap() {
            SttState::Loading => json!({ "stt": "loading", "mic": mic }),
            SttState::Ready(s) => json!({ "stt": "ready", "model": s.model, "mic": mic }),
            SttState::Missing(e) => json!({ "stt": "missing", "error": e, "mic": mic }),
        }
    }

    pub fn stt(&self) -> Option<Arc<Stt>> {
        match &*self.stt.lock().unwrap() {
            SttState::Ready(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.worker.lock().unwrap().send(cmd);
    }

    fn mic(&self, cmd: MicCmd) {
        if let Some(m) = &*self.mic.lock().unwrap() {
            let _ = m.send(cmd);
        }
    }

    /// `who` wants the mic on (or not): it starts when the first does and stops when the last doesn't.
    pub fn hold_mic(&self, who: Holder, on: bool) {
        let mut h = self.holders.lock().unwrap();
        let before = *h;
        if on {
            *h |= who as u8;
        } else {
            *h &= !(who as u8);
        }
        match (before, *h) {
            (0, after) if after != 0 => self.mic(MicCmd::Start),
            (b, 0) if b != 0 => self.mic(MicCmd::Stop),
            _ => {}
        }
    }

    /// Where the Recorder gets its copy of the audio (`None`: it stopped).
    pub fn tap(&self, to: Option<smpsc::Sender<Vec<f32>>>) {
        *self.tap.lock().unwrap() = to;
    }

    /// How long the mic has heard nothing as loud as speech.
    fn quiet_for(&self) -> Duration {
        let loud = Duration::from_millis(self.loud.load(std::sync::atomic::Ordering::Relaxed));
        self.born.elapsed().saturating_sub(loud)
    }

    /// Audio from the mic's owner, 16 kHz mono.
    pub fn audio(&self, samples: Vec<f32>) {
        if level(&samples) >= SPEAKING {
            let ms = u64::try_from(self.born.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.loud.store(ms, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(t) = &*self.tap.lock().unwrap() {
            let _ = t.send(samples.clone());
        }
        self.send(Cmd::Audio(samples));
    }

    /// A mic owner connected; it gets start/stop commands until it disconnects (or another takes over).
    pub fn attach_mic(&self) -> mpsc::UnboundedReceiver<MicCmd> {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.mic.lock().unwrap() = Some(tx);
        rx
    }

    /// The owner went away. Only clears it if no newer owner took over meanwhile.
    pub fn detach_mic(&self) {
        let mut m = self.mic.lock().unwrap();
        if m.as_ref().is_some_and(mpsc::UnboundedSender::is_closed) {
            *m = None;
        }
    }

    pub fn has_mic(&self) -> bool {
        self.mic.lock().unwrap().as_ref().is_some_and(|m| !m.is_closed())
    }
}

/// Maps button edges and gestures onto the worker (see the module docs).
async fn controller(hub: Arc<Hub>, voice: Arc<Voice>) {
    let mut rx = hub.subscribe();
    let mut down = false;
    let mut listening = false;
    // Wheel ticks during this press, and whether it became a volume change.
    let mut ticks = 0u32;
    let mut volume = false;
    let mut tail: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        match ev.event {
            Event::Button { down: true } => {
                down = true;
                ticks = 0;
                volume = false;
                // A new press during the last utterance's tail: finish that one now.
                if let Some(t) = tail.take().filter(|t| !t.is_finished()) {
                    t.abort();
                    voice.send(Cmd::Finish);
                }
                // Not during onboarding: the shell is busy with setup, and nothing listens. Nor while the PIN
                // lock is on (ADR-0016: "No talking while locked"): the mic doesn't even open.
                if hub.core.onboarding() == "done" && !hub.pin.locked() {
                    voice.hold_mic(Holder::Talk, true);
                    voice.send(Cmd::Begin);
                }
            }
            Event::Button { down: false } => {
                down = false;
                if volume {
                    volume = false;
                    hub.emit(Event::Volume { on: false }, Source::Hw);
                }
                if !listening {
                    voice.hold_mic(Holder::Talk, false); // a hold keeps the mic through its tail
                }
            }
            // A press already turned into a volume change doesn't start listening.
            Event::Gesture { kind: GestureKind::HoldStart, .. } if volume => {}
            Event::Gesture { kind: GestureKind::HoldStart, .. } if hub.pin.locked() => {
                hub.emit(voice_event("error", Some("Unlock first: turn up and dial your PIN.")), Source::Api);
            }
            Event::Gesture { kind: GestureKind::HoldStart, .. } => {
                listening = true;
                if !voice.has_mic() {
                    hub.emit(voice_event("error", Some("No microphone yet: the host app isn't connected.")), Source::Api);
                }
                voice.send(Cmd::Listen);
            }
            Event::Gesture { kind: GestureKind::HoldEnd, .. } if listening => {
                listening = false;
                let v = voice.clone();
                // You'd already finished talking: no need to keep listening for a last word (T-042).
                let wait = if voice.quiet_for() >= FINISHED { SHORT_TAIL } else { TAIL };
                hub.perf.released(wait == SHORT_TAIL);
                tail = Some(tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    v.hold_mic(Holder::Talk, false);
                    tokio::time::sleep(DRAIN).await;
                    v.send(Cmd::Finish);
                }));
            }
            // A tap or double tap (button up again): the buffered audio was never speech for us.
            Event::Gesture { kind: GestureKind::Tap | GestureKind::DoubleTap | GestureKind::TripleTap, .. } if !down => voice.send(Cmd::Cancel),
            // Touch's cancel (T-156, docked): what was heard goes, as two turns of the wheel drop it, with no volume.
            Event::TalkCancel if listening || down => {
                listening = false;
                voice.hold_mic(Holder::Talk, false);
                voice.send(Cmd::Cancel);
            }
            Event::Wheel { .. } if down => {
                ticks += 1;
                if ticks >= 2 && !volume {
                    volume = true;
                    listening = false;
                    voice.hold_mic(Holder::Talk, false);
                    voice.send(Cmd::Cancel);
                    hub.emit(Event::Volume { on: true }, Source::Hw);
                }
            }
            _ => {}
        }
    }
}

fn voice_event(state: &str, text: Option<&str>) -> Event {
    Event::Voice { state: state.into(), text: text.map(str::to_owned), intent: None }
}

/// A partial: to the shell, and to System One (crate::system1).
fn partial(hub: &Hub, text: &str, source: Source) {
    hub.system1.partial(text);
    hub.emit(voice_event("partial", Some(text)), source);
}

/// The final words, with what System One makes of them.
fn final_words(hub: &Hub, text: &str, source: Source) {
    hub.perf.final_words();
    hub.system1.partial("");
    let intent = hub.system1.decide(hub, text).map(|i| serde_json::to_value(i).unwrap_or_default());
    if let Some(i) = &intent {
        tracing::info!(intent = %i, "system one");
    }
    hub.emit(Event::Voice { state: "final".into(), text: Some(text.to_owned()), intent }, source);
}

struct Utterance {
    /// For the Translator (its screen is open, crate::translator): the audio is kept for it, and the
    /// English recognizer stays out of it.
    translate: bool,
    /// A Journal voice note (its screen is open, crate::notes): the audio is kept, the words recognised as
    /// usual, and `note` events go out instead of `voice` ones.
    note: bool,
    audio: Vec<f32>,
    /// The recognizer as it was when the button went down (None: still loading, or missing).
    stt: Option<Arc<Stt>>,
    stream: Option<OnlineStream>,
    listening: bool,
    text: String,
    samples: usize,
    last_level: Instant,
}

/// Runs one utterance at a time.
fn worker(hub: &Hub, state: &Mutex<SttState>, rx: &smpsc::Receiver<Cmd>) {
    let mut utt: Option<Utterance> = None;
    for cmd in rx {
        match cmd {
            Cmd::Begin => {
                let translate = hub.translator.active();
                let note = !translate && hub.notes.active();
                // The Translator keeps this recognizer only when one side is English (its fast path).
                let stt = match &*state.lock().unwrap() {
                    SttState::Ready(s) if !translate || hub.translator.english_side() => Some(s.clone()),
                    _ => None,
                };
                utt = Some(Utterance {
                    translate,
                    note,
                    audio: Vec::new(),
                    stream: stt.as_ref().map(|s| s.rec.create_stream()),
                    stt,
                    listening: false,
                    text: String::new(),
                    samples: 0,
                    last_level: Instant::now(),
                });
            }
            Cmd::Audio(samples) => {
                let Some(u) = utt.as_mut() else { continue };
                u.samples += samples.len();
                if u.translate && u.audio.len() < crate::translator::speech::MAX_HEARD_S * RATE as usize {
                    u.audio.extend_from_slice(&samples);
                }
                if u.note {
                    u.audio.extend_from_slice(&samples);
                }
                if let (Some(s), Some(stream)) = (&u.stt, &u.stream) {
                    s.feed(stream, RATE, &samples);
                    if u.listening {
                        let text = s.text(stream);
                        if text != u.text {
                            u.text = text;
                            if u.note {
                                crate::notes::Notes::emit(hub, crate::notes::Note { state: "partial".into(), text: Some(u.text.clone()), ..Default::default() });
                            } else if u.translate {
                                // Shown only once it sounds like English: another language heard as English
                                // words would be noise.
                                if s.confidence(stream).is_some_and(|(p, n)| p >= crate::translator::ENGLISH_SURE && n >= 2) {
                                    hub.translator.partial(hub, &u.text);
                                }
                            } else {
                                partial(hub, &u.text, Source::Hw);
                            }
                        }
                    }
                }
                if u.listening && u.last_level.elapsed() >= LEVEL_EVERY {
                    u.last_level = Instant::now();
                    hub.emit(Event::VoiceLevel { level: level(&samples) }, Source::Hw);
                }
                let cap = if u.note { crate::notes::MAX_NOTE_S } else { MAX_UTTERANCE_S };
                if u.samples > cap * RATE as usize {
                    finish(hub, utt.take());
                }
            }
            Cmd::Listen => {
                let Some(u) = utt.as_mut() else { continue };
                u.listening = true;
                if u.translate {
                    hub.translator.listening(hub);
                    continue;
                }
                if u.note {
                    crate::notes::Notes::emit(hub, crate::notes::Note { state: "listening".into(), ..Default::default() });
                    continue;
                }
                if u.stt.is_none() {
                    let why = match &*state.lock().unwrap() {
                        SttState::Loading => "Still warming up my ears. Try again in a few seconds.",
                        _ => "Speech recognition isn't installed yet (tools/fetch-speech.sh).",
                    };
                    hub.emit(voice_event("error", Some(why)), Source::Api);
                    utt = None;
                    continue;
                }
                hub.emit(voice_event("listening", None), Source::Hw);
                if let (Some(s), Some(stream)) = (&u.stt, &u.stream) {
                    u.text = s.text(stream);
                    if !u.text.is_empty() {
                        partial(hub, &u.text, Source::Hw);
                    }
                }
            }
            Cmd::Finish => finish(hub, utt.take()),
            Cmd::Cancel => match utt.take() {
                Some(u) if u.listening && u.translate => hub.translator.cancelled(hub),
                Some(u) if u.listening && u.note => crate::notes::Notes::emit(hub, crate::notes::Note { state: "cancelled".into(), ..Default::default() }),
                Some(u) if u.listening => {
                    hub.system1.partial("");
                    hub.emit(voice_event("cancelled", None), Source::Hw);
                }
                _ => {}
            },
        }
    }
}

fn finish(hub: &Hub, utt: Option<Utterance>) {
    let Some(u) = utt else { return };
    if !u.listening {
        return;
    }
    if u.translate {
        // English the recognizer is sure of goes straight on; anything else to Whisper.
        if let (Some(s), Some(stream)) = (&u.stt, &u.stream) {
            let text = s.finish(stream, RATE);
            let sure = s.confidence(stream);
            tracing::info!(confidence = ?sure, chars = text.len(), "translator: push-to-talk's recognizer");
            if !text.trim().is_empty()
                && sure.is_some_and(|(p, n)| p >= crate::translator::ENGLISH_SURE && n >= crate::translator::ENGLISH_MIN_TOKENS)
            {
                hub.translator.heard_english(hub, text);
                return;
            }
        }
        hub.translator.heard(hub, u.audio);
        return;
    }
    if u.note {
        let text = match (&u.stt, &u.stream) {
            (Some(s), Some(stream)) => s.finish(stream, RATE),
            _ => u.text.clone(),
        };
        crate::notes::Notes::save(hub, &u.audio, &text);
        return;
    }
    let text = match (&u.stt, &u.stream) {
        (Some(s), Some(stream)) => s.finish(stream, RATE),
        _ => u.text,
    };
    #[allow(clippy::cast_precision_loss)]
    let secs = u.samples as f32 / RATE as f32;
    tracing::info!(audio_s = secs, chars = text.len(), "utterance");
    final_words(hub, &text, Source::Hw);
}

/// Accepts the host app's mic connection (module docs). One owner at a time; the newest wins.
/// `token`: the per-boot token the host's hello must carry (ADR-0026); `None` in the simulator.
pub async fn serve_host_mic(voice: Arc<Voice>, token: Option<String>) {
    let listener = match tokio::net::TcpListener::bind(MIC_ADDR).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(addr = MIC_ADDR, error = %e, "can't listen for the mic");
            return;
        }
    };
    loop {
        let Ok((sock, peer)) = listener.accept().await else { continue };
        let voice = voice.clone();
        let token = token.clone();
        tokio::spawn(async move {
            if let Err(e) = host_mic(&voice, sock, token.as_deref()).await {
                tracing::info!(%peer, error = %e, "mic connection ended");
            }
            voice.detach_mic();
        });
    }
}

async fn host_mic(voice: &Voice, sock: tokio::net::TcpStream, token: Option<&str>) -> anyhow::Result<()> {
    let (rd, mut wr) = sock.into_split();
    let mut rd = BufReader::new(rd);
    let mut hello = String::new();
    tokio::time::timeout(Duration::from_secs(5), rd.read_line(&mut hello)).await??;
    // `r1os-mic 1 16000 <token>`: any local process can reach this port, and only the host may be the mic.
    let hello = hello.trim();
    let (head, given) = hello.rsplit_once(' ').unwrap_or((hello, ""));
    let ok = match token {
        Some(t) => head == MIC_HELLO && given == t,
        None => hello == MIC_HELLO || head == MIC_HELLO,
    };
    anyhow::ensure!(ok, "the mic's hello wasn't the host's");
    tracing::info!("host mic connected");
    let mut cmds = voice.attach_mic();
    let mut buf = vec![0u8; 3200]; // 100 ms
    let mut carry: Option<u8> = None;
    loop {
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return Ok(()) }; // another owner took over
                wr.write_all(if cmd == MicCmd::Start { b"S" } else { b"E" }).await?;
            }
            n = rd.read(&mut buf) => {
                let n = n?;
                if n == 0 {
                    return Ok(());
                }
                voice.audio(pcm16(&buf[..n], &mut carry));
            }
        }
    }
}

/// s16le bytes → f32 samples. `carry` holds an odd byte between reads.
pub fn pcm16(bytes: &[u8], carry: &mut Option<u8>) -> Vec<f32> {
    let mut out = Vec::with_capacity(bytes.len() / 2 + 1);
    let mut it = bytes.iter().copied();
    if let Some(lo) = carry.take() {
        if let Some(hi) = it.next() {
            out.push(f32::from(i16::from_le_bytes([lo, hi])) / 32768.0);
        } else {
            *carry = Some(lo);
            return out;
        }
    }
    loop {
        match (it.next(), it.next()) {
            (Some(lo), Some(hi)) => out.push(f32::from(i16::from_le_bytes([lo, hi])) / 32768.0),
            (Some(lo), None) => {
                *carry = Some(lo);
                break;
            }
            _ => break,
        }
    }
    out
}

/// The harness's scripted utterance: the same events a real hold produces, word by word.
pub async fn say(hub: Arc<Hub>, text: String, ms_per_word: u64) {
    let words: Vec<&str> = text.split_whitespace().collect();
    hub.emit(voice_event("listening", None), Source::Sim);
    let mut so_far = String::new();
    for (i, w) in words.iter().enumerate() {
        for k in 0..3u32 {
            tokio::time::sleep(Duration::from_millis(ms_per_word / 3)).await;
            #[allow(clippy::cast_precision_loss)]
            let lvl = 0.35 + 0.5 * (((i as f32) * 1.7 + k as f32).sin().abs());
            hub.emit(Event::VoiceLevel { level: lvl }, Source::Sim);
        }
        if i > 0 {
            so_far.push(' ');
        }
        so_far.push_str(w);
        partial(&hub, &so_far, Source::Sim);
    }
    tokio::time::sleep(Duration::from_millis(350)).await;
    final_words(&hub, &so_far, Source::Sim);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_sentence_cases_all_caps() {
        assert_eq!(tidy("WHAT TIME IS IT"), "What time is it");
        assert_eq!(tidy("CAN I GET A  TIMER"), "Can I get a timer");
        assert_eq!(tidy("I'M HUNGRY"), "I'm hungry");
        assert_eq!(tidy("Already fine."), "Already fine.");
        assert_eq!(tidy(""), "");
    }

    /// How sure Kroko is of English and of other languages (`R1D_STT_DIR`, and `R1D_WAVS`: 16 kHz WAVs,
    /// `;`-separated): `cargo test -p r1d kroko_confidence -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the speech model and clips"]
    fn kroko_confidence() {
        let stt = Stt::load(std::path::Path::new(&std::env::var("R1D_STT_DIR").unwrap()), 4).unwrap();
        for wav in std::env::var("R1D_WAVS").unwrap().split(';') {
            let w = sherpa_onnx::Wave::read(wav).unwrap();
            let stream = stt.rec.create_stream();
            stt.feed(&stream, w.sample_rate(), w.samples());
            let text = stt.finish(&stream, w.sample_rate());
            println!("{wav}: {text:?} {:?}", stt.confidence(&stream));
        }
    }

    #[test]
    fn pcm16_carries_odd_bytes() {
        let mut carry = None;
        let a = pcm16(&[0x00, 0x40, 0xff], &mut carry); // 16384, then half a sample
        assert_eq!(a, vec![0.5]);
        assert_eq!(carry, Some(0xff));
        let b = pcm16(&[0xff], &mut carry); // completes -1
        assert_eq!(b, vec![-1.0 / 32768.0]);
        assert_eq!(carry, None);
    }

    #[test]
    fn level_maps_silence_and_loud() {
        assert!(level(&[0.0; 160]) < 0.01);
        assert!(level(&[0.5; 160]) > 0.9);
    }
}
