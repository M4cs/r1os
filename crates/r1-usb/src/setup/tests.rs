use super::fake::{Fake, violations};
use super::*;

/// A folder of inputs, removed afterwards.
struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An arm64 ELF header and then [rest].
fn arm64(rest: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[..4].copy_from_slice(b"\x7fELF");
    (b[4], b[5], b[18]) = (2, 1, 183);
    b.extend_from_slice(rest);
    b
}

fn put(dir: &Path, rel: &str, b: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b).unwrap();
}

/// Everything the setup takes, small: a name with a space and a `!` among the Translator's (espeak-ng's voices have
/// them).
fn inputs(name: &str) -> (Temp, Inputs) {
    let d = std::env::temp_dir().join(format!("r1-usb-setup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    put(&d, "r1d", &arm64(b"r1d one"));
    put(&d, "dist/index.html", b"<!doctype html>");
    put(&d, "dist/assets/app.js", b"r1()");
    put(&d, "lib/libsherpa-onnx-c-api.so", b"sherpa");
    put(&d, "lib/libonnxruntime.so", b"ort");
    for f in ["encoder.ort", "decoder.ort", "joiner.ort", "tokens.txt"] {
        put(&d, &format!("stt/{f}"), f.as_bytes());
    }
    put(&d, "system1/model.int8.onnx", b"minilm");
    put(&d, "system1/vocab.txt", b"[PAD]");
    put(&d, "translate/asr/encoder.int8.onnx", b"whisper");
    put(&d, "translate/tts/espeak-ng-data/voices/!v/Mr serious", b"voice");
    put(&d, "box/alpine-minirootfs-3.24.2-aarch64.tar.gz", &[0x1f, 0x8b, 8, 0]);
    put(&d, "box/codex.tar.gz", &[0x1f, 0x8b, 8, 1]);
    put(&d, "box/codex-host.tar.gz", &[0x1f, 0x8b, 8, 2]);
    put(&d, "box/busybox", &arm64(b"busybox"));
    let i = Inputs {
        r1d: d.join("r1d"),
        shell: d.join("dist"),
        speech_libs: d.join("lib"),
        speech_model: d.join("stt"),
        system1: Some(d.join("system1")),
        translator: Some(d.join("translate")),
        agent_box: Some(BoxInputs {
            alpine: d.join("box/alpine-minirootfs-3.24.2-aarch64.tar.gz"),
            codex: Some(d.join("box/codex.tar.gz")),
            codex_host: Some(d.join("box/codex-host.tar.gz")),
            busybox: Some(d.join("box/busybox")),
        }),
    };
    (Temp(d), i)
}

fn quiet() -> impl FnMut(Progress) {
    |_| {}
}

/// Sets [r1] up from [i], as a test's starting point.
fn set_up(r1: &Fake, i: &Inputs) -> Status {
    let plan = Setup::new(i.clone()).unwrap().plan(&status(r1).unwrap()).unwrap();
    let s = run(r1, &plan, &mut quiet()).unwrap();
    r1.ops.borrow_mut().clear();
    s
}

#[test]
fn a_fresh_r1_gets_every_step_in_order() {
    let (_t, i) = inputs("fresh");
    let setup = Setup::new(i).unwrap();
    let r1 = Fake::fresh();
    let s = status(&r1).unwrap();
    assert!(!s.data.grown && s.restarter == Restarter::Init && s.r1d.is_none() && !s.r1d_running && s.online == Some(true));
    assert_eq!(s.image.as_deref(), Some("0.2.18"));
    let plan = setup.plan(&s).unwrap();
    use Step::*;
    let want =
        [GrowData { device: "/dev/block/dm-48".into() }, Speech, System1, Translator, R1d, Shell, RestartR1d, AgentExec, Busybox, AgentBox];
    assert_eq!(plan.steps(), want);
    assert!(plan.already().is_empty());

    r1.ops.borrow_mut().clear();
    let (mut steps, mut lines) = (Vec::new(), Vec::new());
    let after = run(&r1, &plan, &mut |p| match p {
        Progress::Step { index, of, words } => steps.push((index, of, words.to_owned())),
        Progress::Line(l) => lines.push(l.to_owned()),
    })
    .unwrap();
    assert_eq!(
        r1.summary(),
        [
            "read Facts", "read Hashes", "read Sizes", "read Box", // the status again: is the plan still the same?
            "Clear",
            "read Facts", "Grow", "read Facts", // /data read again right before, and after
            "push dir @S/lib", "push dir @S/stt", "Speech",
            "push dir @S/system1", "System1",
            "push dir @S/translate", "Translator",
            "push file @S/r1d", "R1d",
            "push dir @S/shell", "Shell",
            "read Up", "Stop", "read Up",
            "push script @S/agent-exec", "AgentExec",
            "push file @S/busybox", "Busybox",
            "push file @S/alpine.tar.gz", "push file @S/codex.tar.gz", "push file @S/codex-host.tar.gz", "push script @S/setup-rootfs.sh", "BuildBox",
            "Clear",
            "read Facts", "read Hashes", "read Sizes", "read Box", // and once more: anything left?
        ]
    );
    assert_eq!(steps.iter().map(|(n, of, _)| (*n, *of)).collect::<Vec<_>>(), (1..=10).map(|n| (n, 10)).collect::<Vec<_>>());
    assert!(lines.iter().any(|l| l == "== packages"), "the box's build is heard: {lines:?}");
    assert!(r1.bad.borrow().is_empty(), "{:?}", r1.bad.borrow());

    // What's there now is what was given.
    assert!(after.data.grown && after.r1d_running && after.r1d.as_ref().unwrap().version.as_deref() == Some("0.1.0"));
    let b = &after.agent_box;
    assert!(b.rootfs && b.user && b.packages && b.codex && b.codex_host && b.agent_exec && b.busybox == crate::setup::Busybox::Ours);
    assert_eq!((b.alpine.as_deref(), b.claude.as_deref()), (Some("3.24.2"), Some("2.0.14")));
    let st = r1.s.borrow();
    assert_eq!(st.files["bin/r1d"], arm64(b"r1d one"));
    assert_eq!(st.files["bin/agent-exec"], AGENT_EXEC.as_bytes());
    assert_eq!(st.files["models/translate/tts/espeak-ng-data/voices/!v/Mr serious"], b"voice");
    assert!(st.files.contains_key("shell/assets/app.js") && st.files.contains_key("models/stt/tokens.txt"));
    assert!(st.staged.is_empty(), "nothing left in the staging folder");
}

#[test]
fn the_commands_word_for_word() {
    assert_eq!(STAGE, "/data/local/tmp/r1os-setup");
    let (_t, i) = inputs("words");
    let r1 = Fake::fresh();
    set_up_and_keep_ops(&r1, &i);
    let want = [
        "rm -rf @S",
        "resize2fs /dev/block/dm-48 2>&1; rc=$?; sync; exit $rc",
        concat!(
            r#"mkdir -p /data/r1os/lib && for f in @S/lib/*; do t=/data/r1os/lib/${f##*/}; cmp -s "$f" "$t" || { cp -f "$f" "$t.new" && mv -f "$t.new" "$t"; } || exit 1; done && "#,
            r#"rm -rf /data/r1os/models/stt && mkdir -p /data/r1os/models/stt && cp -f @S/stt/* /data/r1os/models/stt/ && "#,
            r#"chown -R 0:0 /data/r1os/lib /data/r1os/models && chmod -R go-rwx /data/r1os/lib /data/r1os/models && "#,
            r#"restorecon -RF /data/r1os/lib /data/r1os/models && rm -rf @S/lib @S/stt"#,
        ),
        concat!(
            r#"mkdir -p /data/r1os/models/system1 && cp -f @S/system1/* /data/r1os/models/system1/ && "#,
            r#"chown -R 0:0 /data/r1os/models/system1 && chmod -R go-rwx /data/r1os/models/system1 && "#,
            r#"restorecon -RF /data/r1os/models/system1 && rm -rf @S/system1"#,
        ),
        concat!(
            r#"cd @S/translate && find . -type f | while read -r f; do t=/data/r1os/models/translate/${f#./}; cmp -s "$f" "$t" || { mkdir -p "${t%/*}" && mv -f "$f" "$t"; } || exit 1; done && "#,
            r#"cd / && chown -R 0:0 /data/r1os/models/translate && chmod -R go-rwx /data/r1os/models/translate && "#,
            r#"restorecon -RF /data/r1os/models/translate && rm -rf @S/translate"#,
        ),
        concat!(
            r#"mkdir -p /data/r1os/bin /data/r1os/log && mv -f @S/r1d /data/r1os/bin/r1d && chmod 755 /data/r1os/bin/r1d && "#,
            r#"chown 0:0 /data/r1os && chmod 700 /data/r1os && chown -R 0:0 /data/r1os/bin && chmod -R go-rwx /data/r1os/bin && "#,
            r#"restorecon -RF /data/r1os/bin"#,
        ),
        concat!(
            r#"mkdir -p /data/r1os && rm -rf /data/r1os/shell && mv @S/shell /data/r1os/shell && chown 0:0 /data/r1os && chmod 700 /data/r1os && "#,
            r#"chown -R 0:0 /data/r1os/shell && chmod -R go-rwx /data/r1os/shell && restorecon -RF /data/r1os/shell"#,
        ),
        "pkill -x r1d; true",
        concat!(
            r#"mkdir -p /data/r1os/bin && mv -f @S/agent-exec /data/r1os/bin/agent-exec && chown 0:0 /data/r1os/bin/agent-exec && "#,
            r#"chmod 700 /data/r1os/bin/agent-exec && restorecon -F /data/r1os/bin/agent-exec"#,
        ),
        concat!(
            r#"mkdir -p /data/r1os/bin && mv -f @S/busybox /data/r1os/bin/busybox && chown 0:0 /data/r1os/bin/busybox && "#,
            r#"chmod 700 /data/r1os/bin/busybox && restorecon -F /data/r1os/bin/busybox && /data/r1os/bin/busybox | head -n 1"#,
        ),
        concat!(
            r#"sh @S/setup-rootfs.sh @S/alpine.tar.gz @S/codex.tar.gz @S/codex-host.tar.gz; rc=$?; "#,
            r#"rm -f @S/alpine.tar.gz @S/codex.tar.gz @S/codex-host.tar.gz @S/setup-rootfs.sh; "#,
            r#"restorecon -RF /data/r1os/bin /data/r1os/agent; exit $rc"#,
        ),
        "rm -rf @S",
    ];
    assert_eq!(r1.writes(), want);
}

fn set_up_and_keep_ops(r1: &Fake, i: &Inputs) {
    let plan = Setup::new(i.clone()).unwrap().plan(&status(r1).unwrap()).unwrap();
    run(r1, &plan, &mut quiet()).unwrap();
}

#[test]
fn nothing_is_done_twice() {
    let (_t, i) = inputs("twice");
    let r1 = Fake::fresh();
    set_up(&r1, &i);
    let setup = Setup::new(i).unwrap();
    let plan = setup.plan(&status(&r1).unwrap()).unwrap();
    assert!(plan.is_empty(), "{:?}", plan.describe());
    assert_eq!(plan.already().len(), 10, "{:?}", plan.already());
    assert!(plan.already().iter().any(|a| a.starts_with("/data is full size")));
    assert!(plan.already().iter().any(|a| a == "The agent box (Alpine 3.24.2, Claude Code 2.0.14)"));
    r1.ops.borrow_mut().clear();
    run(&r1, &plan, &mut quiet()).unwrap();
    assert!(r1.writes().is_empty(), "{:?}", r1.summary());
    assert!(r1.summary().iter().all(|s| s.starts_with("read ")), "{:?}", r1.summary());
}

#[test]
fn only_what_changed_is_done() {
    let (t, i) = inputs("changed");
    let r1 = Fake::fresh();
    set_up(&r1, &i);

    // A new r1d: just it, and the restart.
    put(&t.0, "r1d", &arm64(b"r1d two"));
    let plan = Setup::new(i.clone()).unwrap().plan(&status(&r1).unwrap()).unwrap();
    assert_eq!(plan.steps(), [Step::R1d, Step::RestartR1d]);
    run(&r1, &plan, &mut quiet()).unwrap();
    assert_eq!(r1.summary().iter().filter(|s| !s.starts_with("read ")).collect::<Vec<_>>(), ["Clear", "push file @S/r1d", "R1d", "Stop", "Clear"]);
    assert_eq!(r1.s.borrow().files["bin/r1d"], arm64(b"r1d two"));
    r1.ops.borrow_mut().clear();

    // One of the Translator's files a different size: the Translator, and the restart.
    put(&t.0, "translate/asr/encoder.int8.onnx", b"whisper, newer");
    let plan = Setup::new(i.clone()).unwrap().plan(&status(&r1).unwrap()).unwrap();
    assert_eq!(plan.steps(), [Step::Translator, Step::RestartR1d]);

    // r1d stopped, nothing new: just started again (and nothing pushed).
    let r1 = Fake::fresh();
    set_up(&r1, &i);
    r1.s.borrow_mut().pids.clear();
    let plan = Setup::new(i.clone()).unwrap().plan(&status(&r1).unwrap()).unwrap();
    assert_eq!(plan.steps(), [Step::RestartR1d]);
    assert_eq!(plan.describe(), ["Start r1d, and wait for it"]);
    run(&r1, &plan, &mut quiet()).unwrap();
    assert!(!r1.summary().iter().any(|s| s.starts_with("push") || s == "Clear"), "{:?}", r1.summary());

    // Magisk's BusyBox on the R1 and none given: no BusyBox step.
    let r1 = Fake::fresh();
    r1.s.borrow_mut().magisk_busybox = true;
    let mut no_bb = i.clone();
    no_bb.agent_box.as_mut().unwrap().busybox = None;
    let plan = Setup::new(no_bb).unwrap().plan(&status(&r1).unwrap()).unwrap();
    assert!(!plan.steps().contains(&Step::Busybox));
    assert!(plan.already().iter().any(|a| a.contains("Magisk's BusyBox")));

    // No agent box asked for: nothing of the box's.
    let mut no_box = i.clone();
    no_box.agent_box = None;
    let plan = Setup::new(no_box).unwrap().plan(&status(&Fake::fresh()).unwrap()).unwrap();
    assert!(!plan.steps().iter().any(|s| matches!(s, Step::AgentExec | Step::Busybox | Step::AgentBox)));
}

#[test]
fn bad_inputs_are_refused() {
    let (t, i) = inputs("bad");
    let d = &t.0;
    assert!(Setup::new(i.clone()).is_ok());
    let refused = |change: &dyn Fn(&mut Inputs), why: &str| {
        let mut j = i.clone();
        change(&mut j);
        let e = format!("{:#}", Setup::new(j).unwrap_err());
        assert!(e.contains(why), "{e}");
    };
    put(d, "bad/r1d.sh", b"#!/bin/sh\nexec /system/bin/r1d --harness\n");
    refused(&|j| j.r1d = d.join("bad/r1d.sh"), "isn't an arm64 program");
    put(d, "bad/tiny", b"\x7fELF");
    refused(&|j| j.r1d = d.join("bad/tiny"), "too short to be a program");
    let mut x86 = arm64(b"r1d");
    x86[18] = 62;
    put(d, "bad/r1d-x86", &x86);
    refused(&|j| j.r1d = d.join("bad/r1d-x86"), "isn't an arm64 program");
    refused(&|j| j.r1d = d.join("nothing"), "no ");
    put(d, "bad/dist/app.html", b"");
    refused(&|j| j.shell = d.join("bad/dist"), "no index.html");
    refused(&|j| j.shell = d.join("nothing"), "no folder");
    put(d, "bad/lib/libsherpa-onnx-c-api.so", b"");
    refused(&|j| j.speech_libs = d.join("bad/lib"), "no libonnxruntime.so");
    put(d, "bad/stt/encoder.ort", b"");
    put(d, "bad/stt/decoder.ort", b"");
    put(d, "bad/stt/tokens.txt", b"");
    refused(&|j| j.speech_model = d.join("bad/stt"), "no joiner");
    put(d, "bad/system1/model.int8.onnx", b"");
    put(d, "bad/system1/vocab.txt", b"");
    put(d, "bad/system1/more/x", b"");
    refused(&|j| j.system1 = Some(d.join("bad/system1")), "where only files belong");
    put(d, "bad/translate/mt/x", b"");
    refused(&|j| j.translator = Some(d.join("bad/translate")), "no asr/encoder.int8.onnx");
    std::fs::create_dir_all(d.join("bad/empty")).unwrap();
    refused(&|j| j.translator = Some(d.join("bad/empty")), "is empty");
    put(d, "bad/alpine.tar", b"tar");
    refused(&|j| j.agent_box.as_mut().unwrap().alpine = d.join("bad/alpine.tar"), "isn't a .tar.gz");
    refused(&|j| j.agent_box.as_mut().unwrap().codex = None, "code-mode host without Codex");
    refused(&|j| j.agent_box.as_mut().unwrap().busybox = Some(d.join("bad/r1d.sh")), "isn't an arm64 program");

    for n in ["a\nb", "tab\there", " lead", "trail ", ".", "..", "", "back\\slash"] {
        assert!(!plain_name(n), "{n:?}");
    }
    for n in ["Mr serious", "!v", "index-3f2a.js", "pt-BR", "model.int8.onnx"] {
        assert!(plain_name(n), "{n:?}");
    }
}

#[test]
fn what_the_r1_says_can_stop_the_plan() {
    let (_t, i) = inputs("stops");
    let setup = Setup::new(i.clone()).unwrap();
    let plan_err = |r1: &Fake| format!("{:#}", setup.plan(&status(r1).unwrap()).unwrap_err());

    // /data small and not ext4: not grown, and no setup on it.
    let r1 = Fake::fresh();
    r1.s.borrow_mut().fs_type = "f2fs".into();
    assert!(plan_err(&r1).contains("rather than ext4"));
    // Grown already, whatever its filesystem: fine.
    r1.s.borrow_mut().fs = 114_000_000_000;
    assert!(setup.plan(&status(&r1).unwrap()).is_ok());

    // Nothing to start r1d.
    let r1 = Fake::fresh();
    r1.s.borrow_mut().init = false;
    assert_eq!(status(&r1).unwrap().restarter, Restarter::None);
    assert!(plan_err(&r1).contains("nothing on this R1 starts r1d"));
    r1.s.borrow_mut().magisk_loop = true;
    assert_eq!(status(&r1).unwrap().restarter, Restarter::Magisk);
    assert!(setup.plan(&status(&r1).unwrap()).is_ok());

    // Offline: the box can't be built; without the box, the rest can.
    let r1 = Fake::fresh();
    r1.s.borrow_mut().net = "Active default network: none".into();
    assert_eq!(status(&r1).unwrap().online, Some(false));
    assert!(plan_err(&r1).contains("isn't online"));
    let mut no_box = i.clone();
    no_box.agent_box = None;
    assert!(Setup::new(no_box).unwrap().plan(&status(&r1).unwrap()).is_ok());
    r1.s.borrow_mut().net = String::new();
    assert_eq!(status(&r1).unwrap().online, None, "unknown isn't offline");

    // No BusyBox on the R1, none given.
    let mut no_bb = i.clone();
    no_bb.agent_box.as_mut().unwrap().busybox = None;
    let e = format!("{:#}", Setup::new(no_bb).unwrap().plan(&status(&Fake::fresh()).unwrap()).unwrap_err());
    assert!(e.contains("no BusyBox"), "{e}");

    // /data's block device as the R1 names it: only a plain /dev/block/ path goes into a command.
    for bad in ["/dev/block/../sda", "/dev/sda", "/dev/block/dm-1;reboot", "/dev/block/dm 1", "/dev/block/", "", "/dev/block/dm-1/"] {
        let r1 = Fake::fresh();
        r1.s.borrow_mut().device = bad.into();
        assert!(format!("{:#}", status(&r1).unwrap_err()).contains("block device"), "{bad}");
    }
    assert!(block_device("/dev/block/by-name/userdata").is_some());
}

#[test]
fn the_run_stops_when_the_r1_changed_or_a_step_fails() {
    let (_t, i) = inputs("run-stops");
    let setup = Setup::new(i.clone()).unwrap();

    // A plan from the assumed fresh R1, not read from one.
    let r1 = Fake::fresh();
    let assumed = setup.plan(&Status::fresh()).unwrap();
    assert!(format!("{:#}", run(&r1, &assumed, &mut quiet()).unwrap_err()).contains("wasn't made from"));
    assert!(r1.ops.borrow().is_empty());

    // /data grown between the plan and the run: the plan is different now, so nothing is done.
    let plan = setup.plan(&status(&r1).unwrap()).unwrap();
    r1.s.borrow_mut().fs = 114_000_000_000;
    r1.ops.borrow_mut().clear();
    assert!(format!("{:#}", run(&r1, &plan, &mut quiet()).unwrap_err()).contains("changed since the plan"));
    assert!(r1.writes().is_empty());

    // resize2fs fails: stopped there, nothing pushed after it.
    let r1 = Fake::fresh();
    r1.s.borrow_mut().grow_fails = true;
    let plan = setup.plan(&status(&r1).unwrap()).unwrap();
    let e = format!("{:#}", run(&r1, &plan, &mut quiet()).unwrap_err());
    assert!(e.contains("Grow /data") && e.contains("Permission denied"), "{e}");
    let ops = r1.summary();
    let grow = ops.iter().position(|o| o == "Grow").unwrap();
    assert!(!ops[grow..].iter().any(|o| o.starts_with("push")), "{ops:?}");

    // r1d never comes back: a minute of polls, then an error that says where to look.
    let r1 = Fake::fresh();
    r1.s.borrow_mut().crashes = true;
    let plan = setup.plan(&status(&r1).unwrap()).unwrap();
    let e = format!("{:#}", run(&r1, &plan, &mut quiet()).unwrap_err());
    assert!(e.contains("didn't come back") && e.contains("r1d.log"), "{e}");
    assert_eq!(*r1.waits.borrow(), 120);
    assert!(!r1.summary().iter().any(|o| o == "BuildBox"), "nothing after the failed step");
}

#[test]
fn the_status_only_reads() {
    let (_t, i) = inputs("reads");
    let r1 = Fake::fresh();
    set_up(&r1, &i);
    let s = status(&r1).unwrap();
    assert!(r1.summary().iter().all(|o| o.starts_with("read ")), "{:?}", r1.summary());
    assert_eq!(r1.s.borrow().pids.len(), 1, "r1d left running");
    assert_eq!(s.r1d.unwrap().sha256, sha256_hex(&arm64(b"r1d one")));
    let json = serde_json::to_string(&Status::fresh()).unwrap();
    assert!(json.contains("\"grown\":false") && !json.contains("files"));
}

#[test]
fn every_command_keeps_to_the_rules() {
    let dev = "/dev/block/dm-48";
    let all = [FACTS, HASHES, SIZES, BOX, UP, CLEAR, SPEECH, SYSTEM1, TRANSLATOR, R1D, SHELL, STOP, AGENT_EXEC_IN, BUSYBOX_IN, BUILD_BOX];
    for t in all {
        let c = Cmd::new(Kind::Facts, t);
        assert_eq!(violations(c.text(), dev), Vec::<String>::new(), "{t}");
        assert!(!c.text().contains("@S") && !c.text().contains("@D"));
    }
    let g = grow(&block_device(dev).unwrap());
    assert_eq!(violations(g.text(), dev), Vec::<String>::new());
    assert!(!violations(g.text(), "/dev/block/dm-12").is_empty(), "only /data's own device");

    // And the rules catch what they're for.
    for bad in [
        "chown -R 0:0 /data/r1os",
        "chmod -R go-rwx /data/r1os/",
        "chown -R 2950:2950 /data/r1os/agent/home",
        "chmod -R 700 /data/r1os/agent",
        "rm -rf /data/r1os/agent",
        "rm -rf /data/r1os",
        "rm -rf /data/media/0",
        "cp -f /data/local/tmp/x /system/bin/x",
        "mv /data/r1os/bin/r1d /data/local/tmp/elsewhere",
        "mkdir -p /data/r1os/../adb",
        "dd if=/data/local/tmp/x of=/dev/block/by-name/boot_a",
        "echo 0 > /sys/class/motor/angle",
        "mv -f $X /data/r1os/bin/r1d",
        "resize2fs /dev/block/mmcblk0",
        "sh /data/local/tmp/anything.sh",
        "pkill -f r1d",
        "find /data/r1os -delete",
        "find /data/r1os -exec rm {} +",
        "setprop persist.sys.x 1",
        "reboot bootloader",
        "echo 'x'",
    ] {
        assert!(!violations(bad, dev).is_empty(), "{bad}");
    }
}

#[test]
fn the_box_s_scripts_are_compiled_in_as_the_r1_runs_them() {
    for s in [AGENT_EXEC, SETUP_ROOTFS] {
        assert!(s.starts_with("#!/system/bin/sh\n"));
        assert!(!s.contains('\r'), "LF only: the R1's sh reads a CR as part of the word");
    }
    // setup-rootfs.sh runs the box through agent-exec, which is why that step comes first.
    assert!(SETUP_ROOTFS.contains("EXEC=/data/r1os/bin/agent-exec"));
    assert!(SETUP_ROOTFS.contains("ROOTFS_TGZ=$1") && SETUP_ROOTFS.contains("CODEX_TGZ=$2") && SETUP_ROOTFS.contains("CODEX_HOST_TGZ=${3:-}"));
}

#[test]
fn what_the_r1_prints_is_read_right() {
    let h = "a".repeat(64);
    let parsed = parse_hashes(&format!("{h}  ./bin/r1d\n{h}  shell/a b.js\n{h} *lib/x.so\nnot a hash  x\n"));
    assert_eq!(parsed.keys().collect::<Vec<_>>(), ["bin/r1d", "lib/x.so", "shell/a b.js"]);
    let sizes = parse_sizes("12 ./models/translate/tts/espeak-ng-data/voices/!v/Mr serious\nx y\n");
    assert_eq!(sizes["models/translate/tts/espeak-ng-data/voices/!v/Mr serious"], 12);
    assert_eq!(sizes.len(), 1);
    let line = "2026-10-14T10:00:00.123456Z  INFO r1d: r1d starting version=\"0.1.0\" sim=false wheel=Some(\"/dev/input/event2\")";
    assert_eq!(logged_version(line).as_deref(), Some("0.1.0"));
    assert_eq!(logged_version("r1d starting version=0.2.0 sim=false").as_deref(), Some("0.2.0"));
    assert_eq!(logged_version("== r1d start Tue Oct 14"), None);
    assert_eq!(pids(Some(&"123 456".to_owned())), [123, 456]);
    assert!(pids(Some(&String::new())).is_empty());
    assert_eq!((gb(2_818_572_288), gb(115_000_000_000), bytes(18_385_560), bytes(4_200)), ("2.8 GB".into(), "115 GB".into(), "18 MB".into(), "4 KB".into()));
}

#[test]
fn the_plan_in_words() {
    let (_t, i) = inputs("words-plan");
    let plan = Setup::new(i).unwrap().plan(&Status::fresh()).unwrap();
    let w = plan.describe();
    assert_eq!(w.len(), plan.steps().len());
    assert_eq!(w[0], "Grow /data to fill its partition: its filesystem is 2.8 GB of 115 GB (online; nothing is erased)");
    assert!(w.iter().any(|l| l.starts_with("The Translator's models (")));
    assert!(w.last().unwrap().starts_with("Build the agent box: Alpine 3.24.2 (4 bytes), Codex, and Claude Code"));
}

#[test]
fn a_checkout_s_paths() {
    let d = std::env::temp_dir().join(format!("r1-usb-setup-checkout-{}", std::process::id()));
    let _t = Temp(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    put(&d, "firmware/agent-box/alpine-minirootfs-3.24.2-aarch64.tar.gz", b"");
    put(&d, "firmware/agent-box/codex-code-mode-host-rust-v0.162.0.tar.gz", b"");
    put(&d, "firmware/agent-box/codex-rust-v0.162.0.tar.gz", b"");
    put(&d, "out/system1/model.int8.onnx", b"");
    let i = Inputs::from_checkout(&d);
    assert!(i.r1d.ends_with("target/aarch64-linux-android/release/r1d"));
    assert!(i.speech_model.ends_with("out/speech/stt-r1"));
    assert!(i.system1.is_some() && i.translator.is_none(), "models only if they're there, as deploy.sh takes them");
    let b = i.agent_box.unwrap();
    assert!(b.codex.unwrap().ends_with("codex-rust-v0.162.0.tar.gz"));
    assert!(b.codex_host.unwrap().ends_with("codex-code-mode-host-rust-v0.162.0.tar.gz"));
    assert!(b.alpine.ends_with("alpine-minirootfs-3.24.2-aarch64.tar.gz") && b.busybox.is_none());
}
