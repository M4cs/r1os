//! The Recorder (ADR-0033): records from the R1's mic into a WAV under `<data>/media`, transcribing as it
//! goes with the same on-device speech model as push-to-talk (crate::voice). Started and stopped from the
//! Recorder app or by an agent (`start_recording`, `stop_recording`).
//!
//! - **The mic is shared:** push-to-talk still works while recording, and the mic stays on until both are
//!   done (`Voice::hold_mic`).
//! - **The file:** 16 kHz mono 16-bit PCM, ~1.9 MB a minute; its header is fixed up as it goes, so a
//!   recording cut short by a crash still plays. At most [`MAX`]; it stops when the storage runs low.
//! - **The transcript** is the speech model's running text, saved about every second with the length, and
//!   sent to the shell (`recording`).
//! - **Kept until you delete it.** Unlike push-to-talk's audio (ADR-0009), a recording is stored: you asked
//!   for it.
//! - **Marks** (T-068): while it records, the Recorder's mark key notes the moment, and its photo key the moment
//!   a photo was taken ([`Recorder::mark`]).
//! - **A title and a summary** (T-068), if you turned them on: when it stops, one short no-tools turn on Haiku
//!   reads its words (they go to Claude) and names it, unless you had, and sums it up.

use crate::hub::{Event, Hub, Source};
use std::io::{Seek, SeekFrom, Write};
use std::sync::mpsc as smpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The longest recording.
const MAX: Duration = Duration::from_secs(3 * 3600);
/// Stop when this little space is left.
const MIN_FREE: u64 = 200 * 1024 * 1024;
const RATE: u32 = crate::voice::RATE as u32;
/// The transcript and length are saved and sent at most this often.
const EVERY: Duration = Duration::from_secs(1);
/// How often the meter hears from it.
const LEVEL_EVERY: Duration = Duration::from_millis(100);
/// A tape with fewer words isn't summed up.
const SUMMARY_MIN_WORDS: usize = 12;
/// How much of a long tape's words the summary reads.
const SUMMARY_CHARS: usize = 24_000;
const SUMMARY_PROMPT: &str = "You title and sum up a voice recording from its words, which come from on-device speech recognition, so expect misheard words. Reply with JSON only, no fences: {\"title\": \"at most 6 words\", \"summary\": \"1 to 3 short sentences: what it's about and anything decided or to do\"}. Write in the recording's language. Plain text, no markdown.";

/// What's recording now.
#[derive(Clone, Debug)]
pub struct Now {
    pub id: String,
    pub started: Instant,
}

#[derive(Default)]
pub struct Recorder {
    now: Mutex<Option<Now>>,
}

impl Recorder {
    pub fn current(&self) -> Option<Now> {
        self.now.lock().unwrap().clone()
    }

    /// Starts a recording labelled `label`. Its media id.
    pub fn start(hub: &Arc<Hub>, label: &str) -> anyhow::Result<String> {
        let rec = &hub.recorder;
        anyhow::ensure!(rec.now.lock().unwrap().is_none(), "Already recording.");
        let voice = hub.voice.get().ok_or_else(|| anyhow::anyhow!("The mic isn't ready yet."))?.clone();
        anyhow::ensure!(voice.has_mic(), "No microphone: the host app isn't connected.");
        let data = hub.cfg.data.clone();
        anyhow::ensure!(free_bytes(&data) > MIN_FREE * 2, "The R1's storage is nearly full.");
        let item = crate::media::Item {
            id: crate::core::random_hex(6),
            kind: "recording".into(),
            at: crate::media::now_ms(),
            mime: "audio/wav".into(),
            bytes: 44,
            width: None,
            height: None,
            duration_ms: Some(0),
            source: "recorder".into(),
            label: label.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect(),
            transcript: Some(String::new()),
            marks: Vec::new(),
            summary: None,
        };
        std::fs::create_dir_all(crate::media::dir(&data))?;
        let path = crate::media::file(&data, &item);
        let mut wav = std::fs::File::create(&path)?;
        wav.write_all(&wav_header(0))?;
        hub.core.with(|db| crate::media::insert(db, &item))?;
        let (tx, rx) = smpsc::channel::<Vec<f32>>();
        *rec.now.lock().unwrap() = Some(Now { id: item.id.clone(), started: Instant::now() });
        // The mic's audio comes here too, until the recording stops (which drops the tap, ending the writer).
        voice.tap(Some(tx));
        voice.hold_mic(crate::voice::Holder::Recorder, true);
        let h = hub.clone();
        let id = item.id.clone();
        let stt = voice.stt();
        // For the summary when it's saved (an async turn, from the writer's thread).
        let rt = tokio::runtime::Handle::try_current().ok();
        std::thread::Builder::new().name("recorder".into()).spawn(move || write(&h, &id, wav, stt, &rx, rt))?;
        tracing::info!(id = %item.id, "recording");
        hub.emit(Event::Recording { id: item.id.clone(), state: "recording".into(), seconds: 0, text: String::new() }, Source::Api);
        hub.emit(Event::MediaChanged { kind: "recording".into() }, Source::Api);
        Ok(item.id)
    }

    /// Marks this moment of the recording (T-068): with `photo`, a photo taken now (its media id). The mark.
    pub fn mark(hub: &Hub, photo: Option<String>) -> anyhow::Result<crate::media::Mark> {
        let now = hub.recorder.current().ok_or_else(|| anyhow::anyhow!("Not recording."))?;
        if let Some(p) = &photo {
            let item = hub.core.with(|db| crate::media::get(db, p))?.ok_or_else(|| anyhow::anyhow!("no photo `{p}`"))?;
            anyhow::ensure!(item.kind == "photo", "`{p}` isn't a photo");
        }
        let mark = crate::media::Mark { at_ms: i64::try_from(now.started.elapsed().as_millis()).unwrap_or(0), photo };
        hub.core.with(|db| crate::media::add_mark(db, &now.id, mark.clone()))?;
        hub.emit(Event::MediaChanged { kind: "recording".into() }, Source::Api);
        Ok(mark)
    }

    /// Stops the recording, if there is one. Its id.
    pub fn stop(hub: &Hub) -> Option<String> {
        let now = hub.recorder.now.lock().unwrap().take()?;
        if let Some(v) = hub.voice.get() {
            v.tap(None); // the writer finishes the file and says so
            v.hold_mic(crate::voice::Holder::Recorder, false);
        }
        Some(now.id)
    }
}

/// The writer thread: the file, the transcript, and the stops (max length, low storage).
fn write(
    hub: &Arc<Hub>,
    id: &str,
    mut wav: std::fs::File,
    stt: Option<Arc<crate::voice::Stt>>,
    rx: &smpsc::Receiver<Vec<f32>>,
    rt: Option<tokio::runtime::Handle>,
) {
    let stream = stt.as_ref().map(|s| s.stream());
    let mut samples: u64 = 0;
    let mut last = Instant::now();
    let mut text = String::new();
    let mut buf: Vec<u8> = Vec::with_capacity(32 * 1024);
    // The level for the Recorder's meter: the loudest chunk since the last send, ~10 a second, only while
    // the display is on.
    let mut peak = 0f32;
    let mut last_level = Instant::now();
    let save = |samples: u64, text: &str, wav: &mut std::fs::File| {
        let bytes = 44 + samples * 2;
        let ms = i64::try_from(samples * 1000 / u64::from(RATE)).unwrap_or(0);
        let _ = fix_header(wav, samples);
        let _ = hub.core.with(|db| crate::media::set_recording(db, id, text, ms, i64::try_from(bytes).unwrap_or(0)));
        ms
    };
    loop {
        match rx.recv_timeout(EVERY) {
            Ok(chunk) => {
                buf.clear();
                for s in &chunk {
                    #[allow(clippy::cast_possible_truncation)]
                    let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                    buf.extend_from_slice(&v.to_le_bytes());
                }
                if wav.write_all(&buf).is_err() {
                    tracing::warn!(id, "recording: can't write; stopping");
                    break;
                }
                samples += chunk.len() as u64;
                if let (Some(s), Some(st)) = (&stt, &stream) {
                    s.feed_stream(st, &chunk);
                }
                peak = peak.max(crate::voice::meter(&chunk));
                if last_level.elapsed() >= LEVEL_EVERY {
                    last_level = Instant::now();
                    if hub.services.display_on() {
                        hub.emit(Event::RecordingLevel { level: peak }, Source::Api);
                    }
                    peak = 0.0;
                }
            }
            Err(smpsc::RecvTimeoutError::Timeout) => {}
            Err(smpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last.elapsed() >= EVERY {
            last = Instant::now();
            if let (Some(s), Some(st)) = (&stt, &stream) {
                text = s.text_of(st);
            }
            let ms = save(samples, &text, &mut wav);
            hub.emit(Event::Recording { id: id.to_owned(), state: "recording".into(), seconds: ms / 1000, text: text.clone() }, Source::Api);
            let too_long = Duration::from_millis(u64::try_from(ms).unwrap_or(0)) >= MAX;
            if too_long || free_bytes(&hub.cfg.data) < MIN_FREE {
                tracing::info!(id, too_long, "recording: stopping by itself");
                Recorder::stop(hub);
            }
        }
    }
    if let (Some(s), Some(st)) = (&stt, &stream) {
        text = s.finish_stream(st);
    }
    let ms = save(samples, &text, &mut wav);
    let _ = wav.sync_all();
    tracing::info!(id, seconds = ms / 1000, chars = text.len(), "recording saved");
    let words = text.split_whitespace().count();
    hub.emit(Event::Recording { id: id.to_owned(), state: "saved".into(), seconds: ms / 1000, text: text.clone() }, Source::Api);
    hub.emit(Event::MediaChanged { kind: "recording".into() }, Source::Api);
    if let Some(rt) = rt.filter(|_| words >= SUMMARY_MIN_WORDS && hub.core.recorder_summaries()) {
        let (h, id) = (hub.clone(), id.to_owned());
        rt.spawn(async move {
            if let Err(e) = summarize(&h, &id, &text).await {
                tracing::warn!(id, error = %format!("{e:#}"), "couldn't sum up the recording");
            }
        });
    }
}

/// Titles and sums up a tape from its words (T-068), and keeps them on it.
async fn summarize(hub: &Arc<Hub>, id: &str, text: &str) -> anyhow::Result<()> {
    let (title, summary) = summary_of(hub, text).await?;
    hub.core.with(|db| crate::media::set_summary(db, id, &title, &summary))?;
    hub.emit(Event::MediaChanged { kind: "recording".into() }, Source::Api);
    Ok(())
}

/// A title and a summary for a recording's words: one short no-tools turn on Haiku, in the agents' process budget.
/// Also `POST /harness/summarize`.
pub async fn summary_of(hub: &Hub, text: &str) -> anyhow::Result<(String, String)> {
    let words: String = text.chars().take(SUMMARY_CHARS).collect();
    let req = crate::agent::Request {
        model: "haiku".into(),
        effort: None,
        system_prompt: SUMMARY_PROMPT.into(),
        prompt: format!("The recording's words:

{words}"),
        timeout: Duration::from_secs(90),
    };
    let room = hub.sessions.room(hub, || {}).await;
    let out = crate::agent::claude_no_tools(&req, &mut |_: &str| {}).await;
    drop(room);
    let out = out?;
    let parsed = parse_summary(&out.text).ok_or_else(|| anyhow::anyhow!("not a summary: {}", out.text.chars().take(200).collect::<String>()))?;
    tracing::info!(cost_usd = ?out.cost_usd, ms = out.duration_ms, "a recording summed up");
    Ok(parsed)
}

/// `{title, summary}` from the model's reply, fenced or not.
fn parse_summary(reply: &str) -> Option<(String, String)> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let summary = v["summary"].as_str()?.trim().to_owned();
    (!summary.is_empty()).then(|| (v["title"].as_str().unwrap_or_default().trim().to_owned(), summary))
}

/// A WAV header for 16 kHz mono 16-bit PCM with `samples` samples.
fn wav_header(samples: u64) -> [u8; 44] {
    let data = u32::try_from(samples * 2).unwrap_or(u32::MAX - 36);
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&1u16.to_le_bytes());
    h[24..28].copy_from_slice(&RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(RATE * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// Rewrites the header for the samples written so far, then goes back to the end.
fn fix_header(wav: &mut std::fs::File, samples: u64) -> std::io::Result<()> {
    wav.seek(SeekFrom::Start(0))?;
    wav.write_all(&wav_header(samples))?;
    wav.seek(SeekFrom::End(0))?;
    Ok(())
}

/// Free space where r1d keeps its data.
fn free_bytes(path: &std::path::Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return u64::MAX };
        // SAFETY: a valid C string, and statvfs only writes the struct we pass.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &raw mut st) } == 0 {
            #[allow(clippy::unnecessary_cast)]
            return st.f_bavail as u64 * st.f_frsize as u64;
        }
    }
    let _ = path;
    u64::MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_come_out_of_the_reply() {
        let (t, s) = parse_summary("```json
{\"title\": \"Groceries\", \"summary\": \"Milk and eggs.\"}
```").unwrap();
        assert_eq!((t.as_str(), s.as_str()), ("Groceries", "Milk and eggs."));
        assert!(parse_summary("{\"title\": \"x\", \"summary\": \" \"}").is_none());
        assert!(parse_summary("no json here").is_none());
    }

    #[test]
    fn wav_headers_describe_the_samples() {
        let h = wav_header(16_000);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[40..44].try_into().unwrap()), 32_000);
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 32_036);
        assert_eq!(u32::from_le_bytes(h[24..28].try_into().unwrap()), 16_000);
    }
}
