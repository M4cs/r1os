//! Flashing r1os onto an R1 (ADR-0065 decision 6, ADR-0067): everything from the bootloader's fastboot, never fastbootd
//! (a trip through it, with no writes, broke an R1: T-182). Each write goes through [`crate::safety`].
//!
//! - **Checked first** ([check], which writes nothing and doesn't reboot): the R1 is in the bootloader, not fastbootd;
//!   it's `unlocked`; the active slot; the battery's fit; no Rabbit update is pending (the merge status in `para`); and
//!   the active slot's vendor isn't empty, read from `super`'s own metadata. `super`'s head and `para` are read from the
//!   R1 beforehand (`adb` on an R1 with root, or the backup).
//! - **Prepared** ([prepare]): the `vbmeta` copies with verification off, and the `super` image ([`crate::superimg`]):
//!   r1os's system on both slots, the R1's own vendor kept where it is, built and verified on the PC.
//! - **The plan** ([`Plan::steps`]) is shown, and runs only when [`CONFIRM`] is typed.
//! - **The writes**, in this order: verification off in `vbmeta`, `vbmeta_system` and `vbmeta_vendor` on both slots;
//!   `super`; then `userdata` erased and Rabbit's empty image written. `super` comes before the wipe: if the bootloader
//!   refused the image, nothing of yours would be gone yet. Nothing else: no `boot_*`, no relock, no fastbootd.

use crate::fastboot::{Fastboot, Reboot};
use crate::lp::{Metadata, SECTOR};
use crate::safety::Write;
use crate::stock::Stock;
use crate::superimg::{MergeStatus, SuperImage};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// What has to be typed for the flash to go ahead: it erases everything on the R1.
pub const CONFIRM: &str = "ERASE";
/// How much of `super` to read from the R1 for its metadata: its geometry and every slot fit in the first MiB.
pub const SUPER_HEAD: u64 = 1 << 20;

/// An R1 in the bootloader's fastboot, as the flash sees it: [`Fastboot`], or a stand-in in tests.
pub trait Target {
    fn getvar(&self, name: &str) -> Result<Option<String>>;
    fn reboot(&self, to: Reboot) -> Result<()>;
    /// One write, through [`crate::safety`].
    fn write(&self, w: &Write, progress: &mut dyn FnMut(&str)) -> Result<()>;
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
}

/// What [check] found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Checked {
    /// The active slot, which the flash writes first.
    pub slot: char,
    /// `vendor_<slot>`'s size, from `super`'s metadata.
    pub vendor_bytes: u64,
    pub merge: MergeStatus,
    /// `super`'s metadata for the active slot, as read from the R1.
    #[serde(skip)]
    pub metadata: Metadata,
}

fn in_bootloader(t: &dyn Target) -> Result<()> {
    if t.getvar("is-userspace")?.as_deref() == Some("yes") {
        bail!("the R1 is in fastbootd: reboot it to the bootloader (`fastboot reboot bootloader`). r1os's install never uses fastbootd");
    }
    Ok(())
}

fn unlocked(t: &dyn Target) -> Result<()> {
    match t.getvar("unlocked")?.as_deref() {
        Some("yes") => Ok(()),
        Some(_) => bail!("the bootloader is locked: unlock it first (Rabbit's portal, then `fastboot flashing unlock`)"),
        None => bail!("the bootloader didn't say whether it's unlocked"),
    }
}

/// Reads what has to hold before any write, from the R1 in the bootloader and from [super_head] (`super`'s first
/// [SUPER_HEAD] bytes) and [para], both read from the R1 beforehand. Writes nothing and doesn't reboot.
pub fn check(t: &dyn Target, super_head: &[u8], para: &[u8]) -> Result<Checked> {
    in_bootloader(t)?;
    unlocked(t)?;
    let slot = match t.getvar("current-slot")?.as_deref() {
        Some("a") | Some("_a") => 'a',
        Some("b") | Some("_b") => 'b',
        other => bail!("couldn't read the active slot (got {other:?})"),
    };
    if t.getvar("battery-soc-ok")?.as_deref() == Some("no") {
        bail!("the battery is too low to flash: charge the R1 first");
    }
    let merge = crate::superimg::merge_status(para)?;
    if merge != MergeStatus::None {
        bail!("a Rabbit update is in progress ({merge:?}): boot Rabbit's OS until it finishes, then try again");
    }
    let metadata = crate::lp::read(super_head, if slot == 'a' { 0 } else { 1 })?;
    let vendor_bytes = metadata.partition(&format!("vendor_{slot}")).map(|p| p.sectors() * SECTOR).unwrap_or(0);
    if vendor_bytes == 0 {
        bail!("vendor_{slot} is empty: slot {slot} wouldn't boot after the flash. Rabbit's restore puts it back; don't flash this slot");
    }
    Ok(Checked { slot, vendor_bytes, merge, metadata })
}

/// One step of the flash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The R1 still in the bootloader (not fastbootd), and `unlocked` read again right before the first write.
    Bootloader,
    Write(Write),
    /// Into r1os.
    Boot,
}

/// The flash for an R1 that passed [check], prepared on the PC by [prepare].
#[derive(Debug, Clone)]
pub struct Plan {
    pub slot: char,
    pub system: PathBuf,
    pub stock: Stock,
    /// The `vbmeta` copies with verification off ([vbmeta_disabled]) and the `super` image go here.
    pub work: PathBuf,
    pub image: SuperImage,
}

/// Builds what the flash writes, on the PC: the `vbmeta` copies, and the `super` image for [checked] with [system]
/// (verified as it's built). [progress] hears the system bytes written into the image, and the total.
pub fn prepare(checked: &Checked, system: &Path, stock: Stock, work: &Path, progress: impl FnMut(u64, u64)) -> Result<Plan> {
    std::fs::create_dir_all(work)?;
    for v in ["vbmeta", "vbmeta_system", "vbmeta_vendor"] {
        vbmeta_disabled(&stock.path(&format!("{v}.img")), &work.join(format!("{v}-noverify.img")))?;
    }
    let image = crate::superimg::build(&checked.metadata, checked.slot, system, &work.join("super.img"), progress)?;
    Ok(Plan { slot: checked.slot, system: system.to_path_buf(), stock, work: work.to_path_buf(), image })
}

impl Plan {
    fn vbmeta(&self, name: &str) -> PathBuf {
        self.work.join(format!("{name}-noverify.img"))
    }

    /// Every step, in order. [run] does exactly these.
    pub fn steps(&self) -> Vec<Step> {
        let mut steps = vec![Step::Bootloader];
        for v in ["vbmeta", "vbmeta_system", "vbmeta_vendor"] {
            for slot in ['a', 'b'] {
                steps.push(Step::Write(Write::Flash { part: format!("{v}_{slot}"), image: self.vbmeta(v) }));
            }
        }
        steps.push(Step::Write(Write::FlashSuper(self.image.clone())));
        steps.push(Step::Write(Write::Erase { part: "userdata".into() }));
        steps.push(Step::Write(Write::Flash { part: "userdata".into(), image: self.stock.path("userdata.img") }));
        steps.push(Step::Boot);
        steps
    }

    /// The plan in words, for the screen before [CONFIRM] is typed.
    pub fn describe(&self) -> Vec<String> {
        let name = self.system.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let slots: Vec<String> = self.image.metadata.partitions.iter().filter_map(|p| p.name.strip_prefix("system_").filter(|s| !s.contains('-')).map(str::to_owned)).collect();
        let mut lines = vec![
            "Verification off in vbmeta, vbmeta_system and vbmeta_vendor, on slots a and b".to_owned(),
            format!(
                "super: {name} as the system on slot{} {}, the R1's own vendor kept where it is, Rabbit's product removed",
                if slots.len() > 1 { "s" } else { "" },
                slots.join(" and ")
            ),
            "ERASE userdata (everything on the R1) and write Rabbit's empty userdata".to_owned(),
        ];
        if slots.len() < 2 {
            lines.push("The other slot has no vendor, so it gets no system: this R1 has no fallback slot".to_owned());
        }
        lines.push(format!(
            "Untouched: the kernel (boot), vendor, dtbo, the bootloader, and everything that's this R1's own. Boots slot {}.",
            self.slot
        ));
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
    if plan.image.metadata.partition(&format!("system_{}", checked.slot)).is_none() {
        bail!("the super image has no system for slot {}", checked.slot);
    }
    crate::superimg::verify(&checked.metadata, &plan.image).context("the super image changed since it was built")?;
    for step in plan.steps() {
        match step {
            Step::Bootloader => {
                progress("Checking the bootloader");
                in_bootloader(t)?;
                unlocked(t)?;
            }
            Step::Write(w) => {
                progress(&describe_write(&w));
                t.write(&w, progress)?;
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
        Write::Unlock => "Unlocking".to_owned(),
        Write::FlashSuper(_) => "Writing super: r1os's system, the R1's vendor kept".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::superimg::VIRTUAL_AB_OFFSET;
    use std::cell::RefCell;

    /// An R1 in the bootloader that answers from a table and records what's done to it.
    struct Fake {
        vars: RefCell<std::collections::HashMap<String, String>>,
        done: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(unlocked: &str, slot: &str) -> Self {
            let vars = [("unlocked", unlocked), ("current-slot", slot), ("is-userspace", "no"), ("battery-soc-ok", "yes")];
            Fake { vars: RefCell::new(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()), done: RefCell::default() }
        }
        fn set(&self, k: &str, v: &str) {
            self.vars.borrow_mut().insert(k.into(), v.into());
        }
    }

    impl Target for Fake {
        fn getvar(&self, name: &str) -> Result<Option<String>> {
            Ok(self.vars.borrow().get(name).cloned())
        }
        fn reboot(&self, to: Reboot) -> Result<()> {
            self.done.borrow_mut().push(format!("reboot {to:?}"));
            Ok(())
        }
        fn write(&self, w: &Write, _: &mut dyn FnMut(&str)) -> Result<()> {
            let args = crate::safety::args(w).map_err(anyhow::Error::new)?;
            self.done.borrow_mut().push(args.join(" "));
            Ok(())
        }
    }

    fn para(merge: u8) -> Vec<u8> {
        let mut p = vec![0u8; 64 * 1024];
        p[VIRTUAL_AB_OFFSET..VIRTUAL_AB_OFFSET + 7].copy_from_slice(&[0x02, 0xb0, 0x0a, 0x74, 0x56, merge, 0x01]);
        p
    }

    /// A small R1 (16 MiB super) and what the flash takes: its head, Rabbit's files, a system image.
    fn setup(dir: &Path, vendor_b: u64) -> (Vec<u8>, Stock, PathBuf) {
        std::fs::create_dir_all(dir).unwrap();
        for v in ["vbmeta", "vbmeta_system", "vbmeta_vendor"] {
            let mut b = vec![0u8; 4096];
            b[..4].copy_from_slice(b"AVB0");
            std::fs::write(dir.join(format!("{v}.img")), b).unwrap();
        }
        std::fs::write(dir.join("userdata.img"), b"empty").unwrap();
        std::fs::write(dir.join("system.img"), vec![7u8; 1 << 20]).unwrap();
        let head = crate::lp::tests::head_of(&crate::superimg::tests::small(vendor_b));
        (head, Stock { dir: dir.to_path_buf() }, dir.join("system.img"))
    }

    #[test]
    fn the_check_reads_and_refuses_without_rebooting() {
        let dir = std::env::temp_dir().join(format!("r1-usb-flash-check-{}", std::process::id()));
        let (head, _, _) = setup(&dir, 2048);
        let ok = Fake::new("yes", "a");
        let c = check(&ok, &head, &para(0)).unwrap();
        assert_eq!((c.slot, c.vendor_bytes, c.merge), ('a', 2048 * SECTOR, MergeStatus::None));
        assert!(ok.done.borrow().is_empty(), "no reboots, no writes");
        assert!(format!("{:#}", check(&Fake::new("no", "a"), &head, &para(0)).unwrap_err()).contains("locked"));
        assert!(format!("{:#}", check(&Fake::new("yes", "a"), &head, &para(3)).unwrap_err()).contains("update is in progress"));
        let f = Fake::new("yes", "a");
        f.set("is-userspace", "yes");
        assert!(format!("{:#}", check(&f, &head, &para(0)).unwrap_err()).contains("fastbootd"));
        let f = Fake::new("yes", "a");
        f.set("battery-soc-ok", "no");
        assert!(check(&f, &head, &para(0)).is_err());
        // Slot b of this R1 has an empty vendor.
        let (head0, _, _) = setup(&dir, 0);
        assert!(format!("{:#}", check(&Fake::new("yes", "b"), &head0, &para(0)).unwrap_err()).contains("vendor_b is empty"));
        assert!(check(&Fake::new("yes", "c"), &head, &para(0)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flash_is_vbmeta_then_super_then_the_wipe() {
        let dir = std::env::temp_dir().join(format!("r1-usb-flash-run-{}", std::process::id()));
        let (head, stock, system) = setup(&dir, 2048);
        let t = Fake::new("yes", "a");
        let checked = check(&t, &head, &para(0)).unwrap();
        let plan = prepare(&checked, &system, stock, &dir.join("work"), |_, _| {}).unwrap();
        run(&t, &checked, &plan, "ERASE", &mut |_| {}).unwrap();
        let w = |s: &str| s.replace("$W", &plan.work.to_string_lossy()).replace("$D", &dir.to_string_lossy()).replace('\\', std::path::MAIN_SEPARATOR_STR);
        let want: Vec<String> = [
            "flash vbmeta_a $W\\vbmeta-noverify.img",
            "flash vbmeta_b $W\\vbmeta-noverify.img",
            "flash vbmeta_system_a $W\\vbmeta_system-noverify.img",
            "flash vbmeta_system_b $W\\vbmeta_system-noverify.img",
            "flash vbmeta_vendor_a $W\\vbmeta_vendor-noverify.img",
            "flash vbmeta_vendor_b $W\\vbmeta_vendor-noverify.img",
            "flash super $W\\super.img",
            "erase userdata",
            "flash userdata $D\\userdata.img",
            "reboot System",
        ]
        .iter()
        .map(|s| w(s))
        .collect();
        assert_eq!(*t.done.borrow(), want);
        assert!(!t.done.borrow().iter().any(|c| c.contains("fastboot") || c.contains("delete-logical") || c.contains("snapshot")));
        assert_eq!(&std::fs::read(plan.work.join("vbmeta-noverify.img")).unwrap()[120..124], &[0, 0, 0, 3]);
        assert!(plan.describe().iter().any(|l| l.contains("slots a and b")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_is_written_without_the_word_on_the_wrong_slot_or_after_a_relock() {
        let dir = std::env::temp_dir().join(format!("r1-usb-flash-word-{}", std::process::id()));
        let (head, stock, system) = setup(&dir, 2048);
        let t = Fake::new("yes", "a");
        let checked = check(&t, &head, &para(0)).unwrap();
        let plan = prepare(&checked, &system, stock, &dir.join("work"), |_, _| {}).unwrap();
        for typed in ["", "erase", "yes", "ERASE it"] {
            assert!(run(&t, &checked, &plan, typed, &mut |_| {}).is_err(), "{typed}");
        }
        let other = Checked { slot: 'b', ..checked.clone() };
        assert!(run(&t, &other, &plan, "ERASE", &mut |_| {}).is_err());
        // Relocked, or put into fastbootd, between the check and the run: stopped before the first write.
        t.set("unlocked", "no");
        assert!(run(&t, &checked, &plan, "ERASE", &mut |_| {}).is_err());
        t.set("unlocked", "yes");
        t.set("is-userspace", "yes");
        assert!(run(&t, &checked, &plan, "ERASE", &mut |_| {}).is_err());
        assert!(t.done.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_fallback_slot_is_said() {
        let dir = std::env::temp_dir().join(format!("r1-usb-flash-one-{}", std::process::id()));
        let (head, stock, system) = setup(&dir, 0);
        let t = Fake::new("yes", "a");
        let checked = check(&t, &head, &para(0)).unwrap();
        let plan = prepare(&checked, &system, stock, &dir.join("work"), |_, _| {}).unwrap();
        assert!(plan.describe().iter().any(|l| l.contains("no fallback slot")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_write_passes_the_whitelist() {
        let dir = std::env::temp_dir().join(format!("r1-usb-flash-wl-{}", std::process::id()));
        let (head, stock, system) = setup(&dir, 2048);
        let checked = check(&Fake::new("yes", "a"), &head, &para(0)).unwrap();
        let plan = prepare(&checked, &system, stock, &dir.join("work"), |_, _| {}).unwrap();
        for s in plan.steps() {
            if let Step::Write(w) = s {
                assert!(crate::safety::check(&w).is_ok(), "{w:?}");
            }
        }
        assert!(vbmeta_disabled(Path::new("Cargo.toml"), &dir.join("x")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
