//! One R1 over adb: what the install reads from a booted R1, and the reboot into fastboot. The first setup's pushes
//! (r1d, the shell, the agent box) come with T-170.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

/// What `adb devices` says of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Device,
    /// Waiting for "Allow USB debugging?" on the R1.
    Unauthorized,
    Recovery,
    Sideload,
    Offline,
}

/// The serials `adb devices` lists, with their state.
pub fn devices(exe: &PathBuf) -> Result<Vec<(String, State)>> {
    let out = command(exe).arg("devices").output().context("adb didn't run")?;
    Ok(parse_devices(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_devices(text: &str) -> Vec<(String, State)> {
    text.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (serial, state) = (it.next()?, it.next()?);
            let state = match state {
                "device" => State::Device,
                "unauthorized" => State::Unauthorized,
                "recovery" => State::Recovery,
                "sideload" => State::Sideload,
                "offline" => State::Offline,
                _ => return None,
            };
            // Wireless debugging's addresses aren't the R1 on this cable.
            (!serial.contains(':')).then(|| (serial.to_owned(), state))
        })
        .collect()
}

pub struct Adb {
    exe: PathBuf,
    serial: String,
}

impl std::fmt::Debug for Adb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Adb({})", crate::identity(&self.serial))
    }
}

impl Adb {
    pub fn new(exe: PathBuf, serial: String) -> Self {
        Self { exe, serial }
    }

    /// A system property (`ro.r1os.image.version`, `ro.product.model`…). Read only.
    pub fn getprop(&self, name: &str) -> Result<String> {
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
            bail!("not a property's name");
        }
        let out = command(&self.exe).args(["-s", &self.serial, "shell", "getprop", name]).output().context("adb didn't run")?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    /// Into the bootloader's fastboot (`adb reboot bootloader`), or fastbootd.
    pub fn reboot_to_fastboot(&self, userspace: bool) -> Result<()> {
        let out = command(&self.exe)
            .args(["-s", &self.serial, "reboot", if userspace { "fastboot" } else { "bootloader" }])
            .output()
            .context("adb didn't run")?;
        if !out.status.success() {
            bail!("adb reboot failed: {}", crate::fastboot::scrub(String::from_utf8_lossy(&out.stderr).trim(), &self.serial));
        }
        Ok(())
    }
}

fn command(exe: &PathBuf) -> Command {
    let mut c = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_from_adb_s_output() {
        let out = "* daemon started successfully\nList of devices attached\nABC123\tdevice\nDEF456\tunauthorized\n192.168.1.9:5555\tdevice\n\n";
        assert_eq!(parse_devices(out), vec![("ABC123".to_owned(), State::Device), ("DEF456".to_owned(), State::Unauthorized)]);
        assert!(parse_devices("List of devices attached\n\n").is_empty());
    }
}
