//! Pinned downloads: a URL, its size and its SHA-256, checked before the file is kept (platform-tools, Rabbit's
//! MediaTek driver, uv, mtkclient, UsbDk). A download that isn't byte-for-byte the pinned one is thrown away.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

/// One pinned file.
#[derive(Debug, Clone, Copy)]
pub struct Pin {
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// Fetches [pin] to [to] (via `<to>.part`), checking its size and SHA-256. [progress] hears bytes so far and the total.
pub fn fetch(pin: &Pin, to: &Path, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = to.with_extension("part");
    let resp = ureq::get(pin.url).call().with_context(|| format!("couldn't reach {}", pin.url))?;
    let mut body = resp.into_reader().take(pin.size + 1);
    let mut file = std::fs::File::create(&part)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0u64;
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        got += n as u64;
        progress(got, pin.size);
    }
    drop(file);
    if got != pin.size || hex(&hash.finalize()) != pin.sha256 {
        let _ = std::fs::remove_file(&part);
        bail!("{} isn't the file expected ({got} bytes): not used", pin.url);
    }
    let _ = std::fs::remove_file(to);
    std::fs::rename(&part, to)?;
    Ok(to.to_path_buf())
}

/// A file's SHA-256, as hex.
pub fn sha256_of(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex(&hash.finalize()))
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Unpacks [zip_path] into [to], refusing any entry that would land outside it; with [only], just the entries named.
pub fn unzip(zip_path: &Path, to: &Path, only: Option<&[&str]>) -> Result<usize> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    let mut n = 0;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let Some(rel) = f.enclosed_name() else { bail!("{} has an entry outside its folder", zip_path.display()) };
        if let Some(only) = only
            && !only.iter().any(|o| rel.as_os_str() == *o)
        {
            continue;
        }
        let out = to.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::io::copy(&mut f, &mut std::fs::File::create(&out)?)?;
        n += 1;
    }
    Ok(n)
}
