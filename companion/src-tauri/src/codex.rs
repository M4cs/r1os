//! Codex agents that work on this computer (docs/specs/companion.md §7.2): the unmodified `codex` CLI's app-server
//! (JSON-RPC, a line a message, over its stdio), because `codex exec` can't ask before it acts.
//!
//! - **Its own Codex home** (`%LOCALAPPDATA%\r1os\companion\codex`): the app-server can't ignore your `~/.codex`, whose
//!   settings (an auto-reviewer for approvals, full disk access, your rules, plugins and MCP servers) would decide
//!   for the agent. It's signed in once with Codex's own flow ([`sign_in`]); the CLI keeps the credential there.
//! - **A chat's thread** is started (or resumed) with the agent's workspace as its folder, its other folders writable,
//!   Codex's sandbox (`workspace-write`, unelevated on Windows; `read-only` for plans only), an approval policy that
//!   asks (`untrusted`; `on-request` for auto), r1d's prompt as developer instructions, and r1d's tools (`r1`, the
//!   runner's relay, by the turn's secret path).
//! - **A turn** streams back as the `codex exec --json` lines r1d already reads (crate r1d's `chat::CodexTurn`):
//!   `thread.started`, `item.started` for tool steps, `item.completed` for the agent's messages, `turn.completed`
//!   with the usage, context and window.
//! - **Its questions:** a command, or a change to files, comes as a request; the runner answers it as the agent's
//!   levels say, or asks the R1 (`approve`). Reads outside its folders don't ask with Codex: its sandbox reads the disk.

use crate::runner::{Runner, configures, inside};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// The Codex home the companion's agents use.
pub fn home() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("r1os").join("companion").join("codex")
}

/// Where `codex.exe` is: the real one, also behind npm's `codex.cmd` (which only starts it).
pub fn find() -> Option<PathBuf> {
    let vendored = |dir: &Path| {
        dir.join("node_modules").join("@openai").join("codex").join("node_modules").join("@openai").join("codex-win32-x64").join("vendor").join("x86_64-pc-windows-msvc").join("bin").join("codex.exe")
    };
    let mut places = Vec::new();
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        places.push(dir.join("codex.exe"));
        places.push(vendored(&dir));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        places.push(vendored(&PathBuf::from(appdata).join("npm")));
    }
    places.into_iter().find(|p| p.is_file())
}

/// A message from the app-server: a notification, or a request it waits on (answered with [`Codex::reply`]).
pub struct Incoming {
    pub method: String,
    pub params: Value,
    pub id: Option<Value>,
}

/// Our requests waiting for their answers, by id.
type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// A running app-server.
pub struct Codex {
    child: tokio::process::Child,
    stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
    pending: Pending,
    next: AtomicI64,
    pub incoming: mpsc::UnboundedReceiver<Incoming>,
    pub tail: Arc<Mutex<String>>,
}

impl Codex {
    /// Starts `codex app-server` with the companion's Codex home, and says hello.
    pub async fn start(exe: &Path) -> Result<Codex> {
        let home = home();
        std::fs::create_dir_all(&home)?;
        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg("app-server")
            // Nothing of the CLI's that would act on its own: plugins, apps, computer and browser use, hooks.
            .args(["--disable", "plugins", "--disable", "apps", "--disable", "computer_use", "--disable", "browser_use", "--disable", "hooks"])
            .args(["-c", "notify=[]", "-c", "windows.sandbox=\"unelevated\""])
            .env("CODEX_HOME", &home)
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().context("start codex")?;
        let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take().context("stdin")?));
        let stdout = child.stdout.take().context("stdout")?;
        let err = child.stderr.take().context("stderr")?;
        let tail = Arc::new(Mutex::new(String::new()));
        let sink = tail.clone();
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
        let pending: Pending = Arc::default();
        let (tx, incoming) = mpsc::unbounded_channel();
        let waiting = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
                match (m.get("method").and_then(Value::as_str), m.get("id")) {
                    // A notification, or a request to us.
                    (Some(method), id) => {
                        let _ = tx.send(Incoming { method: method.to_owned(), params: m["params"].clone(), id: id.cloned() });
                    }
                    // An answer to one of ours.
                    (None, Some(id)) => {
                        let Some(id) = id.as_i64() else { continue };
                        if let Some(answer) = waiting.lock().unwrap().remove(&id) {
                            let out = match m.get("error") {
                                Some(e) => Err(e["message"].as_str().unwrap_or("codex said no").to_owned()),
                                None => Ok(m["result"].clone()),
                            };
                            let _ = answer.send(out);
                        }
                    }
                    _ => {}
                }
            }
        });
        let mut codex = Codex { child, stdin, pending, next: AtomicI64::new(1), incoming, tail };
        codex
            .request("initialize", json!({
                "clientInfo": { "name": "r1os_companion", "title": "r1os", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "optOutNotificationMethods": ["remoteControl/status/changed"] },
            }))
            .await?;
        codex.notify("initialized", Value::Null).await?;
        Ok(codex)
    }

    async fn send(&self, msg: Value) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(format!("{msg}\n").as_bytes()).await.context("codex isn't running")?;
        stdin.flush().await?;
        Ok(())
    }

    /// A request, and its result (Codex's error as ours).
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.send(json!({ "id": id, "method": method, "params": params })).await?;
        match tokio::time::timeout(Duration::from_secs(120), rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => bail!("{e}"),
            _ => {
                let err = self.tail.lock().unwrap().clone();
                bail!("codex didn't answer {method}: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("nothing"))
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(if params.is_null() { json!({ "method": method }) } else { json!({ "method": method, "params": params }) }).await
    }

    /// Answers one of its requests.
    pub async fn reply(&self, id: Value, result: Value) -> Result<()> {
        self.send(json!({ "id": id, "result": result })).await
    }

    pub async fn refuse(&self, id: Value, why: &str) -> Result<()> {
        self.send(json!({ "id": id, "error": { "code": -32601, "message": why } })).await
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Whether the companion's Codex is signed in: `{signed_in, account?}`.
pub async fn status(exe: &Path) -> Result<Value> {
    let mut c = Codex::start(exe).await?;
    let r = c.request("account/read", json!({})).await?;
    Ok(json!({ "signed_in": !r["account"].is_null(), "account": r["account"]["email"] }))
}

/// Signs the companion's Codex in with Codex's own flow: its page opens in the browser (`open`), and this waits for
/// it to finish (up to ten minutes). The CLI keeps the credential in its home; r1os never sees it.
pub async fn sign_in(exe: &Path, open: impl FnOnce(String)) -> Result<()> {
    let mut c = Codex::start(exe).await?;
    let r = c.request("account/login/start", json!({ "type": "chatgpt" })).await?;
    let url = r["authUrl"].as_str().context("codex gave no sign-in page")?.to_owned();
    open(url);
    let wait = async {
        while let Some(m) = c.incoming.recv().await {
            if m.method == "account/login/completed" {
                return if m.params["success"] == true { Ok(()) } else { Err(anyhow!("{}", m.params["error"].as_str().unwrap_or("sign-in didn't finish"))) };
            }
        }
        Err(anyhow!("codex stopped"))
    };
    tokio::time::timeout(Duration::from_secs(600), wait).await.map_err(|_| anyhow!("sign-in took too long"))?
}

/// A chat's Codex, kept between its turns.
pub struct Thread {
    pub codex: Codex,
    pub id: String,
}

/// What a turn needs from the runner.
pub struct Setup<'a> {
    pub exe: &'a Path,
    pub workspace: &'a Path,
    pub folders: &'a [PathBuf],
    pub mode: &'a str,
    pub instructions: String,
    pub r1_url: String,
    pub session: Option<String>,
}

/// Starts the app-server and the chat's thread (resumed when it has one).
pub async fn thread(s: Setup<'_>) -> Result<Thread> {
    let mut codex = Codex::start(s.exe).await?;
    let writable: Vec<String> = s.folders[1..].iter().map(|f| f.display().to_string()).collect();
    let (sandbox, policy) = match s.mode {
        "plan" => ("read-only", "untrusted"),
        "auto" => ("workspace-write", "on-request"),
        _ => ("workspace-write", "untrusted"),
    };
    let mut params = json!({
        "cwd": s.workspace.display().to_string(), "sandbox": sandbox, "approvalPolicy": policy, "approvalsReviewer": "user",
        "developerInstructions": s.instructions,
        "config": {
            "sandbox_workspace_write.writable_roots": writable,
            "sandbox_workspace_write.network_access": true,
            "mcp_servers": { "r1": { "url": s.r1_url, "tool_timeout_sec": 180, "default_tools_approval_mode": "approve" } },
            "model_reasoning_effort": "low",
            "tools.web_search": true,
        },
    });
    let started = match &s.session {
        Some(id) => {
            params["threadId"] = json!(id);
            codex.request("thread/resume", params).await?
        }
        None => codex.request("thread/start", params).await?,
    };
    let id = started["thread"]["id"].as_str().context("codex started no thread")?.to_owned();
    let kind = started["sandbox"]["type"].as_str().unwrap_or_default();
    if sandbox == "workspace-write" && kind != "workspaceWrite" {
        tracing::warn!(kind, "codex's sandbox isn't what was asked for");
    }
    Ok(Thread { codex, id })
}

/// The turn: `input` sent, and what comes back translated for r1d (`say`), questions answered as the runner decides.
/// Returns the last lines' summary for the R1: `{thread_id, status, error?}`.
pub async fn turn(
    t: &mut Thread,
    runner: &Arc<Runner>,
    secret: &str,
    folders: &[PathBuf],
    input: Vec<Value>,
    send: &mut r1_link::SendStream,
    stop: &tokio::sync::Notify,
) -> Result<Value> {
    // One line to r1d, as `codex exec --json` writes it.
    async fn say(send: &mut r1_link::SendStream, line: Value) -> Result<()> {
        r1_link::say(send, &r1_link::Msg::Event { event: json!({ "line": line }) }).await.context("the R1 went")
    }
    say(send, json!({ "type": "thread.started", "thread_id": t.id })).await?;
    let started = t.codex.request("turn/start", json!({ "threadId": t.id, "input": input })).await?;
    let turn_id = started["turn"]["id"].clone();
    let mut usage = Value::Null;
    // The file changes it means to make, by item (a change's question names only the item).
    let mut changes: HashMap<String, Vec<String>> = HashMap::new();
    let mut stopping = false;
    loop {
        let next = tokio::select! {
            m = t.codex.incoming.recv() => Some(m),
            () = stop.notified(), if !stopping => None,
        };
        let m = match next {
            Some(Some(m)) => m,
            Some(None) => break,
            // Stopped on the R1: the turn ends as `interrupted`, and the thread carries on.
            None => {
                stopping = true;
                if let Err(e) = t.codex.request("turn/interrupt", json!({ "threadId": t.id, "turnId": turn_id })).await {
                    tracing::info!(error = %format!("{e:#}"), "codex: couldn't interrupt the turn");
                }
                continue;
            }
        };
        let p = &m.params;
        match m.method.as_str() {
            "item/started" => {
                let item = &p["item"];
                if item["type"] == "fileChange" {
                    let paths = item["changes"].as_array().into_iter().flatten().filter_map(|c| c["path"].as_str().map(str::to_owned)).collect();
                    changes.insert(item["id"].as_str().unwrap_or_default().to_owned(), paths);
                }
                let kind = match item["type"].as_str() {
                    Some("mcpToolCall") => Some("mcp_tool_call"),
                    Some("commandExecution") => Some("command_execution"),
                    Some("fileChange") => Some("file_change"),
                    Some("webSearch") => Some("web_search"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    runner.used(secret, item["tool"].as_str().unwrap_or(kind));
                    say(send, json!({ "type": "item.started", "item": { "type": kind, "tool": item["tool"] } })).await?;
                }
            }
            "item/completed" if p["item"]["type"] == "agentMessage" => {
                say(send, json!({ "type": "item.completed", "item": { "type": "agent_message", "text": p["item"]["text"] } })).await?;
            }
            "thread/tokenUsage/updated" => usage = p["tokenUsage"].clone(),
            "turn/completed" => {
                let total = &usage["total"];
                say(send, json!({
                    "type": "turn.completed",
                    "usage": { "input_tokens": total["inputTokens"], "cached_input_tokens": total["cachedInputTokens"], "output_tokens": total["outputTokens"] },
                    "context": usage["last"]["totalTokens"], "window": usage["modelContextWindow"],
                }))
                .await?;
                let status = p["turn"]["status"].as_str().unwrap_or("completed");
                if status == "failed" {
                    let why = p["turn"]["error"]["message"].as_str().unwrap_or("the turn failed").to_owned();
                    say(send, json!({ "type": "turn.failed", "error": { "message": why } })).await?;
                }
                return Ok(json!({ "thread_id": t.id, "status": status }));
            }
            "error" if p["willRetry"] != true => tracing::info!(error = %p["error"]["message"], "codex: an error in the turn"),
            // Its questions.
            "item/commandExecution/requestApproval" => {
                let command = p["command"].as_str().unwrap_or_default();
                let title = format!("run {}", command.chars().take(120).collect::<String>());
                let detail = p["reason"].as_str().unwrap_or_else(|| p["cwd"].as_str().unwrap_or_default()).to_owned();
                let ok = runner.ask(secret, "commands", title, detail).await.is_ok();
                if let Some(id) = m.id {
                    t.codex.reply(id, json!({ "decision": if ok { "accept" } else { "decline" } })).await?;
                }
            }
            "item/fileChange/requestApproval" => {
                let paths = changes.get(p["itemId"].as_str().unwrap_or_default()).cloned().unwrap_or_default();
                let mut ok = !paths.is_empty();
                for path in &paths {
                    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
                    let (area, detail) = if configures(path) {
                        ("outside", format!("{path} (its tools' settings)"))
                    } else if inside(folders, path) {
                        ("files", path.clone())
                    } else {
                        ("outside", format!("{path} (outside its folders)"))
                    };
                    if runner.ask(secret, area, format!("change {name}"), detail).await.is_err() {
                        ok = false;
                        break;
                    }
                }
                if let Some(id) = m.id {
                    t.codex.reply(id, json!({ "decision": if ok { "accept" } else { "decline" } })).await?;
                }
            }
            // r1d's tools are r1d's to check: their prompts are allowed.
            "mcpServer/elicitation/request" => {
                if let Some(id) = m.id {
                    let action = if p["serverName"] == "r1" { "accept" } else { "decline" };
                    t.codex.reply(id, json!({ "action": action })).await?;
                }
            }
            "item/permissions/requestApproval" => {
                if let Some(id) = m.id {
                    t.codex.reply(id, json!({ "permissions": {}, "scope": "turn" })).await?;
                }
            }
            _ => {
                if let Some(id) = m.id {
                    t.codex.refuse(id, "the companion doesn't answer that").await?;
                }
            }
        }
    }
    let err = t.codex.tail.lock().unwrap().clone();
    bail!("codex stopped: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output"))
}
