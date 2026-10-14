//! Installing r1os on an R1 over USB (ADR-0065, docs/specs/companion.md §11): the wizard's commands. So far:
//! Google's platform-tools fetched on first use (into `%LOCALAPPDATA%\r1os\companion\platform-tools`) and the R1s
//! plugged in, in whatever mode (T-167); the drivers (T-168); and a stock R1's backup over the boot ROM (T-169), which
//! only reads. Nothing here writes to an R1 yet: the flash comes with T-170, through `r1_usb::safety`.

use serde_json::{Value, json};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

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

/// The R1s plugged in, each by its identity (a hash of its serial) and mode: `preloader`, `adb`, `unauthorized`,
/// `fastboot`, `fastbootd` or `no_driver`; in fastboot, unlocked and the active slot; booted, r1os's version.
#[tauri::command]
pub async fn usb_scan() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let tools = r1_usb::tools::find(&dir());
        json!({ "tools": tools.is_some(), "r1s": r1_usb::scan::scan(tools.as_ref()) })
    })
    .await
    .map_err(|e| e.to_string())
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
pub fn backup_skip(typed: String) -> bool {
    r1_usb::backup::acknowledged(&typed)
}

