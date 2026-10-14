//! Flashing r1os onto an R1 (ADR-0065 decision 6, T-170): `tools/flash-gsi.sh`'s writes in its order, each one through
//! [`crate::safety`], with the checks ADR-0065 asks for before any wipe.
//!
//! - **Checked first** ([`check`], which writes nothing): the bootloader reports `unlocked`; the active slot; that
//!   slot's `vendor_<slot>` has a real size, read in fastbootd where logical partitions live (an R1 that took Rabbit's
//!   update and then lost its data had `vendor_b` at 0 bytes, and never booted that slot: research 2026-10-09).
//! - **The plan** ([`Plan::steps`]) is shown, and runs only when [`CONFIRM`] is typed.
//! - **The writes**, on the active slot (where the R1's kernel and vendor are): verification off in `vbmeta`,
//!   `vbmeta_system` and `vbmeta_vendor` on both slots; `userdata` erased and Rabbit's empty image written; then in
//!   fastbootd, Rabbit's half-applied update cancelled, `product_<slot>` deleted (Rabbit's launcher and services: room
//!   in `super`, and no battery spent on them), and r1os's system flashed. Nothing else: no `boot_*`, no relock.

use crate::fastboot::{Fastboot, Reboot};
use crate::safety::Write;
use crate::stock::Stock;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// What has to be typed for the flash to go ahead: it erases everything on the R1.
pub const CONFIRM: &str = "ERASE";

/// An R1 in fastboot, as the flash sees it: [`Fastboot`], or a stand-in in tests.
pub trait Target {
    fn getvar(&self, name: &str) -> Result<Option<String>>;
    fn reboot(&self, to: Reboot) -> Result<()>;
    /// One write, through [`crate::safety`].
    fn write(&self, w: &Write, progress: &mut dyn FnMut(&str)) -> Result<()>;
    /// Waits for it to be back in fastboot after a reboot.
    fn wait(&self) -> Result<()>;
}

impl Target for Fastboot {
    fn getvar(&self, name: &str) -> Result<Option<String>> {
        Fastboot::getvar(self, name)
    }

    fn reboot(&self, to: Reboot) -> Result<()> {
        Fastboot::reboot(self, to)
    }

    fn write(&self, w: &Write, progress: &mut dyn FnMut(&str)) -> Result<()> {
        Fastboot::write(self, w, progress)
    }

    fn wait(&self) -> Result<()> {
        Fastboot::wait(self, std::time::Duration::from_secs(90))
    }
}

/// What [check] found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Checked {
    /// The active slot, which the flash writes.
    pub slot: char,
    /// `vendor_<slot>`'s size.
    pub vendor_bytes: u64,
}

fn userspace(t: &dyn Target) -> Result<bool> {
    Ok(t.getvar("is-userspace")?.as_deref() == Some("yes"))
}

fn unlocked(t: &dyn Target) -> Result<()> {
    match t.getvar("unlocked")?.as_deref() {
        Some("yes") => Ok(()),
        Some(_) => bail!("the bootloader is locked: unlock it first (Rabbit's portal, then `fastboot flashing unlock`)"),
        None => bail!("the bootloader didn't say whether it's unlocked"),
    }
}

/// Reads what has to hold before any write. Writes nothing; it reboots into fastbootd to see `vendor_<slot>`, and
/// leaves the R1 there.
pub fn check(t: &dyn Target) -> Result<Checked> {
    unlocked(t)?;
    let slot = match t.getvar("current-slot")?.as_deref() {
        Some("a") | Some("_a") => 'a',
        Some("b") | Some("_b") => 'b',
        other => bail!("couldn't read the active slot (got {other:?})"),
    };
    if !userspace(t)? {
        t.reboot(Reboot::Fastbootd)?;
        t.wait().context("the R1 didn't come back in fastbootd (on Windows, check its driver)")?;
        if !userspace(t)? {
            bail!("the R1 didn't reach fastbootd");
        }
    }
    let size = t.getvar(&format!("partition-size:vendor_{slot}"))?.context("no size for the active slot's vendor")?;
    let vendor_bytes = u64::from_str_radix(size.trim_start_matches("0x").trim_start_matches("0X"), 16).with_context(|| format!("vendor's size isn't a number: {size}"))?;
    if vendor_bytes == 0 {
        bail!("vendor_{slot} is empty: slot {slot} wouldn't boot after the flash. Rabbit's restore puts it back; don't flash this slot");
    }
    Ok(Checked { slot, vendor_bytes })
}

/// One step of the flash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Into the bootloader's fastboot (if it isn't there), and `unlocked` read again right before the wipe.
    Bootloader,
    /// Into fastbootd, where logical partitions are written.
    Fastbootd,
    Write(Write),
    /// A write that may fail without stopping the flash (nothing to cancel; `product` already gone).
    MayFail(Write),
    /// Into r1os.
    Boot,
}

/// The flash for an R1 that passed [check]: r1os's [system] image onto [Checked::slot], with Rabbit's [stock] files.
#[derive(Debug, Clone)]
pub struct Plan {
    pub slot: char,
    pub system: PathBuf,
    pub stock: Stock,
    /// The `vbmeta` copies with verification off ([vbmeta_disabled]) go here.
    pub work: PathBuf,
    /// Keep `product_<slot>` (Rabbit's launcher and services) rather than deleting it.
    pub keep_product: bool,
}

impl Plan {
    fn vbmeta(&self, name: &str) -> PathBuf {
        self.work.join(format!("{name}-noverify.img"))
    }

    /// Every step, in order. [run] does exactly these.
    pub fn steps(&self) -> Vec<Step> {
        let s = self.slot;
        let mut steps = vec![Step::Bootloader];
        for v in ["vbmeta", "vbmeta_system", "vbmeta_vendor"] {
            for slot in ['a', 'b'] {
                steps.push(Step::Write(Write::Flash { part: format!("{v}_{slot}"), image: self.vbmeta(v) }));
            }
        }
        steps.push(Step::Write(Write::Erase { part: "userdata".into() }));
        steps.push(Step::Write(Write::Flash { part: "userdata".into(), image: self.stock.path("userdata.img") }));
        steps.push(Step::Fastbootd);
        steps.push(Step::MayFail(Write::CancelSnapshotUpdate));
        if !self.keep_product {
            steps.push(Step::MayFail(Write::DeleteLogical { part: format!("product_{s}") }));
        }
        steps.push(Step::Write(Write::Flash { part: format!("system_{s}"), image: self.system.clone() }));
        steps.push(Step::Boot);
        steps
    }

    /// The plan in words, for the screen before [CONFIRM] is typed.
    pub fn describe(&self) -> Vec<String> {
        let s = self.slot;
        let mut lines = vec![
            "Verification off in vbmeta, vbmeta_system and vbmeta_vendor, on slots a and b".to_owned(),
            "ERASE userdata (everything on the R1) and write Rabbit's empty userdata".to_owned(),
        ];
        if !self.keep_product {
            lines.push(format!("In fastbootd: delete product_{s} (Rabbit's launcher and services)"));
        }
        lines.push(format!(
            "Flash {} to system_{s}",
            self.system.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ));
        lines.push(format!("Untouched: the kernel (boot), vendor, dtbo, the bootloader, and everything that's this R1's own. Slot {s}."));
        lines
    }
}

/// A copy of a `vbmeta` image with verification off: the AVB header's flags (big-endian, at byte 120) set to 3,
/// `HASHTREE_DISABLED | VERIFICATION_DISABLED`, the edit `fastboot --disable-verity --disable-verification` makes (as
/// `tools/lib.sh`'s `vbmeta_disabled`). The R1 only boots a system that isn't Rabbit's this way.
pub fn vbmeta_disabled(src: &Path, dst: &Path) -> Result<()> {
    let mut b = std::fs::read(src).with_context(|| format!("no {}", src.display()))?;
    if b.len() < 124 || &b[..4] != b"AVB0" {
        bail!("{} doesn't start with an AVB header", src.display());
    }
    b[120..124].copy_from_slice(&3u32.to_be_bytes());
    if let Some(d) = dst.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(dst, b)?;
    Ok(())
}

/// Runs [plan] on an R1 that passed [check], once [typed] is [CONFIRM]. [progress] hears each step and fastboot's
/// lines.
pub fn run(t: &dyn Target, checked: &Checked, plan: &Plan, typed: &str, progress: &mut dyn FnMut(&str)) -> Result<()> {
    if typed.trim() != CONFIRM {
        bail!("not flashed: type {CONFIRM} to go ahead");
    }
    if plan.slot != checked.slot {
        bail!("the plan is for slot {}, but slot {} is active", plan.slot, checked.slot);
    }
    if !plan.system.is_file() {
        bail!("no system image at {}", plan.system.display());
    }
    for v in ["vbmeta", "vbmeta_system", "vbmeta_vendor"] {
        vbmeta_disabled(&plan.stock.path(&format!("{v}.img")), &plan.vbmeta(v))?;
    }
    for step in plan.steps() {
        match step {
            Step::Bootloader => {
                progress("Into the bootloader");
                if userspace(t)? {
                    t.reboot(Reboot::Bootloader)?;
                    t.wait().context("the R1 didn't come back in the bootloader")?;
                }
                if userspace(t)? {
                    bail!("the R1 didn't reach the bootloader");
                }
                unlocked(t)?;
            }
            Step::Fastbootd => {
                progress("Into fastbootd");
                t.reboot(Reboot::Fastbootd)?;
                t.wait().context("the R1 didn't come back in fastbootd (on Windows, check its driver)")?;
                if !userspace(t)? {
                    bail!("the R1 didn't reach fastbootd");
                }
            }
            Step::Write(w) => {
                progress(&describe_write(&w));
                t.write(&w, progress)?;
            }
            Step::MayFail(w) => {
                progress(&describe_write(&w));
                if let Err(e) = t.write(&w, progress) {
                    progress(&format!("(not needed: {e:#})"));
                }
            }
            Step::Boot => {
                progress("Booting r1os: the first boot takes a few minutes");
                t.reboot(Reboot::System)?;
            }
        }
    }
    Ok(())
}

fn describe_write(w: &Write) -> String {
    match w {
        Write::Flash { part, .. } => format!("Flashing {part}"),
        Write::Erase { part } => format!("Erasing {part}"),
        Write::DeleteLogical { part } => format!("Deleting {part}"),
        Write::CancelSnapshotUpdate => "Cancelling Rabbit's half-applied update".to_owned(),
        Write::Unlock => "Unlocking".to_owned(),
        Write::FlashSuper(_) => "Writing super".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// An R1 in fastboot that answers from a table and records what's done to it.
    struct Fake {
        vars: RefCell<std::collections::HashMap<String, String>>,
        done: RefCell<Vec<String>>,
        /// fastbootd after a reboot into it; the bootloader otherwise.
        reaches_fastbootd: bool,
    }

    impl Fake {
        fn new(unlocked: &str, slot: &str, vendor: &str) -> Self {
            let vars = [("unlocked", unlocked), ("current-slot", slot), ("is-userspace", "no"), ("partition-size:vendor_a", vendor), ("partition-size:vendor_b", vendor)];
            Fake { vars: RefCell::new(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()), done: RefCell::default(), reaches_fastbootd: true }
        }
    }

    impl Target for Fake {
        fn getvar(&self, name: &str) -> Result<Option<String>> {
            Ok(self.vars.borrow().get(name).cloned())
        }
        fn reboot(&self, to: Reboot) -> Result<()> {
            self.done.borrow_mut().push(format!("reboot {to:?}"));
            let fastbootd = to == Reboot::Fastbootd && self.reaches_fastbootd;
            self.vars.borrow_mut().insert("is-userspace".into(), if fastbootd { "yes" } else { "no" }.into());
            Ok(())
        }
        fn write(&self, w: &Write, _: &mut dyn FnMut(&str)) -> Result<()> {
            let args = crate::safety::args(w).map_err(anyhow::Error::new)?;
            self.done.borrow_mut().push(args.join(" "));
            Ok(())
        }
        fn wait(&self) -> Result<()> {
            Ok(())
        }
    }

    fn plan(dir: &Path, slot: char) -> Plan {
        std::fs::create_dir_all(dir).unwrap();
        for v in ["vbmeta", "vbmeta_system", "vbmeta_vendor"] {
            let mut b = vec![0u8; 4096];
            b[..4].copy_from_slice(b"AVB0");
            std::fs::write(dir.join(format!("{v}.img")), b).unwrap();
        }
        std::fs::write(dir.join("userdata.img"), b"empty").unwrap();
        std::fs::write(dir.join("system.img"), b"r1os").unwrap();
        Plan { slot, system: dir.join("system.img"), stock: Stock { dir: dir.to_path_buf() }, work: dir.join("work"), keep_product: false }
    }

    #[test]
    fn the_check_reads_and_refuses() {
        let ok = Fake::new("yes", "b", "0x40000000");
        assert_eq!(check(&ok).unwrap(), Checked { slot: 'b', vendor_bytes: 0x4000_0000 });
        // It only rebooted, into fastbootd: no writes.
        assert_eq!(*ok.done.borrow(), ["reboot Fastbootd"]);
        assert!(format!("{:#}", check(&Fake::new("no", "a", "0x1")).unwrap_err()).contains("locked"));
        assert!(format!("{:#}", check(&Fake::new("yes", "a", "0x0")).unwrap_err()).contains("vendor_a is empty"));
        assert!(check(&Fake::new("yes", "c", "0x1")).is_err());
        let mut stuck = Fake::new("yes", "a", "0x1");
        stuck.reaches_fastbootd = false;
        assert!(check(&stuck).is_err());
    }

    #[test]
    fn the_flash_is_flash_gsi_s_writes_in_its_order() {
        let d = std::env::temp_dir().join(format!("r1-usb-flash-{}", std::process::id()));
        let p = plan(&d, 'b');
        let t = Fake::new("yes", "b", "0x1");
        let checked = check(&t).unwrap();
        t.done.borrow_mut().clear();
        run(&t, &checked, &p, "ERASE", &mut |_| {}).unwrap();
        let w = |s: &str| s.replace("$W", &p.work.to_string_lossy()).replace("$D", &d.to_string_lossy());
        let want: Vec<String> = [
            "reboot Bootloader",
            "flash vbmeta_a $W\\vbmeta-noverify.img",
            "flash vbmeta_b $W\\vbmeta-noverify.img",
            "flash vbmeta_system_a $W\\vbmeta_system-noverify.img",
            "flash vbmeta_system_b $W\\vbmeta_system-noverify.img",
            "flash vbmeta_vendor_a $W\\vbmeta_vendor-noverify.img",
            "flash vbmeta_vendor_b $W\\vbmeta_vendor-noverify.img",
            "erase userdata",
            "flash userdata $D\\userdata.img",
            "reboot Fastbootd",
            "snapshot-update cancel",
            "delete-logical-partition product_b",
            "flash system_b $D\\system.img",
            "reboot System",
        ]
        .iter()
        .map(|s| w(s).replace('\\', std::path::MAIN_SEPARATOR_STR))
        .collect();
        assert_eq!(*t.done.borrow(), want);
        // Verification off in the copies.
        let v = std::fs::read(p.work.join("vbmeta-noverify.img")).unwrap();
        assert_eq!(&v[120..124], &[0, 0, 0, 3]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nothing_is_written_without_the_word_or_on_the_wrong_slot() {
        let d = std::env::temp_dir().join(format!("r1-usb-flash-word-{}", std::process::id()));
        let p = plan(&d, 'a');
        let t = Fake::new("yes", "a", "0x1");
        let checked = check(&t).unwrap();
        t.done.borrow_mut().clear();
        for typed in ["", "erase", "yes", "ERASE it"] {
            assert!(run(&t, &checked, &p, typed, &mut |_| {}).is_err(), "{typed}");
        }
        assert!(run(&t, &Checked { slot: 'b', vendor_bytes: 1 }, &p, "ERASE", &mut |_| {}).is_err());
        assert!(t.done.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_relock_mid_flash_stops_it_before_the_wipe() {
        let d = std::env::temp_dir().join(format!("r1-usb-flash-lock-{}", std::process::id()));
        let p = plan(&d, 'a');
        let t = Fake::new("yes", "a", "0x1");
        let checked = check(&t).unwrap();
        t.vars.borrow_mut().insert("unlocked".into(), "no".into());
        t.done.borrow_mut().clear();
        assert!(run(&t, &checked, &p, "ERASE", &mut |_| {}).is_err());
        assert!(!t.done.borrow().iter().any(|c| c.starts_with("flash") || c.starts_with("erase")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_write_passes_the_whitelist() {
        let p = plan(&std::env::temp_dir().join(format!("r1-usb-flash-wl-{}", std::process::id())), 'a');
        for s in p.steps() {
            if let Step::Write(w) | Step::MayFail(w) = s {
                assert!(crate::safety::check(&w).is_ok(), "{w:?}");
            }
        }
        assert!(vbmeta_disabled(Path::new("Cargo.toml"), &p.work.join("x")).is_err());
    }
}
