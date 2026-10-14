//! An R1's backup, as the companion's wizard makes it (T-169). Both routes only read.
//! - A booted R1 on USB with adb allowed and root (r1os's image, or Magisk): `run` reads over adb, at USB speed.
//! - Otherwise (a stock R1), or with `--preloader`: through the preloader at power-on with mtkclient, about 1 MB/s.
//!   `fetch` downloads uv and mtkclient (pinned) first; Windows needs Rabbit's MediaTek driver
//!   (`drivers -- install --mtk`); the R1 is powered off, then plugged in while it waits.
//!
//! Backups go to `Documents\r1os backups` unless `R1OS_BACKUPS` names another folder.
//!
//!     cargo run -p r1-usb --example backup -- status
//!     cargo run -p r1-usb --example backup -- fetch
//!     cargo run -p r1-usb --example backup -- run [--preloader]
//!     cargo run -p r1-usb --example backup -- verify <folder> <identity>
//!     cargo run -p r1-usb --example backup -- latest <identity>
//!     cargo run -p r1-usb --example backup -- gpt <file>

use r1_usb::backup;
use std::path::PathBuf;

/// The R1 booted and on USB with adb allowed, if there's one: platform-tools from the companion's folder, else `adb`.
fn booted_r1() -> Option<r1_usb::adb::Adb> {
    let exe = r1_usb::tools::find(&r1_usb::folders::companion()).map(|t| t.adb).unwrap_or_else(|| PathBuf::from("adb"));
    let (serial, _) = r1_usb::adb::devices(&exe).ok()?.into_iter().find(|(_, s)| *s == r1_usb::adb::State::Device)?;
    Some(r1_usb::adb::Adb::new(exe, serial))
}

/// Whether Rabbit's MediaTek driver is installed (Windows; elsewhere mtkclient needs none).
fn mtk_driver() -> bool {
    #[cfg(windows)]
    return r1_usb::drivers::status().map(|s| s.mtk).unwrap_or(false);
    #[cfg(not(windows))]
    true
}

fn main() -> anyhow::Result<()> {
    let dir = backup::tools_dir();
    let root = std::env::var_os("R1OS_BACKUPS").map(PathBuf::from).unwrap_or_else(backup::default_root);
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("status") | None => println!(
            "{}",
            serde_json::json!({ "tools": backup::find(&dir).is_some(), "tools_dir": dir, "mtk_driver": mtk_driver(), "root": root })
        ),
        Some("fetch") => {
            let mut shown = (String::new(), 0u64);
            backup::fetch(&dir, |what, got, total| {
                let pct = got * 100 / total.max(1);
                if shown.0 != what || pct >= shown.1 + 10 || got == total {
                    eprintln!("{what}: {pct}%");
                    shown = (what.to_owned(), pct);
                }
            })?;
            eprintln!("Python and mtkclient's packages…");
            backup::prepare(&backup::find(&dir).expect("just fetched"))?;
            println!("{}", serde_json::json!({ "tools": true }));
        }
        Some("run") if !args.iter().any(|a| a == "--preloader") && booted_r1().is_some() => {
            // A booted R1 with root: over adb, at USB speed.
            let adb = booted_r1().expect("just found");
            let started = std::time::Instant::now();
            let mut shown = std::time::Instant::now() - std::time::Duration::from_secs(1);
            let b = backup::run_adb(&adb, &root, &backup::stamp(), |part, done, total| {
                if shown.elapsed().as_millis() >= 200 || done == total {
                    shown = std::time::Instant::now();
                    let rate = done as f64 / started.elapsed().as_secs_f64().max(0.001) / 1e6;
                    eprint!("\r  reading {part:<16} {:>4} / {} MB  {rate:>5.1} MB/s   ", done >> 20, total >> 20);
                }
            })?;
            let mb: u64 = b.parts.iter().map(|(_, s)| s).sum::<u64>() >> 20;
            eprintln!("\r  {} partitions, {mb} MB in {:.0} s, checked against the GPT and summed{:20}", b.parts.len(), started.elapsed().as_secs_f64(), "");
            println!("{}", serde_json::json!({ "dir": b.dir, "identity": b.identity }));
        }
        Some("run") => {
            let tools = backup::find(&dir).ok_or_else(|| anyhow::anyhow!("no tools in {} yet: run `fetch` first", dir.display()))?;
            #[cfg(windows)]
            if !r1_usb::drivers::status()?.mtk {
                anyhow::bail!("Rabbit's MediaTek driver isn't installed: `cargo run -p r1-usb --example drivers -- install --mtk`");
            }
            eprintln!("Power the R1 off, then plug it in…");
            let b = backup::run(&tools, &root, &backup::stamp(), &backup::Stop::default(), |line| eprintln!("  {line}"))?;
            println!("{}", serde_json::to_string_pretty(&b)?);
        }
        Some("verify") => {
            let (Some(folder), Some(id)) = (args.get(1), args.get(2)) else { anyhow::bail!("verify <folder> <identity>") };
            backup::verify(folder.as_ref(), id)?;
            println!("{}", serde_json::json!({ "ok": true }));
        }
        Some("gpt") => {
            // A GPT's partitions, and whether it has every one the backup reads (`r gpt`'s file, or the first
            // 176 sectors of an R1's mmcblk0).
            let Some(file) = args.get(1) else { anyhow::bail!("gpt <file>") };
            let parts = backup::gpt_sizes(&std::fs::read(file)?).ok_or_else(|| anyhow::anyhow!("not a GPT"))?;
            let missing: Vec<&str> = backup::PARTS.iter().copied().filter(|p| !parts.iter().any(|(n, _)| n == p)).collect();
            let read: u64 = parts.iter().filter(|(n, _)| backup::PARTS.contains(&n.as_str())).map(|(_, s)| s).sum();
            println!("{}", serde_json::json!({ "parts": parts, "missing": missing, "backup_bytes": read }));
        }
        Some("latest") => {
            let Some(id) = args.get(1) else { anyhow::bail!("latest <identity>") };
            println!("{}", serde_json::json!({ "latest": backup::latest(&root, id) }));
        }
        Some(other) => anyhow::bail!("status, fetch, run, verify or latest, not {other}"),
    }
    Ok(())
}
