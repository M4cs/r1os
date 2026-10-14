//! Raw input from the R1's evdev nodes.
//!
//! The side button and wheel are read straight from their kernel nodes rather than through
//! Android's input stack: Android swallows the button press that wakes the screen, and the shell
//! should see every edge in every screen state (r1ptt; docs/research §1).
//!
//! | Device (`/sys/class/input/eventN/device/name`) | Events |
//! |---|---|
//! | `och1970_holl_key` (wheel)  | EV_KEY KEY_UP (103) / KEY_DOWN (108), a down+up pair per detent |
//! | `mtk-kpd` (side button)     | EV_KEY KEY_POWER (116), value 1 = down, 0 = up |
//!
//! Devices aren't grabbed (no EVIOCGRAB), so Android still sees them: the button's WAKE flag
//! (device/magisk/r1os-base keylayout) keeps waking the screen.

use crate::gesture::Raw;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Devices {
    pub wheel: Option<String>,
    pub button: Option<String>,
}

#[cfg(any(target_os = "android", target_os = "linux"))]
mod imp {
    use super::*;
    use std::fs::{self, File};
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    const WHEEL_NAME: &str = "och1970_holl_key";
    const BUTTON_NAME: &str = "mtk-kpd";
    const EV_KEY: u16 = 1;
    const KEY_UP: u16 = 103;
    const KEY_DOWN: u16 = 108;
    const KEY_POWER: u16 = 116;
    /// arm64 `struct input_event`: timeval (2 × i64), type u16, code u16, value i32.
    const EVENT_SIZE: usize = 24;

    pub fn find(name: &str) -> Option<PathBuf> {
        let entries = fs::read_dir("/sys/class/input").ok()?;
        let mut nodes: Vec<_> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("event"))
            .collect();
        nodes.sort();
        nodes.into_iter().find_map(|n| {
            let dev_name = fs::read_to_string(format!("/sys/class/input/{n}/device/name")).ok()?;
            (dev_name.trim() == name).then(|| Path::new("/dev/input").join(&n))
        })
    }

    pub fn spawn(tx: UnboundedSender<Raw>) -> Devices {
        let wheel = find(WHEEL_NAME);
        let button = find(BUTTON_NAME);
        if let Some(p) = &wheel {
            reader(p.clone(), tx.clone(), |code, value| match (code, value) {
                (KEY_UP, 1) => Some(Raw::Wheel(-1)),
                (KEY_DOWN, 1) => Some(Raw::Wheel(1)),
                _ => None,
            });
        }
        if let Some(p) = &button {
            reader(p.clone(), tx, |code, value| match (code, value) {
                (KEY_POWER, 1) => Some(Raw::Button(true)),
                (KEY_POWER, 0) => Some(Raw::Button(false)),
                _ => None, // value 2 = autorepeat
            });
        }
        Devices {
            wheel: wheel.map(|p| p.display().to_string()),
            button: button.map(|p| p.display().to_string()),
        }
    }

    /// One blocking thread per node. Reopens the node if a read fails (e.g. across suspend).
    fn reader(path: PathBuf, tx: UnboundedSender<Raw>, map: fn(u16, i32) -> Option<Raw>) {
        std::thread::Builder::new()
            .name(format!("evdev:{}", path.display()))
            .spawn(move || {
                loop {
                    match File::open(&path) {
                        Ok(mut f) => {
                            tracing::info!(node = %path.display(), "reading input");
                            let mut buf = [0u8; EVENT_SIZE];
                            while f.read_exact(&mut buf).is_ok() {
                                let typ = u16::from_le_bytes([buf[16], buf[17]]);
                                let code = u16::from_le_bytes([buf[18], buf[19]]);
                                let value = i32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
                                if typ == EV_KEY
                                    && let Some(raw) = map(code, value)
                                    && tx.send(raw).is_err()
                                {
                                    return; // r1d is shutting down
                                }
                            }
                            tracing::warn!(node = %path.display(), "input read failed; reopening");
                        }
                        Err(e) => tracing::warn!(node = %path.display(), error = %e, "can't open input node"),
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .expect("spawn evdev reader");
    }
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
mod imp {
    use super::*;

    /// No evdev here (the desktop simulator): input only arrives through the harness.
    pub fn spawn(_tx: UnboundedSender<Raw>) -> Devices {
        Devices::default()
    }
}

/// Starts a reader thread per device found and returns where they are.
pub fn spawn(tx: UnboundedSender<Raw>) -> Devices {
    imp::spawn(tx)
}
