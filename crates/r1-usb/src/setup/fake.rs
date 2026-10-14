//! A pretend R1 for the setup's tests. It answers the setup's reads from its state, changes that state the way each
//! write would on an R1, records every command and push, and holds each command to the setup's rules ([violations]).

use super::*;
use std::cell::RefCell;
use std::fmt::Write as _;

/// The pretend R1's state.
#[derive(Debug, Clone)]
pub(super) struct State {
    pub image: Option<String>,
    /// init's `r1d` service (r1os's image).
    pub init: bool,
    /// r1os-base's loop (Magisk).
    pub magisk_loop: bool,
    pub magisk_busybox: bool,
    /// `dumpsys connectivity`'s line.
    pub net: String,
    pub device: String,
    pub fs_type: String,
    pub partition: u64,
    pub fs: u64,
    /// `/data/r1os`, by path under it.
    pub files: BTreeMap<String, Vec<u8>>,
    /// What's been pushed into the staging folder, by where it went: (path under it, contents).
    pub staged: BTreeMap<String, Vec<(String, Vec<u8>)>>,
    pub pids: Vec<u32>,
    pub next_pid: u32,
    pub listening: bool,
    /// r1d exits as soon as it starts.
    pub crashes: bool,
    pub grow_fails: bool,
    pub rootfs: bool,
    pub user: bool,
    pub packages: bool,
    pub codex: bool,
    pub codex_host: bool,
    pub claude: Option<String>,
}

/// One thing done to the pretend R1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Op {
    Root(Kind, String),
    Push(&'static str, String),
}

pub(super) struct Fake {
    pub s: RefCell<State>,
    pub ops: RefCell<Vec<Op>>,
    /// Every rule a command broke ([violations]).
    pub bad: RefCell<Vec<String>>,
    pub waits: RefCell<u32>,
}

impl Fake {
    /// Just flashed with r1os's image: init's r1d service waiting for a binary, `/data` still Rabbit's 2.8 GB ext4 on
    /// its 115 GB partition, online, no Magisk.
    pub fn fresh() -> Fake {
        Fake {
            s: RefCell::new(State {
                image: Some("0.2.18".into()),
                init: true,
                magisk_loop: false,
                magisk_busybox: false,
                net: "Active default network: 100".into(),
                device: "/dev/block/dm-48".into(),
                fs_type: "ext4".into(),
                partition: 115_000_000_000,
                fs: 2_818_572_288,
                files: BTreeMap::new(),
                staged: BTreeMap::new(),
                pids: Vec::new(),
                next_pid: 4000,
                listening: false,
                crashes: false,
                grow_fails: false,
                rootfs: false,
                user: false,
                packages: false,
                codex: false,
                codex_host: false,
                claude: None,
            }),
            ops: RefCell::default(),
            bad: RefCell::default(),
            waits: RefCell::default(),
        }
    }

    /// What was done, short: `read Facts`, `Speech`, `push dir @S/lib`…
    pub fn summary(&self) -> Vec<String> {
        self.ops
            .borrow()
            .iter()
            .map(|o| match o {
                Op::Root(k, _) if k.writes() => format!("{k:?}"),
                Op::Root(k, _) => format!("read {k:?}"),
                Op::Push(what, to) => format!("push {what} {}", to.replace(STAGE, "@S")),
            })
            .collect()
    }

    /// The commands that wrote, word for word.
    pub fn writes(&self) -> Vec<String> {
        self.ops
            .borrow()
            .iter()
            .filter_map(|o| match o {
                Op::Root(k, t) if k.writes() => Some(t.replace(STAGE, "@S")),
                _ => None,
            })
            .collect()
    }

    /// The staged files at [to] moved into `/data/r1os/<prefix>/`.
    fn take(s: &mut State, to: &str, prefix: &str) -> bool {
        let Some(files) = s.staged.remove(&format!("{STAGE}/{to}")) else { return false };
        for (rel, b) in files {
            let path = if rel.is_empty() { prefix.to_owned() } else { format!("{prefix}/{rel}") };
            s.files.insert(path, b);
        }
        true
    }

    fn answer(&self, cmd: &Cmd, line: &mut dyn FnMut(&str)) -> Output {
        let mut s = self.s.borrow_mut();
        let mut out = String::new();
        let mut code = 0;
        let sha = |b: &[u8]| sha256_hex(b);
        match cmd.kind() {
            Kind::Facts => {
                let _ = writeln!(out, "image={}", s.image.clone().unwrap_or_default());
                let _ = writeln!(out, "init_r1d={}", if s.init { "running" } else { "" });
                if s.magisk_loop {
                    out.push_str("magisk_loop=1\n");
                }
                let pids: Vec<String> = s.pids.iter().map(u32::to_string).collect();
                let _ = writeln!(out, "r1d_pid={}", pids.join(" "));
                let _ = writeln!(out, "data_dev={}\ndata_type={}\ndata_dev_bytes={}", s.device, s.fs_type, s.partition);
                let _ = writeln!(out, "data_fs={} 4096", s.fs / 4096);
                let _ = writeln!(out, "net={}", s.net);
                if s.files.contains_key("bin/r1d") {
                    out.push_str("r1d_log=2026-10-14T10:00:00.000000Z  INFO r1d: r1d starting version=\"0.1.0\" sim=false motor_sim=false\n");
                }
                if s.magisk_busybox {
                    out.push_str("magisk_busybox=1\n");
                }
            }
            Kind::Hashes => {
                for (p, b) in &s.files {
                    if ["bin/", "lib/", "shell/", "models/stt/", "models/system1/"].iter().any(|d| p.starts_with(d)) {
                        let _ = writeln!(out, "{}  ./{p}", sha(b));
                    }
                }
            }
            Kind::Sizes => {
                for (p, b) in &s.files {
                    if p.starts_with("models/translate/") {
                        let _ = writeln!(out, "{} ./{p}", b.len());
                    }
                }
            }
            Kind::Box => {
                for (k, on) in [("rootfs", s.rootfs), ("user", s.user), ("packages", s.packages), ("codex", s.codex), ("codex_host", s.codex_host)] {
                    if on {
                        let _ = writeln!(out, "{k}=1");
                    }
                }
                let _ = writeln!(out, "alpine={}", if s.rootfs { "3.24.2" } else { "" });
                if let Some(v) = &s.claude {
                    let _ = writeln!(out, "claude=/home/agent/.local/share/claude/versions/{v}");
                }
            }
            Kind::Up => {
                let pids: Vec<String> = s.pids.iter().map(u32::to_string).collect();
                let _ = writeln!(out, "r1d_pid={}", pids.join(" "));
                if s.listening {
                    out.push_str("listening=1\n");
                }
            }
            Kind::Grow if s.grow_fails => {
                out.push_str("resize2fs: Permission denied while trying to resize /dev/block/dm-48\n");
                code = 1;
            }
            Kind::Grow => {
                out.push_str("Filesystem at /dev/block/dm-48 is mounted on /data; on-line resizing required\n");
                s.fs = s.partition / 100 * 98;
            }
            Kind::Clear => s.staged.clear(),
            Kind::Speech => {
                let ok = Self::take(&mut s, "lib", "lib") && {
                    s.files.retain(|p, _| !p.starts_with("models/stt/"));
                    Self::take(&mut s, "stt", "models/stt")
                };
                code = if ok { 0 } else { 1 };
            }
            Kind::System1 => code = if Self::take(&mut s, "system1", "models/system1") { 0 } else { 1 },
            Kind::Translator => code = if Self::take(&mut s, "translate", "models/translate") { 0 } else { 1 },
            Kind::R1d => code = if Self::take(&mut s, "r1d", "bin/r1d") { 0 } else { 1 },
            Kind::Shell => {
                s.files.retain(|p, _| !p.starts_with("shell/"));
                code = if Self::take(&mut s, "shell", "shell") { 0 } else { 1 };
            }
            Kind::Stop => {
                s.pids.clear();
                s.listening = false;
                // init (or r1os-base's loop) starts it again, if there's an r1d and its libraries to start.
                if (s.init || s.magisk_loop) && !s.crashes && s.files.contains_key("bin/r1d") && s.files.contains_key("lib/libsherpa-onnx-c-api.so") {
                    s.pids = vec![s.next_pid];
                    s.next_pid += 1;
                    s.listening = true;
                }
            }
            Kind::AgentExec => code = if Self::take(&mut s, "agent-exec", "bin/agent-exec") { 0 } else { 1 },
            Kind::Busybox => {
                code = if Self::take(&mut s, "busybox", "bin/busybox") { 0 } else { 1 };
                out.push_str("BusyBox v1.36.1 (2025-01-01) multi-call binary.\n");
            }
            Kind::BuildBox => {
                let staged = |n: &str| s.staged.contains_key(&format!("{STAGE}/{n}"));
                let runs = staged("setup-rootfs.sh") && staged("alpine.tar.gz") && s.files.contains_key("bin/agent-exec");
                let (codex, codex_host) = (staged("codex.tar.gz"), staged("codex-host.tar.gz"));
                if runs && (s.magisk_busybox || s.files.contains_key("bin/busybox")) {
                    (s.codex, s.codex_host) = (codex, codex_host);
                    (s.rootfs, s.user, s.packages, s.claude) = (true, true, true, Some("2.0.14".into()));
                    out.push_str("== extracting alpine.tar.gz\n== agent user\n== packages\n== claude code (official installer, as agent)\n");
                } else {
                    out.push_str("agent-exec: no agent box\n");
                    code = 1;
                }
                for n in ["alpine.tar.gz", "codex.tar.gz", "codex-host.tar.gz", "setup-rootfs.sh"] {
                    s.staged.remove(&format!("{STAGE}/{n}"));
                }
            }
        }
        if cmd.kind().writes() {
            for l in out.lines() {
                line(l);
            }
        }
        Output { code, out, err: String::new() }
    }
}

impl Device for Fake {
    fn root(&self, cmd: &Cmd, line: &mut dyn FnMut(&str)) -> Result<Output> {
        self.ops.borrow_mut().push(Op::Root(cmd.kind(), cmd.text().to_owned()));
        let device = self.s.borrow().device.clone();
        self.bad.borrow_mut().extend(violations(cmd.text(), &device).into_iter().map(|v| format!("{:?}: {v}", cmd.kind())));
        Ok(self.answer(cmd, line))
    }

    fn push(&self, p: &Push) -> Result<()> {
        if !p.to().starts_with(&format!("{STAGE}/")) {
            self.bad.borrow_mut().push(format!("a push to {}", p.to()));
        }
        let (what, files) = match p.source() {
            Source::File(f) => ("file", vec![(String::new(), std::fs::read(f)?)]),
            Source::Dir(d) => {
                let mut v = Vec::new();
                read_tree(d, "", &mut v)?;
                ("dir", v)
            }
            Source::Script { text, .. } => ("script", vec![(String::new(), text.as_bytes().to_vec())]),
        };
        self.ops.borrow_mut().push(Op::Push(what, p.to().to_owned()));
        self.s.borrow_mut().staged.insert(p.to().to_owned(), files);
        Ok(())
    }

    fn wait(&self, _: Duration) {
        *self.waits.borrow_mut() += 1;
    }
}

fn read_tree(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if e.file_type()?.is_dir() {
            read_tree(&e.path(), &rel, out)?;
        } else {
            out.push((rel, std::fs::read(e.path())?));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// The rules every command keeps.

/// Where the setup may write: r1os's folder, and its own staging folder.
fn allowed(p: &str) -> bool {
    !p.split('/').any(|x| x == "..")
        && (p == "/data/r1os" || p.starts_with("/data/r1os/") || p == STAGE || p.starts_with(&format!("{STAGE}/")))
}

/// What may be chowned or chmodded recursively: r1os's own folders. Never all of `/data/r1os`, and nothing of the
/// agent's (its home is uid 2950's; a recursive chown broke the box on 2026-10-08).
const RECURSIVE: &[&str] = &[
    "/data/r1os/bin",
    "/data/r1os/shell",
    "/data/r1os/lib",
    "/data/r1os/models",
    "/data/r1os/models/system1",
    "/data/r1os/models/translate",
];

/// A word with a variable at its start replaced by what it was set to (`$t.new`, `${t%/*}`: near enough to tell where
/// it points). An unknown variable stays unknown, which no rule allows.
fn resolve(w: &str, vars: &BTreeMap<String, String>) -> String {
    let Some(rest) = w.strip_prefix('$') else { return w.to_owned() };
    let name_end = |s: &str| s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(s.len());
    let (name, after) = match rest.strip_prefix('{') {
        Some(r) => (&r[..name_end(r)], &r[r.find('}').map_or(r.len(), |i| i + 1)..]),
        None => (&rest[..name_end(rest)], &rest[name_end(rest)..]),
    };
    match vars.get(name) {
        Some(v) => format!("{v}{after}"),
        None => format!("<unknown ${name}>"),
    }
}

/// What's wrong with [cmd] by the setup's rules: every write under `/data/r1os` or [STAGE]; no recursive chown or
/// chmod but of r1os's own folders (never all of `/data/r1os`, never the agent's); no partition (`/dev/block` only as
/// `resize2fs` of /data's own [device]); no program but the ones the templates use; one line, no single quote.
///
/// It reads the templates' simple shell well enough to follow variables set by `X=…`, `for X in …` and `read X` (after
/// a `cd`), which is all they use. It isn't a shell parser: it's there to catch a template that strays.
pub(super) fn violations(cmd: &str, device: &str) -> Vec<String> {
    let mut bad = Vec::new();
    if cmd.contains('\'') {
        bad.push("a single quote (su -c '…' would break)".to_owned());
    }
    if cmd.contains(['\n', '\r']) {
        bad.push("more than one line".to_owned());
    }
    if cmd.contains("/dev/block") && !cmd.starts_with("resize2fs ") {
        bad.push("a block device".to_owned());
    }
    let text = cmd.replace("&&", ";").replace("||", ";").replace("$(", ";").replace('|', ";");
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    let mut cwd = "/".to_owned();
    for piece in text.split(';') {
        let mut words: Vec<String> = piece.split_whitespace().map(|w| w.trim_matches(['"', ')']).to_owned()).collect();
        while words.first().is_some_and(|w| ["do", "then", "else", "if", "while", "{", "}", "done", "fi", "!"].contains(&w.as_str())) {
            words.remove(0);
        }
        let Some(verb) = words.first().cloned() else { continue };
        let mut args = Vec::new();
        for w in &words[1..] {
            match w.find('>') {
                Some(i) => {
                    let to = w[i + 1..].trim_start_matches('>');
                    if to != "/dev/null" && to != "&1" {
                        bad.push(format!("a redirection to {to}"));
                    }
                }
                None => args.push(w.clone()),
            }
        }
        if let Some((name, value)) = verb.split_once('=')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            let v = resolve(value, &vars);
            vars.insert(name.to_owned(), v);
            continue;
        }
        let flags: String = args.iter().filter_map(|a| a.strip_prefix('-')).collect();
        let operands: Vec<String> = args.iter().filter(|a| !a.starts_with('-')).map(|a| resolve(a, &vars)).collect();
        let written: Vec<String> = match verb.as_str() {
            "echo" | "getprop" | "ps" | "grep" | "pidof" | "set" | "blockdev" | "stat" | "dumpsys" | "tail" | "head" | "cat"
            | "readlink" | "[" | "cmp" | "sha256sum" | "true" | "exit" | "sync" | "/data/r1os/bin/busybox" => vec![],
            "sed" if flags.contains('i') => {
                bad.push("sed -i".to_owned());
                vec![]
            }
            "sed" => vec![],
            "find" => {
                let exec_ok = args.windows(2).all(|w| w[0] != "-exec" || ["sha256sum", "stat"].contains(&w[1].as_str()));
                if args.iter().any(|a| a == "-delete") || !exec_ok {
                    bad.push(format!("find {}", args.join(" ")));
                }
                vec![]
            }
            "cd" => {
                cwd = operands.first().cloned().unwrap_or_default();
                vec![]
            }
            "read" => {
                if let Some(n) = operands.last() {
                    vars.insert(n.clone(), format!("{cwd}/"));
                }
                vec![]
            }
            "for" => {
                if let (Some(n), Some(v)) = (operands.first(), operands.get(2)) {
                    vars.insert(n.clone(), v.clone());
                }
                vec![]
            }
            "mkdir" | "rm" | "mv" | "restorecon" => operands.clone(),
            "cp" => operands.last().cloned().into_iter().collect(),
            "chown" | "chmod" => operands.iter().skip(1).cloned().collect(),
            "pkill" => {
                if args != ["-x", "r1d"] {
                    bad.push(format!("pkill {}", args.join(" ")));
                }
                vec![]
            }
            "resize2fs" => {
                if operands != [device] {
                    bad.push(format!("resize2fs {}", operands.join(" ")));
                }
                vec![]
            }
            "sh" => {
                if operands.first() != Some(&format!("{STAGE}/setup-rootfs.sh")) {
                    bad.push(format!("sh {}", operands.join(" ")));
                }
                vec![]
            }
            other => {
                bad.push(format!("an unknown program: {other}"));
                vec![]
            }
        };
        let recursive = flags.contains('R') || (verb == "rm" && flags.contains('r'));
        for p in &written {
            if !allowed(p) {
                bad.push(format!("{verb} writes {p}"));
            }
            let ok = match verb.as_str() {
                _ if !recursive => true,
                // Relabeling is what ADR-0035 asks for, the agent's files included.
                "restorecon" => true,
                "rm" => p != "/data/r1os" && !p.starts_with("/data/r1os/agent"),
                _ => RECURSIVE.contains(&p.as_str()),
            };
            if !ok {
                bad.push(format!("{verb} -R {p}"));
            }
        }
    }
    bad
}
