//! Installing r1os on an R1 over USB (ADR-0065, docs/specs/companion.md §11): the wizard's commands (T-171's
//! screens). Google's platform-tools fetched on first use (into `%LOCALAPPDATA%\r1os\companion\platform-tools`) and
//! the R1s plugged in, in whatever mode (T-167); the drivers (T-168); a stock R1's backup over the boot ROM (T-169),
//! which only reads; the flash, all from the bootloader (T-170, ADR-0067); and the first setup once r1os has started.
//!
//! Two things write to an R1. [flash_run], with the plan [flash_prepare] made for the R1 [flash_check] checked, and
//! `ERASE` typed: every write in it goes through `r1_usb::safety`. [setup_run], with the plan [setup_plan] made from
//! the R1's own status, after a click: it erases nothing, and every command in it comes from `r1_usb::setup`'s fixed
//! templates. The rest reads, or asks for a boot mode.

use r1_usb::fastboot::{Fastboot, Reboot};
use r1_usb::flash::{self, Checked, Plan, Step};
use r1_usb::safety::Write;
use r1_usb::scan::Mode;
use r1_usb::setup::{self, Inputs, OverAdb, Progress, Setup, Status};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use tauri::{AppHandle, Emitter, State};

/// Where the companion keeps what it fetched for installs.
fn dir() -> PathBuf {
    r1_usb::folders::companion()
}

/// Whether platform-tools are here, and the pinned version and Google's terms (shown before fetching).
#[tauri::command]
pub fn usb_tools() -> Value {
    let pin = r1_usb::tools::PLATFORM_TOOLS.as_ref();
    json!({
        "version": r1_usb::tools::find(&dir()).map(|t| t.version),
        "pinned": pin.map(|p| p.version),
        "size": pin.map(|p| p.size),
        "terms": r1_usb::tools::TERMS,
    })
}

/// Fetches platform-tools, checked against their pinned SHA-256; `usb-tools-progress {got, total}` as it goes.
#[tauri::command]
pub async fn usb_fetch_tools(app: AppHandle) -> Result<Value, String> {
    let tools = tauri::async_runtime::spawn_blocking(move || {
        let mut last = 0u64;
        r1_usb::tools::fetch(&dir(), |got, total| {
            // A step a percent at most, and the end.
            if got == total || got - last >= total / 100 {
                last = got;
                let _ = app.emit("usb-tools-progress", json!({ "got": got, "total": total }));
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))?;
    Ok(json!({ "version": tools.version }))
}

/// One command talking to the R1 at a time. Two fastboots at once can break a write, so the scan stands aside while
/// the flash (or its check, or a read) runs.
static USB: Mutex<()> = Mutex::new(());

/// Waits for the R1 to be free, and holds it.
fn usb() -> MutexGuard<'static, ()> {
    USB.lock().unwrap_or_else(|e| e.into_inner())
}

/// The R1s plugged in, each by its identity (a hash of its serial) and mode: `preloader`, `adb`, `unauthorized`,
/// `fastboot`, `fastbootd` or `no_driver`; in fastboot, unlocked and the active slot; booted, r1os's version. While
/// something else is talking to the R1, `{busy: true}` and nothing is asked. A flash's check holds only while its R1
/// stays in the bootloader: seen in any other mode, it's forgotten.
#[tauri::command]
pub async fn usb_scan(flash: State<'_, Flash>) -> Result<Value, String> {
    let found = tauri::async_runtime::spawn_blocking(|| {
        let _usb = match USB.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let tools = r1_usb::tools::find(&dir());
        Some((tools.is_some(), r1_usb::scan::scan(tools.as_ref())))
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some((tools, r1s)) = found else { return Ok(json!({ "busy": true })) };
    let left = flash.kept().checked.as_ref().is_some_and(|(id, _)| r1s.iter().any(|f| &f.id == id && f.mode != Mode::Fastboot));
    if left {
        flash.forget();
    }
    Ok(json!({ "tools": tools, "r1s": r1s }))
}

/// Where the drivers' package, scripts and logs go.
fn drivers_dir() -> PathBuf {
    dir().join("drivers")
}

/// What's installed: `{r1, mtk}` (T-168).
#[tauri::command]
pub async fn drivers_status() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| r1_usb::drivers::status().map(|s| json!(s)).map_err(|e| format!("{e:#}")))
        .await
        .map_err(|e| e.to_string())?
}

/// Installs the R1's driver (if it isn't already), and with `mtk` Rabbit's MediaTek preloader driver and with `usbdk`
/// UsbDk for the backup (each fetched and checked first), behind one administrator prompt. Returns the new status and
/// the step's log.
#[tauri::command]
pub async fn drivers_install(mtk: bool, usbdk: bool) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = drivers_dir();
        let exe = if mtk { Some(r1_usb::drivers::fetch_mtk(&dir)?) } else { None };
        let msi = if usbdk { Some(r1_usb::backup::fetch_usbdk(&dir)?) } else { None };
        let log = r1_usb::drivers::install(&dir, r1_usb::drivers::Extras { mtk_exe: exe.as_deref(), usbdk_msi: msi.as_deref() })?;
        Ok::<_, anyhow::Error>(json!({ "status": r1_usb::drivers::status()?, "log": log }))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Takes the R1's driver and its certificate back out, behind one administrator prompt.
#[tauri::command]
pub async fn drivers_undo() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let log = r1_usb::drivers::undo(&drivers_dir())?;
        Ok::<_, anyhow::Error>(json!({ "status": r1_usb::drivers::status()?, "log": log }))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Where uv and mtkclient go (T-169).
fn backup_tools_dir() -> PathBuf {
    r1_usb::backup::tools_dir()
}

/// The running backup's stop.
static BACKUP_STOP: std::sync::LazyLock<r1_usb::backup::Stop> = std::sync::LazyLock::new(Default::default);

/// The backup step: whether uv and mtkclient are fetched and Rabbit's MediaTek driver installed (mtkclient reaches the
/// preloader through its serial port), where backups go, what has to be typed to skip it, and for `identity`'s R1 the
/// newest backup that verifies.
#[tauri::command]
pub async fn backup_status(identity: Option<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = r1_usb::backup::default_root();
        let latest = identity.as_deref().and_then(|id| r1_usb::backup::latest(&root, id));
        json!({
            "tools": r1_usb::backup::find(&backup_tools_dir()).is_some(),
            "mtk_driver": r1_usb::drivers::status().map(|s| s.mtk).unwrap_or(false),
            "root": root,
            "latest": latest,
            "skip_phrase": r1_usb::backup::SKIP_PHRASE,
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Fetches uv and mtkclient, checked against their pins, and gets Python and mtkclient's packages ready;
/// `backup-tools-progress {what, got, total}` as it goes (`what` is `uv`, `mtkclient` or `python`).
#[tauri::command]
pub async fn backup_fetch_tools(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = (String::new(), 0u64);
        let tools = r1_usb::backup::fetch(&backup_tools_dir(), |what, got, total| {
            if what != last.0 || got == total || got - last.1 >= total / 100 {
                last = (what.to_owned(), got);
                let _ = app.emit("backup-tools-progress", json!({ "what": what, "got": got, "total": total }));
            }
        })?;
        let _ = app.emit("backup-tools-progress", json!({ "what": "python", "got": 0, "total": 0 }));
        r1_usb::backup::prepare(&tools)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Reads the R1's own partitions into `Documents\r1os backups`. A booted R1 on USB with root (r1os's image, or Magisk)
/// is read over adb at USB speed, with `backup-progress {part, done, total}` events. Otherwise (a stock R1) through the
/// preloader at power-on: the R1 powered off, then plugged in while this waits, with `backup-line` events carrying
/// mtkclient's lines, anything naming the device taken out. Returns the backup (its folder, identity, date and parts).
#[tauri::command]
pub async fn backup_run(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = r1_usb::backup::default_root();
        if let Some(tools) = r1_usb::tools::find(&dir())
            && let Some((serial, _)) =
                r1_usb::adb::devices(&tools.adb)?.into_iter().find(|(_, s)| *s == r1_usb::adb::State::Device)
        {
            let adb = r1_usb::adb::Adb::new(tools.adb, serial);
            let mut shown = std::time::Instant::now();
            let b = r1_usb::backup::run_adb(&adb, &root, &r1_usb::backup::stamp(), |part, done, total| {
                if shown.elapsed().as_millis() >= 200 || done == total {
                    shown = std::time::Instant::now();
                    let _ = app.emit("backup-progress", json!({ "part": part, "done": done, "total": total }));
                }
            })?;
            return Ok(json!(b));
        }
        let tools = r1_usb::backup::find(&backup_tools_dir()).ok_or_else(|| anyhow::anyhow!("the backup's tools aren't fetched yet"))?;
        if !r1_usb::drivers::status()?.mtk {
            anyhow::bail!("Rabbit's MediaTek driver isn't installed yet");
        }
        let b = r1_usb::backup::run(&tools, &root, &r1_usb::backup::stamp(), &BACKUP_STOP, |line| {
            let _ = app.emit("backup-line", line);
        })?;
        Ok(json!(b))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Stops a running backup (nothing it read is kept).
#[tauri::command]
pub fn backup_stop() {
    BACKUP_STOP.stop();
}

/// Whether `typed` is what skipping the backup takes.
#[tauri::command]
pub fn backup_skip(typed: String, identity: Option<String>) -> bool {
    let ok = r1_usb::backup::acknowledged(&typed);
    if ok {
        SKIPPED.lock().unwrap_or_else(|e| e.into_inner()).insert(identity.unwrap_or_default());
    }
    ok
}

/// The R1s whose backup was skipped by typing the phrase (`""`: before any R1 was seen), for [flash_run]. Only while
/// the companion runs.
static SKIPPED: Mutex<std::collections::BTreeSet<String>> = Mutex::new(std::collections::BTreeSet::new());

// ---- the flash (T-170, ADR-0067): everything from the bootloader, never fastbootd ----------------------------------

/// Where Rabbit's four files for the flash are kept.
fn stock_dir() -> PathBuf {
    dir().join("stock-v0.8.293")
}

/// Where an R1's `super` head and `para` are kept, between reading them (booted) and the check (in the bootloader).
fn r1_dir(id: &str) -> PathBuf {
    dir().join("r1").join(id)
}

/// Where the flash's `vbmeta` copies and `super` image are built.
fn work_dir() -> PathBuf {
    dir().join("flash-work")
}

/// An R1's identity as the scan gives it (12 hex digits), so it can name a folder.
fn identity_ok(id: &str) -> bool {
    id.len() == 12 && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn tools() -> anyhow::Result<r1_usb::tools::Tools> {
    r1_usb::tools::find(&dir()).ok_or_else(|| anyhow::anyhow!("the USB tools aren't fetched yet"))
}

/// Runs [f] off the window's thread, its error in words.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
}

/// The one R1 the scan finds in one of [modes], or [none] if there isn't one.
fn the_r1(tools: &r1_usb::tools::Tools, modes: &[Mode], none: &str) -> anyhow::Result<r1_usb::scan::Found> {
    let mut found: Vec<_> = r1_usb::scan::scan(Some(tools)).into_iter().filter(|f| modes.contains(&f.mode)).collect();
    match found.len() {
        0 => anyhow::bail!("{none}"),
        1 => Ok(found.remove(0)),
        _ => anyhow::bail!("more than one R1 is plugged in: leave just the one to install on"),
    }
}

/// The R1 in the bootloader's fastboot: its fastboot, its identity, and its serial (only to take out of fastboot's
/// lines; never shown or kept).
fn bootloader() -> anyhow::Result<(Fastboot, String, String)> {
    let tools = tools()?;
    let f = the_r1(&tools, &[Mode::Fastboot, Mode::Fastbootd], "no R1 in its bootloader")?;
    if f.mode == Mode::Fastbootd {
        anyhow::bail!("the R1 is in fastbootd, which r1os's install never uses: reboot it to the bootloader (`fastboot reboot bootloader`)");
    }
    let serial = r1_usb::fastboot::devices(&tools.fastboot)?
        .into_iter()
        .find(|s| r1_usb::identity(s) == f.id)
        .ok_or_else(|| anyhow::anyhow!("the R1 left its bootloader"))?;
    Ok((Fastboot::new(tools.fastboot, serial.clone()), f.id, serial))
}

/// The R1 booted, on USB with debugging allowed.
fn booted() -> anyhow::Result<r1_usb::adb::Adb> {
    let tools = tools()?;
    let f = the_r1(&tools, &[Mode::Adb, Mode::Unauthorized], "no R1 booted on USB with USB debugging allowed")?;
    if f.mode == Mode::Unauthorized {
        anyhow::bail!("the R1 is asking whether to allow USB debugging: allow it on its screen");
    }
    f.adb(&tools).ok_or_else(|| anyhow::anyhow!("the R1 isn't booted"))
}

/// What the flash keeps between its steps: the R1 checked (by its identity) and what the check found, then the plan
/// built for it. A new check drops the plan; a run, a boot mode asked for, or the R1 seen out of its bootloader drops
/// both.
#[derive(Default)]
pub struct Flash(Mutex<Kept>);

#[derive(Default)]
struct Kept {
    checked: Option<(String, Checked)>,
    plan: Option<Plan>,
}

impl Flash {
    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn forget(&self) {
        *self.kept() = Kept::default();
    }
}

/// Rabbit's files for the flash (its three `vbmeta` images and its empty `userdata`): whether they're here and checked,
/// and what fetching them takes (just their entries out of Rabbit's release, by range requests).
#[tauri::command]
pub async fn flash_stock_status() -> Result<Value, String> {
    blocking(|| {
        Ok(json!({
            "ready": r1_usb::stock::find(&stock_dir()).is_some(),
            "bytes": r1_usb::stock::FILES.iter().map(|f| f.size).sum::<u64>(),
            "files": r1_usb::stock::FILES.iter().map(|f| f.name).collect::<Vec<_>>(),
            "release": r1_usb::stock::ZIP_URL,
        }))
    })
    .await
}

/// Fetches Rabbit's files, each checked against its SHA-256, with `flash-stock-progress {file, index, count}` as each
/// is read; or, with [from], copies them out of Rabbit's release already extracted there.
#[tauri::command]
pub async fn flash_fetch_stock(app: AppHandle, from: Option<String>) -> Result<(), String> {
    blocking(move || {
        match from {
            Some(src) => r1_usb::stock::from_folder(Path::new(&src), &stock_dir())?,
            None => {
                let count = r1_usb::stock::FILES.len();
                let mut index = 0;
                r1_usb::stock::fetch(&stock_dir(), |file| {
                    let _ = app.emit("flash-stock-progress", json!({ "file": file, "index": index, "count": count }));
                    index += 1;
                })?
            }
        };
        Ok(())
    })
    .await
}

/// What's been read from [identity]'s R1 for the flash: whether `super`'s head and `para` are here, when they were
/// read, a Rabbit update's merge status, and each slot's vendor size from `super`'s metadata (0: empty).
#[tauri::command]
pub async fn flash_read_status(identity: String) -> Result<Value, String> {
    if !identity_ok(&identity) {
        return Err("not an R1's identity".into());
    }
    blocking(move || Ok(read_status(&identity))).await
}

fn read_status(id: &str) -> Value {
    let dir = r1_dir(id);
    let (Ok(head), Ok(para)) = (std::fs::read(dir.join("super-head.bin")), std::fs::read(dir.join("para.bin"))) else {
        return json!({ "read": false });
    };
    let at = std::fs::metadata(dir.join("super-head.bin"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    let vendor = |slot: char| {
        r1_usb::lp::read(&head, if slot == 'a' { 0 } else { 1 })
            .ok()
            .map(|m| m.partition(&format!("vendor_{slot}")).map_or(0, |p| p.sectors() * r1_usb::lp::SECTOR))
    };
    json!({
        "read": true,
        "at": at,
        "merge": r1_usb::superimg::merge_status(&para).ok(),
        "vendor_a": vendor('a'),
        "vendor_b": vendor('b'),
    })
}

/// Reads what the flash needs from a booted R1 with root (r1os's image, or Magisk) over adb: `super`'s first MiB (its
/// LP metadata) and `para` (a Rabbit update's merge status), kept in the companion's folder for this R1, and only if
/// they read as what they are. Reads only. Returns [flash_read_status]'s answer with the R1's identity.
#[tauri::command]
pub async fn flash_read_r1() -> Result<Value, String> {
    blocking(|| {
        let _usb = usb();
        let adb = booted()?;
        let root = adb.root()?;
        let id = adb.id();
        let dir = r1_dir(&id);
        std::fs::create_dir_all(&dir)?;
        let (head, para) = (dir.join("super-head.bin.part"), dir.join("para.bin.part"));
        let read = (|| {
            adb.read_partition_head(root, "super", flash::SUPER_HEAD, &head)?;
            adb.read_partition(root, "para", &para, |_| {})?;
            r1_usb::lp::read(&std::fs::read(&head)?, 0).map_err(|e| anyhow::anyhow!("what was read from super isn't its metadata: {e:#}"))?;
            r1_usb::superimg::merge_status(&std::fs::read(&para)?)?;
            std::fs::rename(&head, dir.join("super-head.bin"))?;
            std::fs::rename(&para, dir.join("para.bin"))?;
            Ok::<_, anyhow::Error>(())
        })();
        if read.is_err() {
            let _ = std::fs::remove_file(&head);
            let _ = std::fs::remove_file(&para);
        }
        read?;
        let mut status = read_status(&id);
        status["r1"] = json!(id);
        Ok(status)
    })
    .await
}

/// Restarts a booted R1 into its bootloader (`adb reboot bootloader`): a boot mode, nothing written.
#[tauri::command]
pub async fn flash_to_bootloader(flash: State<'_, Flash>) -> Result<(), String> {
    flash.forget();
    blocking(|| {
        let _usb = usb();
        booted()?.reboot_to_bootloader()
    })
    .await
}

/// Leaves the bootloader without flashing: the R1 starts its own system again.
#[tauri::command]
pub async fn flash_leave_bootloader(flash: State<'_, Flash>) -> Result<(), String> {
    flash.forget();
    blocking(|| {
        let _usb = usb();
        bootloader()?.0.reboot(Reboot::System)
    })
    .await
}

/// Stops [flash_enter_fastboot]'s wait.
static FASTBOOT_STOP: AtomicBool = AtomicBool::new(false);

/// Into the bootloader from power-off: waits up to ten minutes for a powered-off R1 to be plugged in, and asks its
/// preloader for fastboot (`READY`, then `FASTBOOT`): a boot mode, nothing written. On Windows the preloader's port
/// needs Rabbit's MediaTek driver.
#[tauri::command]
pub async fn flash_enter_fastboot(flash: State<'_, Flash>) -> Result<(), String> {
    flash.forget();
    FASTBOOT_STOP.store(false, Ordering::SeqCst);
    blocking(|| {
        if !r1_usb::drivers::status()?.mtk {
            anyhow::bail!("Rabbit's MediaTek driver isn't installed yet");
        }
        r1_usb::preloader::enter_fastboot(std::time::Duration::from_secs(600), &FASTBOOT_STOP)
    })
    .await
}

/// Stops waiting for the R1 at power-on.
#[tauri::command]
pub fn flash_enter_fastboot_stop() {
    FASTBOOT_STOP.store(true, Ordering::SeqCst);
}

/// The R1 in its bootloader: whether it's unlocked, and `fastboot flashing get_unlock_ability`'s number as the
/// bootloader gives it (what the R1's number means isn't settled, so it's shown, not judged). Reads only.
#[tauri::command]
pub async fn flash_unlock_ability() -> Result<Value, String> {
    blocking(|| {
        let _usb = usb();
        let (fb, id, _) = bootloader()?;
        Ok(json!({ "r1": id, "unlocked": fb.unlocked()?, "ability": fb.unlock_ability()? }))
    })
    .await
}

fn checked_json(id: &str, c: &Checked) -> Value {
    let other = if c.slot == 'a' { 'b' } else { 'a' };
    let other_vendor = c.metadata.partition(&format!("vendor_{other}")).map_or(0, |p| p.sectors() * r1_usb::lp::SECTOR);
    json!({ "r1": id, "slot": c.slot.to_string(), "vendor_bytes": c.vendor_bytes, "other_vendor_bytes": other_vendor, "merge": c.merge })
}

/// Checks the R1 in its bootloader against what was read from it (`r1_usb::flash::check`): the bootloader, not
/// fastbootd; unlocked; the battery; no Rabbit update pending; the active slot's vendor not empty. Writes nothing and
/// doesn't reboot. Kept for [flash_prepare]: `{r1, slot, vendor_bytes, other_vendor_bytes, merge}`.
#[tauri::command]
pub async fn flash_check(flash: State<'_, Flash>) -> Result<Value, String> {
    flash.forget();
    let (id, checked) = blocking(|| {
        let _usb = usb();
        let (fb, id, _) = bootloader()?;
        let dir = r1_dir(&id);
        let head = std::fs::read(dir.join("super-head.bin"))
            .map_err(|_| anyhow::anyhow!("nothing's been read from this R1 yet: start it, read it, then bring it back to the bootloader"))?;
        let para = std::fs::read(dir.join("para.bin"))?;
        let checked = flash::check(&fb, &head, &para)?;
        Ok((id, checked))
    })
    .await?;
    let out = checked_json(&id, &checked);
    flash.kept().checked = Some((id, checked));
    Ok(out)
}

/// A file's size, or 0.
fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// One write, for the screen: what (`flash`, `erase`), the partition and the bytes sent.
fn write_json(w: &Write) -> Value {
    match w {
        Write::Flash { part, image } => json!({ "what": "flash", "part": part, "bytes": size(image) }),
        Write::Erase { part } => json!({ "what": "erase", "part": part, "bytes": 0 }),
        Write::FlashSuper(image) => json!({ "what": "flash", "part": "super", "bytes": size(image.path()) }),
        Write::Unlock => json!({ "what": "unlock", "part": null, "bytes": 0 }),
    }
}

/// A plan for the screen: the R1 and slot, the system image, the plan in words, each write and the bytes they send,
/// and what has to be typed.
fn plan_json(id: &str, plan: &Plan) -> Value {
    let writes: Vec<Value> = plan.steps().iter().filter_map(|s| if let Step::Write(w) = s { Some(write_json(w)) } else { None }).collect();
    let bytes: u64 = writes.iter().filter_map(|w| w["bytes"].as_u64()).sum();
    json!({
        "r1": id,
        "slot": plan.slot.to_string(),
        "system": plan.system.file_name().map(|n| n.to_string_lossy().into_owned()),
        "system_bytes": size(&plan.system),
        "lines": plan.describe(),
        "writes": writes,
        "bytes": bytes,
        "confirm": flash::CONFIRM,
    })
}

/// Builds the flash for the R1 checked, on the PC (`r1_usb::flash::prepare`): the `vbmeta` copies with verification
/// off, and `super` with [system] (r1os's system image) on both slots and the R1's own vendor kept where it is,
/// verified as it's built. `flash-prepare-progress {done, total}` (the system's bytes written into the image). Nothing
/// goes to the R1. Kept for [flash_run]; returns the plan ([plan_json]).
#[tauri::command]
pub async fn flash_prepare(app: AppHandle, flash: State<'_, Flash>, system: String) -> Result<Value, String> {
    let (id, checked) = flash.kept().checked.clone().ok_or("check the R1 first")?;
    flash.kept().plan = None;
    let was = checked.clone();
    let plan = blocking(move || {
        let stock = r1_usb::stock::find(&stock_dir()).ok_or_else(|| anyhow::anyhow!("Rabbit's files aren't fetched yet"))?;
        let mut shown = std::time::Instant::now();
        flash::prepare(&checked, Path::new(&system), stock, &work_dir(), |done, total| {
            if shown.elapsed().as_millis() >= 200 || done == total {
                shown = std::time::Instant::now();
                let _ = app.emit("flash-prepare-progress", json!({ "done": done, "total": total }));
            }
        })
    })
    .await?;
    let mut kept = flash.kept();
    // Checked again, or forgotten, while it was built: this plan is for a check that's gone.
    if kept.checked.as_ref().is_none_or(|(i, c)| *i != id || *c != was) {
        return Err("the R1 changed while the flash was being built: check it and build it again".into());
    }
    let out = plan_json(&id, &plan);
    kept.plan = Some(plan);
    Ok(out)
}

/// What the flash holds now: the check and the plan, each `null` until made.
#[tauri::command]
pub fn flash_prepared(flash: State<'_, Flash>) -> Value {
    let kept = flash.kept();
    json!({
        "checked": kept.checked.as_ref().map(|(id, c)| checked_json(id, c)),
        "plan": kept.plan.as_ref().zip(kept.checked.as_ref()).map(|(p, (id, _))| plan_json(id, p)),
    })
}

/// The R1 in the bootloader for [flash_run]: its fastboot, saying each step as it starts (`flash-progress`).
struct Watched {
    fb: Fastboot,
    app: AppHandle,
    step: std::cell::Cell<usize>,
    steps: usize,
}

impl Watched {
    fn say(&self, mut step: Value) {
        self.step.set(self.step.get() + 1);
        step["step"] = json!(self.step.get());
        step["steps"] = json!(self.steps);
        let _ = self.app.emit("flash-progress", step);
    }
}

impl flash::Target for Watched {
    fn getvar(&self, name: &str) -> anyhow::Result<Option<String>> {
        self.fb.getvar(name)
    }

    fn reboot(&self, to: Reboot) -> anyhow::Result<()> {
        self.say(json!({ "what": "boot", "part": null, "bytes": 0 }));
        self.fb.reboot(to)
    }

    fn write(&self, w: &Write, progress: &mut dyn FnMut(&str)) -> anyhow::Result<()> {
        self.say(write_json(w));
        self.fb.write(w, progress)
    }
}

/// The flash (`r1_usb::flash::run`): the plan [flash_prepare] built, on the R1 [flash_check] checked, once [typed] is
/// `ERASE`. Refused with no plan, or if the R1 in the bootloader isn't the one checked. It writes vbmeta, then
/// `super`, then erases `userdata`, then starts r1os: `flash-progress {step, steps, what, part, bytes}` as each step
/// starts, and `flash-line` with each line said (fastboot's among them, the serial taken out). Once it has started,
/// a flash after it starts from a new check, however it ended.
#[tauri::command]
pub async fn flash_run(app: AppHandle, flash: State<'_, Flash>, typed: String) -> Result<(), String> {
    if typed.trim() != flash::CONFIRM {
        return Err(format!("not flashed: type {} to go ahead", flash::CONFIRM));
    }
    let (id, checked, plan) = {
        let kept = flash.kept();
        match (&kept.checked, &kept.plan) {
            (Some((id, c)), Some(p)) => (id.clone(), c.clone(), p.clone()),
            _ => return Err("nothing to flash yet: check the R1 and build the flash first".into()),
        }
    };
    let started = std::sync::Arc::new(AtomicBool::new(false));
    let ran = started.clone();
    let done = blocking(move || {
        let _usb = usb();
        let (fb, found, serial) = bootloader()?;
        if found != id {
            anyhow::bail!("the R1 in the bootloader isn't the one that was checked: check it first");
        }
        // Before the wipe, a backup of this R1 that verifies, or its skip typed (CLAUDE.md; ADR-0065): the screen asks
        // for one, and this holds it.
        let skipped = SKIPPED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|s| s.is_empty() || *s == id);
        if !skipped && r1_usb::backup::latest(&r1_usb::backup::default_root(), &id).is_none() {
            anyhow::bail!(
                "not flashed: no backup of this R1 here verifies. Back it up first, or type “{}” on the backup step",
                r1_usb::backup::SKIP_PHRASE
            );
        }
        let steps = plan.steps().iter().filter(|s| !matches!(s, Step::Bootloader)).count();
        let target = Watched { fb, app: app.clone(), step: std::cell::Cell::new(0), steps };
        ran.store(true, Ordering::SeqCst);
        let scrub = |s: &str| s.replace(&serial, "<this R1>");
        flash::run(&target, &checked, &plan, &typed, &mut |line| {
            let _ = app.emit("flash-line", scrub(line));
        })
        .map_err(|e| anyhow::anyhow!(scrub(&format!("{e:#}"))))
    })
    .await;
    if started.load(Ordering::SeqCst) {
        flash.forget();
    }
    done
}

// ---- the first setup (T-170, T-171): r1d, the shell, speech, the models and the agent box, over adb ----------------

/// Where the setup remembers the r1os checkout its files come from.
fn setup_file() -> PathBuf {
    dir().join("setup.json")
}

/// The r1os checkout picked last, if one was (there are no releases yet: T-166).
fn setup_from() -> Option<PathBuf> {
    let v: Value = serde_json::from_slice(&std::fs::read(setup_file()).ok()?).ok()?;
    v["checkout"].as_str().map(PathBuf::from)
}

/// Each input a checkout has for the setup, for the screen: which, where under [root], whether it's there, and
/// whether the setup goes on without it (`from_checkout` leaves an optional one out when it isn't there).
fn inputs_json(root: &Path, i: &Inputs) -> Vec<Value> {
    let one = |key: &str, p: Option<&Path>, optional: bool| {
        let path = p.map(|p| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/"));
        json!({ "key": key, "path": path, "here": p.is_some_and(Path::exists), "optional": optional })
    };
    let b = i.agent_box.as_ref();
    vec![
        one("r1d", Some(&i.r1d), false),
        one("shell", Some(&i.shell), false),
        one("speech_libs", Some(&i.speech_libs), false),
        one("speech_model", Some(&i.speech_model), false),
        one("system1", i.system1.as_deref(), true),
        one("translator", i.translator.as_deref(), true),
        one("alpine", b.map(|b| b.alpine.as_path()), false),
        one("codex", b.and_then(|b| b.codex.as_deref()), true),
        one("codex_host", b.and_then(|b| b.codex_host.as_deref()), true),
        one("busybox", b.and_then(|b| b.busybox.as_deref()), true),
    ]
}

/// Where the first setup's files come from: [from], an r1os checkout picked on this computer, remembered for next
/// time; with none, the one picked before. Reads only this computer, every file checked and hashed as the setup will
/// take it: `{from, found, error, ready}`, each input and whether it's there, and what `Setup::new` found wrong, in
/// words. `from` is `null` until a checkout is picked.
#[tauri::command]
pub async fn setup_inputs(from: Option<String>) -> Result<Value, String> {
    blocking(move || {
        let root = match from {
            Some(f) => {
                let root = PathBuf::from(f);
                if !root.join("crates").join("r1d").is_dir() {
                    anyhow::bail!("{} isn't an r1os checkout: it has no crates\\r1d", root.display());
                }
                std::fs::create_dir_all(dir())?;
                std::fs::write(setup_file(), serde_json::to_vec_pretty(&json!({ "checkout": root }))?)?;
                root
            }
            None => match setup_from() {
                Some(root) => root,
                None => return Ok(json!({ "from": null, "found": [], "error": null, "ready": false })),
            },
        };
        let inputs = Inputs::from_checkout(&root);
        let found = inputs_json(&root, &inputs);
        let error = Setup::new(inputs).err().map(|e| format!("{e:#}"));
        Ok(json!({ "from": root, "found": found, "ready": error.is_none(), "error": error }))
    })
    .await
}

/// What's on the booted R1 for the setup (`r1_usb::setup::status`): r1os's image, `/data`, r1d, the agent box, and
/// whether it's online. Reads only. `{r1, status}`, the R1 by its identity.
#[tauri::command]
pub async fn setup_status() -> Result<Value, String> {
    blocking(|| {
        let _usb = usb();
        let adb = booted()?;
        let status = setup::status(&OverAdb::new(&adb)?)?;
        Ok(json!({ "r1": adb.id(), "status": status }))
    })
    .await
}

/// A plan as [setup_plan] made it: for the R1 it read (by its identity), or for an assumed just-flashed one (and why
/// none was read), with the status it was made from, and each step's bytes for the screen's estimate.
#[derive(Clone)]
struct Planned {
    r1: Option<String>,
    why: Option<String>,
    plan: setup::Plan,
    status: Status,
    bytes: Vec<u64>,
    box_later: bool,
}

/// What the first setup keeps between its steps: the plan, until it's run or a new one is made.
#[derive(Default)]
pub struct FirstSetup(Mutex<Option<Planned>>);

impl FirstSetup {
    fn kept(&self) -> MutexGuard<'_, Option<Planned>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Every file's bytes under [dir].
fn folder_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_bytes(&e.path()),
            _ => e.metadata().map_or(0, |m| m.len()),
        })
        .sum()
}

/// What a step pushes over USB: with what it does on the R1, the screen's estimate of how long it takes.
fn step_bytes(step: &setup::Step, i: &Inputs) -> u64 {
    let b = i.agent_box.as_ref();
    match step {
        setup::Step::Speech => folder_bytes(&i.speech_libs) + folder_bytes(&i.speech_model),
        setup::Step::System1 => i.system1.as_deref().map_or(0, folder_bytes),
        setup::Step::Translator => i.translator.as_deref().map_or(0, folder_bytes),
        setup::Step::R1d => size(&i.r1d),
        setup::Step::Shell => folder_bytes(&i.shell),
        setup::Step::AgentExec => setup::AGENT_EXEC.len() as u64,
        setup::Step::Busybox => b.and_then(|b| b.busybox.as_deref()).map_or(0, size),
        setup::Step::AgentBox => b.map_or(0, |b| size(&b.alpine) + b.codex.as_deref().map_or(0, size) + b.codex_host.as_deref().map_or(0, size)),
        setup::Step::GrowData { .. } | setup::Step::RestartR1d => 0,
    }
}

/// A step's name for the screen.
fn step_what(step: &setup::Step) -> &'static str {
    match step {
        setup::Step::GrowData { .. } => "grow",
        setup::Step::Speech => "speech",
        setup::Step::System1 => "system1",
        setup::Step::Translator => "translator",
        setup::Step::R1d => "r1d",
        setup::Step::Shell => "shell",
        setup::Step::RestartR1d => "restart",
        setup::Step::AgentExec => "agent_exec",
        setup::Step::Busybox => "busybox",
        setup::Step::AgentBox => "agent_box",
    }
}

/// A plan for the screen: the R1 (`null` when assumed, with `why`), each step (`what`, its `words`, its `bytes`), what's
/// there already, whether the agent box waits for the R1 to be online, and the status it was made from.
fn planned_json(p: &Planned) -> Value {
    let steps: Vec<Value> = p
        .plan
        .steps()
        .iter()
        .zip(p.plan.describe())
        .zip(&p.bytes)
        .map(|((s, words), bytes)| json!({ "what": step_what(s), "words": words, "bytes": bytes }))
        .collect();
    json!({
        "r1": p.r1,
        "assumed": p.r1.is_none(),
        "why": p.why,
        "steps": steps,
        "already": p.plan.already(),
        "box_later": p.box_later,
        "status": p.status,
    })
}

/// The plan for an R1 in [status], and the inputs it takes. An offline R1 whose agent box is still to be built gets its
/// plan without the box, which downloads on the R1 (Q76), and `true` for that; one with its box built plans as it is.
fn plan_for(checked: &Setup, status: &Status) -> anyhow::Result<(setup::Plan, Inputs, bool)> {
    match checked.plan(status) {
        Err(_) if status.online == Some(false) && checked.inputs().agent_box.is_some() => {
            let no_box = Inputs { agent_box: None, ..checked.inputs().clone() };
            Ok((Setup::new(no_box.clone())?.plan(status)?, no_box, true))
        }
        plan => Ok((plan?, checked.inputs().clone(), false)),
    }
}

/// The first setup's plan, from the checkout picked ([setup_inputs]), for the booted R1's own status; with none
/// booted, for a just-flashed R1 as it's assumed to be (`Status::fresh`), to show what it will do: that one can't be
/// run. An R1 that isn't online gets a plan without the agent box, whose build downloads on the R1 (Q76): it comes
/// once the R1 is on Wi-Fi, from a new plan. Reads only. Kept for [setup_run]; returns it ([planned_json]).
#[tauri::command]
pub async fn setup_plan(kept: State<'_, FirstSetup>) -> Result<Value, String> {
    *kept.kept() = None;
    let planned = blocking(|| {
        let root = setup_from().ok_or_else(|| anyhow::anyhow!("choose your r1os checkout first"))?;
        let checked = Setup::new(Inputs::from_checkout(&root))?;
        let (r1, why, status) = {
            let _usb = usb();
            match booted() {
                Ok(adb) => (Some(adb.id()), None, setup::status(&OverAdb::new(&adb)?)?),
                Err(e) => (None, Some(format!("{e:#}")), Status::fresh()),
            }
        };
        let (plan, inputs, box_later) = plan_for(&checked, &status)?;
        let bytes = plan.steps().iter().map(|s| step_bytes(s, &inputs)).collect();
        Ok(Planned { r1, why, plan, status, bytes, box_later })
    })
    .await?;
    let out = planned_json(&planned);
    *kept.kept() = Some(planned);
    Ok(out)
}

/// The plan the setup holds now, or `null`.
#[tauri::command]
pub fn setup_planned(kept: State<'_, FirstSetup>) -> Value {
    kept.kept().as_ref().map_or(Value::Null, planned_json)
}

/// The first setup (`r1_usb::setup::run`): the plan [setup_plan] made, on the R1 it was made for. It writes to the R1
/// (`/data` grown online, `/data/r1os`, r1d restarted, the agent box built) but erases nothing, so a click after the
/// plan is shown starts it, with no word to type. Refused with no plan, an assumed R1's plan, or another R1 plugged
/// in; the run reads the R1 again first and stops if the plan would differ. `setup-progress {index, of, words}` as
/// each step starts, and `setup-line` with each line the R1 prints. Returns `{r1, status}`, the R1 as it is after.
/// Once it has started, a setup after it starts from a new plan, however it ended.
#[tauri::command]
pub async fn setup_run(app: AppHandle, kept: State<'_, FirstSetup>) -> Result<Value, String> {
    let p = kept.kept().clone().ok_or("nothing to set up yet: read the R1 for its plan first")?;
    let Some(id) = p.r1.clone() else {
        return Err("this plan is for an assumed R1: plug the R1 in, booted, and read it for its own plan".into());
    };
    let started = std::sync::Arc::new(AtomicBool::new(false));
    let ran = started.clone();
    let done = blocking(move || {
        let _usb = usb();
        let adb = booted()?;
        if adb.id() != id {
            anyhow::bail!("the R1 plugged in isn't the one the plan was made for: read it for its own plan");
        }
        let r1 = OverAdb::new(&adb)?;
        ran.store(true, Ordering::SeqCst);
        let after = setup::run(&r1, &p.plan, &mut |progress| match progress {
            Progress::Step { index, of, words } => {
                let _ = app.emit("setup-progress", json!({ "index": index, "of": of, "words": words }));
            }
            Progress::Line(l) => {
                let _ = app.emit("setup-line", l);
            }
        })?;
        Ok(json!({ "r1": id, "status": after }))
    })
    .await;
    if started.load(Ordering::SeqCst) {
        *kept.kept() = None;
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checkout with the least the setup takes, and an agent box's Alpine and BusyBox.
    fn checkout(dir: &Path) -> Inputs {
        let put = |rel: &str, b: &[u8]| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b).unwrap();
        };
        // An arm64 program's ELF header.
        let mut elf = vec![0u8; 64];
        elf[..4].copy_from_slice(b"\x7fELF");
        (elf[4], elf[5], elf[18]) = (2, 1, 183);
        std::fs::create_dir_all(dir.join("crates").join("r1d")).unwrap();
        put("target/aarch64-linux-android/release/r1d", &elf);
        put("shell/dist/index.html", b"<!doctype html>");
        put("out/speech/lib/arm64-v8a/libsherpa-onnx-c-api.so", b"sherpa");
        put("out/speech/lib/arm64-v8a/libonnxruntime.so", b"ort");
        for f in ["encoder.ort", "decoder.ort", "joiner.ort", "tokens.txt"] {
            put(&format!("out/speech/stt-r1/{f}"), f.as_bytes());
        }
        put("firmware/agent-box/alpine-minirootfs-3.24.2-aarch64.tar.gz", &[0x1f, 0x8b, 8, 0]);
        put("firmware/agent-box/busybox-arm64", &elf);
        Inputs::from_checkout(dir)
    }

    #[test]
    fn an_offline_r1_is_set_up_without_its_box() {
        let dir = std::env::temp_dir().join(format!("r1os-companion-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let checked = Setup::new(checkout(&dir)).unwrap();

        // Online (as a just-flashed R1 is assumed to be): every step, the box last.
        let (plan, inputs, later) = plan_for(&checked, &Status::fresh()).unwrap();
        let what: Vec<_> = plan.steps().iter().map(step_what).collect();
        assert_eq!(what, ["grow", "speech", "r1d", "shell", "restart", "agent_exec", "busybox", "agent_box"]);
        assert!(!later && inputs.agent_box.is_some());
        let bytes: Vec<u64> = plan.steps().iter().map(|s| step_bytes(s, &inputs)).collect();
        assert_eq!(bytes, [0, 9 + 42, 64, 15, 0, setup::AGENT_EXEC.len() as u64, 64, 4]);

        // Offline: the rest now, the box once it's on Wi-Fi.
        let mut offline = Status::fresh();
        offline.online = Some(false);
        assert!(checked.plan(&offline).is_err(), "r1_usb::setup refuses the box offline");
        let (plan, inputs, later) = plan_for(&checked, &offline).unwrap();
        let what: Vec<_> = plan.steps().iter().map(step_what).collect();
        assert_eq!(what, ["grow", "speech", "r1d", "shell", "restart"]);
        assert!(later && inputs.agent_box.is_none());

        // Not known isn't offline.
        offline.online = None;
        assert!(!plan_for(&checked, &offline).unwrap().2);

        // The screen's list: each input under the checkout, with `/`, and what's left out.
        let found = inputs_json(&dir, checked.inputs());
        let r1d = found.iter().find(|f| f["key"] == "r1d").unwrap();
        assert_eq!((r1d["path"].as_str(), r1d["here"].as_bool()), (Some("target/aarch64-linux-android/release/r1d"), Some(true)));
        let codex = found.iter().find(|f| f["key"] == "codex").unwrap();
        assert!(codex["path"].is_null() && codex["here"] == false && codex["optional"] == true);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

