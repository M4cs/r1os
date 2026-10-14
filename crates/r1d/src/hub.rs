//! Shared state, the event stream, and the gesture task.
//!
//! Every input goes through one path: raw edges (from evdev, or from the harness in the
//! simulator) → `gesture::Gestures` → stamped `Event`s → broadcast to WebSocket clients and kept
//! in a small ring buffer for the harness. So simulated input exercises the same gesture logic as
//! the real hardware.

use crate::device::Motor;
use crate::gesture::{self, GestureKind, Out, Raw};
use crate::input::Devices;
use serde::Serialize;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};

const LOG_LEN: usize = 256;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub shell: PathBuf,
    /// r1d's own data: installed apps (apps/<id>/enee.yaml) and their databases (data/<id>/).
    pub data: PathBuf,
    /// No hardware: input only comes from the harness, and the motor is simulated.
    pub sim: bool,
    /// Expose /harness/* (input injection, screenshots, event log). Dev builds and the simulator.
    pub harness: bool,
    pub gestures: gesture::Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The R1's wheel and side button.
    Hw,
    /// Injected through the harness (the simulator, tests).
    Sim,
    /// A request from the shell (e.g. moving the motor).
    Api,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Wheel { delta: i32 },
    WheelSuppressed { delta: i32 },
    Button { down: bool },
    /// `woke`: its first press came while the display was off (a double click then wakes straight to home, ADR-0016).
    /// `shortcut`: a triple tap's (the `triple_press` setting: recorder, camera, journal, timer or whats_up).
    Gesture {
        kind: GestureKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        held_ms: Option<u64>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        woke: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        shortcut: Option<String>,
    },
    /// The camera's motor: its target (`moving`), or where it came to rest.
    Motor { deg: u16, moving: bool },
    /// An app's tables changed (a step ran): the shell re-runs the queries that read them.
    AppChanged { app: String, tables: Vec<String> },
    AppInstalled { app: String },
    AppRemoved { app: String },
    /// An open app's live values (crate::services): `/services/<name>` → its latest value. Not kept in the
    /// event log (feeds can be large).
    AppServices { app: String, values: serde_json::Map<String, serde_json::Value> },
    /// The user decided about an app's permissions (crate::broker): screens showing them reload.
    AppPermissions { app: String },
    /// An app's trigger notified (crate::scheduler): the display was woken, and the shell shows it.
    Notify {
        /// Kept until it's seen (crate::notices, T-049): `POST /api/notices/{id}/seen`.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<i64>,
        app: String,
        /// The app's name, icon and accent, so the shell can show it at once.
        name: String,
        icon: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        accent: Option<String>,
        title: String,
        body: String,
    },
    /// An alarm started ringing, or stopped (crate::alarms): the shell's alarm screen.
    Alarm {
        id: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        ringing: bool,
    },
    /// A reminder went off (crate::alarms): the display was woken, and the shell shows it.
    Reminder {
        id: String,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        at: i64,
    },
    /// Alarms or reminders were set, changed or removed: what shows them reloads.
    AlarmsChanged,
    /// A timer started ringing, or stopped (crate::timers, ADR-0050): the shell's alarm screen.
    Timer {
        id: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        ringing: bool,
    },
    /// A pomodoro moved on to its next phase (`focus`, `break` or `long`), which runs until `ends`: the display was
    /// woken, and the shell says what's next.
    TimerPhase {
        id: String,
        label: String,
        phase: String,
        round: i64,
        rounds: i64,
        length: i64,
        ends: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
    },
    /// Timers were started, changed or removed: what shows them reloads.
    TimersChanged,
    /// The PIN lock (crate::pin, ADR-0016): whether one is `set`, and whether the R1 is `locked` now.
    Lock { set: bool, locked: bool },
    /// A routine ran and asked to be shown (crate::routines): the display was woken; a note says its reply is there.
    Routine { id: String, agent: String, chat: String, title: String, text: String },
    /// Routines were made, changed or removed: what shows them reloads.
    RoutinesChanged,
    /// The host holds the CPU awake (its wake lock) while `why` runs with the screen off: a routine's turn.
    Awake { why: String, on: bool },
    /// Open one of r1os's own screens (crate::screens): the shell's target (`{t: tool, id}`, `{t: settings, page}`…).
    OpenScreen { target: serde_json::Value },
    /// The host draws the boot animation now (crate::boot): r1os's own cast for the image (`default`), else yours.
    Bootanim { default: bool },
    /// What the agents' CLIs reach changed (crate::connections, ADR-0052): a connector, its status, or an agent's rules.
    ConnectionsChanged,
    /// Do `op` in the R1's browser (crate::browser, ADR-0054); the host answers at `POST /api/browser/result`.
    Browser { id: String, op: String, agent: String, chat: String, args: serde_json::Value },
    /// Do `op` in the app window, an Android app an agent drives (crate::appwin, ADR-0061); the host answers at
    /// `POST /api/apps/window/result`.
    AppWindow { id: String, op: String, agent: String, chat: String, args: serde_json::Value },
    /// A photo or recording was added, changed or removed (crate::media): galleries reload.
    MediaChanged { kind: String },
    /// The Journal changed (crate::journal): its screen reloads.
    JournalChanged,
    /// The Recorder (crate::recorder): `recording` about every second with the length and the transcript so
    /// far, then `saved`.
    Recording { id: String, state: String, seconds: i64, text: String },
    /// The Recorder's level, about 10 times a second while the display is on (crate::recorder; not kept in
    /// the harness's log).
    RecordingLevel { level: f32 },
    /// The Journal's voice notes (crate::notes): listening, partial words, saved, cancelled, error.
    Note(crate::notes::Note),
    /// An app's page reported a problem (crate::apps errors): its page in Manage apps offers a fix.
    AppErrors { app: String, count: usize },
    /// An app's `open_url` step (r1d checked it and its permission): the native shell opens the link in its viewer.
    /// Not `app_…`: the shell keeps it from the app's page.
    OpenLink { app: String, url: String },
    /// An app's `copy` step: the native shell puts the text on the clipboard.
    Copy { app: String, text: String },
    /// The R1's player (crate::player, ADR-0040), for the host: `play` this (`url` is what to open: r1d's relay,
    /// or an HLS stream itself), or `pause`, `resume`, `stop`, `toggle` what's playing. Not `app_…`: for the shell.
    Media {
        op: String,
        app: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        subtitle: Option<String>,
    },
    /// An agent asks the host to `open` (its app comes up) or `dismiss` an Android app's notification.
    /// An Android app's music (R7): `op` pause, resume, toggle, stop, next, previous, or `play` (`uri` in `package`
    /// when there's one, which plays at once; else `query`, which the app searches for).
    AndroidMedia {
        op: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    AndroidNotice {
        op: String,
        key: String,
        /// A reply's words.
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// What the player is doing, for the app that's playing: its page's `/player`.
    AppPlayer { app: String, player: serde_json::Value },
    /// What the player is doing, whichever app it is (the glance screen, crate::player::State).
    Player { player: serde_json::Value },
    /// The glance screen's widgets changed (crate::glance, ADR-0042): all of them, in order.
    Glance { widgets: Vec<serde_json::Value> },
    /// The device's look changed (crate::look, ADR-0043): all of it.
    Look { look: serde_json::Value },
    /// The apps pinned to home changed (ADR-0044): all of them, in order.
    Home { pins: Vec<String> },
    /// An agent asks to change something (ADR-0045): the shell shows the approval card.
    ApprovalAsked { approval: serde_json::Value },
    /// It was answered (or nobody did in time): the card goes.
    ApprovalDone { id: String, allowed: bool },
    /// The host applies these Android settings (crate::device_settings): brightness, timeout, volume, sounds.
    DeviceSet { changes: serde_json::Value },
    /// Pairing with a computer (crate::link, ADR-0048): `paired`, `refused` or `failed`, with the code both showed.
    Link { state: String, name: String, code: String, why: Option<String> },
    /// A linked computer changed something here (crate::link): the shell says so at the top for a moment.
    ChangedFrom { computer: String, what: String },
    /// A sign-in changed: waiting · code_entered · signed_in · signed_out · failed · expired.
    Account { brain: String, state: String, #[serde(skip_serializing_if = "Option::is_none")] detail: Option<String> },
    /// A conversation: user · delta · tool · build · done · error · note (r1d tidied the context) ·
    /// mail (another agent's message landed here) · mail_out (the agent sent one from here; ADR-0012).
    Chat {
        /// The agent the chat is with.
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        chat: String,
        kind: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        build: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        app: Option<String>,
        /// A mail card's details, as the message's meta keeps them (crate::mail).
        #[serde(skip_serializing_if = "Option::is_none")]
        mail: Option<serde_json::Value>,
        /// A card the agent showed (`kind: card`, crate::cards).
        #[serde(skip_serializing_if = "Option::is_none")]
        card: Option<serde_json::Value>,
    },
    /// What an agent is doing in a chat: idle · waiting (for room to run) · thinking · speaking ·
    /// working · error (the home character).
    AgentState {
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        chat: Option<String>,
        /// The agent whose message started this turn (ADR-0012), not the user: the shell keeps its
        /// focus where it was. Filled in by [`Hub::emit`].
        #[serde(skip_serializing_if = "Option::is_none")]
        by: Option<String>,
    },
    /// An agent was added, changed, removed or reordered: the shell reloads `/api/agents`.
    AgentsChanged,
    /// Replies the user hasn't seen changed: `count` is the agent's total, after a reply in `chat`
    /// or the user seeing it. `from`: another agent's message caused it; `mail`: the newest one still
    /// waiting for the user (`{from, kind, text}`, ADR-0012), which home shows.
    Unread {
        agent: String,
        chat: String,
        count: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mail: Option<serde_json::Value>,
    },
    /// One agent messaged another (crate::mail, ADR-0012): ask · tell · reply. `woke`: it runs a turn now.
    Mail { from: String, to: String, kind: String, text: String, woke: bool },
    /// The agent asked the shell to open an app (mcp `open_app`).
    Open { app: String },
    /// An app build's progress (crate::create).
    Build {
        id: u64,
        stage: crate::create::Stage,
        #[serde(flatten)]
        progress: crate::create::Progress,
    },
    /// Push-to-talk (crate::voice): listening · partial · final · cancelled · error.
    Voice {
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// On `final`: what System One makes of the words (crate::system1), if it's sure.
        #[serde(skip_serializing_if = "Option::is_none")]
        intent: Option<serde_json::Value>,
    },
    /// System One while you talk (crate::system1): what's coming, so the shell can get ready (null: nothing).
    Intent { state: String, intent: Option<serde_json::Value> },
    /// The Translator (crate::translator): a turn's progress, listening → hearing → heard → translated →
    /// spoken (or error / cancelled).
    Translation(crate::translator::Turn),
    /// How ready the Translator is (crate::translator): `loading`, `english` (English can be translated; the
    /// other language's ears are still loading), `ready`, or `off` (its models were dropped).
    TranslatorStage { stage: String },
    /// The mic level while listening, 0–1, ~15 times a second. Not kept in the event log.
    VoiceLevel { level: f32 },
    /// Hold + wheel (ADR-0014): the second wheel tick of a press turns it into a volume change
    /// (`on: true`, push-to-talk cancelled); letting go ends it (`on: false`). The shell sets the
    /// volume (Android's AudioManager, from the host) and draws the level.
    Volume { on: bool },
    /// The harness says how the R1 is held (T-156): 1 on its right side, -1 its left, 0 upright; `None` back to the
    /// accelerometer. For trying docked mode without hands on the R1.
    Tilt { side: Option<i32> },
    /// The harness holds the glance screen's rest at a step (T-157): 0 awake, 1 still, 2 low; `None` back to its own
    /// timer. `brightness` (0..1) tries another low backlight. For measuring each step without waiting.
    Rest { step: Option<i32>, brightness: Option<f64> },
    /// Touch's cancel of a talk (T-156, docked): crate::voice drops what it heard, with no turn to volume.
    TalkCancel,
}

impl Event {
    /// A `chat` event in `agent`'s chat.
    pub fn chat(agent: &str, chat: &str, kind: &str, text: Option<String>) -> Event {
        let agent = (!agent.is_empty()).then(|| agent.to_owned());
        Event::Chat { agent, chat: chat.to_owned(), kind: kind.to_owned(), text, build: None, app: None, mail: None, card: None }
    }

    pub fn with_build(mut self, id: u64) -> Event {
        if let Event::Chat { build, .. } = &mut self {
            *build = Some(id);
        }
        self
    }

    pub fn with_app(mut self, id: Option<String>) -> Event {
        if let Event::Chat { app, .. } = &mut self {
            *app = id;
        }
        self
    }

    pub fn with_mail(mut self, meta: serde_json::Value) -> Event {
        if let Event::Chat { mail, .. } = &mut self {
            *mail = Some(meta);
        }
        self
    }

    pub fn with_card(mut self, c: serde_json::Value) -> Event {
        if let Event::Chat { card, .. } = &mut self {
            *card = Some(c);
        }
        self
    }

    pub fn agent_state(agent: &str, state: &str, chat: &str) -> Event {
        Event::AgentState { agent: Some(agent.to_owned()), state: state.to_owned(), chat: Some(chat.to_owned()), by: None }
    }
}

/// How busy a state is: an agent with several chats running shows the busiest.
fn rank(state: &str) -> u8 {
    match state {
        "working" => 4,
        "speaking" => 3,
        "thinking" => 2,
        "waiting" => 1,
        _ => 0,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Stamped {
    pub seq: u64,
    /// Milliseconds since r1d started.
    pub t_ms: u64,
    pub source: Source,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Stats {
    pub wheel: u64,
    pub wheel_suppressed: u64,
    pub buttons: u64,
    pub gestures: u64,
}

pub struct Hub {
    pub cfg: Config,
    pub started: Instant,
    pub devices: Devices,
    pub motor: Motor,
    pub apps: Arc<crate::apps::Apps>,
    /// Who may call r1d: the per-boot token and apps' page tokens (crate::auth).
    pub auth: crate::auth::Auth,
    /// Open apps' live data (crate::services).
    pub services: crate::services::Services,
    /// Apps' triggers: reminders and the like (crate::scheduler).
    pub scheduler: crate::scheduler::Scheduler,
    pub builds: crate::create::Builds,
    pub core: crate::core::Core,
    pub accounts: Arc<crate::accounts::Accounts>,
    pub tool_tokens: crate::mcp::Tokens,
    /// Running turns you can stop (crate::stops).
    pub stops: crate::stops::Stops,
    /// Latency marks for `r1h perf` (crate::perf).
    pub perf: crate::perf::Perf,
    /// Chats with a turn in flight (one turn per chat at a time).
    pub chat_busy: Mutex<std::collections::HashSet<String>>,
    /// Agent processes in the box: warm Claude sessions and the budget every process shares
    /// (crate::session).
    pub sessions: crate::session::Sessions,
    /// What each agent is doing, per chat with a turn in flight (agent → chat → state).
    agent_states: Mutex<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,
    /// Push-to-talk, set once at startup (it needs the hub to exist first).
    pub voice: std::sync::OnceLock<Arc<crate::voice::Voice>>,
    /// The Recorder (crate::recorder).
    pub recorder: crate::recorder::Recorder,
    /// System One (crate::system1).
    pub system1: crate::system1::System1,
    /// The Translator (crate::translator).
    pub translator: crate::translator::Translator,
    /// The Journal's voice notes (crate::notes).
    pub notes: crate::notes::Notes,
    /// Agents' messages to each other (crate::mail).
    pub mail: crate::mail::Mail,
    /// The Android apps installed, as the host last listed them (crate::android).
    pub android: crate::android::Android,
    /// The R1's player for apps (crate::player).
    pub player: crate::player::Player,
    /// Apps' cards on the glance screen (crate::glance).
    pub glance: crate::glance::Glance,
    /// Agents' requests to change things, waiting for your answer (crate::approvals).
    pub approvals: crate::approvals::Approvals,
    /// What the agents' CLIs reach on your accounts (crate::connections, ADR-0052).
    pub connections: crate::connections::Connections,
    /// The R1's browser: requests waiting for the host, and turns allowed to act (crate::browser, ADR-0054).
    pub browser: crate::browser::Browser,
    /// The app window: requests waiting for the host, and turns allowed to act in an app (crate::appwin, ADR-0061).
    pub app_window: crate::appwin::AppWindow,
    /// The R1's own settings, as the host reports them (crate::device_settings).
    pub device_settings: crate::device_settings::DeviceSettings,
    /// Linked computers (crate::link, ADR-0048).
    pub link: crate::link::Link,
    /// Sharing by QR, out and in (crate::share, ADR-0056).
    pub shares: crate::share::Shares,
    /// The PIN lock: whether the R1 is locked (crate::pin, ADR-0016).
    pub pin: crate::pin::Pin,
    /// The wheel and button settings the gestures follow (Settings › Controls): kept here so input needn't read
    /// the database.
    pub controls: Controls,
    raw_tx: mpsc::UnboundedSender<(Raw, Source)>,
    events: broadcast::Sender<Stamped>,
    log: Mutex<VecDeque<Stamped>>,
    stats: Mutex<Stats>,
    seq: AtomicU64,
}

impl Hub {
    /// Builds the hub and starts the gesture task. `devices` are the evdev readers already running
    /// (their raw input arrives through `raw_sender`).
    pub fn start(
        cfg: Config,
        devices: Devices,
        motor: Motor,
        apps: crate::apps::Apps,
        core: crate::core::Core,
    ) -> (Arc<Self>, mpsc::UnboundedSender<(Raw, Source)>) {
        let (raw_tx, raw_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(512);
        let services = crate::services::Services::new(&cfg.data);
        // `/data/r1os/auth-off` (root-only, like everything there) opens the API: a way back if the host
        // ever can't read its token. Loud in the log.
        let off = cfg.data.join("auth-off").exists();
        if off && !cfg.sim {
            tracing::warn!("auth: OFF (/data/r1os/auth-off exists): any local process can call r1d");
        }
        let auth = crate::auth::Auth::new(!cfg.sim && !off, &cfg.data);
        let pin = crate::pin::Pin::new(&core);
        let controls = Controls::new(&core);
        let hub = Arc::new(Self {
            cfg,
            started: Instant::now(),
            devices,
            motor,
            apps: Arc::new(apps),
            auth,
            services,
            scheduler: crate::scheduler::Scheduler::new(),
            builds: crate::create::Builds::default(),
            core,
            accounts: Arc::default(),
            tool_tokens: crate::mcp::Tokens::default(),
            stops: crate::stops::Stops::default(),
            perf: crate::perf::Perf::default(),
            chat_busy: Mutex::default(),
            sessions: crate::session::Sessions::default(),
            agent_states: Mutex::default(),
            voice: std::sync::OnceLock::new(),
            recorder: crate::recorder::Recorder::default(),
            system1: crate::system1::System1::default(),
            translator: crate::translator::Translator::default(),
            notes: crate::notes::Notes::default(),
            mail: crate::mail::Mail::default(),
            android: crate::android::Android::default(),
            player: crate::player::Player::default(),
            glance: crate::glance::Glance::default(),
            approvals: crate::approvals::Approvals::default(),
            connections: crate::connections::Connections::default(),
            browser: crate::browser::Browser::default(),
            app_window: crate::appwin::AppWindow::default(),
            device_settings: crate::device_settings::DeviceSettings::default(),
            link: crate::link::Link::default(),
            shares: crate::share::Shares::default(),
            pin,
            controls,
            raw_tx: raw_tx.clone(),
            events,
            log: Mutex::new(VecDeque::with_capacity(LOG_LEN)),
            stats: Mutex::new(Stats::default()),
            seq: AtomicU64::new(0),
        });
        tokio::spawn(run_gestures(hub.clone(), raw_rx));
        (hub, raw_tx)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Stamped> {
        self.events.subscribe()
    }

    pub fn clients(&self) -> usize {
        self.events.receiver_count()
    }

    /// Injects a raw input as if it came from hardware (the harness and the simulator use this).
    pub fn inject(&self, raw: Raw) {
        let _ = self.raw_tx.send((raw, Source::Sim));
    }

    pub fn emit(&self, mut event: Event, source: Source) {
        // A turn another agent's message started says so, whichever part of r1d reports it.
        if let Event::AgentState { chat: Some(chat), by, .. } = &mut event
            && by.is_none()
        {
            *by = self.mail.woken(chat).map(|w| w.by);
        }
        {
            let mut s = self.stats.lock().unwrap();
            match &event {
                Event::Wheel { .. } => s.wheel += 1,
                Event::WheelSuppressed { .. } => s.wheel_suppressed += 1,
                Event::Button { .. } => s.buttons += 1,
                Event::Gesture { .. } => s.gestures += 1,
                Event::Motor { .. }
                | Event::AppChanged { .. }
                | Event::AppInstalled { .. }
                | Event::AppRemoved { .. }
                | Event::AppServices { .. }
                | Event::AppPermissions { .. }
                | Event::Notify { .. }
                | Event::Alarm { .. }
                | Event::Reminder { .. }
                | Event::AlarmsChanged
                | Event::Timer { .. }
                | Event::TimerPhase { .. }
                | Event::TimersChanged
                | Event::Lock { .. }
                | Event::Routine { .. }
                | Event::RoutinesChanged
                | Event::Awake { .. }
                | Event::Tilt { .. }
                | Event::Rest { .. }
                | Event::TalkCancel
                | Event::OpenScreen { .. }
                | Event::Bootanim { .. }
                | Event::ConnectionsChanged
                | Event::Browser { .. }
                | Event::AppWindow { .. }
                | Event::MediaChanged { .. }
                | Event::Recording { .. }
                | Event::RecordingLevel { .. }
                | Event::Note(_)
                | Event::JournalChanged
                | Event::OpenLink { .. }
                | Event::AppErrors { .. }
                | Event::Copy { .. }
                | Event::Media { .. }
                | Event::AndroidNotice { .. }
                | Event::AndroidMedia { .. }
                | Event::AppPlayer { .. }
                | Event::Player { .. }
                | Event::Glance { .. }
                | Event::Look { .. }
                | Event::Home { .. }
                | Event::ApprovalAsked { .. }
                | Event::ApprovalDone { .. }
                | Event::DeviceSet { .. }
                | Event::Link { .. }
                | Event::ChangedFrom { .. }
                | Event::Build { .. }
                | Event::Account { .. }
                | Event::Chat { .. }
                | Event::AgentState { .. }
                | Event::AgentsChanged
                | Event::Unread { .. }
                | Event::Mail { .. }
                | Event::Open { .. }
                | Event::Voice { .. }
                | Event::Intent { .. }
                | Event::Translation(_)
                | Event::TranslatorStage { .. }
                | Event::VoiceLevel { .. }
                | Event::Volume { .. } => {}
            }
        }
        if let Event::AgentState { agent: Some(agent), state, chat: Some(chat), .. } = &event {
            let mut states = self.agent_states.lock().unwrap();
            let chats = states.entry(agent.clone()).or_default();
            // Only work in flight is kept: idle and error end the chat's turn.
            if rank(state) > 0 {
                chats.insert(chat.clone(), state.clone());
            } else {
                chats.remove(chat);
            }
        }
        // What's due may have changed: an app came or went, a permission, a setting a trigger's time binds.
        match &event {
            Event::AppInstalled { .. } | Event::AppRemoved { .. } | Event::AppPermissions { .. } => {
                self.scheduler.wake();
                // Which apps fetch in the background (`schedule`) may have changed.
                self.services.background_changed();
            }
            Event::AppChanged { tables, .. } if tables.iter().any(|t| t == "settings") => self.scheduler.wake(),
            _ => {}
        }
        // The glance screen's cards (ADR-0042): an app's change fills its card again.
        match &event {
            Event::AppInstalled { app, .. } => self.glance.installed(self, app),
            Event::AppRemoved { app, .. } => {
                self.glance.removed(self, app);
                // Off home too (the shell reads its pins again on `app_removed`).
                let pins = self.core.home_pins();
                if pins.iter().any(|p| p == app) {
                    let kept: Vec<String> = pins.into_iter().filter(|p| p != app).collect();
                    let _ = self.core.set_home_pins(&kept);
                }
            }
            Event::AppChanged { app, .. } | Event::AppServices { app, .. } | Event::AppPermissions { app, .. } => {
                self.glance.dirty(app)
            }
            _ => {}
        }
        let stamped = Stamped {
            seq: self.seq.fetch_add(1, Ordering::Relaxed) + 1,
            t_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            source,
            event,
        };
        tracing::debug!(?stamped, "event");
        if !matches!(stamped.event, Event::VoiceLevel { .. } | Event::AppServices { .. } | Event::Recording { .. } | Event::RecordingLevel { .. }) {
            let mut log = self.log.lock().unwrap();
            if log.len() == LOG_LEN {
                log.pop_front();
            }
            log.push_back(stamped.clone());
        }
        let _ = self.events.send(stamped); // no subscribers is fine
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().unwrap().clone()
    }

    /// Tells the shell how many replies `agent` has waiting, after a reply or a seen in `chat`.
    pub fn unread_changed(&self, agent: &str, chat: &str) {
        self.unread_from(agent, chat, None);
    }

    /// The same, after another agent's message (`from`) landed in `chat` (ADR-0012).
    pub fn unread_from(&self, agent: &str, chat: &str, from: Option<&str>) {
        match self.core.unread(agent) {
            Ok(count) => self.emit(
                Event::Unread {
                    agent: agent.to_owned(),
                    chat: chat.to_owned(),
                    count,
                    from: from.map(str::to_owned),
                    mail: crate::mail::latest(self, agent),
                },
                Source::Api,
            ),
            Err(e) => tracing::warn!(agent, error = %e, "couldn't count unread replies"),
        }
    }

    /// What `agent` is doing (its busiest chat), and in which chat; idle when nothing is running.
    pub fn agent_state(&self, agent: &str) -> (String, Option<String>) {
        let states = self.agent_states.lock().unwrap();
        states
            .get(agent)
            .and_then(|chats| chats.iter().max_by_key(|(_, s)| rank(s)))
            .map_or_else(|| ("idle".to_owned(), None), |(c, s)| (s.clone(), Some(c.clone())))
    }

    /// Events with `seq > after`, oldest first (at most the ring buffer's length).
    pub fn events_after(&self, after: u64) -> Vec<Stamped> {
        self.log.lock().unwrap().iter().filter(|e| e.seq > after).cloned().collect()
    }
}

/// The wheel's direction and the triple press's shortcut (Settings › Controls; core.db keeps them).
pub struct Controls {
    invert: std::sync::atomic::AtomicBool,
    triple: Mutex<String>,
}

impl Controls {
    fn new(core: &crate::core::Core) -> Self {
        Controls { invert: core.invert_wheel().into(), triple: Mutex::new(core.triple_press()) }
    }

    pub fn invert(&self) -> bool {
        self.invert.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_invert(&self, on: bool) {
        self.invert.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// The triple press's shortcut, or None when it's off.
    pub fn triple(&self) -> Option<String> {
        Some(self.triple.lock().unwrap().clone()).filter(|t| t != "off")
    }

    pub fn set_triple(&self, s: &str) {
        *self.triple.lock().unwrap() = s.to_owned();
    }
}

async fn run_gestures(hub: Arc<Hub>, mut rx: mpsc::UnboundedReceiver<(Raw, Source)>) {
    let mut g = gesture::Gestures::new(hub.cfg.gestures);
    let mut source = Source::Hw;
    // A gesture under way, and whether its first press came with the display off: the press that wakes the R1 is
    // ignored by the shell, but a double click that woke it goes home (ADR-0016), so it's marked.
    let mut pressing = false;
    let mut asleep = false;
    loop {
        let next = match g.deadline() {
            Some(deadline) => tokio::select! {
                r = rx.recv() => Some(r),
                () = tokio::time::sleep_until(deadline.into()) => None,
            },
            None => Some(rx.recv().await),
        };
        g.set_triple(hub.controls.triple().is_some());
        let outs = match next {
            Some(Some((raw, src))) => {
                source = src;
                g.input(raw, Instant::now())
            }
            Some(None) => return, // every sender dropped
            None => g.tick(Instant::now()),
        };
        for out in outs {
            hub.emit(
                match out {
                    // Inverted (Settings › Controls), for everything r1os does with the wheel.
                    Out::Wheel(delta) => Event::Wheel { delta: if hub.controls.invert() { -delta } else { delta } },
                    Out::WheelSuppressed(delta) => Event::WheelSuppressed { delta: if hub.controls.invert() { -delta } else { delta } },
                    Out::Button(down) => {
                        if down && !pressing {
                            pressing = true;
                            asleep = !hub.services.display_on();
                        }
                        Event::Button { down }
                    }
                    Out::Gesture { kind, held_ms } => {
                        let woke = asleep;
                        if matches!(kind, GestureKind::Tap | GestureKind::DoubleTap | GestureKind::TripleTap | GestureKind::HoldEnd) {
                            pressing = false;
                            asleep = false;
                        }
                        let shortcut = if kind == GestureKind::TripleTap { hub.controls.triple() } else { None };
                        Event::Gesture { kind, held_ms, woke, shortcut }
                    }
                },
                source,
            );
        }
    }
}
