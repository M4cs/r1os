//! What the companion may write to an R1 (ADR-0065 decision 6, ADR-0067; CLAUDE.md's device safety rules).
//!
//! The install writes from the bootloader's fastboot and nothing more: verification off in the three `vbmeta` images on
//! both slots, `super` rewritten in place by [`crate::superimg`] (r1os's system in; the R1's own vendor never written),
//! and `userdata` erased and written. Nothing goes through fastbootd (ADR-0067, after T-182): no `system_*` or
//! `product_*` by name, no snapshot cancel. A partition is allowed by its exact name, never a prefix. There is no relock
//! operation: `fastboot flashing lock` on a modified image can brick an R1, so it can't even be asked for. `boot_*` and
//! `dtbo_*` stay out until a decision needs them (Rabbit's kernel stays: ADR-0001).

use std::fmt;

/// One write to an R1 in the bootloader's fastboot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
    /// `fastboot flash <part> <image>`.
    Flash { part: String, image: std::path::PathBuf },
    /// `fastboot erase <part>`.
    Erase { part: String },
    /// `fastboot flashing unlock`: wipes the R1's data; the R1 itself asks, on its side button.
    Unlock,
    /// `fastboot flash super <image>` (ADR-0067): only an image [`crate::superimg`] built and verified (the R1's
    /// vendor never written), which nothing else can make.
    FlashSuper(crate::superimg::SuperImage),
}

/// Partitions that may be flashed by name: the `vbmeta` images (verification off) on either slot, and `userdata` (the
/// stock empty image). `super` only as [Write::FlashSuper].
pub const FLASH: &[&str] = &["vbmeta_a", "vbmeta_b", "vbmeta_system_a", "vbmeta_system_b", "vbmeta_vendor_a", "vbmeta_vendor_b", "userdata"];
/// Partitions that may be erased.
pub const ERASE: &[&str] = &["userdata"];

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
        Write::Unlock | Write::FlashSuper(_) => return Ok(()),
    };
    if allowed.contains(&part) {
        return Ok(());
    }
    let base = part.trim_end_matches("_a").trim_end_matches("_b");
    if NEVER.iter().any(|n| base == *n || base.starts_with(n)) {
        return Err(Refused(format!("`{part}` is never written by r1os: it's one of the R1's own (its identity, its radio, Rabbit's boot chain)")));
    }
    if part == "super" {
        return Err(Refused("`super` is written only as an image r1os built and verified (ADR-0067)".into()));
    }
    Err(Refused(format!("`{part}` can't be {what}: r1os's install only touches {}", allowed.join(", "))))
}

/// fastboot's arguments for [w], after [check]. The only way the companion builds a writing command.
pub fn args(w: &Write) -> Result<Vec<String>, Refused> {
    check(w)?;
    Ok(match w {
        Write::Flash { part, image } => vec!["flash".into(), part.clone(), image.to_string_lossy().into_owned()],
        Write::Erase { part } => vec!["erase".into(), part.clone()],
        Write::Unlock => vec!["flashing".into(), "unlock".into()],
        Write::FlashSuper(image) => vec!["flash".into(), "super".into(), image.path().to_string_lossy().into_owned()],
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
        for p in ["vbmeta_a", "vbmeta_system_b", "vbmeta_vendor_a", "userdata"] {
            assert!(check(&flash(p)).is_ok(), "{p}");
        }
        assert!(check(&Write::Erase { part: "userdata".into() }).is_ok());
        assert!(check(&Write::Unlock).is_ok());
    }

    #[test]
    fn everything_else_is_refused() {
        // The device's own and Rabbit's boot chain, by name.
        for p in ["nvram", "nvdata", "proinfo", "persist", "seccfg", "frp", "para", "md_udc", "preloader_a", "lk_b", "md1img_a", "tee_a", "init_boot_a", "vendor_boot_b"] {
            let e = check(&flash(p)).unwrap_err();
            assert!(e.0.contains("never written"), "{p}: {e}");
        }
        // Not by name: super (only a built image), and fastbootd's old writes (ADR-0067).
        assert!(check(&flash("super")).unwrap_err().0.contains("ADR-0067"));
        for p in ["system_a", "system_b", "product_a", "system", "system_c", "vbmeta_xyz", "boot_a", "dtbo_a", "vendor_a", "metadata", "userdata2", ""] {
            assert!(check(&flash(p)).is_err(), "{p}");
        }
        assert!(check(&Write::Erase { part: "system_a".into() }).is_err());
        assert!(check(&Write::Erase { part: "md_udc".into() }).is_err(), "/metadata on the R1");
    }

    #[test]
    fn no_command_relocks() {
        let all = [flash("vbmeta_a"), Write::Erase { part: "userdata".into() }, Write::Unlock];
        for w in all {
            let a = args(&w).unwrap();
            assert!(!a.iter().any(|x| x == "lock" || x == "lock_critical"), "{a:?}");
        }
        assert_eq!(args(&Write::Unlock).unwrap(), ["flashing", "unlock"]);
        assert!(args(&flash("nvram")).is_err());
    }
}
