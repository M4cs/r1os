//! One R1 over adb: what the install reads from a booted R1 (its properties; its partitions as root, for the backup,
//! T-169), the reboot into fastboot, and the first setup's commands and pushes (T-170), which only
//! [`crate::setup`] can make.

use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How a booted R1's shell gets root, for reading partitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    /// adbd runs as root (r1os's image, after `adb root`).
    Adbd,
    /// Magisk's `su`.
    Su,
}

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

    /// This R1's identity (a hash of its serial), as the scan and backups name it.
    pub fn id(&self) -> String {
        crate::identity(&self.serial)
    }

    /// A system property (`ro.r1os.image.version`, `ro.product.model`…). Read only.
    pub fn getprop(&self, name: &str) -> Result<String> {
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
            bail!("not a property's name");
        }
        let out = command(&self.exe).args(["-s", &self.serial, "shell", "getprop", name]).output().context("adb didn't run")?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn shell(&self, cmd: &str) -> Result<std::process::Output> {
        command(&self.exe).args(["-s", &self.serial, "shell", cmd]).output().context("adb didn't run")
    }

    /// Root for reading partitions: adbd as root if it is or can be (`adb root`, which restarts adbd: r1os's image),
    /// else Magisk's `su`.
    pub fn root(&self) -> Result<Root> {
        let is_root = |o: std::process::Output| String::from_utf8_lossy(&o.stdout).trim() == "0";
        if is_root(self.shell("id -u")?) {
            return Ok(Root::Adbd);
        }
        let r = command(&self.exe).args(["-s", &self.serial, "root"]).output().context("adb didn't run")?;
        if r.status.success() && !String::from_utf8_lossy(&r.stdout).contains("cannot") {
            let _ = command(&self.exe).args(["-s", &self.serial, "wait-for-device"]).output();
            if is_root(self.shell("id -u")?) {
                return Ok(Root::Adbd);
            }
        }
        if is_root(self.shell("su -c 'id -u'")?) {
            return Ok(Root::Su);
        }
        bail!("this R1 has no root over adb: it needs r1os's image (adb root) or Magisk's su")
    }

    /// The first 34 sectors of the R1's eMMC: the protective MBR, the GPT's header and its entries. Read only.
    pub fn read_gpt(&self, root: Root, to: &Path) -> Result<u64> {
        self.read(root, "dd if=/dev/block/mmcblk0 bs=512 count=34 2>/dev/null", to, |_| {})
    }

    /// One partition, whole, into [to] (`dd` from `/dev/block/by-name`). [progress] hears the bytes so far. Read only.
    pub fn read_partition(&self, root: Root, name: &str, to: &Path, progress: impl FnMut(u64)) -> Result<u64> {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            bail!("not a partition's name");
        }
        self.read(root, &format!("dd if=/dev/block/by-name/{name} bs=4194304 2>/dev/null"), to, progress)
    }

    /// A partition's first [bytes] (whole 4 KiB blocks) into [to]: `super`'s geometry and LP metadata for the flash
    /// (ADR-0067). Read only.
    pub fn read_partition_head(&self, root: Root, name: &str, bytes: u64, to: &Path) -> Result<u64> {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            bail!("not a partition's name");
        }
        if bytes == 0 || !bytes.is_multiple_of(4096) {
            bail!("a head of whole 4 KiB blocks");
        }
        self.read(root, &format!("dd if=/dev/block/by-name/{name} bs=4096 count={} 2>/dev/null", bytes / 4096), to, |_| {})
    }

    fn read(&self, root: Root, cmd: &str, to: &Path, mut progress: impl FnMut(u64)) -> Result<u64> {
        let cmd = match root {
            Root::Adbd => cmd.to_owned(),
            Root::Su => format!("su -c '{cmd}'"),
        };
        let mut child = command(&self.exe)
            .args(["-s", &self.serial, "exec-out", &cmd])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("adb didn't run")?;
        let mut out = child.stdout.take().context("no output from adb")?;
        let mut file = std::io::BufWriter::with_capacity(4 << 20, std::fs::File::create(to)?);
        let mut buf = vec![0u8; 1 << 20];
        let mut got = 0u64;
        loop {
            let n = out.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            got += n as u64;
            progress(got);
        }
        file.flush()?;
        child.wait()?;
        Ok(got)
    }

    /// Into the bootloader's fastboot (`adb reboot bootloader`). Never fastbootd: a trip there and back once left a
    /// slot unbootable (T-182), and r1os's install doesn't use it (ADR-0067).
    pub fn reboot_to_bootloader(&self) -> Result<()> {
        let out = command(&self.exe)
            .args(["-s", &self.serial, "reboot", "bootloader"])
            .output()
            .context("adb didn't run")?;
        if !out.status.success() {
            bail!("adb reboot failed: {}", crate::fastboot::scrub(String::from_utf8_lossy(&out.stderr).trim(), &self.serial));
        }
        Ok(())
    }

    /// One of the first setup's commands, as root (T-170). [`crate::setup`] builds every one from fixed templates, and
    /// nothing else can make one. [line] hears its output line by line as it comes; the serial is scrubbed from it and
    /// from adb's own errors.
    pub fn setup_command(&self, root: Root, cmd: &crate::setup::Cmd, line: &mut dyn FnMut(&str)) -> Result<crate::setup::Output> {
        let text = match root {
            Root::Adbd => cmd.text().to_owned(),
            // Magisk's su takes the command in single quotes, so it can't have one.
            Root::Su if cmd.text().contains('\'') => bail!("a setup command with a single quote can't go through su"),
            Root::Su => format!("su -c '{}'", cmd.text()),
        };
        let mut child = command(&self.exe)
            .args(["-s", &self.serial, "shell", &text])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("adb didn't run")?;
        let mut err = child.stderr.take().context("no output from adb")?;
        let errs = std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = err.read_to_end(&mut s);
            String::from_utf8_lossy(&s).into_owned()
        });
        let mut out = String::new();
        let mut r = BufReader::new(child.stdout.take().context("no output from adb")?);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            if r.read_until(b'\n', &mut buf)? == 0 {
                break;
            }
            let l = crate::fastboot::scrub(String::from_utf8_lossy(&buf).trim_end_matches(['\n', '\r']), &self.serial);
            line(&l);
            out.push_str(&l);
            out.push('\n');
        }
        let status = child.wait()?;
        let err = errs.join().unwrap_or_default();
        Ok(crate::setup::Output { code: status.code().unwrap_or(-1), out, err: crate::fastboot::scrub(err.trim(), &self.serial) })
    }

    /// One of the first setup's pushes (T-170): a file, a folder's contents or one of the agent box's compiled-in
    /// scripts, into its place under [`crate::setup::STAGE`]. Only [`crate::setup`] can make one.
    pub fn setup_push(&self, p: &crate::setup::Push) -> Result<()> {
        use crate::setup::Source;
        if !p.to().starts_with(&format!("{}/", crate::setup::STAGE)) {
            bail!("the setup pushes only into {}", crate::setup::STAGE);
        }
        let mut written = None;
        let from: std::ffi::OsString = match p.source() {
            Source::File(f) => f.clone().into(),
            // The folder's contents, as `tools/deploy.sh` pushes them (`<folder>/.`).
            Source::Dir(d) => {
                let mut s = d.clone().into_os_string();
                s.push("/.");
                s
            }
            Source::Script { name, text } => {
                let dir = std::env::temp_dir().join(format!("r1os-setup-{}", std::process::id()));
                std::fs::create_dir_all(&dir)?;
                let f = dir.join(name);
                std::fs::write(&f, text)?;
                written = Some(f.clone());
                f.into()
            }
        };
        let out = command(&self.exe).args(["-s", &self.serial, "push"]).arg(&from).arg(p.to()).stdin(Stdio::null()).output();
        if let Some(f) = written {
            let _ = std::fs::remove_file(f);
        }
        let out = out.context("adb didn't run")?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr);
            bail!("adb push to {} failed: {}", p.to(), crate::fastboot::scrub(why.trim(), &self.serial));
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
