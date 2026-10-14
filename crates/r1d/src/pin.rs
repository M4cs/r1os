//! The PIN lock (ADR-0016, Q28; roadmap R6): a 4-digit code dialled on the wheel, like a safe's. r1d keeps it as a
//! salted hash in core.db and decides whether the R1 is locked: it locks as the display goes off whenever a PIN is set
//! (and when r1d starts), and only the code unlocks it. While it's locked the mic doesn't open for talking
//! (crate::voice), and the shell's glance screen shows the dial instead of going home.
//!
//! - **Guessing:** five wrong codes are free; each one after that waits, 30 s and doubling up to 15 minutes. The count
//!   is kept in core.db, so restarting r1d or the R1 doesn't buy more tries.
//! - **Only on the R1:** set, changed and turned off in Settings › Lock, each with the current code. Not from a linked
//!   computer (crate::link's allowlist leaves `/api/lock` out) nor by an agent (there's no tool).
//! - **10,000 codes:** a lock against someone picking the R1 up, not a vault (ADR-0016). The hash is PBKDF2-SHA256 with
//!   a random salt, so the code itself is never stored.
//! - **Lock after** (Max, 2026-10-11: "auto unlock for X amount of time after locking… it will re-lock after for example
//!   10 minutes"): a grace, so a quick sleep and wake doesn't ask. It locks once the display has been off that long
//!   ([GRACES]; right away by default), counted on the wall clock, so a sleeping CPU doesn't stretch it. Waking within
//!   it starts over at the next sleep.

use crate::core::Core;
use crate::hub::{Event, Hub, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Mutex;

/// PBKDF2's rounds: ~0.1 s on the R1, so a check is quick and a stolen hash is slow to try every code against.
const ROUNDS: u32 = 50_000;
/// Wrong codes that don't wait.
const FREE_TRIES: u32 = 5;
const FIRST_WAIT_S: i64 = 30;
const LONGEST_WAIT_S: i64 = 15 * 60;

/// A PIN as kept: its salt, its hash (hex) and the rounds that made it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hashed {
    salt: String,
    hash: String,
    rounds: u32,
}

/// Wrong codes since the last right one, and until when the next try waits (ms; 0: it doesn't).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tries {
    pub wrong: u32,
    pub until: i64,
}

/// Why a code wasn't taken (the dial says it).
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not the code; the next try waits this long.
    Wrong { wait_s: i64 },
    /// Too many wrong ones: no tries for this long.
    Wait { wait_s: i64 },
    NotSet,
    /// A new PIN that isn't 4 digits.
    Malformed,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Wrong { wait_s: 0 } => write!(f, "wrong code"),
            Refused::Wrong { wait_s } => write!(f, "wrong code · wait {}", wait(*wait_s)),
            Refused::Wait { wait_s } => write!(f, "too many tries · wait {}", wait(*wait_s)),
            Refused::NotSet => write!(f, "there's no PIN set"),
            Refused::Malformed => write!(f, "a PIN is 4 digits"),
        }
    }
}

/// "30 s", "2 min".
fn wait(s: i64) -> String {
    if s < 60 { format!("{s} s") } else { format!("{} min", (s + 59) / 60) }
}

/// "Lock after": how long the display may be off before the R1 locks (s). 0: as it goes off.
pub const GRACES: &[i64] = &[0, 60, 300, 600, 900, 1800, 3600];

/// Whether the R1 is locked (r1d's own; the shell follows the `lock` events).
pub struct Pin {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    locked: bool,
    /// When the display went off, while a grace runs (ms).
    off_at: Option<i64>,
    grace_s: i64,
}

impl Pin {
    /// Locked from the start when a PIN is set: r1d (or the R1) restarting doesn't open it.
    pub fn new(core: &Core) -> Self {
        Pin { state: Mutex::new(State { locked: core.pin().is_some(), off_at: None, grace_s: core.pin_grace_s() }) }
    }

    /// Locked now: locked, or off longer than the grace (the display coming back on before r1d has heard).
    pub fn locked(&self) -> bool {
        self.locked_at(crate::alarms::clock().now_ms())
    }

    fn locked_at(&self, now: i64) -> bool {
        let s = self.state.lock().unwrap();
        s.locked || s.off_at.is_some_and(|t| now - t >= s.grace_s * 1000)
    }

    /// `{set, locked, wait_s, grace_s}` for the shell.
    pub fn view(&self, core: &Core, now: i64) -> Value {
        let t = core.pin_tries();
        json!({
            "set": core.pin().is_some(),
            "locked": self.locked_at(now),
            "wait_s": ((t.until - now).max(0) + 999) / 1000,
            "grace_s": self.state.lock().unwrap().grace_s,
        })
    }

    /// The display went off: locked at once with a PIN set, or (with a grace) once it's been off that long.
    pub fn display_off(&self, hub: &Hub, now: i64) {
        if hub.core.pin().is_some() && self.went_off(now) {
            tracing::info!("pin: locked");
            hub.emit(Event::Lock { set: true, locked: true }, Source::Api);
        }
    }

    /// The display came on: locked if it was off longer than the grace.
    pub fn display_on(&self, hub: &Hub, now: i64) {
        if self.came_on(now) {
            tracing::info!("pin: locked (the grace ran out)");
            hub.emit(Event::Lock { set: true, locked: true }, Source::Api);
        }
    }

    /// Whether it locked now.
    fn went_off(&self, now: i64) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.locked {
            return false;
        }
        if s.grace_s == 0 {
            s.locked = true;
            return true;
        }
        s.off_at.get_or_insert(now);
        false
    }

    /// Whether it locked now.
    fn came_on(&self, now: i64) -> bool {
        let mut s = self.state.lock().unwrap();
        let Some(t) = s.off_at.take() else { return false };
        if !s.locked && now - t >= s.grace_s * 1000 {
            s.locked = true;
            return true;
        }
        false
    }

    /// How long the display may be off before it locks (one of [GRACES]).
    pub fn set_grace(&self, hub: &Hub, grace_s: i64) -> anyhow::Result<()> {
        anyhow::ensure!(GRACES.contains(&grace_s), "lock after: one of {GRACES:?} seconds");
        hub.core.set_pin_grace_s(grace_s)?;
        self.state.lock().unwrap().grace_s = grace_s;
        hub.emit(Event::Lock { set: hub.core.pin().is_some(), locked: self.locked() }, Source::Api);
        Ok(())
    }

    fn open(&self) {
        let mut s = self.state.lock().unwrap();
        s.locked = false;
        s.off_at = None;
    }

    /// The right code: unlocked.
    pub fn unlock(&self, hub: &Hub, code: &str, now: i64) -> Result<(), Refused> {
        check(&hub.core, code, now)?;
        self.open();
        tracing::info!("pin: unlocked");
        hub.emit(Event::Lock { set: true, locked: false }, Source::Api);
        Ok(())
    }

    /// A new PIN (`current`: the one set, if any).
    pub fn set(&self, hub: &Hub, code: &str, current: Option<&str>, now: i64) -> Result<(), Refused> {
        set(&hub.core, code, current, now)?;
        hub.emit(Event::Lock { set: true, locked: self.locked() }, Source::Api);
        Ok(())
    }

    /// No PIN any more (with the one set): unlocked, and it stays so.
    pub fn off(&self, hub: &Hub, current: &str, now: i64) -> Result<(), Refused> {
        check(&hub.core, current, now)?;
        hub.core.set_pin(None).map_err(|_| Refused::NotSet)?;
        self.open();
        hub.emit(Event::Lock { set: false, locked: false }, Source::Api);
        Ok(())
    }
}

/// Checks `code` against the PIN, counting a wrong one (and refusing while a wait is on).
pub fn check(core: &Core, code: &str, now: i64) -> Result<(), Refused> {
    let Some(h) = core.pin() else { return Err(Refused::NotSet) };
    let mut t = core.pin_tries();
    if t.until > now {
        return Err(Refused::Wait { wait_s: (t.until - now + 999) / 1000 });
    }
    if !(valid(code) && same(&hash(code, &h.salt, h.rounds), &h.hash)) {
        t.wrong += 1;
        let w = wait_after(t.wrong);
        t.until = if w > 0 { now + w * 1000 } else { 0 };
        let _ = core.set_pin_tries(t);
        tracing::info!(wrong = t.wrong, wait_s = w, "pin: a wrong code");
        return Err(Refused::Wrong { wait_s: w });
    }
    if t != Tries::default() {
        let _ = core.set_pin_tries(Tries::default());
    }
    Ok(())
}

/// A new PIN: with the current one when one is set.
pub fn set(core: &Core, code: &str, current: Option<&str>, now: i64) -> Result<(), Refused> {
    if !valid(code) {
        return Err(Refused::Malformed);
    }
    if core.pin().is_some() {
        check(core, current.unwrap_or_default(), now)?;
    }
    let salt = crate::core::random_hex(16);
    let h = Hashed { hash: hash(code, &salt, ROUNDS), salt, rounds: ROUNDS };
    core.set_pin(Some(&h)).map_err(|_| Refused::NotSet)?;
    let _ = core.set_pin_tries(Tries::default());
    tracing::info!("pin: set");
    Ok(())
}

/// Four digits.
fn valid(code: &str) -> bool {
    code.len() == 4 && code.bytes().all(|b| b.is_ascii_digit())
}

/// How long the next try waits after `wrong` wrong codes (s).
fn wait_after(wrong: u32) -> i64 {
    if wrong < FREE_TRIES { 0 } else { (FIRST_WAIT_S << (wrong - FREE_TRIES).min(10)).min(LONGEST_WAIT_S) }
}

fn hash(code: &str, salt: &str, rounds: u32) -> String {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(code.as_bytes(), salt.as_bytes(), rounds, &mut out);
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// Equal, in the same time whatever the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> Core {
        Core::open(&std::env::temp_dir().join(format!("r1d-pin-{}", crate::core::random_hex(8)))).unwrap()
    }

    const NOW: i64 = 1_791_700_000_000;

    #[test]
    fn a_pin_is_set_checked_and_changed() {
        let core = core();
        assert_eq!(check(&core, "1234", NOW), Err(Refused::NotSet));
        assert_eq!(set(&core, "12a4", None, NOW), Err(Refused::Malformed));
        assert_eq!(set(&core, "123", None, NOW), Err(Refused::Malformed));
        set(&core, "0420", None, NOW).unwrap();
        assert!(!serde_json::to_string(&core.pin()).unwrap().contains("0420"), "never the code itself");
        assert_eq!(check(&core, "0420", NOW), Ok(()));
        // Changing it takes the current one.
        assert_eq!(set(&core, "9999", Some("1111"), NOW), Err(Refused::Wrong { wait_s: 0 }));
        set(&core, "9999", Some("0420"), NOW).unwrap();
        assert_eq!(check(&core, "0420", NOW), Err(Refused::Wrong { wait_s: 0 }));
        assert_eq!(check(&core, "9999", NOW), Ok(()));
        assert_eq!(core.pin_tries(), Tries::default(), "a right code clears the count");
    }

    #[test]
    fn wrong_codes_wait_longer_and_longer() {
        let core = core();
        set(&core, "2468", None, NOW).unwrap();
        for _ in 0..4 {
            assert_eq!(check(&core, "0000", NOW), Err(Refused::Wrong { wait_s: 0 }));
        }
        assert_eq!(check(&core, "0000", NOW), Err(Refused::Wrong { wait_s: 30 }));
        // Waiting: even the right code isn't tried.
        assert_eq!(check(&core, "2468", NOW + 10_000), Err(Refused::Wait { wait_s: 20 }));
        assert_eq!(check(&core, "1111", NOW + 30_000), Err(Refused::Wrong { wait_s: 60 }));
        assert_eq!(wait_after(9), 480);
        assert_eq!(wait_after(30), LONGEST_WAIT_S);
        // A fresh start of r1d keeps the count (core.db).
        assert_eq!(core.pin_tries().wrong, 6);
        assert_eq!(check(&core, "2468", NOW + 90_000), Ok(()));
        assert_eq!(core.pin_tries(), Tries::default());
        assert_eq!(Refused::Wait { wait_s: 125 }.to_string(), "too many tries · wait 3 min");
    }

    #[test]
    fn a_grace_keeps_it_open_for_a_while() {
        let pin = |grace_s| Pin { state: Mutex::new(State { locked: false, off_at: None, grace_s }) };
        // Right away (the default): locked as the display goes off.
        let p = pin(0);
        assert!(p.went_off(NOW));
        assert!(p.locked_at(NOW));
        // Ten minutes: a quick sleep and wake stays open, and the next sleep starts over.
        let p = pin(600);
        assert!(!p.went_off(NOW));
        assert!(!p.locked_at(NOW + 5 * 60_000));
        assert!(!p.came_on(NOW + 5 * 60_000));
        assert!(!p.went_off(NOW + 6 * 60_000));
        assert!(!p.locked_at(NOW + 15 * 60_000), "ten minutes from the second sleep, not the first");
        // Off longer than that: locked as it wakes, even before r1d hears the display came on.
        assert!(p.locked_at(NOW + 17 * 60_000));
        assert!(p.came_on(NOW + 17 * 60_000));
        assert!(p.locked_at(NOW + 17 * 60_000));
        p.open();
        assert!(!p.locked_at(NOW + 18 * 60_000));
    }
}
