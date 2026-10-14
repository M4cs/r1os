//! Diagnostics for the mistakes models actually make: stable codes, YAML paths, lines and useful hints.

use enee_spec::diag::{Diagnostic, Severity};
use enee_spec::{Options, compile};

const BASE: &str = r#"enee: 1
app: { id: todo, name: Todo }
permissions:
  notify: Tell you when a task is added
data:
  items: { name: text!, done: bool = false }
queries:
  open: SELECT id, name FROM items WHERE done = 0
  stats: SELECT count(*) AS n FROM items
actions:
  add:
    params: { name: text }
    do:
      - sql: INSERT INTO items (name) VALUES (:name)
      - notify: { title: "Added {{params.name}}" }
views:
  main:
    - stat: { label: Open, value: "{{stats.n}}" }
    - list: { data: open, title: "{{row.name}}" }
"#;

fn diags(src: &str) -> Vec<Diagnostic> {
    compile(src, &Options::default()).diagnostics
}

fn only(src: &str, code: &str) -> Diagnostic {
    let all = diags(src);
    let found: Vec<&Diagnostic> = all.iter().filter(|d| d.code == code).collect();
    assert_eq!(found.len(), 1, "expected one {code}, got {all:#?}");
    found[0].clone()
}

fn patch(from: &str, to: &str) -> String {
    assert!(BASE.contains(from), "base spec has no `{from}`");
    BASE.replacen(from, to, 1)
}

#[test]
fn base_spec_is_clean() {
    assert_eq!(diags(BASE), Vec::<Diagnostic>::new());
}

#[test]
fn unquoted_binding_is_a_yaml_error_with_a_quoting_hint() {
    let d = only(&patch(r#"value: "{{stats.n}}""#, "value: {{stats.n}}"), "E_YAML");
    assert!(d.hint.unwrap().contains("Quote every binding"));
    assert_eq!(d.line, Some(18));
}

#[test]
fn unknown_sql_column_lists_the_schema() {
    let d = only(&patch("SELECT id, name FROM items WHERE done = 0", "SELECT id, nam FROM items"), "E_SQL");
    assert_eq!((d.path.as_str(), d.line), ("queries.open", Some(8)));
    assert!(d.message.contains("no such column: nam"), "{}", d.message);
    assert!(d.hint.unwrap().contains("`items`(id, name, done, created_at, updated_at)"));
}

#[test]
fn queries_must_be_read_only() {
    only(&patch("SELECT count(*) AS n FROM items", "DELETE FROM items"), "E_SQL_NOT_READONLY");
}

#[test]
fn binding_to_a_missing_column_suggests_the_real_ones() {
    let d = only(&patch("{{stats.n}}", "{{stats.count}}"), "E_BIND_COLUMN");
    assert_eq!((d.path.as_str(), d.line), ("views.main[0].stat.value", Some(18)));
    assert!(d.hint.unwrap().contains("Columns of `stats`: n"));
}

#[test]
fn a_row_holds_at_most_three_stats_and_nothing_else() {
    let stat = r#"{ stat: { label: Open, value: "{{stats.n}}" } }"#;
    let four = patch(
        r#"    - stat: { label: Open, value: "{{stats.n}}" }"#,
        &format!("    - row: [ {stat}, {stat}, {stat}, {stat} ]"),
    );
    let d = only(&four, "E_LAYOUT");
    assert_eq!(d.path, "views.main[0].row");
    assert!(d.message.contains("at most 3 stats"), "{}", d.message);
    let three = patch(
        r#"    - stat: { label: Open, value: "{{stats.n}}" }"#,
        &format!("    - row: [ {stat}, {stat}, {stat} ]"),
    );
    assert_eq!(diags(&three), Vec::<Diagnostic>::new(), "three line up");
    let mixed = patch(
        r#"    - stat: { label: Open, value: "{{stats.n}}" }"#,
        &format!("    - row: [ {stat}, {{ text: Hello }} ]"),
    );
    assert_eq!(only(&mixed, "W_LAYOUT").severity, Severity::Warning);
}

#[test]
fn unknown_block_suggests_the_closest() {
    let d = only(&patch("- stat: {", "- stats: {"), "E_BLOCK_UNKNOWN");
    assert!(d.hint.unwrap().starts_with("Did you mean `stat`?"));
}

#[test]
fn launch_steps_start_a_desktop_app_by_name() {
    let launcher = patch(
        "      - notify: { title: \"Added {{params.name}}\" }
",
        "      - notify: { title: \"Added {{params.name}}\" }
  play:
    do: [ { launch: Spotify } ]
",
    );
    let d = only(&launcher, "E_PERM_MISSING");
    assert_eq!(d.path, "actions.play.do[0].launch");
    let allowed = launcher.replacen(
        "permissions:
",
        "permissions:
  apps.launch: Start Spotify
",
        1,
    );
    let out = compile(&allowed, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    assert_eq!(out.app.unwrap().actions["play"]["steps"][0], serde_json::json!({ "launch": "Spotify" }));
    let blank = allowed.replace("launch: Spotify", "launch: \"\"");
    assert!(only(&blank, "E_STEP").message.contains("the app's name"));
}

#[test]
fn timers_count_down_or_up_and_count_as_stats() {
    let timer = |props: &str| {
        patch(
            "    - stat: { label: Open, value: \"{{stats.n}}\" }
",
            &format!(
                "    - timer: {props}
"
            ),
        )
    };
    let out =
        compile(&timer("{ label: Focus, until: \"{{open.0.name}}\", caption: left, done: Done }"), &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/timer-1"];
    assert_eq!(el["type"], "Timer");
    assert_eq!(el["props"]["until"], serde_json::json!({ "$state": "/q/open/0/name" }));
    assert_eq!(el["props"]["done"], "Done");
    assert!(compile(&timer("{ label: Up, since: \"{{open.0.name}}\" }"), &Options::default()).ok);

    // Both: it counts down (a session with an end), and says so.
    let both = timer("{ label: X, until: \"{{open.0.name}}\", since: \"{{open.0.name}}\" }");
    assert!(only(&both, "W_AUTOFIX").message.contains("kept `until`"));
    let out = compile(&both, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/timer-1"];
    assert!(el["props"].get("since").is_none() && el["props"].get("until").is_some());
    let neither = timer("{ label: X }");
    assert!(only(&neither, "E_PROP_MISSING").message.contains("`until`"));

    let crowded = patch(
        "    - stat: { label: Open, value: \"{{stats.n}}\" }
",
        "    - row:
        - stat: { label: A, value: \"{{stats.n}}\" }
        - stat: { label: B, value: \"{{stats.n}}\" }
        - stat: { label: C, value: \"{{stats.n}}\" }
        - timer: { label: D, since: \"{{open.0.name}}\" }
",
    );
    assert!(only(&crowded, "E_LAYOUT").message.contains("has 4"));
}

#[test]
fn gauges_need_a_value_and_three_fill_a_row() {
    let row = |n: usize| {
        let gauges: String = (0..n)
            .map(|i| format!("        - gauge: {{ label: G{i}, value: \"{{{{stats.n}}}}\", unit: \"%\" }}\n"))
            .collect();
        patch("    - stat: { label: Open, value: \"{{stats.n}}\" }\n", &format!("    - row:\n{gauges}"))
    };
    let out = compile(&row(3), &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/row-1/gauge-1"];
    assert_eq!(el["type"], "Gauge");
    assert_eq!(el["props"]["value"], serde_json::json!({ "$state": "/q/stats/0/n" }));
    assert_eq!(el["props"]["unit"], "%");
    assert!(only(&row(4), "E_LAYOUT").message.contains("3 gauges"));
    let bare = patch("    - stat: { label: Open, value: \"{{stats.n}}\" }\n", "    - gauge: { label: CPU }\n");
    assert!(only(&bare, "E_PROP_MISSING").message.contains("`value`"));
}

#[test]
fn donut_charts_show_shares_and_take_no_x_domain() {
    let chart = |props: &str| {
        patch("    - stat: { label: Open, value: \"{{stats.n}}\" }\n", &format!("    - chart: {props}\n"))
    };
    let out = compile(&chart("{ kind: donut, data: open, x: name, y: id, title: Shares }"), &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/chart-1"];
    assert_eq!(el["type"], "DonutChart");
    assert_eq!((el["props"]["x"].as_str(), el["props"]["y"].as_str()), (Some("name"), Some("id")));
    let pie = compile(&chart("{ kind: pie, data: open, x: name, y: id }"), &Options::default());
    assert!(pie.ok, "`pie` is a donut too");
    let ranged = chart("{ kind: donut, data: open, x: name, y: id, x_domain: \"0..23\" }");
    assert_eq!(only(&ranged, "W_PROP_IGNORED").path, "views.main[0].chart.x_domain");
    let out = compile(&ranged, &Options::default());
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    assert!(views[0]["spec"]["elements"]["main/chart-1"]["props"].get("xDomain").is_none());
    assert!(
        only(&chart("{ kind: radar, data: open, x: name, y: id }"), "E_PROP_VALUE").hint.unwrap().contains("donut")
    );
}

#[test]
fn calendar_heatmaps_shade_days_from_a_query() {
    let heat = |props: &str| {
        patch("    - stat: { label: Open, value: \"{{stats.n}}\" }\n", &format!("    - calendar_heatmap: {props}\n"))
    };
    let out = compile(&heat("{ data: open, date: name, value: id, weeks: 8, title: Weeks }"), &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/calendar_heatmap-1"];
    assert_eq!(el["type"], "CalendarHeatmap");
    assert_eq!(el["props"]["weeks"], 8);
    assert_eq!(
        only(&heat("{ data: open, date: day, value: id }"), "E_BIND_COLUMN").path,
        "views.main[0].calendar_heatmap.date"
    );
    assert!(
        only(&heat("{ data: open, date: name, value: id, weeks: 60 }"), "E_PROP_VALUE").message.contains("1 to 53")
    );
    assert!(only(&heat("{ data: open, date: name }"), "E_PROP_MISSING").message.contains("`value`"));
}

/// A clipboard history (ADR-0022): every copy into a table, shown with thumbnails, copied back as what it was.
const CLIPS: &str = r#"enee: 1
app: { id: clips, name: Clips }
permissions:
  clipboard.read: Keep a history of what you copy
data:
  clips: { kind: enum(text|image|files)!, text: text, files: text, image: text, thumb: text, source: text, pinned: bool = false }
services:
  history: { use: clipboard, into: clips, limit: 300 }
queries:
  recent: SELECT id, kind, coalesce(text, '') AS text, files, image, thumb FROM clips ORDER BY id DESC LIMIT 50
actions:
  copy_clip:
    params: { text: text?, image: text?, files: text? }
    do:
      - copy: { text: "{{params.text}}", image: "{{params.image}}", files: "{{params.files}}" }
      - toast: Copied
views:
  main:
    - list:
        data: recent
        title: "{{row.text}}"
        image: "{{row.thumb}}"
        row_actions: [ { icon: copy, label: Copy, action: copy_clip, params: { text: "{{row.text}}", image: "{{row.image}}", files: "{{row.files}}" } } ]
"#;

#[test]
fn clipboard_history_writes_into_a_table_and_copies_items_back() {
    let out = compile(CLIPS, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let app = out.app.unwrap();
    let service = &app.services[0];
    assert_eq!(
        (service.uses.as_str(), service.options["into"].as_str(), service.options["limit"].as_u64()),
        ("clipboard", Some("clips"), Some(300))
    );
    assert_eq!(
        app.actions["copy_clip"]["steps"][0]["copy_parts"]["image"],
        serde_json::json!({ "p": "/params/image" })
    );
    let views = serde_json::to_value(&app.views).unwrap();
    assert_eq!(views[0]["spec"]["elements"]["main/list-1"]["props"]["image"], serde_json::json!({ "i": "thumb" }));

    let without = |from: &str, to: &str| CLIPS.replacen(from, to, 1);
    assert_eq!(
        only(&without("clipboard.read: Keep a history of what you copy\n", ""), "E_PERM_MISSING").path,
        "services.history.use"
    );
    assert!(only(&without(", into: clips", ""), "E_SERVICE").message.contains("needs `into"));
    assert!(only(&without("{ kind: enum(text|image|files)!, ", "{ "), "E_SERVICE").message.contains("`kind` column"));
    assert!(only(&without("limit: 300", "every: 5s"), "E_SERVICE").message.contains("no `every`"));
    assert!(only(&without("limit: 300", "limit: 6000"), "E_SERVICE").message.contains("1 to 5000"));
    let bound = only(&without("limit: 300", "limit: \"{{settings.keep}}\""), "E_SERVICE");
    assert!(bound.hint.as_deref().is_some_and(|h| h.contains("a setting can't change it")), "{bound:?}");
    let bad_part = without("files: \"{{params.files}}\" }", "files: \"{{params.files}}\", html: x }");
    assert!(only(&bad_part, "E_STEP").message.contains("not `html`"));
}

#[test]
fn persist_keeps_declared_state_keys() {
    let kept = patch(
        "queries:
",
        "state:
  _persist: [ draft, tab, draft ]
  draft: { name: \"\" }
  tab: open
  scroll: 0
queries:
",
    );
    let out = compile(&kept, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let app = out.app.unwrap();
    assert_eq!(app.persist, ["draft", "tab"], "kept once each, in order");
    assert!(!app.state.contains_key("_persist"), "`_persist` isn't a state key");
    assert_eq!(serde_json::to_value(&app).unwrap()["persist"], serde_json::json!(["draft", "tab"]));

    let unknown = kept.replace("_persist: [ draft, tab, draft ]", "_persist: [ draft, tabs ]");
    let d = only(&unknown, "E_REF_STATE");
    assert_eq!(d.path, "state._persist");
    assert!(d.message.contains("`tabs`") && d.hint.as_deref().unwrap_or("").contains("tab"), "{d:#?}");
    let not_a_list = kept.replace("_persist: [ draft, tab, draft ]", "_persist: draft");
    assert_eq!(only(&not_a_list, "E_SHAPE").path, "state._persist");

    // Apps that keep nothing compile as before (no `persist` in the output).
    let plain = compile(BASE, &Options::default()).app.unwrap();
    assert!(plain.persist.is_empty() && serde_json::to_value(&plain).unwrap().get("persist").is_none());
}

#[test]
fn a_triggers_copy_needs_clipboard_write_and_a_clicks_does_not() {
    let trigger = format!(
        "{}triggers:
  morning:
    at: \"09:00\"
    do: [ {{ copy: Standup notes }} ]
",
        BASE.replacen(
            "permissions:
",
            "permissions:
  schedule: Copy the notes each morning
",
            1
        )
    );
    assert_eq!(only(&trigger, "E_PERM_MISSING").path, "triggers.morning.do[0].copy");
    let allowed = trigger.replacen(
        "permissions:
",
        "permissions:
  clipboard.write: Put the notes on the clipboard
",
        1,
    );
    assert!(compile(&allowed, &Options::default()).ok, "{:#?}", diags(&allowed));
    // An action's copy runs on a click: no permission at all.
    let click = patch(
        "      - notify: { title: \"Added {{params.name}}\" }
",
        "      - copy: \"{{params.name}}\"
",
    )
    .replace(
        "permissions:
  notify: Tell you when a task is added
",
        "",
    );
    assert!(compile(&click, &Options::default()).ok, "{:#?}", diags(&click));
}

#[test]
fn permission_coverage() {
    let d = only(&patch("permissions:\n  notify: Tell you when a task is added\n", ""), "E_PERM_MISSING");
    assert_eq!(d.path, "actions.add.do[1].notify");
    let d = only(&patch("permissions:\n", "permissions:\n  open.url: Open links\n"), "W_PERM_UNUSED");
    assert_eq!(d.severity, Severity::Warning);
    only(&patch("permissions:\n", "permissions:\n  notifications: Ping\n"), "E_PERM_UNKNOWN");
}

#[test]
fn buttons_are_checked_against_action_signatures() {
    let button = |b: &str| patch("    - list:", &format!("    - button: {b}\n    - list:"));
    let d = only(&button("{ label: Go, action: ad, params: { name: x } }"), "E_REF_ACTION");
    assert!(d.hint.unwrap().starts_with("Did you mean `add`?"));
    only(&button("{ label: Go, action: add }"), "E_ARG_MISSING");
    only(&button("{ label: Go, action: add, params: { name: x, title: y } }"), "E_ARG_UNKNOWN");
}

#[test]
fn row_bindings_only_inside_lists() {
    only(&patch("{{stats.n}}", "{{row.n}}"), "E_BIND_ROW");
    let d = only(&patch("{{row.name}}", "{{row.title}}"), "E_BIND_COLUMN");
    assert!(d.hint.unwrap().contains("Row columns: id, name"));
}

#[test]
fn sql_step_params_must_be_declared() {
    let d = only(&patch("VALUES (:name)", "VALUES (:nme)"), "E_SQL_PARAM");
    assert_eq!(d.path, "actions.add.do[0].sql");
}

#[test]
fn schema_changes_are_denied_in_steps() {
    only(&patch("INSERT INTO items (name) VALUES (:name)", "DROP TABLE items"), "E_SQL_DENIED");
}

#[test]
fn triggers_only_run_host_steps() {
    let src = format!("{BASE}triggers:\n  nag:\n    every: 1h\n    do: [ {{ toast: hi }} ]\n")
        .replace("permissions:\n", "permissions:\n  schedule: Nag hourly\n");
    let d = only(&src, "E_TRIGGER_STEP");
    assert_eq!(d.path, "triggers.nag.do[0].toast");
}

#[test]
fn reserved_and_unknown_names() {
    only(&patch("done: bool = false", "order: int"), "E_NAME_RESERVED");
    let d = only(&format!("{BASE}view: {{}}\n"), "E_KEY_UNKNOWN");
    assert_eq!(d.hint.as_deref(), Some("Did you mean `views`?"));
}

#[test]
fn yaml_1_2_scalars_stay_strings() {
    let src = format!(
        "{BASE}triggers:
  nag:
    on: open
    do: [ {{ refresh: open }} ]
"
    )
    .replace(
        "permissions:
",
        "permissions:
  schedule: Refresh on open
",
    )
    .replace(
        "- list: { data: open, title: \"{{row.name}}\" }",
        "- list: { data: open, title: \"{{row.name}}\" }
    - chart: { kind: bar, data: open, x: name, y: id }
    - text: { text: no, tone: muted }",
    );
    let out = compile(&src, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let app = out.app.unwrap();
    assert_eq!(app.triggers[0]["on"], "open");
    let text = &app.views[0].spec["elements"]["main/text-1"]["props"]["text"];
    assert_eq!(text, "no");
    let time = compile(
        "enee: 1
app: { id: t, name: T }
settings:
  at: { type: time, default: 21:30 }
views: { main: [] }
",
        &Options::default(),
    );
    assert!(time.ok, "{:#?}", time.diagnostics);
    assert_eq!(time.app.unwrap().settings["at"]["default"], "21:30");
}

#[test]
fn enum_params_make_form_selects() {
    let src = BASE
        .replace("data:\n", "state:\n  draft: { name: \"\", kind: home }\ndata:\n")
        .replace("params: { name: text }", "params: { name: text, kind: enum(home|work)? }")
        .replace("- list: {", "- form: { fields: [ { bind: state.draft.name }, { bind: state.draft.kind } ], submit: { label: Add, action: add } }\n    - list: {");
    let src = {
        // `state` must come after `queries`; move it to keep the section order.
        let state = "state:\n  draft: { name: \"\", kind: home }\n";
        src.replace(state, "").replace("actions:\n", &format!("{state}actions:\n"))
    };
    let out = compile(&src, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let el = &out.app.unwrap().views[0].spec["elements"]["main/form-1/input-2"];
    assert_eq!(el["props"]["type"], "select");
    assert_eq!(el["props"]["options"], serde_json::json!(["home", "work"]));
}

#[test]
fn commas_in_flow_maps_are_rejoined() {
    let src = BASE.replace(
        "app: { id: todo, name: Todo }",
        "app: { id: todo, name: Todo, description: Add, check off, repeat. }",
    );
    let out = compile(&src, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    assert_eq!(out.app.unwrap().app.description.as_deref(), Some("Add, check off, repeat."));
    let fixes: Vec<_> = out.diagnostics.iter().filter(|d| d.code == "W_AUTOFIX").collect();
    assert_eq!(fixes.len(), 2, "{:#?}", out.diagnostics);
    assert_eq!(fixes[0].path, "app.description");
}

#[test]
fn section_order_is_a_warning() {
    let src =
        BASE.replace("permissions:\n  notify: Tell you when a task is added\n", "") + "permissions:\n  notify: Ping\n";
    let all = diags(&src);
    assert!(all.iter().any(|d| d.code == "W_SECTION_ORDER" && d.severity == Severity::Warning), "{all:#?}");
    assert!(!all.iter().any(|d| d.severity == Severity::Error), "{all:#?}");
}

#[test]
fn preview_mode_skips_sql_but_still_binds() {
    let src = patch("SELECT id, name FROM items WHERE done = 0", "SELECT id, nam FROM items");
    let out = compile(&src, &Options { check_sql: false });
    assert!(out.ok, "{:#?}", out.diagnostics);
    let app = out.app.unwrap();
    assert!(app.queries.iter().all(|q| q.columns.is_empty()));
}

const WEB: &str = r#"enee: 1
app: { id: quotes, name: Quotes }
permissions:
  net.http: { hosts: [api.example.com], reason: Fetch quotes }
state:
  symbol: AAPL
services:
  quote: { use: http.json, url: "https://api.example.com/v1/quote/{{state.symbol}}?range=1d", every: 1m }
views:
  main:
    - stat: { label: Price, value: "{{services.quote.price | default:—}}" }
"#;

#[test]
fn http_urls_bind_plain_values_in_the_path_and_query_only() {
    let out = compile(WEB, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    assert_eq!(out.app.unwrap().services[0].deps, ["/state/symbol"]);
    let web = |from: &str, to: &str| {
        assert!(WEB.contains(from), "no `{from}`");
        WEB.replacen(from, to, 1)
    };
    let host = only(&web("https://api.example.com/v1", "https://{{state.symbol}}.example.com/v1"), "E_SERVICE");
    assert!(host.message.contains("host must be written out"), "{}", host.message);
    let filtered = only(&web("{{state.symbol}}", "{{state.symbol | upper}}"), "E_SERVICE");
    assert!(filtered.hint.unwrap().contains("no filters"));
    only(&web("{{state.symbol}}", "{{state.nope}}"), "E_SERVICE");
    only(&web("https://api.example.com/v1", "http://api.example.com/v1"), "E_SERVICE");
    only(&web("https://api.example.com/v1", "https://other.example.com/v1"), "E_PERM_SCOPE");
}

#[test]
fn feeds_and_pages_compile_and_only_feeds_fill_tables() {
    let rss = WEB.replacen("use: http.json", "use: http.rss", 1);
    assert!(compile(&rss, &Options::default()).ok, "{:#?}", diags(&rss));
    let page = WEB.replacen("use: http.json", "use: http.fetch", 1);
    assert!(compile(&page, &Options::default()).ok, "{:#?}", diags(&page));
    let with_table = |columns: &str, uses: &str| {
        WEB.replacen("use: http.json", &format!("use: {uses}"), 1)
            .replacen("every: 1m }", "every: 15m, into: items }", 1)
            .replacen(
                "state:
",
                &format!(
                    "data:
  items: {{ {columns} }}
state:
"
                ),
                1,
            )
    };
    let kept = with_table("guid: text!, title: text, read: bool = false", "http.rss");
    assert!(compile(&kept, &Options::default()).ok, "{:#?}", diags(&kept));
    let no_guid = only(&with_table("title: text", "http.rss"), "E_SERVICE");
    assert!(no_guid.message.contains("guid: text") && no_guid.hint.unwrap().contains("read: bool = false"));
    assert!(only(&with_table("guid: text!", "http.json"), "E_SERVICE").message.contains("only `http.rss`"));
}

#[test]
fn schedule_keeps_services_running_so_it_is_used_by_them() {
    let background =
        WEB.replacen("permissions:\n", "permissions:\n  schedule: Keep prices current in the background\n", 1);
    assert!(compile(&background, &Options::default()).diagnostics.iter().all(|d| d.code != "W_PERM_UNUSED"));
    let unused = patch("permissions:\n", "permissions:\n  schedule: Nothing\n");
    only(&unused, "W_PERM_UNUSED");
}

#[test]
fn a_form_without_submit_only_edits_state() {
    // A converter's fields feed a query over state as they're typed: no button needed.
    let src = r#"enee: 1
app: { id: km, name: Km }
queries:
  miles:
    sql: "SELECT round(coalesce(nullif(:km, ''), 0) * 0.621371, 2) AS value"
    params: { km: "{{state.km}}" }
state: { km: 10 }
views:
  main:
    - form: { fields: [ { bind: state.km, label: Kilometres } ] }
    - stat: { label: Miles, value: "{{miles.value}}" }
"#;
    let out = compile(src, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let elements = &views[0]["spec"]["elements"];
    let card = elements.as_object().unwrap().values().find(|e| e["type"] == "Card").unwrap();
    let children: Vec<&str> = card["children"].as_array().unwrap().iter().filter_map(|c| c.as_str()).collect();
    assert_eq!(children.len(), 1, "the field and no button: {children:?}");
    assert_eq!(elements[children[0]]["type"], "Input");
}

#[test]
fn steps_written_without_do_are_read_as_do() {
    let src = r#"enee: 1
app: { id: sets, name: Sets }
permissions:
  notify: Remind you
  schedule: Check every hour
data:
  sets: { reps: int! }
queries:
  today: SELECT count(*) AS n FROM sets
actions:
  log:
    params: { reps: int }
    sql: INSERT INTO sets (reps) VALUES (:reps)
    toast: Logged
triggers:
  nudge:
    every: 1h
    notify: { title: Move }
views:
  main:
    - button: { label: Log, action: log, params: { reps: 10 } }
checks:
  - do: { action: log, params: { reps: 10 } }
    expect: { today.n: 1 }
"#;
    let out = compile(src, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let fixes = out.diagnostics.iter().filter(|d| d.code == "W_AUTOFIX").count();
    assert_eq!(fixes, 2, "one for the action, one for the trigger: {:#?}", out.diagnostics);
    assert!(!out.diagnostics.iter().any(|d| d.code == "W_KEY_UNKNOWN"));
    let app = out.app.unwrap();
    let steps = &app.actions["log"]["steps"];
    assert_eq!(steps[0]["sql"], "INSERT INTO sets (reps) VALUES (:reps)", "in the order written");
    assert_eq!(steps[1]["toast"], "Logged");
    assert_eq!(app.triggers[0]["steps"][0]["notify"]["title"], "Move");
}

#[test]
fn a_check_in_an_app_without_actions_is_dropped() {
    let src = r#"enee: 1
app: { id: tip, name: Tip }
queries:
  tip:
    sql: "SELECT round(coalesce(nullif(:bill, ''), 0) * 0.18, 2) AS value"
    params: { bill: "{{state.bill}}" }
state: { bill: 40 }
views:
  main:
    - input: { bind: state.bill, label: Bill }
    - stat: { label: Tip, value: "{{tip.value}}" }
checks:
  - do: { action: log, params: { bill: 40 } }
    expect: { tip.value: 7.2 }
  - expect: { tip.value: 7.2 }
"#;
    let out = compile(src, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    assert!(out.diagnostics.iter().any(|d| d.code == "W_AUTOFIX" && d.message.contains("no actions")));
    assert_eq!(out.app.unwrap().checks.len(), 1, "the check that runs nothing stays");
}

/// BASE with `state:` declared and one block in place of the stat.
fn with_block(state: &str, block: &str) -> String {
    let s = patch("views:\n", &format!("state: {state}\nviews:\n"));
    s.replacen("    - stat: { label: Open, value: \"{{stats.n}}\" }\n", &format!("    - {block}\n"), 1)
}

#[test]
fn wheels_turn_numbers_and_options() {
    let out = compile(&with_block("{ km: 5 }", "wheel: { bind: state.km, min: 0, max: 100, step: 0.5, unit: km }"), &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/wheel-1"];
    assert_eq!(el["type"], "Wheel");
    assert_eq!(el["props"]["value"], serde_json::json!({ "$bindState": "/state/km" }));
    assert_eq!((el["props"]["min"].as_f64(), el["props"]["max"].as_f64(), el["props"]["step"].as_f64()), (Some(0.0), Some(100.0), Some(0.5)));
    assert_eq!(el["props"]["label"], "Km");
    assert_eq!(el["props"]["unit"], "km");

    let sizes = with_block("{ size: medium }", "wheel: { bind: state.size, options: [small, medium, large], wrap: true }");
    let out = compile(&sizes, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    assert_eq!(views[0]["spec"]["elements"]["main/wheel-1"]["props"]["options"], serde_json::json!(["small", "medium", "large"]));

    // A number wheel needs its ends, in order, with a sensible step.
    assert!(only(&with_block("{ km: 5 }", "wheel: { bind: state.km }"), "E_PROP_MISSING").message.contains("`min` and `max`"));
    assert!(only(&with_block("{ km: 5 }", "wheel: { bind: state.km, min: 10, max: 1 }"), "E_PROP_VALUE").message.contains("above `min`"));
    assert!(only(&with_block("{ km: 5 }", "wheel: { bind: state.km, min: 0, max: 10, step: 0 }"), "E_PROP_VALUE").message.contains("above 0"));
    assert!(only(&with_block("{ km: 5 }", "wheel: { bind: state.km, min: 0, max: 1000000, step: 0.001 }"), "E_PROP_VALUE").message.contains("100 000"));
    assert!(only(&with_block("{ km: 5 }", "wheel: { bind: nope.km, min: 0, max: 10 }"), "E_BIND_TARGET").message.contains("not a state path"));
}

#[test]
fn number_fields_carry_their_range() {
    let out = compile(&with_block("{ reps: 10 }", "input: { bind: state.reps, min: 1, max: 50 }"), &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/input-1"];
    assert_eq!((el["type"].as_str(), el["props"]["type"].as_str()), (Some("Input"), Some("number")));
    assert_eq!((el["props"]["min"].as_f64(), el["props"]["max"].as_f64()), (Some(1.0), Some(50.0)));
}

#[test]
fn games_are_sandboxed_scripts() {
    let game = |props: &str| with_block("{ best: 0 }", &format!("game: {props}"));
    let ok = game("{ label: Catch, script: \"r1.loop((dt) => { r1.ctx.fillRect(0, 0, 10, 10) })\", actions: [add], height: 320 }");
    let out = compile(&ok, &Options::default());
    assert!(out.ok, "{:#?}", out.diagnostics);
    let views = serde_json::to_value(&out.app.unwrap().views).unwrap();
    let el = &views[0]["spec"]["elements"]["main/game-1"];
    assert_eq!(el["type"], "Game");
    assert_eq!(el["props"]["actions"], serde_json::json!(["add"]));
    assert!(el["props"]["script"].as_str().unwrap().contains("r1.loop"));

    // No network, no page, no eval; only declared actions; a script that never touches r1 is suspicious.
    assert!(only(&game("{ label: X, script: \"fetch('https://x.y')\" }"), "E_GAME_SCRIPT").message.contains("network"));
    assert!(only(&game("{ label: X, script: \"document.body.innerHTML = 1\" }"), "E_GAME_SCRIPT").message.contains("no page"));
    assert!(only(&game("{ label: X, script: \"r1.loop(() => {})\", actions: [nope] }"), "E_ACTION_UNKNOWN").message.contains("nope"));
    assert!(only(&game("{ label: X, script: \"let a = 1\" }"), "W_GAME_SCRIPT").message.contains("never uses"));
    assert!(only(&game("{ label: X, script: \"r1.loop(() => {})\", height: 9000 }"), "E_PROP_VALUE").message.contains("160"));
    // `ui:` can't carry one around the checks.
    assert!(!diags(&with_block("{ best: 0 }", "ui: { type: Game, props: { label: X, script: \"fetch(1)\" } }")).is_empty());
}

const RADIO: &str = r#"enee: 1
app: { id: radio, name: Radio, icon: radio }
permissions:
  audio.play: Play the stations you pick
data:
  stations: { name: text!, url: text! }
queries:
  all: SELECT id, name, url FROM stations ORDER BY name
actions:
  listen:
    params: { name: text, url: text }
    do:
      - play: { url: "{{params.url}}", title: "{{params.name}}", subtitle: Radio }
  pause_it:
    do:
      - toggle: true
  off:
    do:
      - stop: true
views:
  main:
    - heading: { text: "{{player.title}}" }
    - text: { text: "{{player.song}}" }
    - button: { label: "Pause", action: pause_it }
    - list: { data: all, title: "{{row.name}}", row_actions: [ { icon: play, label: Play, action: listen, params: { name: "{{row.name}}", url: "{{row.url}}" } } ] }
"#;

#[test]
fn a_radio_compiles_clean() {
    let errors: Vec<Diagnostic> = diags(RADIO).into_iter().filter(|d| d.severity == Severity::Error).collect();
    assert_eq!(errors, Vec::<Diagnostic>::new());
}

#[test]
fn playing_needs_the_permission() {
    let src = RADIO.replace("permissions:\n  audio.play: Play the stations you pick\n", "");
    let d = only(&src, "E_PERM_MISSING");
    assert!(d.message.contains("`audio.play`"), "{}", d.message);
}

#[test]
fn play_takes_a_stream_url_and_its_names() {
    let d = only(&RADIO.replace(r#"- play: { url: "{{params.url}}","#, r#"- play: { url: "{{params.url}}", volume: 3,"#), "E_STEP");
    assert!(d.message.contains("`url`, `title` and `subtitle`"), "{}", d.message);
    only(&RADIO.replace(r#"url: "{{params.url}}", title"#, r#"url: "radio.example/stream", title"#), "E_STEP");
    let d = only(&RADIO.replace("{{player.song}}", "{{player.volume}}"), "E_BIND_PLAYER");
    assert!(d.hint.unwrap().contains("playing, state, title"));
}

const LOGIC: &str = r#"enee: 1
app: { id: scores, name: Scores }
permissions:
  net.http: { hosts: [example.com], reason: Get scores }
data:
  rounds: { points: int! }
queries:
  all: SELECT id, points FROM rounds ORDER BY id DESC
actions:
  add:
    params: { points: int }
    do: [ { sql: "INSERT INTO rounds (points) VALUES (:points)" } ]
  reset:
    do: [ { run: clear } ]
views:
  main:
    - stat: { label: Best, value: "{{logic.best}}" }
    - list: { data: "{{logic.rows}}", title: "{{row.label}}" }
    - logic_view: board
logic:
  values: [best, rows, board]
  actions: [clear]
  script: |
    r1.on("change", () => {
      const pts = r1.q.all.map((r) => r.points);
      r1.set("best", pts.length ? Math.max(...pts) : 0);
      r1.set("rows", pts.map((p, i) => ({ label: `#${i + 1}: ${p}` })));
    });
    r1.action("clear", async () => { await r1.fetch("https://example.com/x"); });
"#;

#[test]
fn logic_compiles_and_binds() {
    let errors: Vec<Diagnostic> = diags(LOGIC).into_iter().filter(|d| d.severity == Severity::Error).collect();
    assert_eq!(errors, Vec::<Diagnostic>::new());
}

#[test]
fn logic_is_checked() {
    let d = only(&LOGIC.replace("{{logic.best}}", "{{logic.worst}}"), "E_BIND_UNKNOWN");
    assert!(d.message.contains("worst"), "{}", d.message);
    only(&LOGIC.replace("do: [ { run: clear } ]", "do: [ { run: wipe } ]"), "E_REF_LOGIC");
    let d = only(&LOGIC.replace("- logic_view: board", "- logic_view: boards"), "E_BIND_UNKNOWN");
    assert!(d.message.contains("boards"), "{}", d.message);
    let d = only(&LOGIC.replace("await r1.fetch(", "await fetch("), "E_LOGIC_FORBIDDEN");
    assert!(d.message.contains("r1.fetch"), "{}", d.message);
    only(&LOGIC.replace("r1.action(\"clear\"", "document.title = 1; r1.action(\"clear\""), "E_LOGIC_FORBIDDEN");
}

const GLANCE: &str = r#"enee: 1
app: { id: water, name: Water, archetype: widget }
data:
  drinks: { glasses: int! }
queries:
  today: SELECT coalesce(sum(glasses), 0) AS glasses FROM drinks WHERE date(created_at) = date('now', 'localtime')
  week: SELECT date(created_at) AS day, sum(glasses) AS glasses FROM drinks GROUP BY 1 ORDER BY 1 DESC LIMIT 7
state:
  goal: 8
views:
  main:
    - stat: { label: Today, value: "{{today.0.glasses}}" }
glance:
  type: bars
  title: "Water: {{today.0.glasses | default:0}} of {{state.goal}}"
  items: { each: week, take: 7, label: "{{row.day | date}}", value: "{{row.glasses}}" }
"#;

#[test]
fn a_glance_card_compiles() {
    let out = compile(GLANCE, &Options::default());
    let errors: Vec<&Diagnostic> = out.diagnostics.iter().filter(|d| d.severity == Severity::Error).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let glance = out.app.expect("compiles").glance.expect("has a glance card");
    assert_eq!(glance["type"], "bars");
    assert_eq!(glance["items"]["each"], serde_json::json!({ "p": "/q/week" }));
    assert_eq!(glance["items"]["take"], 7);
    assert_eq!(glance["items"]["item"]["value"], serde_json::json!({ "i": "glasses" }));
}

#[test]
fn a_glance_card_is_checked() {
    let d = only(&GLANCE.replace("type: bars", "type: chart"), "E_GLANCE_TYPE");
    assert!(d.message.contains("chart"), "{}", d.message);
    let d = only(&GLANCE.replace("each: week,", "each: weeks,"), "E_REF_QUERY");
    assert!(d.hint.as_deref().unwrap_or_default().contains("week"), "{d:?}");
    only(&GLANCE.replace("{{row.glasses}}", "{{row.cups}}"), "E_BIND_COLUMN");
    only(&GLANCE.replace("{{state.goal}}", "{{state.target}}"), "E_BIND_UNKNOWN");
    only(&GLANCE.replace("title: \"Water:", "title: { each: week }\n  subtitle: \"Water:"), "E_GLANCE_EACH");
    only(&GLANCE.replace("take: 7", "take: 0"), "E_GLANCE_EACH");
}

#[test]
fn a_glance_card_warns_of_fields_it_would_drop() {
    let d = only(&GLANCE.replace("title: \"Water:", "subtitle: \"Water:"), "W_GLANCE_KEY");
    assert!(d.message.contains("subtitle"), "{}", d.message);
    only(&GLANCE.replace("value: \"{{row.glasses}}\"", "value: \"{{row.glasses}}\", note: x"), "W_GLANCE_KEY");
}

#[test]
fn a_patch_replaces_the_glance_card_whole() {
    let patched = enee_spec::patch::apply(GLANCE, "glance:\n  type: stats\n  items: [ { label: Today, value: \"{{today.0.glasses}}\" } ]\n").unwrap();
    let out = compile(&patched, &Options::default());
    let glance = out.app.expect("compiles").glance.expect("a glance card");
    assert_eq!(glance["type"], "stats");
    assert!(glance.get("title").is_none(), "the old card's title went with it");
}

#[test]
fn rows_of_unknown_shape_take_paths() {
    let feed = r#"enee: 1
app: { id: wx, name: Weather }
permissions:
  net.http: { hosts: [wttr.in], reason: Weather }
services:
  now: { use: http.json, url: "https://wttr.in/?format=j1" }
views:
  main:
    - list: { data: services.now.weather, title: "{{row.date}}", caption: "{{row.hourly.4.weatherDesc.0.value}}" }
"#;
    let errors: Vec<Diagnostic> = diags(feed).into_iter().filter(|d| d.severity == Severity::Error).collect();
    assert_eq!(errors, Vec::<Diagnostic>::new());
    // A query's rows are its columns: no paths inside them.
    only(&GLANCE.replace("{{row.glasses}}", "{{row.glasses.total}}"), "E_BIND_ROW");
}
