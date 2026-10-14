//! The first setup after the flash, as the companion's wizard will do it (T-170, ADR-0065 decision 7): `/data` grown if
//! it's still Rabbit's small filesystem, then r1d, the shell, speech-to-text, the models there are, and the agent box.
//!
//! **`run` writes to the R1.** It grows `/data` (`resize2fs`, online), writes `/data/r1os`, restarts r1d, and builds the
//! agent box (which downloads Alpine's packages and Claude Code on the R1). CLAUDE.md: an agent asks the R1's owner in
//! chat before running it, naming this command. It shows the plan, then goes ahead only when `yes` is typed (`YES=1`
//! skips the question: only after the owner's OK for that run).
//!
//! `plan` needs no R1: it shows the steps for a just-flashed one ([`r1_usb::setup::Status::fresh`]). `status` only reads.
//!
//! The inputs come from a checkout (`--checkout`, default `.`): `target/aarch64-linux-android/release/r1d`
//! (`tools/build-r1d.sh`), `shell/dist`, `out/speech`, `out/system1` and `out/translate` if they're there, and
//! `firmware/agent-box` (`tools/agent-box.sh install`'s downloads).
//!
//!     cargo run -p r1-usb --example setup -- plan   [--checkout <dir>] [--no-box] [--no-models]
//!     cargo run -p r1-usb --example setup -- status
//!     cargo run -p r1-usb --example setup -- run    [--checkout <dir>] [--no-box] [--no-models]

use r1_usb::setup::{self, Inputs, Progress, Setup, Status};
use std::path::PathBuf;

/// The R1 booted and on USB with adb allowed: platform-tools from the companion's folder, else `adb`.
fn booted_r1() -> anyhow::Result<r1_usb::adb::Adb> {
    let exe = r1_usb::tools::find(&r1_usb::folders::companion()).map(|t| t.adb).unwrap_or_else(|| PathBuf::from("adb"));
    let mut found: Vec<String> =
        r1_usb::adb::devices(&exe)?.into_iter().filter(|(_, s)| *s == r1_usb::adb::State::Device).map(|(serial, _)| serial).collect();
    match found.len() {
        0 => anyhow::bail!("no booted R1 on USB with adb allowed"),
        1 => Ok(r1_usb::adb::Adb::new(exe, found.remove(0))),
        _ => anyhow::bail!("more than one device on adb: leave just the R1"),
    }
}

fn inputs(args: &[String]) -> anyhow::Result<Setup> {
    let checkout = args.iter().position(|a| a == "--checkout").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let mut i = Inputs::from_checkout(&checkout);
    if args.iter().any(|a| a == "--no-box") {
        i.agent_box = None;
    }
    if args.iter().any(|a| a == "--no-models") {
        (i.system1, i.translator) = (None, None);
    }
    eprintln!("Reading the inputs from {}…", checkout.display());
    Setup::new(i)
}

fn show(plan: &setup::Plan) {
    for a in plan.already() {
        println!("  already there: {a}");
    }
    if plan.is_empty() {
        println!("Nothing to do.");
    }
    for (n, w) in plan.describe().iter().enumerate() {
        println!("{:>3}. {w}", n + 1);
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("plan") => {
            let plan = inputs(&args)?.plan(&Status::fresh())?;
            println!("On a just-flashed R1 (assumed: `status` reads a real one):");
            show(&plan);
        }
        Some("status") => {
            let adb = booted_r1()?;
            let s = setup::status(&setup::OverAdb::new(&adb)?)?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "r1": adb.id(), "status": s }))?);
        }
        Some("run") => {
            let setup = inputs(&args)?;
            let adb = booted_r1()?;
            let r1 = setup::OverAdb::new(&adb)?;
            let plan = setup.plan(&setup::status(&r1)?)?;
            println!("On R1 {}:", adb.id());
            show(&plan);
            if plan.is_empty() {
                return Ok(());
            }
            if std::env::var("YES").as_deref() != Ok("1") {
                eprint!("This writes to the R1. Type yes to go ahead: ");
                let mut typed = String::new();
                std::io::stdin().read_line(&mut typed)?;
                if typed.trim() != "yes" {
                    anyhow::bail!("not set up");
                }
            }
            let after = setup::run(&r1, &plan, &mut |p| match p {
                Progress::Step { index, of, words } => eprintln!("[{index}/{of}] {words}"),
                Progress::Line(l) => eprintln!("      {l}"),
            })?;
            println!("{}", serde_json::to_string_pretty(&after)?);
        }
        _ => anyhow::bail!("plan | status | run (see the top of examples/setup.rs)"),
    }
    Ok(())
}
