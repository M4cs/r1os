//! Agent processes in the box (ADR-0010, ADR-0011; docs/specs/agents.md §3).
//!
//! - **One budget:** at most MAX_PROCS agent CLI processes in the box at once (~250–300 MB each),
//!   whatever they are: warm Claude sessions, Codex turns, app builds. Each holds a permit for its
//!   life ([`Sessions::room`]). A process that needs room first stops the least recently used idle
//!   warm session; when every permit belongs to a running turn, it waits (the agent shows
//!   "waiting"). So agents run in parallel, up to the budget.
//! - **Warm Claude sessions:** one `claude -p --input-format stream-json` process per active chat,
//!   kept between messages, so a reply doesn't pay the CLI's start (~5 s on the R1; a warm turn
//!   measured 1.7–2.2 s). The shell asks for one as soon as you start talking
//!   (`POST /api/chats/warm`). A chat with a session gets a process resuming it; a new chat gets a
//!   *spare* for its agent, which its first message adopts.
//! - A process idle for a while is stopped; the next message resumes the session cold. A while is IDLE_ON (15 min)
//!   with the display on, where a cold start (~10 s to the first word) is what you'd wait on, and IDLE_OFF (3 min)
//!   with it off, where an idle CLI only costs (~2% of a core, and its timers wake the CPU; T-043).
//! - **Each process** runs in its agent's jail (crate::jail) with that agent's system prompt
//!   (persona, tools, memories, and the chat's summary when it has one) and an MCP config with its
//!   own tool token, fixed at start. Changing an agent stops its processes.
//! - **Context:** every turn reports how much context the session holds (the last model call's
//!   usage), and [`Sessions::summarize`] asks a session to write the note r1d continues from in a
//!   fresh one (crate::chat::tidy).

use crate::chat::{self, CLAUDE_TOOLS, Turn};
use crate::core::Chat;
use crate::hub::{Event, Hub, Source};
use crate::jail::{CLAUDE, Jail};
use anyhow::{Context, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Agent CLI processes in the box at once (Max, 2026-10-08: two to start, three if the headroom is comfortable, Q3).
/// Three since 2026-10-12 (T-004): two answering at once took MemAvailable from 2.34 GB to 2.03 GB.
pub const MAX_PROCS: usize = 3;
const IDLE_ON: Duration = Duration::from_secs(15 * 60);
const IDLE_OFF: Duration = Duration::from_secs(3 * 60);
/// The reaper wakes this long after a process is due, so it isn't a hair early.
const REAP_SLACK: Duration = Duration::from_millis(500);
/// A turn ends if the CLI says nothing this long (longer than an `app_ask_user` waits for the user: 5 minutes)...
const TURN_QUIET: Duration = Duration::from_secs(6 * 60);
/// ...or runs this long in all. It was 4 minutes in all until T-144: an intricate order in the app window takes more.
const TURN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The context window when the CLI doesn't say (Sonnet's).
const DEFAULT_WINDOW: i64 = 200_000;

struct Live {
    agent: String,
    /// The chat it serves; None for a spare waiting for a new chat of `agent`.
    chat: Option<String>,
    session_id: String,
    /// Started with --session-id: nothing on disk until its first turn finishes.
    fresh: bool,
    token: String,
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    stderr: Arc<Mutex<String>>,
    last_used: Instant,
    /// When it started, and the turns it has taken: warm or cold for `r1h perf` (crate::perf).
    born: Instant,
    turns: u32,
    /// Its place in the budget, given back when it's dropped.
    permit: OwnedSemaphorePermit,
}

pub struct Sessions {
    /// Idle warm processes. One in use is taken out, and put back after its turn.
    live: tokio::sync::Mutex<Vec<Live>>,
    procs: Arc<Semaphore>,
    /// Bumped by `clear`, so a process started from an older profile isn't put back.
    generation: AtomicU64,
    /// One turn (or tidy) at a time per chat.
    chat_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Wakes the reaper: a process was put back, or the display went off (T-035).
    nudge: tokio::sync::Notify,
}

impl Default for Sessions {
    fn default() -> Self {
        Sessions {
            live: tokio::sync::Mutex::default(),
            procs: Arc::new(Semaphore::new(MAX_PROCS)),
            generation: AtomicU64::new(0),
            chat_locks: Mutex::default(),
            nudge: tokio::sync::Notify::new(),
        }
    }
}

impl Sessions {
    /// The lock a chat's turns and tidies take, so they run one after another.
    pub fn chat_lock(&self, chat: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.chat_locks.lock().unwrap().entry(chat.to_owned()).or_default().clone()
    }

    /// Room for one more process in the box: stops idle warm sessions (least recently used first)
    /// if it must, and otherwise waits for a running one to finish. `on_wait` runs once, if it waits.
    pub async fn room(&self, hub: &Hub, on_wait: impl FnOnce()) -> OwnedSemaphorePermit {
        let mut on_wait = Some(on_wait);
        loop {
            if let Ok(p) = self.procs.clone().try_acquire_owned() {
                return p;
            }
            if self.stop_idle(hub, None).await {
                continue;
            }
            if let Some(f) = on_wait.take() {
                f();
            }
            // A running turn's process comes back to the pool idle (still holding its permit), so
            // poll rather than wait on the semaphore alone.
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Room without waiting: None when every process is busy with a turn (warming can skip).
    async fn room_now(&self, hub: &Hub, keep: Option<&str>) -> Option<OwnedSemaphorePermit> {
        loop {
            if let Ok(p) = self.procs.clone().try_acquire_owned() {
                return Some(p);
            }
            if !self.stop_idle(hub, keep).await {
                return None;
            }
        }
    }

    /// Stops the least recently used idle process (not `keep`'s chat's). Whether one was stopped.
    async fn stop_idle(&self, hub: &Hub, keep: Option<&str>) -> bool {
        let mut live = self.live.lock().await;
        let lru = live
            .iter()
            .enumerate()
            .filter(|(_, l)| keep.is_none() || l.chat.as_deref() != keep)
            .min_by_key(|(_, l)| l.last_used)
            .map(|(i, _)| i);
        match lru {
            Some(i) => {
                let l = live.swap_remove(i);
                tracing::info!(agent = %l.agent, chat = ?l.chat, "stopping an idle claude session to make room");
                discard(hub, l);
                true
            }
            None => false,
        }
    }

    /// Makes sure a process is starting or running for `chat`, or a spare for a new chat of
    /// `agent`. Skips quietly when the box is full of running turns.
    pub async fn warm(&self, hub: &Arc<Hub>, agent: &str, chat: Option<&str>) -> anyhow::Result<()> {
        let (agent, session, summary) = match chat {
            Some(id) => {
                let c = hub.core.chat(id)?.context("no such chat")?;
                // A running turn has its process; a running tidy is about to replace the session.
                if c.brain != "claude" || hub.chat_busy.lock().unwrap().contains(id) || self.chat_lock(id).try_lock().is_err() {
                    return Ok(());
                }
                (c.agent, c.session, c.summary.is_some())
            }
            None => {
                let a = hub.core.agent(agent).context("no such agent")?;
                if a.brain != "claude" {
                    return Ok(());
                }
                (a.id, None, false)
            }
        };
        // An agent that works on a computer runs there (crate::remote): nothing to warm here.
        if hub.core.agent(&agent).is_some_and(|a| a.computer().is_some()) {
            return Ok(());
        }
        // A chat that already has a session (or a summary to start from) needs its own process;
        // a new chat is served by a spare.
        let own = session.is_some() || summary;
        let wanted = |l: &Live| l.agent == agent && if own { l.chat.as_deref() == chat } else { l.chat.as_deref() == chat || l.chat.is_none() };
        {
            let mut live = self.live.lock().await;
            if let Some(l) = live.iter_mut().find(|l| wanted(l)) {
                l.last_used = Instant::now();
                return Ok(());
            }
        }
        let Some(permit) = self.room_now(hub, chat).await else {
            tracing::info!(agent = %agent, "no room to warm a session; every process is busy");
            return Ok(());
        };
        let l = spawn(hub, &agent, if own { chat } else { None }, session.as_deref(), permit).await?;
        tracing::info!(agent = %agent, chat = ?l.chat, resume = session.is_some(), "warming a claude session");
        let mut live = self.live.lock().await;
        if live.iter().any(&wanted) {
            discard(hub, l); // another warm got there first
        } else {
            live.push(l);
            self.nudge();
        }
        Ok(())
    }

    /// One user turn in a Claude chat: on its warm process (started now if there isn't one).
    pub async fn turn(&self, hub: &Arc<Hub>, chat: &Chat, text: &str, photos: &[crate::media::Item]) -> anyhow::Result<Turn> {
        let generation = self.generation.load(Ordering::Acquire);
        let mut l = self.take(hub, chat).await?;
        // Photos go in as image blocks before the words (ADR-0031).
        let mut content = Vec::new();
        for p in photos {
            let data = crate::media::photo_b64(&hub.cfg.data, p)?;
            content.push(json!({ "type": "image", "source": { "type": "base64", "media_type": p.mime, "data": data } }));
        }
        content.push(json!({ "type": "text", "text": crate::skills::not_a_command(text) }));
        let out = match l.turn_content(hub, &chat.id, &content, false).await {
            Err(e) if l.exited() && !e.is::<crate::stops::Stopped>() => {
                // The process had died (idle, crashed): start another in its place and try once more.
                tracing::info!(chat = %chat.id, error = %e, "claude session ended; starting another");
                let permit = retire(hub, l);
                l = spawn(hub, &chat.agent, Some(&chat.id), chat.session.as_deref(), permit).await?;
                l.turn_content(hub, &chat.id, &content, false).await
            }
            other => other,
        };
        match out {
            Ok(t) => {
                if l.fresh {
                    hub.core.set_session(&chat.id, &l.session_id)?;
                    l.fresh = false;
                }
                l.last_used = Instant::now();
                if self.generation.load(Ordering::Acquire) == generation {
                    self.live.lock().await.push(l);
                    self.nudge();
                } else {
                    discard(hub, l);
                }
                Ok(t)
            }
            // You stopped it, and the CLI stopped when asked (crate::stops): the process carries on, kept for the
            // chat's next turn, which knows it was stopped.
            Err(e) if e.downcast_ref::<crate::stops::Stopped>().is_some_and(|s| !s.killed) => {
                if l.fresh {
                    hub.core.set_session(&chat.id, &l.session_id)?;
                    l.fresh = false;
                }
                l.last_used = Instant::now();
                if self.generation.load(Ordering::Acquire) == generation {
                    self.live.lock().await.push(l);
                    self.nudge();
                } else {
                    discard(hub, l);
                }
                Err(e)
            }
            Err(e) => {
                discard(hub, l);
                Err(e)
            }
        }
    }

    /// Asks the chat's session for a note to continue from (crate::chat::tidy), without showing
    /// it in the chat. The session's process stops afterwards: the chat moves to a fresh one.
    pub async fn summarize(&self, hub: &Arc<Hub>, chat: &Chat, prompt: &str) -> anyhow::Result<String> {
        let mut l = self.take_quiet(hub, chat, true).await?;
        let out = l.turn(hub, &chat.id, prompt, true).await;
        discard(hub, l);
        Ok(out?.text)
    }

    /// Room for one of `agent`'s turns in `chat`: shows "waiting" while it waits, and "thinking"
    /// again once it has room. `quiet` (r1d's own requests) shows nothing.
    pub async fn room_for(&self, hub: &Hub, agent: &str, chat: &str, quiet: bool) -> OwnedSemaphorePermit {
        let waited = std::sync::atomic::AtomicBool::new(false);
        let permit = self
            .room(hub, || {
                waited.store(true, Ordering::Relaxed);
                if !quiet {
                    hub.emit(Event::agent_state(agent, "waiting", chat), Source::Api);
                }
            })
            .await;
        if waited.load(Ordering::Relaxed) && !quiet {
            hub.emit(Event::agent_state(agent, "thinking", chat), Source::Api);
        }
        permit
    }

    /// Takes the chat's process out of the pool (it's in use), adopting a spare or starting one.
    async fn take(&self, hub: &Arc<Hub>, chat: &Chat) -> anyhow::Result<Live> {
        self.take_quiet(hub, chat, false).await
    }

    async fn take_quiet(&self, hub: &Arc<Hub>, chat: &Chat, quiet: bool) -> anyhow::Result<Live> {
        {
            let mut live = self.live.lock().await;
            if let Some(i) = live.iter().position(|l| l.chat.as_deref() == Some(chat.id.as_str())) {
                return Ok(live.swap_remove(i));
            }
            if chat.session.is_none()
                && chat.summary.is_none()
                && let Some(i) = live.iter().position(|l| l.chat.is_none() && l.agent == chat.agent)
            {
                let mut l = live.swap_remove(i);
                hub.tool_tokens.rebind(&l.token, &chat.id);
                l.chat = Some(chat.id.clone());
                return Ok(l);
            }
        }
        let permit = self.room_for(hub, &chat.agent, &chat.id, quiet).await;
        spawn(hub, &chat.agent, Some(&chat.id), chat.session.as_deref(), permit).await
    }

    /// Stops the idle process serving `chat`, if any (its session was replaced, or it was deleted).
    pub async fn drop_chat(&self, hub: &Hub, chat: &str) {
        let mut live = self.live.lock().await;
        if let Some(i) = live.iter().position(|l| l.chat.as_deref() == Some(chat)) {
            discard(hub, live.swap_remove(i));
        }
    }

    /// Stops `agent`'s processes (its profile changed, or it was deleted), or everyone's.
    pub async fn clear(&self, hub: &Hub, agent: Option<&str>) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let mut live = self.live.lock().await;
        let (stop, keep): (Vec<Live>, Vec<Live>) = live.drain(..).partition(|l| agent.is_none_or(|a| l.agent == a));
        *live = keep;
        for l in stop {
            discard(hub, l);
        }
    }

    /// Stops processes idle for longer than IDLE_ON (the display on) or IDLE_OFF (off), and says how long until the
    /// next one is (none: no process is idle). The reaper sleeps that long, or until [`Sessions::nudged`] (T-035: no
    /// tick with nothing to do).
    pub async fn reap(&self, hub: &Hub) -> Option<Duration> {
        let idle = if hub.services.display_on() { IDLE_ON } else { IDLE_OFF };
        let mut live = self.live.lock().await;
        let (keep, stale): (Vec<Live>, Vec<Live>) = live.drain(..).partition(|l| l.last_used.elapsed() < idle);
        *live = keep;
        for l in stale {
            tracing::info!(agent = %l.agent, chat = ?l.chat, "stopping an idle claude session");
            discard(hub, l);
        }
        live.iter().map(|l| idle.saturating_sub(l.last_used.elapsed())).min().map(|d| d + REAP_SLACK)
    }

    /// The reaper's to look again: a process went back to the pool, or the display went off (its limit is shorter).
    pub fn nudge(&self) {
        self.nudge.notify_one();
    }

    pub async fn nudged(&self) {
        self.nudge.notified().await;
    }
}

fn discard(hub: &Hub, l: Live) {
    drop(retire(hub, l));
}

/// Stops a process and hands back its place in the budget (for a replacement).
fn retire(hub: &Hub, mut l: Live) -> OwnedSemaphorePermit {
    hub.tool_tokens.revoke(&l.token);
    let _ = l.child.start_kill();
    l.permit
}

async fn spawn(hub: &Arc<Hub>, agent_id: &str, chat: Option<&str>, session: Option<&str>, permit: OwnedSemaphorePermit) -> anyhow::Result<Live> {
    let agent = hub.core.agent_or_first(agent_id);
    let jail = Jail::for_agent(&agent.id)?;
    jail.prepare()?;
    jail.check_quota()?;
    jail.refresh_dns().await;
    let summary = match chat {
        Some(c) => hub.core.chat(c)?.and_then(|c| c.summary),
        None => None,
    };
    let token = hub.tool_tokens.issue(&agent.id, chat.unwrap_or(""));
    let name = format!("live-{}", crate::core::random_hex(4));
    let (host_dir, box_dir) = chat::session_dir(&jail, &name).await?;
    let prompt = chat::system_prompt(&agent, &chat::others(hub, &agent.id), &chat::memories(hub, &agent.id), "claude", summary.as_deref(), None, &crate::abilities::block(hub, &agent));
    tokio::fs::write(format!("{host_dir}/system.md"), prompt).await?;
    tokio::fs::write(format!("{host_dir}/mcp.json"), chat::mcp_config(hub, &token).to_string()).await?;
    chat::own(&host_dir).await;
    let files = chat::files_dir(&jail).await?;

    let (flag, session_id, fresh) = match session {
        Some(s) => ("--resume", s.to_owned(), false),
        None => ("--session-id", crate::core::uuid(), true),
    };
    let system = format!("{box_dir}/system.md");
    let mcp = format!("{box_dir}/mcp.json");
    // Skills (crate::skills): the Skill tool, and the ones off for this agent.
    let tools = format!("{CLAUDE_TOOLS},Skill");
    let allowed = format!("{CLAUDE_TOOLS},Skill,mcp__r1");
    let skills_off = crate::skills::claude_settings(hub, &agent.id);
    // The user's connections (ADR-0052): the claude.ai connectors load beside r1d's tools; anything not allowed
    // outright (their tools) asks r1d first (crate::connections::permission), and the ones turned off for this agent
    // aren't there at all.
    let off = crate::connections::disallowed(hub, &agent.id).join(",");
    let mut args = vec![
        "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--include-partial-messages", "--verbose",
        "--model", "sonnet", "--effort", "low",
        "--tools", &tools, "--allowedTools", &allowed,
        "--mcp-config", &mcp,
        // No --disable-slash-commands: it turns skills off too. A message starting with "/" is the user's words
        // (skills::not_a_command).
        "--permission-mode", "manual", "--permission-prompt-tool", "mcp__r1__permission",
        "--system-prompt-file", &system, flag, &session_id,
    ];
    args.extend(["--settings", skills_off.as_str()]);
    if !off.is_empty() {
        args.extend(["--disallowedTools", off.as_str()]);
    }
    let spawned = jail
        // A tool may wait for your approval on the R1 (crate::approvals::WAIT): the CLI waits longer.
        // Connectors' tools load up front (ADR-0052): chats have no ToolSearch to reach deferred ones.
        .command(CLAUDE, &args, &files, &[("NO_COLOR", "1"), ("MCP_TOOL_TIMEOUT", "180000"), ("ENABLE_TOOL_SEARCH", "false")])?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("start the agent");
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            hub.tool_tokens.revoke(&token);
            return Err(e);
        }
    };
    let stdin = child.stdin.take().context("stdin")?;
    let lines = BufReader::new(child.stdout.take().context("stdout")?).lines();
    let stderr = Arc::new(Mutex::new(String::new()));
    let sink = stderr.clone();
    let err = child.stderr.take().context("stderr")?;
    tokio::spawn(async move {
        let mut lines = BufReader::new(err).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let mut s = sink.lock().unwrap();
            if s.len() < 4000 {
                s.push_str(&l);
                s.push('\n');
            }
        }
    });
    Ok(Live {
        agent: agent.id,
        chat: chat.map(str::to_owned),
        session_id,
        fresh,
        token,
        child,
        stdin,
        lines,
        stderr,
        last_used: Instant::now(),
        born: Instant::now(),
        turns: 0,
        permit,
    })
}

/// The context a model call used: everything it read plus what it wrote (Anthropic usage fields).
pub fn context_of(usage: &Value) -> Option<i64> {
    let input = usage["input_tokens"].as_i64()?;
    Some(
        input
            + usage["cache_creation_input_tokens"].as_i64().unwrap_or(0)
            + usage["cache_read_input_tokens"].as_i64().unwrap_or(0)
            + usage["output_tokens"].as_i64().unwrap_or(0),
    )
}

/// The model's context window from a result event's `modelUsage`, if the CLI reports it.
pub fn window_of(result: &Value) -> i64 {
    result["modelUsage"]
        .as_object()
        .and_then(|m| m.values().filter_map(|u| u["contextWindow"].as_i64()).max())
        .unwrap_or(DEFAULT_WINDOW)
}

impl Live {
    fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Sends one user message and streams the reply until its `result` event. `quiet`: nothing
    /// goes out as chat events (r1d's own requests, like a summary).
    async fn turn(&mut self, hub: &Arc<Hub>, chat_id: &str, text: &str, quiet: bool) -> anyhow::Result<Turn> {
        self.turn_content(hub, chat_id, &[json!({ "type": "text", "text": text })], quiet).await
    }

    /// One user message of content blocks (text, images).
    async fn turn_content(&mut self, hub: &Arc<Hub>, chat_id: &str, content: &[Value], quiet: bool) -> anyhow::Result<Turn> {
        let msg = json!({ "type": "user", "message": { "role": "user", "content": content } });
        self.stdin.write_all(format!("{msg}\n").as_bytes()).await.context("the agent isn't running")?;
        self.stdin.flush().await?;
        let mut result = Value::Null;
        let mut speaking = false;
        let mut context = None;
        let mut compacted = false;
        let agent = self.agent.clone();
        // How long the reply takes (crate::perf): warm when the process was running already (the CLI takes ~5 s to
        // start on the R1).
        let sent = Instant::now();
        let warm = self.turns > 0 || self.born.elapsed() >= Duration::from_secs(10);
        let mut first = quiet;
        let mut started = quiet;
        self.turns += 1;
        // You can stop it (crate::stops): the CLI is asked to, and its turn ends with an error result. What it had
        // said by then stays in the chat.
        let stop = hub.stops.get(chat_id);
        let mut stopping: Option<tokio::time::Instant> = None;
        let mut said = String::new();
        let (lines, stdin) = (&mut self.lines, &mut self.stdin);
        let read = async {
            let mut quiet_until = tokio::time::Instant::now() + TURN_QUIET;
            loop {
                let line = tokio::select! {
                    l = lines.next_line() => l?,
                    () = tokio::time::sleep_until(quiet_until) => anyhow::bail!("no answer for {} s", TURN_QUIET.as_secs()),
                    () = crate::stops::fired(&stop), if stopping.is_none() => {
                        stopping = Some(tokio::time::Instant::now() + crate::stops::STOP_WAIT);
                        stdin.write_all(crate::stops::interrupt().as_bytes()).await?;
                        stdin.flush().await?;
                        continue;
                    }
                    // It didn't stop when asked.
                    () = tokio::time::sleep_until(stopping.unwrap_or_else(tokio::time::Instant::now)), if stopping.is_some() => return anyhow::Ok(false),
                };
                let Some(line) = line else { return anyhow::Ok(false) };
                quiet_until = tokio::time::Instant::now() + TURN_QUIET;
                let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
                // The CLI has taken the turn (its init, before the model's call): its own overhead (crate::perf).
                if !started && ev["type"] == "system" && ev["subtype"] == "init" {
                    started = true;
                    hub.perf.mark(if warm { "reply.init.warm" } else { "reply.init.cold" }, sent.elapsed());
                }
                if !first && ev["type"] == "stream_event" && ev["event"]["delta"]["type"] == "text_delta" {
                    first = true;
                    hub.perf.mark(if warm { "reply.first_word.warm" } else { "reply.first_word.cold" }, sent.elapsed());
                    hub.perf.first_word();
                }
                crate::stops::follow(&mut said, &ev);
                match (ev["type"].as_str(), ev["subtype"].as_str()) {
                    // Each model call's usage: the last one is what the session holds now.
                    (Some("assistant"), _) => context = context_of(&ev["message"]["usage"]).or(context),
                    // The CLI compacted the session itself (its own safety net).
                    (Some("system"), Some("compact_boundary")) => compacted = true,
                    // What it reaches on the user's accounts (crate::connections).
                    (Some("system"), Some("init")) => {
                        crate::connections::seen(hub, &ev);
                    }
                    _ => {}
                }
                if quiet {
                    if ev["type"] == "result" {
                        result = ev;
                        return anyhow::Ok(true);
                    }
                } else if chat::stream_event(hub, &agent, chat_id, &ev, &mut speaking) {
                    result = ev;
                    return anyhow::Ok(true);
                }
            }
        };
        let read = tokio::time::timeout(TURN_TIMEOUT, read).await;
        // Stopped, unless it had just finished anyway: killed if it didn't stop when asked.
        if stopping.is_some() && !(matches!(read, Ok(Ok(true))) && result["subtype"] == "success") {
            let killed = !matches!(read, Ok(Ok(true)));
            if killed {
                let _ = self.child.start_kill();
            }
            return Err(anyhow::Error::new(crate::stops::Stopped { said, killed }));
        }
        match read {
            Err(_) => bail!("no answer after {} s", TURN_TIMEOUT.as_secs()),
            Ok(Err(e)) => return Err(e),
            Ok(Ok(false)) => {
                let err = self.stderr.lock().unwrap().clone();
                bail!("the agent stopped: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output"));
            }
            Ok(Ok(true)) => {}
        }
        if result["is_error"].as_bool() == Some(true) {
            bail!("{}", result["result"].as_str().unwrap_or("the agent hit an error"));
        }
        if !quiet {
            hub.perf.mark("reply.done.claude", sent.elapsed());
        }
        let window = window_of(&result);
        let context = context.unwrap_or(0);
        let meta = json!({ "cost_usd": result["total_cost_usd"], "ms": result["duration_ms"], "warm": true, "context": context });
        Ok(Turn { text: result["result"].as_str().unwrap_or_default().to_owned(), meta, context, window, compacted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_and_window() {
        let usage = json!({ "input_tokens": 12, "cache_creation_input_tokens": 3000, "cache_read_input_tokens": 41000, "output_tokens": 220 });
        assert_eq!(context_of(&usage), Some(44_232));
        assert_eq!(context_of(&json!({})), None);
        let result = json!({ "modelUsage": { "claude-sonnet-x": { "contextWindow": 1_000_000 }, "claude-haiku-x": { "contextWindow": 200_000 } } });
        assert_eq!(window_of(&result), 1_000_000);
        assert_eq!(window_of(&json!({ "type": "result" })), DEFAULT_WINDOW);
    }
}
