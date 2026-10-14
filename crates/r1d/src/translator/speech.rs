//! The Translator's ears and voice (ADR-0034), both sherpa-onnx, already in r1d for push-to-talk.
//!
//! - **Ears:** OpenAI's Whisper (base, multilingual, int8; MIT), which hears ~100 languages and says
//!   which one it heard. Through sherpa-onnx's C API directly: the crate's wrapper drops the language.
//! - **Voice:** Piper's VITS voices (one per language, `tts/<lang>/model.onnx`), with the espeak-ng data
//!   they share for pronunciation (`tts/espeak-ng-data`).

use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig};
use sherpa_onnx_sys as sys;
use std::ffi::{CStr, CString};
use std::path::Path;

/// Whisper hears 30 s at a time; a translator turn is a sentence or two.
pub const MAX_HEARD_S: usize = 30;

pub struct Whisper {
    rec: *const sys::OfflineRecognizer,
    /// The strings its config points at, alive as long as it is.
    _strings: Vec<CString>,
    pub model: String,
}

// SAFETY: sherpa-onnx's recognizer is safe to use from one thread at a time; r1d decodes under a lock.
unsafe impl Send for Whisper {}
unsafe impl Sync for Whisper {}

pub struct Heard {
    pub text: String,
    /// ISO 639-1, as Whisper names it (`en`, `es`, `zh`...).
    pub lang: String,
}

impl Whisper {
    /// From `dir`: `encoder.int8.onnx`, `decoder.int8.onnx`, `tokens.txt`.
    pub fn load(dir: &Path, threads: i32) -> anyhow::Result<Self> {
        let file = |name: &str| -> anyhow::Result<CString> {
            let p = dir.join(name);
            anyhow::ensure!(p.is_file(), "no {name} in {}", dir.display());
            Ok(CString::new(p.to_string_lossy().into_owned())?)
        };
        let strings = vec![file("encoder.int8.onnx")?, file("decoder.int8.onnx")?, file("tokens.txt")?, CString::new("transcribe")?];
        // SAFETY: a zeroed config is sherpa-onnx's documented default (every field falls back when 0/null).
        let mut cfg: sys::OfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        cfg.model_config.whisper.encoder = strings[0].as_ptr();
        cfg.model_config.whisper.decoder = strings[1].as_ptr();
        cfg.model_config.tokens = strings[2].as_ptr();
        cfg.model_config.whisper.task = strings[3].as_ptr();
        cfg.model_config.num_threads = threads;
        // No language: Whisper detects it, and says which.
        let rec = unsafe { sys::SherpaOnnxCreateOfflineRecognizer(&raw const cfg) };
        anyhow::ensure!(!rec.is_null(), "sherpa-onnx couldn't load Whisper from {}", dir.display());
        let model = std::fs::read_to_string(dir.join("MODEL")).map(|s| s.trim().to_owned()).unwrap_or_else(|_| "whisper".into());
        Ok(Whisper { rec, _strings: strings, model })
    }

    /// What was said (16 kHz mono), and in which language.
    pub fn hear(&self, samples: &[f32]) -> Option<Heard> {
        let samples = &samples[..samples.len().min(MAX_HEARD_S * 16_000)];
        unsafe {
            let stream = sys::SherpaOnnxCreateOfflineStream(self.rec);
            if stream.is_null() {
                return None;
            }
            sys::SherpaOnnxAcceptWaveformOffline(stream, 16_000, samples.as_ptr(), i32::try_from(samples.len()).unwrap_or(0));
            sys::SherpaOnnxDecodeOfflineStream(self.rec, stream);
            let json = sys::SherpaOnnxGetOfflineStreamResultAsJson(stream);
            let out = if json.is_null() {
                None
            } else {
                let s = CStr::from_ptr(json).to_string_lossy().into_owned();
                sys::SherpaOnnxDestroyOfflineStreamResultJson(json);
                serde_json::from_str::<serde_json::Value>(&s).ok()
            };
            sys::SherpaOnnxDestroyOfflineStream(stream);
            let v = out?;
            let text = without_tags(v["text"].as_str().unwrap_or_default());
            // `<|es|>` or `es`.
            let lang = v["lang"].as_str().unwrap_or_default().trim_matches(|c: char| c == '<' || c == '>' || c == '|').to_owned();
            Some(Heard { text, lang })
        }
    }
}

impl Drop for Whisper {
    fn drop(&mut self) {
        unsafe { sys::SherpaOnnxDestroyOfflineRecognizer(self.rec) }
    }
}

/// One language's voice.
pub struct Speaker {
    tts: OfflineTts,
}

impl Speaker {
    /// From `dir` (`model.onnx`, `tokens.txt`), with the shared espeak-ng data at `espeak`; or, for a voice
    /// that has one (Chinese), its own `lexicon.txt` and text rules (`*.fst`). sherpa-onnx ends the process
    /// rather than failing when a voice's pieces are missing (2026-10-10: the Chinese voice without its
    /// lexicon took r1d down), so what's needed is checked here first.
    pub fn load(dir: &Path, espeak: &Path, threads: i32) -> anyhow::Result<Self> {
        let model = dir.join("model.onnx");
        let tokens = dir.join("tokens.txt");
        anyhow::ensure!(model.is_file() && tokens.is_file(), "no voice in {}", dir.display());
        let mut cfg = OfflineTtsConfig::default();
        cfg.model.vits.model = Some(model.to_string_lossy().into_owned());
        cfg.model.vits.tokens = Some(tokens.to_string_lossy().into_owned());
        let lexicon = dir.join("lexicon.txt");
        if lexicon.is_file() {
            cfg.model.vits.lexicon = Some(lexicon.to_string_lossy().into_owned());
            let mut rules: Vec<String> = std::fs::read_dir(dir)?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "fst"))
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            rules.sort();
            if !rules.is_empty() {
                cfg.rule_fsts = Some(rules.join(","));
            }
        } else {
            anyhow::ensure!(espeak.join("phontab").is_file(), "no espeak-ng data at {}", espeak.display());
            cfg.model.vits.data_dir = Some(espeak.to_string_lossy().into_owned());
        }
        cfg.model.num_threads = threads;
        cfg.max_num_sentences = 1;
        let tts = OfflineTts::create(&cfg).ok_or_else(|| anyhow::anyhow!("sherpa-onnx couldn't load the voice in {}", dir.display()))?;
        Ok(Speaker { tts })
    }

    /// `text` spoken, as a WAV file.
    pub fn say(&self, text: &str) -> Option<Vec<u8>> {
        let audio = self.tts.generate_with_config(text, &GenerationConfig::default(), None::<fn(&[f32], f32) -> bool>)?;
        let samples = audio.samples();
        if samples.is_empty() {
            return None;
        }
        Some(wav(samples, audio.sample_rate()))
    }
}

/// Whisper's words without its non-speech tags ("[Bell]", "(music)", "♪"): noise, not something said.
fn without_tags(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            '♪' | '♫' => {}
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 16-bit mono WAV.
pub fn wav(samples: &[f32], rate: i32) -> Vec<u8> {
    let data_len = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let rate = u32::try_from(rate).unwrap_or(22_050);
    let mut w = Vec::with_capacity(44 + samples.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        #[allow(clippy::cast_possible_truncation)]
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

#[cfg(test)]
mod tests {
    #[test]
    fn noise_tags_are_dropped() {
        assert_eq!(super::without_tags(" [Bell]"), "");
        assert_eq!(super::without_tags("(music) Hola, ¿qué tal? ♪"), "Hola, ¿qué tal?");
    }
}
