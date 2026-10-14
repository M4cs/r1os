//! Installing r1os on an R1 over USB (ADR-0065, docs/specs/companion.md §11): the wizard's commands. For now what
//! T-167 gives it: Google's platform-tools fetched on first use (into `%LOCALAPPDATA%\r1os\companion\platform-tools`),
//! and the R1s plugged in, in whatever mode. Nothing here writes to an R1 yet: the flash comes with T-170, through
//! `r1_usb::safety`.

use serde_json::{Value, json};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

/// Where the companion keeps what it fetched for installs.
fn dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("r1os").join("companion")
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

/// Installs the R1's driver (and, with `mtk`, Rabbit's MediaTek preloader driver, fetched and checked first) behind
/// one administrator prompt. Returns the new status and the step's log.
#[tauri::command]
pub async fn drivers_install(mtk: bool) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = drivers_dir();
        let exe = if mtk { Some(r1_usb::drivers::fetch_mtk(&dir)?) } else { None };
        let log = r1_usb::drivers::install(&dir, exe.as_deref())?;
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

