//! The Journal's voice notes (ADR-0037): while the Journal's screen is open (a lease it renews), a hold of
//! the button records a note instead of talking to an agent. crate::voice keeps the hold's audio and runs
//! the recognizer as usual, but sends `note` events instead of `voice` ones; on release the audio becomes a
//! recording in the media store (16 kHz WAV, its words as its transcript) and a journal entry.

use crate::hub::{Event, Hub, Source};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The screen renews it each time it opens; each note extends it.
const LEASE: Duration = Duration::from_secs(15 * 60);
/// A note's longest (the Recorder is for longer).
pub const MAX_NOTE_S: usize = 5 * 60;
/// Shorter than this is a slip of the thumb, not a note.
const MIN_NOTE_MS: i64 = 600;

/// A `note` event.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Note {
    /// listening · partial · saved · cancelled · error
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Notes {
    lease: Mutex<Option<Instant>>,
}

impl Notes {
    /// The Journal's screen is open: holds are notes.
    pub fn active(&self) -> bool {
        self.lease.lock().unwrap().is_some_and(|t| t > Instant::now())
    }

    pub fn start(&self) {
        *self.lease.lock().unwrap() = Some(Instant::now() + LEASE);
    }

    pub fn stop(&self) {
        *self.lease.lock().unwrap() = None;
    }

    pub fn emit(hub: &Hub, n: Note) {
        hub.emit(Event::Note(n), Source::Hw);
    }

    /// A finished hold: kept as a recording and a journal entry (on the voice worker's thread).
    pub fn save(hub: &Hub, audio: &[f32], text: &str) {
        if hub.notes.active() {
            hub.notes.start();
        }
        let ms = i64::try_from(audio.len() as u64 * 1000 / 16_000).unwrap_or(0);
        if ms < MIN_NOTE_MS {
            Notes::emit(hub, Note { state: "cancelled".into(), ..Note::default() });
            return;
        }
        match keep(hub, audio, text, ms) {
            Ok((entry, media)) => {
                tracing::info!(entry, media, ms, chars = text.len(), "voice note");
                Notes::emit(hub, Note { state: "saved".into(), text: Some(text.to_owned()), entry: Some(entry), media: Some(media), ms: Some(ms), ..Note::default() });
                hub.emit(Event::MediaChanged { kind: "recording".into() }, Source::Api);
                hub.emit(Event::JournalChanged, Source::Api);
            }
            Err(e) => Notes::emit(hub, Note { state: "error".into(), error: Some(format!("{e:#}")), ..Note::default() }),
        }
    }
}

fn keep(hub: &Hub, audio: &[f32], text: &str, ms: i64) -> anyhow::Result<(String, String)> {
    let wav = crate::translator::speech::wav(audio, 16_000);
    let item = crate::media::Item {
        id: crate::core::random_hex(6),
        kind: "recording".into(),
        at: crate::media::now_ms() - ms,
        mime: "audio/wav".into(),
        bytes: i64::try_from(wav.len()).unwrap_or(0),
        width: None,
        height: None,
        duration_ms: Some(ms),
        source: "journal".into(),
        label: text.split_whitespace().take(8).collect::<Vec<_>>().join(" "),
        transcript: Some(text.to_owned()),
        marks: Vec::new(),
        summary: None,
    };
    let data = &hub.cfg.data;
    std::fs::create_dir_all(crate::media::dir(data))?;
    std::fs::write(crate::media::file(data, &item), &wav)?;
    hub.core.with(|db| crate::media::insert(db, &item))?;
    let entry = hub.core.with(|db| crate::journal::add(db, text, Some(&item.id), None, item.at))?;
    Ok((entry.id, item.id))
}
