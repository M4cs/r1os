//! The R1's power policy (T-037, architecture.md §7), after r1ptt's BATTERY.md: Android settings that cost battery
//! for nothing on an R1. Each change is logged.
//!
//! Two kinds (T-176), so that on anyone's R1 r1d changes only what's safe for any R1 and nothing you chose:
//!
//! - **r1os's own, put right at every start** ([OWNED]): settings Android's own screens don't offer, or that mean
//!   nothing on an R1.
//!   - No network recommendations; mobile data isn't kept up beside Wi-Fi.
//!   - Airplane mode is the cell radio alone, so turning it on leaves Wi-Fi and Bluetooth as they are.
//!   - **Doze sooner:** light idle as the screen goes off, deep idle 30 s after (Android waits half an hour or more,
//!     and for the R1 to lie still). The host is on Doze's list, so its mic service and its link to r1d go on in deep
//!     idle; r1d, the agent box and the kernel alarm r1d wakes by aren't apps, so Doze doesn't hold them.
//!   - **No doze display:** the screen going off is plain sleep, not SystemUI's DozeService (whose pulses r1os
//!     doesn't use, and whose window turned a docked R1 upright as it slept). The panel can't show anything dozing
//!     anyway: it runs in video mode (T-159).
//! - **Defaults, put once** ([DEFAULTS]): settings you can change in Android's own Settings. r1d puts each once, then
//!   leaves it as you set it (core.db's `power_defaults` remembers which).
//!   - **No location:** nothing in r1os asks for it (the time zone comes from the network, ADR-0030). An Android app
//!     that needs it can have it turned on again in Android's settings, and it stays on.
//!   - **No background scanning:** Wi-Fi doesn't turn itself on near saved networks, nor scan (Wi-Fi or Bluetooth)
//!     for location while it's off.
//!   - **The cellular radio off while there's no SIM** (it searches for a network all the same): airplane mode on, once
//!     for each spell without a SIM. Turn it off and it stays off; put a SIM in and take it out again, and it's put on
//!     once more. With a SIM nothing changes. Only `ABSENT` in every slot is "no SIM": `NOT_READY` is what a SIM reads
//!     before the modem has loaded it at boot, so it's "don't know", and the radio is left alone.
//!
//! Nothing here takes a radio down while it's in use: airplane mode goes on only with no SIM (so no data, and the R1
//! makes no calls), and only once it's read back as the cell radio alone (2026-10-12, when r1d couldn't reach the
//! settings service, an airplane mode with Wi-Fi in it took Wi-Fi down for a minute).

use std::collections::BTreeSet;
use std::time::Duration;

/// r1os's own: `settings put <namespace> <key> <value>`, put right at every start.
pub const OWNED: &[(&str, &str, &str)] = &[
    ("global", "network_recommendations_enabled", "0"),
    ("global", "mobile_data_always_on", "0"),
    ("global", "airplane_mode_radios", "cell,nfc,wimax,uwb"),
    ("secure", "doze_enabled", "0"),
];

/// Defaults: each put once, then yours.
pub const DEFAULTS: &[(&str, &str, &str)] = &[
    ("secure", "location_mode", "0"),
    ("global", "wifi_scan_always_enabled", "0"),
    ("global", "ble_scan_always_enabled", "0"),
    ("global", "wifi_wakeup_enabled", "0"),
];

/// Doze's timings (`device_config put device_idle`), in ms: r1os's own.
pub const DOZE: &[(&str, &str)] = &[
    ("light_after_inactive_to", "0"),
    ("inactive_to", "30000"),
    ("sensing_to", "0"),
    ("locating_to", "0"),
    ("motion_inactive_to", "0"),
    ("idle_after_inactive_to", "0"),
];

/// [Done]'s mark for the cell radio's default, while there's no SIM.
const NO_SIM: &str = "airplane_mode_on:no_sim";

/// Which [DEFAULTS] (by key, and [NO_SIM]) r1d has put in place, or found in place: those are yours from then on.
pub type Done = BTreeSet<String>;

const HOST: &str = "dev.r1os.host";
const DEADLINE: Duration = Duration::from_secs(6);
/// How long r1d waits at start for the modem to say whether there's a SIM, when airplane mode is off.
const SIM_WAIT: Duration = Duration::from_secs(60);
const SIM_POLL: Duration = Duration::from_secs(5);

/// Puts the policy in place: what changed, for the log.
pub async fn apply(core: &crate::core::Core) -> Vec<String> {
    let mut changed = Vec::new();
    if !cfg!(target_os = "android") {
        return changed;
    }
    for (ns, key, want) in OWNED {
        let now = out("settings", &["get", ns, key]).await;
        if now.as_deref() != Some(*want) && put(&["settings", "put", ns, key, want]).await {
            changed.push(format!("{ns} {key}: {} → {want}", now.unwrap_or_default()));
        }
    }
    let doze = out("device_config", &["list", "device_idle"]).await.unwrap_or_default();
    for (key, want) in DOZE {
        let now = doze.lines().find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_owned));
        if now.as_deref() != Some(*want) && put(&["device_config", "put", "device_idle", key, want]).await {
            changed.push(format!("device_idle {key}: {} → {want}", now.unwrap_or_default()));
        }
    }
    let listed = out("cmd", &["deviceidle", "whitelist"]).await.unwrap_or_default();
    if !listed.lines().any(|l| l.split(',').nth(1) == Some(HOST)) && put(&["cmd", "deviceidle", "whitelist", &format!("+{HOST}")]).await {
        changed.push(format!("deviceidle whitelist: +{HOST}"));
    }

    let mut done = core.power_defaults();
    let before = done.clone();
    for (ns, key, want) in DEFAULTS {
        if done.contains(*key) {
            continue;
        }
        // Unreadable is "don't know": tried again next start.
        let Some(now) = out("settings", &["get", ns, key]).await else { continue };
        if now == *want {
            done.insert((*key).to_owned());
        } else if put(&["settings", "put", ns, key, want]).await {
            changed.push(format!("{ns} {key}: {now} → {want} (once: yours from now on)"));
            done.insert((*key).to_owned());
        }
    }

    let airplane = out("settings", &["get", "global", "airplane_mode_on"]).await;
    let mut sim = sim(&out("getprop", &["gsm.sim.state"]).await.unwrap_or_default());
    // At boot the modem may not have read the SIM yet: wait a little for it, only while it could matter.
    let mut waited = Duration::ZERO;
    while sim == Sim::Unknown && airplane.as_deref() == Some("0") && !done.contains(NO_SIM) && waited < SIM_WAIT {
        tokio::time::sleep(SIM_POLL).await;
        waited += SIM_POLL;
        sim = self::sim(&out("getprop", &["gsm.sim.state"]).await.unwrap_or_default());
    }
    let radios = out("settings", &["get", "global", "airplane_mode_radios"]).await;
    match cell_radio(sim, airplane.as_deref(), cell_only(radios.as_deref()), done.contains(NO_SIM)) {
        Radio::Leave => {}
        Radio::Mark => {
            done.insert(NO_SIM.to_owned());
        }
        Radio::Forget => {
            done.remove(NO_SIM);
        }
        Radio::Off => {
            if put(&["cmd", "connectivity", "airplane-mode", "enable"]).await {
                changed.push("the cellular radio: off (no SIM; once: yours from now on)".to_owned());
                done.insert(NO_SIM.to_owned());
            }
        }
    }
    if done != before
        && let Err(e) = core.set_power_defaults(&done)
    {
        tracing::warn!(error = %format!("{e:#}"), "power policy: couldn't remember its defaults");
    }

    if changed.is_empty() {
        tracing::info!("power policy: in place");
    } else {
        tracing::info!(changed = ?changed, "power policy: put right");
    }
    changed
}

/// Whether there's a SIM, from `gsm.sim.state` (one value per slot, comma-separated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sim {
    /// `ABSENT` in every slot.
    Absent,
    /// A card in some slot: loaded, ready, locked or faulty.
    Present,
    /// Unreadable, or `NOT_READY`/`UNKNOWN` (the modem hasn't said yet).
    Unknown,
}

fn sim(state: &str) -> Sim {
    let slots: Vec<&str> = state.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if slots.is_empty() {
        Sim::Unknown
    } else if slots.iter().all(|s| *s == "ABSENT") {
        Sim::Absent
    } else if slots.iter().any(|s| !matches!(*s, "ABSENT" | "NOT_READY" | "UNKNOWN")) {
        Sim::Present
    } else {
        Sim::Unknown
    }
}

/// Whether airplane mode, as read back, takes only the cell radio (and not Wi-Fi or Bluetooth).
fn cell_only(radios: Option<&str>) -> bool {
    radios.is_some_and(|r| r.split(',').any(|x| x == "cell") && !r.split(',').any(|x| x == "wifi" || x == "bluetooth"))
}

/// What to do about the cell radio's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Radio {
    Leave,
    /// It's in place already: remember that, so it's yours from now on.
    Mark,
    /// A SIM: the next spell without one starts over.
    Forget,
    /// Airplane mode on.
    Off,
}

fn cell_radio(sim: Sim, airplane: Option<&str>, cell_only: bool, done: bool) -> Radio {
    match sim {
        Sim::Present if done => Radio::Forget,
        Sim::Absent if !done && airplane == Some("1") => Radio::Mark,
        Sim::Absent if !done && airplane == Some("0") && cell_only => Radio::Off,
        _ => Radio::Leave,
    }
}

/// A tool's trimmed stdout, if it ran.
async fn out(program: &str, args: &[&str]) -> Option<String> {
    match crate::tool::run(program, args, DEADLINE).await {
        Ok(o) if o.ok => Some(o.stdout.trim().to_owned()),
        Ok(o) => {
            tracing::warn!(program, ?args, error = %o.stderr.trim(), "power policy: couldn't read");
            None
        }
        Err(e) => {
            tracing::warn!(program, ?args, error = %e, "power policy: couldn't read");
            None
        }
    }
}

async fn put(cmd: &[&str]) -> bool {
    match crate::tool::run(cmd[0], &cmd[1..], DEADLINE).await {
        Ok(o) if o.ok => true,
        Ok(o) => {
            tracing::warn!(?cmd, error = %format!("{}{}", o.stdout, o.stderr).trim(), "power policy: couldn't set");
            false
        }
        Err(e) => {
            tracing::warn!(?cmd, error = %e, "power policy: couldn't set");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn airplane_mode_leaves_wifi_and_bluetooth_alone() {
        let radios = OWNED.iter().find(|(_, k, _)| *k == "airplane_mode_radios").unwrap().2;
        assert!(cell_only(Some(radios)), "{radios}");
        assert!(!cell_only(Some("cell,bluetooth,wifi,nfc")));
        assert!(!cell_only(None));
    }

    #[test]
    fn what_you_can_change_is_put_once() {
        // Android's Settings offers these: r1d mustn't take them back at every start.
        for key in ["location_mode", "wifi_scan_always_enabled", "ble_scan_always_enabled", "wifi_wakeup_enabled"] {
            assert!(DEFAULTS.iter().any(|(_, k, _)| *k == key), "{key} isn't a default");
            assert!(!OWNED.iter().any(|(_, k, _)| *k == key), "{key} is put at every start");
        }
    }

    #[test]
    fn only_absent_is_no_sim() {
        assert_eq!(sim("ABSENT"), Sim::Absent);
        assert_eq!(sim("ABSENT,ABSENT"), Sim::Absent);
        // At boot, before the modem has read the card.
        assert_eq!(sim("NOT_READY"), Sim::Unknown);
        assert_eq!(sim("ABSENT,NOT_READY"), Sim::Unknown);
        assert_eq!(sim(""), Sim::Unknown);
        assert_eq!(sim("LOADED"), Sim::Present);
        assert_eq!(sim("ABSENT,READY"), Sim::Present);
        assert_eq!(sim("PIN_REQUIRED"), Sim::Present);
    }

    #[test]
    fn the_cell_radio_goes_off_once_without_a_sim() {
        // No SIM, radio on: off, once.
        assert_eq!(cell_radio(Sim::Absent, Some("0"), true, false), Radio::Off);
        // Turned back on by you: left.
        assert_eq!(cell_radio(Sim::Absent, Some("0"), true, true), Radio::Leave);
        // Already off: remembered, not touched.
        assert_eq!(cell_radio(Sim::Absent, Some("1"), true, false), Radio::Mark);
        // Never while airplane mode would take Wi-Fi or Bluetooth too, or it couldn't be read.
        assert_eq!(cell_radio(Sim::Absent, Some("0"), false, false), Radio::Leave);
        assert_eq!(cell_radio(Sim::Absent, None, true, false), Radio::Leave);
        // A SIM still loading at boot: left alone.
        assert_eq!(cell_radio(Sim::Unknown, Some("0"), true, false), Radio::Leave);
        // A SIM: never touched, and the next spell without one starts over.
        assert_eq!(cell_radio(Sim::Present, Some("0"), true, false), Radio::Leave);
        assert_eq!(cell_radio(Sim::Present, Some("1"), true, true), Radio::Forget);
    }
}
