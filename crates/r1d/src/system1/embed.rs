//! System One's sentence embedder (ADR-0032): all-MiniLM-L6-v2 (int8 for ARM64) on ONNX Runtime, the same
//! library speech-to-text ships on the R1, loaded at run time. A sentence becomes 384 numbers (the tokens'
//! mean, normalised), and sentences that mean the same land close: [`cosine`] near 1.

use super::tokenizer::WordPiece;
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use std::sync::Mutex;

/// Tokens per sentence at most: what's said in one hold fits.
const MAX_TOKENS: usize = 64;

pub struct Embedder {
    session: Mutex<Session>,
    tok: WordPiece,
    pub model: String,
}

impl Embedder {
    /// From `dir` (`model.int8.onnx` or `model.onnx`, `vocab.txt`), with ONNX Runtime at `lib` (loaded once
    /// per process; `None`: whatever the system finds).
    pub fn load(dir: &Path, lib: Option<&Path>) -> anyhow::Result<Self> {
        let model = ["model.int8.onnx", "model.onnx"]
            .iter()
            .map(|n| dir.join(n))
            .find(|p| p.is_file())
            .ok_or_else(|| anyhow::anyhow!("no model.onnx in {}", dir.display()))?;
        let vocab = std::fs::read_to_string(dir.join("vocab.txt"))?;
        super::ort_ready(lib)?;
        let session = Session::builder()?.with_intra_threads(2).map_err(|e| anyhow::anyhow!("{e}"))?.commit_from_file(&model)?;
        let name = std::fs::read_to_string(dir.join("MODEL")).map(|s| s.trim().to_owned()).unwrap_or_else(|_| "all-MiniLM-L6-v2".into());
        Ok(Embedder { session: Mutex::new(session), tok: WordPiece::from_vocab(&vocab)?, model: name })
    }

    /// The sentence's embedding: unit length.
    pub fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let ids = self.tok.encode(text, MAX_TOKENS);
        let n = ids.len();
        let inputs = ort::inputs![
            "input_ids" => Tensor::from_array(([1usize, n], ids))?,
            "attention_mask" => Tensor::from_array(([1usize, n], vec![1i64; n]))?,
            "token_type_ids" => Tensor::from_array(([1usize, n], vec![0i64; n]))?,
        ];
        let mut session = self.session.lock().unwrap();
        let out = session.run(inputs)?;
        let (shape, data) = out[0].try_extract_tensor::<f32>()?;
        let dim = usize::try_from(*shape.last().unwrap_or(&0)).unwrap_or(0);
        anyhow::ensure!(dim > 0 && data.len() >= n * dim, "the model's output isn't [1, tokens, dim]");
        let mut v = vec![0f32; dim];
        for t in 0..n {
            for (i, x) in v.iter_mut().enumerate() {
                *x += data[t * dim + i];
            }
        }
        normalise(&mut v);
        Ok(v)
    }
}

fn normalise(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// Two unit vectors' cosine (their dot product).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
