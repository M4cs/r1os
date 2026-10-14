//! The R1's own settings (ADR-0045), in one place for Settings and for agents:
//!
//! - **The host's** (they're Android's): the display's brightness (and whether it's automatic), how long until it
//!   sleeps, the volume, the UI's sounds and haptics, the 24-hour clock, and docked mode (the glance screen stays on
//!   while charging). The host reports them (`POST /api/device`, on connecting and whenever they change) and applies
//!   changes r1d sends it (`device_set`).
//! - **r1d's own:** the time zone (or following the network), the model app builds use, pausing agents, the wheel's
//!   direction and what a triple press opens (Settings › Controls).
//!
//! Agents read them with `device_settings` and change them with `change_device_settings`, which asks you first unless
//! you've allowed them (crate::approvals).

use crate::hub::{Event, Hub, Source};
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

/// How long the display stays on (seconds): Settings' choices.
pub const TIMEOUTS: &[u64] = &[15, 30, 60, 120, 300, 600];
/// What the host owns and reports.
pub const HOST_KEYS: &[&str] = &["brightness", "auto_brightness", "screen_timeout", "volume", "sounds", "haptics", "haptics_wheel", "clock_24h", "docked", "bluetooth"];
/// How strong the R1's haptics are (the host's Haptics: it has no strength control, so a level picks effects).
pub const HAPTICS: &[&str] = &["off", "light", "medium", "strong"];
/// What r1d keeps itself.
pub const OWN_KEYS: &[&str] = &["time_zone", "time_auto", "build_model", "agents_paused", "invert_wheel", "triple_press"];
/// What a triple press can open (Settings › Controls).
pub const TRIPLE: &[&str] = &["off", "recorder", "camera", "journal", "timer", "whats_up"];

#[derive(Default)]
pub struct DeviceSettings {
    /// The host's last report.
    host: Mutex<Map<String, Value>>,
    /// The network's DNS servers, as the host reads them from Android (crate::link looks names up there).
    dns: Mutex<Vec<std::net::IpAddr>>,
}

impl DeviceSettings {
    /// The host's report of Android's settings.
    pub fn report(&self, v: &Map<String, Value>) {
        if let Some(list) = v.get("dns").and_then(Value::as_array) {
            *self.dns.lock().unwrap() = list.iter().filter_map(|s| s.as_str()?.parse().ok()).collect();
        }
        let mut host = self.host.lock().unwrap();
        for k in HOST_KEYS {
            if let Some(x) = v.get(*k) {
                host.insert((*k).to_owned(), x.clone());
            }
        }
    }

    /// The network's DNS servers (none until the host has said).
    pub fn dns(&self) -> Vec<std::net::IpAddr> {
        self.dns.lock().unwrap().clone()
    }

    /// All of them, now.
    pub fn all(&self, hub: &Hub) -> Map<String, Value> {
        let mut out = self.host.lock().unwrap().clone();
        out.insert("time_zone".into(), json!(crate::alarms::zone()));
        out.insert("time_auto".into(), json!(hub.core.zone_auto()));
        out.insert("build_model".into(), json!(hub.core.build_model()));
        out.insert("agents_paused".into(), json!(hub.core.agents_paused()));
        out.insert("invert_wheel".into(), json!(hub.core.invert_wheel()));
        out.insert("triple_press".into(), json!(hub.core.triple_press()));
        out
    }
}

/// A change, checked.
#[derive(Debug)]
pub struct Checked {
    /// What goes to the host.
    pub host: Map<String, Value>,
    /// What r1d does itself.
    pub own: Map<String, Value>,
    /// The change in words, for the approval card and the agent.
    pub words: Vec<String>,
}

/// Checks a change, or says why it can't be.
pub fn check(changes: &Map<String, Value>) -> Result<Checked, String> {
    let mut host = Map::new();
    let mut own = Map::new();
    let mut words = Vec::new();
    let percent = |k: &str, v: &Value| -> Result<i64, String> {
        v.as_f64().filter(|n| (0.0..=100.0).contains(n)).map(|n| n.round() as i64).ok_or_else(|| format!("{k} is a percentage, 0–100"))
    };
    let flag = |k: &str, v: &Value| v.as_bool().ok_or_else(|| format!("{k} is true or false"));
    for (k, v) in changes {
        match k.as_str() {
            "brightness" => {
                let n = percent(k, v)?;
                host.insert(k.clone(), json!(n));
                words.push(format!("brightness {n}%"));
            }
            "auto_brightness" => {
                let on = flag(k, v)?;
                host.insert(k.clone(), json!(on));
                words.push(format!("automatic brightness {}", if on { "on" } else { "off" }));
            }
            "screen_timeout" => {
                let s = v.as_u64().filter(|s| TIMEOUTS.contains(s)).ok_or_else(|| {
                    format!("screen_timeout is one of {} seconds", TIMEOUTS.iter().map(u64::to_string).collect::<Vec<_>>().join(", "))
                })?;
                host.insert(k.clone(), json!(s));
                words.push(format!("the screen sleeps after {}", duration_words(s)));
            }
            "volume" => {
                let n = percent(k, v)?;
                host.insert(k.clone(), json!(n));
                words.push(format!("volume {n}%"));
            }
            "sounds" => {
                let on = flag(k, v)?;
                host.insert(k.clone(), json!(on));
                words.push(format!("the R1's sounds {}", if on { "on" } else { "off" }));
            }
            "haptics" => {
                let lv = v.as_str().filter(|l| HAPTICS.contains(l)).ok_or_else(|| format!("haptics is one of {}", HAPTICS.join(", ")))?;
                host.insert(k.clone(), json!(lv));
                words.push(if lv == "off" { "haptics off".into() } else { format!("{lv} haptics") });
            }
            "haptics_wheel" => {
                let on = flag(k, v)?;
                host.insert(k.clone(), json!(on));
                words.push(format!("the wheel {}", if on { "ticks under your thumb" } else { "turns without a tick" }));
            }
            "clock_24h" => {
                let on = flag(k, v)?;
                host.insert(k.clone(), json!(on));
                words.push(if on { "a 24-hour clock".into() } else { "a 12-hour clock".into() });
            }
            "bluetooth" => {
                let on = flag(k, v)?;
                host.insert(k.clone(), json!(on));
                words.push(format!("bluetooth {}", if on { "on" } else { "off" }));
            }
            "docked" => {
                let on = flag(k, v)?;
                host.insert(k.clone(), json!(on));
                words.push(format!("docked mode {}", if on { "on: the glance screen stays on while charging" } else { "off" }));
            }
            "invert_wheel" => {
                let on = flag(k, v)?;
                own.insert(k.clone(), json!(on));
                words.push(if on { "the wheel turned the other way round".into() } else { "the wheel the usual way round".into() });
            }
            "triple_press" => {
                let t = v.as_str().filter(|t| TRIPLE.contains(t)).ok_or_else(|| format!("triple_press is one of {}", TRIPLE.join(", ")))?;
                own.insert(k.clone(), json!(t));
                words.push(if t == "off" { "a triple press does nothing".into() } else { format!("a triple press opens {}", t.replace('_', "'s ")) });
            }
            "time_zone" => {
                let z = v.as_str().map(str::trim).filter(|z| !z.is_empty()).ok_or("time_zone is a name like Europe/London")?;
                own.insert(k.clone(), json!(z));
                words.push(format!("the time zone {z}"));
            }
            "time_auto" => {
                let on = flag(k, v)?;
                own.insert(k.clone(), json!(on));
                words.push(if on { "the time zone follows the network".into() } else { "the time zone stays put".into() });
            }
            "build_model" => {
                let m = v.as_str().filter(|m| crate::core::BUILD_MODELS.contains(m)).ok_or_else(|| format!("build_model is one of {}", crate::core::BUILD_MODELS.join(", ")))?;
                own.insert(k.clone(), json!(m));
                words.push(format!("app builds use {m}"));
            }
            "agents_paused" => {
                let on = flag(k, v)?;
                own.insert(k.clone(), json!(on));
                words.push(if on { "agents stop waking each other".into() } else { "agents can wake each other again".into() });
            }
            other => return Err(format!("there's no setting `{other}`: they're {}, {}", HOST_KEYS.join(", "), OWN_KEYS.join(", "))),
        }
    }
    if words.is_empty() {
        return Err("give `changes`: setting → new value".into());
    }
    Ok(Checked { host, own, words })
}

/// Applies a checked change: the host's through a `device_set` event, r1d's here.
pub async fn apply(hub: &Arc<Hub>, Checked { host, own, .. }: Checked) -> anyhow::Result<()> {
    if !host.is_empty() {
        hub.device_settings.report(&host);
        hub.emit(Event::DeviceSet { changes: Value::Object(host) }, Source::Api);
    }
    for (k, v) in own {
        match k.as_str() {
            "time_zone" => crate::alarms::set_zone(hub, v.as_str().unwrap_or_default()).await?,
            "time_auto" => crate::alarms::set_zone_auto(hub, v == true).await?,
            "build_model" => hub.core.set_build_model(v.as_str().unwrap_or_default())?,
            "agents_paused" => {
                hub.core.set_agents_paused(v == true)?;
                hub.emit(Event::AgentsChanged, Source::Api);
            }
            "invert_wheel" => {
                hub.core.set_invert_wheel(v == true)?;
                hub.controls.set_invert(v == true);
            }
            "triple_press" => {
                let t = v.as_str().unwrap_or("off");
                hub.core.set_triple_press(t)?;
                hub.controls.set_triple(t);
            }
            _ => {}
        }
    }
    Ok(())
}

fn duration_words(s: u64) -> String {
    if s < 60 { format!("{s} seconds") } else if s == 60 { "a minute".into() } else { format!("{} minutes", s / 60) }
}

/// Every setting and what it can be, for the agents' tool and Settings.
pub fn options() -> Value {
    json!({
        "brightness": "0–100 (%)", "auto_brightness": "true or false", "screen_timeout": TIMEOUTS,
        "volume": "0–100 (%)", "sounds": "true or false (the R1's own clicks and ticks)",
        "haptics": HAPTICS, "haptics_wheel": "true or false (a tick from the motor as the wheel turns)",
        "clock_24h": "true or false", "docked": "true: the glance screen stays on while it charges",
        "bluetooth": "true or false (pairing and connecting are in Settings › Bluetooth on the R1)",
        "invert_wheel": "true: the wheel turns the other way round", "triple_press": TRIPLE,
        "time_zone": "a zone like Europe/London", "time_auto": "true: follow the network",
        "build_model": crate::core::BUILD_MODELS, "agents_paused": "true: agents' messages to each other wait as badges instead of starting turns",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_checked_and_said() {
        let c = json!({ "brightness": 40, "screen_timeout": 60, "build_model": "opus" });
        let Checked { host, own, words } = check(c.as_object().unwrap()).unwrap();
        assert_eq!(host.len(), 2);
        assert_eq!(own["build_model"], "opus");
        assert!(words.contains(&"the screen sleeps after a minute".to_owned()), "{words:?}");
        assert!(check(json!({ "brightness": 140 }).as_object().unwrap()).is_err());
        assert!(check(json!({ "screen_timeout": 45 }).as_object().unwrap()).is_err());
        assert!(check(json!({ "wallpaper": "x" }).as_object().unwrap()).unwrap_err().contains("no setting"));
    }
}
