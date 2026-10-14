//! Rabbit's stock firmware files the flash writes (ADR-0065 decision 6, T-170): the three `vbmeta` images (flashed with
//! verification off) and the empty `userdata` image, from Rabbit's v0.8.293 release, as `tools/flash-gsi.sh` takes
//! them. Only those four entries are read out of its 922 MB zip, by HTTP range requests (about 3.3 MB), and each is
//! checked against its SHA-256 before it's kept. A developer can point at the zip already extracted instead.

use crate::download::{hex, sha256_of};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

/// Rabbit's v0.8.293 release (2025-05-27); GitHub gives no digest, so its size is the pin, and each file's SHA-256.
pub const ZIP_URL: &str = "https://github.com/rabbit-hmi-oss/firmware/releases/download/v0.8.293/rabbit_OS_v0.8.293.zip";
pub const ZIP_SIZE: u64 = 967_477_397;
/// The folder in the zip holding the images.
pub const ZIP_DIR: &str = "rabbit_OS_v0.8.293_20250516110545";

/// One file the flash takes from Rabbit's firmware.
#[derive(Debug, Clone, Copy)]
pub struct File {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// The four, with their SHA-256s as Max's copy of the release has them (2026-10-14).
pub const FILES: &[File] = &[
    File { name: "vbmeta.img", size: 4096, sha256: "91fabaff6e5d61d7dbab02f181c36d4f9ab3aff9f833b92a6ddec92bf43dc6dc" },
    File { name: "vbmeta_system.img", size: 4096, sha256: "10d7c932c1b2974efcb35288e2ceea9d35aff2480a19a4882d86f0e2a245d373" },
    File { name: "vbmeta_vendor.img", size: 4096, sha256: "5b23d1e31f4b8b196b988ae1490a95fba649f8403dd2b11309efa612ac41ddfc" },
    File { name: "userdata.img", size: 7_356_684, sha256: "9b06c8b4ff198d6fa1dc92bcbb2d085bdd65a8a33e1f8c08c258d1d07e2b0981" },
];

/// A folder holding the four, checked.
#[derive(Debug, Clone)]
pub struct Stock {
    pub dir: PathBuf,
}

impl Stock {
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

/// The four in [dir], if every one is there and is the pinned file.
pub fn find(dir: &Path) -> Option<Stock> {
    FILES
        .iter()
        .all(|f| {
            let p = dir.join(f.name);
            std::fs::metadata(&p).map(|m| m.len() == f.size).unwrap_or(false) && sha256_of(&p).map(|h| h == f.sha256).unwrap_or(false)
        })
        .then(|| Stock { dir: dir.to_path_buf() })
}

/// Copies the four out of an extracted firmware folder ([src] holds them, or a folder under it does), checked.
pub fn from_folder(src: &Path, dir: &Path) -> Result<Stock> {
    let inner = src.join(ZIP_DIR);
    let src = if inner.join("vbmeta.img").is_file() { inner } else { src.to_path_buf() };
    std::fs::create_dir_all(dir)?;
    for f in FILES {
        let from = src.join(f.name);
        if sha256_of(&from).with_context(|| format!("no {} in {}", f.name, src.display()))? != f.sha256 {
            bail!("{} in {} isn't Rabbit's v0.8.293", f.name, src.display());
        }
        std::fs::copy(&from, dir.join(f.name))?;
    }
    find(dir).context("copied, but they didn't check out")
}

/// Fetches the four from Rabbit's release into [dir]: just their entries, by range requests. [progress] hears each
/// file's name as it's read.
pub fn fetch(dir: &Path, mut progress: impl FnMut(&str)) -> Result<Stock> {
    std::fs::create_dir_all(dir)?;
    let mut zip = zip::ZipArchive::new(Remote::new(ZIP_URL, ZIP_SIZE)).context("couldn't read Rabbit's firmware zip")?;
    for f in FILES {
        progress(f.name);
        let mut entry = zip.by_name(&format!("{ZIP_DIR}/{}", f.name)).with_context(|| format!("no {} in Rabbit's zip", f.name))?;
        let part = dir.join(format!("{}.part", f.name));
        let mut out = std::fs::File::create(&part)?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut got = 0u64;
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            got += n as u64;
        }
        drop(out);
        if got != f.size || hex(&hash.finalize()) != f.sha256 {
            let _ = std::fs::remove_file(&part);
            bail!("{} from Rabbit's zip isn't the file expected: not used", f.name);
        }
        std::fs::rename(&part, dir.join(f.name))?;
    }
    find(dir).context("fetched, but they didn't check out")
}

/// A file on a web server read by range requests: seekable, so a zip's central directory and a few entries can be read
/// without the rest. Reads a megabyte at a time.
struct Remote {
    url: &'static str,
    len: u64,
    pos: u64,
    buf: Vec<u8>,
    buf_at: u64,
}

impl Remote {
    const CHUNK: u64 = 1 << 20;

    fn new(url: &'static str, len: u64) -> Self {
        Self { url, len, pos: 0, buf: Vec::new(), buf_at: 0 }
    }

    fn load(&mut self) -> std::io::Result<()> {
        let end = (self.pos + Self::CHUNK).min(self.len) - 1;
        let resp = ureq::get(self.url)
            .set("Range", &format!("bytes={}-{end}", self.pos))
            .call()
            .map_err(|e| std::io::Error::other(format!("couldn't reach Rabbit's release: {e}")))?;
        if resp.status() != 206 {
            return Err(std::io::Error::other(format!("Rabbit's release answered {} to a range request", resp.status())));
        }
        let want = end - self.pos + 1;
        let mut buf = Vec::with_capacity(want as usize);
        resp.into_reader().take(want).read_to_end(&mut buf)?;
        if buf.len() as u64 != want {
            return Err(std::io::Error::other("a short read from Rabbit's release"));
        }
        self.buf = buf;
        self.buf_at = self.pos;
        Ok(())
    }
}

impl Read for Remote {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        if self.pos < self.buf_at || self.pos >= self.buf_at + self.buf.len() as u64 {
            self.load()?;
        }
        let from = (self.pos - self.buf_at) as usize;
        let n = out.len().min(self.buf.len() - from);
        out[..n].copy_from_slice(&self.buf[from..from + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Remote {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if pos < 0 {
            return Err(std::io::Error::other("seek before the start"));
        }
        self.pos = pos as u64;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_with_other_files_is_refused() {
        let d = std::env::temp_dir().join(format!("r1-usb-stock-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for f in FILES {
            std::fs::write(d.join(f.name), vec![0u8; f.size as usize]).unwrap();
        }
        assert!(find(&d).is_none());
        assert!(from_folder(&d, &d.join("out")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
