//! Windows drivers for the R1 (ADR-0065 decision 4, T-168), behind one administrator prompt.
//!
//! - **The R1 itself** (`USB\VID_0E8D&PID_201C`: its bootloader's fastboot and, booted, adb): bound to Windows' own
//!   WinUSB with Android's device interface GUID, which `fastboot` and `adb` look for. Booted, Windows finds WinUSB by
//!   itself; the bootloader offers it nothing to match, so it shows as "Android" with code 28, and a driver picked by
//!   hand is lost after every Android boot (the same device to Windows). An INF that names the R1's id binds it in
//!   both, every time. It's made, catalogued and signed on this PC, as Zadig and libwdi do: a fresh self-signed
//!   certificate, only its public half trusted (the machine's Root and TrustedPublisher stores), its private key
//!   deleted once the catalog's signed, so nothing else can ever be signed with it.
//! - **The MediaTek preloader's serial port** (`0E8D:2000`, the second at power-on when a stock R1 can be asked into
//!   fastboot): Rabbit's own installer, from rabbit-hmi-oss/firmware's `drivers` release, pinned by SHA-256 and run
//!   silently. The installer isn't code-signed; the drivers it installs (`cdc-acm.inf`, `usb2ser_win764.inf`) are
//!   WHQL-signed by Microsoft.
//!
//! Both come back out with [`undo_script`] (the R1's INF and its certificate; MediaTek's from Windows' apps list).

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// The INF's file name, which is how it's found again in the driver store.
pub const INF_NAME: &str = "r1os_r1_usb.inf";
/// The certificate's subject: how the undo finds it.
pub const CERT_SUBJECT: &str = "CN=r1os R1 USB driver (made on this PC)";
/// Android's USB device interface GUID (`ANDROID_USB_CLASS_ID`), which adb and fastboot enumerate.
pub const ANDROID_INTERFACE_GUID: &str = "{F72FE0D4-CBCB-407d-8814-9ED673D0DD6B}";

/// The R1's INF: WinUSB, from Windows' own `winusb.inf`, with Android's interface GUID.
pub fn inf() -> String {
    format!(
        r#"; r1os: the Rabbit R1 (its bootloader's fastboot, and adb booted) on Windows' own WinUSB driver (ADR-0065, T-168).
; Made and signed on this PC by the r1os Companion; the signing certificate's private key was deleted after signing.

[Version]
Signature   = "$Windows NT$"
Class       = USBDevice
ClassGuid   = {{88BAE032-5A81-49f0-BC3D-A4FF138216D6}}
Provider    = %Provider%
CatalogFile = r1os_r1_usb.cat
DriverVer   = 10/14/2026,1.0.0.0
PnpLockdown = 1

[Manufacturer]
%Provider% = R1, NTamd64, NTarm64, NTx86

[R1.NTamd64]
%R1% = R1_Install, USB\VID_0E8D&PID_201C

[R1.NTarm64]
%R1% = R1_Install, USB\VID_0E8D&PID_201C

[R1.NTx86]
%R1% = R1_Install, USB\VID_0E8D&PID_201C

[R1_Install]
Include = winusb.inf
Needs   = WINUSB.NT

[R1_Install.Services]
Include = winusb.inf
Needs   = WINUSB.NT.Services

[R1_Install.HW]
AddReg = R1_AddReg

[R1_AddReg]
HKR,,DeviceInterfaceGUIDs,0x10000,"{guid}"

[Strings]
Provider = "r1os"
R1       = "Rabbit R1 (r1os: fastboot and adb)"
"#,
        guid = ANDROID_INTERFACE_GUID
    )
}

/// Rabbit's MediaTek preloader driver installer, pinned (2026-10-14: the release's only asset; GitHub gives no digest,
/// so the SHA-256 is the one this file had then).
pub struct Mtk;

impl Mtk {
    pub const URL: &'static str =
        "https://github.com/rabbit-hmi-oss/firmware/releases/download/drivers/MediaTek-Preloader_USB_VCOM_drivers-signed.zip";
    pub const SIZE: u64 = 9_279_246;
    pub const SHA256: &'static str = "c83f7ba3e657abd3577640448009178d98c07a849f49bcdf5f59577158ea7dab";
    pub const EXE: &'static str = "MediaTek_Preloader_USB_VCOM_drivers.exe";
    /// The INFs it puts in the driver store, by their original names.
    pub const INFS: &'static [&'static str] = &["cdc-acm.inf", "usb2ser_win764.inf"];
}

/// Fetches Rabbit's MediaTek installer into [dir], checked against its pin, and returns the installer's path.
pub fn fetch_mtk(dir: &Path) -> Result<PathBuf> {
    let pin = crate::download::Pin { url: Mtk::URL, size: Mtk::SIZE, sha256: Mtk::SHA256 };
    let zip_path = crate::download::fetch(&pin, &dir.join("mtk-vcom.zip"), |_, _| {})?;
    let n = crate::download::unzip(&zip_path, dir, Some(&[Mtk::EXE]))?;
    let _ = std::fs::remove_file(&zip_path);
    if n != 1 {
        bail!("the installer isn't in Rabbit's zip");
    }
    Ok(dir.join(Mtk::EXE))
}

/// What else the elevated step installs, beside the R1's own driver.
#[derive(Debug, Clone, Copy, Default)]
pub struct Extras<'a> {
    /// Rabbit's MediaTek preloader driver installer ([`fetch_mtk`]).
    pub mtk_exe: Option<&'a Path>,
    /// UsbDk's MSI, for a stock R1's backup ([`crate::backup::fetch_usbdk`], T-169).
    pub usbdk_msi: Option<&'a Path>,
}

/// What's installed, from the driver store (`pnputil /enum-drivers`), matched by the INFs' original names, which
/// aren't translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// The R1's own (this module's INF).
    pub r1: bool,
    /// MediaTek's preloader serial port.
    pub mtk: bool,
}

pub fn status() -> Result<Status> {
    let out = std::process::Command::new("pnputil").arg("/enum-drivers").output().context("pnputil didn't run")?;
    Ok(status_of(&String::from_utf8_lossy(&out.stdout)))
}

fn status_of(store: &str) -> Status {
    let names: Vec<String> = store.lines().filter_map(|l| l.split_once(':').map(|(_, v)| v.trim().to_ascii_lowercase())).collect();
    Status { r1: names.iter().any(|n| n == INF_NAME), mtk: Mtk::INFS.iter().all(|i| names.iter().any(|n| n == i)) }
}

/// The elevated install, as a PowerShell script: Rabbit's installer and UsbDk's (if given), then the R1's INF
/// catalogued, signed with a certificate made for it, its public half trusted, its private key deleted, and the driver
/// added and installed (given [package]: none when it's installed already, as each install makes a certificate).
/// Writes what it did to [log].
pub fn install_script(package: Option<&Path>, extras: Extras<'_>, log: &Path) -> String {
    let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
    let mut mtk = extras.mtk_exe.map(|e| format!(
        "  $p = Start-Process -FilePath '{}' -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -Wait -PassThru\n  \"mediatek: installer exited $($p.ExitCode)\"\n",
        q(e)
    )).unwrap_or_default();
    if let Some(msi) = extras.usbdk_msi {
        mtk.push_str(&format!(
            "  $p = Start-Process -FilePath 'msiexec.exe' -ArgumentList '/i','\"{}\"','/qn','/norestart' -Wait -PassThru\n  \"usbdk: msiexec exited $($p.ExitCode)\"\n  if ($p.ExitCode -ne 0 -and $p.ExitCode -ne 3010) {{ exit 3 }}\n",
            q(msi)
        ));
    }
    let Some(package) = package else {
        return format!(
            r#"$ErrorActionPreference = 'Stop'
Start-Transcript -Path '{log}' | Out-Null
try {{
{mtk}  "the R1's driver is installed already"
}} catch {{
  "error: $_"
  exit 1
}} finally {{
  Stop-Transcript | Out-Null
}}
"#,
            log = q(log)
        );
    };
    format!(
        r#"$ErrorActionPreference = 'Stop'
Start-Transcript -Path '{log}' | Out-Null
try {{
{mtk}  $dir = '{pkg}'
  $inf = Join-Path $dir '{inf}'
  $cat = Join-Path $dir 'r1os_r1_usb.cat'
  New-FileCatalog -Path $dir -CatalogFilePath $cat -CatalogVersion 2.0 | Out-Null
  $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject '{subject}' -CertStoreLocation Cert:\CurrentUser\My -KeyExportPolicy NonExportable -NotAfter (Get-Date).AddYears(30)
  # Its public half trusted first, so the signature checks out as it's made (signed before, it read UnknownError).
  $pub = Join-Path $env:TEMP 'r1os_r1_usb.cer'
  Export-Certificate -Cert $cert -FilePath $pub | Out-Null
  Import-Certificate -FilePath $pub -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
  Import-Certificate -FilePath $pub -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
  Remove-Item $pub
  $sig = Set-AuthenticodeSignature -FilePath $cat -Certificate $cert -HashAlgorithm SHA256
  "catalog signed: $($sig.Status)"
  Remove-Item -Path ("Cert:\CurrentUser\My\" + $cert.Thumbprint) -DeleteKey
  "certificate trusted, its private key deleted"
  pnputil /add-driver $inf /install
  "pnputil exited $LASTEXITCODE"
  if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 3010 -and $LASTEXITCODE -ne 259) {{ exit 2 }}
}} catch {{
  "error: $_"
  exit 1
}} finally {{
  Stop-Transcript | Out-Null
}}
"#,
        log = q(log),
        pkg = q(package),
        inf = INF_NAME,
        subject = CERT_SUBJECT,
    )
}

/// The elevated undo: the R1's INF out of the driver store (and off the R1), and its certificate out of the trusted
/// stores. MediaTek's driver stays; it's uninstalled from Windows' apps list like any other.
pub fn undo_script(log: &Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Continue'
Start-Transcript -Path '{log}' | Out-Null
$store = pnputil /enum-drivers | Out-String
$blocks = $store -split "(\r?\n){{2,}}"
foreach ($b in $blocks) {{
  if ($b -match '(?im)^\s*\S[^:]*:\s*{inf}\s*$') {{
    $oem = [regex]::Match($b, 'oem\d+\.inf').Value
    if ($oem) {{ pnputil /delete-driver $oem /uninstall /force; "removed $oem" }}
  }}
}}
foreach ($s in 'Root','TrustedPublisher') {{
  Get-ChildItem ("Cert:\LocalMachine\" + $s) | Where-Object {{ $_.Subject -eq '{subject}' }} | ForEach-Object {{ Remove-Item $_.PSPath; "certificate removed from $s" }}
}}
Stop-Transcript | Out-Null
"#,
        log = log.to_string_lossy().replace('\'', "''"),
        inf = regex_escape(INF_NAME),
        subject = CERT_SUBJECT,
    )
}

fn regex_escape(s: &str) -> String {
    s.replace('.', "\\.")
}

/// Writes the R1's driver package (its INF alone: the catalog's made in the elevated step) into [dir].
pub fn write_package(dir: &Path) -> Result<PathBuf> {
    let pkg = dir.join("r1-driver");
    let _ = std::fs::remove_dir_all(&pkg);
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(pkg.join(INF_NAME), inf().replace('\n', "\r\n"))?;
    Ok(pkg)
}

/// Runs [script] in PowerShell with Windows' administrator prompt, waits for it, and returns its exit code. The
/// prompt is the user's: declining it is an error here, and nothing changed.
#[cfg(windows)]
pub fn run_elevated(script_path: &Path) -> Result<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let verb = w("runas");
    let file = w("powershell.exe");
    let params = w(&format!("-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\"", script_path.display()));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_HIDE;
    // SAFETY: every string outlives the call; the process handle is closed below.
    unsafe {
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            bail!("the administrator prompt was declined, or PowerShell didn't start");
        }
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 {
            bail!("couldn't wait for the elevated step");
        }
        Ok(code)
    }
}

/// Installs the drivers behind one administrator prompt: the R1's, and [extras] (Rabbit's MediaTek one, UsbDk).
/// [dir] is where the package, the script and the log go. Returns the log.
#[cfg(windows)]
pub fn install(dir: &Path, extras: Extras<'_>) -> Result<String> {
    let pkg = if status()?.r1 { None } else { Some(write_package(dir)?) };
    std::fs::create_dir_all(dir)?;
    let log = dir.join("drivers-install.log");
    let script = dir.join("drivers-install.ps1");
    std::fs::write(&script, install_script(pkg.as_deref(), extras, &log))?;
    let code = run_elevated(&script)?;
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    if code != 0 {
        bail!("installing the drivers failed ({code}):\n{text}");
    }
    Ok(text)
}

/// Takes the R1's driver and its certificate back out, behind one administrator prompt. Returns the log.
#[cfg(windows)]
pub fn undo(dir: &Path) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let log = dir.join("drivers-undo.log");
    let script = dir.join("drivers-undo.ps1");
    std::fs::write(&script, undo_script(&log))?;
    run_elevated(&script)?;
    Ok(std::fs::read_to_string(&log).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inf_names_the_r1_and_android_s_interface() {
        let i = inf();
        assert!(i.contains(r"%R1% = R1_Install, USB\VID_0E8D&PID_201C"));
        assert!(i.contains("Include = winusb.inf") && i.contains("Needs   = WINUSB.NT.Services"));
        assert!(i.contains(r#"DeviceInterfaceGUIDs,0x10000,"{F72FE0D4-CBCB-407d-8814-9ED673D0DD6B}""#));
        assert!(i.contains("CatalogFile = r1os_r1_usb.cat"));
        // Nothing else of MediaTek's: not the preloader, not other products.
        assert_eq!(i.matches("VID_0E8D").count(), 3);
        assert!(!i.contains("PID_2000"));
    }

    #[test]
    fn what_the_store_holds() {
        let store = "Published Name:     oem64.inf\nOriginal Name:      cdc-acm.inf\nProvider Name:      MediaTek Inc.\n\n\
                     Published Name:     oem62.inf\nOriginal Name:      usb2ser_win764.inf\n\n\
                     Published Name:     oem200.inf\nOriginal Name:      r1os_r1_usb.inf\n";
        assert_eq!(status_of(store), Status { r1: true, mtk: true });
        assert_eq!(status_of("Original Name: cdc-acm.inf\n"), Status { r1: false, mtk: false });
        // A translated label still has the names after its colon.
        assert_eq!(status_of("Ursprünglicher Name:  r1os_r1_usb.inf\n"), Status { r1: true, mtk: false });
    }

    #[test]
    fn the_install_script_deletes_the_key_and_trusts_only_the_public_half() {
        let s = install_script(Some(Path::new(r"C:\x\r1-driver")), Extras { mtk_exe: Some(Path::new(r"C:\x\mtk.exe")), usbdk_msi: Some(Path::new(r"C:\x\UsbDk.msi")) }, Path::new(r"C:\x\log.txt"));
        assert!(s.contains("-DeleteKey"));
        assert!(s.contains("Cert:\\LocalMachine\\Root") && s.contains("Cert:\\LocalMachine\\TrustedPublisher"));
        assert!(s.contains("Export-Certificate") && !s.contains("Export-PfxCertificate"));
        assert!(s.contains("/VERYSILENT") && s.contains(r"C:\x\mtk.exe"));
        assert!(s.contains("pnputil /add-driver $inf /install"));
        assert!(s.contains("msiexec.exe") && s.contains(r"C:\x\UsbDk.msi") && s.contains("'/qn'"));
        let none = install_script(Some(Path::new(r"C:\x")), Extras::default(), Path::new(r"C:\x\l"));
        assert!(!none.contains("VERYSILENT") && !none.contains("msiexec"));
        // Paths with a quote can't break out of PowerShell's single quotes.
        let q = install_script(Some(Path::new(r"C:\Sam's\x")), Extras::default(), Path::new(r"C:\l"));
        assert!(q.contains(r"C:\Sam''s\x"));
        // The R1's installed already: only the extras, and no second certificate.
        let only = install_script(None, Extras { mtk_exe: None, usbdk_msi: Some(Path::new(r"C:\x\UsbDk.msi")) }, Path::new(r"C:\l"));
        assert!(only.contains("msiexec") && only.contains("installed already") && !only.contains("New-SelfSignedCertificate"));
    }

    #[test]
    fn the_undo_finds_its_own() {
        let s = undo_script(Path::new(r"C:\x\undo.log"));
        assert!(s.contains(r"r1os_r1_usb\.inf") && s.contains(CERT_SUBJECT) && s.contains("/delete-driver"));
    }
}
