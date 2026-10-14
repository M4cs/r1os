//! Finding the R1, whatever it's doing: the USB devices themselves (nusb sees one even with no driver), then what
//! adb and fastboot make of them. One plugged in as `0E8D:201C` that neither tool lists has no driver yet: on Windows,
//! the bootloader's fastboot until one is picked (T-168).

use crate::adb::{self, Adb, State};
use crate::fastboot::{self, Fastboot};
use crate::tools::Tools;
use crate::{PID_ANDROID, PID_PRELOADER, VID_MEDIATEK};
use serde::Serialize;

/// What an R1 is doing, as far as the PC can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The MediaTek preloader's serial port: the second at power-on when it can be asked into fastboot.
    Preloader,
    /// Booted, with USB debugging allowed.
    Adb,
    /// Booted, waiting for "Allow USB debugging?" on the R1.
    Unauthorized,
    /// The bootloader's fastboot.
    Fastboot,
    /// fastbootd, in userspace (where `system` and `product` are written).
    Fastbootd,
    /// Plugged in, but neither adb nor fastboot can reach it: a driver's missing (or, without platform-tools, not
    /// known yet).
    NoDriver,
}

/// One R1 found.
#[derive(Debug, Clone, Serialize)]
pub struct Found {
    /// Its name here: a hash of its serial ([`crate::identity`]).
    pub id: String,
    pub mode: Mode,
    /// In fastboot: whether the bootloader's unlocked.
    pub unlocked: Option<bool>,
    /// In fastboot: the active slot (`a` or `b`).
    pub slot: Option<String>,
    /// Booted: r1os's image version, if it's running r1os (`ro.r1os.image.version`).
    pub r1os: Option<String>,
    #[serde(skip)]
    serial: String,
}

impl Found {
    /// Its fastboot, to read from or write to (writes go through [`crate::safety`]).
    pub fn fastboot(&self, tools: &Tools) -> Option<Fastboot> {
        matches!(self.mode, Mode::Fastboot | Mode::Fastbootd).then(|| Fastboot::new(tools.fastboot.clone(), self.serial.clone()))
    }

    pub fn adb(&self, tools: &Tools) -> Option<Adb> {
        (self.mode == Mode::Adb).then(|| Adb::new(tools.adb.clone(), self.serial.clone()))
    }
}

/// A USB device: its vendor and product ids and serial.
#[derive(Debug, Clone)]
pub struct Usb {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
}

/// The MediaTek devices plugged in, driver or not.
#[cfg(not(windows))]
pub fn usb() -> Vec<Usb> {
    nusb::list_devices()
        .map(|it| {
            it.filter(|d| d.vendor_id() == VID_MEDIATEK)
                .map(|d| Usb { vid: d.vendor_id(), pid: d.product_id(), serial: d.serial_number().map(str::to_owned) })
                .collect()
        })
        .unwrap_or_default()
}

/// The MediaTek devices plugged in, driver or not, from Windows' configuration manager: nusb doesn't list a device
/// with no driver, which is just the R1 in its bootloader before one's picked (seen 2026-10-14: "Android", code 28).
/// Its instance id is `USB\VID_0E8D&PID_201C\<serial>`, the serial adb and fastboot know it by.
#[cfg(windows)]
pub fn usb() -> Vec<Usb> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW, CR_SUCCESS,
    };
    let filter: Vec<u16> = "USB\0".encode_utf16().collect();
    let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
    let mut len = 0u32;
    // SAFETY: the filter is NUL-terminated and the buffer is the length the first call reported.
    let ids = unsafe {
        if CM_Get_Device_ID_List_SizeW(&mut len, filter.as_ptr(), flags) != CR_SUCCESS || len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_ID_ListW(filter.as_ptr(), buf.as_mut_ptr(), len, flags) != CR_SUCCESS {
            return Vec::new();
        }
        buf
    };
    ids.split(|c| *c == 0).filter(|s| !s.is_empty()).filter_map(|s| parse_instance(&String::from_utf16_lossy(s))).collect()
}

/// `USB\VID_0E8D&PID_201C\<serial>` as a [Usb]; a composite device's interfaces (`&MI_01`) and other vendors aren't.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_instance(id: &str) -> Option<Usb> {
    let mut parts = id.split('\\');
    if !parts.next()?.eq_ignore_ascii_case("USB") {
        return None;
    }
    let hw = parts.next()?.to_ascii_uppercase();
    if hw.contains("&MI_") {
        return None;
    }
    let vid = u16::from_str_radix(hw.strip_prefix("VID_")?.get(..4)?, 16).ok()?;
    let pid = u16::from_str_radix(hw.split("&PID_").nth(1)?.get(..4)?, 16).ok()?;
    if vid != VID_MEDIATEK {
        return None;
    }
    // A device with no serial of its own gets one made up by Windows (it has `&` in it): not the R1's.
    let serial = parts.next().filter(|s| !s.contains('&')).map(str::to_owned);
    Some(Usb { vid, pid, serial })
}

/// Every R1 plugged in, and what it's doing. With no [tools] yet, what USB alone can say.
pub fn scan(tools: Option<&Tools>) -> Vec<Found> {
    let usb = usb();
    let (adb_list, fb_list) = match tools {
        Some(t) => (adb::devices(&t.adb).unwrap_or_default(), fastboot::devices(&t.fastboot).unwrap_or_default()),
        None => (Vec::new(), Vec::new()),
    };
    let mut found = merge(&usb, &adb_list, &fb_list);
    // What each one says of itself.
    if let Some(t) = tools {
        for f in &mut found {
            match f.mode {
                Mode::Fastboot | Mode::Fastbootd => {
                    let fb = Fastboot::new(t.fastboot.clone(), f.serial.clone());
                    if fb.userspace().unwrap_or(false) {
                        f.mode = Mode::Fastbootd;
                    }
                    f.unlocked = fb.unlocked().ok().flatten();
                    f.slot = fb.getvar("current-slot").ok().flatten();
                }
                Mode::Adb => {
                    let a = Adb::new(t.adb.clone(), f.serial.clone());
                    f.r1os = a.getprop("ro.r1os.image.version").ok().filter(|v| !v.is_empty());
                }
                _ => {}
            }
        }
    }
    found
}

/// What USB, adb and fastboot list, as R1s. adb's and fastboot's own lists come first (a device there has its driver);
/// a MediaTek Android device neither lists has none.
fn merge(usb: &[Usb], adb_list: &[(String, State)], fb_list: &[String]) -> Vec<Found> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |serial: &str, mode: Mode, found: &mut Vec<Found>| {
        if seen.insert(serial.to_owned()) {
            found.push(Found { id: crate::identity(serial), mode, unlocked: None, slot: None, r1os: None, serial: serial.to_owned() });
        }
    };
    for (s, state) in adb_list {
        let mode = match state {
            State::Unauthorized => Mode::Unauthorized,
            _ => Mode::Adb,
        };
        // Only R1s: adb lists any phone; one that USB shows isn't MediaTek's is left out.
        if usb.is_empty() || usb.iter().any(|u| u.serial.as_deref() == Some(s)) {
            add(s, mode, &mut found);
        }
    }
    for s in fb_list {
        if usb.is_empty() || usb.iter().any(|u| u.serial.as_deref() == Some(s)) {
            add(s, Mode::Fastboot, &mut found);
        }
    }
    for u in usb {
        let serial = u.serial.clone().unwrap_or_else(|| format!("usb-{:04x}:{:04x}", u.vid, u.pid));
        match u.pid {
            PID_PRELOADER => add(&serial, Mode::Preloader, &mut found),
            PID_ANDROID => add(&serial, Mode::NoDriver, &mut found),
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb(pid: u16, serial: &str) -> Usb {
        Usb { vid: VID_MEDIATEK, pid, serial: Some(serial.into()) }
    }

    #[test]
    fn each_mode_from_what_the_pc_sees() {
        // Booted with debugging allowed; in fastboot; plugged in with no driver; the preloader.
        let u = [usb(PID_ANDROID, "R1A"), usb(PID_ANDROID, "R1B"), usb(PID_ANDROID, "R1C"), usb(PID_PRELOADER, "R1D")];
        let f = merge(&u, &[("R1A".into(), State::Device)], &["R1B".into()]);
        let modes: Vec<_> = f.iter().map(|x| x.mode).collect();
        assert_eq!(modes, [Mode::Adb, Mode::Fastboot, Mode::NoDriver, Mode::Preloader]);
        // Named by their identity, never their serial.
        assert!(f.iter().all(|x| x.id.len() == 12 && !x.id.contains("R1")));
        let json = serde_json::to_string(&f).unwrap();
        assert!(!json.contains("R1A") && !json.contains("serial"));
    }

    #[test]
    fn other_phones_are_left_out() {
        let u = [usb(PID_ANDROID, "R1A")];
        let f = merge(&u, &[("R1A".into(), State::Device), ("PIXEL9".into(), State::Device)], &[]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].mode, Mode::Adb);
    }

    #[test]
    fn windows_instance_ids() {
        let u = parse_instance(r"USB\VID_0E8D&PID_201C\0123456789ABCDEFGHIJ").unwrap();
        assert_eq!((u.vid, u.pid, u.serial.as_deref()), (VID_MEDIATEK, PID_ANDROID, Some("0123456789ABCDEFGHIJ")));
        assert_eq!(parse_instance(r"USB\VID_0E8D&PID_2000\5&1a2b3c4d&0&2").unwrap().serial, None);
        assert!(parse_instance(r"USB\VID_0E8D&PID_201C&MI_01\6&2d3e&0&0001").is_none());
        assert!(parse_instance(r"USB\VID_046D&PID_C52B\X").is_none());
        assert!(parse_instance(r"HID\VID_0E8D&PID_201C\X").is_none());
    }

    #[test]
    fn waiting_for_the_r1_s_allow() {
        let f = merge(&[usb(PID_ANDROID, "R1A")], &[("R1A".into(), State::Unauthorized)], &[]);
        assert_eq!(f[0].mode, Mode::Unauthorized);
    }
}
