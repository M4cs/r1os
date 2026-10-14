//! The R1's boot: the boot animation r1d keeps (T-029) and verbose boot (T-101).
//!
//! - **The animation:** the host draws your agents into Android's `bootanimation.zip` when their looks change
//!   (`POST /api/bootanim`); r1d keeps it as `<data>/boot/bootanimation.zip`, which the image's init binds over its
//!   own (`/product/media/bootanimation.zip`) once /data is up, before the animation starts. `?default=1` keeps
//!   r1os's own cast as `default.zip` instead, which `tools/r1h bootanim` pulls for the image.
//! - **Verbose boot:** a file on /metadata (mounted before /data), `/metadata/r1os/verbose_boot`, which the image's
//!   `r1os-bootlog` looks for at `on init`: with it, the kernel's log is on the screen until SurfaceFlinger starts.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The verbose boot flag (the image makes /metadata/r1os).
pub const VERBOSE_FLAG: &str = "/metadata/r1os/verbose_boot";
/// The most an animation may be (Android maps it whole).
const MAX: usize = 24 << 20;

fn dir(data: &Path) -> PathBuf {
    data.join("boot")
}

fn file(data: &Path, default: bool) -> PathBuf {
    dir(data).join(if default { "default.zip" } else { "bootanimation.zip" })
}

/// What's set: `{verbose, animation: {at, bytes}?, default: {at, bytes}?}` (times in ms).
pub fn state(data: &Path) -> Value {
    let about = |p: PathBuf| {
        std::fs::metadata(&p).ok().map(|m| {
            let at = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
            json!({ "at": at, "bytes": m.len() })
        })
    };
    json!({
        "verbose": Path::new(VERBOSE_FLAG).exists(),
        "animation": about(file(data, false)),
        "default": about(file(data, true)),
    })
}

/// Verbose boot on or off (from the next boot).
pub fn set_verbose(on: bool) -> Result<()> {
    let flag = Path::new(VERBOSE_FLAG);
    let parent = flag.parent().context("no directory")?;
    // The image makes /metadata/r1os at boot (v0.2.15); r1d's domain may look in /metadata, not stat it.
    ensure!(parent.is_dir(), "verbose boot needs image v0.2.15 or later");
    if on {
        std::fs::write(flag, b"1\n")?;
    } else if flag.exists() {
        std::fs::remove_file(flag)?;
    }
    Ok(())
}

/// Keeps a boot animation the host drew: checked to be a zip with a `desc.txt`, written whole then put in place.
pub fn save(data: &Path, zip: &[u8], default: bool) -> Result<Value> {
    ensure!(zip.len() <= MAX, "the animation is over {} MB", MAX >> 20);
    ensure!(zip.starts_with(b"PK\x03\x04"), "that isn't a zip");
    ensure!(zip.windows(8).any(|w| w == b"desc.txt"), "a boot animation needs a desc.txt");
    std::fs::create_dir_all(dir(data))?;
    let to = file(data, default);
    let tmp = to.with_extension("zip.new");
    std::fs::write(&tmp, zip)?;
    // Android's boot animation reads it as `graphics`, through the image's bind mount (r1d's umask is 077).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    }
    std::fs::rename(&tmp, &to)?;
    tracing::info!(bytes = zip.len(), default, "boot animation: kept");
    Ok(state(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_boot_animation_is_kept() {
        let data = std::env::temp_dir().join(format!("r1os-boot-{}", std::process::id()));
        assert!(save(&data, b"not a zip", false).is_err());
        assert!(save(&data, b"PK\x03\x04 no description", false).is_err());
        let s = save(&data, b"PK\x03\x04....desc.txt....", false).unwrap();
        assert_eq!(s["animation"]["bytes"], 20);
        assert!(s["default"].is_null());
        let _ = std::fs::remove_dir_all(&data);
    }
}
