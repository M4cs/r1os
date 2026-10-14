//! Connections (ADR-0052, R8): what the agents' CLIs reach on the user's accounts, through the CLIs and their
//! providers. r1os holds no tokens. A Claude agent has the claude.ai connectors the user added (Gmail, Google Calendar,
//! Drive…, their tokens at Anthropic); a Codex agent has the ChatGPT apps (theirs at OpenAI, Q50).
//!
//! - **What's there:** the servers each chat's CLI reported when it started (its stream-json `init`: names, status,
//!   tools), and `claude mcp list` when asked to look again ([`check`]). Kept in memory; nothing about them is stored.
//! - **On for every agent** unless turned off for one (`connection_rules`: `server:<name>` → `off`). A Claude chat gets
//!   the ones turned off as `--disallowedTools`.
//! - **Reads run; writes ask** (ADR-0045's card): a Claude chat's permission prompts come to [`permission`] (r1d's
//!   `permission` MCP tool, `--permission-prompt-tool`). A tool that reads (by its name) goes ahead; one that acts asks
//!   on the R1, unless you said "always" for that agent and tool (`tool:<name>` → `allow`). One it can't tell asks.
//!   A Codex chat's MCP tools (the ChatGPT apps, `codex_apps`) come to the same rules through a PreToolUse hook
//!   ([`codex_hook`], ADR-0058): Codex runs with its own approvals bypassed, and ChatGPT doesn't ask either (T-083).
//! - **Signing in:** a connector's link from the CLI (`claude mcp login "<name>" --no-browser`), which the R1 shows as
//!   a QR code and the companion opens on the PC. The link holds no secret; the sign-in happens on claude.ai.
//! - **Your own servers:** one that signs in with OAuth is added on claude.ai as a custom connector (its tokens stay at
//!   Anthropic, and it signs in like the others). The CLI's own sign-in can't finish without a terminal (2.1.295: "stdin
//!   isn't a terminal", and no callback listener). One that takes an API key is added from the companion ([`add`]):
//!   `claude mcp add --scope user` with the key as a header, which the CLI keeps in the box like its other credentials.

use crate::hub::{Event, Hub, Source};
use crate::jail::CLAUDE;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Where connectors are added and signed in to, for a QR code or the PC's browser.
pub const CLAUDE_ADD_URL: &str = "https://claude.ai/customize/connectors";
pub const CHATGPT_APPS_URL: &str = "https://chatgpt.com/apps";
/// r1d's own MCP server in the agents' CLIs: not a connection.
const OWN: &str = "r1";
/// Codex's server for the ChatGPT apps (`mcp__codex_apps__<app>__<tool>`: `pets__select_pet`).
const CODEX_APPS: &str = "codex_apps";

/// One server a CLI reaches.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Server {
    /// As the CLI names it: "claude.ai Gmail".
    pub name: String,
    /// `connected`, `needs-auth`, `failed`, or `pending`.
    pub status: String,
    /// `claudeai` (a claude.ai connector), or where the CLI's config has it.
    pub source: String,
    /// Its tools' own names ("search", "send_email"), from a chat's start; none until one has seen it.
    pub tools: Vec<String>,
}

#[derive(Default)]
pub struct Connections {
    servers: Mutex<Vec<Server>>,
    /// What core.db kept has been read in (after a restart, before any chat has started).
    loaded: std::sync::atomic::AtomicBool,
}

/// The servers known, read from core.db the first time (what chats saw before r1d last started).
fn known(hub: &Hub) -> std::sync::MutexGuard<'_, Vec<Server>> {
    let c = &hub.connections;
    if !c.loaded.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let kept = hub.core.connections_seen();
        let mut s = c.servers.lock().unwrap();
        if s.is_empty() {
            *s = kept;
        }
        return s;
    }
    c.servers.lock().unwrap()
}

/// Keeps what's known for the next start.
fn save(hub: &Hub, servers: &[Server]) {
    if let Err(e) = hub.core.set_connections_seen(servers) {
        tracing::warn!(error = %format!("{e:#}"), "connections: couldn't keep what's known");
    }
}

pub fn migrate(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS connection_rules (
            agent TEXT NOT NULL,
            key   TEXT NOT NULL,
            level TEXT NOT NULL,
            PRIMARY KEY (agent, key)
        );",
    )?;
    Ok(())
}

/// The name the CLI gives a server inside tool names: "claude.ai Google Drive" → "claude_ai_Google_Drive".
pub fn slug(server: &str) -> String {
    server.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

/// `mcp__<server>__<tool>` → (server's slug, tool).
pub fn split(tool: &str) -> Option<(&str, &str)> {
    tool.strip_prefix("mcp__")?.split_once("__")
}

/// Words that start a tool that only looks: search, read, list…
const READS: &[&str] = &[
    "search", "list", "get", "read", "fetch", "find", "query", "view", "export", "guide", "lookup", "look", "describe",
    "show", "check", "count", "download", "browse", "retrieve", "preview", "inspect", "whoami", "status", "info", "open",
];
/// Words that start a tool that acts, even after a service's name ("gmail_send_message").
const WRITES: &[&str] = &[
    "send", "create", "update", "delete", "remove", "move", "share", "trash", "upload", "post", "reply", "forward",
    "modify", "edit", "add", "set", "archive", "label", "mark", "rsvp", "cancel", "write", "insert", "append", "batch",
    "put", "patch", "copy", "rename", "invite", "publish", "draft", "schedule", "book", "buy", "pay", "submit", "run",
];

/// Whether a tool only reads, by its name (its words: "search_threads", "getEvents"). One it can't tell acts.
pub fn reads(tool: &str) -> bool {
    reads_within(tool, 2)
}

/// [`reads`], with the reading word among the first `n` (after a service's name: "gmail_search_messages").
fn reads_within(tool: &str, n: usize) -> bool {
    let mut words = Vec::new();
    let mut w = String::new();
    for c in tool.chars() {
        if (c == '_' || c == '-' || c == '.' || c.is_ascii_uppercase()) && !w.is_empty() {
            words.push(std::mem::take(&mut w));
        }
        if c.is_ascii_alphanumeric() {
            w.push(c.to_ascii_lowercase());
        }
    }
    if !w.is_empty() {
        words.push(w);
    }
    if words.iter().any(|w| WRITES.contains(&w.as_str())) {
        return false;
    }
    words.iter().take(n).any(|w| READS.contains(&w.as_str()))
}

/// "claude.ai Google Drive" → "Google Drive", for people.
pub fn label(server: &str) -> &str {
    server.strip_prefix("claude.ai ").unwrap_or(server)
}

/// What's known now.
pub fn servers(hub: &Hub) -> Vec<Server> {
    known(hub).clone()
}

/// The server a tool belongs to, by its slug.
pub fn server_of(hub: &Hub, tool: &str) -> Option<Server> {
    let (s, _) = split(tool)?;
    known(hub).iter().find(|x| slug(&x.name) == s).cloned()
}

/// A Claude chat started (its stream-json `init`): which servers it has, their status and tools. True when that
/// changed what's known.
pub fn seen(hub: &Hub, init: &Value) -> bool {
    let Some(servers) = init["mcp_servers"].as_array() else { return false };
    let tools: Vec<&str> = init["tools"].as_array().map(|t| t.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut now: Vec<Server> = Vec::new();
    for s in servers {
        let name = s["name"].as_str().unwrap_or_default();
        if name.is_empty() || name == OWN {
            continue;
        }
        let prefix = slug(name);
        now.push(Server {
            name: name.to_owned(),
            status: s["status"].as_str().unwrap_or("pending").to_owned(),
            source: s["source"].as_str().unwrap_or_default().to_owned(),
            tools: tools.iter().filter_map(|t| split(t).filter(|(s, _)| *s == prefix).map(|(_, t)| t.to_owned())).collect(),
        });
    }
    let changed = {
        let mut known = known(hub);
        let before = known.clone();
        for s in now {
            match known.iter_mut().find(|k| k.name == s.name) {
                // A server that's still connecting has no tools yet: keep what an earlier chat saw.
                Some(k) => {
                    let tools = if s.tools.is_empty() && s.status != "connected" { std::mem::take(&mut k.tools) } else { s.tools };
                    *k = Server { tools, ..s };
                }
                None => known.push(s),
            }
        }
        let changed = *known != before;
        if changed {
            save(hub, &known);
        }
        changed
    };
    if changed {
        hub.emit(Event::ConnectionsChanged, Source::Api);
    }
    changed
}

/// Looks again with `claude mcp list` (no model call): new connectors show up, gone ones go. Its health check can say
/// "connected" when a chat's would say it needs signing in, so a chat's word wins for a server it has seen.
pub async fn check(hub: &Arc<Hub>) -> anyhow::Result<()> {
    let out = hub.accounts.cli(CLAUDE, &["mcp", "list"], std::time::Duration::from_secs(60)).await?;
    let listed = parse_list(&out);
    {
        let mut known = known(hub);
        known.retain(|k| listed.iter().any(|l| l.name == k.name));
        for l in listed {
            match known.iter_mut().find(|k| k.name == l.name) {
                Some(k) if !k.tools.is_empty() || k.status == "needs-auth" => {
                    if l.status != "connected" {
                        k.status = l.status;
                    }
                }
                Some(k) => *k = l,
                None => known.push(l),
            }
        }
        save(hub, &known);
    }
    hub.emit(Event::ConnectionsChanged, Source::Api);
    Ok(())
}

/// `claude mcp list`'s lines: "claude.ai Gmail: https://… - ✔ Connected", "x: cmd args - ! Needs authentication".
fn parse_list(out: &str) -> Vec<Server> {
    out.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(": ")?;
            let (_, state) = rest.rsplit_once(" - ")?;
            let name = name.trim();
            if name.is_empty() || name == OWN {
                return None;
            }
            let s = state.to_lowercase();
            let status = if s.contains("connected") && !s.contains("not connected") {
                "connected"
            } else if s.contains("auth") {
                "needs-auth"
            } else {
                "failed"
            };
            Some(Server {
                name: name.to_owned(),
                status: status.into(),
                source: if name.starts_with("claude.ai ") { "claudeai" } else { "user" }.into(),
                tools: Vec::new(),
            })
        })
        .collect()
}

/// A connector's sign-in link (`claude mcp login "<name>" --no-browser`): a claude.ai page, no secret in it.
pub async fn login_link(hub: &Arc<Hub>, server: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        server.starts_with("claude.ai "),
        "{server} was added with a key, so there's nothing to sign in to; a server that signs in with OAuth is added on claude.ai as a custom connector"
    );
    let out = hub.accounts.cli(CLAUDE, &["mcp", "login", server, "--no-browser"], std::time::Duration::from_secs(30)).await?;
    out.split_whitespace()
        .find(|w| w.starts_with("https://claude.ai/"))
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("the CLI gave no link: {}", out.lines().next().unwrap_or("").trim()))
}

/// A name for your own server: letters, digits, `-` and `_`, and not a claude.ai connector's.
fn valid_name(name: &str) -> bool {
    (1..=40).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && name != OWN
}

/// Adds your own server by URL (from the companion): `claude mcp add --transport http --scope user`, with its API key
/// as a header (`Authorization: Bearer <key>`, or `<header>: <key>`). The CLI keeps it; r1d never logs it.
pub async fn add(hub: &Arc<Hub>, name: &str, url: &str, key: Option<&str>, header: Option<&str>) -> anyhow::Result<()> {
    anyhow::ensure!(valid_name(name), "a name is letters, digits, - and _ (40 at most)");
    anyhow::ensure!(url.starts_with("https://") || url.starts_with("http://"), "the URL starts with https://");
    anyhow::ensure!(!url.chars().any(char::is_whitespace), "the URL has a space in it");
    anyhow::ensure!(!servers(hub).iter().any(|s| s.name == name), "there's already one called {name}");
    let line = key.filter(|k| !k.trim().is_empty()).map(|k| match header.map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => format!("{h}: {}", k.trim()),
        None => format!("Authorization: Bearer {}", k.trim()),
    });
    anyhow::ensure!(line.as_ref().is_none_or(|l| !l.contains(['\r', '\n'])), "the key has a line break in it");
    let mut args = vec!["mcp", "add", "--transport", "http", "--scope", "user", name, url];
    if let Some(l) = line.as_deref() {
        args.extend(["--header", l]);
    }
    let out = hub.accounts.cli(CLAUDE, &args, std::time::Duration::from_secs(30)).await?;
    anyhow::ensure!(out.contains("Added"), "the CLI didn't add it: {}", out.lines().next().unwrap_or("").trim());
    tracing::info!(server = name, "connections: added a server");
    check(hub).await
}

/// Removes one of your own servers (not a claude.ai connector: those are removed on claude.ai).
pub async fn remove(hub: &Arc<Hub>, name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(valid_name(name), "claude.ai connectors are removed on claude.ai");
    let out = hub.accounts.cli(CLAUDE, &["mcp", "remove", "--scope", "user", name], std::time::Duration::from_secs(30)).await?;
    anyhow::ensure!(out.contains("Removed"), "the CLI didn't remove it: {}", out.lines().next().unwrap_or("").trim());
    hub.core.with(|db| {
        db.execute("DELETE FROM connection_rules WHERE key = ?1 OR key LIKE ?2", params![format!("server:{name}"), format!("tool:mcp__{}__%", slug(name))])?;
        anyhow::Ok(())
    })?;
    check(hub).await
}

// ---- per agent ----------------------------------------------------------------------------------------------------

fn rules(db: &Connection, agent: &str, prefix: &str, level: &str) -> anyhow::Result<Vec<String>> {
    let mut st = db.prepare("SELECT key FROM connection_rules WHERE agent = ?1 AND level = ?2 AND key LIKE ?3 ORDER BY key")?;
    let keys = st.query_map(params![agent, level, format!("{prefix}%")], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(keys.into_iter().map(|k| k[prefix.len()..].to_owned()).collect())
}

/// The servers turned off for an agent.
pub fn off(db: &Connection, agent: &str) -> anyhow::Result<Vec<String>> {
    rules(db, agent, "server:", "off")
}

/// The tools an agent may use without asking ("always").
pub fn always(db: &Connection, agent: &str) -> anyhow::Result<Vec<String>> {
    rules(db, agent, "tool:", "allow")
}

pub fn set_on(db: &Connection, agent: &str, server: &str, on: bool) -> anyhow::Result<()> {
    let key = format!("server:{server}");
    if on {
        db.execute("DELETE FROM connection_rules WHERE agent = ?1 AND key = ?2", params![agent, key])?;
    } else {
        db.execute("INSERT OR REPLACE INTO connection_rules (agent, key, level) VALUES (?1, ?2, 'off')", params![agent, key])?;
    }
    Ok(())
}

pub fn set_always(db: &Connection, agent: &str, tool: &str, allow: bool) -> anyhow::Result<()> {
    let key = format!("tool:{tool}");
    if allow {
        db.execute("INSERT OR REPLACE INTO connection_rules (agent, key, level) VALUES (?1, ?2, 'allow')", params![agent, key])?;
    } else {
        db.execute("DELETE FROM connection_rules WHERE agent = ?1 AND key = ?2", params![agent, key])?;
    }
    Ok(())
}

/// A Claude chat's `--disallowedTools` for this agent: the servers turned off for it.
pub fn disallowed(hub: &Hub, agent: &str) -> Vec<String> {
    hub.core.with(|db| off(db, agent)).unwrap_or_default().iter().map(|s| format!("mcp__{}", slug(s))).collect()
}

/// Everything for the R1's Settings › Connections and the companion: the servers, and each agent's.
pub fn state(hub: &Hub) -> anyhow::Result<Value> {
    let agents: Vec<Value> = hub
        .core
        .agents()?
        .into_iter()
        .map(|a| {
            let (off, always) = hub.core.with(|db| anyhow::Ok((off(db, &a.id)?, always(db, &a.id)?))).unwrap_or_default();
            json!({ "id": a.id, "name": a.name, "brain": a.brain, "off": off, "always": always })
        })
        .collect();
    let servers: Vec<Value> = servers(hub)
        .into_iter()
        .map(|s| {
            let reading: Vec<&String> = s.tools.iter().filter(|t| reads(t)).collect();
            let acting: Vec<&String> = s.tools.iter().filter(|t| !reads(t)).collect();
            json!({
                "name": s.name, "label": label(&s.name), "status": s.status, "source": s.source, "brain": "claude",
                "slug": slug(&s.name), "reads": reading, "acts": acting,
            })
        })
        .collect();
    Ok(json!({ "servers": servers, "agents": agents, "add_url": CLAUDE_ADD_URL, "chatgpt_url": CHATGPT_APPS_URL }))
}

// ---- permission prompts -------------------------------------------------------------------------------------------

/// A tool's name and what it's given, for the card: "send email" · "to: sam@… · subject: running late".
fn card(short: &str, input: &Value) -> (String, String) {
    let what = short.split(['_', '-']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ").to_lowercase();
    (what, summary(input))
}

/// "plugin management" → "Plugin management".
fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Keys that say what an action is about, first: who it goes to, what it's called, when, then its words.
const KEYS: &[&str] = &[
    "to", "recipient", "recipients", "cc", "bcc", "attendees", "subject", "title", "name", "summary", "start", "end", "when",
    "date", "query", "path", "folder", "text", "body", "message", "content", "markdown", "description",
];

/// What a tool is given, in a line or two: its strings by their keys, the telling ones first ("to: sam@x · subject:
/// running late · body: I'll be ten minutes…"), each cut short. No JSON.
fn summary(input: &Value) -> String {
    fn walk<'a>(key: &'a str, v: &'a Value, out: &mut Vec<(&'a str, String)>) {
        match v {
            Value::String(s) if !s.trim().is_empty() => out.push((key, s.split_whitespace().collect::<Vec<_>>().join(" "))),
            Value::Array(a) if a.iter().all(Value::is_string) && !a.is_empty() => {
                out.push((key, a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(key, x, out)),
            Value::Object(m) => m.iter().for_each(|(k, x)| walk(k, x, out)),
            _ => {}
        }
    }
    let mut parts = Vec::new();
    walk("", input, &mut parts);
    parts.sort_by_key(|(k, _)| KEYS.iter().position(|x| x.eq_ignore_ascii_case(k)).unwrap_or(KEYS.len()));
    parts
        .into_iter()
        .take(4)
        .map(|(k, v)| {
            let v = if v.chars().count() > 70 { v.chars().take(70).collect::<String>() + "…" } else { v };
            if k.is_empty() { v } else { format!("{}: {v}", k.replace('_', " ")) }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// What the CLI asks before a tool runs that isn't allowed outright (`--permission-prompt-tool`): `{behavior: allow,
/// updatedInput}` or `{behavior: deny, message}`, as the tool's text.
pub async fn permission(hub: &Arc<Hub>, agent: &str, chat: &str, args: &Value) -> Value {
    let tool = args["tool_name"].as_str().unwrap_or_default();
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    let allow = || json!({ "behavior": "allow", "updatedInput": input.clone() });
    let deny = |m: String| json!({ "behavior": "deny", "message": m });
    let Some((slug_, short)) = split(tool) else {
        return deny(format!("`{tool}` isn't one of your tools here."));
    };
    if slug_ == OWN {
        return allow();
    }
    // A ChatGPT app's tool is the app's name, then its own ("pets__select_pet"): "ChatGPT Pets", "select pet".
    let (app, short) = match short.split_once("__") {
        Some((app, t)) if slug_ == CODEX_APPS => (Some(app), t),
        _ => (None, short),
    };
    let server = match app {
        Some(a) => format!("ChatGPT {}", capitalized(&a.replace('_', " "))),
        None if slug_ == CODEX_APPS => "ChatGPT".to_owned(),
        None => server_of(hub, tool).map(|s| s.name).unwrap_or_else(|| slug_.replace('_', " ")),
    };
    let name = label(&server).to_owned();
    if hub.core.with(|db| off(db, agent)).unwrap_or_default().iter().any(|s| slug(s) == slug_) {
        return deny(format!("{name} is turned off for you (the user can turn it on in Settings › Connections on the R1)."));
    }
    // One whose app couldn't be told apart may start with the app's name, of up to three words.
    if reads_within(short, if slug_ == CODEX_APPS && app.is_none() { 4 } else { 2 }) {
        return allow();
    }
    // Another agent's message started this turn: nothing that acts on the user's accounts (ADR-0012).
    if hub.mail.woken(chat).is_some() {
        return deny("Another agent's message started this turn: nothing that acts on the user's accounts. Tell the user instead.".into());
    }
    if hub.core.with(|db| always(db, agent)).unwrap_or_default().iter().any(|t| t == tool) {
        return allow();
    }
    let (what, detail) = card(short, &input);
    let ask = crate::approvals::Ask::new("connections", format!("use {name}: {what}"), detail)
        .rule(tool.to_owned(), format!("{name}: {what}"))
        .chat(chat);
    match hub.approvals.gate_rule(hub, agent, ask).await {
        Ok(()) => allow(),
        Err(why) => deny(why),
    }
}

/// How long the hook waits on r1d, beyond the card's own wait: past it, it denies.
const HOOK_WAIT_S: u64 = crate::approvals::WAIT.as_secs() + 30;

/// A Codex turn's `-c` for its PreToolUse hook (ADR-0058): every MCP tool but r1's own goes to [`permission`]'s rules
/// before it runs. The hook is `curl` in the box, posting what Codex hands it to r1d's `/hooks/pre-tool-use` with the
/// turn's tool token; r1d answers nothing (run it) or a deny with its reason, which Codex gives the model. Codex runs a
/// tool whose hook fails, so the hook denies (exit 2) whenever r1d's answer doesn't come.
pub fn codex_hook(port: u16) -> String {
    let command = format!(
        "curl -sf -m {HOOK_WAIT_S} -H \"Authorization: Bearer $R1_TOOL_TOKEN\" -H Content-Type:application/json --data-binary @- \
         http://127.0.0.1:{port}/hooks/pre-tool-use || {{ echo \"r1os couldn't ask the user, so this didn't run.\" >&2; exit 2; }}"
    );
    // r1's own tools (`mcp__r1__…`) don't match: Codex's matcher has no look-ahead, so "not `r1_`" is spelt out.
    format!(
        "hooks.PreToolUse=[{{matcher=\"^mcp__(?:[^r]|r[^1]|r1[^_])\", hooks=[{{type=\"command\", command={}, timeout={}}}]}}]",
        serde_json::to_string(&command).unwrap_or_default(),
        HOOK_WAIT_S + 15
    )
}

/// What r1d tells Codex's hook, from [`permission`]'s verdict: nothing to let the tool run, or a deny with its reason.
pub fn hook_answer(verdict: &Value) -> Option<Value> {
    (verdict["behavior"] != "allow").then(|| {
        json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": verdict["message"].as_str().unwrap_or("Not allowed on the R1."),
        } })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_asks_r1d_first() {
        let c = codex_hook(7171);
        assert!(c.starts_with("hooks.PreToolUse=[{matcher=\"^mcp__(?:[^r]|r[^1]|r1[^_])\""), "{c}");
        assert!(c.contains("http://127.0.0.1:7171/hooks/pre-tool-use") && c.contains("exit 2") && c.contains("$R1_TOOL_TOKEN"));
        assert!(c.ends_with(&format!("timeout={}}}]}}]", HOOK_WAIT_S + 15)), "{c}");
        assert_eq!(hook_answer(&json!({ "behavior": "allow", "updatedInput": {} })), None);
        let no = hook_answer(&json!({ "behavior": "deny", "message": "The user said no." })).unwrap();
        assert_eq!(no["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(no["hookSpecificOutput"]["permissionDecisionReason"], "The user said no.");
    }

    #[test]
    fn a_chatgpt_apps_tool_reads_after_its_name() {
        // As the hook gets them (Codex 0.162): the app's name, then its tool.
        assert_eq!(split("mcp__codex_apps__pets__select_pet"), Some(("codex_apps", "pets__select_pet")));
        assert!(reads("list_pets") && !reads("select_pet"));
        assert_eq!(card("select_pet", &json!({ "pet_id": "rocky" })), ("select pet".to_owned(), "pet id: rocky".to_owned()));
        assert_eq!(capitalized("plugin management"), "Plugin management");
        for t in ["plugin_management_search_plugins", "pets_list_pets", "codex_document_control_list_sessions", "local_hotline_lookup"] {
            assert!(reads_within(t, 4), "{t} reads");
        }
        for t in ["pets_select_pet", "sites_deploy_site", "safety_settings_update_parental_control", "pages_share_page"] {
            assert!(!reads_within(t, 4), "{t} acts");
        }
    }

    #[test]
    fn reading_or_acting_by_name() {
        for t in ["search_threads", "read", "getEvents", "list_files", "query", "export", "guide", "gmail_search_messages", "fetch_document"] {
            assert!(reads(t), "{t} reads");
        }
        for t in ["send_email", "create", "delete", "update", "batch", "gmail_send_message", "createEvent", "share_file", "draft_reply", "frobnicate"] {
            assert!(!reads(t), "{t} acts");
        }
    }

    #[test]
    fn what_a_tool_is_given() {
        let i = json!({ "body": "I'll be ten
minutes late", "to": ["sam@example.com"], "subject": "running late", "thread": 7 });
        assert_eq!(summary(&i), "to: sam@example.com · subject: running late · body: I'll be ten minutes late");
        let b = json!({ "container": "project", "create": { "name": "r1os test", "doc": { "markdown": "# r1os test" } } });
        assert_eq!(summary(&b), "name: r1os test · markdown: # r1os test · container: project");
    }

    #[test]
    fn your_own_servers_names() {
        assert!(valid_name("home-assistant") && valid_name("my_tools2"));
        assert!(!valid_name("claude.ai Gmail") && !valid_name("") && !valid_name("r1") && !valid_name("a b"));
    }

    #[test]
    fn names_and_slugs() {
        assert_eq!(slug("claude.ai Google Drive"), "claude_ai_Google_Drive");
        assert_eq!(split("mcp__claude_ai_Gmail__send_email"), Some(("claude_ai_Gmail", "send_email")));
        assert_eq!(split("Bash"), None);
        assert_eq!(label("claude.ai Gmail"), "Gmail");
    }

    #[test]
    fn the_clis_list() {
        let out = "Checking MCP server health…\n\nclaude.ai Claude Docs: https://api.anthropic.com/v1/pages/mcp - ✔ Connected\nclaude.ai Gmail: https://gmail.mcp.claude.com/mcp - ! Needs authentication\nlocal: npx thing --flag - ✗ Failed to connect\n";
        let l = parse_list(out);
        assert_eq!(l.len(), 3);
        assert_eq!((l[0].name.as_str(), l[0].status.as_str(), l[0].source.as_str()), ("claude.ai Claude Docs", "connected", "claudeai"));
        assert_eq!(l[1].status, "needs-auth");
        assert_eq!((l[2].status.as_str(), l[2].source.as_str()), ("failed", "user"));
    }

    #[test]
    fn rules_per_agent() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        set_on(&db, "a", "claude.ai Gmail", false).unwrap();
        set_always(&db, "a", "mcp__claude_ai_Gmail__send_email", true).unwrap();
        assert_eq!(off(&db, "a").unwrap(), vec!["claude.ai Gmail"]);
        assert_eq!(always(&db, "a").unwrap(), vec!["mcp__claude_ai_Gmail__send_email"]);
        assert!(off(&db, "b").unwrap().is_empty());
        set_on(&db, "a", "claude.ai Gmail", true).unwrap();
        set_always(&db, "a", "mcp__claude_ai_Gmail__send_email", false).unwrap();
        assert!(off(&db, "a").unwrap().is_empty() && always(&db, "a").unwrap().is_empty());
    }
}
