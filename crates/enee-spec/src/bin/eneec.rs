//! `eneec compile <file> [--out <path>] [--pretty] [--no-sql] [--diagnostics] [--checks]`: compiles an EneeSpec
//! file and prints the output JSON (`{ ok, diagnostics, app, checks? }`). `--checks` also runs the spec's checks on a
//! scratch database. Exit code 1 when there are errors or failing checks.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: eneec compile <file|-> [--out <path>] [--pretty] [--no-sql] [--diagnostics] [--checks]";
    let Some(("compile", rest)) = args.split_first().map(|(c, r)| (c.as_str(), r)) else {
        eprintln!("{usage}");
        return ExitCode::from(2);
    };
    let mut file = None;
    let mut out = None;
    let mut pretty = false;
    let mut opts = enee_spec::Options::default();
    let mut diagnostics_only = false;
    let mut run_checks = false;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = it.next().cloned(),
            "--pretty" => pretty = true,
            "--no-sql" => opts.check_sql = false,
            "--diagnostics" => diagnostics_only = true,
            "--checks" => run_checks = true,
            f if file.is_none() => file = Some(f.to_string()),
            other => {
                eprintln!("unexpected argument `{other}`\n{usage}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(file) = file else {
        eprintln!("{usage}");
        return ExitCode::from(2);
    };
    let src = if file == "-" {
        let mut s = String::new();
        if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s) {
            eprintln!("read stdin: {e}");
            return ExitCode::from(2);
        }
        s
    } else {
        match std::fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("read {file}: {e}");
                return ExitCode::from(2);
            }
        }
    };
    let started = std::time::Instant::now();
    let output = enee_spec::compile(&src, &opts);
    let elapsed = started.elapsed();
    let checks = match (&output.app, run_checks && output.ok) {
        (Some(app), true) => enee_spec::run_checks(app),
        _ => Vec::new(),
    };
    let checks_ok = checks.iter().all(|c| c.ok);
    let mut value = if diagnostics_only {
        serde_json::json!({ "ok": output.ok, "diagnostics": output.diagnostics })
    } else {
        serde_json::to_value(&output).expect("serializable")
    };
    if run_checks {
        value["checks"] = serde_json::to_value(&checks).expect("serializable");
    }
    let json = if pretty { serde_json::to_string_pretty(&value) } else { serde_json::to_string(&value) }.expect("json");
    match out {
        Some(path) => {
            let body = match (output.ok, &output.app) {
                (true, Some(app)) => serde_json::to_string_pretty(app).expect("json"),
                _ => json.clone(),
            };
            if let Err(e) = std::fs::write(&path, body) {
                eprintln!("write {path}: {e}");
                return ExitCode::from(2);
            }
            for d in &output.diagnostics {
                eprintln!("{}:{} {} {}: {}", file, d.line.unwrap_or(0), d.code, d.path, d.message);
            }
        }
        None => println!("{json}"),
    }
    eprintln!("compiled in {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    if output.ok && checks_ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
