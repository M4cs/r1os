//! The first setup after the flash (ADR-0065 decision 7, T-170): what `tools/grow-data.sh`, `tools/deploy.sh` and
//! `tools/agent-box.sh install` do from Git Bash, as typed steps the companion's wizard runs over adb.
//!
//! - **What it takes** ([Inputs]): paths, never fixed places. r1d built for the R1, the shell's `dist/`, the
//!   speech-to-text libraries and model, System One's and the Translator's models if chosen, and the agent box's Alpine
//!   minirootfs (with Codex, its code-mode host and a static BusyBox when given). Today they're in `out/`, `shell/dist`
//!   and `firmware/agent-box` ([Inputs::from_checkout]); later a release brings them (T-166), its manifest checking each
//!   one's SHA-256 as it downloads. [Setup::new] checks each is what it says (r1d an arm64 program, the model folders
//!   complete) and reads every file's size and SHA-256.
//! - **What's there** ([status], which only reads): r1os's image, `/data`'s filesystem against its partition, what
//!   restarts r1d, every file r1os keeps in `/data/r1os` by SHA-256 (the Translator's 2.2 GB by size), whether r1d runs
//!   and the version it logged, the agent box, and whether the R1 is online.
//! - **The plan** ([Setup::plan]): only what isn't there yet, in order, each step in words for the screen.
//! - **The run** ([run]): reads the status again and goes on only if the plan is still the same. Each step goes
//!   through a [Device] (adb with root, [OverAdb]; a fake in tests): pushes into [STAGE], then root commands that move
//!   them into `/data/r1os`, chown and chmod r1os's own folders (never all of `/data/r1os`, never `agent/`: CLAUDE.md),
//!   and `restorecon -RF` them for r1d's SELinux domain (ADR-0035). Last it reads the status once more and fails if
//!   anything is still to do.
//!
//! Every command is built here from fixed templates: a [Cmd] can't be made anywhere else, and the one value from the R1
//! that goes into one is `/data`'s block device, checked first. Nothing writes a partition. The one write outside
//! `/data/r1os` and [STAGE] is growing `/data`'s ext4 filesystem online with `resize2fs`, as `grow-data.sh` does: only
//! when it's under 90 % of its partition, checked again right before.

use crate::adb::{Adb, Root};
use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
mod fake;

/// Where the setup stages what it pushes, on the R1: emptied before the first push and after the last.
pub const STAGE: &str = "/data/local/tmp/r1os-setup";

/// The agent box's scripts as this build of r1os has them (ADR-0003). They're compiled in rather than taken from a
/// path, because the setup runs them as root on the R1.
pub const AGENT_EXEC: &str = include_str!("../../../agent-box/agent-exec.sh");
pub const SETUP_ROOTFS: &str = include_str!("../../../agent-box/setup-rootfs.sh");

/// How long r1d gets to come back after a restart: init restarts a service at most every 5 s, and on a fresh R1 the
/// service is still waiting out the missing binary.
const R1D_BACK: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------------------------------------------------
// The commands. `@S` is [STAGE]. Each is one line, with no single quotes (Magisk's `su -c '…'` wraps it).

/// What the R1 is: r1os's image, what restarts r1d (init's service, or r1os-base's loop under Magisk), r1d's pids,
/// `/data`'s block device, filesystem and sizes (as `grow-data.sh` reads them), the default network, the version r1d
/// last logged, and Magisk's BusyBox.
const FACTS: &str = concat!(
    r#"echo "image=$(getprop ro.r1os.image.version)"; echo "init_r1d=$(getprop init.svc.r1d)"; "#,
    r#"ps -A -o ARGS | grep -q "[r]1os_base/service.sh" && echo magisk_loop=1; echo "r1d_pid=$(pidof r1d)"; "#,
    r#"set -- $(grep " /data " /proc/mounts | tail -n 1); echo "data_dev=$1"; echo "data_type=$3"; "#,
    r#"echo "data_dev_bytes=$(blockdev --getsize64 "$1" 2>/dev/null)"; echo "data_fs=$(stat -f -c "%b %S" /data)"; "#,
    r#"echo "net=$(dumpsys connectivity 2>/dev/null | grep -m 1 "Active default network")"; "#,
    r#"grep -h "r1d starting" /data/r1os/log/r1d.log.1 /data/r1os/log/r1d.log 2>/dev/null | tail -n 1 | sed "s/^/r1d_log=/"; "#,
    r#"[ -x /data/adb/magisk/busybox ] && echo magisk_busybox=1; true"#,
);
/// The SHA-256 of every file r1os keeps in `/data/r1os`, but the Translator's.
const HASHES: &str =
    r#"cd /data/r1os 2>/dev/null && find bin lib shell models/stt models/system1 -type f -exec sha256sum {} + 2>/dev/null; true"#;
/// The Translator's files by size: hashing 2.2 GB on the R1 would take most of a minute.
const SIZES: &str = r#"cd /data/r1os 2>/dev/null && find models/translate -type f -exec stat -c "%s %n" {} + 2>/dev/null; true"#;
/// The agent box, without running anything in it: `agent-exec` would write the box's `resolv.conf`.
const BOX: &str = concat!(
    r#"R=/data/r1os/agent/rootfs; [ -x $R/bin/busybox ] && echo rootfs=1; grep -q "^agent:" $R/etc/passwd 2>/dev/null && echo user=1; "#,
    r#"[ -x $R/bin/bash ] && [ -x $R/bin/setpriv ] && echo packages=1; [ -x $R/usr/local/bin/codex ] && echo codex=1; "#,
    r#"[ -x $R/usr/local/bin/codex-code-mode-host ] && echo codex_host=1; echo "alpine=$(cat $R/etc/alpine-release 2>/dev/null)"; "#,
    r#"C=/data/r1os/agent/home/.local/bin/claude; { [ -L $C ] || [ -e $C ]; } && echo "claude=$(readlink $C)"; true"#,
);
/// r1d's pids, and whether anything listens on 127.0.0.1:7171 (`0100007F:1C03`), r1d's port.
const UP: &str = r#"echo "r1d_pid=$(pidof r1d)"; grep -q " 0100007F:1C03 00000000:0000 0A " /proc/net/tcp && echo listening=1; true"#;
/// `grow-data.sh`'s write: online, nothing erased. `@D` is `/data`'s block device, checked.
const GROW: &str = r#"resize2fs @D 2>&1; rc=$?; sync; exit $rc"#;
const CLEAR: &str = r#"rm -rf @S"#;
/// `deploy.sh`'s speech step. r1d has the libraries mapped: an unchanged one is skipped and a changed one goes in under
/// a new name and is renamed over the old (a copy in place crashed r1d, 2026-10-08).
const SPEECH: &str = concat!(
    r#"mkdir -p /data/r1os/lib && for f in @S/lib/*; do t=/data/r1os/lib/${f##*/}; cmp -s "$f" "$t" || { cp -f "$f" "$t.new" && mv -f "$t.new" "$t"; } || exit 1; done && "#,
    r#"rm -rf /data/r1os/models/stt && mkdir -p /data/r1os/models/stt && cp -f @S/stt/* /data/r1os/models/stt/ && "#,
    r#"chown -R 0:0 /data/r1os/lib /data/r1os/models && chmod -R go-rwx /data/r1os/lib /data/r1os/models && "#,
    r#"restorecon -RF /data/r1os/lib /data/r1os/models && rm -rf @S/lib @S/stt"#,
);
const SYSTEM1: &str = concat!(
    r#"mkdir -p /data/r1os/models/system1 && cp -f @S/system1/* /data/r1os/models/system1/ && "#,
    r#"chown -R 0:0 /data/r1os/models/system1 && chmod -R go-rwx /data/r1os/models/system1 && "#,
    r#"restorecon -RF /data/r1os/models/system1 && rm -rf @S/system1"#,
);
/// Only the files that differ are moved in, each renamed over the old (the running r1d may have one open).
const TRANSLATOR: &str = concat!(
    r#"cd @S/translate && find . -type f | while read -r f; do t=/data/r1os/models/translate/${f#./}; cmp -s "$f" "$t" || { mkdir -p "${t%/*}" && mv -f "$f" "$t"; } || exit 1; done && "#,
    r#"cd / && chown -R 0:0 /data/r1os/models/translate && chmod -R go-rwx /data/r1os/models/translate && "#,
    r#"restorecon -RF /data/r1os/models/translate && rm -rf @S/translate"#,
);
/// The running r1d keeps its copy of the binary until the restart: `mv` renames over it.
const R1D: &str = concat!(
    r#"mkdir -p /data/r1os/bin /data/r1os/log && mv -f @S/r1d /data/r1os/bin/r1d && chmod 755 /data/r1os/bin/r1d && "#,
    r#"chown 0:0 /data/r1os && chmod 700 /data/r1os && chown -R 0:0 /data/r1os/bin && chmod -R go-rwx /data/r1os/bin && "#,
    r#"restorecon -RF /data/r1os/bin"#,
);
const SHELL: &str = concat!(
    r#"mkdir -p /data/r1os && rm -rf /data/r1os/shell && mv @S/shell /data/r1os/shell && chown 0:0 /data/r1os && chmod 700 /data/r1os && "#,
    r#"chown -R 0:0 /data/r1os/shell && chmod -R go-rwx /data/r1os/shell && restorecon -RF /data/r1os/shell"#,
);
/// init (r1os's image) or r1os-base's loop starts the new r1d.
const STOP: &str = r#"pkill -x r1d; true"#;
const AGENT_EXEC_IN: &str = concat!(
    r#"mkdir -p /data/r1os/bin && mv -f @S/agent-exec /data/r1os/bin/agent-exec && chown 0:0 /data/r1os/bin/agent-exec && "#,
    r#"chmod 700 /data/r1os/bin/agent-exec && restorecon -F /data/r1os/bin/agent-exec"#,
);
const BUSYBOX_IN: &str = concat!(
    r#"mkdir -p /data/r1os/bin && mv -f @S/busybox /data/r1os/bin/busybox && chown 0:0 /data/r1os/bin/busybox && "#,
    r#"chmod 700 /data/r1os/bin/busybox && restorecon -F /data/r1os/bin/busybox && /data/r1os/bin/busybox | head -n 1"#,
);
/// `agent-box.sh install`'s build: `setup-rootfs.sh` extracts Alpine (first time only), adds the agent user and the
/// packages, Codex if pushed, and Claude Code with its own installer; then the box's labels (ADR-0035).
const BUILD_BOX: &str = concat!(
    r#"sh @S/setup-rootfs.sh @S/alpine.tar.gz @S/codex.tar.gz @S/codex-host.tar.gz; rc=$?; "#,
    r#"rm -f @S/alpine.tar.gz @S/codex.tar.gz @S/codex-host.tar.gz @S/setup-rootfs.sh; "#,
    r#"restorecon -RF /data/r1os/bin /data/r1os/agent; exit $rc"#,
);

/// What a [Cmd] does: one of the reads, or which write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Facts,
    Hashes,
    Sizes,
    Box,
    Up,
    Grow,
    Clear,
    Speech,
    System1,
    Translator,
    R1d,
    Shell,
    Stop,
    AgentExec,
    Busybox,
    BuildBox,
}

impl Kind {
    /// Whether it changes anything on the R1.
    pub fn writes(self) -> bool {
        !matches!(self, Kind::Facts | Kind::Hashes | Kind::Sizes | Kind::Box | Kind::Up)
    }
}

/// One root shell command of the setup's. Only this module makes them, from the templates above: there's no way to
/// ask for another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    kind: Kind,
    text: String,
}

impl Cmd {
    fn new(kind: Kind, template: &str) -> Cmd {
        Cmd { kind, text: template.replace("@S", STAGE) }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The command line, as the R1's `sh` runs it.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// `/data`'s block device, as `/proc/mounts` names it: `/dev/block/` and plain names, nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockDevice(String);

fn block_device(s: &str) -> Option<BlockDevice> {
    let rest = s.strip_prefix("/dev/block/")?;
    let plain = |p: &str| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    rest.split('/').all(plain).then(|| BlockDevice(s.to_owned()))
}

fn grow(d: &BlockDevice) -> Cmd {
    let mut c = Cmd::new(Kind::Grow, GROW);
    c.text = c.text.replace("@D", &d.0);
    c
}

/// What a push takes from the PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    /// A folder's contents.
    Dir(PathBuf),
    /// One of the agent box's compiled-in scripts.
    Script { name: &'static str, text: &'static str },
}

/// One push into [STAGE]. Only this module makes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    source: Source,
    to: String,
}

impl Push {
    fn new(source: Source, name: &str) -> Push {
        Push { source, to: format!("{STAGE}/{name}") }
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Where it goes on the R1: always under [STAGE].
    pub fn to(&self) -> &str {
        &self.to
    }
}

/// What a command printed, and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub code: i32,
    pub out: String,
    pub err: String,
}

/// A booted R1 with root, as the setup sees it: [OverAdb], or a stand-in in tests.
pub trait Device {
    /// Runs [cmd] as root. [line] hears what it prints, line by line, as it comes.
    fn root(&self, cmd: &Cmd, line: &mut dyn FnMut(&str)) -> Result<Output>;
    /// One file or folder from the PC into [STAGE].
    fn push(&self, push: &Push) -> Result<()>;
    /// Between polls while r1d comes back.
    fn wait(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// A booted R1 over adb, with root: adbd as root if it is or can be (`adb root`, r1os's image), else Magisk's `su`.
#[derive(Debug)]
pub struct OverAdb<'a> {
    adb: &'a Adb,
    root: Root,
}

impl<'a> OverAdb<'a> {
    pub fn new(adb: &'a Adb) -> Result<Self> {
        Ok(Self { adb, root: adb.root()? })
    }
}

impl Device for OverAdb<'_> {
    fn root(&self, cmd: &Cmd, line: &mut dyn FnMut(&str)) -> Result<Output> {
        self.adb.setup_command(self.root, cmd, line)
    }

    fn push(&self, push: &Push) -> Result<()> {
        self.adb.setup_push(push)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// What the setup takes.

/// What the setup puts on the R1, by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// r1d built for the R1 (`tools/build-r1d.sh`: `target/aarch64-linux-android/release/r1d`).
    pub r1d: PathBuf,
    /// The shell's build (`shell/dist`), with its `index.html`.
    pub shell: PathBuf,
    /// sherpa-onnx's C API and ONNX Runtime for the R1 (`out/speech/lib/arm64-v8a`; `tools/fetch-speech.sh`). r1d
    /// doesn't start without them.
    pub speech_libs: PathBuf,
    /// The speech-to-text model in the R1's `.ort` form, with its `tokens.txt` (`out/speech/stt-r1`).
    pub speech_model: PathBuf,
    /// System One's model (`out/system1`; `tools/fetch-models.sh`), if chosen.
    pub system1: Option<PathBuf>,
    /// The Translator's models (`out/translate`, about 2.2 GB), if chosen.
    pub translator: Option<PathBuf>,
    /// The agent box, if it's to be built now. It's built on the R1, which has to be online for it.
    pub agent_box: Option<BoxInputs>,
}

/// What the agent box is built from (`agent-box.sh`'s downloads, checked against their SHA-256s there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxInputs {
    /// Alpine's aarch64 minirootfs (`alpine-minirootfs-<version>-aarch64.tar.gz`).
    pub alpine: PathBuf,
    /// Codex's static musl build, if the box is to have Codex.
    pub codex: Option<PathBuf>,
    /// Codex's code-mode host, without which its tools fail closed. Needs [BoxInputs::codex].
    pub codex_host: Option<PathBuf>,
    /// A static arm64 BusyBox (Magisk's `libbusybox.so`), for an R1 without Magisk's own: r1os's image.
    pub busybox: Option<PathBuf>,
}

impl Inputs {
    /// Where a checkout has them, after `tools/build-r1d.sh`, `pnpm --dir shell build`, `tools/fetch-speech.sh`,
    /// `tools/fetch-models.sh` (each model folder only if it's there, as `deploy.sh` takes them) and `agent-box.sh`'s
    /// downloads in `firmware/agent-box` (the newest Alpine and Codex there). Reads nothing but folder listings;
    /// [Setup::new] says what's missing.
    pub fn from_checkout(root: &Path) -> Inputs {
        let out = root.join("out");
        let fw = root.join("firmware").join("agent-box");
        // The newest `<prefix>….tar.gz` there, but not `<not>…`.
        let newest = |prefix: &str, not: Option<&str>| -> Option<PathBuf> {
            std::fs::read_dir(&fw)
                .ok()?
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with(prefix) && !not.is_some_and(|x| n.starts_with(x)) && n.ends_with(".tar.gz")
                })
                .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
                .map(|e| e.path())
        };
        let codex = newest("codex-", Some("codex-code-mode-host-"));
        let codex_host = codex
            .as_ref()
            .and_then(|c| {
                let tag = c.file_name()?.to_str()?.strip_prefix("codex-")?.strip_suffix(".tar.gz")?;
                Some(fw.join(format!("codex-code-mode-host-{tag}.tar.gz")))
            })
            .filter(|p| p.is_file());
        Inputs {
            r1d: root.join("target").join("aarch64-linux-android").join("release").join("r1d"),
            shell: root.join("shell").join("dist"),
            speech_libs: out.join("speech").join("lib").join("arm64-v8a"),
            speech_model: out.join("speech").join("stt-r1"),
            system1: Some(out.join("system1")).filter(|d| d.join("model.int8.onnx").is_file()),
            translator: Some(out.join("translate")).filter(|d| d.join("asr").join("encoder.int8.onnx").is_file()),
            agent_box: Some(BoxInputs {
                alpine: newest("alpine-minirootfs-", None).unwrap_or_else(|| fw.join("alpine-minirootfs-3.24.2-aarch64.tar.gz")),
                codex,
                codex_host,
                busybox: Some(fw.join("busybox-arm64")).filter(|p| p.is_file()),
            }),
        }
    }
}

/// One file to put on the R1: its path under its folder, its size, and its SHA-256 (not for the Translator's).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Local {
    rel: String,
    size: u64,
    sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct LocalBox {
    alpine_bytes: u64,
    alpine_version: Option<String>,
    codex: bool,
    codex_host: bool,
    busybox: Option<Local>,
}

/// [Inputs], checked, with every file's size and SHA-256.
#[derive(Debug, Clone)]
pub struct Setup {
    inputs: Inputs,
    r1d: Local,
    shell: Vec<Local>,
    libs: Vec<Local>,
    stt: Vec<Local>,
    system1: Option<Vec<Local>>,
    translator: Option<Vec<Local>>,
    agent_box: Option<LocalBox>,
}

impl Setup {
    /// Checks [inputs] and reads their files. Refuses anything that isn't what it should be: r1d not an arm64 program,
    /// a model folder without its model, a link or a name `sh` would misread, a folder where only files belong.
    pub fn new(inputs: Inputs) -> Result<Setup> {
        let i = &inputs;
        arm64_program(&i.r1d).context("r1d (build it with tools/build-r1d.sh)")?;
        let r1d = Local { rel: "r1d".into(), ..local(&i.r1d, true)? };
        let shell = tree(&i.shell, true, false).context("the shell (build it with `pnpm --dir shell build`)")?;
        need(&shell, &["index.html"]).context("the shell (build it with `pnpm --dir shell build`)")?;
        let libs = tree(&i.speech_libs, true, true).context("speech-to-text's libraries (tools/fetch-speech.sh)")?;
        need(&libs, &["libsherpa-onnx-c-api.so", "libonnxruntime.so"]).context("speech-to-text's libraries (tools/fetch-speech.sh)")?;
        let stt = tree(&i.speech_model, true, true).context("the speech-to-text model (tools/fetch-speech.sh)")?;
        need(&stt, &["tokens.txt"]).context("the speech-to-text model")?;
        for stem in ["encoder", "decoder", "joiner"] {
            // r1d takes the first of these it finds (crates/r1d/src/voice.rs).
            if !["int8.ort", "ort", "int8.onnx", "onnx"].iter().any(|x| has(&stt, &format!("{stem}.{x}"))) {
                bail!("the speech-to-text model in {} has no {stem}", i.speech_model.display());
            }
        }
        let system1 = match &i.system1 {
            Some(d) => {
                let t = tree(d, true, true).context("System One's model (tools/fetch-models.sh)")?;
                need(&t, &["vocab.txt"]).context("System One's model")?;
                if !has(&t, "model.int8.onnx") && !has(&t, "model.onnx") {
                    bail!("System One's model in {} has no model.int8.onnx", d.display());
                }
                Some(t)
            }
            None => None,
        };
        let translator = match &i.translator {
            Some(d) => {
                let t = tree(d, false, false).context("the Translator's models (tools/fetch-models.sh)")?;
                need(&t, &["asr/encoder.int8.onnx"]).context("the Translator's models")?;
                Some(t)
            }
            None => None,
        };
        let agent_box = match &i.agent_box {
            Some(b) => {
                let alpine = gzip(&b.alpine).context("Alpine's minirootfs")?;
                if let Some(c) = &b.codex {
                    gzip(c).context("Codex")?;
                }
                if let Some(h) = &b.codex_host {
                    if b.codex.is_none() {
                        bail!("Codex's code-mode host without Codex");
                    }
                    gzip(h).context("Codex's code-mode host")?;
                }
                let busybox = match &b.busybox {
                    Some(p) => {
                        arm64_program(p).context("BusyBox")?;
                        Some(Local { rel: "busybox".into(), ..local(p, true)? })
                    }
                    None => None,
                };
                let name = b.alpine.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                let alpine_version = name.strip_prefix("alpine-minirootfs-").and_then(|n| n.strip_suffix("-aarch64.tar.gz")).map(str::to_owned);
                Some(LocalBox { alpine_bytes: alpine, alpine_version, codex: b.codex.is_some(), codex_host: b.codex_host.is_some(), busybox })
            }
            None => None,
        };
        Ok(Setup { inputs, r1d, shell, libs, stt, system1, translator, agent_box })
    }

    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// What has to be done on an R1 in [status]'s state, in order, and what's already there. Refuses when it can't be
    /// done: `/data` small and not ext4; nothing to restart r1d; the agent box to build on an R1 that isn't online, or
    /// with no BusyBox to run it.
    pub fn plan(&self, s: &Status) -> Result<Plan> {
        let mut p = Plan { steps: Vec::new(), words: Vec::new(), already: Vec::new(), setup: self.clone(), assumed: s.assumed };
        let d = &s.data;
        if d.grown {
            p.already.push(format!("/data is full size ({})", gb(d.fs_bytes)));
        } else if d.kind != "ext4" {
            bail!(
                "/data is {} of its {} partition, and {} rather than ext4, so it can't be grown online as Rabbit's ext4 image can",
                gb(d.fs_bytes),
                gb(d.partition_bytes),
                if d.kind.is_empty() { "unknown" } else { &d.kind }
            );
        } else {
            p.add(
                Step::GrowData { device: d.device.clone() },
                format!(
                    "Grow /data to fill its partition: its filesystem is {} of {} (online; nothing is erased)",
                    gb(d.fs_bytes),
                    gb(d.partition_bytes)
                ),
            );
        }

        let deploy = p.steps.len();
        let sum = |f: &[Local]| f.iter().map(|l| l.size).sum::<u64>();
        if s.has("lib", &self.libs) && s.has("models/stt", &self.stt) {
            p.already.push("Speech-to-text".into());
        } else {
            p.add(Step::Speech, format!("Speech-to-text: its libraries and model ({})", bytes(sum(&self.libs) + sum(&self.stt))));
        }
        if let Some(m) = &self.system1 {
            if s.has("models/system1", m) {
                p.already.push("System One's model".into());
            } else {
                p.add(Step::System1, format!("System One's model ({})", bytes(sum(m))));
            }
        }
        if let Some(m) = &self.translator {
            if s.has("models/translate", m) {
                p.already.push("The Translator's models".into());
            } else {
                p.add(Step::Translator, format!("The Translator's models ({}: the first time takes a few minutes)", bytes(sum(m))));
            }
        }
        if s.has("bin", std::slice::from_ref(&self.r1d)) {
            p.already.push("r1d, this build".into());
        } else {
            p.add(Step::R1d, format!("r1d, r1os's core ({})", bytes(self.r1d.size)));
        }
        if s.has("shell", &self.shell) {
            p.already.push("The shell, this build".into());
        } else {
            p.add(Step::Shell, format!("The shell, r1os's screens ({})", bytes(sum(&self.shell))));
        }
        if p.steps.len() > deploy || !s.r1d_running {
            if s.restarter == Restarter::None {
                bail!("nothing on this R1 starts r1d: it isn't running r1os's image (init's r1d service) or r1os-base");
            }
            let w = if s.r1d_running { "Restart r1d with what's new, and wait for it" } else { "Start r1d, and wait for it" };
            p.add(Step::RestartR1d, w.into());
        } else {
            p.already.push("r1d is running".into());
        }

        if let Some(b) = &self.agent_box {
            if s.files.get("bin/agent-exec").and_then(|r| r.sha256.as_deref()) == Some(&sha256_hex(AGENT_EXEC.as_bytes())) {
                p.already.push("The agent box's agent-exec, this build".into());
            } else {
                p.add(Step::AgentExec, "The agent box's agent-exec, which runs the agents' CLIs in it".into());
            }
            let ab = &s.agent_box;
            match (&ab.busybox, &b.busybox) {
                (Busybox::Magisk, _) => p.already.push("Magisk's BusyBox, for the agent box".into()),
                (_, Some(bb)) if s.has("bin", std::slice::from_ref(bb)) => p.already.push("BusyBox, for the agent box".into()),
                (_, Some(bb)) => p.add(Step::Busybox, format!("A static BusyBox for the agent box ({}): r1os's image has no Magisk", bytes(bb.size))),
                (Busybox::Ours, None) => p.already.push("BusyBox, for the agent box".into()),
                (Busybox::None, None) => bail!("this R1 has no BusyBox for the agent box (r1os's image has no Magisk): the setup needs one"),
            }
            let built = ab.rootfs && ab.user && ab.packages && ab.claude.is_some() && (ab.codex || !b.codex) && (ab.codex_host || !b.codex_host);
            if built {
                p.already.push(format!(
                    "The agent box (Alpine {}, Claude Code {})",
                    ab.alpine.as_deref().unwrap_or("?"),
                    ab.claude.as_deref().unwrap_or("?")
                ));
            } else if s.online == Some(false) {
                bail!(
                    "the R1 isn't online: building the agent box downloads Alpine's packages and Claude Code on it. Connect it to Wi-Fi, or leave the box out for now"
                );
            } else {
                let alpine = match &b.alpine_version {
                    Some(v) => format!("Alpine {v}"),
                    None => "Alpine".into(),
                };
                p.add(
                    Step::AgentBox,
                    format!(
                        "Build the agent box: {alpine} ({}){}, and Claude Code from its installer. The R1 downloads packages, so it has to be online; a few minutes",
                        bytes(b.alpine_bytes),
                        if b.codex { ", Codex" } else { "" }
                    ),
                );
            }
        }
        Ok(p)
    }
}

/// Whether [files] has [rel].
fn has(files: &[Local], rel: &str) -> bool {
    files.iter().any(|f| f.rel == rel)
}

fn need(files: &[Local], names: &[&str]) -> Result<()> {
    match names.iter().find(|n| !has(files, n)) {
        Some(n) => bail!("no {n}"),
        None => Ok(()),
    }
}

fn sha256_hex(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn local(path: &Path, hash: bool) -> Result<Local> {
    let m = std::fs::metadata(path).with_context(|| format!("no {}", path.display()))?;
    if !m.is_file() {
        bail!("{} isn't a file", path.display());
    }
    let sha256 = if hash {
        let mut h = Sha256::new();
        std::io::copy(&mut std::fs::File::open(path)?, &mut h)?;
        Some(hex(&h.finalize()))
    } else {
        None
    };
    let rel = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Local { rel, size: m.len(), sha256 })
}

/// A name that goes through `sh` on the R1 as it is: no control characters (a newline would split `read`'s lines), no
/// spaces at its ends (`read` drops them), not `.` or `..`.
fn plain_name(n: &str) -> bool {
    !n.is_empty() && n != "." && n != ".." && n.trim() == n && !n.chars().any(|c| c.is_control() || c == '\\')
}

/// Every file under [dir], sorted, with `/` between folders. [flat]: files only (`cp dir/*` copies no folders).
fn tree(dir: &Path, hash: bool, flat: bool) -> Result<Vec<Local>> {
    if !dir.is_dir() {
        bail!("no folder {}", dir.display());
    }
    let mut out = Vec::new();
    walk(dir, "", hash, flat, &mut out)?;
    if out.is_empty() {
        bail!("{} is empty", dir.display());
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, hash: bool, flat: bool, out: &mut Vec<Local>) -> Result<()> {
    let mut entries = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().into_string().map_err(|_| anyhow!("a name in {} isn't Unicode", dir.display()))?;
        if !plain_name(&name) {
            bail!("{}: {name:?} can't go to the R1 as it is", dir.display());
        }
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let t = e.file_type()?;
        if t.is_symlink() {
            bail!("{}: {rel} is a link", dir.display());
        } else if t.is_dir() {
            if flat {
                bail!("{}: {rel} is a folder, where only files belong", dir.display());
            }
            walk(&e.path(), &rel, hash, flat, out)?;
        } else {
            out.push(Local { rel, ..local(&e.path(), hash)? });
        }
    }
    Ok(())
}

/// An ELF program for arm64 (`e_machine` 183), as the R1 runs.
fn arm64_program(path: &Path) -> Result<()> {
    let mut head = [0u8; 20];
    std::fs::File::open(path)
        .with_context(|| format!("no {}", path.display()))?
        .read_exact(&mut head)
        .map_err(|_| anyhow!("{} is too short to be a program", path.display()))?;
    if &head[..4] != b"\x7fELF" || head[4] != 2 || head[5] != 1 || u16::from_le_bytes([head[18], head[19]]) != 183 {
        bail!("{} isn't an arm64 program for the R1", path.display());
    }
    Ok(())
}

/// A gzip file's size.
fn gzip(path: &Path) -> Result<u64> {
    let mut head = [0u8; 2];
    let mut f = std::fs::File::open(path).with_context(|| format!("no {}", path.display()))?;
    if f.read_exact(&mut head).is_err() || head != [0x1f, 0x8b] {
        bail!("{} isn't a .tar.gz", path.display());
    }
    Ok(f.metadata()?.len())
}

fn bytes(b: u64) -> String {
    match b {
        b if b >= 1_000_000_000 => gb(b),
        b if b >= 1_000_000 => format!("{} MB", (b + 500_000) / 1_000_000),
        b if b >= 1_000 => format!("{} KB", (b + 500) / 1_000),
        b => format!("{b} bytes"),
    }
}

fn gb(b: u64) -> String {
    let g = b as f64 / 1e9;
    if g >= 10.0 { format!("{g:.0} GB") } else { format!("{g:.1} GB") }
}

// ---------------------------------------------------------------------------------------------------------------------
// What's on the R1.

/// `/data`, as `grow-data.sh` reads it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Data {
    /// Its block device (`/dev/block/dm-…`, the metadata-encryption device spanning the partition).
    pub device: String,
    /// Its filesystem: `ext4` for Rabbit's image.
    pub kind: String,
    /// The filesystem's size (`statfs`).
    pub fs_bytes: u64,
    /// The block device's.
    pub partition_bytes: u64,
    /// At least 90 % of the partition (ext4's own metadata takes a little): nothing to grow.
    pub grown: bool,
}

/// What starts r1d again when it exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Restarter {
    /// init's `r1d` service: r1os's own image.
    Init,
    /// r1os-base's `service.sh` loop, under Magisk (the LineageOS developer base).
    Magisk,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct R1d {
    /// `/data/r1os/bin/r1d`'s SHA-256: what tells builds apart (r1d's own version is its crate's, the same in each).
    pub sha256: String,
    /// The version it logged when it last started, if its log says.
    pub version: Option<String>,
}

/// Which BusyBox the agent box's scripts would use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Busybox {
    /// `/data/adb/magisk/busybox`.
    Magisk,
    /// `/data/r1os/bin/busybox`.
    Ours,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BoxStatus {
    /// Alpine extracted (`rootfs/bin/busybox`).
    pub rootfs: bool,
    /// The agent user (uid 2950).
    pub user: bool,
    /// The packages the CLIs and agent-exec need (bash, setpriv).
    pub packages: bool,
    pub codex: bool,
    pub codex_host: bool,
    /// `/etc/alpine-release`.
    pub alpine: Option<String>,
    /// Claude Code: its version where the installer's link names one, else `installed`.
    pub claude: Option<String>,
    pub agent_exec: bool,
    pub busybox: Busybox,
}

/// What [status] read from the R1.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// r1os's image version (`ro.r1os.image.version`), if it's r1os's image.
    pub image: Option<String>,
    pub data: Data,
    pub restarter: Restarter,
    /// r1d as installed in `/data/r1os/bin`.
    pub r1d: Option<R1d>,
    pub r1d_running: bool,
    /// Whether Android has a default network (`dumpsys connectivity`), if it said.
    pub online: Option<bool>,
    pub agent_box: BoxStatus,
    /// [Status::fresh]'s guess, not read from an R1: [run] refuses a plan made from it.
    pub assumed: bool,
    /// Every file [HASHES] and [SIZES] listed, by its path under `/data/r1os`.
    #[serde(skip)]
    files: BTreeMap<String, Remote>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Remote {
    sha256: Option<String>,
    size: Option<u64>,
}

impl Status {
    /// A just-flashed R1 as it's assumed to be, for showing a plan without one: r1os's image, nothing of r1os's in
    /// `/data/r1os` yet, `/data` still Rabbit's 2.8 GB filesystem on its 115 GB partition, no Magisk, online.
    pub fn fresh() -> Status {
        Status {
            image: None,
            data: Data { device: String::new(), kind: "ext4".into(), fs_bytes: 2_818_572_288, partition_bytes: 115_000_000_000, grown: false },
            restarter: Restarter::Init,
            r1d: None,
            r1d_running: false,
            online: Some(true),
            agent_box: BoxStatus {
                rootfs: false,
                user: false,
                packages: false,
                codex: false,
                codex_host: false,
                alpine: None,
                claude: None,
                agent_exec: false,
                busybox: Busybox::None,
            },
            assumed: true,
            files: BTreeMap::new(),
        }
    }

    /// Whether every one of [files] is in `/data/r1os/<prefix>/`, by SHA-256 (or by size where there's none).
    fn has(&self, prefix: &str, files: &[Local]) -> bool {
        files.iter().all(|f| match self.files.get(&format!("{prefix}/{}", f.rel)) {
            Some(r) => match &f.sha256 {
                Some(h) => r.sha256.as_deref() == Some(h.as_str()),
                None => r.size == Some(f.size),
            },
            None => false,
        })
    }
}

/// `key=value` lines.
fn kv(text: &str) -> BTreeMap<String, String> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect()
}

fn pids(v: Option<&String>) -> Vec<u32> {
    v.map(|v| v.split_whitespace().filter_map(|p| p.parse().ok()).collect()).unwrap_or_default()
}

/// `sha256sum`'s lines (`<hash>  <path>`), paths without their `./`.
fn parse_hashes(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (h, path) = (l.get(..64)?, l.get(64..)?);
            if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let path = path.strip_prefix("  ").or_else(|| path.strip_prefix(" *"))?;
            Some((path.trim_start_matches("./").to_owned(), h.to_ascii_lowercase()))
        })
        .collect()
}

/// `stat -c "%s %n"`'s lines.
fn parse_sizes(text: &str) -> BTreeMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let (n, path) = l.split_once(' ')?;
            Some((path.trim_start_matches("./").to_owned(), n.parse().ok()?))
        })
        .collect()
}

/// The version in r1d's `r1d starting version="…"` line.
fn logged_version(line: &str) -> Option<String> {
    let (_, v) = line.split_once("version=")?;
    let v: String = v.trim_start_matches('"').chars().take_while(|c| !c.is_whitespace() && *c != '"').collect();
    (!v.is_empty()).then_some(v)
}

/// The facts [FACTS] prints.
struct Facts {
    image: Option<String>,
    restarter: Restarter,
    pids: Vec<u32>,
    data: Data,
    online: Option<bool>,
    version: Option<String>,
    magisk_busybox: bool,
}

impl Facts {
    fn read(dev: &dyn Device) -> Result<Facts> {
        let f = kv(&read(dev, &Cmd::new(Kind::Facts, FACTS))?);
        let get = |k: &str| f.get(k).map(String::as_str).unwrap_or_default();
        let device = block_device(get("data_dev")).ok_or_else(|| anyhow!("couldn't find /data's block device (got {:?})", get("data_dev")))?;
        let partition_bytes: u64 = get("data_dev_bytes").parse().map_err(|_| anyhow!("couldn't read the size of /data's block device"))?;
        let mut fs = get("data_fs").split_whitespace().map(str::parse::<u64>);
        let fs_bytes = match (fs.next(), fs.next()) {
            (Some(Ok(blocks)), Some(Ok(size))) => blocks.checked_mul(size).context("/data's size")?,
            _ => bail!("couldn't read /data's filesystem size"),
        };
        if partition_bytes == 0 {
            bail!("/data's block device has no size");
        }
        // statfs reports a little less than the device because of ext4's metadata: 90 % is grown (grow-data.sh).
        let grown = fs_bytes as u128 * 10 >= partition_bytes as u128 * 9;
        let online = get("net").split_once("Active default network:").map(|(_, v)| v.trim() != "none" && !v.trim().is_empty());
        Ok(Facts {
            image: Some(get("image").to_owned()).filter(|v| !v.is_empty()),
            restarter: if !get("init_r1d").is_empty() {
                Restarter::Init
            } else if f.contains_key("magisk_loop") {
                Restarter::Magisk
            } else {
                Restarter::None
            },
            pids: pids(f.get("r1d_pid")),
            data: Data { device: device.0, kind: get("data_type").to_owned(), fs_bytes, partition_bytes, grown },
            online,
            version: f.get("r1d_log").and_then(|l| logged_version(l)),
            magisk_busybox: f.contains_key("magisk_busybox"),
        })
    }
}

fn read(dev: &dyn Device, cmd: &Cmd) -> Result<String> {
    debug_assert!(!cmd.kind.writes());
    let o = dev.root(cmd, &mut |_| {})?;
    if o.code != 0 {
        bail!("reading the R1 failed ({}): {}", o.code, o.err.trim());
    }
    Ok(o.out)
}

/// What's on the R1. Only reads: `getprop`, `/proc`, `stat`, `blockdev --getsize64`, `dumpsys connectivity`,
/// `sha256sum` over r1os's files, and a look at the agent box's files without running it.
pub fn status(dev: &dyn Device) -> Result<Status> {
    let f = Facts::read(dev)?;
    let mut files: BTreeMap<String, Remote> = BTreeMap::new();
    for (path, h) in parse_hashes(&read(dev, &Cmd::new(Kind::Hashes, HASHES))?) {
        files.entry(path).or_default().sha256 = Some(h);
    }
    for (path, n) in parse_sizes(&read(dev, &Cmd::new(Kind::Sizes, SIZES))?) {
        files.entry(path).or_default().size = Some(n);
    }
    let b = kv(&read(dev, &Cmd::new(Kind::Box, BOX))?);
    let claude = b.get("claude").map(|target| {
        let v = target.rsplit('/').next().unwrap_or_default();
        if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.') { v.to_owned() } else { "installed".to_owned() }
    });
    let busybox = if f.magisk_busybox {
        Busybox::Magisk
    } else if files.contains_key("bin/busybox") {
        Busybox::Ours
    } else {
        Busybox::None
    };
    let agent_box = BoxStatus {
        rootfs: b.contains_key("rootfs"),
        user: b.contains_key("user"),
        packages: b.contains_key("packages"),
        codex: b.contains_key("codex"),
        codex_host: b.contains_key("codex_host"),
        alpine: b.get("alpine").filter(|v| !v.is_empty()).cloned(),
        claude,
        agent_exec: files.contains_key("bin/agent-exec"),
        busybox,
    };
    let r1d = files.get("bin/r1d").and_then(|r| r.sha256.clone()).map(|sha256| R1d { sha256, version: f.version.clone() });
    Ok(Status {
        image: f.image,
        data: f.data,
        restarter: f.restarter,
        r1d,
        r1d_running: !f.pids.is_empty(),
        online: f.online,
        agent_box,
        assumed: false,
        files,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// The plan and the run.

/// One step of the setup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Step {
    /// `grow-data.sh`: `/data`'s ext4 filesystem grown online to fill its partition.
    GrowData { device: String },
    /// sherpa-onnx's libraries into `/data/r1os/lib`, the model into `models/stt`.
    Speech,
    System1,
    Translator,
    R1d,
    Shell,
    /// `pkill -x r1d`; init or r1os-base starts the new one, which has to come back listening.
    RestartR1d,
    AgentExec,
    Busybox,
    /// `setup-rootfs.sh` on the R1: Alpine, the agent user, packages, Codex, Claude Code.
    AgentBox,
}

impl Step {
    fn pushes(&self) -> bool {
        !matches!(self, Step::GrowData { .. } | Step::RestartR1d)
    }
}

/// What [Setup::plan] found to do.
#[derive(Debug, Clone)]
pub struct Plan {
    steps: Vec<Step>,
    words: Vec<String>,
    already: Vec<String>,
    setup: Setup,
    assumed: bool,
}

impl Plan {
    fn add(&mut self, step: Step, words: String) {
        self.steps.push(step);
        self.words.push(words);
    }

    /// Every step, in order. [run] does exactly these.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Each step in words, for the screen.
    pub fn describe(&self) -> &[String] {
        &self.words
    }

    /// What's on the R1 already, and left alone.
    pub fn already(&self) -> &[String] {
        &self.already
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// What [run] tells as it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress<'a> {
    /// Step [index] (from 1) of [of] starts.
    Step { index: usize, of: usize, words: &'a str },
    /// A line the R1 printed (the agent box's build says what it's doing).
    Line(&'a str),
}

fn write(dev: &dyn Device, cmd: &Cmd, what: &str, progress: &mut dyn FnMut(Progress)) -> Result<Output> {
    let o = dev.root(cmd, &mut |l| progress(Progress::Line(l)))?;
    if o.code != 0 {
        let tail: Vec<&str> = o.out.lines().chain(o.err.lines()).filter(|l| !l.trim().is_empty()).collect();
        bail!("{what}: failed ({}): {}", o.code, tail[tail.len().saturating_sub(4)..].join(" / "));
    }
    Ok(o)
}

/// Runs [plan] on the R1 it was made for. It reads the status again first and stops if the plan would differ (the R1
/// changed, or it's [Status::fresh]'s plan); `/data`'s block device and size are read once more right before it's
/// grown. Afterwards it reads the status again, which it returns, and fails if anything is still to do.
pub fn run(dev: &dyn Device, plan: &Plan, progress: &mut dyn FnMut(Progress)) -> Result<Status> {
    if plan.assumed {
        bail!("this plan wasn't made from an R1's status: read the R1 first");
    }
    let setup = &plan.setup;
    let now = status(dev)?;
    if setup.plan(&now)?.steps != plan.steps {
        bail!("the R1 changed since the plan was made: look at the new plan");
    }
    let staged = plan.steps.iter().any(Step::pushes);
    if staged {
        write(dev, &Cmd::new(Kind::Clear, CLEAR), "Emptying the setup's folder", progress)?;
    }
    let i = &setup.inputs;
    let of = plan.steps.len();
    for (n, (step, words)) in plan.steps.iter().zip(&plan.words).enumerate() {
        progress(Progress::Step { index: n + 1, of, words });
        let push = |source: Source, name: &str| dev.push(&Push::new(source, name)).with_context(|| words.clone());
        match step {
            Step::GrowData { device } => {
                let f = Facts::read(dev)?;
                if f.data.device != *device {
                    bail!("/data's block device changed since the plan: read the R1 again");
                }
                if f.data.grown {
                    progress(Progress::Line("/data is full size already"));
                    continue;
                }
                if f.data.kind != "ext4" {
                    bail!("/data isn't ext4: not grown");
                }
                let d = block_device(device).context("not a block device")?;
                write(dev, &grow(&d), words, progress)?;
                let f = Facts::read(dev)?;
                if !f.data.grown {
                    bail!("/data is still {} of {} after resize2fs", gb(f.data.fs_bytes), gb(f.data.partition_bytes));
                }
            }
            Step::Speech => {
                push(Source::Dir(i.speech_libs.clone()), "lib")?;
                push(Source::Dir(i.speech_model.clone()), "stt")?;
                write(dev, &Cmd::new(Kind::Speech, SPEECH), words, progress)?;
            }
            Step::System1 => {
                push(Source::Dir(i.system1.clone().context("no System One in the inputs")?), "system1")?;
                write(dev, &Cmd::new(Kind::System1, SYSTEM1), words, progress)?;
            }
            Step::Translator => {
                push(Source::Dir(i.translator.clone().context("no Translator in the inputs")?), "translate")?;
                write(dev, &Cmd::new(Kind::Translator, TRANSLATOR), words, progress)?;
            }
            Step::R1d => {
                push(Source::File(i.r1d.clone()), "r1d")?;
                write(dev, &Cmd::new(Kind::R1d, R1D), words, progress)?;
            }
            Step::Shell => {
                push(Source::Dir(i.shell.clone()), "shell")?;
                write(dev, &Cmd::new(Kind::Shell, SHELL), words, progress)?;
            }
            Step::RestartR1d => {
                let up = Cmd::new(Kind::Up, UP);
                let before = pids(kv(&read(dev, &up)?).get("r1d_pid"));
                write(dev, &Cmd::new(Kind::Stop, STOP), words, progress)?;
                let mut back = false;
                for _ in 0..(R1D_BACK.as_millis() / POLL.as_millis()) {
                    dev.wait(POLL);
                    let f = kv(&read(dev, &up)?);
                    let now = pids(f.get("r1d_pid"));
                    if !now.is_empty() && !now.iter().any(|p| before.contains(p)) && f.contains_key("listening") {
                        back = true;
                        break;
                    }
                }
                if !back {
                    bail!("r1d didn't come back within {} s: its log is /data/r1os/log/r1d.log", R1D_BACK.as_secs());
                }
            }
            Step::AgentExec => {
                push(Source::Script { name: "agent-exec", text: AGENT_EXEC }, "agent-exec")?;
                write(dev, &Cmd::new(Kind::AgentExec, AGENT_EXEC_IN), words, progress)?;
            }
            Step::Busybox => {
                let b = i.agent_box.as_ref().and_then(|b| b.busybox.clone()).context("no BusyBox in the inputs")?;
                push(Source::File(b), "busybox")?;
                write(dev, &Cmd::new(Kind::Busybox, BUSYBOX_IN), words, progress)?;
            }
            Step::AgentBox => {
                let b = i.agent_box.as_ref().context("no agent box in the inputs")?;
                push(Source::File(b.alpine.clone()), "alpine.tar.gz")?;
                if let Some(c) = &b.codex {
                    push(Source::File(c.clone()), "codex.tar.gz")?;
                }
                if let Some(h) = &b.codex_host {
                    push(Source::File(h.clone()), "codex-host.tar.gz")?;
                }
                push(Source::Script { name: "setup-rootfs.sh", text: SETUP_ROOTFS }, "setup-rootfs.sh")?;
                write(dev, &Cmd::new(Kind::BuildBox, BUILD_BOX), words, progress)?;
            }
        }
    }
    if staged {
        write(dev, &Cmd::new(Kind::Clear, CLEAR), "Emptying the setup's folder", progress)?;
    }
    let after = status(dev)?;
    let left = setup.plan(&after).context("after the setup")?;
    if !left.is_empty() {
        bail!("after the setup, still to do: {}", left.words.join("; "));
    }
    Ok(after)
}

#[cfg(test)]
mod tests;
