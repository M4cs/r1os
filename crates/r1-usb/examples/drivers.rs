//! The R1's Windows drivers, as the companion's wizard installs them (T-168). `install` and `undo` show Windows'
//! administrator prompt; `status` only reads the driver store.
//!
//!     cargo run -p r1-usb --example drivers -- status
//!     cargo run -p r1-usb --example drivers -- install [--mtk]
//!     cargo run -p r1-usb --example drivers -- undo

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use r1_usb::drivers;
    let dir = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("r1os")
        .join("companion")
        .join("drivers");
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("status") | None => println!("{}", serde_json::to_string(&drivers::status()?)?),
        Some("install") => {
            let mtk = if args.iter().any(|a| a == "--mtk") {
                eprintln!("fetching Rabbit's MediaTek driver installer…");
                Some(drivers::fetch_mtk(&dir)?)
            } else {
                None
            };
            eprintln!("Windows will ask for permission…");
            print!("{}", drivers::install(&dir, mtk.as_deref())?);
            println!("{}", serde_json::to_string(&drivers::status()?)?);
        }
        Some("undo") => {
            eprintln!("Windows will ask for permission…");
            print!("{}", drivers::undo(&dir)?);
            println!("{}", serde_json::to_string(&drivers::status()?)?);
        }
        Some(other) => anyhow::bail!("status, install [--mtk] or undo, not {other}"),
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("drivers are a Windows matter");
}
