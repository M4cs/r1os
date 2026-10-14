//! Photos and recordings on the R1 (ADR-0031). The files live under `<data>/media` (`<id>.jpg`,
//! `<id>.wav`), and core.db's `media` table lists them: when, what kind, where they came from, a label,
//! and a recording's transcript.
//!
//! - **Photos** come from the host, which owns the camera: the Camera app, a chat's camera, and System One's
//!   "what am I looking at" (crate::system1). It posts the JPEG (`POST /api/media`).
//! - **Recordings** are written by r1d itself from the mic (crate::recorder).
//! - **Agents see them** through their tools (`list_media`, `view_photo`), and a chat message can carry
//!   photos to its agent (crate::chat).
//!
//! Nothing here leaves the R1 except what you send to an agent.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The largest photo the host may post.
pub const MAX_PHOTO: usize = 12 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Item {
    pub id: String,
    /// `photo` or `recording`.
    pub kind: String,
    /// When it was taken or started (ms).
    pub at: i64,
    pub mime: String,
    pub bytes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// Where it came from: `camera`, `chat`, `look` (System One), `recorder`, `journal`.
    pub source: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// A recording's marks (T-068): moments you clicked while it recorded, some with a photo.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    /// A recording's summary, when you asked the Recorder for them (T-068).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// A moment in a recording: how far in, and a photo taken then (its media id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mark {
    pub at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
}

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS media (
           id TEXT PRIMARY KEY, kind TEXT NOT NULL, at INTEGER NOT NULL, mime TEXT NOT NULL, bytes INTEGER NOT NULL,
           width INTEGER, height INTEGER, duration_ms INTEGER, source TEXT NOT NULL DEFAULT '', label TEXT NOT NULL DEFAULT '',
           transcript TEXT);
         CREATE INDEX IF NOT EXISTS media_at ON media(kind, at);",
    )?;
    crate::core::add_column(db, "media", "marks", "TEXT")?;
    crate::core::add_column(db, "media", "summary", "TEXT")
}

const COLUMNS: &str = "id, kind, at, mime, bytes, width, height, duration_ms, source, label, transcript, marks, summary";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        kind: r.get(1)?,
        at: r.get(2)?,
        mime: r.get(3)?,
        bytes: r.get(4)?,
        width: r.get(5)?,
        height: r.get(6)?,
        duration_ms: r.get(7)?,
        source: r.get(8)?,
        label: r.get(9)?,
        transcript: r.get(10)?,
        marks: r.get::<_, Option<String>>(11)?.and_then(|m| serde_json::from_str(&m).ok()).unwrap_or_default(),
        summary: r.get(12)?,
    })
}

/// Where the files are: `<data>/media`.
pub fn dir(data: &Path) -> PathBuf {
    data.join("media")
}

/// The file for an item.
pub fn file(data: &Path, item: &Item) -> PathBuf {
    let ext = if item.kind == "recording" { "wav" } else { "jpg" };
    dir(data).join(format!("{}.{ext}", item.id))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A new photo (a JPEG, as the host's camera made it): written, then listed.
pub fn add_photo(db: &Connection, data: &Path, jpeg: &[u8], source: &str) -> anyhow::Result<Item> {
    anyhow::ensure!(jpeg.len() <= MAX_PHOTO, "the photo is over {} MB", MAX_PHOTO >> 20);
    anyhow::ensure!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]), "that isn't a JPEG");
    let (width, height) = jpeg_size(jpeg).unzip();
    let item = Item {
        id: crate::core::random_hex(6),
        kind: "photo".into(),
        at: now_ms(),
        mime: "image/jpeg".into(),
        bytes: jpeg.len() as i64,
        width,
        height,
        duration_ms: None,
        source: clean(source, 20),
        label: String::new(),
        transcript: None,
        marks: Vec::new(),
        summary: None,
    };
    let path = file(data, &item);
    std::fs::create_dir_all(dir(data))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, jpeg)?;
    std::fs::rename(&tmp, &path)?;
    insert(db, &item)?;
    Ok(item)
}

pub fn insert(db: &Connection, i: &Item) -> rusqlite::Result<()> {
    db.execute(
        &format!("INSERT INTO media ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
        params![
            i.id, i.kind, i.at, i.mime, i.bytes, i.width, i.height, i.duration_ms, i.source, i.label, i.transcript,
            (!i.marks.is_empty()).then(|| serde_json::to_string(&i.marks).unwrap_or_default()), i.summary
        ],
    )?;
    Ok(())
}

/// Newest first; `kind` narrows it (`photo`, `recording`).
pub fn list(db: &Connection, kind: Option<&str>, limit: i64) -> rusqlite::Result<Vec<Item>> {
    let mut st = db.prepare(&format!(
        "SELECT {COLUMNS} FROM media WHERE ?1 IS NULL OR kind = ?1 ORDER BY at DESC LIMIT ?2"
    ))?;
    st.query_map(params![kind, limit.clamp(1, 500)], row)?.collect()
}

pub fn get(db: &Connection, id: &str) -> rusqlite::Result<Option<Item>> {
    db.query_row(&format!("SELECT {COLUMNS} FROM media WHERE id = ?1"), [id], row).optional()
}

/// Its label (a photo's caption, a recording's title).
pub fn set_label(db: &Connection, id: &str, label: &str) -> rusqlite::Result<bool> {
    Ok(db.execute("UPDATE media SET label = ?2 WHERE id = ?1", params![id, clean(label, 120)])? == 1)
}

/// A mark on a recording (T-068), kept in time order.
pub fn add_mark(db: &Connection, id: &str, mark: Mark) -> anyhow::Result<()> {
    let mut item = get(db, id)?.ok_or_else(|| anyhow::anyhow!("no recording `{id}`"))?;
    item.marks.push(mark);
    item.marks.sort_by_key(|m| m.at_ms);
    db.execute("UPDATE media SET marks = ?2 WHERE id = ?1", params![id, serde_json::to_string(&item.marks)?])?;
    Ok(())
}

/// A recording's summary (T-068), and its title if it hasn't one of its own.
pub fn set_summary(db: &Connection, id: &str, title: &str, summary: &str) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE media SET summary = ?2, label = CASE WHEN label = '' THEN ?3 ELSE label END WHERE id = ?1",
        params![id, summary.trim(), clean(title, 120)],
    )?;
    Ok(())
}

/// A recording's transcript so far (crate::recorder), and its length.
pub fn set_recording(db: &Connection, id: &str, transcript: &str, duration_ms: i64, bytes: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE media SET transcript = ?2, duration_ms = ?3, bytes = ?4 WHERE id = ?1",
        params![id, transcript, duration_ms, bytes],
    )?;
    Ok(())
}

/// Removes it and its file. Whether it was there.
pub fn remove(db: &Connection, data: &Path, id: &str) -> anyhow::Result<bool> {
    let Some(item) = get(db, id)? else { return Ok(false) };
    let _ = std::fs::remove_file(file(data, &item));
    db.execute("DELETE FROM media WHERE id = ?1", [id])?;
    Ok(true)
}

/// A recording's loudness, `per_s` values a second (0..1, as the live meters show it): what the Recorder's
/// and the Journal's meters play back. Reads the 16-bit mono WAV r1d wrote.
pub fn peaks(data: &Path, item: &Item, per_s: u32) -> anyhow::Result<Vec<f32>> {
    anyhow::ensure!(item.kind == "recording", "`{}` isn't a recording", item.id);
    let bytes = std::fs::read(file(data, item))?;
    Ok(wav_peaks(&bytes, per_s))
}

fn wav_peaks(bytes: &[u8], per_s: u32) -> Vec<f32> {
    let body = bytes.get(44..).unwrap_or_default();
    let window = (16_000 / per_s.clamp(1, 100)) as usize * 2;
    body.chunks(window)
        .map(|c| {
            let s: Vec<f32> = c.chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0).collect();
            (crate::voice::meter(&s) * 100.0).round() / 100.0
        })
        .collect()
}

/// A photo's bytes as base64, for an agent (Claude's image blocks, MCP image results).
pub fn photo_b64(data: &Path, item: &Item) -> anyhow::Result<String> {
    use base64::Engine;
    anyhow::ensure!(item.kind == "photo", "`{}` isn't a photo", item.id);
    let bytes = std::fs::read(file(data, item))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn clean(s: &str, max: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// A JPEG's width and height, from its first start-of-frame marker.
fn jpeg_size(b: &[u8]) -> Option<(i64, i64)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let marker = b[i + 1];
        let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        // SOF0–SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let h = i64::from(u16::from_be_bytes([b[i + 5], b[i + 6]]));
            let w = i64::from(u16::from_be_bytes([b[i + 7], b[i + 8]]));
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recordings_marks_and_summary_are_kept() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        // An older database without the columns gets them too.
        migrate(&db).unwrap();
        let item = Item {
            id: "r1".into(), kind: "recording".into(), at: 1, mime: "audio/wav".into(), bytes: 44, width: None, height: None,
            duration_ms: Some(0), source: "recorder".into(), label: String::new(), transcript: Some(String::new()),
            marks: Vec::new(), summary: None,
        };
        insert(&db, &item).unwrap();
        add_mark(&db, "r1", Mark { at_ms: 9000, photo: Some("p1".into()) }).unwrap();
        add_mark(&db, "r1", Mark { at_ms: 4000, photo: None }).unwrap();
        set_summary(&db, "r1", "Groceries", "Milk and eggs.").unwrap();
        let got = get(&db, "r1").unwrap().unwrap();
        assert_eq!(got.marks.iter().map(|m| m.at_ms).collect::<Vec<_>>(), vec![4000, 9000]);
        assert_eq!(got.marks[1].photo.as_deref(), Some("p1"));
        assert_eq!((got.label.as_str(), got.summary.as_deref()), ("Groceries", Some("Milk and eggs.")));
        // A title of your own stays.
        set_label(&db, "r1", "Shopping").unwrap();
        set_summary(&db, "r1", "Other", "Again.").unwrap();
        assert_eq!(get(&db, "r1").unwrap().unwrap().label, "Shopping");
        assert!(add_mark(&db, "nope", Mark { at_ms: 1, photo: None }).is_err());
    }

    #[test]
    fn peaks_follow_the_loudness() {
        // Half a second of silence, then half a second at about half scale.
        let mut wav = vec![0u8; 44];
        for i in 0..16_000 {
            let v: i16 = if i < 8000 { 0 } else if i % 2 == 0 { 16_000 } else { -16_000 };
            wav.extend_from_slice(&v.to_le_bytes());
        }
        let p = wav_peaks(&wav, 10);
        assert_eq!(p.len(), 10);
        assert!(p[0] < 0.05 && p[9] > 0.8, "{p:?}");
    }

    /// A 2×1 JPEG's header (SOI, APP0, then SOF0 with height 1 and width 2).
    const TINY: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0xFF, 0xC0, 0x00, 0x0B, 8, 0x00, 0x01,
        0x00, 0x02, 1, 1, 0x11, 0, 0xFF, 0xD9,
    ];

    #[test]
    fn photos_are_kept_listed_and_removed() {
        let data = std::env::temp_dir().join(format!("r1d-media-{}", crate::core::random_hex(4)));
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        assert!(add_photo(&db, &data, b"not a jpeg", "chat").is_err());
        let p = add_photo(&db, &data, TINY, "chat").unwrap();
        assert_eq!((p.width, p.height), (Some(2), Some(1)));
        assert!(file(&data, &p).exists());
        assert_eq!(list(&db, Some("photo"), 10).unwrap(), vec![p.clone()]);
        assert!(list(&db, Some("recording"), 10).unwrap().is_empty());
        assert!(set_label(&db, &p.id, "  the  fridge ").unwrap());
        assert_eq!(get(&db, &p.id).unwrap().unwrap().label, "the fridge");
        assert!(!photo_b64(&data, &p).unwrap().is_empty());
        assert!(remove(&db, &data, &p.id).unwrap());
        assert!(!file(&data, &p).exists());
        let _ = std::fs::remove_dir_all(data);
    }
}
