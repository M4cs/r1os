//! r1d: the r1os core daemon (docs/specs/r1d.md, ADR-0002).
//!
//! On the R1 it runs as root, started by init's r1d service on r1os's image (or r1os-base's service.sh
//! under Magisk, the older route). It reads the
//! wheel and side button from evdev, drives the camera motor, and serves the shell to the host
//! app's WebView over 127.0.0.1. On a PC, `--sim` makes it the desktop simulator: no hardware,
//! input through the harness.
// The agents' tool list (crate::mcp) nests json! deeper than its default limit allows.
#![recursion_limit = "256"]

mod accounts;
mod agent;
mod alarms;
mod approvals;
mod appwin;
mod android;
mod apps;
mod auth;
mod backup;
mod boot;
mod cards;
mod broker;
mod chat;
mod core;
mod create;
mod jail;
mod logic_check;
mod journal;
mod mail;
mod mcp;
mod media;
mod notes;
mod notices;
mod player;
mod remote;
mod recorder;
mod wifi;
mod device;
mod device_settings;
mod link;
mod feed;
mod fetch;
mod gesture;
mod glance;
mod look;
mod hub;
mod input;
mod perf;
mod power;
mod pin;
mod routines;
mod share;
mod connections;
mod missed;
mod abilities;
mod browser;
mod scheduler;
mod screens;
mod timers;
mod server;
mod services;
mod system1;
mod translator;
mod tts;
mod session;
mod skills;
mod stops;
mod tool;
mod voice;
mod wardrobe;
mod wake;
mod whatsup;

use anyhow::{Context, bail};
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "\
r1d: r1os core daemon

USAGE: r1d [options]
  --listen ADDR         default 127.0.0.1:7171
  --shell DIR           shell files (default /data/r1os/shell on the R1, shell/dist elsewhere)
  --data DIR            installed apps and their data (default /data/r1os on the R1, .r1os-sim elsewhere)
  --sim                 no hardware: input via the harness, simulated motor (default off the R1)
  --hw                  force hardware mode
  --harness             expose /harness/* (input injection, event log, screenshots)
  --hold-ms N           press length that becomes a hold (default 350)
  --double-ms N         double-tap window (default 280)
  --wheel-guard-ms N    drop wheel ticks this close to a button edge (default 120)
  -h, --help
Logging: RUST_LOG (default info).";

fn parse_args() -> anyhow::Result<hub::Config> {
    let on_device = cfg!(target_os = "android");
    let mut cfg = hub::Config {
        listen: "127.0.0.1:7171".parse()?,
        shell: PathBuf::from(if on_device { "/data/r1os/shell" } else { "shell/dist" }),
        data: PathBuf::from(if on_device { "/data/r1os" } else { ".r1os-sim" }),
        sim: !on_device,
        harness: false,
        gestures: gesture::Config::default(),
    };
    let mut args = std::env::args().skip(1);
    let ms = |v: Option<String>, flag: &str| -> anyhow::Result<Duration> {
        let n: u64 = v.with_context(|| format!("{flag} needs a value"))?.parse().with_context(|| format!("{flag}: not a number"))?;
        Ok(Duration::from_millis(n))
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => cfg.listen = args.next().context("--listen needs an address")?.parse()?,
            "--shell" => cfg.shell = PathBuf::from(args.next().context("--shell needs a directory")?),
            "--data" => cfg.data = PathBuf::from(args.next().context("--data needs a directory")?),
            "--sim" => cfg.sim = true,
            "--hw" => cfg.sim = false,
            "--harness" => cfg.harness = true,
            "--hold-ms" => cfg.gestures.hold = ms(args.next(), &a)?,
            "--double-ms" => cfg.gestures.double = ms(args.next(), &a)?,
            "--wheel-guard-ms" => cfg.gestures.wheel_guard = ms(args.next(), &a)?,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}'\n\n{USAGE}"),
        }
    }
    Ok(cfg)
}

fn main() -> anyhow::Result<()> {
    // r1d is a `net_domain` in the image from v0.2.6, so netd marks its sockets like any app's. Until then it
    // opted out of netd's marking (ANDROID_NO_USE_FWMARK_CLIENT), because netd refused its sockets
    // (EMSGSIZE on every connect, 2026-10-09).
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_ansi(!cfg!(target_os = "android")) // service.sh writes our stderr to a log file
        .init();

    // netdev (under iroh, crate::link) asks for Android's JVM context, and panics if none was ever set. A daemon has no
    // JVM: an empty one makes netdev find no JVM and go without (it checks). iroh's DNS resolver doesn't check, so it's
    // given the network's servers instead and never asks (ADR-0048).
    #[cfg(target_os = "android")]
    // SAFETY: set once, before any thread could read it; the null pointers are refused by jni before use.
    unsafe {
        ndk_context::initialize_android_context(std::ptr::null_mut(), std::ptr::null_mut());
    }

    let cfg = parse_args()?;
    let motor = device::Motor::detect(cfg.sim);

    // Hardware readers feed the same raw channel the harness injects into (Source::Hw vs Sim).
    let (hw_tx, mut hw_rx) = tokio::sync::mpsc::unbounded_channel();
    let devices = if cfg.sim { input::Devices::default() } else { input::spawn(hw_tx) };
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        sim = cfg.sim,
        wheel = ?devices.wheel,
        button = ?devices.button,
        motor_sim = motor.is_sim(),
        "r1d starting"
    );
    if !cfg.sim && (devices.wheel.is_none() || devices.button.is_none()) {
        tracing::warn!("wheel or side button not found; is this Rabbit's kernel?");
    }

    let apps = apps::Apps::open(&cfg.data)?;
    let core = core::Core::open(&cfg.data)?;
    // Before multiple agents, the box had one workspace and one set of CLI chat stores: they become
    // the first agent's (once; crate::jail).
    if let Err(e) = jail::Jail::for_agent(&core.first_agent().id).and_then(|j| j.adopt_legacy_files()) {
        tracing::warn!(error = %format!("{e:#}"), "couldn't move the agent box's files into the first agent");
    }
    let (hub, raw_tx) = hub::Hub::start(cfg, devices, motor, apps, core);
    // Linked computers follow some events (crate::link, ADR-0048).
    link::forward_events(hub.clone());
    // The camera's motor came to rest (ADR-0046): the shell's gauge settles there.
    let h = std::sync::Arc::downgrade(&hub);
    hub.motor.on_rest(move |deg| {
        if let Some(h) = h.upgrade() {
            h.emit(hub::Event::Motor { deg, moving: false }, hub::Source::Api);
        }
    });
    let voice = voice::Voice::start(hub.clone(), hub.cfg.data.join("models").join("stt"));
    // The host's camera and mic (ADR-0031): granted at start, and again when the host asks (a fresh install). Then,
    // with Android's services up, the power policy (T-037, T-176), once per start.
    let h = hub.clone();
    tokio::spawn(async move {
        server::grant_host().await;
        power::apply(&h.core).await;
    });
    // System One (ADR-0032): reads what's said as it's said.
    let (s1_dir, s1_lib) = system1::paths(&hub.cfg.data);
    system1::System1::start(&hub, s1_dir, s1_lib);
    // The Translator (ADR-0034): its worker; models load when its screen opens.
    let (tr_dir, tr_lib) = translator::paths(&hub.cfg.data);
    translator::Translator::init(&hub, tr_dir, tr_lib);
    let _ = hub.voice.set(voice.clone());
    // The token goes where the host app and the tools can read it, before anything can ask (ADR-0026).
    hub.auth.publish();
    let mic_token = hub.auth.enabled().then(|| hub.auth.master().to_owned());
    tokio::spawn(voice::serve_host_mic(voice, mic_token));
    // The time zone from where the R1 is (ADR-0030), once the network's likely up; the shell asks again when
    // a network comes up.
    {
        let h = hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            if let Err(e) = alarms::auto_zone(&h, true).await {
                tracing::warn!(error = %format!("{e:#}"), "time zone: couldn't look it up");
            }
        });
    }
    services::Services::start(&hub);
    scheduler::Scheduler::start(&hub);
    glance::Glance::start(&hub);
    // Idle warm sessions stop after a while; the trash keeps removed apps for a week. No tick (T-035): it sleeps until
    // the next idle process is due, or the trash, or until the sessions nudge it.
    let h = hub.clone();
    tokio::spawn(async move {
        const PURGE_EVERY: Duration = Duration::from_secs(6 * 3600);
        let mut purge_at = tokio::time::Instant::now();
        loop {
            let next = h.sessions.reap(&h).await;
            let now = tokio::time::Instant::now();
            if now >= purge_at {
                let a = h.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    for id in a.apps.purge_trash() {
                        a.core.forget_grants(&id);
                    }
                })
                .await;
                purge_at = now + PURGE_EVERY;
            }
            let until = next.map_or(purge_at, |d| (now + d).min(purge_at));
            tokio::select! {
                () = tokio::time::sleep_until(until) => {}
                () = h.sessions.nudged() => {}
            }
        }
    });
    tokio::spawn(async move {
        while let Some(raw) = hw_rx.recv().await {
            if raw_tx.send((raw, hub::Source::Hw)).is_err() {
                return;
            }
        }
    });

    server::serve(hub).await
}
