//! The camera motor and read-only system facts (battery, memory).
//!
//! Motor: `drivers/misc/step_motor_ms35774.c` in Rabbit's kernel. Its sysfs `orientation` attribute takes 0–180
//! (degrees; 0 faces you, 180 the world) and reads back where the driver thinks it is. It's an open-loop stepper
//! (6 steps a degree, ~0.41 ms a step, no position sensor). At boot it believes it's at 180 and drives to 0, into the
//! end stop, which homes it; then it goes to 90, and it parks at 90 at shutdown.
//!
//! **Never write while it moves** (ADR-0046). The driver's thread takes the target when a move starts and, when the
//! move ends, records *the latest target written* as where it is, dropping that request. A new target mid-move (a
//! fast turn of the wheel) left it believing it was somewhere it wasn't: the camera undershot after that. So one
//! worker writes, waits for each move to end (the read-back reaching the target), and only then writes the next;
//! targets asked for meanwhile collapse into the latest.
//!
//! That same behaviour re-homes it ([`Motor::home`]): a second target written mid-move makes the driver believe it's
//! further from 0 than it is, and the next move to 0 runs on into the end stop, as at boot.
//!
//! r1d isn't the only writer: Rabbit's camera HAL turns it to 180 when the camera opens and parks it at 90 when it
//! closes. So when nothing's moving, where it is is what the driver says, and the host sends its target again once
//! the camera has opened (ADR-0046).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const MOTOR_SYSFS: &str = "/sys/devices/platform/step_motor_ms35774/orientation";
pub const MOTOR_MAX: u16 = 180;
/// Facing you, and facing the world.
pub const USER: u16 = 0;
pub const WORLD: u16 = 180;
/// How far past 0 turning to face you drives into the stop, re-seating it there each time (a short buzz).
const SEAT_DEG: u16 = 15;
/// How far past 0 a re-home drives: it finds the stop from up to this far off.
const HOME_DEG: u16 = 90;

/// What the worker does next (only the latest is kept).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Job {
    To(u16),
    /// To 0, on into the stop by this much: it ends homed there.
    Seat(u16),
    /// Re-homed (from up to [`HOME_DEG`] off), then to this.
    Home(u16),
}

pub enum Motor {
    Sysfs(Arc<Mover>),
    /// Desktop simulator, or a device without the node.
    Sim(AtomicU16),
}

impl Motor {
    pub fn detect(sim: bool) -> Self {
        let path = PathBuf::from(MOTOR_SYSFS);
        if !sim && path.exists() {
            Motor::Sysfs(Mover::start(Box::new(Sysfs(path))))
        } else {
            Motor::Sim(AtomicU16::new(90))
        }
    }

    pub fn is_sim(&self) -> bool {
        matches!(self, Motor::Sim(_))
    }

    /// Where it's going (or is): the latest target.
    pub fn get(&self) -> Option<u16> {
        match self {
            Motor::Sysfs(m) => m.target(),
            Motor::Sim(v) => Some(v.load(Ordering::Relaxed)),
        }
    }

    pub fn moving(&self) -> bool {
        match self {
            Motor::Sysfs(m) => m.moving(),
            Motor::Sim(_) => false,
        }
    }

    /// Turns to `deg` (clamped to 0–180), after the move under way. Returns the target.
    pub fn set(&self, deg: u16) -> u16 {
        let deg = deg.min(MOTOR_MAX);
        match self {
            Motor::Sysfs(m) => m.ask(Job::To(deg), deg),
            Motor::Sim(v) => v.store(deg, Ordering::Relaxed),
        }
        deg
    }

    /// Faces you, seated against the stop (so a drift toward the world is undone each time).
    pub fn face_user(&self) -> u16 {
        match self {
            Motor::Sysfs(m) => m.ask(Job::Seat(SEAT_DEG), USER),
            Motor::Sim(v) => v.store(USER, Ordering::Relaxed),
        }
        USER
    }

    /// Re-homed against the stop at 0, then to `then`: for when it's lost track (it was blocked or pushed).
    pub fn home(&self, then: u16) -> u16 {
        let then = then.min(MOTOR_MAX);
        match self {
            Motor::Sysfs(m) => m.ask(Job::Home(then), then),
            Motor::Sim(v) => v.store(then, Ordering::Relaxed),
        }
        then
    }

    /// Called with the position each time the motor comes to rest (the hub sends `motor {moving: false}`).
    pub fn on_rest(&self, f: impl Fn(u16) + Send + 'static) {
        if let Motor::Sysfs(m) = self {
            *m.rest.lock().unwrap() = Some(Box::new(f));
        }
    }
}

/// Told where the motor came to rest.
type OnRest = Box<dyn Fn(u16) + Send>;

/// The driver's `orientation` node (a fake one in tests).
trait Node: Send + Sync {
    fn read(&self) -> Option<u16>;
    fn write(&self, deg: u16) -> std::io::Result<()>;
}

struct Sysfs(PathBuf);

impl Node for Sysfs {
    fn read(&self) -> Option<u16> {
        std::fs::read_to_string(&self.0).ok()?.trim().parse().ok()
    }

    fn write(&self, deg: u16) -> std::io::Result<()> {
        std::fs::write(&self.0, deg.to_string())
    }
}

/// The one writer of the motor's node, on its own thread.
pub struct Mover {
    node: Box<dyn Node>,
    state: Mutex<MoverState>,
    wake: Condvar,
    rest: Mutex<Option<OnRest>>,
}

#[derive(Default)]
struct MoverState {
    job: Option<Job>,
    target: Option<u16>,
    busy: bool,
}

impl Mover {
    fn start(node: Box<dyn Node>) -> Arc<Self> {
        let m = Arc::new(Mover { node, state: Mutex::default(), wake: Condvar::new(), rest: Mutex::new(None) });
        let w = m.clone();
        std::thread::Builder::new().name("motor".into()).spawn(move || w.run()).expect("the motor's thread");
        m
    }

    fn ask(&self, job: Job, target: u16) {
        let mut s = self.state.lock().unwrap();
        s.job = Some(job);
        s.target = Some(target);
        self.wake.notify_one();
    }

    /// Where it's going while it moves; where the driver says it is when it doesn't (the camera HAL moves it too).
    fn target(&self) -> Option<u16> {
        let s = self.state.lock().unwrap();
        if s.busy || s.job.is_some() {
            return s.target;
        }
        drop(s);
        self.read()
    }

    fn moving(&self) -> bool {
        let s = self.state.lock().unwrap();
        s.busy || s.job.is_some()
    }

    fn run(&self) {
        loop {
            let job = {
                let mut s = self.state.lock().unwrap();
                while s.job.is_none() {
                    s = self.wake.wait(s).unwrap();
                }
                s.busy = true;
                s.job.take()
            };
            match job {
                Some(Job::To(t)) => self.go(t),
                Some(Job::Seat(over)) => self.seat(over),
                Some(Job::Home(then)) => {
                    self.seat(HOME_DEG);
                    self.go(then);
                }
                None => {}
            }
            let rest = {
                let mut s = self.state.lock().unwrap();
                if s.job.is_some() {
                    continue;
                }
                s.busy = false;
                self.read()
            };
            if let (Some(at), Some(f)) = (rest, self.rest.lock().unwrap().as_ref()) {
                f(at);
            }
        }
    }

    fn read(&self) -> Option<u16> {
        self.node.read()
    }

    fn write(&self, deg: u16) {
        if let Err(e) = self.node.write(deg) {
            tracing::warn!(error = %e, deg, "motor: couldn't write");
        }
    }

    /// A plain move, waited out.
    fn go(&self, t: u16) {
        let Some(from) = self.read() else { return };
        if from == t {
            return;
        }
        self.write(t);
        self.until(t, from);
    }

    /// Waits until the driver says it's at `t` (it records that when the move ends), or it's had time and more.
    fn until(&self, t: u16, from: u16) {
        let deadline = Instant::now() + move_time(from, t) + Duration::from_millis(600);
        std::thread::sleep(move_time(from, t).mul_f32(0.8));
        while self.read() != Some(t) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(at) = self.read().filter(|at| *at != t) {
            tracing::warn!(sent = t, at, "motor: it ended elsewhere: something else wrote mid-move (the camera HAL?)");
        }
    }

    /// To 0 and on into the stop by `over` degrees: written 0, then `over` while that move runs, so the driver ends
    /// believing it's at `over` while it's at 0; then 0 again drives `over` degrees into the stop.
    fn seat(&self, over: u16) {
        let Some(mut b) = self.read() else { return };
        // The first move has to be long enough to write into, and to end somewhere other than `over`.
        if b < 100 || b == over {
            self.go(170);
            b = self.read().unwrap_or(170);
        }
        self.write(0);
        std::thread::sleep(Duration::from_millis(30));
        self.write(over);
        self.until(over, b);
        self.go(0);
        tracing::info!(over, "motor: seated at 0");
    }
}

/// About how long a move takes: 10 ms to power up, then 6 steps a degree at ~0.41 ms a step.
fn move_time(from: u16, to: u16) -> Duration {
    Duration::from_micros(10_000 + u64::from(from.abs_diff(to)) * 6 * 430)
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct System {
    pub battery_pct: Option<u8>,
    pub charging: Option<bool>,
    pub mem_available_mb: Option<u64>,
    pub mem_total_mb: Option<u64>,
    /// /data's free and whole size (where r1os, its apps, photos and models live).
    pub storage_free_mb: Option<u64>,
    pub storage_total_mb: Option<u64>,
}

pub fn system() -> System {
    let read = |p: &str| std::fs::read_to_string(p).ok().map(|s| s.trim().to_owned());
    let meminfo = read("/proc/meminfo").unwrap_or_default();
    let mem = |key: &str| {
        meminfo
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb / 1024)
    };
    let storage = storage(if std::path::Path::new("/data").is_dir() { "/data" } else { "/" });
    System {
        battery_pct: read("/sys/class/power_supply/battery/capacity").and_then(|s| s.parse().ok()),
        charging: read("/sys/class/power_supply/battery/status").map(|s| s == "Charging" || s == "Full"),
        mem_available_mb: mem("MemAvailable:"),
        mem_total_mb: mem("MemTotal:"),
        storage_free_mb: storage.map(|s| s.0),
        storage_total_mb: storage.map(|s| s.1),
    }
}

/// The battery's sysfs node.
const BATTERY: &str = "/sys/class/power_supply/battery";

/// Android's own estimate of the time to full charge, as the host last passed it (`POST /api/battery`): from how long each
/// percent has taken while charging (`BatteryManager.computeChargeTimeRemaining`), steadier than the battery's own,
/// which swings with the current from moment to moment (2.0, 1.3 and 2.8 h within minutes at 45%, 2026-10-13).
static ANDROID_ESTIMATE: Mutex<Option<(Instant, i64)>> = Mutex::new(None);

/// The host's latest estimate (ms to full; none when Android hasn't one).
pub fn set_android_estimate(charge_ms: Option<i64>) {
    *ANDROID_ESTIMATE.lock().unwrap() = charge_ms.filter(|ms| *ms > 0).map(|ms| (Instant::now(), ms));
}

/// Android's estimate as of now, if it's fresh: a battery change comes with each percent while charging.
fn android_estimate_hours() -> Option<f64> {
    let (at, ms) = (*ANDROID_ESTIMATE.lock().unwrap())?;
    let age = at.elapsed();
    (age < Duration::from_secs(20 * 60)).then(|| (ms as f64 / 1000.0 - age.as_secs_f64()).max(0.0) / 3600.0)
}

/// What the R1 is plugged into, by the type of the power supply that's online (the R1's MediaTek charger is
/// `mt6370_pmu_charger`: `USB_DCP` on a wall charger, `USB` on a computer): ("usb", words) for a computer's port.
fn plugged_into() -> Option<(&'static str, &'static str)> {
    let mut best: Option<(&'static str, &'static str)> = None;
    for e in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let read = |f: &str| std::fs::read_to_string(e.path().join(f)).ok().map(|s| s.trim().to_owned()).unwrap_or_default();
        if read("online") != "1" {
            continue;
        }
        let found = match read("type").as_str() {
            "Mains" | "USB_DCP" | "USB_HVDCP" | "USB_HVDCP_3" | "USB_PD" | "USB_C" => ("ac", "a charger"),
            "Wireless" => ("wireless", "a wireless charger"),
            "USB" | "USB_SDP" | "USB_CDP" | "USB_ACA" => ("usb", "a computer's USB port"),
            _ => continue,
        };
        // A charger says more than a port does, where both are online.
        if best.is_none_or(|b| b.0 == "usb") {
            best = Some(found);
        }
    }
    best
}

/// How the battery stands, for an agent asked how long until it's charged (T-152). Read when asked: the current is
/// averaged over [`CURRENT_SAMPLES`] readings [`CURRENT_EVERY`] apart, and nothing is kept or polled after.
pub async fn battery() -> serde_json::Value {
    let read = |f: &str| std::fs::read_to_string(format!("{BATTERY}/{f}")).ok().map(|s| s.trim().to_owned());
    let num = |f: &str| read(f).and_then(|s| s.parse::<i64>().ok());
    let level = num("capacity");
    let status = read("status").unwrap_or_default();
    let from = plugged_into();
    let mut sum = 0i64;
    let mut n = 0i64;
    for i in 0..CURRENT_SAMPLES {
        if let Some(c) = num("current_now") {
            sum += c;
            n += 1;
        }
        if i + 1 < CURRENT_SAMPLES {
            tokio::time::sleep(CURRENT_EVERY).await;
        }
    }
    let current_ua = (n > 0).then(|| sum / n);
    let counter = num("charge_counter");
    let full = num("charge_full").filter(|f| *f > 0).or_else(|| num("charge_full_design").filter(|f| *f > 0)).or_else(|| match (counter, level) {
        (Some(c), Some(l)) if l > 0 => Some(c * 100 / l),
        _ => None,
    });
    let charging = status == "Charging";
    let android = if charging { android_estimate_hours() } else { None };
    let kernel_s = num("time_to_full_now").filter(|s| *s > 0);
    let (estimate, how) = match (charging, android, kernel_s) {
        (true, Some(h), _) => (Some(h), "Android's own estimate, from how long each percent has taken"),
        (true, None, Some(s)) => (Some(s as f64 / 3600.0), "the battery's own estimate, which swings with what the R1 is doing"),
        (true, None, None) => (estimate_hours(true, level, counter, full, current_ua), "from the charge left to go and the current now; charging slows above 80%"),
        (false, ..) => (estimate_hours(false, level, counter, full, current_ua), "from the charge left and what the R1 is drawing now; it changes with what it's doing"),
    };
    let mut out = serde_json::json!({
        "level_percent": level,
        "state": match status.as_str() { "Charging" => "charging", "Full" => "full", "Discharging" | "Not charging" if from.is_some() => "plugged in, not charging", "" => "unknown", _ => "on battery" },
        "plugged_into": from.map(|f| f.1),
        "current_ma": current_ua.map(|c| c.abs() / 1000),
    });
    if let Some(h) = estimate {
        let key = if charging { "until_full" } else { "until_empty" };
        out[key] = serde_json::json!(words_for(h));
        out["estimate"] = serde_json::json!(how);
    }
    if charging && from.is_some_and(|f| f.0 == "usb") {
        out["note"] = serde_json::json!("A computer's USB port gives about half an amp; a wall charger is faster, and the R1 can drain on a USB port while it's busy.");
    }
    out
}

const CURRENT_SAMPLES: usize = 8;
const CURRENT_EVERY: Duration = Duration::from_millis(250);

/// About how many hours until full (charging) or empty (not), from the charge counter and full charge (µAh) and the
/// current (µA, either sign: kernels differ). Charging slows above 80%: that part is taken at half the current now.
pub fn estimate_hours(charging: bool, level: Option<i64>, counter: Option<i64>, full: Option<i64>, current_ua: Option<i64>) -> Option<f64> {
    let i = current_ua?.abs() as f64;
    if i < 20_000.0 {
        return None;
    }
    let (c, f) = (counter? as f64, full? as f64);
    if !charging {
        return Some(c / i);
    }
    if level? >= 100 || c >= f {
        return Some(0.0);
    }
    let knee = 0.8 * f;
    let steady = (knee - c).max(0.0);
    let tail = f - c.max(knee);
    Some(steady / i + tail / (i * 0.5))
}

/// Hours as an agent would say them: "about 1 h 40 min", "about 25 min".
pub fn words_for(h: f64) -> String {
    if h <= 0.0 {
        return "full now".into();
    }
    if h > 48.0 {
        return "more than two days at this rate".into();
    }
    let m = (h * 60.0 / 5.0).round() as i64 * 5;
    match (m / 60, m % 60) {
        (0, 0) => "about 5 min".into(),
        (0, mm) => format!("about {mm} min"),
        (hh, 0) => format!("about {hh} h"),
        (hh, mm) => format!("about {hh} h {mm} min"),
    }
}

/// A filesystem's free (to apps) and whole size, in MiB.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // the fields' widths differ by target
fn storage(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    // SAFETY: statvfs fills the zeroed struct from a valid C string; nothing is kept.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let unit = s.f_frsize as u64;
    Some(((s.f_bavail as u64 * unit) >> 20, (s.f_blocks as u64 * unit) >> 20))
}

#[cfg(not(unix))]
fn storage(_: &str) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn time_to_full_slows_above_80() {
        // 1,000 mAh, at 40%, charging at 500 mA: 400 mAh to 80% (0.8 h), then 200 mAh at 250 mA (0.8 h).
        let h = super::estimate_hours(true, Some(40), Some(400_000), Some(1_000_000), Some(500_000)).unwrap();
        assert!((h - 1.6).abs() < 0.01, "{h}");
        // Kernels differ on the sign.
        assert_eq!(super::estimate_hours(true, Some(40), Some(400_000), Some(1_000_000), Some(-500_000)), Some(h));
        // Full, or no real current: nothing to say.
        assert_eq!(super::estimate_hours(true, Some(100), Some(1_000_000), Some(1_000_000), Some(500_000)), Some(0.0));
        assert_eq!(super::estimate_hours(true, Some(40), Some(400_000), Some(1_000_000), Some(5_000)), None);
        // On battery: what's left over what it's drawing.
        let h = super::estimate_hours(false, Some(50), Some(500_000), Some(1_000_000), Some(250_000)).unwrap();
        assert!((h - 2.0).abs() < 0.01, "{h}");
        assert_eq!(super::words_for(1.6), "about 1 h 35 min");
        assert_eq!(super::words_for(0.42), "about 25 min");
        assert_eq!(super::words_for(0.0), "full now");
    }

    use super::*;

    /// Rabbit's driver as `step_motor_ms35774.c` runs it: a thread takes the target when a move starts, moves, then
    /// records the latest target written as where it is. The lens stops at 0 (an end stop); `physical` is where it is.
    struct Driver {
        s: Mutex<DriverState>,
        wake: Condvar,
    }

    struct DriverState {
        orientation: u16,
        request: u16,
        run: bool,
        physical: i32,
    }

    impl Driver {
        fn new(believes: u16, physical: i32) -> Arc<Self> {
            let d = Arc::new(Driver {
                s: Mutex::new(DriverState { orientation: believes, request: believes, run: false, physical }),
                wake: Condvar::new(),
            });
            let t = d.clone();
            std::thread::spawn(move || t.thread());
            d
        }

        fn thread(&self) {
            loop {
                let (from, to) = {
                    let mut s = self.s.lock().unwrap();
                    while !s.run {
                        s = self.wake.wait(s).unwrap();
                    }
                    if s.orientation == s.request {
                        s.run = false;
                        continue;
                    }
                    (s.orientation, s.request)
                };
                std::thread::sleep(move_time(from, to));
                let mut s = self.s.lock().unwrap();
                s.physical = (s.physical + i32::from(to) - i32::from(from)).max(0);
                s.run = false;
                s.orientation = s.request;
            }
        }

        fn at(&self) -> (u16, i32) {
            let s = self.s.lock().unwrap();
            (s.orientation, s.physical)
        }
    }

    impl Node for Arc<Driver> {
        fn read(&self) -> Option<u16> {
            Some(self.s.lock().unwrap().orientation)
        }

        fn write(&self, deg: u16) -> std::io::Result<()> {
            let mut s = self.s.lock().unwrap();
            s.request = deg;
            s.run = true;
            self.wake.notify_one();
            Ok(())
        }
    }

    fn settle(m: &Mover) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while m.moving() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn writing_mid_move_loses_track() {
        // What a fast wheel did before: the driver ends believing 120 while the lens is at 180.
        let d = Driver::new(90, 90);
        d.write(180).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        d.write(120).unwrap();
        std::thread::sleep(move_time(90, 180) + Duration::from_millis(200));
        assert_eq!(d.at(), (120, 180));
    }

    #[test]
    fn moves_wait_their_turn() {
        let d = Driver::new(90, 90);
        let m = Mover::start(Box::new(d.clone()));
        m.ask(Job::To(180), 180);
        std::thread::sleep(Duration::from_millis(60));
        m.ask(Job::To(150), 150);
        m.ask(Job::To(120), 120);
        settle(&m);
        assert_eq!(d.at(), (120, 120));
    }

    #[test]
    fn seating_and_homing_find_the_stop() {
        // It lost track: it believes 10, the lens is at 22. Facing you seats it.
        let d = Driver::new(10, 22);
        let m = Mover::start(Box::new(d.clone()));
        m.ask(Job::Seat(SEAT_DEG), USER);
        settle(&m);
        assert_eq!(d.at(), (0, 0));
        // Pushed 50° off: a re-home finds the stop, then goes back.
        let d = Driver::new(90, 140);
        let m = Mover::start(Box::new(d.clone()));
        m.ask(Job::Home(90), 90);
        settle(&m);
        assert_eq!(d.at(), (90, 90));
    }
}
