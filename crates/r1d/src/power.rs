//! The R1's power policy (T-037, architecture.md §7), after r1ptt's BATTERY.md: Android settings that cost battery
//! for nothing on an R1, put right at every start, changing only what differs (each change is logged).
//!
//! - **No background scanning:** Wi-Fi doesn't turn itself on near saved networks, nor scan (Wi-Fi or Bluetooth) for
//!   location while it's off; no network recommendations.
//! - **No location:** nothing in r1os asks for it (the time zone comes from the network, ADR-0030). An Android app
//!   that needs it can have it turned on again in Android's settings.
//! - **The cellular radio off while there's no SIM:** airplane mode is the cell radio alone (Wi-Fi and Bluetooth stay
//!   as they are), on at start when no SIM is in. With a SIM, nothing changes; mobile data isn't kept up beside Wi-Fi.
//! - **Doze sooner:** light idle as the screen goes off, deep idle 30 s after (Android waits half an hour or more, and
//!   for the R1 to lie still). The host is on Doze's list, so its mic service and its link to r1d go on in deep idle;
//!   r1d, the agent box and the kernel alarm r1d wakes by aren't apps, so Doze doesn't hold them.
//! - **No doze display:** the screen going off is plain sleep, not SystemUI's DozeService (whose pulses r1os doesn't
//!   use, and whose window turned a docked R1 upright as it slept). The panel can't show anything dozing anyway: it
//!   runs in video mode (T-159).

use std::time::Duration;

/// `settings put <namespace> <key> <value>`.
pub const SETTINGS: &[(&str, &str, &str)] = &[
    ("global", "wifi_wakeup_enabled", "0"),
    ("global", "wifi_scan_always_enabled", "0"),
    ("global", "ble_scan_always_enabled", "0"),
    ("global", "network_recommendations_enabled", "0"),
    ("global", "mobile_data_always_on", "0"),
    ("global", "airplane_mode_radios", "cell,nfc,wimax,uwb"),
    ("secure", "location_mode", "0"),
    ("secure", "doze_enabled", "0"),
];

/// Doze's timings (`device_config put device_idle`), in ms.
pub const DOZE: &[(&str, &str)] = &[
    ("light_after_inactive_to", "0"),
    ("inactive_to", "30000"),
    ("sensing_to", "0"),
    ("locating_to", "0"),
    ("motion_inactive_to", "0"),
    ("idle_after_inactive_to", "0"),
];

const HOST: &str = "dev.r1os.host";
const DEADLINE: Duration = Duration::from_secs(6);

/// Puts the policy in place: what changed, for the log and `/harness/power`.
pub async fn apply() -> Vec<String> {
    let mut changed = Vec::new();
    if !cfg!(target_os = "android") {
        return changed;
    }
    for (ns, key, want) in SETTINGS {
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
    // The cellular radio: off while there's no SIM (it searches for a network all the same).
    let sim = out("getprop", &["gsm.sim.state"]).await.unwrap_or_default();
    // Unreadable (empty) is "don't know": the radio's left alone.
    let no_sim = !sim.trim().is_empty() && sim.split(',').all(|s| matches!(s.trim(), "ABSENT" | "NOT_READY"));
    let airplane = out("settings", &["get", "global", "airplane_mode_on"]).await;
    // Only once airplane mode is read back as the cell radio alone: otherwise it would turn Wi-Fi and Bluetooth off too
    // (2026-10-12, when r1d couldn't reach the settings service: Wi-Fi went off for a minute).
    let radios = out("settings", &["get", "global", "airplane_mode_radios"]).await;
    let cell_only = radios.as_deref().is_some_and(|r| r.split(',').any(|x| x == "cell") && !r.split(',').any(|x| x == "wifi" || x == "bluetooth"));
    if no_sim && cell_only && airplane.as_deref() == Some("0") && put(&["cmd", "connectivity", "airplane-mode", "enable"]).await {
        changed.push("the cellular radio: off (no SIM)".to_owned());
    }
    if changed.is_empty() {
        tracing::info!("power policy: in place");
    } else {
        tracing::info!(changed = ?changed, "power policy: put right");
    }
    changed
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
        let radios = SETTINGS.iter().find(|(_, k, _)| *k == "airplane_mode_radios").unwrap().2;
        assert!(radios.split(',').any(|r| r == "cell"));
        assert!(!radios.split(',').any(|r| r == "wifi" || r == "bluetooth"), "{radios}");
    }
}
