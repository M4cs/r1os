//! What the companion may write to an R1 (ADR-0065 decision 6; CLAUDE.md's device safety rules).
//!
//! The install writes what `tools/flash-gsi.sh` does and nothing more: verification off in the three `vbmeta`
//! images on both slots, `userdata` erased and written, `product_<slot>` deleted to make room, `system_<slot>`
//! flashed. A partition is allowed by its exact name, never a prefix. There is no relock operation: `fastboot
//! flashing lock` on a modified image can brick an R1, so it can't even be asked for. `boot_*` and `dtbo_*` stay out
//! until a decision needs them (Rabbit's kernel stays: ADR-0001).

use std::fmt;

/// One write to an R1 in fastboot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
    /// `fastboot flash <part> <image>`.
    Flash { part: String, image: std::path::PathBuf },
    /// `fastboot erase <part>`.
    Erase { part: String },
    /// `fastboot delete-logical-partition <part>` (fastbootd).
    DeleteLogical { part: String },
    /// `fastboot snapshot-update cancel` (fastbootd): drops a half-applied Rabbit update so `super` can be resized.
    CancelSnapshotUpdate,
    /// `fastboot flashing unlock`: wipes the R1's data; the R1 itself asks, on its side button.
    Unlock,
}

/// Partitions that may be flashed: the `vbmeta` images (verification off) and `system`, each on either slot, and
/// `userdata` (the stock empty image).
pub const FLASH: &[&str] = &[
    "vbmeta_a", "vbmeta_b", "vbmeta_system_a", "vbmeta_system_b", "vbmeta_vendor_a", "vbmeta_vendor_b", "system_a", "system_b",
    "userdata",
];
/// Partitions that may be erased.
pub const ERASE: &[&str] = &["userdata"];
/// Logical partitions that may be deleted (to make room in `super` for r1os's system).
pub const DELETE: &[&str] = &["product_a", "product_b"];

/// Never written by r1os, whatever asks (CLAUDE.md): the device-unique partitions (the IMEI and radio calibration
/// among them) and Rabbit's boot chain. Named in a refusal so it says why.
pub const NEVER: &[&str] = &[
    "preloader", "lk", "nvram", "nvdata", "nvcfg", "protect1", "protect2", "proinfo", "persist", "seccfg", "frp", "otp", "sec1",
    "md_udc", "para", "boot_para", "md1img", "tee", "gz", "scp", "sspm", "spmfw", "dpm", "init_boot", "vendor_boot", "mmcblk0boot",
];

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Whether [w] may be sent. Every write goes through this before fastboot runs.
pub fn check(w: &Write) -> Result<(), Refused> {
    let (part, allowed, what) = match w {
        Write::Flash { part, .. } => (part.as_str(), FLASH, "flashed"),
        Write::Erase { part } => (part.as_str(), ERASE, "erased"),
        Write::DeleteLogical { part } => (part.as_str(), DELETE, "deleted"),
        Write::CancelSnapshotUpdate | Write::Unlock => return Ok(()),
    };
    if allowed.contains(&part) {
        return Ok(());
    }
    let base = part.trim_end_matches("_a").trim_end_matches("_b");
    if NEVER.iter().any(|n| base == *n || base.starts_with(n)) {
        return Err(Refused(format!("`{part}` is never written by r1os: it's one of the R1's own (its identity, its radio, Rabbit's boot chain)")));
    }
    Err(Refused(format!("`{part}` can't be {what}: r1os's install only touches {}", allowed.join(", "))))
}

/// fastboot's arguments for [w], after [check]. The only way the companion builds a writing command.
pub fn args(w: &Write) -> Result<Vec<String>, Refused> {
    check(w)?;
    Ok(match w {
        Write::Flash { part, image } => vec!["flash".into(), part.clone(), image.to_string_lossy().into_owned()],
        Write::Erase { part } => vec!["erase".into(), part.clone()],
        Write::DeleteLogical { part } => vec!["delete-logical-partition".into(), part.clone()],
        Write::CancelSnapshotUpdate => vec!["snapshot-update".into(), "cancel".into()],
        Write::Unlock => vec!["flashing".into(), "unlock".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn flash(p: &str) -> Write {
        Write::Flash { part: p.into(), image: PathBuf::from("x.img") }
    }

    #[test]
    fn the_install_s_writes_go_through() {
        for p in ["vbmeta_a", "vbmeta_system_b", "vbmeta_vendor_a", "system_a", "system_b", "userdata"] {
            assert!(check(&flash(p)).is_ok(), "{p}");
        }
        assert!(check(&Write::Erase { part: "userdata".into() }).is_ok());
        assert!(check(&Write::DeleteLogical { part: "product_b".into() }).is_ok());
        assert!(check(&Write::CancelSnapshotUpdate).is_ok() && check(&Write::Unlock).is_ok());
    }

    #[test]
    fn everything_else_is_refused() {
        // The device's own and Rabbit's boot chain, by name.
        for p in ["nvram", "nvdata", "proinfo", "persist", "seccfg", "frp", "preloader_a", "lk_b", "md1img_a", "tee_a", "init_boot_a", "vendor_boot_b"] {
            let e = check(&flash(p)).unwrap_err();
            assert!(e.0.contains("never written"), "{p}: {e}");
        }
        // Allowed by exact name only: no prefix, no slotless `system`, nothing r1os's install doesn't write.
        for p in ["system", "system_c", "vbmeta_xyz", "boot_a", "dtbo_a", "super", "vendor_a", "metadata", "userdata2", ""] {
            assert!(check(&flash(p)).is_err(), "{p}");
        }
        assert!(check(&Write::Erase { part: "system_a".into() }).is_err());
        assert!(check(&Write::DeleteLogical { part: "system_a".into() }).is_err());
        assert!(check(&Write::DeleteLogical { part: "vendor_a".into() }).is_err());
    }

    #[test]
    fn no_command_relocks() {
        let all = [flash("system_a"), Write::Erase { part: "userdata".into() }, Write::DeleteLogical { part: "product_a".into() }, Write::CancelSnapshotUpdate, Write::Unlock];
        for w in all {
            let a = args(&w).unwrap();
            assert!(!a.iter().any(|x| x == "lock" || x == "lock_critical"), "{a:?}");
        }
        assert_eq!(args(&Write::Unlock).unwrap(), ["flashing", "unlock"]);
        assert!(args(&flash("nvram")).is_err());
    }
}
