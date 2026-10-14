//! Flashing r1os onto an R1, as the companion's wizard does it (T-170, ADR-0067): everything from the bootloader, never
//! fastbootd. `stock`, `status`, `read-r1`, `check` and the `super-*` commands only read (and build files on the PC);
//! `run` writes the R1: vbmeta, `super` (r1os's system in, the R1's own vendor kept), then the wipe, after showing the
//! plan and reading `ERASE` typed back.
//!
//!     cargo run -p r1-usb --example flash -- stock [--from <extracted firmware folder>]
//!     cargo run -p r1-usb --example flash -- read-r1           (a booted R1 with root: super's head and para, read only)
//!     cargo run -p r1-usb --example flash -- status
//!     cargo run -p r1-usb --example flash -- fastboot          (from power-off, through the preloader)
//!     cargo run -p r1-usb --example flash -- check             (in the bootloader; no reboots)
//!     cargo run -p r1-usb --example flash -- run <r1os system .img>
//!     cargo run -p r1-usb --example flash -- super-head <file> | super-build <head> <system> <out> [a|b] | super-probe <head> <out>

use r1_usb::{fastboot::Fastboot, flash, stock};
use std::path::PathBuf;

fn stock_dir() -> PathBuf {
    r1_usb::folders::companion().join("stock-v0.8.293")
}

/// Where an R1's `super` head and `para` are kept between `read-r1` (booted) and `check` (in the bootloader).
fn r1_dir(id: &str) -> PathBuf {
    r1_usb::folders::companion().join("r1").join(id)
}

/// `super`'s head and `para` as `read-r1` kept them for this R1.
fn read_back(fb: &Fastboot) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let dir = r1_dir(&fb.id());
    let head = std::fs::read(dir.join("super-head.bin")).map_err(|_| anyhow::anyhow!("nothing read from this R1 yet: boot it and run `read-r1` first"))?;
    Ok((head, std::fs::read(dir.join("para.bin"))?))
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
        Some("read-r1") => {
            let exe = r1_usb::tools::find(&r1_usb::folders::companion()).map(|t| t.adb).unwrap_or_else(|| PathBuf::from("adb"));
            let (serial, _) = r1_usb::adb::devices(&exe)?
                .into_iter()
                .find(|(_, s)| *s == r1_usb::adb::State::Device)
                .ok_or_else(|| anyhow::anyhow!("no booted R1 on USB with adb allowed"))?;
            let adb = r1_usb::adb::Adb::new(exe, serial);
            let root = adb.root()?;
            let dir = r1_dir(&adb.id());
            std::fs::create_dir_all(&dir)?;
            adb.read_partition_head(root, "super", flash::SUPER_HEAD, &dir.join("super-head.bin"))?;
            adb.read_partition(root, "para", &dir.join("para.bin"), |_| {})?;
            let head = std::fs::read(dir.join("super-head.bin"))?;
            r1_usb::lp::read(&head, 0)?;
            let merge = r1_usb::superimg::merge_status(&std::fs::read(dir.join("para.bin"))?)?;
            println!("{}", serde_json::json!({ "r1": adb.id(), "dir": dir, "merge": merge }));
        }
        Some("check") => {
            let fb = r1()?;
            let (head, para) = read_back(&fb)?;
            println!("{}", serde_json::to_string(&flash::check(&fb, &head, &para)?)?);
        }
        Some("run") => {
            let system = PathBuf::from(args.get(1).ok_or_else(|| anyhow::anyhow!("run <r1os system .img>"))?);
            let stock = stock::find(&stock_dir()).ok_or_else(|| anyhow::anyhow!("no stock files yet: run `stock` first"))?;
            let fb = r1()?;
            let (head, para) = read_back(&fb)?;
            let checked = flash::check(&fb, &head, &para)?;
            eprintln!("Building super for this R1 (on the PC; nothing written yet)…");
            let plan = flash::prepare(&checked, &system, stock, &r1_usb::folders::companion().join("flash-work"), |_, _| {})?;
            eprintln!("
About to:");
            for l in plan.describe() {
                eprintln!("  - {l}");
            }
            eprint!("
Type {} to go ahead: ", flash::CONFIRM);
            let mut typed = String::new();
            std::io::stdin().read_line(&mut typed)?;
            flash::run(&fb, &checked, &plan, &typed, &mut |l| eprintln!("  {l}"))?;
            eprintln!("Done. Never take Rabbit's update on r1os.");
        }
        _ => anyhow::bail!("stock, read-r1, status, fastboot, check, run, super-head, super-build or super-probe"),
    }
    Ok(())
}
