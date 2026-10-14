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
        Some("super-head") => {
            // A `super`'s first bytes (its geometry and LP metadata): what's in it, and whether writing it back gives
            // the same bytes (ADR-0067's builder, checked against a real R1's).
            let file = args.get(1).ok_or_else(|| anyhow::anyhow!("super-head <file>"))?;
            let head = std::fs::read(file)?;
            let g = r1_usb::lp::geometry(&head)?;
            for slot in 0..g.metadata_slot_count {
                let m = r1_usb::lp::read(&head, slot)?;
                let bytes = m.to_bytes()?;
                let at = (r1_usb::lp::METADATA_OFFSET + slot as u64 * g.metadata_max_size as u64) as usize;
                let same = head[at..at + bytes.len()] == bytes[..];
                // liblp writes only the metadata, so a slot can keep bytes from a longer one past its tables.
                let stale = head[at + bytes.len()..at + g.metadata_max_size as usize].iter().filter(|&&b| b != 0).count();
                println!(
                    "slot {slot}: {} partitions, written back {}{}",
                    m.partitions.len(),
                    if same { "identical" } else { "DIFFERENT" },
                    if stale > 0 { format!(" ({stale} stale bytes past its tables, not part of it)") } else { String::new() }
                );
                if slot == 0 {
                    for p in &m.partitions {
                        println!("  {:<12} {:>10} sectors in {} extent(s), group {}", p.name, p.sectors(), p.extents.len(), m.groups[p.group as usize].name);
                    }
                }
            }
        }
        Some("super-build") => {
            // ADR-0067's image, on the PC only: built from a `super` head and a system image, then verified.
            let (Some(head), Some(system), Some(out)) = (args.get(1), args.get(2), args.get(3)) else {
                anyhow::bail!("super-build <super head> <system .img> <out> [a|b]")
            };
            let slot = args.get(4).and_then(|s| s.chars().next()).unwrap_or('a');
            let head = std::fs::read(head)?;
            let current = r1_usb::lp::read(&head, if slot == 'b' { 1 } else { 0 })?;
            let started = std::time::Instant::now();
            let mut shown = 0u64;
            let image = r1_usb::superimg::build(&current, slot, system.as_ref(), out.as_ref(), |done, total| {
                if done >= shown + (256 << 20) || done == total {
                    shown = done;
                    eprint!("\r  {} / {} MB", done >> 20, total >> 20);
                }
            })?;
            eprintln!("\r  built and verified in {:.0} s{:20}", started.elapsed().as_secs_f64(), "");
            for p in &image.metadata.partitions {
                let ex: Vec<String> = p.extents.iter().map(|e| format!("{}+{}", e.target_data, e.sectors)).collect();
                println!("  {:<10} {}", p.name, ex.join(", "));
            }
            println!("{}", serde_json::json!({ "image": image.path(), "bytes": std::fs::metadata(image.path())?.len() }));
        }
        Some("super-probe") => {
            let (Some(head), Some(out)) = (args.get(1), args.get(2)) else { anyhow::bail!("super-probe <super head> <out>") };
            let image = r1_usb::superimg::probe(&std::fs::read(head)?, out.as_ref())?;
            println!("{}", serde_json::json!({ "probe": image.path(), "bytes": std::fs::metadata(image.path())?.len() }));
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
