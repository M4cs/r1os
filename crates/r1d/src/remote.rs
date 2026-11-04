//! Agents that work on a linked computer (docs/specs/companion.md §7.2, ADR-0047, ADR-0049).
//!
//! - **r1d keeps the chat** as for any agent: its transcript, events, system prompt, memories and tools. The
//!   computer's companion runs the turn: the unmodified `claude` CLI, in the agent's folders there (never the Agent
//!   SDK with a subscription; CLAUDE.md).
//! - **One call a turn** over the link (`turn {…}`, crate::link): the companion streams the CLI's own output back a
//!   line at a time, and r1d reads it exactly as it reads the CLI in the box (crate::session): the reply streams to
//!   the R1, tool steps show, and the turn ends at the CLI's `result`.
//! - **r1d's tools** reach the agent through the link, as that agent, with a token for the turn (`mcp`); its questions
//!   (a command to run, a file outside its folders) come to the R1's approval card (`approve`), as its levels say.
//! - **The session** the computer keeps is the chat's while the agent stays there (`pc:<key>:<id>`). A chat that moved
//!   (from the R1, or another computer) starts a fresh session from the note it continues from, or what was said last.
//! - **A computer that isn't there:** the turn waits a minute for it ("waiting"); then the message waits in core.db
//!   ([`Waiting`]: a note in the chat says so) and goes when that computer links again ([`flush`]).
//! - **Brains:** Claude (its `stream-json`, read by crate::chat::stream_event), and Codex, which the companion drives
//!   through its app-server (so it asks before commands and changes) and translates into the `codex exec --json` lines
//!   r1d reads in the box ([`crate::chat::CodexTurn`]).

use crate::chat::{self, Turn};
use crate::core::{Agent, Chat};
use crate::hub::{Event, Hub, Source};
use anyhow::{Context, Result, bail};
use r1_link::Msg;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// The longest a turn on a computer may take: real work there, and waits on the R1's approval card.
const TURN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// How long a turn waits for a computer that isn't reachable (trying again this often) before it's kept for later.
const WAITS: u32 = 2;
const WAIT_EVERY: Duration = Duration::from_secs(30);

/// A turn whose computer isn't there: kept, and sent when it links again.
#[derive(Debug)]
pub struct Waiting(pub String);

impl std::fmt::Display for Waiting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} isn't linked right now: this goes to it as soon as it is", self.0)
    }
}

impl std::error::Error for Waiting {}

/// The link was gone before the computer took the turn.
#[derive(Debug)]
struct Lost;

impl std::fmt::Display for Lost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the link was gone")
    }
}

impl std::error::Error for Lost {}

/// A computer linked: the messages that waited for its agents go now, each in its chat's turn.
pub fn flush(hub: &Arc<Hub>, computer: &str) {
    let on = |agent: &str| hub.core.agent(agent).is_some_and(|a| a.computer() == Some(computer));
    let waiting = match hub.core.take_waiting(on) {
        Ok(w) => w,
        Err(e) => return tracing::warn!(error = %e, "couldn't read the waiting messages"),
    };
    for (chat, said, images) in waiting {
        tracing::info!(chat = %chat, "a message that waited for its computer goes now");
        let h = hub.clone();
        tokio::spawn(async move {
            if let Err(e) = chat::again(h, chat.clone(), said, images).await {
                tracing::info!(chat = %chat, error = %format!("{e:#}"), "a message that waited didn't go");
            }
        });
    }
}
/// How much of a moved chat a fresh session starts from: the last messages, each at most this long.
const RECENT: usize = 12;
const RECENT_CHARS: usize = 500;

/// The levels a computer applies to the agent (crate::approvals AREAS for a computer).
pub const PC_AREAS: &[&str] = &["files", "commands", "outside", "browser"];

/// A computer's name, if this R1 still trusts it.
pub fn computer_name(hub: &Hub, key: &str) -> Option<String> {
    hub.core.computers().into_iter().find(|c| c.id == key).map(|c| c.name)
}

/// Runs one of `agent`'s turns in `chat` on its computer. `quiet`: nothing streams to the R1 (r1d's own requests,
/// like the note a tidy keeps).
pub async fn turn(hub: &Arc<Hub>, chat: &Chat, agent: &Agent, said: &str, quiet: bool, photos: &[crate::media::Item]) -> Result<Turn> {
    let key = agent.computer().context("the agent works on the R1")?;
    let name = computer_name(hub, key).context("the computer it works on isn't linked to this R1 any more: move it back to the R1")?;
    let prefix = format!("pc:{key}:");
    let resume = chat.session.as_deref().and_then(|s| s.strip_prefix(&prefix)).map(str::to_owned);
    let first = match run(hub, chat, agent, &name, said, quiet, photos, resume.clone()).await {
        // The link was gone before the computer took the turn (it quit, and the R1 hadn't noticed): dialled again,
        // or kept for when it's back.
        Err(e) if e.downcast_ref::<Lost>().is_some() => {
            tracing::info!(chat = %chat.id, "the link was gone before the turn started; dialling again");
            run(hub, chat, agent, &name, said, quiet, photos, resume.clone()).await
        }
        other => other,
    };
    match first {
        // The computer no longer has the session (its CLI's history was cleared): once more, fresh.
        Err(e) if resume.is_some() && ["No conversation found", "no rollout found"].iter().any(|s| format!("{e:#}").contains(s)) => {
            tracing::info!(chat = %chat.id, "the computer lost the session; starting a fresh one");
            run(hub, chat, agent, &name, said, quiet, photos, None).await
        }
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    hub: &Arc<Hub>,
    chat: &Chat,
    agent: &Agent,
    name: &str,
    said: &str,
    quiet: bool,
    photos: &[crate::media::Item],
    resume: Option<String>,
) -> Result<Turn> {
    let key = agent.computer().context("the agent works on the R1")?;
    let conn = reach(hub, agent, chat, key, name).await?;
    // A fresh session starts from the note the chat continues from or, if it moved here, what was said lately.
    let summary = match (&resume, &chat.summary) {
        (Some(_), _) => None,
        (None, Some(s)) => Some(s.clone()),
        (None, None) => recent(hub, &chat.id),
    };
    let brain = if chat.brain == "codex" { "codex" } else { "claude" };
    let prompt = chat::system_prompt(agent, &chat::others(hub, &agent.id), &chat::memories(hub, &agent.id), brain, summary.as_deref(), Some(name), &crate::abilities::block(hub, agent));
    let images: Vec<Value> = photos
        .iter()
        .map(|p| Ok(json!({ "media_type": p.mime, "data": crate::media::photo_b64(&hub.cfg.data, p)? })))
        .collect::<Result<_>>()?;
    let levels: serde_json::Map<String, Value> = PC_AREAS.iter().map(|a| ((*a).to_owned(), json!(hub.core.permission(&agent.id, a)))).collect();
    let token = hub.tool_tokens.issue(&agent.id, &chat.id);
    let mode = chat.mode.clone().unwrap_or_else(|| agent.mode.clone());
    // What the prompt says, but not when (its last sentence is the time it was written): the companion keeps a
    // chat's process warm while this stays the same.
    let prompt_key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        prompt.rsplit_once("When this session started").map_or(prompt.as_str(), |(before, _)| before).hash(&mut h);
        format!("{:016x}", h.finish())
    };
    let params = json!({
        "agent": { "id": agent.id, "name": agent.name }, "chat": chat.id, "brain": brain, "session": resume, "mode": mode,
        "prompt_key": prompt_key,
        "prompt": prompt, "text": said, "images": images, "token": token, "folders": agent.folders, "levels": levels,
        "quiet": quiet,
    });

    let (mut speaking, mut context, mut compacted, mut heard) = (false, None, false, false);
    let mut codex = chat::CodexTurn::default();
    // You can stop it (crate::stops): its companion is asked to, and the turn's stream ends as it stops.
    let stop = hub.stops.get(&chat.id);
    let (mut stopped, mut said) = (false, String::new());
    // How long the reply takes (crate::perf), from the call to its first word and its end.
    let (sent, mut first) = (std::time::Instant::now(), quiet);
    hub.link.busy(key, true);
    let streamed = tokio::time::timeout(TURN_TIMEOUT, async {
        let msg = Msg::Call { method: "turn".into(), params };
        let asked = r1_link::ask_stream(&conn, &msg, |m| {
            heard = true;
            let Msg::Event { event } = m else { return };
            hub.link.touch(key);
            let line = &event["line"];
            if brain == "codex" {
                codex.line(hub, &agent.id, &chat.id, line, quiet);
                return;
            }
            crate::stops::follow(&mut said, line);
            if !first && line["type"] == "stream_event" && line["event"]["delta"]["type"] == "text_delta" {
                first = true;
                hub.perf.mark("reply.first_word.pc", sent.elapsed());
                hub.perf.first_word();
            }
            // As crate::session reads the CLI: each model call's usage, the CLI's own compaction, the stream.
            match (line["type"].as_str(), line["subtype"].as_str()) {
                (Some("assistant"), _) => context = crate::session::context_of(&line["message"]["usage"]).or(context),
                (Some("system"), Some("compact_boundary")) => compacted = true,
                _ => {}
            }
            if !quiet {
                chat::stream_event(hub, &agent.id, &chat.id, line, &mut speaking);
            }
        });
        tokio::pin!(asked);
        tokio::select! {
            r = &mut asked => r,
            () = crate::stops::fired(&stop) => {
                stopped = true;
                // A companion too old to know `stop` lets the turn finish there; its words don't come here.
                if let Err(e) = r1_link::ask(&conn, &Msg::Call { method: "stop".into(), params: json!({ "chat": chat.id }) }).await {
                    tracing::info!(computer = %name, error = %format!("{e:#}"), "couldn't ask the computer to stop");
                }
                tokio::time::timeout(crate::stops::STOP_WAIT, &mut asked).await.unwrap_or_else(|_| Err(anyhow::anyhow!("it didn't stop")))
            }
        }
    })
    .await;
    hub.link.busy(key, false);
    hub.tool_tokens.revoke(&token);
    if !quiet && !stopped {
        hub.perf.mark("reply.done.pc", sent.elapsed());
    }
    if stopped {
        let said = if brain == "codex" { std::mem::take(&mut codex.reply) } else { said };
        return Err(anyhow::Error::new(crate::stops::Stopped { said, killed: false }));
    }
    let last = match streamed {
        Err(_) => bail!("no answer from {name} after {} minutes", TURN_TIMEOUT.as_secs() / 60),
        // Nothing came back at all: the computer never had the turn.
        Ok(Err(e)) if !heard => {
            tracing::info!(computer = %name, error = %format!("{e:#}"), "the link was gone");
            hub.link.forget(key);
            return Err(anyhow::Error::new(Lost));
        }
        Ok(Err(e)) => bail!("{name} went away during the turn: {e:#}"),
        Ok(Ok(m)) => m,
    };
    let value = match last {
        Msg::Result { ok: true, value } => value,
        Msg::Result { ok: false, value } => bail!("{}", value.as_str().unwrap_or("the turn failed on the computer")),
        other => bail!("{name} answered {other:?}"),
    };
    if brain == "codex" {
        if let Some(why) = codex.failed.take().or_else(|| value["error"].as_str().map(str::to_owned)) {
            bail!("{why}");
        }
        anyhow::ensure!(!codex.reply.is_empty(), "Codex didn't answer on {name}");
        let thread = codex.thread_id.clone().or_else(|| value["thread_id"].as_str().map(str::to_owned));
        if !quiet && let Some(t) = thread {
            hub.core.set_session(&chat.id, &format!("pc:{key}:{t}"))?;
        }
        let context = codex.context.unwrap_or(0);
        let meta = json!({ "brain": "codex", "usage": codex.usage, "context": context, "on": name });
        return Ok(Turn { text: codex.reply, meta, context, window: codex.window.unwrap_or(200_000), compacted: false });
    }
    let result = &value["result"];
    if result["is_error"].as_bool() == Some(true) {
        bail!("{}", result["result"].as_str().unwrap_or("the agent hit an error"));
    }
    if !quiet && let Some(session) = result["session_id"].as_str() {
        hub.core.set_session(&chat.id, &format!("pc:{key}:{session}"))?;
    }
    let window = crate::session::window_of(result);
    let context = context.unwrap_or(0);
    let meta = json!({ "cost_usd": result["total_cost_usd"], "ms": result["duration_ms"], "context": context, "on": name });
    Ok(Turn { text: result["result"].as_str().unwrap_or_default().to_owned(), meta, context, window, compacted })
}

/// The link to the agent's computer: dialled now if it isn't up, waiting a few minutes ("waiting") if it isn't there.
async fn reach(hub: &Arc<Hub>, agent: &Agent, chat: &Chat, key: &str, name: &str) -> Result<r1_link::Connection> {
    for attempt in 0..=WAITS {
        match hub.link.reach(hub, key).await {
            Ok(conn) => {
                if attempt > 0 {
                    hub.emit(Event::agent_state(&agent.id, "thinking", &chat.id), Source::Api);
                }
                return Ok(conn);
            }
            Err(e) if attempt < WAITS => {
                if attempt == 0 {
                    tracing::info!(computer = %name, error = %format!("{e:#}"), "a turn waits for its computer");
                    hub.emit(Event::agent_state(&agent.id, "waiting", &chat.id), Source::Api);
                }
                tokio::time::sleep(WAIT_EVERY).await;
            }
            Err(e) => {
                tracing::info!(computer = %name, error = %format!("{e:#}"), "a turn waits for its computer to link again");
                return Err(anyhow::Error::new(Waiting(name.to_owned())));
            }
        }
    }
    unreachable!()
}

/// What was said lately in a chat, for a session that starts somewhere new.
pub(crate) fn recent(hub: &Hub, chat: &str) -> Option<String> {
    let messages = hub.core.messages(chat).ok()?;
    let said: Vec<String> = messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .rev()
        .skip(1) // the message this turn answers is the newest
        .take(RECENT)
        .map(|m| {
            let who = if m.role == "user" { "The user" } else { "You" };
            let text: String = m.text.chars().take(RECENT_CHARS).collect();
            format!("{who}: {text}")
        })
        .collect();
    if said.is_empty() {
        return None;
    }
    Some(format!(
        "(This chat started somewhere else, so this session is new. What was said last, oldest first:)\n{}",
        said.into_iter().rev().collect::<Vec<_>>().join("\n")
    ))
}
