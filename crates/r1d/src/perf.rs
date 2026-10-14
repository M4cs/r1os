//! Latency marks (T-032): how long the steps you wait on take, kept for `r1h perf` (`GET /harness/perf`) and the
//! baseline (docs/research). In memory: the last [`KEEP`], lost when r1d restarts.
//!
//! - `stt.load`: speech-to-text's model, from r1d's start.
//! - `voice.release_to_final.short` / `.full`: the button up after talking → the final words (crate::voice); `short`
//!   when you'd stopped talking before letting go and it didn't keep listening (T-042).
//! - `reply.first_word.warm` / `.cold`: a Claude chat's message sent to its CLI → the first word of the reply; warm when
//!   the process was already running, cold when it started for this turn (crate::session).
//! - `reply.init.warm` / `.cold`: → the CLI's `init` for the turn (its own overhead, before the model's call).
//! - `reply.done.<brain>`: → the whole reply (`claude`, `codex`, `pc` for a computer's).
//! - `voice.release_to_first_word`: the button up → the first word of the reply, when the reply follows what you said.
//! - `app.<op>`: a step in the app window (crate::appwin: `app.open`, `app.click`, `app.read`…), r1d's request → the
//!   host's answer, settling included.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Marks kept.
const KEEP: usize = 400;
/// A reply this long after the button came up still counts as answering what you said.
const ANSWERING: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize)]
pub struct Mark {
    pub name: String,
    pub ms: u64,
    /// When (ms since the epoch).
    pub at: i64,
}

#[derive(Default)]
pub struct Perf {
    marks: Mutex<VecDeque<Mark>>,
    /// The button came up after talking: when, and whether a reply's first word has been counted against it.
    release: Mutex<Option<(Instant, bool)>>,
    /// Whether that release waited only briefly (you'd stopped talking).
    short: std::sync::atomic::AtomicBool,
}

impl Perf {
    pub fn mark(&self, name: &str, took: Duration) {
        let ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
        tracing::info!(target: "perf", mark = name, ms);
        let mut m = self.marks.lock().unwrap();
        if m.len() >= KEEP {
            m.pop_front();
        }
        m.push_back(Mark { name: name.to_owned(), ms, at: crate::core::now() });
    }

    /// You let go of the button after talking; `short`: it listened only briefly after.
    pub fn released(&self, short: bool) {
        *self.release.lock().unwrap() = Some((Instant::now(), false));
        self.short.store(short, std::sync::atomic::Ordering::Relaxed);
    }

    /// The final words came: since the button came up.
    pub fn final_words(&self) {
        let since = self.release.lock().unwrap().map(|(t, _)| t.elapsed());
        if let Some(d) = since.filter(|d| *d < Duration::from_secs(15)) {
            let short = self.short.load(std::sync::atomic::Ordering::Relaxed);
            self.mark(if short { "voice.release_to_final.short" } else { "voice.release_to_final.full" }, d);
        }
    }

    /// A reply's first word: since the button came up, once per release.
    pub fn first_word(&self) {
        let since = {
            let mut r = self.release.lock().unwrap();
            match r.as_mut() {
                Some((t, counted)) if !*counted && t.elapsed() < ANSWERING => {
                    *counted = true;
                    Some(t.elapsed())
                }
                _ => None,
            }
        };
        if let Some(d) = since {
            self.mark("voice.release_to_first_word", d);
        }
    }

    pub fn reset(&self) {
        self.marks.lock().unwrap().clear();
    }

    /// The marks (newest last) and, per name, how many, the last, the median, the 90th percentile and the most.
    pub fn summary(&self) -> Value {
        let marks: Vec<Mark> = self.marks.lock().unwrap().iter().cloned().collect();
        let mut by: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
        for m in &marks {
            by.entry(m.name.as_str()).or_default().push(m.ms);
        }
        let stats: serde_json::Map<String, Value> = by
            .into_iter()
            .map(|(name, mut v)| {
                let last = *v.last().unwrap_or(&0);
                v.sort_unstable();
                let at = |p: f64| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
                    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
                    v[i.min(v.len() - 1)]
                };
                (name.to_owned(), json!({ "n": v.len(), "last": last, "p50": at(0.5), "p90": at(0.9), "max": v[v.len() - 1] }))
            })
            .collect();
        let recent: Vec<&Mark> = marks.iter().rev().take(50).collect();
        json!({ "stats": stats, "recent": recent })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_and_releases() {
        let p = Perf::default();
        for ms in [100, 300, 200, 900] {
            p.mark("reply.first_word.warm", Duration::from_millis(ms));
        }
        let s = p.summary();
        let w = &s["stats"]["reply.first_word.warm"];
        assert_eq!((w["n"].as_u64(), w["last"].as_u64(), w["p50"].as_u64(), w["max"].as_u64()), (Some(4), Some(900), Some(300), Some(900)));
        // A first word counts against the release once.
        p.released(false);
        p.first_word();
        p.first_word();
        assert_eq!(p.summary()["stats"]["voice.release_to_first_word"]["n"], 1);
        p.reset();
        assert!(p.summary()["recent"].as_array().unwrap().is_empty());
    }
}
