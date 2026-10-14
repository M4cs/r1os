//! Flashing r1os onto an R1, as the companion's wizard does it (T-170). `stock` and `status` only read; `check` reboots
//! the R1 into fastbootd to read its vendor partition's size (no writes); `run` erases the R1 and flashes it, after
//! showing the plan and reading `ERASE` typed back.
//!
//!     cargo run -p r1-usb --example flash -- stock [--from <extracted firmware folder>]
//!     cargo run -p r1-usb --example flash -- status
//!     cargo run -p r1-usb --example flash -- fastboot      (from power-off, through the preloader)
//!     cargo run -p r1-usb --example flash -- check
//!     cargo run -p r1-usb --example flash -- run <r1os system .img> [--keep-product]

use r1_usb::{fastboot::Fastboot, flash, stock};
use std::path::PathBuf;

fn stock_dir() -> PathBuf {
    r1_usb::folders::companion().join("stock-v0.8.293")
}

/// The R1 in fastboot: platform-tools from the companion's folder, else `fastboot`.
fn r1() -> anyhow::Result<Fastboot> {
    let exe = r1_usb::tools::find(&r1_usb::folders::companion()).map(|t| t.fastboot).unwrap_or_else(|| PathBuf::from("fastboot"));
    let mut found = r1_usb::fastboot::devices(&exe)?;
    match found.len() {
        0 => anyhow::bail!("no R1 in fastboot (from r1os: `adb reboot bootloader`)"),
        1 => Ok(Fastboot::new(exe, found.remove(0))),
        _ => anyhow::bail!("more than one device in fastboot: leave just the R1"),
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("stock") => {
            let s = match args.iter().position(|a| a == "--from").and_then(|i| args.get(i + 1)) {
                Some(src) => stock::from_folder(src.as_ref(), &stock_dir())?,
                None => match stock::find(&stock_dir()) {
                    Some(s) => s,
                    None => stock::fetch(&stock_dir(), |name| eprintln!("  {name}"))?,
                },
            };
            println!("{}", serde_json::json!({ "stock": s.dir }));
        }
        Some("status") => {
            let fb = r1()?;
            let v = |n: &str| fb.getvar(n).ok().flatten();
            println!(
                "{}",
                serde_json::json!({
                    "r1": fb.id(), "fastbootd": v("is-userspace").as_deref() == Some("yes"),
                    "unlocked": v("unlocked"), "unlock_ability": fb.unlock_ability().ok().flatten(), "slot": v("current-slot"),
                    "stock": stock::find(&stock_dir()).is_some(),
                })
            );
        }
        Some("fastboot") => {
            eprintln!("Power the R1 off, then plug it in…");
            r1_usb::preloader::enter_fastboot(std::time::Duration::from_secs(600), &Default::default())?;
            eprintln!("Asked for fastboot: `fastboot devices` should list the R1 in a few seconds.");
        }
        Some("check") => println!("{}", serde_json::to_string(&flash::check(&r1()?)?)?),
        Some("run") => {
            let system = PathBuf::from(args.get(1).ok_or_else(|| anyhow::anyhow!("run <r1os system .img>"))?);
            let stock = stock::find(&stock_dir()).ok_or_else(|| anyhow::anyhow!("no stock files yet: run `stock` first"))?;
            let fb = r1()?;
            eprintln!("Checking the R1 (into fastbootd; nothing written)…");
            let checked = flash::check(&fb)?;
            let plan = flash::Plan {
                slot: checked.slot,
                system,
                stock,
                work: r1_usb::folders::companion().join("flash-work"),
                keep_product: args.iter().any(|a| a == "--keep-product"),
            };
            eprintln!("\nAbout to, on slot {}:", plan.slot);
            for l in plan.describe() {
                eprintln!("  - {l}");
            }
            eprint!("\nType {} to go ahead: ", flash::CONFIRM);
            let mut typed = String::new();
            std::io::stdin().read_line(&mut typed)?;
            flash::run(&fb, &checked, &plan, &typed, &mut |l| eprintln!("  {l}"))?;
            eprintln!("Done. Never take Rabbit's update on r1os.");
        }
        _ => anyhow::bail!("stock, status, fastboot, check or run"),
    }
    Ok(())
}
