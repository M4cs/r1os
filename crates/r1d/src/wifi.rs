//! Wi-Fi for onboarding and Settings, through Android's `cmd wifi` (r1d runs as root).

use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rssi: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Network {
    pub ssid: String,
    pub rssi: i32,
    /// open · wpa2 · wpa3 · owe · wep (what `connect-network` takes)
    pub security: String,
    /// 0–4 bars.
    pub bars: u8,
}

async fn cmd(args: &[&str]) -> anyhow::Result<String> {
    if !cfg!(target_os = "android") {
        anyhow::bail!("Wi-Fi is only managed on the R1");
    }
    let mut all = vec!["wifi"];
    all.extend_from_slice(args);
    let out = crate::tool::run("cmd", &all, crate::tool::DEADLINE).await?;
    Ok(out.stdout + &out.stderr)
}

pub async fn status() -> anyhow::Result<Status> {
    let text = cmd(&["status"]).await?;
    Ok(parse_status(&text))
}

fn parse_status(text: &str) -> Status {
    let enabled = text.contains("Wifi is enabled");
    let ssid = text
        .lines()
        .find_map(|l| l.strip_prefix("Wifi is connected to \"").and_then(|r| r.strip_suffix('"')))
        .map(str::to_owned);
    let ip = text.split("IP: /").nth(1).and_then(|r| r.split(',').next()).map(|s| s.trim().to_owned());
    let rssi = text.split("RSSI: ").nth(1).and_then(|r| r.split(',').next()).and_then(|s| s.trim().parse().ok());
    Status { enabled, ip: ssid.as_ref().and(ip), ssid, rssi }
}

pub async fn scan() -> anyhow::Result<()> {
    cmd(&["start-scan"]).await.map(|_| ())
}

/// Nearby networks, strongest first, one entry per SSID (hidden ones skipped).
pub async fn networks() -> anyhow::Result<Vec<Network>> {
    Ok(parse_scan(&cmd(&["list-scan-results"]).await?))
}

fn parse_scan(text: &str) -> Vec<Network> {
    let mut best: BTreeMap<String, Network> = BTreeMap::new();
    for line in text.lines().skip(1) {
        // BSSID  Frequency  RSSI  Age(sec)  SSID…  Flags; the SSID may contain spaces, flags start with '['.
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 {
            continue;
        }
        let Ok(rssi) = cols[2].parse::<i32>() else { continue };
        let flags_at = cols.iter().position(|c| c.starts_with('[')).unwrap_or(cols.len());
        if flags_at <= 4 {
            continue; // hidden network
        }
        let ssid = cols[4..flags_at].join(" ");
        let flags = cols[flags_at..].join("");
        let security = if flags.contains("SAE") && !flags.contains("PSK-CCMP]") && !flags.contains("WPA2") {
            "wpa3"
        } else if flags.contains("PSK") || flags.contains("WPA") {
            "wpa2"
        } else if flags.contains("OWE") {
            "owe"
        } else if flags.contains("WEP") {
            "wep"
        } else {
            "open"
        };
        let bars = match rssi {
            r if r >= -55 => 4,
            r if r >= -66 => 3,
            r if r >= -77 => 2,
            r if r >= -88 => 1,
            _ => 0,
        };
        let n = Network { ssid: ssid.clone(), rssi, security: security.into(), bars };
        if best.get(&ssid).is_none_or(|b| b.rssi < rssi) {
            best.insert(ssid, n);
        }
    }
    let mut v: Vec<Network> = best.into_values().collect();
    v.sort_by_key(|n| std::cmp::Reverse(n.rssi));
    v
}

/// Connects (and saves) a network. The password goes straight to Android's Wi-Fi service and
/// isn't logged or stored by r1d.
pub async fn connect(ssid: &str, security: &str, password: Option<&str>) -> anyhow::Result<()> {
    anyhow::ensure!(!ssid.is_empty() && ssid.len() <= 32, "bad network name");
    anyhow::ensure!(["open", "owe", "wpa2", "wpa3", "wep"].contains(&security), "bad security type");
    let mut args = vec!["connect-network", ssid, security];
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        args.push(p);
    }
    let out = cmd(&args).await?;
    if out.to_lowercase().contains("fail") || out.to_lowercase().contains("error") {
        anyhow::bail!("Android couldn't connect: {}", out.lines().next().unwrap_or("").trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status() {
        let s = parse_status(
            "Wifi is enabled\nWifi is connected to \"Home Net\"\nWifiInfo: SSID: \"Home Net\", IP: /192.168.1.218, Security type: 4, RSSI: -60, Link",
        );
        assert!(s.enabled);
        assert_eq!(s.ssid.as_deref(), Some("Home Net"));
        assert_eq!(s.ip.as_deref(), Some("192.168.1.218"));
        assert_eq!(s.rssi, Some(-60));
    }

    #[test]
    fn parses_scan_results() {
        let t = "    BSSID              Frequency      RSSI           Age(sec)     SSID                                 Flags
  aa:bb:cc:dd:ee:01       5240        -59            >1000.0    Home Net                    [WPA2-PSK-CCMP][RSN-PSK+SAE-CCMP][ESS]
  aa:bb:cc:dd:ee:02       5500        -84            >1000.0                                      [WPA2-PSK-CCMP][ESS]
  aa:bb:cc:dd:ee:03       2437        -46            >1000.0    Home Net                    [WPA2-PSK-CCMP][ESS]
  aa:bb:cc:dd:ee:04       2462        -77            >1000.0    Cafe                    [ESS]";
        let n = parse_scan(t);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].ssid, "Home Net");
        assert_eq!(n[0].rssi, -46);
        assert_eq!(n[0].security, "wpa2");
        assert_eq!(n[1].security, "open");
    }
}
