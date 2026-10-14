//! The agent box, enforced by r1d itself (ADR-0003; replaces the BusyBox agent-exec script for
//! r1d's own sessions).
//!
//! Every agent process (builds, logins, chats) starts through [`Jail::command`]. Between fork and
//! exec, the child:
//!   1. joins the agent memory cgroup (`/dev/memcg/r1os-agent`, 1.5 GB);
//!   2. unshares its mount namespace and makes it private, so nothing leaks into Android's;
//!   3. mounts a fresh /proc, a 512 MB /tmp, a minimal /dev (null zero random urandom tty + its own
//!      devpts), and binds the agent's home and the workspace;
//!   4. chroots into the Alpine rootfs;
//!   5. drops to uid/gid 2950 with groups {2950, 3003 inet}, so all capabilities are gone;
//!   6. sets no_new_privs and installs a seccomp filter that refuses mount/namespace/ptrace/module/
//!      bpf/keyring/clock/reboot syscalls with EPERM;
//!   7. enters the `r1d_agent` SELinux domain (ADR-0035): the box's files, its /tmp and /dev, the
//!      network and r1d's API, and nothing else of Android's.
//!
//! The environment is cleared and rebuilt. Rabbit's kernel has no user namespaces (Q2), so this is
//! the boundary: the CLIs' own bubblewrap sandboxes can't run. A disk quota is checked before each
//! session ([`Jail::check_quota`]).
//!
//! **One jail per agent** (ADR-0011, docs/specs/agents.md §4). An agent's processes see only their
//! own files: its workspace (`agents/<id>/work`) at /work, and its own CLI chat stores bound over
//! the shared home's (`~/.claude/projects`, `~/.claude/file-history`, `~/.codex/sessions`). The
//! rest of the home (the CLIs' logins and settings) is shared, so one sign-in serves every agent.
//! `agents/` itself is root-only. r1d's own sessions (app builds, sign-ins) use [`Jail::default`],
//! which sees the box's shared `work/`.

use std::path::{Path, PathBuf};

pub const BOX: &str = "/data/r1os/agent";
pub const AGENT_UID: u32 = 2950;
#[cfg_attr(not(any(target_os = "android", target_os = "linux")), allow(dead_code))]
pub const INET_GID: u32 = 3003;
/// Memory cap for everything in the box.
#[cfg_attr(not(any(target_os = "android", target_os = "linux")), allow(dead_code))]
pub const MEMORY_LIMIT: u64 = 1536 * 1024 * 1024;
/// Disk cap for the agent's home + workspaces (checked before each session).
pub const DISK_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
#[cfg_attr(not(any(target_os = "android", target_os = "linux")), allow(dead_code))]
const MEMCG: &str = "/dev/memcg/r1os-agent";

pub const CLAUDE: &str = "/home/agent/.local/bin/claude";
pub const CODEX: &str = "/usr/local/bin/codex";

/// An agent's own CLI state, kept in `agents/<id>/<dir>` and bound over `<path>` in the shared
/// home: its Claude transcripts and file checkpoints, and its Codex threads.
const PER_AGENT_HOME: &[(&str, &str)] = &[
    ("claude-projects", ".claude/projects"),
    ("claude-file-history", ".claude/file-history"),
    ("codex-sessions", ".codex/sessions"),
];

pub struct Jail {
    root: PathBuf,
    /// The agent this jail is for (only its files are mounted); None for r1d's own sessions.
    agent: Option<String>,
}

impl Default for Jail {
    fn default() -> Self {
        Jail { root: PathBuf::from(BOX), agent: None }
    }
}

impl Jail {
    /// The jail for one agent's processes. `id` is checked (crate::core::valid_agent_id): it names
    /// a directory.
    pub fn for_agent(id: &str) -> anyhow::Result<Jail> {
        anyhow::ensure!(crate::core::valid_agent_id(id), "bad agent id `{id}`");
        Ok(Jail { root: PathBuf::from(BOX), agent: Some(id.to_owned()) })
    }

    pub fn available(&self) -> bool {
        cfg!(any(target_os = "android", target_os = "linux")) && self.root.join("rootfs/bin/busybox").exists()
    }

    fn agent_dir(&self) -> Option<PathBuf> {
        self.agent.as_ref().map(|id| self.root.join("agents").join(id))
    }

    /// Host path of the workspace root (`/work` in the box): the agent's own, or the shared one.
    pub fn work(&self) -> PathBuf {
        self.agent_dir().map_or_else(|| self.root.join("work"), |d| d.join("work"))
    }

    /// Creates the agent's directories and the mount points it needs in the shared home. Run before
    /// starting a process in this jail (cheap when they exist).
    pub fn prepare(&self) -> anyhow::Result<()> {
        let Some(dir) = self.agent_dir() else { return Ok(()) };
        if !self.available() {
            return Ok(());
        }
        let agents = self.root.join("agents");
        if !agents.exists() {
            std::fs::create_dir_all(&agents)?;
            restrict(&agents, None)?; // root-only: no agent can list the others
        }
        // Root-only too: only its subdirectories are mounted into the box. (r1d's umask isn't relied on:
        // under Magisk it was 0, so a plain create_dir_all would leave it 0777.)
        std::fs::create_dir_all(&dir)?;
        restrict(&dir, None)?;
        for sub in ["work", "work/files"].iter().copied().chain(PER_AGENT_HOME.iter().map(|(d, _)| *d)) {
            let p = dir.join(sub);
            if !p.exists() {
                std::fs::create_dir_all(&p)?;
                restrict(&p, Some(AGENT_UID))?;
            }
        }
        for (_, in_home) in PER_AGENT_HOME {
            let p = self.root.join("home").join(in_home);
            if !p.exists() {
                std::fs::create_dir_all(&p)?;
                restrict(&p, Some(AGENT_UID))?;
                if let Some(parent) = p.parent() {
                    restrict(parent, Some(AGENT_UID))?;
                }
            }
        }
        Ok(())
    }

    /// Moves the files from before multiple agents (the shared workspace and the CLIs' chat
    /// stores) into this agent, the first one. Runs once: only while `agents/` doesn't exist yet.
    /// Renames on one filesystem, so ownership stays the agent user's.
    pub fn adopt_legacy_files(&self) -> anyhow::Result<()> {
        let Some(dir) = self.agent_dir() else { return Ok(()) };
        if !self.available() || self.root.join("agents").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(dir.join("work"))?;
        let mut moves: Vec<(PathBuf, PathBuf)> = vec![(self.root.join("work/files"), dir.join("work/files"))];
        moves.extend(PER_AGENT_HOME.iter().map(|(d, in_home)| (self.root.join("home").join(in_home), dir.join(d))));
        for (from, to) in moves {
            if from.exists() && !to.exists() {
                std::fs::rename(&from, &to)?;
                tracing::info!(from = %from.display(), to = %to.display(), "moved into the first agent");
            }
        }
        restrict(&self.root.join("agents"), None)?;
        restrict(&dir.join("work"), Some(AGENT_UID))?;
        self.prepare()
    }

    /// Deletes the agent's files (its workspace and chat stores). For a deleted agent.
    pub fn remove_files(&self) -> anyhow::Result<()> {
        match self.agent_dir() {
            Some(d) if d.exists() => Ok(std::fs::remove_dir_all(d)?),
            _ => Ok(()),
        }
    }

    /// Refuses to start a session when the agents' files outgrow [`DISK_LIMIT`].
    pub fn check_quota(&self) -> anyhow::Result<()> {
        let used = dir_size(&self.root.join("home")) + dir_size(&self.root.join("work")) + dir_size(&self.root.join("agents"));
        if used > DISK_LIMIT {
            anyhow::bail!("the agents' files use {} MB, over their {} MB limit", used >> 20, DISK_LIMIT >> 20);
        }
        Ok(())
    }

    /// What's mounted into the box besides the rootfs: (host path, path inside the rootfs). The
    /// home first, so the per-agent stores land on top of it.
    #[cfg_attr(not(any(target_os = "android", target_os = "linux")), allow(dead_code))]
    fn binds(&self) -> Vec<(PathBuf, String)> {
        let mut b = vec![(self.root.join("home"), "home/agent".to_owned()), (self.work(), "work".to_owned())];
        if let Some(dir) = self.agent_dir() {
            b.extend(PER_AGENT_HOME.iter().map(|(d, in_home)| (dir.join(d), format!("home/agent/{in_home}"))));
        }
        b
    }

    /// Writes the box's /etc/resolv.conf from Android's active network (shared by all sessions).
    pub async fn refresh_dns(&self) {
        let out = tokio::process::Command::new("dumpsys").arg("connectivity").output().await;
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let mut servers: Vec<String> = text
            .lines()
            .find_map(|l| l.split_once("DnsAddresses: [").map(|(_, rest)| rest.split(']').next().unwrap_or("").to_owned()))
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().trim_start_matches('/').to_owned())
            .filter(|s| s.parse::<std::net::Ipv4Addr>().is_ok())
            .collect();
        if servers.is_empty() {
            servers.push("1.1.1.1".into());
        }
        let conf: String = servers.iter().map(|s| format!("nameserver {s}\n")).collect();
        let _ = tokio::fs::write(self.root.join("rootfs/etc/resolv.conf"), conf).await;
    }

    /// A command that runs `program` (a path inside the box) as the agent user, in `cwd` (inside
    /// the box), with only `env` (plus HOME, USER, PATH, LANG, TERM).
    #[cfg(any(target_os = "android", target_os = "linux"))]
    pub fn command(&self, program: &str, args: &[&str], cwd: &str, env: &[(&str, &str)]) -> anyhow::Result<tokio::process::Command> {
        imp::command(&self.root, &self.binds(), program, args, cwd, env)
    }

    #[cfg(not(any(target_os = "android", target_os = "linux")))]
    pub fn command(&self, _program: &str, _args: &[&str], _cwd: &str, _env: &[(&str, &str)]) -> anyhow::Result<tokio::process::Command> {
        anyhow::bail!("the agent box only exists on the R1")
    }
}

/// Owner and mode for a directory r1d makes in the box: the agent user's (0700), or root-only.
#[cfg(unix)]
fn restrict(path: &Path, uid: Option<u32>) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let id = uid.unwrap_or(0);
    std::os::unix::fs::chown(path, Some(id), Some(id))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _uid: Option<u32>) -> anyhow::Result<()> {
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(e.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(any(target_os = "android", target_os = "linux"))]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::io;

    const AUDIT_ARCH_AARCH64: u32 = 0xC000_00B7;
    /// The agent box's SELinux domain (device/aosp/vendor_r1os/sepolicy/r1d_agent.te).
    const AGENT_CONTEXT: &str = "u:r:r1d_agent:s0";
    /// aarch64 syscall numbers refused inside the box (EPERM).
    const DENIED: &[u32] = &[
        39, 40, 41, 51, // umount2, mount, pivot_root, chroot
        97, 268, // unshare, setns
        117, 270, 271, // ptrace, process_vm_readv, process_vm_writev
        104, 294, 105, 273, 106, // kexec_load, kexec_file_load, init_module, finit_module, delete_module
        280, 241, 282, // bpf, perf_event_open, userfaultfd
        217, 218, 219, // add_key, request_key, keyctl
        224, 225, 142, 89, 60, // swapon, swapoff, reboot, acct, quotactl
        170, 112, 171, 266, // settimeofday, clock_settime, adjtimex, clock_adjtime
        264, 265, // name_to_handle_at, open_by_handle_at
        116, 58, // syslog, vhangup
        425, 426, 427, // io_uring_setup, io_uring_enter, io_uring_register
        428, 429, 430, 431, 432, 433, // open_tree, move_mount, fsopen, fsconfig, fsmount, fspick
    ];

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    const BPF_LD_W_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const BPF_JEQ_K: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
    const BPF_RET_K: u16 = 0x06; // BPF_RET | BPF_K
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;

    fn filter() -> Vec<SockFilter> {
        let stmt = |code, k| SockFilter { code, jt: 0, jf: 0, k };
        let mut f = vec![
            stmt(BPF_LD_W_ABS, 4), // seccomp_data.arch
            SockFilter { code: BPF_JEQ_K, jt: 1, jf: 0, k: AUDIT_ARCH_AARCH64 },
            stmt(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
            stmt(BPF_LD_W_ABS, 0), // seccomp_data.nr
        ];
        for &nr in DENIED {
            f.push(SockFilter { code: BPF_JEQ_K, jt: 0, jf: 1, k: nr });
            f.push(stmt(BPF_RET_K, SECCOMP_RET_ERRNO | libc::EPERM as u32));
        }
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ALLOW));
        f
    }

    fn cstr(s: impl AsRef<str>) -> io::Result<CString> {
        CString::new(s.as_ref()).map_err(|_| io::Error::other("NUL in path"))
    }

    fn check(rc: libc::c_int, what: &'static str) -> io::Result<()> {
        if rc == 0 { Ok(()) } else { Err(io::Error::new(io::Error::last_os_error().kind(), what)) }
    }

    /// Creates the memory cgroup (once) and returns its cgroup.procs path, if memcg exists.
    fn memcg() -> Option<CString> {
        let dir = Path::new(MEMCG);
        if !Path::new("/dev/memcg").exists() {
            return None;
        }
        if !dir.exists() {
            std::fs::create_dir(dir).ok()?;
        }
        let _ = std::fs::write(dir.join("memory.limit_in_bytes"), MEMORY_LIMIT.to_string());
        CString::new(format!("{MEMCG}/cgroup.procs")).ok()
    }

    pub fn command(
        root: &Path,
        binds: &[(PathBuf, String)],
        program: &str,
        args: &[&str],
        cwd: &str,
        env: &[(&str, &str)],
    ) -> anyhow::Result<tokio::process::Command> {
        let r = root.join("rootfs");
        let p = |rel: &str| cstr(format!("{}/{rel}", r.display()));
        // Everything the child needs, prepared before fork (no allocation between fork and exec).
        let rootfs = cstr(r.display().to_string())?;
        let proc_ = p("proc")?;
        let tmp = p("tmp")?;
        let dev = p("dev")?;
        let dev_pts = p("dev/pts")?;
        let dev_shm = p("dev/shm")?;
        let dev_ptmx = p("dev/ptmx")?;
        let nodes: Vec<(CString, CString)> = ["null", "zero", "random", "urandom", "tty"]
            .iter()
            .map(|n| Ok((cstr(format!("/dev/{n}"))?, p(&format!("dev/{n}"))?)))
            .collect::<io::Result<_>>()?;
        let binds: Vec<(CString, CString)> =
            binds.iter().map(|(src, dst)| Ok((cstr(src.display().to_string())?, p(dst)?))).collect::<io::Result<_>>()?;
        let cwd_c = cstr(cwd)?;
        let procs = memcg();
        let filter = filter();
        let lit = |s: &str| CString::new(s).unwrap();
        let (k_proc, k_tmpfs, k_devpts, k_none) = (lit("proc"), lit("tmpfs"), lit("devpts"), lit("none"));
        let tmp_opts = lit("mode=1777,size=512m");
        let dev_opts = lit("mode=755,size=1m");
        let pts_opts = lit("newinstance,ptmxmode=0666,mode=620");
        let pts_target = lit("pts/ptmx");
        let groups: [libc::gid_t; 2] = [AGENT_UID, INET_GID];
        let attr_current = lit("/proc/thread-self/attr/current");
        let selinux = cfg!(target_os = "android");
        // On r1os's image the domain must be there: without it the box would run in r1d's own, far wider (T-185).
        // Only the Magisk developer base, whose policy has no `r1d_agent`, runs the box without it.
        let need_domain = selinux && r1os_image();
        if selinux && !need_domain {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| tracing::warn!("not r1os's image: the agent box runs in r1d's SELinux domain if the policy has no r1d_agent"));
        }

        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).env_clear();
        for (k, v) in [
            ("HOME", "/home/agent"),
            ("USER", "agent"),
            ("SHELL", "/bin/bash"),
            ("PATH", "/home/agent/.local/bin:/usr/local/bin:/usr/bin:/bin"),
            ("LANG", "C.UTF-8"),
            ("TERM", "dumb"),
            ("USE_BUILTIN_RIPGREP", "0"),
        ]
        .into_iter()
        .chain(env.iter().copied())
        {
            cmd.env(k, v);
        }

        // SAFETY: runs in the forked child before exec. Only syscalls on data prepared above.
        unsafe {
            cmd.pre_exec(move || {
                use libc::*;
                if let Some(procs) = &procs {
                    let fd = open(procs.as_ptr(), O_WRONLY | O_CLOEXEC);
                    if fd >= 0 {
                        let mut buf = [0u8; 16];
                        let mut n = getpid() as u32;
                        let mut i = buf.len();
                        loop {
                            i -= 1;
                            buf[i] = b'0' + (n % 10) as u8;
                            n /= 10;
                            if n == 0 {
                                break;
                            }
                        }
                        write(fd, buf[i..].as_ptr().cast(), buf.len() - i);
                        close(fd);
                    }
                }
                check(unshare(CLONE_NEWNS), "unshare")?;
                check(mount(std::ptr::null(), c"/".as_ptr(), std::ptr::null(), MS_REC | MS_PRIVATE, std::ptr::null()), "make-rprivate")?;
                check(mount(k_proc.as_ptr(), proc_.as_ptr(), k_proc.as_ptr(), MS_NOSUID | MS_NODEV | MS_NOEXEC, std::ptr::null()), "proc")?;
                check(mount(k_tmpfs.as_ptr(), tmp.as_ptr(), k_tmpfs.as_ptr(), MS_NOSUID | MS_NODEV, tmp_opts.as_ptr().cast()), "tmp")?;
                check(mount(k_tmpfs.as_ptr(), dev.as_ptr(), k_tmpfs.as_ptr(), MS_NOSUID | MS_NOEXEC, dev_opts.as_ptr().cast()), "dev")?;
                for (src, dst) in &nodes {
                    let fd = open(dst.as_ptr(), O_CREAT | O_WRONLY | O_CLOEXEC, 0o666);
                    if fd >= 0 {
                        close(fd);
                    }
                    check(mount(src.as_ptr(), dst.as_ptr(), k_none.as_ptr(), MS_BIND, std::ptr::null()), "dev node")?;
                }
                mkdir(dev_pts.as_ptr(), 0o755);
                mkdir(dev_shm.as_ptr(), 0o1777);
                check(mount(k_devpts.as_ptr(), dev_pts.as_ptr(), k_devpts.as_ptr(), MS_NOSUID | MS_NOEXEC, pts_opts.as_ptr().cast()), "devpts")?;
                symlink(pts_target.as_ptr(), dev_ptmx.as_ptr());
                // The home, the workspace, then an agent's own chat stores over the home's.
                for (src, dst) in &binds {
                    check(mount(src.as_ptr(), dst.as_ptr(), k_none.as_ptr(), MS_BIND, std::ptr::null()), "bind")?;
                }
                check(chroot(rootfs.as_ptr()), "chroot")?;
                check(chdir(cwd_c.as_ptr()), "chdir")?;
                // Empty the capability bounding set while still root (needs CAP_SETPCAP). Past the
                // last capability the kernel answers EINVAL, which ends the loop.
                for cap in 0..64 {
                    if prctl(PR_CAPBSET_DROP, cap, 0, 0, 0) != 0 {
                        break;
                    }
                }
                check(setgroups(groups.len() as _, groups.as_ptr()), "setgroups")?;
                check(setresgid(AGENT_UID, AGENT_UID, AGENT_UID), "setgid")?;
                check(setresuid(AGENT_UID, AGENT_UID, AGENT_UID), "setuid")?;
                check(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), "no_new_privs")?;
                let prog = SockFprog { len: filter.len() as u16, filter: filter.as_ptr() };
                check(prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER as libc::c_ulong, &prog as *const SockFprog as libc::c_ulong, 0, 0), "seccomp")?;
                // Last, the agent box's SELinux domain (ADR-0035): set here (setcon), as the exec can't
                // enter it from a file in /data. Through the box's own /proc, mounted above. A policy
                // without the domain answers EINVAL: on r1os's image that's refused (T-185); on the Magisk
                // base the box runs in r1d's.
                if selinux {
                    let fd = open(attr_current.as_ptr(), O_WRONLY | O_CLOEXEC);
                    if fd < 0 {
                        return Err(io::Error::new(io::Error::last_os_error().kind(), "selinux context"));
                    }
                    let n = write(fd, AGENT_CONTEXT.as_ptr().cast(), AGENT_CONTEXT.len());
                    let err = io::Error::last_os_error();
                    close(fd);
                    if n < 0 && (need_domain || err.raw_os_error() != Some(EINVAL)) {
                        return Err(io::Error::new(err.kind(), "selinux context: the image's policy has no r1d_agent domain"));
                    }
                }
                Ok(())
            });
        }
        Ok(cmd)
    }

    /// Whether this is r1os's own image (`ro.r1os.image.version` set), whose policy has the agent box's domain.
    fn r1os_image() -> bool {
        #[cfg(target_os = "android")]
        {
            static IMAGE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            *IMAGE.get_or_init(|| {
                let mut buf = [0u8; 92];
                // SAFETY: the name is a C string and the buffer is PROP_VALUE_MAX (92) bytes.
                let n = unsafe { libc::__system_property_get(c"ro.r1os.image.version".as_ptr(), buf.as_mut_ptr().cast()) };
                n > 0
            })
        }
        #[cfg(not(target_os = "android"))]
        false
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn filter_shape() {
            let f = super::filter();
            assert_eq!(f.len(), 4 + 2 * super::DENIED.len() + 1);
            assert!(f.len() < 256);
        }
    }
}
