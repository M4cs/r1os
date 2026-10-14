//! Google's platform-tools (`adb`, `fastboot`), fetched on first use (ADR-0065 decision 3): a pinned version from
//! Google's own repository, its size and SHA-256 checked before anything is unpacked, kept in the companion's data
//! folder. Not bundled with the companion: Google's terms cover its binaries, and the user accepts them by fetching.
//!
//! Only what `adb` and `fastboot` need is unpacked (the executables and the three DLLs beside them on Windows).

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

/// A pinned download.
pub struct Pinned {
    pub version: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// The files kept from the archive's `platform-tools/` folder.
    pub keep: &'static [&'static str],
}

/// Google's terms for the SDK's binaries, linked where the companion fetches them.
pub const TERMS: &str = "https://developer.android.com/studio/terms";

/// platform-tools 37.0.1 for Windows: the size and SHA-1 in Google's `repository2-3.xml`, the SHA-256 computed from
/// that same file (2026-10-14).
#[cfg(windows)]
pub const PLATFORM_TOOLS: Option<Pinned> = Some(Pinned {
    version: "37.0.1",
    url: "https://dl.google.com/android/repository/platform-tools_r37.0.1-win.zip",
    size: 8_044_989,
    sha256: "45f4d63113e895ebde0c90f194099a4676b6ac653bd28d54314a9e022bbc1a99",
    keep: &["adb.exe", "fastboot.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll", "libwinpthread-1.dll"],
});
/// macOS and Linux come with the companion's builds for them (T-022).
#[cfg(not(windows))]
pub const PLATFORM_TOOLS: Option<Pinned> = None;

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// Where `adb` and `fastboot` are, once fetched.
#[derive(Debug, Clone)]
pub struct Tools {
    pub adb: PathBuf,
    pub fastboot: PathBuf,
    pub version: String,
}

/// The tools in [dir] (`<dir>/platform-tools/`), if they're there and are the pinned version.
pub fn find(dir: &Path) -> Option<Tools> {
    let pin = PLATFORM_TOOLS.as_ref()?;
    let home = dir.join("platform-tools");
    let version = std::fs::read_to_string(home.join("VERSION")).ok()?;
    let tools = Tools { adb: home.join(format!("adb{EXE}")), fastboot: home.join(format!("fastboot{EXE}")), version: version.trim().to_owned() };
    (tools.version == pin.version && tools.adb.is_file() && tools.fastboot.is_file()).then_some(tools)
}

/// Fetches the pinned platform-tools into [dir], checking the download before unpacking it. [progress] hears the bytes
/// so far and the total.
pub fn fetch(dir: &Path, mut progress: impl FnMut(u64, u64)) -> Result<Tools> {
    let Some(pin) = PLATFORM_TOOLS.as_ref() else { bail!("platform-tools aren't pinned for this system yet") };
    std::fs::create_dir_all(dir)?;
    let zip_path = dir.join("platform-tools.zip.part");
    {
        let resp = ureq::get(pin.url).call().with_context(|| format!("couldn't reach {}", pin.url))?;
        let mut body = resp.into_reader().take(pin.size + 1);
        let mut file = std::fs::File::create(&zip_path)?;
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
        let sum: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != pin.size || sum != pin.sha256 {
            drop(file);
            let _ = std::fs::remove_file(&zip_path);
            bail!("the platform-tools download isn't the one expected ({got} bytes): not used");
        }
    }
    let home = dir.join("platform-tools");
    let staging = dir.join("platform-tools.new");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    unpack(&zip_path, &staging, pin.keep)?;
    std::fs::write(staging.join("VERSION"), pin.version)?;
    let _ = std::fs::remove_dir_all(&home);
    std::fs::rename(&staging, &home)?;
    let _ = std::fs::remove_file(&zip_path);
    find(dir).context("platform-tools unpacked but not found")
}

/// The files named in [keep] from the archive's `platform-tools/` folder, into [to].
fn unpack(zip_path: &Path, to: &Path, keep: &[&str]) -> Result<()> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    let mut found = 0;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let Some(name) = f.name().strip_prefix("platform-tools/").map(str::to_owned) else { continue };
        if !keep.contains(&name.as_str()) {
            continue;
        }
        let mut out = std::fs::File::create(to.join(&name))?;
        std::io::copy(&mut f, &mut out)?;
        found += 1;
    }
    if found != keep.len() {
        bail!("platform-tools' archive is missing files ({found} of {})", keep.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_found_where_nothing_was_fetched() {
        let dir = std::env::temp_dir().join(format!("r1-usb-test-{}", std::process::id()));
        assert!(find(&dir).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn the_pin_is_whole() {
        let p = PLATFORM_TOOLS.as_ref().unwrap();
        assert_eq!(p.sha256.len(), 64);
        assert!(p.url.starts_with("https://dl.google.com/android/repository/"));
        assert!(p.keep.contains(&"adb.exe") && p.keep.contains(&"fastboot.exe"));
    }
}
