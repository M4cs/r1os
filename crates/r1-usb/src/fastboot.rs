//! One R1 in fastboot (the bootloader's, or fastbootd's in userspace): reads by name, writes only through
//! [`crate::safety`]. The serial addresses it and goes nowhere else.

use crate::safety::{self, Write};
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Where to reboot to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reboot {
    System,
    /// The bootloader's fastboot.
    Bootloader,
    /// fastbootd (userspace, where logical partitions live).
    Fastbootd,
}

pub struct Fastboot {
    exe: PathBuf,
    serial: String,
}

impl std::fmt::Debug for Fastboot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Fastboot({})", crate::identity(&self.serial))
    }
}

/// The serials `fastboot devices` lists.
pub fn devices(exe: &PathBuf) -> Result<Vec<String>> {
    let out = command(exe).arg("devices").output().context("fastboot didn't run")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            match (it.next(), it.next()) {
                (Some(s), Some("fastboot")) => Some(s.to_owned()),
                _ => None,
            }
        })
        .collect())
}

impl Fastboot {
    pub fn new(exe: PathBuf, serial: String) -> Self {
        Self { exe, serial }
    }

    pub fn id(&self) -> String {
        crate::identity(&self.serial)
    }

    /// A variable the bootloader reports (`unlocked`, `current-slot`, `is-userspace`, `partition-size:<part>`…).
    /// fastboot prints them on stderr as `name: value`.
    pub fn getvar(&self, name: &str) -> Result<Option<String>> {
        if name.is_empty() || name.contains(char::is_whitespace) {
            bail!("not a variable's name");
        }
        let out = command(&self.exe).args(["-s", &self.serial, "getvar", name]).output().context("fastboot didn't run")?;
        let text = String::from_utf8_lossy(&out.stderr);
        Ok(value_of(&text, name))
    }

    /// Whether it's unlocked (`getvar unlocked`).
    pub fn unlocked(&self) -> Result<Option<bool>> {
        Ok(self.getvar("unlocked")?.map(|v| v == "yes"))
    }

    /// `fastboot flashing get_unlock_ability`'s number, as the bootloader gives it. Read only. The R1's says
    /// `(bootloader) unlock_ability = 16777216` (`0x01000000`, unlocked, 2026-10-14), where 2026-10-08's note had `0` on
    /// the same R1: what it means isn't settled (**unverified**), so it's reported, not judged.
    pub fn unlock_ability(&self) -> Result<Option<u64>> {
        let out = command(&self.exe).args(["-s", &self.serial, "flashing", "get_unlock_ability"]).output().context("fastboot didn't run")?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
        Ok(unlock_ability_of(&text))
    }

    /// fastbootd (userspace) rather than the bootloader.
    pub fn userspace(&self) -> Result<bool> {
        Ok(self.getvar("is-userspace")?.as_deref() == Some("yes"))
    }

    pub fn reboot(&self, to: Reboot) -> Result<()> {
        let mut c = command(&self.exe);
        c.args(["-s", &self.serial]);
        match to {
            Reboot::System => c.arg("reboot"),
            Reboot::Bootloader => c.args(["reboot", "bootloader"]),
            Reboot::Fastbootd => c.args(["reboot", "fastboot"]),
        };
        check_status(c.output().context("fastboot didn't run")?, "reboot")
    }

    /// Waits until `fastboot devices` lists this R1 again (after a reboot into the bootloader or fastbootd), up to
    /// [limit].
    pub fn wait(&self, limit: std::time::Duration) -> Result<()> {
        let until = std::time::Instant::now() + limit;
        while std::time::Instant::now() < until {
            if devices(&self.exe).map(|d| d.contains(&self.serial)).unwrap_or(false) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        bail!("the R1 wasn't back in fastboot after {} s", limit.as_secs())
    }

    /// One write, refused unless [`safety::check`] allows it. [progress] hears fastboot's lines as they come.
    pub fn write(&self, w: &Write, mut progress: impl FnMut(&str)) -> Result<()> {
        let args = safety::args(w).map_err(anyhow::Error::new)?;
        let mut child = command(&self.exe)
            .args(["-s", &self.serial])
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("fastboot didn't run")?;
        let mut last = String::new();
        if let Some(err) = child.stderr.take() {
            for line in BufReader::new(err).lines() {
                let line = line?;
                progress(&line);
                last = line;
            }
        }
        let status = child.wait()?;
        if !status.success() {
            bail!("fastboot {} failed: {}", args[0], scrub(&last, &self.serial));
        }
        Ok(())
    }
}

fn command(exe: &PathBuf) -> Command {
    let mut c = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up from a GUI app.
        c.creation_flags(0x0800_0000);
    }
    c
}

fn check_status(out: std::process::Output, what: &str) -> Result<()> {
    if out.status.success() {
        return Ok(());
    }
    bail!("fastboot {what} failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or_default())
}

/// `name: value` from fastboot's output (`(bootloader) name: value` in some versions).
fn value_of(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.trim().trim_start_matches("(bootloader)").trim();
        let v = l.strip_prefix(name)?.strip_prefix(':')?.trim();
        (!v.is_empty()).then(|| v.to_owned())
    })
}

/// The number in `unlock_ability = N` (the R1's bootloader) or `get_unlock_ability: N` (AOSP's).
fn unlock_ability_of(text: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let (_, v) = l.split_once("unlock_ability")?;
        v.trim_start_matches([' ', '=', ':']).trim().parse().ok()
    })
}

/// An error's text without the serial in it.
pub(crate) fn scrub(text: &str, serial: &str) -> String {
    if serial.is_empty() { text.to_owned() } else { text.replace(serial, "<this R1>") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_from_fastboot_s_output() {
        let out = "unlocked: yes\nFinished. Total time: 0.002s\n";
        assert_eq!(value_of(out, "unlocked").as_deref(), Some("yes"));
        assert_eq!(value_of("(bootloader) current-slot: b\n", "current-slot").as_deref(), Some("b"));
        assert_eq!(value_of("is-userspace: no\n", "is-userspace").as_deref(), Some("no"));
        assert_eq!(value_of("unlocked:\n", "unlocked"), None);
        assert_eq!(value_of("something else\n", "unlocked"), None);
    }

    #[test]
    fn unlock_ability_either_way() {
        assert_eq!(unlock_ability_of("(bootloader) unlock_ability = 16777216\nOKAY [  0.001s]\n"), Some(16_777_216));
        assert_eq!(unlock_ability_of("(bootloader) get_unlock_ability: 1\n"), Some(1));
        assert_eq!(unlock_ability_of("FAILED (remote: 'unknown command')\n"), None);
    }

    #[test]
    fn errors_never_carry_the_serial() {
        assert_eq!(scrub("< waiting for ABC123 >", "ABC123"), "< waiting for <this R1> >");
        let f = Fastboot::new(PathBuf::from("fastboot"), "ABC123".into());
        assert!(!format!("{f:?}").contains("ABC123"));
    }
}
