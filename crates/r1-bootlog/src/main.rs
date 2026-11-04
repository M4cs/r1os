//! Verbose boot (T-101): the kernel's log, and init's progress in it, on the R1's screen from early boot until
//! SurfaceFlinger takes the display (`init.svc.surfaceflinger`, a property: init doesn't log its start). Init starts this at `on init` (r1os.rc); it does nothing unless
//! `/metadata/r1os/verbose_boot` exists (Settings › About › verbose boot, through r1d). It draws straight to the
//! framebuffer (`/dev/graphics/fb0`, MediaTek's mtkfb: 480×640, 32 bits) with an 8×8 font drawn twice its size,
//! and reads `/dev/kmsg` without blocking. Anything that fails makes it exit: boot never waits on it.

// The runtime below is Unix's (the R1's); on another host only the parsing is built, for its tests.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

mod font;

#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

const FLAG: &str = "/metadata/r1os/verbose_boot";
const FB: &str = "/dev/graphics/fb0";
/// How long it waits for /metadata to be mounted (it can be, just after `on init`).
const METADATA_WAIT: Duration = Duration::from_secs(4);
/// It gives up after this, whatever happens.
const MOST: Duration = Duration::from_secs(60);
/// It looks for new lines this often, and redraws at most this often.
const TICK: Duration = Duration::from_millis(150);
/// Glyphs are 8×8, drawn at this scale.
const SCALE: usize = 2;
const CELL: usize = 8 * SCALE;
/// Lines that say nothing about booting: MediaTek's charger, power and thermal drivers, many a second.
const NOISE: &[&str] = &[
    "[Power/PPM]", "mt6370_pmu_charger", "psy_chr_type", "get_pmic_vbus", "get_charger_type", "get_battery_voltage",
    "is_battery_exist", "pinctrl err", "_pd_is_algo_ready", "psy_charger_get_property", "[Thermal", "PD dbg info",
    "charger_thread", "tsbuck",
];

// ---- the framebuffer (Unix only: the parsing below is tested on any host) -----------------------------------

#[cfg(unix)]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Bitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[cfg(unix)]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct VarInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: Bitfield,
    green: Bitfield,
    blue: Bitfield,
    transp: Bitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[cfg(unix)]
#[repr(C)]
#[derive(Clone, Copy)]
struct FixInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    kind: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

#[cfg(unix)]
const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
#[cfg(unix)]
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
#[cfg(unix)]
const FBIOPAN_DISPLAY: libc::c_ulong = 0x4606;

#[cfg(unix)]
struct Screen {
    file: File,
    var: VarInfo,
    mem: *mut u8,
    len: usize,
    stride: usize,
    /// Byte offset of the page being shown.
    page: usize,
    w: usize,
    h: usize,
}

#[cfg(unix)]
impl Screen {
    fn open() -> Option<Screen> {
        let file = OpenOptions::new().read(true).write(true).open(FB).ok()?;
        let fd = file.as_raw_fd();
        let mut var = VarInfo::default();
        // SAFETY: the ioctls fill structs laid out as the kernel's (repr(C), the same fields).
        let mut fix: FixInfo = unsafe { std::mem::zeroed() };
        unsafe {
            if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) != 0 || libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix) != 0 {
                return None;
            }
        }
        if var.bits_per_pixel != 32 || var.xres == 0 || var.yres == 0 {
            return None;
        }
        let len = fix.smem_len as usize;
        // SAFETY: a shared mapping of the framebuffer's memory, its own length; unmapped when the process exits.
        let mem = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0) };
        if mem == libc::MAP_FAILED {
            return None;
        }
        let stride = fix.line_length as usize;
        let page = var.yoffset as usize * stride;
        if page + var.yres as usize * stride > len {
            return None;
        }
        Some(Screen { file, var, mem: mem.cast(), len, stride, page, w: var.xres as usize, h: var.yres as usize })
    }

    fn color(&self, (r, g, b): (u8, u8, u8)) -> u32 {
        let v = &self.var;
        let alpha = if v.transp.length > 0 { 0xffu32 << v.transp.offset } else { 0 };
        (u32::from(r) << v.red.offset) | (u32::from(g) << v.green.offset) | (u32::from(b) << v.blue.offset) | alpha
    }

    fn px(&mut self, x: usize, y: usize, c: u32) {
        if x >= self.w || y >= self.h {
            return;
        }
        let at = self.page + y * self.stride + x * 4;
        if at + 4 <= self.len {
            // SAFETY: inside the mapping (checked above), 4-byte pixels.
            unsafe { self.mem.add(at).cast::<u32>().write_unaligned(c) };
        }
    }

    fn fill(&mut self, x0: usize, y0: usize, w: usize, h: usize, c: u32) {
        for y in y0..(y0 + h).min(self.h) {
            for x in x0..(x0 + w).min(self.w) {
                self.px(x, y, c);
            }
        }
    }

    fn glyph(&mut self, ch: char, x0: usize, y0: usize, c: u32) {
        let g = font::FONT[if (ch as u32) < 128 { ch as usize } else { b'?' as usize }];
        for (row, bits) in g.iter().enumerate() {
            for col in 0..8 {
                if bits & (1 << col) != 0 {
                    self.fill(x0 + col * SCALE, y0 + row * SCALE, SCALE, SCALE, c);
                }
            }
        }
    }

    fn text(&mut self, s: &str, col: usize, row: usize, c: u32) {
        for (i, ch) in s.chars().enumerate() {
            self.glyph(ch, (col + i) * CELL, row * CELL, c);
        }
    }

    /// Shows the page (the driver redraws on a pan).
    fn show(&self) {
        let mut v = self.var;
        // SAFETY: the same struct the kernel gave, panned where it already is.
        unsafe { libc::ioctl(self.file.as_raw_fd(), FBIOPAN_DISPLAY as _, &mut v) };
    }
}

// ---- the kernel's log -------------------------------------------------------------------------------------

struct Line {
    secs: f64,
    text: String,
    level: u8,
}

/// One `/dev/kmsg` record (`prio,seq,usecs,flags;message`), if it's worth showing.
fn parse(rec: &str) -> Option<Line> {
    let (head, body) = rec.split_once(';')?;
    let mut f = head.split(',');
    let prio: u32 = f.next()?.parse().ok()?;
    let _seq = f.next()?;
    let usecs: u64 = f.next()?.parse().ok()?;
    let text = body.lines().next()?.trim();
    // MediaTek's kernel prefixes its own lines with the CPU and the task: "(3)[123:name]". Lines logged in this
    // process are its own echo: each frame it shows makes the display driver log two (`[DDP/DSI]
    // DSI_PHY_TIMCONFIG`), which were drawn, which showed a frame… (v0.2.16's first boot: 22 a second).
    let text = match text.strip_prefix('(') {
        Some(rest) => match rest.split_once(']') {
            Some((head, _)) if head.contains(":r1os-bootlog") => return None,
            Some((_, t)) => t.trim_start(),
            None => text,
        },
        None => text,
    };
    if text.is_empty() || NOISE.iter().any(|n| text.contains(n)) {
        return None;
    }
    Some(Line { secs: usecs as f64 / 1e6, text: text.to_owned(), level: (prio & 7) as u8 })
}

/// An Android property (empty if unset or unreadable).
#[cfg(target_os = "android")]
fn prop(name: &str) -> String {
    unsafe extern "C" {
        fn __system_property_get(name: *const libc::c_char, value: *mut libc::c_char) -> libc::c_int;
    }
    let Ok(n) = std::ffi::CString::new(name) else { return String::new() };
    let mut buf = [0 as libc::c_char; 92];
    // SAFETY: bionic writes at most PROP_VALUE_MAX (92) bytes, NUL included.
    unsafe {
        __system_property_get(n.as_ptr(), buf.as_mut_ptr());
        std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

#[cfg(all(unix, not(target_os = "android")))]
fn prop(_: &str) -> String {
    String::new()
}

/// SurfaceFlinger is up: the display is about to be its (init doesn't log its start to the kernel).
#[cfg(unix)]
fn display_taken() -> bool {
    prop("init.svc.surfaceflinger") == "running"
}

// ---- a friend in the corner (Max, 2026-10-12: "a little screenbuffer pixelart friend") ------------------------

/// r1os's orb, 16×15, drawn [FRIEND_PX] times its size: k outline, o body, h highlight, d shade, e eyes, p cheeks.
const FRIEND: [&str; 15] = [
    ".....kkkkkk.....",
    "...kkoooooookk..",
    "..kohhoooooodk..",
    ".kohhoooooooodk.",
    ".kohooooooooodk.",
    "kooooooooooooodk",
    "kooooeooooeooodk",
    "kooooeooooeooodk",
    "koopoooooooopodk",
    "kooooooooooooddk",
    ".kooooooooooddk.",
    ".kdoooooooooddk.",
    "..kddooooodddk..",
    "...kkddddddkk...",
    ".....kkkkkk.....",
];
const FRIEND_PX: usize = 4;

/// What the friend is doing on tick `n`: up or down (it bobs), eyes shut (a blink now and then), or looking at the
/// log (a new line just came).
#[derive(PartialEq, Eq, Clone, Copy)]
struct Pose {
    up: bool,
    shut: bool,
    look: bool,
}

fn pose(n: u64, last_line: u64) -> Pose {
    Pose { up: (n / 4) % 2 == 1, shut: n.is_multiple_of(26), look: n.saturating_sub(last_line) < 3 }
}

#[cfg(unix)]
fn draw_friend(screen: &mut Screen, p: Pose, bg: u32, ink: &dyn Fn(char) -> Option<u32>, shadow: u32) {
    let (fw, fh) = (16 * FRIEND_PX, (FRIEND.len() + 3) * FRIEND_PX);
    let x0 = screen.w.saturating_sub(fw + 12);
    let y0 = screen.h.saturating_sub(fh + 10);
    screen.fill(x0, y0, fw, fh, bg);
    let lift = if p.up { 0 } else { FRIEND_PX };
    for (r, row) in FRIEND.iter().enumerate() {
        for (c, ch) in row.chars().enumerate() {
            let eye_row = ch == 'e' || (row.contains('e') && ch == 'o');
            let ch = match (ch, eye_row) {
                // Shut: the upper half of each eye is body, so a line is left.
                ('e', _) if p.shut && r == 6 => 'o',
                // Looking left at the log: each eye one pixel over.
                ('e', _) if p.look => 'o',
                ('o', true) if p.look && !(p.shut && r == 6) && (row.as_bytes().get(c + 1) == Some(&b'e')) => 'e',
                (ch, _) => ch,
            };
            if let Some(color) = ink(ch) {
                screen.fill(x0 + c * FRIEND_PX, y0 + FRIEND_PX + lift + r * FRIEND_PX, FRIEND_PX, FRIEND_PX, color);
            }
        }
    }
    // Its shadow, smaller while it's up.
    let half = if p.up { 4 } else { 5 };
    let sy = y0 + (FRIEND.len() + 2) * FRIEND_PX;
    screen.fill(x0 + (8 - half) * FRIEND_PX, sy, 2 * half * FRIEND_PX, FRIEND_PX, shadow);
}

#[cfg(unix)]
fn metadata_mounted() -> bool {
    std::fs::read_to_string("/proc/mounts").is_ok_and(|m| m.lines().any(|l| l.split(' ').nth(1) == Some("/metadata")))
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn main() {
    let started = Instant::now();
    while !metadata_mounted() {
        if started.elapsed() > METADATA_WAIT {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if std::fs::metadata(FLAG).is_err() {
        return;
    }
    let Some(mut screen) = Screen::open() else { return };
    let Ok(mut kmsg) = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open("/dev/kmsg") else { return };

    let bg = screen.color((14, 16, 19));
    let fg = screen.color((236, 238, 241));
    let dim = screen.color((120, 128, 140));
    let accent = screen.color((255, 138, 61));
    let init_c = screen.color((140, 196, 255));
    let bad = screen.color((255, 112, 112));
    let cols = screen.w / CELL;
    let rows = screen.h / CELL;
    let body_rows = rows.saturating_sub(2);
    let palette = [
        ('k', screen.color((122, 59, 18))),
        ('o', screen.color((255, 138, 61))),
        ('h', screen.color((255, 196, 140))),
        ('d', screen.color((217, 106, 36))),
        ('e', screen.color((20, 22, 26))),
        ('p', screen.color((255, 120, 150))),
    ];
    let ink = |ch: char| palette.iter().find(|(k, _)| *k == ch).map(|(_, c)| *c);
    let shadow = screen.color((6, 7, 9));
    let mut n: u64 = 0;
    let mut last_line: u64 = u64::MAX / 2;
    let mut was: Option<Pose> = None;

    let mut wrapped: Vec<(String, u32)> = Vec::new();
    let mut buf = vec![0u8; 8192];
    let mut dirty = true;
    while started.elapsed() < MOST {
        // Every record waiting (one a read); EAGAIN when there are none.
        loop {
            match kmsg.read(&mut buf) {
                Ok(len) if len > 0 => {
                    let rec = String::from_utf8_lossy(&buf[..len]);
                    let Some(line) = parse(&rec) else { continue };
                    let c = if line.level <= 3 { bad } else if line.text.starts_with("init:") { init_c } else { fg };
                    let full = format!("{:>5.1} {}", line.secs, line.text);
                    let chars: Vec<char> = full.chars().map(|ch| if ch.is_ascii() && !ch.is_ascii_control() { ch } else { '?' }).collect();
                    for (i, chunk) in chars.chunks(cols.max(8) - 1).enumerate() {
                        let s: String = chunk.iter().collect();
                        wrapped.push((if i == 0 { s } else { format!(" {s}") }, c));
                    }
                    dirty = true;
                    last_line = n;
                }
                Ok(_) => break,
                Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue, // overwritten before it was read
                Err(_) => break,
            }
        }
        if display_taken() {
            break;
        }
        if wrapped.len() > body_rows {
            wrapped.drain(..wrapped.len() - body_rows);
        }
        let now = pose(n, last_line);
        if was != Some(now) {
            dirty = true;
        }
        if dirty {
            let (w, h) = (screen.w, screen.h);
            screen.fill(0, 0, w, h, bg);
            screen.text("r1os", 0, 0, accent);
            screen.text("verbose boot", 5, 0, dim);
            for (i, (s, c)) in wrapped.iter().enumerate() {
                screen.text(s, 0, i + 2, *c);
            }
            draw_friend(&mut screen, now, bg, &ink, shadow);
            screen.show();
            dirty = false;
            was = Some(now);
        }
        n += 1;
        std::thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kmsg_records_are_read_and_noise_is_dropped() {
        let l = parse("6,1234,6876543,-;(3)[372:init]init: starting service 'surfaceflinger'...\n SUBSYSTEM=x").unwrap();
        assert_eq!(l.text, "init: starting service 'surfaceflinger'...");
        assert!((l.secs - 6.876543).abs() < 1e-9);
        assert_eq!(l.level, 6);
        assert!(parse("6,1,2,-;(7)[19502:kworker/7:0][Thermal/TZ/PMIC] t_current=31515").is_none());
        assert!(parse("6,1,2,-;mt6370_pmu_charger mt6370_pmu_charger: online = 0").is_none());
        assert_eq!(parse("3,9,1000000,-;EXT4-fs error").unwrap().level, 3);
        assert!(parse("garbage").is_none());
        assert!(parse("6,5,20625975,-;(6)[473:r1os-bootlog][DDP/DSI]===>DSI_PHY_TIMCONFIG start").is_none(), "its own echo");
    }
}
