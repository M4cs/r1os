//! r1os Companion (ADR-0047, docs/specs/companion.md): your R1, on your computer.
//!
//! A tray app with one window. The link ([`link::Link`]) runs from start to quit, window or not; closing the window
//! hides it. `--minimized` (how it starts with Windows, when that's on) starts with the window hidden.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod link;
mod codex;
mod install;
mod runner;

use base64::Engine;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

type Shared = Arc<link::Link>;

#[tauri::command]
fn link_state(link: State<'_, Shared>) -> Value {
    link.state()
}

#[tauri::command]
fn pair_start(link: State<'_, Shared>) -> Result<Value, String> {
    link.start_pairing().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn pair_stop(link: State<'_, Shared>) {
    link.stop_pairing();
}

#[tauri::command]
fn pair_answer(link: State<'_, Shared>, yes: bool) {
    link.answer(yes);
}

#[tauri::command]
fn unlink(link: State<'_, Shared>, id: String) -> Result<(), String> {
    link.unlink(&id).map_err(|e| format!("{e:#}"))
}

/// One of an R1's allowed routes (r1d decides which): `{status, body}`. `base64` and `mime` send bytes instead of a
/// JSON body.
#[tauri::command]
async fn r1_api(
    link: State<'_, Shared>,
    id: String,
    method: String,
    path: String,
    body: Option<Value>,
    base64: Option<String>,
    mime: Option<String>,
) -> Result<Value, String> {
    let mut params = json!({ "method": method, "path": path, "body": body.unwrap_or(Value::Null) });
    if let Some(b) = base64 {
        params["base64"] = json!(b);
        params["mime"] = json!(mime.unwrap_or_else(|| "application/octet-stream".into()));
    }
    link.call_api(&id, params).await.map_err(|e| format!("{e:#}"))
}

/// The companion's settings (how long an R1's link stays when nothing's going on).
#[tauri::command]
fn settings_get(link: State<'_, Shared>) -> link::Settings {
    link.settings()
}

#[tauri::command]
async fn settings_set(link: State<'_, Shared>, settings: link::Settings) -> Result<(), String> {
    link.set_settings(settings).await.map_err(|e| format!("{e:#}"))
}

/// Agents working on this computer: the CLIs here, turns running, the last ones (crate::runner).
#[tauri::command]
async fn pc_status(link: State<'_, Shared>) -> Result<Value, String> {
    match link.runner() {
        Some(r) => Ok(r.status().await),
        None => Err("the runner isn't up".into()),
    }
}

/// Signs Claude Code in on this computer (its own `claude auth login`).
#[tauri::command]
async fn claude_sign_in(link: State<'_, Shared>) -> Result<(), String> {
    let runner = link.runner().ok_or("the runner isn't up")?;
    runner.claude_sign_in().await.map_err(|e| format!("{e:#}"))
}

/// Signs the companion's own Codex in (Codex's page in the browser; the CLI keeps the login: crate::codex).
#[tauri::command]
async fn codex_sign_in(link: State<'_, Shared>) -> Result<(), String> {
    let runner = link.runner().ok_or("the runner isn't up")?;
    runner.codex_sign_in().await.map_err(|e| format!("{e:#}"))
}

/// A name that's free in `dir`: `name`, else `name (2)`, `name (3)`…
fn free(dir: &Path, name: &str) -> PathBuf {
    const BAD: &str = "<>:\"/\\|?*";
    let clean: String = name.chars().map(|c| if BAD.contains(c) || c.is_control() { '-' } else { c }).collect();
    let (stem, ext) = match clean.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
        _ => (clean.clone(), String::new()),
    };
    (1..).map(|n| dir.join(if n == 1 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") })).find(|p| !p.exists()).unwrap()
}

/// Saves something from the R1 (a photo, a recording, an app's spec) in Downloads\r1os: its path.
#[tauri::command]
fn save_download(app: AppHandle, name: String, base64: Option<String>, text: Option<String>) -> Result<String, String> {
    let dir = app.path().download_dir().map_err(|e| e.to_string())?.join("r1os");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = match (base64, text) {
        (Some(b), _) => base64::engine::general_purpose::STANDARD.decode(b).map_err(|e| e.to_string())?,
        (None, Some(t)) => t.into_bytes(),
        _ => return Err("nothing to save".into()),
    };
    let path = free(&dir, &name);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Backs an R1's data up into Documents\r1os backups\<label> (r1d's crate::backup: core.db, the apps, their data,
/// media, the trash), a part at a time, saying how far it's got (`backup {r1, done, total}`): `{dir, files, bytes}`.
#[tauri::command]
async fn backup_r1(app: AppHandle, link: State<'_, Shared>, id: String, label: String) -> Result<Value, String> {
    let fail = |e: anyhow::Error| format!("{e:#}");
    let got = link.api(&id, "GET", "/api/backup", Value::Null).await.map_err(fail)?;
    if got["status"].as_u64() != Some(200) {
        return Err(got["body"]["error"].as_str().unwrap_or("the r1 couldn't start a backup").to_owned());
    }
    let list = &got["body"];
    let total = list["bytes"].as_u64().unwrap_or(0);
    let root = app.path().document_dir().map_err(|e| e.to_string())?.join("r1os backups");
    let dir = free(&root, &label);
    let mut done = 0u64;
    for f in list["files"].as_array().cloned().unwrap_or_default() {
        let path = f["path"].as_str().unwrap_or_default();
        let size = f["bytes"].as_u64().unwrap_or(0);
        let to = dir.join(path);
        std::fs::create_dir_all(to.parent().unwrap_or(&dir)).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(size as usize);
        while (out.len() as u64) < size {
            let q = format!("/api/backup/file?path={}&offset={}&len={}", encode(path), out.len(), 8 << 20);
            let part = link.api(&id, "GET", &q, Value::Null).await.map_err(fail)?;
            let Some(b64) = part["base64"].as_str() else {
                return Err(part["body"]["error"].as_str().unwrap_or("a part didn't come").to_owned());
            };
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
            if bytes.is_empty() {
                break;
            }
            done += bytes.len() as u64;
            out.extend_from_slice(&bytes);
            let _ = app.emit("backup", json!({ "r1": id, "done": done, "total": total, "file": path }));
        }
        std::fs::write(&to, out).map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join("backup.json"), serde_json::to_vec_pretty(list).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(json!({ "dir": dir.to_string_lossy(), "files": list["files"].as_array().map_or(0, Vec::len), "bytes": done }))
}

/// A backup path in a query: letters, digits and `/-_.` as they are, the rest %-encoded.
fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"/-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,iroh=warn,noq=warn".into()))
        .init();
    let minimized = std::env::args().any(|a| a == "--minimized");
    tauri::Builder::default()
        // Sign-in pages open in the browser (ADR-0047 §7.1).
        .plugin(tauri_plugin_opener::init())
        // Folders for agents working here.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .invoke_handler(tauri::generate_handler![
            link_state, pair_start, pair_stop, pair_answer, unlink, r1_api, save_download, backup_r1, settings_get, settings_set, pc_status, codex_sign_in, claude_sign_in,
            install::usb_tools, install::usb_fetch_tools, install::usb_scan, install::drivers_status, install::drivers_install,
            install::drivers_undo, install::backup_status, install::backup_fetch_tools, install::backup_run,
            install::backup_stop, install::backup_skip, install::flash_stock_status, install::flash_fetch_stock,
            install::flash_read_status, install::flash_read_r1, install::flash_to_bootloader, install::flash_leave_bootloader,
            install::flash_enter_fastboot, install::flash_enter_fastboot_stop, install::flash_unlock_ability,
            install::flash_check, install::flash_prepare, install::flash_prepared, install::flash_run
        ])
        // The flash's check and plan, between its steps.
        .manage(install::Flash::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            let link = tauri::async_runtime::block_on(link::Link::start(handle.clone()))?;
            app.manage(link);

            let open = MenuItem::with_id(app, "open", "open r1os", true, None::<&str>)?;
            let pair = MenuItem::with_id(app, "pair", "link an R1", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &pair, &quit])?;
            TrayIconBuilder::with_id("r1os")
                .icon(app.default_window_icon().cloned().ok_or("no icon")?)
                .tooltip("r1os")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id().as_ref() {
                    "open" => show(app),
                    "pair" => {
                        show(app);
                        let _ = app.emit("pair", ());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            if !minimized {
                show(&handle);
            }
            Ok(())
        })
        .on_window_event(|w, e| {
            // Closing the window keeps the companion in the tray (the link stays).
            if let WindowEvent::CloseRequested { api, .. } = e {
                let _ = w.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("r1os Companion couldn't start");
}
