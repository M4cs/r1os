//! Backing up a stock R1 before anything is written (ADR-0065 decision 5, T-169).
//!
//! A stock R1 has no root, so its device-unique partitions (the IMEI and radio calibration among them) are read at
//! power-on with mtkclient (GPL-3.0: fetched on demand, never bundled), through the preloader. On Windows it talks to
//! the preloader over Rabbit's MediaTek serial port driver ([`crate::drivers`]); elsewhere over libusb. mtkclient runs
//! from its pinned source with only the five packages its command line needs (its package would pull in a GUI
//! toolkit), on a Python uv fetches.
//!
//! A backup goes to `Documents\r1os backups\<identity>\<date>\` with `SHA256SUMS` and `info.json`. Its identity is the
//! R1's own ([`crate::identity`]): the serial sits at the start of `proinfo` (seen in three backups, 2026-10-14),
//! so a backup made at power-on matches the R1 the scan finds booted or in fastboot. Reads only: mtkclient's `rl`.
//!
//! Skipping it takes typing [`SKIP_PHRASE`]: r1os's flash never writes these partitions; a backup is what gets them
//! back if something else ever does.

use crate::download::{self, Pin};
use anyhow::{Context, Result, bail};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The partitions read, as `tools/backup.sh` keeps them, by their names in the R1's GPT (2026-10-14; it has no
/// `persist` or `dpm`): the R1's own, which no image can give back (the IMEI, radio calibration, the unlock flag), then
/// the boot chain on both slots. Not super or userdata, and not `preloader_a`/`_b`: they're the eMMC boot areas
/// (`mmcblk0boot0`/`1`), outside the GPT, which mtkclient 2.1.4.1 sizes from the user area; they hold Rabbit's
/// preloader, the same on every R1, which r1os never writes.
pub const PARTS: &[&str] = &[
    "nvram", "nvdata", "nvcfg", "protect1", "protect2", "proinfo", "seccfg", "frp", "otp", "sec1", "md_udc", "para",
    "boot_para", "logo",
    "lk_a", "lk_b", "boot_a", "boot_b", "init_boot_a", "init_boot_b", "vendor_boot_a",
    "vendor_boot_b", "dtbo_a", "dtbo_b", "vbmeta_a", "vbmeta_b", "vbmeta_system_a", "vbmeta_system_b",
    "vbmeta_vendor_a", "vbmeta_vendor_b", "md1img_a", "md1img_b", "tee_a", "tee_b", "gz_a", "gz_b", "scp_a", "scp_b",
    "sspm_a", "sspm_b", "spmfw_a", "spmfw_b",
];

/// What has to be typed to go on without a backup.
pub const SKIP_PHRASE: &str = "I understand";

pub fn acknowledged(typed: &str) -> bool {
    typed.trim().eq_ignore_ascii_case(SKIP_PHRASE)
}

/// uv 0.10.9 for Windows (its SHA-256 as astral publishes it): runs mtkclient on a Python it fetches. It works under a
/// packaged app's redirected `AppData\Local` too (run from the Claude desktop app, 2026-10-14), where uv 0.13.1's check
/// of its minor-version junction fails: the junction names the real path, which the redirected files aren't at.
pub const UV: Pin = Pin {
    url: "https://github.com/astral-sh/uv/releases/download/0.10.9/uv-x86_64-pc-windows-msvc.zip",
    size: 22_310_473,
    sha256: "f58dc40896000229db7c52b8bdd931394040ef2ad59abd1eda841f6d70b13d7a",
};
/// mtkclient 2.1.4.1's source (GPL-3.0; the SHA-256 is the archive's as fetched 2026-10-14).
pub const MTKCLIENT: Pin = Pin {
    url: "https://github.com/bkerler/mtkclient/archive/refs/tags/v2.1.4.1.zip",
    size: 32_229_485,
    sha256: "2971f4074e99667af72d90f4ae53633fc1aa93179bd185e387689449f237cd7a",
};
pub const MTKCLIENT_DIR: &str = "mtkclient-2.1.4.1";
/// uv's folder, by its version, so a new pin is fetched again.
pub const UV_DIR: &str = "uv-0.10.9";
/// UsbDk 1.0.22 (64-bit), signed by Red Hat (the SHA-256 is the MSI's as fetched 2026-10-14; GitHub gives none).
pub const USBDK: Pin = Pin {
    url: "https://github.com/daynix/UsbDk/releases/download/v1.00-22/UsbDk_1.0.22_x64.msi",
    size: 6_348_800,
    sha256: "91f6f695e1e13c656024e6d3b55620bf08d8835ef05ee0496935ba6bb62466a5",
};
/// The command line's packages, pinned (pycryptodome stays on 3.x, with pycryptodomex).
pub const PACKAGES: &[&str] = &["pyusb==1.3.1", "pycryptodome==3.24.1", "pycryptodomex==3.24.1", "colorama==0.4.6", "pyserial==3.5"];
pub const PYTHON: &str = "3.12";

/// Where uv and mtkclient are, once fetched.
#[derive(Debug, Clone)]
pub struct Tools {
    pub uv: PathBuf,
    pub mtkclient: PathBuf,
    /// uv's own cache and Pythons, kept beside them (nothing in the user's own uv setup is touched).
    pub home: PathBuf,
}

/// The tools in [dir], if they're there.
pub fn find(dir: &Path) -> Option<Tools> {
    let t = Tools { uv: dir.join(UV_DIR).join("uv.exe"), mtkclient: dir.join(MTKCLIENT_DIR), home: dir.to_path_buf() };
    (t.uv.is_file() && t.mtkclient.join("mtk.py").is_file()).then_some(t)
}

/// Fetches uv and mtkclient into [dir], checked against their pins.
pub fn fetch(dir: &Path, mut progress: impl FnMut(&str, u64, u64)) -> Result<Tools> {
    let uv_zip = download::fetch(&UV, &dir.join("uv.zip"), |g, t| progress("uv", g, t))?;
    download::unzip(&uv_zip, &dir.join(UV_DIR), Some(&["uv.exe"]))?;
    let _ = std::fs::remove_file(uv_zip);
    let src = download::fetch(&MTKCLIENT, &dir.join("mtkclient.zip"), |g, t| progress("mtkclient", g, t))?;
    let _ = std::fs::remove_dir_all(dir.join(MTKCLIENT_DIR));
    download::unzip(&src, dir, None)?;
    let _ = std::fs::remove_file(src);
    find(dir).context("uv and mtkclient unpacked but not found")
}

/// Fetches UsbDk's installer (run in [`crate::drivers::install`]'s elevated step).
pub fn fetch_usbdk(dir: &Path) -> Result<PathBuf> {
    download::fetch(&USBDK, &dir.join("UsbDk_1.0.22_x64.msi"), |_, _| {})
}

/// Whether UsbDk's driver is installed (its service exists).
pub fn usbdk_installed() -> bool {
    Command::new("sc").args(["query", "UsbDk"]).output().map(|o| o.status.success()).unwrap_or(false)
}

/// Where backups go: `Documents\r1os backups`, wherever Windows keeps Documents.
pub fn default_root() -> PathBuf {
    crate::folders::documents().join("r1os backups")
}

/// Where uv, mtkclient and uv's Python go: `%LOCALAPPDATA%\r1os\companion\backup-tools`.
pub fn tools_dir() -> PathBuf {
    crate::folders::companion().join("backup-tools")
}

/// Now, in UTC, as a backup's folder name: `20261014-153000`.
pub fn stamp() -> String {
    stamp_of(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

fn stamp_of(unix: u64) -> String {
    let (days, secs) = ((unix / 86_400) as i64, unix % 86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// A backup, made or found.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Backup {
    pub dir: PathBuf,
    /// The R1's identity, from the serial in its `proinfo`.
    pub identity: String,
    pub date: String,
    pub parts: Vec<(String, u64)>,
}

/// What runs mtkclient's `mtk.py` (its pinned source left as it is), with fixes for its serial port on Windows (a test R1,
/// 2026-10-14):
/// - mtkclient 2.1.4.1 asks for USB endpoints' packet sizes (xflash's stage 2 upload, its reads), which its
///   `SerialClass` doesn't have (`AttributeError … EP_OUT`). They get USB 2.0's bulk size, 512, as its serial `write`
///   already uses.
/// - Speed: the first backup read about 1 MB/s on a 480 Mbit/s link, and pyserial gives the port a 4 KB receive
///   queue. Here it's 8 MB, set once after pyserial's own setup.
///   - Not tried again: setting only the timeouts between reads, in place of pyserial's whole setup (`SetCommState`).
///     With it, the DA's first reply after stage 1's jump never came (`xread error: unpack requires a buffer of 12
///     bytes`, 2026-10-14). The whole setup also re-sends DTR/RTS, which the DA may wait for (**unverified**).
/// - It says the DA's packet sizes, which bound how much is read between acknowledgements.
pub const RUNNER: &str = r#"# r1os (crates/r1-usb/src/backup.rs): runs mtkclient's mtk.py with fixes for its serial port.
import os, runpy, sys, types
mtk = sys.argv[1]
sys.path.insert(0, os.path.dirname(mtk))
from mtkclient.Library.Connection import seriallib
seriallib.SerialClass.EP_IN = seriallib.SerialClass.EP_OUT = types.SimpleNamespace(wMaxPacketSize=512)

if sys.platform == "win32":
    from serial import serialwin32, win32
    _setup = serialwin32.Serial._reconfigure_port

    def _reconfigure_port(self):
        _setup(self)
        if not getattr(self, "_r1os_queue", False):
            self._r1os_queue = True
            try:
                win32.SetupComm(self._port_handle, 8 << 20, 1 << 20)
            except Exception:
                pass

    serialwin32.Serial._reconfigure_port = _reconfigure_port

from mtkclient.Library.DA.xflash import xflash_lib
_plen = xflash_lib.DAXFlash.get_packet_length

def get_packet_length(self):
    p = _plen(self)
    if p:
        print(f"r1os - DA packet length: read {p.read_packet_length:#x}, write {p.write_packet_length:#x}", flush=True)
    return p

xflash_lib.DAXFlash.get_packet_length = get_packet_length
sys.argv = [mtk] + sys.argv[2:]
runpy.run_path(mtk, run_name="__main__")
"#;

/// mtkclient through uv and [RUNNER], with [args] after `mtk.py`: uv's cache and Pythons kept in the tools' folder.
fn mtk(tools: &Tools, args: &[&std::ffi::OsStr]) -> Result<Command> {
    let runner = tools.home.join("r1os_mtk.py");
    std::fs::write(&runner, RUNNER)?;
    let mut cmd = Command::new(&tools.uv);
    cmd.args(["run", "--no-project", "--python", PYTHON]);
    for p in PACKAGES {
        cmd.args(["--with", p]);
    }
    cmd.arg("python")
        .arg(runner)
        .arg(tools.mtkclient.join("mtk.py"))
        .args(args)
        .current_dir(&tools.mtkclient)
        .env("UV_CACHE_DIR", tools.home.join("uv-cache"))
        .env("UV_PYTHON_INSTALL_DIR", tools.home.join("uv-python"))
        .env("UV_NO_CONFIG", "1")
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // No window of its own when there's no console (the companion); in a terminal it shares the console, so Ctrl+C
    // reaches it rather than leaving it holding the R1's port.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // SAFETY: no arguments; it only says whether this process has a console.
        if unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null() {
            cmd.creation_flags(0x0800_0000);
        }
    }
    Ok(cmd)
}

/// Gets Python and mtkclient's packages in place (a first time, ~40 MB) by asking mtkclient for its help, so the
/// backup itself starts at once when the R1 is plugged in.
pub fn prepare(tools: &Tools) -> Result<()> {
    let out = mtk(tools, &["r".as_ref(), "--help".as_ref()])?.output().context("uv didn't start")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("mtkclient isn't ready ({}): {}", out.status, err.lines().last().unwrap_or(""));
    }
    Ok(())
}

/// Stops a running backup from another thread (the wizard's Cancel while it waits for the R1).
#[derive(Debug, Default, Clone)]
pub struct Stop(std::sync::Arc<std::sync::atomic::AtomicU32>);

impl Stop {
    /// Ends mtkclient and uv's Python under it, if a backup is running.
    pub fn stop(&self) {
        let pid = self.0.swap(0, std::sync::atomic::Ordering::SeqCst);
        if pid == 0 {
            return;
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).output();
        }
        #[cfg(not(windows))]
        let _ = Command::new("kill").arg(pid.to_string()).output();
    }
}

/// Reads [PARTS] over the boot ROM into [root] (`Documents\r1os backups`): the R1 powered off, then plugged in while
/// this waits. [progress] hears mtkclient's lines, with anything that names the chip taken out. A backup that doesn't
/// finish leaves nothing behind (what it read holds the IMEI).
pub fn run(tools: &Tools, root: &Path, date: &str, stop: &Stop, progress: impl FnMut(&str)) -> Result<Backup> {
    let pending = root.join(format!(".pending-{date}"));
    let _ = std::fs::remove_dir_all(&pending);
    std::fs::create_dir_all(&pending)?;
    let made = read(tools, &pending, stop, progress).and_then(|()| finish(root, &pending, date));
    if made.is_err() {
        let _ = std::fs::remove_dir_all(&pending);
    }
    made
}

/// Reads [PARTS] from a booted R1 over adb as root, as `tools/backup.sh` does: at USB speed, where [run]'s route
/// through the preloader's serial port managed about 1 MB/s (2026-10-14). For an R1 with root (r1os's image, or
/// Magisk); a stock R1 has none, so it takes [run]. The same folder, checks and sums as [run]. [progress] hears the
/// partition, the bytes read so far and the total.
pub fn run_adb(adb: &crate::adb::Adb, root: &Path, date: &str, mut progress: impl FnMut(&str, u64, u64)) -> Result<Backup> {
    let pending = root.join(format!(".pending-{date}"));
    let _ = std::fs::remove_dir_all(&pending);
    std::fs::create_dir_all(&pending)?;
    let made = (|| {
        let r = adb.root()?;
        adb.read_gpt(r, &pending.join("gpt.bin"))?;
        let gpt = gpt_sizes(&std::fs::read(pending.join("gpt.bin"))?).context("the GPT didn't read right")?;
        let total: u64 = PARTS.iter().filter_map(|p| gpt.iter().find(|(n, _)| n == p).map(|(_, s)| *s)).sum();
        let mut done = 0;
        for p in PARTS {
            done += adb.read_partition(r, p, &pending.join(format!("{p}.bin")), |n| progress(p, done + n, total))?;
        }
        finish(root, &pending, date)
    })();
    if made.is_err() {
        let _ = std::fs::remove_dir_all(&pending);
    }
    made
}

/// The R1's GPT partitions not read: the OS and the user's data (downloadable or disposable), and what goes with them.
/// With [PARTS], every name in the R1's GPT (2026-10-14).
pub const NOT_READ: &[&str] = &["super", "userdata", "metadata", "expdb", "flashinfo"];

/// mtkclient's arguments after `mtk.py`: `rl` reads the GPT and then each partition at its own size, into [pending] as
/// `gpt.bin` and `<name>.bin` (not `r` with several partitions: 2.1.4.1 reads them all at the first one's size, T-181).
/// On Windows it talks to the preloader through Rabbit's MediaTek serial port driver (`--serialport`, as r1_escape
/// does): mtkclient's USB route, through UsbDk, missed the preloader's moment at power-on on a test R1 (2026-10-14).
/// `--serialport` goes last: its value is optional, and it would take a path after it as the port's name.
pub fn args(pending: &Path) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = vec!["rl".into(), pending.into(), "--skip".into(), NOT_READ.join(",").into()];
    if cfg!(windows) {
        a.push("--serialport".into());
    }
    a
}

fn read(tools: &Tools, pending: &Path, stop: &Stop, mut progress: impl FnMut(&str)) -> Result<()> {
    let args = args(pending);
    let mut child = mtk(tools, &args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>())?.spawn().context("mtkclient didn't start")?;
    stop.0.store(child.id(), std::sync::atomic::Ordering::SeqCst);
    let err = child.stderr.take();
    let errs = std::thread::spawn(move || {
        let mut lines = Vec::new();
        if let Some(e) = err {
            each_line(e, |l| lines.push(scrub(l)));
        }
        lines
    });
    if let Some(out) = child.stdout.take() {
        each_line(out, |l| progress(&scrub(l)));
    }
    let status = child.wait()?;
    // Swapped to 0 already if it was stopped.
    let stopped = stop.0.swap(0, std::sync::atomic::Ordering::SeqCst) == 0;
    let errs = errs.join().unwrap_or_default();
    for l in &errs {
        progress(l);
    }
    if stopped {
        bail!("the backup was stopped");
    }
    if !status.success() {
        bail!("mtkclient stopped ({status}): {}", errs.last().map(String::as_str).unwrap_or(""));
    }
    Ok(())
}

/// [r]'s output a line at a time, split at `\r` too (mtkclient's progress redraws one line).
fn each_line(r: impl std::io::Read, mut f: impl FnMut(&str)) {
    let mut r = BufReader::new(r);
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while matches!(r.read(&mut byte), Ok(1)) {
        if byte[0] == b'\n' || byte[0] == b'\r' {
            if !line.is_empty() {
                f(String::from_utf8_lossy(&line).trim_end());
                line.clear();
            }
        } else {
            line.push(byte[0]);
        }
    }
    if !line.is_empty() {
        f(String::from_utf8_lossy(&line).trim_end());
    }
}

/// The partitions in a GPT, with their sizes in bytes (512-byte sectors). [gpt] starts at LBA 0, as mtkclient's
/// `gpt.bin` does (the protective MBR, the header, then the entries as far as it read: 124 of them), or at the header.
/// Entries past its end are left out; [finish] names any partition it then can't find.
pub fn gpt_sizes(gpt: &[u8]) -> Option<Vec<(String, u64)>> {
    const SECTOR: usize = 512;
    let le64 = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let le32 = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    // Where LBA 0 is in [gpt]: the header is LBA 1.
    let lba0: i64 = if gpt.get(SECTOR..SECTOR + 8) == Some(b"EFI PART".as_slice()) {
        0
    } else if gpt.get(..8) == Some(b"EFI PART".as_slice()) {
        -(SECTOR as i64)
    } else {
        return None;
    };
    let h = gpt.get((lba0 + SECTOR as i64) as usize..)?.get(..92)?;
    let (n, size) = (le32(h, 80), le32(h, 84));
    if size < 128 || n > 1024 {
        return None;
    }
    let start = lba0 + i64::try_from(le64(h, 72)).ok()?.checked_mul(SECTOR as i64)?;
    let mut out = Vec::new();
    for i in 0..n as i64 {
        let from = start + i * size as i64;
        let Some(e) = usize::try_from(from).ok().and_then(|f| gpt.get(f..f + size)) else { break };
        let (first, last) = (le64(e, 32), le64(e, 40));
        if first == 0 || last < first {
            continue;
        }
        let name: Vec<u16> = e[56..128].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        out.push((String::from_utf16_lossy(&name), (last - first + 1) * SECTOR as u64));
    }
    (!out.is_empty()).then_some(out)
}

/// Checks every partition came back whole (its size as the R1's GPT has it), names the backup by the R1 in its
/// `proinfo`, writes `SHA256SUMS` and `info.json`, and moves it into place.
fn finish(root: &Path, pending: &Path, date: &str) -> Result<Backup> {
    let gpt = gpt_sizes(&std::fs::read(pending.join("gpt.bin")).context("the GPT wasn't read")?).context("the GPT didn't read right")?;
    let mut parts = Vec::new();
    for p in PARTS {
        let want = gpt.iter().find(|(n, _)| n.eq_ignore_ascii_case(p)).map(|(_, s)| *s).with_context(|| format!("this R1 has no {p}"))?;
        let size = std::fs::metadata(pending.join(format!("{p}.bin"))).with_context(|| format!("{p} wasn't read"))?.len();
        if size != want {
            bail!("{p} came back as {size} bytes, not {want}");
        }
        parts.push((p.to_string(), size));
    }
    // mtkclient's session state and its second copy of the GPT's sectors: not part of the backup.
    let _ = std::fs::remove_file(pending.join(".state"));
    let _ = std::fs::remove_file(pending.join("gpt_backup.bin"));
    let identity = identity_of_proinfo(&std::fs::read(pending.join("proinfo.bin"))?).context("no serial at the start of proinfo")?;
    let mut sums = format!("{}  gpt.bin\n", download::sha256_of(&pending.join("gpt.bin"))?);
    for (p, _) in &parts {
        sums.push_str(&format!("{}  {p}.bin\n", download::sha256_of(&pending.join(format!("{p}.bin")))?));
    }
    std::fs::write(pending.join("SHA256SUMS"), sums)?;
    let b = Backup { dir: root.join(&identity).join(date), identity, date: date.to_owned(), parts };
    let info = serde_json::json!({
        "identity": b.identity, "date": b.date,
        "parts": b.parts.iter().map(|(p, s)| serde_json::json!({ "name": p, "bytes": s })).collect::<Vec<_>>(),
        "made_with": format!("mtkclient {} over the boot ROM", MTKCLIENT_DIR.trim_start_matches("mtkclient-")),
    });
    std::fs::write(pending.join("info.json"), serde_json::to_string_pretty(&info)?)?;
    std::fs::create_dir_all(b.dir.parent().unwrap())?;
    std::fs::rename(pending, &b.dir)?;
    Ok(b)
}

/// The R1's identity from its `proinfo`: the serial at its start (letters and digits).
pub fn identity_of_proinfo(proinfo: &[u8]) -> Option<String> {
    let serial: String = proinfo.iter().take(64).take_while(|b| b.is_ascii_alphanumeric()).map(|&b| b as char).collect();
    (serial.len() >= 8).then(|| crate::identity(&serial))
}

/// Checks a backup: every file's SHA-256 against `SHA256SUMS`, and its `proinfo` naming [identity]'s R1.
pub fn verify(dir: &Path, identity: &str) -> Result<()> {
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).context("no SHA256SUMS")?;
    let mut n = 0;
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        // `<sum>  <file>`, or `<sum> *<file>` (binary mode, as tools/backup.sh writes it).
        let (want, rest) = line.split_once(' ').context("SHA256SUMS isn't `<sum>  <file>`")?;
        let name = rest.strip_prefix(['*', ' ']).unwrap_or(rest);
        if name.contains(['/', '\\']) || name.contains("..") {
            bail!("SHA256SUMS names a file outside its folder");
        }
        if download::sha256_of(&dir.join(name))? != want {
            bail!("{name} has changed since the backup");
        }
        n += 1;
    }
    if n == 0 {
        bail!("SHA256SUMS is empty");
    }
    let proinfo = ["proinfo.bin", "proinfo.img"].iter().map(|n| dir.join(n)).find(|p| p.is_file()).context("no proinfo in it")?;
    if identity_of_proinfo(&std::fs::read(proinfo)?).as_deref() != Some(identity) {
        bail!("it's another R1's backup");
    }
    Ok(())
}

/// The newest backup of [identity]'s R1 under [root] that verifies.
pub fn latest(root: &Path, identity: &str) -> Option<PathBuf> {
    let mut dates: Vec<PathBuf> = std::fs::read_dir(root.join(identity)).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    dates.sort();
    dates.into_iter().rev().find(|d| verify(d, identity).is_ok())
}

/// mtkclient's line without the chip's own ids (ME_ID, SOC_ID, the eMMC's CID, a serial, the IMEI) or a long hex run.
/// A line is hidden when its label (before the first `:`, or the whole line without one) has one of those as a word,
/// so an error about mtkclient's `SerialClass` still reads.
pub fn scrub(line: &str) -> String {
    const NAMES: &[&str] = &["me_id", "meid", "soc_id", "socid", "cid", "serial", "serialno", "imei", "hrid"];
    let label = line.split_once(':').map_or(line, |(k, _)| k).to_ascii_lowercase();
    if label.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|w| NAMES.contains(&w)) {
        return match line.split_once(':') {
            Some((k, _)) => format!("{k}: <hidden>"),
            None => "<a line naming the device, hidden>".to_owned(),
        };
    }
    let mut out = String::with_capacity(line.len());
    let mut run = String::new();
    for c in line.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_hexdigit() {
            run.push(c);
            continue;
        }
        if run.len() >= 16 { out.push_str("<hex>") } else { out.push_str(&run) }
        run.clear();
        out.push(c);
    }
    out.pop();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_from_proinfo() {
        let mut p = b"0123456789ABCDEFGHIJ".to_vec();
        p.extend_from_slice(&[0u8; 100]);
        assert_eq!(identity_of_proinfo(&p).as_deref(), Some(crate::identity("0123456789ABCDEFGHIJ").as_str()));
        assert!(identity_of_proinfo(&[0u8; 64]).is_none());
        assert!(identity_of_proinfo(b"ab\0").is_none());
    }

    #[test]
    fn stamps() {
        assert_eq!(stamp_of(0), "19700101-000000");
        assert_eq!(stamp_of(951_782_400), "20000229-000000");
        assert_eq!(stamp_of(1_700_000_000), "20231114-221320");
    }

    #[test]
    fn skipping_takes_the_words() {
        assert!(acknowledged("i understand ") && acknowledged("I UNDERSTAND"));
        assert!(!acknowledged("") && !acknowledged("yes") && !acknowledged("I understand it"));
    }

    #[test]
    fn mtkclient_s_lines_name_nothing() {
        assert_eq!(scrub("ME_ID: 0123456789ABCDEF0123456789ABCDEF"), "ME_ID: <hidden>");
        assert_eq!(scrub("\tSOC_ID:\t\tAABB"), "\tSOC_ID: <hidden>");
        assert_eq!(scrub("Dumping nvram 0x00 / 0x1000000 at 0123456789abcdef0123"), "Dumping nvram 0x00 / 0x1000000 at <hex>");
        assert_eq!(scrub("Read proinfo: done"), "Read proinfo: done");
        assert_eq!(scrub("DAXFlash - EMMC CID: 1501004d434732"), "DAXFlash - EMMC CID: <hidden>");
        assert_eq!(scrub("Serial number 0123ABCD"), "<a line naming the device, hidden>");
        // An error about mtkclient's own classes still reads.
        let e = "AttributeError: 'SerialClass' object has no attribute 'EP_OUT'";
        assert_eq!(scrub(e), e);
    }

    /// A GPT's first sectors with [parts] (name, sectors), one after another from sector 2048.
    fn gpt_of(parts: &[(&str, u64)]) -> Vec<u8> {
        let mut g = vec![0u8; 0x16000];
        g[512..520].copy_from_slice(b"EFI PART");
        g[512 + 72..512 + 80].copy_from_slice(&2u64.to_le_bytes());
        g[512 + 80..512 + 84].copy_from_slice(&(parts.len() as u32).to_le_bytes());
        g[512 + 84..512 + 88].copy_from_slice(&128u32.to_le_bytes());
        let mut lba = 2048u64;
        for (i, (name, sectors)) in parts.iter().enumerate() {
            let e = 1024 + i * 128;
            g[e + 32..e + 40].copy_from_slice(&lba.to_le_bytes());
            g[e + 40..e + 48].copy_from_slice(&(lba + sectors - 1).to_le_bytes());
            for (j, c) in name.encode_utf16().enumerate() {
                g[e + 56 + j * 2..e + 58 + j * 2].copy_from_slice(&c.to_le_bytes());
            }
            lba += sectors;
        }
        g
    }

    #[test]
    fn the_gpt_s_sizes() {
        let g = gpt_of(&[("nvram", 8), ("boot_a", 65536)]);
        assert_eq!(gpt_sizes(&g), Some(vec![("nvram".to_owned(), 4096), ("boot_a".to_owned(), 32 << 20)]));
        assert_eq!(gpt_sizes(&[0u8; 1024]), None);
        // From the header, as well as from LBA 0.
        assert_eq!(gpt_sizes(&g[512..]), gpt_sizes(&g));
        // Cut short: the entries it has (finish names any missing), none if it has none.
        assert_eq!(gpt_sizes(&g[..1024 + 128]), Some(vec![("nvram".to_owned(), 4096)]));
        assert_eq!(gpt_sizes(&g[..1100]), None);
    }

    #[test]
    fn mtkclient_reads_the_list_and_never_the_os_or_data() {
        let a = args(Path::new(r"C:\Sam Example\r1os backups\.pending-x"));
        assert_eq!(a[0], "rl");
        assert_eq!(a[1], r"C:\Sam Example\r1os backups\.pending-x");
        assert_eq!(a[2], "--skip");
        let skip = a[3].to_string_lossy();
        assert!(["super", "userdata", "metadata"].iter().all(|p| skip.split(',').any(|s| s == *p)));
        // Nothing both read and skipped; `--serialport` last, so it never takes a path for its port.
        assert!(PARTS.iter().all(|p| !NOT_READ.contains(p)));
        assert_eq!(a.last().unwrap() == "--serialport", cfg!(windows));
    }

    #[test]
    fn a_backup_verified_as_this_r1_s() {
        let root = std::env::temp_dir().join(format!("r1-usb-backup-{}", std::process::id()));
        let pending = root.join(".pending-x");
        std::fs::create_dir_all(&pending).unwrap();
        let gpt: Vec<(&str, u64)> = PARTS.iter().map(|p| (*p, 1)).collect();
        std::fs::write(pending.join("gpt.bin"), gpt_of(&gpt)).unwrap();
        std::fs::write(pending.join("gpt_backup.bin"), gpt_of(&gpt)).unwrap();
        for p in PARTS {
            let mut body = if *p == "proinfo" { b"0123456789ABCDEFGHIJ\0\0".to_vec() } else { format!("{p} bytes").into_bytes() };
            body.resize(512, 0);
            std::fs::write(pending.join(format!("{p}.bin")), body).unwrap();
        }
        // One partition short: refused, and named.
        std::fs::write(pending.join("nvram.bin"), [0u8; 100]).unwrap();
        assert!(format!("{:#}", finish(&root, &pending, "x").unwrap_err()).contains("nvram came back as 100 bytes"));
        std::fs::write(pending.join("nvram.bin"), [0u8; 512]).unwrap();
        std::fs::write(pending.join(".state"), b"{}").unwrap();
        let b = finish(&root, &pending, "20261014-120000").unwrap();
        assert!(!b.dir.join(".state").exists() && !b.dir.join("gpt_backup.bin").exists() && b.dir.join("gpt.bin").exists());
        let id = crate::identity("0123456789ABCDEFGHIJ");
        assert_eq!(b.identity, id);
        assert!(verify(&b.dir, &id).is_ok());
        assert!(verify(&b.dir, "000000000000").is_err());
        assert_eq!(latest(&root, &id), Some(b.dir.clone()));
        // tools/backup.sh's binary-mode sums read the same.
        let sums = std::fs::read_to_string(b.dir.join("SHA256SUMS")).unwrap().replace("  ", " *");
        std::fs::write(b.dir.join("SHA256SUMS"), sums).unwrap();
        assert!(verify(&b.dir, &id).is_ok());
        // A changed file fails it.
        std::fs::write(b.dir.join("nvram.bin"), b"tampered").unwrap();
        assert!(verify(&b.dir, &id).is_err());
        assert_eq!(latest(&root, &id), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
