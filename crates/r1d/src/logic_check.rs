//! A trial run of an app's logic before it's installed (ADR-0041): the build runs the script once in the agent box,
//! with a stand-in `r1` (the app's state and settings, empty queries, nothing playing), and its start and change
//! handlers. What goes wrong comes back as build problems, which the builder repairs like the compiler's.
//!
//! It catches what the compiler can't: a syntax error, a name that isn't defined, a value published that isn't in
//! `logic.values`, a state key saved that isn't declared, an action in `logic.actions` with no `r1.action`, a loop that
//! never ends. It can't know real data: `fetch`, `run` and `query` answer nothing (their promises never settle), so
//! code after them isn't tried, and no timer fires.
//!
//! The engine is QuickJS (`qjs`, Alpine's `quickjs` package in the box); without it the check is skipped.

use enee_spec::compiled::CompiledApp;
use serde_json::{Value, json};
use std::time::Duration;

/// A script that takes longer than this (to start and handle its first change) never ends.
const DEADLINE: Duration = Duration::from_secs(4);

/// The stand-in and the trial, then the app's script: one file that `qjs` (or `node`) runs, printing one JSON line.
pub fn harness(app: &CompiledApp) -> Option<String> {
    let logic = app.logic.as_ref()?;
    let settings: serde_json::Map<String, Value> =
        app.settings.iter().map(|(k, d)| (k.clone(), d.get("default").cloned().unwrap_or(Value::Null))).collect();
    let q: serde_json::Map<String, Value> = app.queries.iter().map(|q| (q.name.clone(), json!([]))).collect();
    let fixture = json!({
        "values": logic.values,
        "actions": logic.actions,
        "appActions": app.actions.keys().collect::<Vec<_>>(),
        "queries": app.queries.iter().map(|q| &q.name).collect::<Vec<_>>(),
        "state": app.state,
        "settings": settings,
        "q": q,
    });
    Some(format!(
        "{PRELUDE}\nconst FIXTURE = {fixture};\nfunction logic(r1) {{\n{script}\n}}\n{TRIAL}\n",
        script = logic.script
    ))
}

/// The stand-in `r1`: the same surface as the page's (packages/runtime/src/logic.tsx), checking what it's given.
const PRELUDE: &str = r#""use strict";
const problems = [];
const handlers = {};
const actions = {};
const never = () => new Promise(() => {});
const say = (m) => { if (problems.length < 10) problems.push(m); };
const view = { state: {}, settings: {}, q: {}, services: {}, player: { playing: false, state: "stopped" } };
const r1 = {
  get state() { return view.state; }, get settings() { return view.settings; }, get q() { return view.q; },
  get services() { return view.services; }, get player() { return view.player; },
  on(name, fn) { if (typeof fn !== "function") say("r1.on(\"" + name + "\") needs a function"); (handlers[name] = handlers[name] || []).push(fn); },
  set(name, value) { if (!FIXTURE.values.includes(String(name))) say("r1.set(\"" + name + "\"): `" + name + "` isn't in logic.values"); },
  save(key, value) { if (!(String(key) in FIXTURE.state)) say("r1.save(\"" + key + "\"): `" + key + "` isn't a state key"); },
  run(name) { if (!FIXTURE.appActions.includes(String(name))) say("r1.run(\"" + name + "\"): the app has no action `" + name + "`"); return never(); },
  query(name) { if (!FIXTURE.queries.includes(String(name))) say("r1.query(\"" + name + "\"): the app has no query `" + name + "`"); return never(); },
  fetch() { return never(); },
  action(name, fn) { if (typeof fn !== "function") say("r1.action(\"" + name + "\") needs a function"); actions[name] = fn; },
  every() { return {}; }, after() { return {}; }, cancel() {},
};
const where = (e) => String((e && e.message) || e) + (e && e.stack ? " at " + String(e.stack).split("\n").slice(0, 2).join(" ").trim() : "");
"#;

/// Start, then a change, as the page does; then what was wrong, as one JSON line.
const TRIAL: &str = r#"
view.state = JSON.parse(JSON.stringify(FIXTURE.state));
view.settings = FIXTURE.settings;
view.q = FIXTURE.q;
const fire = (name) => {
  for (const h of handlers[name] || []) {
    try { const r = h(); if (r && typeof r.catch === "function") r.catch((e) => say("in r1.on(\"" + name + "\"): " + where(e))); }
    catch (e) { say("in r1.on(\"" + name + "\"): " + where(e)); }
  }
};
try { logic(r1); } catch (e) { say("when it starts: " + where(e)); }
fire("start");
fire("change");
for (const a of FIXTURE.actions) if (!actions[a]) say("logic.actions has `" + a + "`, but the script never answers it with r1.action(\"" + a + "\", …)");
Promise.resolve().then(() => console.log(JSON.stringify({ problems })));
"#;

/// Runs the trial in the agent box. Its problems, as build problems (`E_LOGIC_RUN …`); none if it went well, or if
/// there's no box or no `qjs` to run it with.
pub async fn run(app: &CompiledApp) -> Vec<String> {
    let Some(js) = harness(app) else { return Vec::new() };
    let jail = crate::jail::Jail::default();
    if !jail.available() {
        return Vec::new();
    }
    let dir = jail.work().join(".logic-check");
    let name = format!("{}-{}.js", app.app.id, crate::core::random_hex(4));
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(dir.join(&name), &js).is_err() {
        return Vec::new();
    }
    // r1d's umask is 077 (its init wrapper): the box's agent user must be able to read it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::set_permissions(dir.join(&name), std::fs::Permissions::from_mode(0o644));
    }
    let inside = format!("/work/.logic-check/{name}");
    let out = async {
        let mut cmd = jail.command("/usr/bin/qjs", &["--std", &inside], "/work", &[]).ok()?;
        cmd.kill_on_drop(true).stdin(std::process::Stdio::null());
        cmd.output().await.ok()
    };
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(DEADLINE, out).await;
    let _ = std::fs::remove_file(dir.join(&name));
    let problems = match result {
        Err(_) => vec!["E_LOGIC_RUN the script didn't finish starting in 4 s: a loop that never ends?".to_owned()],
        Ok(None) => Vec::new(),
        Ok(Some(o)) => read(&String::from_utf8_lossy(&o.stdout), &String::from_utf8_lossy(&o.stderr), o.status.success()),
    };
    tracing::info!(app = %app.app.id, ms = started.elapsed().as_millis(), problems = problems.len(), "the logic's trial run");
    problems
}

/// The trial's output: its `{problems}` line, or (no line) why the engine stopped.
fn read(stdout: &str, stderr: &str, ok: bool) -> Vec<String> {
    if let Some(v) = stdout.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok()) {
        return v["problems"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|p| format!("E_LOGIC_RUN {p}")).collect();
    }
    // `qjs` missing in the box: no trial.
    if stderr.contains("not found") || stderr.contains("No such file") {
        return Vec::new();
    }
    let why = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or(if ok { "it printed nothing" } else { "it failed" });
    vec![format!("E_LOGIC_RUN the script doesn't run: {}", why.trim())]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test -p r1d trial_with_node -- --ignored`: the trial on the dice fixture, and broken copies, with node.
    #[test]
    #[ignore]
    fn trial_with_node() {
        let run = |src: &str| -> Vec<String> {
            let app = crate::apps::compile(src).expect("compiles");
            let js = harness(&app).expect("has logic");
            let path = std::env::temp_dir().join(format!("r1os-trial-{}.js", crate::core::random_hex(4)));
            std::fs::write(&path, js).unwrap();
            let o = std::process::Command::new("node").arg(&path).output().expect("node");
            let _ = std::fs::remove_file(&path);
            read(&String::from_utf8_lossy(&o.stdout), &String::from_utf8_lossy(&o.stderr), o.status.success())
        };
        let dice = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/apps/dice.enee.yaml")).unwrap();
        assert_eq!(run(&dice), Vec::<String>::new(), "the fixture runs clean");
        let bad = dice.replace("r1.set(\"summary\"", "r1.set(\"sumary\"");
        assert!(run(&bad)[0].contains("`sumary` isn't in logic.values"), "{:?}", run(&bad));
        let bad = dice.replace("const faces =", "const faces = undefinedThing.x; const f2 =");
        assert!(run(&bad)[0].contains("undefinedThing"), "{:?}", run(&bad));
        let bad = dice.replace("r1.action(\"roll\"", "r1.action(\"rol\"");
        assert!(run(&bad).iter().any(|p| p.contains("never answers it")), "{:?}", run(&bad));
        let bad = dice.replace("r1.on(\"change\", () => {", "r1.on(\"change\", () => { r1.q.recent.foo.bar;");
        assert!(run(&bad)[0].contains("in r1.on(\"change\")"), "{:?}", run(&bad));
        let tally = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/apps/tally.enee.yaml")).unwrap();
        assert_eq!(run(&tally), Vec::<String>::new(), "the tally fixture runs clean");
    }

    /// `R1OS_SPEC=<spec> R1OS_OUT=<file> cargo test -p r1d dump_trial -- --ignored`: the trial file for a spec, to run by hand.
    #[test]
    #[ignore]
    fn dump_trial() {
        let src = std::fs::read_to_string(std::env::var("R1OS_SPEC").expect("R1OS_SPEC")).unwrap();
        let app = crate::apps::compile(&src).expect("compiles");
        std::fs::write(std::env::var("R1OS_OUT").expect("R1OS_OUT"), harness(&app).expect("has logic")).unwrap();
    }

    #[test]
    fn reading_the_trial() {
        assert_eq!(read("{\"problems\":[]}\n", "", true), Vec::<String>::new());
        assert_eq!(read("noise\n{\"problems\":[\"r1.set(\\\"x\\\"): `x` isn't in logic.values\"]}", "", true).len(), 1);
        assert_eq!(read("", "SyntaxError: unexpected token in expression: ')'\n    at x.js:3", false), ["E_LOGIC_RUN the script doesn't run: SyntaxError: unexpected token in expression: ')'"]);
        assert!(read("", "/bin/sh: /usr/bin/qjs: not found", false).is_empty(), "no engine: no trial");
    }
}
