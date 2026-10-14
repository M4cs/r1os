//! Speaking on the R1 (roadmap R3, Q14): text to speech with sherpa-onnx, which r1d already links for
//! hearing. One [`Voice`] loads any of the voice packages sherpa-onnx publishes, by what's in its
//! directory:
//!
//! | Kind | Files |
//! |---|---|
//! | `supertonic` | `tts.json`, `voice.bin`, `duration_predictor*.onnx`, `text_encoder*.onnx`, `vector_estimator*.onnx`, `vocoder*.onnx`, `unicode_indexer.bin` |
//! | `pocket` (Kyutai's) | `lm_main*.onnx`, `lm_flow*.onnx`, `encoder.onnx`, `decoder*.onnx`, `text_conditioner.onnx`, `vocab.json`, `token_scores.json`, and a reference clip of the voice |
//! | `kokoro` | `voices.bin`, a model, `tokens.txt`, `espeak-ng-data/` (+ `lexicon-us-en.txt`, `dict/` for v1.x) |
//! | `kitten` | `voices.bin`, a model, `tokens.txt`, `espeak-ng-data/` |
//! | `matcha` | `model-steps-*.onnx`, a `vocos*.onnx` vocoder, `tokens.txt`, `espeak-ng-data/` |
//! | `vits` (Piper and others) | one model, `tokens.txt`, `espeak-ng-data/` |
//!
//! sherpa-onnx ends the process rather than failing when a package's pieces are missing (the Translator
//! learned that, 2026-10-10), so [`Voice::load`] checks for them first. `/harness/tts` times them on the R1.

use serde::Serialize;
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// How a voice speaks: which of a package's speakers, how fast, and (for the models that take them)
/// steps, language and a reference clip.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The speaker in a multi-speaker package (Kokoro, Kitten, Supertonic, some Piper voices).
    pub sid: i32,
    /// 1.0 is the voice's own pace; larger is faster.
    pub speed: Option<f32>,
    /// Flow-matching steps (Supertonic, Matcha): fewer is faster, more is cleaner.
    pub steps: Option<i32>,
    /// Supertonic's language (default `en`).
    pub lang: Option<String>,
    /// Pocket's voice: a short WAV of the speaker (default: the package's first test clip).
    pub reference: Option<PathBuf>,
    /// ONNX Runtime's execution provider: `cpu` (default), `xnnpack` or `nnapi` (the R1's build has all three).
    pub provider: Option<String>,
}

pub struct Voice {
    tts: OfflineTts,
    pub kind: &'static str,
    config: GenerationConfig,
}

/// One sentence spoken: its samples, and how long they took to make.
pub struct Spoken {
    pub samples: Vec<f32>,
    pub rate: i32,
    pub ms: u64,
}

fn s(p: &Path) -> Option<String> {
    Some(p.to_string_lossy().into_owned())
}

/// The package's file for `stem` (`model`, `decoder`, …): its int8 build if there is one, else the plain one.
fn pick(dir: &Path, stem: &str) -> Option<PathBuf> {
    [format!("{stem}.int8.onnx"), format!("{stem}.onnx")].iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

fn need(p: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(p.exists(), "the voice has no {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Ok(p.to_string_lossy().into_owned())
}

/// Which kind of package `dir` holds.
pub fn kind_of(dir: &Path) -> anyhow::Result<&'static str> {
    let has = |n: &str| dir.join(n).exists();
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let fs = files(dir);
    let any = |prefix: &str| fs.iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)));
    Ok(if has("tts.json") && has("voice.bin") {
        "supertonic"
    } else if any("lm_main") {
        "pocket"
    } else if has("voices.bin") && (name.contains("kokoro") || has("lexicon-us-en.txt")) {
        "kokoro"
    } else if has("voices.bin") {
        "kitten"
    } else if any("model-steps-") {
        "matcha"
    } else if has("tokens.txt") && fs.iter().any(|p| p.extension().is_some_and(|x| x == "onnx")) {
        "vits"
    } else {
        anyhow::bail!("{} isn't a voice package sherpa-onnx can speak with", dir.display())
    })
}

impl Voice {
    /// Loads the voice package in `dir` with `threads` threads.
    pub fn load(dir: &Path, threads: i32, o: &Options) -> anyhow::Result<Voice> {
        let kind = kind_of(dir)?;
        let espeak = dir.join("espeak-ng-data");
        let mut cfg = OfflineTtsConfig::default();
        let mut config = GenerationConfig { sid: o.sid, ..GenerationConfig::default() };
        if let Some(sp) = o.speed {
            config.speed = sp;
        }
        if let Some(n) = o.steps {
            config.num_steps = n;
        }
        let m = &mut cfg.model;
        match kind {
            "supertonic" => {
                let part = |stem: &str| pick(dir, stem).ok_or_else(|| anyhow::anyhow!("the voice has no {stem}"));
                m.supertonic.duration_predictor = s(&part("duration_predictor")?);
                m.supertonic.text_encoder = s(&part("text_encoder")?);
                m.supertonic.vector_estimator = s(&part("vector_estimator")?);
                m.supertonic.vocoder = s(&part("vocoder")?);
                m.supertonic.tts_json = Some(need(&dir.join("tts.json"))?);
                m.supertonic.unicode_indexer = Some(need(&dir.join("unicode_indexer.bin"))?);
                m.supertonic.voice_style = Some(need(&dir.join("voice.bin"))?);
                let lang = o.lang.clone().unwrap_or_else(|| "en".into());
                config.extra = Some(HashMap::from([("lang".to_owned(), serde_json::Value::String(lang))]));
            }
            "pocket" => {
                let part = |stem: &str| pick(dir, stem).ok_or_else(|| anyhow::anyhow!("the voice has no {stem}"));
                m.pocket.lm_main = s(&part("lm_main")?);
                m.pocket.lm_flow = s(&part("lm_flow")?);
                m.pocket.encoder = s(&part("encoder")?);
                m.pocket.decoder = s(&part("decoder")?);
                m.pocket.text_conditioner = s(&part("text_conditioner")?);
                m.pocket.vocab_json = Some(need(&dir.join("vocab.json"))?);
                m.pocket.token_scores_json = Some(need(&dir.join("token_scores.json"))?);
                m.pocket.voice_embedding_cache_capacity = 4;
                let clip = match &o.reference {
                    Some(r) if r.is_absolute() => r.clone(),
                    Some(r) => dir.join(r),
                    None => files(&dir.join("test_wavs")).into_iter().find(|p| p.extension().is_some_and(|x| x == "wav")).unwrap_or_default(),
                };
                let (audio, rate) = read_wav(&clip)?;
                config.reference_audio = Some(audio);
                config.reference_sample_rate = i32::try_from(rate).unwrap_or(24_000);
            }
            "kokoro" | "kitten" => {
                let model = pick(dir, "model").ok_or_else(|| anyhow::anyhow!("the voice has no model"))?;
                anyhow::ensure!(espeak.join("phontab").is_file(), "the voice has no espeak-ng data");
                let (voices, tokens) = (need(&dir.join("voices.bin"))?, need(&dir.join("tokens.txt"))?);
                if kind == "kokoro" {
                    m.kokoro.model = s(&model);
                    m.kokoro.voices = Some(voices);
                    m.kokoro.tokens = Some(tokens);
                    m.kokoro.data_dir = s(&espeak);
                    if dir.join("lexicon-us-en.txt").is_file() {
                        m.kokoro.lexicon = s(&dir.join("lexicon-us-en.txt"));
                    }
                    if dir.join("dict").is_dir() {
                        m.kokoro.dict_dir = s(&dir.join("dict"));
                    }
                } else {
                    m.kitten.model = s(&model);
                    m.kitten.voices = Some(voices);
                    m.kitten.tokens = Some(tokens);
                    m.kitten.data_dir = s(&espeak);
                }
            }
            "matcha" => {
                let fs = files(dir);
                let acoustic = fs.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("model-steps-")));
                let vocoder = fs.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("vocos")));
                anyhow::ensure!(espeak.join("phontab").is_file(), "the voice has no espeak-ng data");
                m.matcha.acoustic_model = acoustic.and_then(|p| s(p));
                m.matcha.vocoder = Some(vocoder.and_then(|p| s(p)).ok_or_else(|| anyhow::anyhow!("the voice has no vocoder (vocos*.onnx)"))?);
                m.matcha.tokens = Some(need(&dir.join("tokens.txt"))?);
                m.matcha.data_dir = s(&espeak);
            }
            _ => {
                let model = files(dir).into_iter().find(|p| p.extension().is_some_and(|x| x == "onnx")).ok_or_else(|| anyhow::anyhow!("no model"))?;
                m.vits.model = s(&model);
                m.vits.tokens = Some(need(&dir.join("tokens.txt"))?);
                anyhow::ensure!(espeak.join("phontab").is_file(), "the voice has no espeak-ng data");
                m.vits.data_dir = s(&espeak);
            }
        }
        cfg.model.num_threads = threads;
        if let Some(p) = &o.provider {
            cfg.model.provider = Some(p.clone());
        }
        cfg.max_num_sentences = 1;
        let tts = OfflineTts::create(&cfg).ok_or_else(|| anyhow::anyhow!("sherpa-onnx couldn't load the voice in {}", dir.display()))?;
        Ok(Voice { tts, kind, config })
    }

    /// `text` spoken (a sentence or two), timed.
    pub fn speak(&self, text: &str) -> Option<Spoken> {
        let t = Instant::now();
        let audio = self.tts.generate_with_config(text, &self.config, None::<fn(&[f32], f32) -> bool>)?;
        let samples = audio.samples().to_vec();
        let ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
        (!samples.is_empty()).then(|| Spoken { samples, rate: audio.sample_rate(), ms })
    }

    pub fn speakers(&self) -> i32 {
        self.tts.num_speakers()
    }
}

/// A 16-bit PCM WAV's samples (its first channel) and rate.
pub fn read_wav(path: &Path) -> anyhow::Result<(Vec<f32>, u32)> {
    let b = std::fs::read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    anyhow::ensure!(b.len() > 44 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE", "{} isn't a WAV file", path.display());
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
            anyhow::ensure!(bits == 16, "{} isn't 16-bit audio", path.display());
            let step = 2 * usize::from(channels.max(1));
            let samples = body.chunks_exact(step).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect();
            return Ok((samples, rate));
        }
        at += 8 + len + (len & 1);
    }
    anyhow::bail!("{} has no audio", path.display())
}

/// One voice's benchmark (`/harness/tts`): its load, and each sentence's time against its length.
#[derive(Debug, Serialize)]
pub struct Bench {
    pub kind: &'static str,
    pub threads: i32,
    pub load_ms: u64,
    pub rate: i32,
    pub speakers: i32,
    pub sentences: Vec<Sentence>,
    /// Compute over audio length, all sentences together (< 1: faster than real time).
    pub rtf: f32,
}

#[derive(Debug, Serialize)]
pub struct Sentence {
    pub text: String,
    pub ms: u64,
    pub audio_s: f32,
    pub rtf: f32,
    /// Where its WAV went, if asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wav: Option<String>,
}

/// Loads the voice in `dir`, speaks each of `texts` (after one unmeasured warm-up), and writes
/// `{out}-{n}.wav` for each when `out` is given.
pub fn bench(dir: &Path, threads: i32, o: &Options, texts: &[String], out: Option<&Path>) -> anyhow::Result<Bench> {
    let t = Instant::now();
    let voice = Voice::load(dir, threads, o)?;
    let load_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
    // The first call pays for ONNX Runtime's own warm-up; a voice that's kept loaded doesn't.
    let _ = voice.speak("Hello.");
    let (mut rate, mut sentences, mut total_ms, mut total_s) = (0, Vec::new(), 0u64, 0f32);
    for (n, text) in texts.iter().enumerate() {
        let spoken = voice.speak(text).ok_or_else(|| anyhow::anyhow!("no audio for \"{text}\""))?;
        rate = spoken.rate;
        #[allow(clippy::cast_precision_loss)]
        let audio_s = spoken.samples.len() as f32 / spoken.rate.max(1) as f32;
        let wav = match out {
            Some(base) => {
                let p = PathBuf::from(format!("{}-{n}.wav", base.display()));
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&p, crate::translator::speech::wav(&spoken.samples, spoken.rate))?;
                Some(p.to_string_lossy().into_owned())
            }
            None => None,
        };
        #[allow(clippy::cast_precision_loss)]
        let rtf = spoken.ms as f32 / 1000.0 / audio_s.max(0.001);
        total_ms += spoken.ms;
        total_s += audio_s;
        sentences.push(Sentence { text: text.clone(), ms: spoken.ms, audio_s, rtf, wav });
    }
    #[allow(clippy::cast_precision_loss)]
    let rtf = total_ms as f32 / 1000.0 / total_s.max(0.001);
    Ok(Bench { kind: voice.kind, threads, load_ms, rate, speakers: voice.speakers(), sentences, rtf })
}

/// Paradee-8M (Kokoro's `af_heart`, distilled to 8M parameters; Apache-2.0): not a sherpa-onnx model, so it
/// runs on r1d's own ONNX Runtime. It takes Kokoro's phoneme ids (misaki's G2P) and speaks at 24 kHz. For
/// now only the benchmark: `ids` come from misaki on a PC, one list per sentence.
pub fn paradee_bench(lib: Option<&Path>, model: &Path, threads: usize, ids: &[Vec<i64>], out: Option<&Path>) -> anyhow::Result<Bench> {
    use ort::session::Session;
    use ort::value::Tensor;
    crate::system1::ort_ready(lib)?;
    let t = Instant::now();
    let mut session = Session::builder()?
        .with_intra_threads(threads)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .commit_from_file(model)?;
    let load_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut speak = |ids: &[i64]| -> anyhow::Result<(Vec<f32>, u64)> {
        // A pad (0) at each end, as the model was trained.
        let mut padded = Vec::with_capacity(ids.len() + 2);
        padded.push(0);
        padded.extend_from_slice(ids);
        padded.push(0);
        let n = padded.len();
        let t = Instant::now();
        let out = session.run(ort::inputs![
            "input_ids" => Tensor::from_array(([1usize, n], padded))?,
            "speed" => Tensor::from_array(([1usize], vec![1.0f32]))?,
        ])?;
        let (_, wave) = out["waveform"].try_extract_tensor::<f32>()?;
        Ok((wave.to_vec(), u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)))
    };
    let _ = speak(ids.first().map_or(&[][..], Vec::as_slice));
    let (mut sentences, mut total_ms, mut total_s) = (Vec::new(), 0u64, 0f32);
    for (n, sentence) in ids.iter().enumerate() {
        let (samples, ms) = speak(sentence)?;
        #[allow(clippy::cast_precision_loss)]
        let audio_s = samples.len() as f32 / 24_000.0;
        let wav = match out {
            Some(base) => {
                let p = PathBuf::from(format!("{}-{n}.wav", base.display()));
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&p, crate::translator::speech::wav(&samples, 24_000))?;
                Some(p.to_string_lossy().into_owned())
            }
            None => None,
        };
        #[allow(clippy::cast_precision_loss)]
        let rtf = ms as f32 / 1000.0 / audio_s.max(0.001);
        total_ms += ms;
        total_s += audio_s;
        sentences.push(Sentence { text: format!("{} phonemes", sentence.len()), ms, audio_s, rtf, wav });
    }
    #[allow(clippy::cast_precision_loss)]
    let rtf = total_ms as f32 / 1000.0 / total_s.max(0.001);
    Ok(Bench { kind: "paradee", threads: i32::try_from(threads).unwrap_or(1), load_ms, rate: 24_000, speakers: 1, sentences, rtf })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_packages_apart() {
        let root = std::env::temp_dir().join(format!("r1d-tts-{}", std::process::id()));
        let make = |name: &str, files: &[&str]| {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            for f in files {
                std::fs::write(d.join(f), b"").unwrap();
            }
            d
        };
        let kind = |d: PathBuf| kind_of(&d).unwrap();
        assert_eq!(kind(make("sherpa-onnx-supertonic-3", &["tts.json", "voice.bin", "vocoder.int8.onnx"])), "supertonic");
        assert_eq!(kind(make("pocket", &["lm_main.int8.onnx", "lm_flow.int8.onnx"])), "pocket");
        assert_eq!(kind(make("kokoro-int8-multi-lang-v1_0", &["voices.bin", "model.int8.onnx", "tokens.txt"])), "kokoro");
        assert_eq!(kind(make("kitten-mini-en-v0_8", &["voices.bin", "model.onnx", "tokens.txt"])), "kitten");
        assert_eq!(kind(make("matcha-icefall-en_US-ljspeech", &["model-steps-3.onnx", "vocos-22khz-univ.onnx", "tokens.txt"])), "matcha");
        assert_eq!(kind(make("vits-piper-en_US-lessac-high", &["en_US-lessac-high.onnx", "tokens.txt"])), "vits");
        assert!(kind_of(&make("empty", &[])).is_err());
        let p = make("picks", &["model.onnx", "model.int8.onnx", "decoder.onnx"]);
        assert_eq!(pick(&p, "model").unwrap().file_name().unwrap(), "model.int8.onnx");
        assert_eq!(pick(&p, "decoder").unwrap().file_name().unwrap(), "decoder.onnx");
        let _ = std::fs::remove_dir_all(root);
    }
}
