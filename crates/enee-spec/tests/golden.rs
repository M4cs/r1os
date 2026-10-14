//! Golden test: `fixtures/apps/pushups.enee.yaml` compiles to `fixtures/apps/pushups.compiled.json` (the compiler ⇄
//! runtime contract example). Re-bless after an intended output change with `UPDATE_GOLDEN=1 cargo test -p enee-spec`.

use std::path::PathBuf;

use enee_spec::{Options, compile};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/apps")
}

#[test]
fn pushups_matches_golden() {
    let src = std::fs::read_to_string(fixtures().join("pushups.enee.yaml")).unwrap();
    let out = compile(&src, &Options::default());
    assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
    let actual = serde_json::to_value(out.app.expect("compiled app")).unwrap();
    let golden = fixtures().join("pushups.compiled.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&golden).unwrap()).unwrap();
    pretty_assertions::assert_eq!(expected, actual);
}

#[test]
fn pushups_checks_pass_and_wrong_expectations_fail() {
    let src = std::fs::read_to_string(fixtures().join("pushups.enee.yaml")).unwrap();
    let app = compile(&src, &Options::default()).app.unwrap();
    let results = enee_spec::run_checks(&app);
    assert!(results.len() == 1 && results[0].ok, "{results:#?}");

    let wrong = src.replace("expect: { today.total: 20, today.n: 1 }", "expect: { today.total: 25, today.n: 1 }");
    let app = compile(&wrong, &Options::default()).app.unwrap();
    let results = enee_spec::run_checks(&app);
    assert_eq!(results[0].failures, ["`/q/today/0/total` is 20, expected 25"]);
}

/// Every app fixture (the shell installs them as dev apps) compiles without diagnostics and passes its own checks.
#[test]
fn every_fixture_app_compiles_and_passes_its_checks() {
    let mut seen = 0;
    for entry in std::fs::read_dir(fixtures()).unwrap() {
        let path = entry.unwrap().path();
        if !path.to_string_lossy().ends_with(".enee.yaml") {
            continue;
        }
        seen += 1;
        let out = compile(&std::fs::read_to_string(&path).unwrap(), &Options::default());
        assert!(out.diagnostics.is_empty(), "{}: {:#?}", path.display(), out.diagnostics);
        let results = enee_spec::run_checks(&out.app.unwrap());
        assert!(results.iter().all(|r| r.ok), "{}: {results:#?}", path.display());
    }
    assert!(seen >= 2, "expected the pushups and expenses fixtures");
}

#[test]
fn compile_is_deterministic() {
    let src = std::fs::read_to_string(fixtures().join("pushups.enee.yaml")).unwrap();
    let a = serde_json::to_string(&compile(&src, &Options::default())).unwrap();
    let b = serde_json::to_string(&compile(&src, &Options::default())).unwrap();
    assert_eq!(a, b);
}
