//! The companion's USB layer for installing r1os on an R1 (ADR-0065, T-167; docs/specs/companion.md §11).
//!
//! - [`tools`]: Google's platform-tools, a pinned version fetched on first use and checked against its SHA-256 (not
//!   bundled: Google's terms).
//! - [`scan`]: the R1 in each mode it can be in: the MediaTek preloader, adb, the bootloader's fastboot, fastbootd,
//!   or plugged in with no driver for it yet.
//! - [`safety`]: what may be written. Every fastboot write passes through it; anything outside a short list of exact
//!   partition names is refused in code, and there's no way to ask for a relock at all.
//! - [`fastboot`] and [`adb`]: typed operations on one R1, never free-form arguments.
//!
//! An R1 is named by [`identity`], a hash of its serial: the serial itself (and the IMEI) never goes in a log, an
//! error or anything the companion shows.

pub mod adb;
pub mod backup;
pub mod download;
#[cfg(windows)]
pub mod drivers;
pub mod fastboot;
pub mod folders;
pub mod safety;
pub mod scan;
pub mod tools;

use sha2::{Digest, Sha256};

/// MediaTek's USB vendor id, which the R1 uses in every mode.
pub const VID_MEDIATEK: u16 = 0x0E8D;
/// The preloader's serial port, for about a second at power-on (the way into fastboot from power-off).
pub const PID_PRELOADER: u16 = 0x2000;
/// Android (adb) and the bootloader's fastboot share this one: Windows needs a driver picked for the latter.
pub const PID_ANDROID: u16 = 0x201C;

/// An R1's name in the companion: the first 12 hex digits of a hash of its USB serial. Stable, and it says nothing
/// about the serial.
pub fn identity(serial: &str) -> String {
    let d = Sha256::digest(format!("r1os-r1:{serial}").as_bytes());
    d.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_hides_the_serial() {
        let id = identity("ABCDEF0123456789");
        assert_eq!(id.len(), 12);
        assert!(!id.contains("ABCDEF"));
        assert_eq!(id, identity("ABCDEF0123456789"));
        assert_ne!(id, identity("ABCDEF0123456780"));
    }
}
