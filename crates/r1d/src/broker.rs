//! App permissions on the R1 (ADR-0023, docs/specs/apps.md §6): what an app may do beyond its own data,
//! decided by the user in the native shell, never by the app or an agent.
//!
//! - **Declared, then asked.** An app lists what it needs under `permissions:` with a reason. The first time
//!   it opens, the shell's consent card asks about each one the R1 can grant ([`ON_R1`]), in r1os's words
//!   ([`label`]) with the app's reason under it. The card's answer comes back through
//!   `POST /api/apps/{id}/permissions` (the shell's own UI: the app's page and the agent have no route to it).
//! - **Bound to the declaration.** A decision holds for the permission as it was declared then
//!   ([`decl`]: its id and scope). An edit that changes it (another host) asks again; one that removes it
//!   drops it; the others keep their decisions.
//! - **Checked on every use** ([`allows`], [`allows_host`]): crate::services fetches only for a granted
//!   `net.http`, and only from its hosts.
//! - Permissions the R1 can't grant yet are listed as such and never granted. Builds can't use them
//!   ([`r1_problems`]); a spec installed from a file still installs, without them.
//!
//! Ported in spirit from eneeware's broker (permissions-and-services.md §3–4 there): T1 only, no HMAC (the
//! decisions live in core.db, root-only on the R1), no audit chain yet, just the last use.

use crate::core::Core;
use enee_spec::compiled::{CompiledApp, Permission};
use enee_spec::diag::{Diagnostic, Severity};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// The permissions the R1 can grant now.
pub const ON_R1: &[&str] = &["net.http", "notify", "schedule", "open.url", "clipboard.write", "audio.play"];

/// The permission as declared, for binding a decision to it: its id and scope.
pub fn decl(p: &Permission) -> String {
    json!({ "id": p.id, "scope": p.scope }).to_string()
}

/// A `net.http` permission's hosts, lowercase.
pub fn hosts(p: &Permission) -> Vec<String> {
    p.scope
        .as_ref()
        .and_then(|s| s.get("hosts"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect())
        .unwrap_or_default()
}

/// What it lets the app do, in r1os's words (the app's reason is shown under it).
pub fn label(p: &Permission) -> String {
    match p.id.as_str() {
        "net.http" => format!("get data from {}", and_list(&hosts(p))),
        "notify" => "send you reminders".into(),
        "schedule" => "run on a schedule, even when it's closed".into(),
        "open.url" => "open links".into(),
        "audio.play" => match hosts(p).as_slice() {
            [] => "play audio from the web, even after you leave it".into(),
            h => format!("play audio from {}, even after you leave it", and_list(h)),
        },
        "clipboard.read" => "read what you copy".into(),
        "clipboard.write" => "copy text to the clipboard".into(),
        other => other.to_owned(),
    }
}

pub fn and_list(items: &[String]) -> String {
    match items {
        [] => "the web".into(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One of an app's permissions, as its page and the consent card show it.
#[derive(Debug, Clone, Serialize)]
pub struct State {
    pub id: String,
    pub label: String,
    /// The app's own reason.
    pub reason: String,
    /// `granted` · `denied` · `ask` (no decision yet, or the app changed what it asks for) · `unavailable`
    /// (the R1 can't grant it yet).
    pub state: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_at: Option<i64>,
    pub uses: i64,
}

/// Every permission the app declares, with the user's decision.
pub fn states(core: &Core, app: &CompiledApp) -> Vec<State> {
    let grants = core.grants(&app.app.id);
    app.permissions
        .iter()
        .map(|p| {
            let grant = grants.get(&p.id).filter(|g| g.decl == decl(p));
            let state = match grant {
                _ if !ON_R1.contains(&p.id.as_str()) => "unavailable",
                Some(g) if g.granted => "granted",
                Some(_) => "denied",
                None => "ask",
            };
            State {
                id: p.id.clone(),
                label: label(p),
                reason: p.reason.clone(),
                state,
                hosts: if p.id == "net.http" { hosts(p) } else { Vec::new() },
                used_at: grant.and_then(|g| g.used_at),
                uses: grant.map_or(0, |g| g.uses),
            }
        })
        .collect()
}

/// The app declares `perm`, the R1 can grant it, and the user granted it as it's declared now.
pub fn allows(core: &Core, app: &CompiledApp, perm: &str) -> bool {
    let Some(p) = app.permissions.iter().find(|p| p.id == perm) else { return false };
    ON_R1.contains(&perm) && core.grants(&app.app.id).get(perm).is_some_and(|g| g.granted && g.decl == decl(p))
}

/// The app may fetch from `host`: a granted `net.http` that lists it.
pub fn allows_host(core: &Core, app: &CompiledApp, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    allows(core, app, "net.http")
        && app.permissions.iter().filter(|p| p.id == "net.http").flat_map(hosts).any(|h| h == host)
}

/// The app may play a stream from `host`: its `audio.play` names no hosts (anywhere on the web), or names this one.
/// (Whether it's granted is checked apart: [`allows`].)
pub fn allows_audio_host(app: &CompiledApp, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    app.permissions.iter().filter(|p| p.id == "audio.play").all(|p| {
        let named = hosts(p);
        named.is_empty() || named.contains(&host)
    })
}

/// Records the user's decisions (`{perm: true|false}`) for the permissions the app declares and the R1 can
/// grant. Returns how many were recorded.
pub fn decide(core: &Core, app: &CompiledApp, decisions: &Map<String, Value>, via: &str) -> anyhow::Result<usize> {
    let mut n = 0;
    for (id, granted) in decisions {
        let Some(granted) = granted.as_bool() else { continue };
        let Some(p) = app.permissions.iter().find(|p| &p.id == id) else { continue };
        if !ON_R1.contains(&id.as_str()) {
            continue;
        }
        core.set_grant(&app.app.id, id, &decl(p), granted, via)?;
        n += 1;
    }
    Ok(n)
}

/// What a build mustn't use on the R1 yet: permissions it can't grant and services it doesn't run. Errors,
/// for the builder to repair.
pub fn r1_problems(app: &CompiledApp) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut error = |path: String, message: String, hint: &str| {
        out.push(Diagnostic { code: "E_R1", severity: Severity::Error, path, line: None, message, hint: Some(hint.into()) });
    };
    for p in &app.permissions {
        if !ON_R1.contains(&p.id.as_str()) {
            error(
                format!("permissions.{}", p.id),
                format!("the R1 can't grant `{}` yet", p.id),
                "Remove it and the feature that needs it. The R1 grants only `net.http`, `notify`, `schedule`, `open.url`, `clipboard.write` and `audio.play` for now.",
            );
        }
    }
    // An app's own steps run on its page: it opens links (the R1's link viewer) and copies text, but has no
    // notifications, launcher, or clipboard history.
    for (name, def) in &app.actions {
        for step in def.get("steps").and_then(Value::as_array).into_iter().flatten() {
            if let Some(kind) = ["notify", "launch", "copy_parts"].into_iter().find(|k| step.get(*k).is_some()) {
                error(
                    format!("actions.{name}"),
                    format!("the R1 doesn't run `{kind}` steps in actions"),
                    "Remove the step. A reminder is a trigger's `notify`; `copy` copies text only.",
                );
            }
        }
    }
    for s in &app.services {
        if !s.uses.starts_with("http.") {
            error(
                format!("services.{}", s.name),
                format!("the R1 doesn't run `{}`", s.uses),
                "The R1 runs `http.json`, `http.rss` and `http.fetch` services only.",
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(yaml: &str) -> CompiledApp {
        crate::apps::compile(yaml).unwrap_or_else(|r| panic!("{:?}", r.diagnostics))
    }

    const WEATHER: &str = r#"enee: 1
app: { id: weather-now, name: Weather now }
permissions:
  net.http: { hosts: [wttr.in], reason: Get the forecast for your town }
settings:
  town: { type: text, default: London, label: Town }
services:
  now: { use: http.json, url: "https://wttr.in/{{settings.town}}?format=j1", every: 30m }
views:
  main:
    - stat: { label: Now, value: "{{services.now.current_condition.0.temp_C}}°" }
"#;

    #[test]
    fn decisions_bind_to_the_declaration() {
        let dir = std::env::temp_dir().join(format!("r1d-broker-{}", crate::core::random_hex(6)));
        let core = Core::open(&dir).unwrap();
        let a = app(WEATHER);
        assert_eq!(states(&core, &a)[0].state, "ask");
        assert_eq!(states(&core, &a)[0].label, "get data from wttr.in");
        assert!(!allows_host(&core, &a, "wttr.in"));

        let yes = Map::from_iter([("net.http".to_string(), json!(true)), ("notify".to_string(), json!(true))]);
        assert_eq!(decide(&core, &a, &yes, "consent").unwrap(), 1, "only what it declares and the R1 grants");
        assert!(allows_host(&core, &a, "WTTR.in"));
        assert!(!allows_host(&core, &a, "example.com"));

        // Another host: the old decision doesn't cover it, so it asks again.
        let b = app(&WEATHER.replace("hosts: [wttr.in]", "hosts: [wttr.in, api.open-meteo.com]"));
        assert_eq!(states(&core, &b)[0].state, "ask");
        assert!(!allows_host(&core, &b, "wttr.in"));
        assert_eq!(states(&core, &b)[0].label, "get data from wttr.in and api.open-meteo.com");

        let no = Map::from_iter([("net.http".to_string(), json!(false))]);
        decide(&core, &b, &no, "settings").unwrap();
        assert_eq!(states(&core, &b)[0].state, "denied");
        core.forget_grants("weather-now");
        assert_eq!(states(&core, &b)[0].state, "ask");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builds_use_only_what_the_r1_runs() {
        assert!(r1_problems(&app(WEATHER)).is_empty());
        // Its feed, reminders, schedule and links all run on the R1.
        let source = include_str!("../../../fixtures/apps/news.enee.yaml");
        assert!(r1_problems(&app(source)).is_empty(), "{:?}", r1_problems(&app(source)));
        // Starting other apps doesn't.
        let launcher = app(&source.replace("open.url", "apps.launch").replace("open_url:", "launch:"));
        let codes: Vec<String> = r1_problems(&launcher).iter().map(|d| d.path.clone()).collect();
        assert_eq!(codes, vec!["permissions.apps.launch".to_string(), "actions.open".into()]);
    }
}
