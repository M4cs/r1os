//! r1os's own screens, by name (Max, 2026-10-11: "all default system apps [should] have handlers and APIs to deep link
//! open into by the agents and they should be supported by system one"). Agents open them with `open_screen`; System
//! One does for "open the translator for English to Portuguese", "open bluetooth", "show me my photos". The shell
//! opens what r1d sends (`open_screen {target}`), as its own targets: a tool, a Settings page, or the camera.

use serde_json::{Value, json};

/// One screen: its id, how the shell opens it, and the names it's asked for by.
pub struct Screen {
    pub id: &'static str,
    /// The shell's target kind: `tool`, `settings` or `camera`.
    kind: &'static str,
    /// What it's called, spoken ("the translator", "wi-fi"), lowercase.
    pub names: &'static [&'static str],
    pub about: &'static str,
}

/// What can be pinned to home as `tool:<id>` (T-091): r1os's tools, the camera and your browser, and the shell's own
/// Inputs and System.
pub fn pinnable(id: &str) -> bool {
    SCREENS.iter().any(|s| s.id == id && matches!(s.kind, "tool" | "camera" | "browser")) || matches!(id, "inputs" | "system")
}

pub const SCREENS: &[Screen] = &[
    Screen { id: "camera", kind: "camera", names: &["camera", "the camera"], about: "the camera, to take a photo" },
    Screen { id: "photos", kind: "tool", names: &["photos", "my photos", "the photos", "gallery", "my pictures", "pictures"], about: "the R1's photos" },
    Screen { id: "recorder", kind: "tool", names: &["recorder", "the recorder", "voice recorder"], about: "the recorder (start_recording records)" },
    Screen { id: "journal", kind: "tool", names: &["journal", "my journal", "the journal", "diary"], about: "the journal" },
    Screen { id: "translator", kind: "tool", names: &["translator", "the translator", "translate"], about: "the Translator, with `mine` (the user's language) and `theirs` (the other one) as codes" },
    Screen { id: "alarms", kind: "tool", names: &["alarms", "my alarms", "the alarms"], about: "alarms" },
    Screen { id: "reminders", kind: "tool", names: &["reminders", "my reminders", "the reminders", "to-do list", "my to-do list", "to do list", "my to do list", "todo list", "my todo list", "my to-dos", "my todos"], about: "reminders and the to-do list (set_reminder adds one; with no time it's a to-do)" },
    Screen { id: "timer", kind: "tool", names: &["timer", "timers", "the timer", "my timers", "pomodoro"], about: "timers (set_timer starts one)" },
    Screen { id: "routines", kind: "tool", names: &["routines", "my routines"], about: "routines (set_routine makes one)" },
    Screen { id: "whats_up", kind: "tool", names: &["what's up", "whats up"], about: "what's up: what's new and what's next" },
    Screen { id: "notifications", kind: "tool", names: &["notifications", "my notifications"], about: "Android apps' notifications" },
    Screen { id: "clock", kind: "tool", names: &["clock", "the clock"], about: "a big clock" },
    Screen { id: "wifi", kind: "settings", names: &["wifi", "wi-fi", "wi fi", "wifi settings", "wi-fi settings", "the wifi"], about: "Settings › Wi-Fi" },
    Screen { id: "bluetooth", kind: "settings", names: &["bluetooth", "bluetooth settings"], about: "Settings › Bluetooth" },
    Screen { id: "display", kind: "settings", names: &["display", "display settings", "sound settings", "display and sound", "brightness"], about: "Settings › Display & sound" },
    Screen { id: "look", kind: "settings", names: &["look", "the look", "look settings", "themes", "theme"], about: "Settings › Look" },
    Screen { id: "lock", kind: "settings", names: &["lock", "lock settings", "pin", "the pin"], about: "Settings › Lock (the PIN)" },
    Screen { id: "controls", kind: "settings", names: &["controls", "control settings", "button settings", "wheel settings"], about: "Settings › Controls (the wheel, a triple press)" },
    Screen { id: "browser", kind: "browser", names: &["browser", "the browser", "web browser", "my browser", "internet"], about: "the R1's browser (a site by name opens it there: \"go to amazon.com\")" },
    Screen { id: "connections", kind: "settings", names: &["connections", "connected apps", "my connections", "connection settings"], about: "Settings › Connections (the apps agents reach on your accounts: Gmail, Calendar, Drive…; signing in)" },
    Screen { id: "power", kind: "settings", names: &["power", "power options", "power settings"], about: "Settings › Power (turn off, restart)" },
    Screen { id: "time", kind: "settings", names: &["date and time", "time settings", "date & time"], about: "Settings › Date & time" },
    Screen { id: "accounts", kind: "settings", names: &["accounts", "my accounts"], about: "Settings › Accounts (Claude and Codex sign-ins)" },
    Screen { id: "agents", kind: "settings", names: &["agents settings", "agent settings"], about: "Settings › Agents" },
    Screen { id: "you", kind: "settings", names: &["about you", "about me"], about: "Settings › About you" },
    Screen { id: "computers", kind: "settings", names: &["computers", "linked computers"], about: "Settings › Computers (linked PCs)" },
    Screen { id: "builds", kind: "settings", names: &["app builds", "build settings"], about: "Settings › App builds" },
    Screen { id: "about", kind: "settings", names: &["about", "about the r1", "about this r1"], about: "Settings › About" },
];

/// The screen asked for by `name` (its id or one of its names; any case, a leading "the"/"my" and a trailing
/// "app"/"settings"/"page"/"screen" don't matter).
pub fn named(name: &str) -> Option<&'static Screen> {
    let n = name.trim().to_lowercase();
    let n = n.trim_start_matches("the ").trim_start_matches("my ").trim();
    let bare = n.trim_end_matches(" app").trim_end_matches(" screen").trim_end_matches(" page").trim();
    let without = bare.trim_end_matches(" settings").trim();
    SCREENS.iter().find(|s| s.id == bare || s.id == without || [n, bare].iter().any(|x| s.names.contains(x)) || (bare != without && s.names.contains(&without)))
}

/// The shell's target for a screen: `{t, id}` (a tool), `{t: settings, page}`, or `{t: camera}`, with the Translator's
/// languages when given (checked codes from crate::translator::LANGUAGES).
pub fn target(s: &Screen, mine: Option<&str>, theirs: Option<&str>) -> Result<Value, String> {
    let mut t = match s.kind {
        "settings" => json!({ "t": "settings", "page": s.id }),
        "camera" => json!({ "t": "camera" }),
        "browser" => json!({ "t": "browser" }),
        _ => json!({ "t": "tool", "id": s.id }),
    };
    if s.id == "translator" {
        for (k, v) in [("mine", mine), ("theirs", theirs)] {
            if let Some(code) = v.filter(|c| !c.is_empty()) {
                let code = language(code).ok_or_else(|| format!("{k}: `{code}` isn't one of the Translator's languages ({})", codes()))?;
                t[k] = json!(code);
            }
        }
    }
    Ok(t)
}

/// A language's code from its code or its English name ("pt", "Portuguese"), forgiving a slip of spelling or of
/// speech-to-text ("portugese"): one letter in a name of five or more, two in one of eight or more.
pub fn language(s: &str) -> Option<&'static str> {
    let s = s.trim().to_lowercase();
    let langs = crate::translator::LANGUAGES;
    if let Some((code, _)) = langs.iter().find(|(code, name)| *code == s || name.to_lowercase() == s) {
        return Some(code);
    }
    if let Some((_, code)) = OTHER_NAMES.iter().find(|(name, _)| *name == s) {
        return Some(code);
    }
    let slips = match s.chars().count() {
        n if n >= 8 => 2,
        n if n >= 5 => 1,
        _ => return None,
    };
    let near: Vec<&str> = langs.iter().filter(|(_, name)| distance(&s, &name.to_lowercase()) <= slips).map(|(code, _)| *code).collect();
    // Only when it's near one language, not two.
    if let [code] = near[..] { Some(code) } else { None }
}

/// Names people say for a language besides its English one.
const OTHER_NAMES: &[(&str, &str)] = &[("mandarin", "zh"), ("brazilian", "pt"), ("castilian", "es")];

/// Letters added, dropped, changed or swapped to get from `a` to `b` (Damerau's optimal string alignment).
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

fn codes() -> String {
    crate::translator::LANGUAGES.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", ")
}

/// For an agent's tool: every screen and what it is.
pub fn list() -> String {
    SCREENS.iter().map(|s| format!("{} ({})", s.id, s.about)).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_pinned_to_home() {
        for id in ["camera", "photos", "reminders", "browser", "whats_up", "inputs", "system"] {
            assert!(pinnable(id), "{id}");
        }
        for id in ["wifi", "connections", "nothing", ""] {
            assert!(!pinnable(id), "{id}");
        }
    }

    #[test]
    fn screens_by_name() {
        assert_eq!(named("the Translator").map(|s| s.id), Some("translator"));
        assert_eq!(named("bluetooth settings").map(|s| s.id), Some("bluetooth"));
        assert_eq!(named("Wi-Fi").map(|s| s.id), Some("wifi"));
        assert_eq!(named("my photos").map(|s| s.id), Some("photos"));
        assert_eq!(named("whats_up").map(|s| s.id), Some("whats_up"));
        assert!(named("spotify").is_none());
        let t = target(named("translator").unwrap(), Some("English"), Some("pt")).unwrap();
        assert_eq!(t, json!({ "t": "tool", "id": "translator", "mine": "en", "theirs": "pt" }));
        assert!(target(named("translator").unwrap(), None, Some("klingon")).is_err());
        // Slips of spelling and of speech-to-text.
        assert_eq!(language("portugese"), Some("pt"));
        assert_eq!(language("Portugeuse"), Some("pt"));
        assert_eq!(language("spansih"), Some("es"));
        assert_eq!(language("Mandarin"), Some("zh"));
        assert_eq!(language("germany"), Some("de"));
        assert_eq!(language("thia"), None);
        assert_eq!(language("finish"), None);
        assert_eq!(language("sentence"), None);
        assert_eq!(target(named("bluetooth").unwrap(), None, None).unwrap(), json!({ "t": "settings", "page": "bluetooth" }));
        assert_eq!(target(named("camera").unwrap(), None, None).unwrap(), json!({ "t": "camera" }));
    }
}
