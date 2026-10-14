//! On-device translation (ADR-0034): Helsinki-NLP's OPUS-MT models (Marian, one per direction, CC-BY-4.0),
//! int8 and exported to ONNX by transformers.js (Xenova), on the ONNX Runtime the R1 already has for
//! speech. Greedy decoding with the decoder's key/value cache: one decoder step per output token.
//!
//! A model directory holds `encoder.onnx`, `decoder.onnx` (the merged decoder: with and without the
//! cache), `tokenizer.json` (its SentencePiece unigram vocabulary, whose order is the model's ids) and
//! `config.json`. The tokenizer is here rather than a library: Marian's needs only whitespace splitting,
//! `▁` word marks and the unigram's best split.

use ort::session::Session;
use ort::session::SessionInputValue;
use ort::value::{DynValue, Tensor};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Input tokens per request at most (Marian's limit is 512; a held button says far less).
const MAX_IN: usize = 256;
/// The piece SentencePiece puts before each word.
const MARK: char = '\u{2581}';

pub struct Marian {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    tok: Pieces,
    layers: usize,
    heads: usize,
    head_dim: usize,
}

/// The SentencePiece unigram vocabulary: pieces in id order, with their scores.
struct Pieces {
    pieces: Vec<String>,
    scores: Vec<f32>,
    index: HashMap<String, i64>,
    longest: usize,
    /// What a character no piece covers costs (SentencePiece: the lowest score, less 10).
    unk_score: f32,
    unk: i64,
    eos: i64,
    pad: i64,
}

impl Marian {
    pub fn load(dir: &Path, threads: usize) -> anyhow::Result<Self> {
        let config: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let tok: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("tokenizer.json"))?)?;
        let vocab = tok["model"]["vocab"].as_array().ok_or_else(|| anyhow::anyhow!("tokenizer.json has no unigram vocab"))?;
        let num = |k: &str, d: i64| config[k].as_i64().unwrap_or(d);
        #[allow(clippy::cast_possible_truncation)]
        let tok = Pieces::new(
            vocab.iter().map(|v| (v[0].as_str().unwrap_or_default().to_owned(), v[1].as_f64().unwrap_or(-100.0) as f32)).collect(),
            num("eos_token_id", 0),
            num("decoder_start_token_id", num("pad_token_id", 0)),
        );
        let d_model = usize::try_from(num("d_model", 512)).unwrap_or(512);
        let heads = usize::try_from(num("decoder_attention_heads", 8)).unwrap_or(8);
        let session = |name: &str| load_session(dir, name, threads);
        Ok(Marian {
            encoder: Mutex::new(session("encoder")?),
            decoder: Mutex::new(session("decoder")?),
            tok,
            layers: usize::try_from(num("decoder_layers", 6)).unwrap_or(6),
            heads,
            head_dim: d_model / heads.max(1),
        })
    }

    /// `text` in the model's target language. `target`: the target-language token a multi-target model
    /// wants first (`>>es<<` for en-ROMANCE). A sentence at a time: given two, OPUS-MT can drop one
    /// ("Where is the station? I need the last train." came back as the second alone, 2026-10-10).
    pub fn translate(&self, text: &str, target: Option<&str>) -> anyhow::Result<String> {
        let mut out = String::new();
        for s in sentences(text) {
            let t = self.translate_one(&s, target)?;
            if !out.is_empty() && !out.ends_with(['。', '！', '？']) {
                out.push(' ');
            }
            out.push_str(&t);
        }
        Ok(out)
    }

    fn translate_one(&self, text: &str, target: Option<&str>) -> anyhow::Result<String> {
        let ids = self.tok.encode(text, target);
        let n = ids.len();
        let hidden = {
            let mut enc = self.encoder.lock().unwrap();
            let out = enc.run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, n], ids))?,
                "attention_mask" => Tensor::from_array(([1usize, n], vec![1i64; n]))?,
            ])?;
            let (_, h) = out[0].try_extract_tensor::<f32>()?;
            h.to_vec()
        };
        let d = hidden.len() / n.max(1);
        let empty = || Tensor::from_array(([1usize, self.heads, 0, self.head_dim], Vec::<f32>::new()));
        // Per layer, key then value: the decoder's own (they grow a step at a time), and the encoder's,
        // computed on the first step and kept: on later steps the merged decoder's encoder outputs are
        // placeholders (transformers.js keeps the first ones too).
        let mut past: Vec<DynValue> = Vec::new();
        let mut encoder_past: Vec<DynValue> = Vec::new();
        let mut out_ids: Vec<i64> = Vec::new();
        let (eos, pad) = (self.tok.eos, self.tok.pad);
        let mut token = pad;
        let max_new = (n * 2 + 16).min(MAX_IN * 2);
        let mut dec = self.decoder.lock().unwrap();
        for step in 0..max_new {
            let mut inputs: Vec<(Cow<'_, str>, SessionInputValue<'_>)> = vec![
                ("encoder_attention_mask".into(), Tensor::from_array(([1usize, n], vec![1i64; n]))?.into()),
                ("input_ids".into(), Tensor::from_array(([1usize, 1], vec![token]))?.into()),
                ("encoder_hidden_states".into(), Tensor::from_array(([1usize, n, d], hidden.clone()))?.into()),
                ("use_cache_branch".into(), Tensor::from_array(([1usize], vec![step > 0]))?.into()),
            ];
            let mut own = std::mem::take(&mut past).into_iter();
            for l in 0..self.layers {
                for (i, kind) in ["decoder.key", "decoder.value", "encoder.key", "encoder.value"].iter().enumerate() {
                    let v: SessionInputValue<'_> = match (step, i) {
                        (0, _) => empty()?.into(),
                        (_, 0 | 1) => own.next().ok_or_else(|| anyhow::anyhow!("missing the decoder's cache"))?.into(),
                        _ => (&encoder_past[l * 2 + i - 2]).into(),
                    };
                    inputs.push((format!("past_key_values.{l}.{kind}").into(), v));
                }
            }
            let mut out = dec.run(inputs)?;
            let next = {
                let (_, logits) = out["logits"].try_extract_tensor::<f32>()?;
                let vocab = self.tok.pieces.len().min(logits.len());
                let last = &logits[logits.len() - vocab..];
                let mut best = (f32::NEG_INFINITY, eos);
                for (i, &x) in last.iter().enumerate() {
                    let id = i64::try_from(i).unwrap_or(0);
                    if id != pad && x > best.0 {
                        best = (x, id);
                    }
                }
                best.1
            };
            for l in 0..self.layers {
                let kinds: &[&str] = if step == 0 { &["decoder.key", "decoder.value", "encoder.key", "encoder.value"] } else { &["decoder.key", "decoder.value"] };
                for kind in kinds {
                    let v = out.remove(format!("present.{l}.{kind}")).ok_or_else(|| anyhow::anyhow!("the decoder gave no present.{l}.{kind}"))?;
                    if kind.starts_with("encoder") {
                        encoder_past.push(v);
                    } else {
                        past.push(v);
                    }
                }
            }
            if next == eos {
                break;
            }
            out_ids.push(next);
            token = next;
            // A model stuck on one piece.
            if out_ids.len() >= 8 && out_ids[out_ids.len() - 8..].iter().all(|&t| t == next) {
                break;
            }
        }
        Ok(self.tok.detokenize(&out_ids))
    }
}

/// `<name>.onnx` from `dir`. ONNX Runtime's full graph optimisation is most of a load (~7 s of it on the R1),
/// so the first load keeps its result (`<name>.opt-<arch>.onnx`: optimised for this CPU, so a PC's never
/// passes for the R1's) and later loads take that as it is. A kept one older than its model is made again;
/// one that won't load is dropped.
fn load_session(dir: &Path, name: &str, threads: usize) -> anyhow::Result<Session> {
    use ort::session::builder::GraphOptimizationLevel;
    let src = dir.join(format!("{name}.onnx"));
    let opt = dir.join(format!("{name}.opt-{}.onnx", std::env::consts::ARCH));
    let fresh = |p: &Path| p.metadata().and_then(|m| m.modified()).ok();
    let builder = || -> anyhow::Result<ort::session::builder::SessionBuilder> {
        Session::builder()?.with_intra_threads(threads).map_err(|e| anyhow::anyhow!("{e}"))
    };
    if opt.is_file() && fresh(&opt) >= fresh(&src) {
        let kept = builder()?
            .with_optimization_level(GraphOptimizationLevel::Disable)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .commit_from_file(&opt);
        match kept {
            Ok(s) => return Ok(s),
            Err(e) => tracing::warn!(model = %opt.display(), error = %e, "translation: the kept optimised model won't load; making it again"),
        }
    }
    let _ = std::fs::remove_file(&opt);
    Ok(builder()?.with_optimized_model_path(&opt).map_err(|e| anyhow::anyhow!("{e}"))?.commit_from_file(&src)?)
}

/// `text` cut after each sentence's end (. ! ? and their CJK forms), keeping the marks.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        cur.push(c);
        let wide = matches!(c, '。' | '！' | '？');
        let end = wide || (matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace()));
        if end && !matches!(chars.peek(), Some('.' | '!' | '?' | '"' | '\'' | ')' | '」')) {
            let s = cur.trim().to_owned();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
        }
    }
    let s = cur.trim().to_owned();
    if !s.is_empty() {
        out.push(s);
    }
    out
}

impl Pieces {
    fn new(vocab: Vec<(String, f32)>, eos: i64, pad: i64) -> Self {
        let index: HashMap<String, i64> = vocab.iter().enumerate().map(|(i, (p, _))| (p.clone(), i64::try_from(i).unwrap_or(0))).collect();
        let longest = vocab.iter().map(|(p, _)| p.chars().count()).max().unwrap_or(1).min(32);
        let unk_score = vocab.iter().map(|(_, s)| *s).fold(0.0f32, f32::min) - 10.0;
        let (pieces, scores) = vocab.into_iter().unzip();
        Pieces { unk: *index.get("<unk>").unwrap_or(&1), pieces, scores, index, longest, unk_score, eos, pad }
    }

    /// The model's ids for `text`: each word's best unigram split, then `</s>`.
    fn encode(&self, text: &str, target: Option<&str>) -> Vec<i64> {
        let mut ids = Vec::new();
        if let Some(t) = target.and_then(|t| self.index.get(t)) {
            ids.push(*t);
        }
        for word in text.split_whitespace() {
            ids.extend(self.split(&format!("{MARK}{word}")));
            if ids.len() >= MAX_IN - 1 {
                ids.truncate(MAX_IN - 1);
                break;
            }
        }
        ids.push(self.eos);
        ids
    }

    /// The highest-scoring way to cut one word into vocabulary pieces (Viterbi). A character no piece
    /// covers is `<unk>`.
    fn split(&self, word: &str) -> Vec<i64> {
        let chars: Vec<char> = word.chars().collect();
        let n = chars.len();
        // best[i]: score of the best split of chars[..i], and where its last piece starts, and its id.
        let mut best: Vec<(f32, usize, i64)> = vec![(f32::NEG_INFINITY, 0, self.unk); n + 1];
        best[0].0 = 0.0;
        for start in 0..n {
            if best[start].0 == f32::NEG_INFINITY {
                continue;
            }
            let mut covered = false;
            let mut piece = String::new();
            for end in start + 1..=(start + self.longest).min(n) {
                piece.push(chars[end - 1]);
                if let Some(&id) = self.index.get(&piece) {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    let s = best[start].0 + self.scores[id as usize];
                    if s > best[end].0 {
                        best[end] = (s, start, id);
                    }
                    if end == start + 1 {
                        covered = true;
                    }
                }
            }
            if !covered {
                let s = best[start].0 + self.unk_score;
                if s > best[start + 1].0 {
                    best[start + 1] = (s, start, self.unk);
                }
            }
        }
        let mut ids = Vec::new();
        let mut at = n;
        while at > 0 {
            let (_, start, id) = best[at];
            ids.push(id);
            at = start;
        }
        ids.reverse();
        // Consecutive unknowns are one <unk>, as SentencePiece writes them.
        ids.dedup_by(|a, b| *a == self.unk && *b == self.unk);
        ids
    }

    fn detokenize(&self, ids: &[i64]) -> String {
        let mut s = String::new();
        for &id in ids {
            if id == self.eos || id == self.pad || id == self.unk {
                continue;
            }
            if let Some(p) = usize::try_from(id).ok().and_then(|i| self.pieces.get(i)) {
                s.push_str(p);
            }
        }
        s.replace(MARK, " ").trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_into_the_best_pieces() {
        // The model itself runs on the R1 (/harness/translate); this is the tokenizer.
        let vocab = [("</s>", 0.0), ("<unk>", 0.0), ("▁he", -5.0), ("llo", -5.0), ("▁hello", -3.0), ("▁wor", -4.0), ("ld", -4.0), ("▁", -6.0), (">>es<<", -100.0), ("<pad>", 0.0)];
        let t = Pieces::new(vocab.iter().map(|(p, s)| ((*p).to_owned(), *s)).collect(), 0, 9);
        assert_eq!(t.encode("hello world", None), vec![4, 5, 6, 0], "the higher-scoring whole word wins");
        assert_eq!(t.encode("hello", Some(">>es<<")), vec![8, 4, 0], "a target token goes first");
        assert_eq!(t.encode("héé", None), vec![7, 1, 0], "unknown characters are one <unk>");
        assert_eq!(t.detokenize(&[4, 5, 6, 0]), "hello world");
    }

    #[test]
    fn text_is_cut_into_sentences() {
        assert_eq!(sentences("Where is it? I need the 3.15 train. Thanks!"), vec!["Where is it?", "I need the 3.15 train.", "Thanks!"]);
        assert_eq!(sentences("駅はどこですか？これはいくらですか？"), vec!["駅はどこですか？", "これはいくらですか？"]);
        assert_eq!(sentences("Wait... what?!"), vec!["Wait...", "what?!"]);
        assert_eq!(sentences("no end"), vec!["no end"]);
    }
}

/// With a real model (`R1D_MT_MODEL=<dir>`, and `ORT_DYLIB_PATH` to an ONNX Runtime ≥ 1.24):
/// `cargo test -p r1d real_model -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore = "needs a model and ONNX Runtime"]
fn real_model() {
    let dir = std::path::PathBuf::from(std::env::var("R1D_MT_MODEL").expect("R1D_MT_MODEL"));
    let lib = std::env::var_os("ORT_DYLIB_PATH").map(std::path::PathBuf::from);
    crate::system1::ort_ready(lib.as_deref()).unwrap();
    let m = Marian::load(&dir, 4).unwrap();
    let target = std::env::var("R1D_MT_TARGET").ok();
    let texts = std::env::var("R1D_MT_TEXT").unwrap_or_else(|_| "Where is the train station?|I would like two coffees, please.|How much does this cost?".into());
    for s in texts.split('|') {
        let t = std::time::Instant::now();
        let out = m.translate(s, target.as_deref()).unwrap();
        println!("{s} -> {out} ({} ms)", t.elapsed().as_millis());
    }
}
