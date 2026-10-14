//! Stopping a running turn (T-013): you stop a chat's agent mid-turn, on the R1 or in the companion.
//!
//! - Each running turn has a signal ([`Stops::begin`], in crate::chat's `respond`); `POST /api/chats/{id}/stop`
//!   fires it.
//! - A Claude turn on the R1 sends its CLI an interrupt (stream-json's `control_request`), so the warm process stays
//!   and its session keeps that you stopped it; a Codex turn's process is stopped. A turn on a computer asks its
//!   companion to stop (`stop` over the link), which interrupts its CLI the same way (crate::remote).
//! - What the agent was waiting on goes too: its approval cards (crate::approvals) and its ask in the R1's browser.
//! - The chat keeps what the agent had said by then, then a note: `chat {kind: "stopped", text}`.
//! - Its next turn starts with a line saying you stopped it ([`Stops::take_note`]): a Codex process that was killed
//!   doesn't know, and Claude once took a stopped story as finished (2026-10-11).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// How long a stopped turn has to end before its process is killed (or, on a computer, left to finish there).
pub const STOP_WAIT: Duration = Duration::from_secs(10);

/// The chats with a turn running, each with its stop signal; and those whose last turn you stopped.
#[derive(Default)]
pub struct Stops {
    running: Mutex<HashMap<String, Arc<Notify>>>,
    /// Chat → the words its stopped answer ended on, for the next turn ([`Stops::take_note`]).
    stopped: Mutex<HashMap<String, String>>,
}

impl Stops {
    /// A turn starts in `chat`: its signal, until the guard goes.
    pub fn begin(&self, chat: &str) -> Running<'_> {
        self.running.lock().unwrap().insert(chat.to_owned(), Arc::new(Notify::new()));
        Running { stops: self, chat: chat.to_owned() }
    }

    /// The running turn's signal in `chat`, if there is one (a tidy has none).
    pub fn get(&self, chat: &str) -> Option<Arc<Notify>> {
        self.running.lock().unwrap().get(chat).cloned()
    }

    /// You stopped `chat`'s turn after it said `said`: its next turn hears so.
    pub fn note(&self, chat: &str, said: &str) {
        self.stopped.lock().unwrap().insert(chat.to_owned(), said.trim().to_owned());
    }

    /// The line the turn after a stop starts with, once.
    pub fn take_note(&self, chat: &str) -> Option<String> {
        let said = self.stopped.lock().unwrap().remove(chat)?;
        let n = said.chars().count();
        Some(if said.is_empty() {
            "[r1os: the user stopped your last turn before you answered. Don't carry on with it unless they ask.]".to_owned()
        } else {
            let tail: String = said.chars().skip(n.saturating_sub(120)).collect();
            let tail = if n > 120 { format!("…{}", tail.trim_start()) } else { tail };
            format!("[r1os: the user stopped your last answer partway; it ended at \"{tail}\". Don't carry on with it unless they ask.]")
        })
    }

    /// Stops `chat`'s turn: whether one was running. The signal keeps until the turn looks (Notify's permit).
    pub fn stop(&self, chat: &str) -> bool {
        match self.get(chat) {
            Some(n) => {
                n.notify_one();
                true
            }
            None => false,
        }
    }
}

/// A running turn's place in [`Stops`]: gone with it.
pub struct Running<'a> {
    stops: &'a Stops,
    chat: String,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.stops.running.lock().unwrap().remove(&self.chat);
    }
}

/// Waits for the signal; forever without one.
pub async fn fired(stop: &Option<Arc<Notify>>) {
    match stop {
        Some(n) => n.notified().await,
        None => std::future::pending().await,
    }
}

/// A turn ended because you stopped it: what the agent had said by then (since its last tool), and whether its
/// process had to be killed (it didn't stop when asked: it isn't kept warm).
#[derive(Debug)]
pub struct Stopped {
    pub said: String,
    pub killed: bool,
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("you stopped it")
    }
}

impl std::error::Error for Stopped {}

/// The words a Claude stream line adds to what the agent is saying now: text as it streams; a tool call starts over
/// (as the chat shows it).
pub fn follow(said: &mut String, ev: &serde_json::Value) {
    match ev["type"].as_str() {
        Some("stream_event") if ev["event"]["delta"]["type"] == "text_delta" => said.push_str(ev["event"]["delta"]["text"].as_str().unwrap_or_default()),
        Some("assistant") if ev["message"]["content"].as_array().is_some_and(|c| c.iter().any(|b| b["type"] == "tool_use")) => said.clear(),
        _ => {}
    }
}

/// The CLI's interrupt (what the Agent SDK's `interrupt()` sends): the turn ends with an `error_during_execution`
/// result, and the process carries on (checked on Claude Code 2.1.280, 2026-10-11).
pub fn interrupt() -> String {
    let msg = serde_json::json!({ "type": "control_request", "request_id": format!("stop-{}", crate::core::random_hex(4)), "request": { "subtype": "interrupt" } });
    format!("{msg}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a_stop_waits_for_its_turn() {
        let stops = Stops::default();
        assert!(!stops.stop("c1"), "nothing runs");
        let running = stops.begin("c1");
        assert!(stops.stop("c1"));
        // Stopped before the turn looked: it still sees it.
        let signal = stops.get("c1");
        tokio::time::timeout(std::time::Duration::from_millis(100), fired(&signal)).await.expect("the stop keeps");
        drop(running);
        assert!(stops.get("c1").is_none());
        assert!(!stops.stop("c1"));
    }

    #[test]
    fn the_next_turn_hears_once() {
        let stops = Stops::default();
        assert_eq!(stops.take_note("c1"), None);
        stops.note("c1", "Once upon a time");
        assert_eq!(stops.take_note("c1").unwrap(), "[r1os: the user stopped your last answer partway; it ended at \"Once upon a time\". Don't carry on with it unless they ask.]");
        assert_eq!(stops.take_note("c1"), None);
        stops.note("c1", &"word ".repeat(60));
        assert!(stops.take_note("c1").unwrap().contains("ended at \"…word"));
    }

    #[test]
    fn what_it_was_saying() {
        let mut said = String::new();
        follow(&mut said, &json!({ "type": "stream_event", "event": { "delta": { "type": "text_delta", "text": "Let me look" } } }));
        follow(&mut said, &json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "name": "x" }] } }));
        assert_eq!(said, "");
        follow(&mut said, &json!({ "type": "stream_event", "event": { "delta": { "type": "text_delta", "text": "The first" } } }));
        follow(&mut said, &json!({ "type": "stream_event", "event": { "delta": { "type": "text_delta", "text": " result is" } } }));
        assert_eq!(said, "The first result is");
    }
}
