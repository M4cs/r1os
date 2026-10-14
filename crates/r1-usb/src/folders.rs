//! Where the companion keeps things, from Windows' known folders rather than the environment, which can be unset or
//! point elsewhere.
//!
//! A process inside a packaged (MSIX) app, such as a command the Claude desktop app runs, has its writes under
//! `AppData\Local` redirected to `AppData\Local\Packages\<app>\LocalCache\Local`, which nothing outside that app sees.
//! What's fetched there isn't found from another terminal (2026-10-14), so fetch from where it runs.

use std::path::PathBuf;

/// `%LOCALAPPDATA%\r1os\companion`: what the companion fetches (platform-tools, drivers, the backup's tools).
pub fn companion() -> PathBuf {
    local_app_data().join("r1os").join("companion")
}

/// The user's local application data (`AppData\Local`).
pub fn local_app_data() -> PathBuf {
    #[cfg(windows)]
    if let Some(p) = known(&windows_sys::Win32::UI::Shell::FOLDERID_LocalAppData) {
        return p;
    }
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"))
}

/// The user's Documents, wherever Windows keeps it.
pub fn documents() -> PathBuf {
    #[cfg(windows)]
    if let Some(p) = known(&windows_sys::Win32::UI::Shell::FOLDERID_Documents) {
        return p;
    }
    home().join("Documents")
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

#[cfg(windows)]
fn known(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    let mut p: *mut u16 = std::ptr::null_mut();
    // SAFETY: the path Windows gives back is NUL-terminated, and freed here whatever the result.
    unsafe {
        let hr = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as _, std::ptr::null_mut(), &mut p);
        let out = (hr >= 0 && !p.is_null()).then(|| {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(p, len)))
        });
        CoTaskMemFree(p as _);
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_folders_without_the_environment() {
        // Windows answers whatever the environment says.
        let (local, docs) = (super::local_app_data(), super::documents());
        assert!(local.is_absolute() && docs.is_absolute());
        assert!(super::companion().ends_with("r1os/companion") || super::companion().ends_with(r"r1os\companion"));
    }
}
