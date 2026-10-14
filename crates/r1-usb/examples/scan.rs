//! Fetches platform-tools into a folder (once) and lists the R1s plugged in, as the companion will show them.
//! Reads only.
//!
//!     cargo run -p r1-usb --example scan -- <folder>

fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "r1-usb-tools".into()));
    let tools = match r1_usb::tools::find(&dir) {
        Some(t) => t,
        None => {
            eprintln!("fetching platform-tools into {}…", dir.display());
            r1_usb::tools::fetch(&dir, |got, total| {
                if got == total {
                    eprintln!("  {total} bytes, checked");
                }
            })?
        }
    };
    eprintln!("platform-tools {}", tools.version);
    println!("{}", serde_json::to_string_pretty(&r1_usb::scan::scan(Some(&tools)))?);
    Ok(())
}
