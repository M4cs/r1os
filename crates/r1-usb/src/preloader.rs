//! Into fastboot from power-off (T-170). The R1 has no volume keys, so there's no button combination: for about a
//! second as a powered-off R1 starts, its preloader opens a USB serial port (`0E8D:2000`) and prints `READY`, and
//! answering `FASTBOOT` makes the bootloader stop in fastboot. `tools/fastboot-entry.py` does the same (from r1ptt;
//! r1_escape's `mtkbootcmd.py` too). It's a boot-mode request: nothing is written to the R1.
//!
//! On Windows the port appears only with Rabbit's MediaTek driver ([`crate::drivers`]). From a booted R1,
//! `adb reboot bootloader` is simpler.

use anyhow::{Result, bail};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The preloader's serial port, if one is there now.
pub fn port() -> Option<String> {
    serialport::available_ports().ok()?.into_iter().find_map(|p| match p.port_type {
        serialport::SerialPortType::UsbPort(u) if u.vid == crate::VID_MEDIATEK && u.pid == crate::PID_PRELOADER => Some(p.port_name),
        _ => None,
    })
}

/// Waits up to [limit] for a powered-off R1 to be plugged in, and asks its preloader for fastboot. [stop] ends the
/// wait early (the wizard's Cancel).
pub fn enter_fastboot(limit: Duration, stop: &AtomicBool) -> Result<()> {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            bail!("stopped");
        }
        if let Some(name) = port()
            && answer(&name).unwrap_or(false)
        {
            return Ok(());
        }
        // The window is about a second: look often.
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("no R1 started up within {} s (power it off, then plug it in; on Windows, Rabbit's MediaTek driver)", limit.as_secs())
}

/// `READY` read, `FASTBOOT` sent. False if the port went before it said so (the moment was missed).
fn answer(name: &str) -> Result<bool> {
    let mut p = serialport::new(name, 115_200).timeout(Duration::from_secs(2)).open()?;
    let mut ready = [0u8; 5];
    p.read_exact(&mut ready)?;
    if &ready != b"READY" {
        return Ok(false);
    }
    p.write_all(b"FASTBOOT")?;
    p.flush()?;
    Ok(true)
}
