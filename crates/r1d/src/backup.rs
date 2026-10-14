//! Backing r1os's data up to a linked computer (docs/specs/companion.md §6, About): core.db (agents, chats,
//! memories, the journal, alarms, the look; a snapshot taken as the backup starts), the apps' specs and data, photos
//! and recordings, and the trash. Never the agent box (the CLIs' logins and the agents' workspaces), the link's key,
//! r1d's binaries and libraries, models, caches or logs: no partitions, no credentials.
//!
//! The companion asks for the list (`GET /api/backup`), then reads each file in parts (`GET /api/backup/file`), so a
//! big recording never has to fit in one message over the link.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

/// What's backed up besides core.db, under `<data>`.
pub const ROOTS: &[&str] = &["apps", "data", "media", "trash"];
/// The most one part of a file holds.
pub const PART: u64 = 8 << 20;

/// Where core.db's snapshot goes while a backup reads it.
fn snapshot(data: &Path) -> PathBuf {
    data.join("cache").join("backup-core.db")
}

/// Takes core.db's snapshot and lists everything a backup holds: `{files: [{path, bytes}], bytes, at}`.
pub fn manifest(hub: &crate::hub::Hub) -> Result<Value> {
    let data = &hub.cfg.data;
    let snap = snapshot(data);
    std::fs::create_dir_all(snap.parent().context("cache")?)?;
    let _ = std::fs::remove_file(&snap);
    hub.core.with(|db| db.execute("VACUUM INTO ?1", [snap.to_string_lossy().as_ref()])).context("snapshot core.db")?;
    let mut files = vec![json!({ "path": "core.db", "bytes": std::fs::metadata(&snap)?.len() })];
    for root in ROOTS {
        walk(data, &data.join(root), &mut files)?;
    }
    let bytes: u64 = files.iter().filter_map(|f| f["bytes"].as_u64()).sum();
    Ok(json!({ "files": files, "bytes": bytes, "at": crate::core::now() }))
}

fn walk(data: &Path, dir: &Path, out: &mut Vec<Value>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    for e in entries.flatten() {
        let kind = e.file_type()?;
        if kind.is_dir() {
            walk(data, &e.path(), out)?;
        } else if kind.is_file() {
            let rel = e.path().strip_prefix(data)?.to_string_lossy().replace('\\', "/");
            out.push(json!({ "path": rel, "bytes": e.metadata()?.len() }));
        }
    }
    Ok(())
}

/// The file a backup path names, if it's one a backup holds.
fn file(data: &Path, path: &str) -> Result<PathBuf> {
    if path == "core.db" {
        return Ok(snapshot(data));
    }
    let p = Path::new(path);
    let ok = p.components().all(|c| matches!(c, Component::Normal(_)))
        && p.components().next().is_some_and(|c| ROOTS.iter().any(|r| c.as_os_str() == *r));
    if !ok {
        bail!("`{path}` isn't part of a backup");
    }
    Ok(data.join(p))
}

/// A part of one of the backup's files: from `offset`, at most `len` bytes (and at most [`PART`]).
pub fn read(data: &Path, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
    let mut f = std::fs::File::open(file(data, path)?).with_context(|| format!("open `{path}`"))?;
    f.seek(SeekFrom::Start(offset))?;
    let mut out = Vec::new();
    f.take(len.min(PART)).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_a_backup_holds() {
        let dir = std::env::temp_dir().join(format!("r1d-backup-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("media")).unwrap();
        std::fs::write(dir.join("media/a.jpg"), b"0123456789").unwrap();
        assert_eq!(read(&dir, "media/a.jpg", 3, 4).unwrap(), b"3456");
        assert_eq!(read(&dir, "media/a.jpg", 8, 100).unwrap(), b"89");
        for bad in ["agent/home/.claude.json", "link/key", "../etc/passwd", "media/../link/key", "/data/r1os/core.db", ""] {
            assert!(read(&dir, bad, 0, 10).is_err(), "{bad}");
        }
        let mut files = Vec::new();
        walk(&dir, &dir.join("media"), &mut files).unwrap();
        assert_eq!(files, vec![json!({ "path": "media/a.jpg", "bytes": 10 })]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
