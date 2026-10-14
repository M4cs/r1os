//! Agent sessions in the box (ADR-0003).
//!
//! A build session is one headless `claude -p` run in r1d's jail (crate::jail: private mount
//! namespace, chroot, agent uid without caps, seccomp, memory cgroup). It's fed the prompt on stdin and parsed from
//! `--output-format stream-json`. No-tools builds (eneeware's fast mode) always strip every tool,
//! MCP server, skill and slash command. Without `--strict-mcp-config`, Max's claude.ai connectors
//! would load as MCP tools (docs/research, "no-tools builds").
//!
//! A Codex build (T-045: the evals' Codex arm, and `codex` among the build models) is one `codex exec` in the same
//! jail, without its code-mode host, so any tool it reaches for fails closed: no tools there either.

use anyhow::{Context, bail};
use serde_json::Value;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::jail::{AGENT_UID, CLAUDE, CODEX, Jail};

#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    /// `--effort` (low, medium, high); None leaves the CLI's default.
    pub effort: Option<String>,
    /// Replaces Claude Code's default system prompt (the builder prompt).
    pub system_prompt: String,
    pub prompt: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Outcome {
    pub text: String,
    pub cost_usd: Option<f64>,
    pub duration_ms: u64,
    pub output_tokens: Option<u64>,
}

/// Progress callbacks while the model streams.
pub trait Progress: Send {
    fn text(&mut self, delta: &str);
}

impl<F: FnMut(&str) + Send> Progress for F {
    fn text(&mut self, delta: &str) {
        self(delta)
    }
}

/// Runs one no-tools Claude turn in the box and returns its final text.
pub async fn claude_no_tools(req: &Request, progress: &mut dyn Progress) -> anyhow::Result<Outcome> {
    let jail = Jail::default();
    if !jail.available() {
        bail!("agent sessions need the R1's agent box");
    }
    jail.check_quota()?;
    jail.refresh_dns().await;
    // The system prompt can be long, so it goes through a file in the box's /work.
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let session = format!("r1d-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let host_dir = format!("{}/{session}", crate::chat::sessions_dir(&jail).await?);
    tokio::fs::create_dir_all(&host_dir).await.context("session dir")?;
    tokio::fs::write(format!("{host_dir}/system.md"), &req.system_prompt).await?;
    // The agent user must be able to read it (the box drops to uid 2950).
    let _ = Command::new("chown").args(["-R", &format!("{AGENT_UID}:{AGENT_UID}"), &host_dir]).status().await;
    let box_prompt = format!("/work/.sessions/{session}/system.md");

    let started = Instant::now();
    let mut args: Vec<&str> = vec!["-p", "--output-format", "stream-json", "--include-partial-messages", "--verbose"];
    args.extend(["--model", &req.model]);
    if let Some(e) = req.effort.as_deref() {
        args.extend(["--effort", e]);
    }
    args.extend(["--tools", "", "--strict-mcp-config", "--restricted", "--disable-slash-commands"]);
    args.extend(["--no-session-persistence", "--system-prompt-file", &box_prompt]);
    let mut child = jail
        .command(CLAUDE, &args, "/work", &[("NO_COLOR", "1")])?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("start claude in the agent box")?;

    let mut stdin = child.stdin.take().context("stdin")?;
    stdin.write_all(req.prompt.as_bytes()).await?;
    drop(stdin); // EOF: -p reads the whole prompt from stdin

    let stdout = child.stdout.take().context("stdout")?;
    let stderr = child.stderr.take().context("stderr")?;
    let err_task = tokio::spawn(async move {
        let mut s = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if s.len() < 4000 {
                s.push_str(&l);
                s.push('\n');
            }
        }
        s
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut result: Option<Value> = None;
    let read = async {
        while let Some(line) = lines.next_line().await? {
            let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
            match ev["type"].as_str() {
                Some("stream_event") => {
                    let d = &ev["event"]["delta"];
                    if d["type"] == "text_delta"
                        && let Some(t) = d["text"].as_str()
                    {
                        progress.text(t);
                    }
                }
                Some("result") => result = Some(ev),
                _ => {}
            }
        }
        anyhow::Ok(())
    };
    let timed_out = tokio::time::timeout(req.timeout, read).await.is_err();
    if timed_out {
        let _ = child.kill().await;
    }
    let status = child.wait().await?;
    let stderr_text = err_task.await.unwrap_or_default();
    let _ = tokio::fs::remove_dir_all(&host_dir).await;

    if timed_out {
        bail!("the agent took longer than {} s", req.timeout.as_secs());
    }
    let Some(r) = result else {
        bail!("the agent exited ({status}) without a result: {}", stderr_text.trim());
    };
    if r["is_error"].as_bool() == Some(true) {
        bail!("the agent failed: {}", r["result"].as_str().unwrap_or("unknown error"));
    }
    Ok(Outcome {
        text: r["result"].as_str().unwrap_or_default().to_owned(),
        cost_usd: r["total_cost_usd"].as_f64(),
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        output_tokens: r["usage"]["output_tokens"].as_u64(),
    })
}

/// Is `model` one for Codex (`codex`, or `codex:<model>`)? Else it's Claude's.
pub fn is_codex(model: &str) -> bool {
    model == "codex" || model.starts_with("codex:")
}

/// Runs one Codex turn in the box with no working tools and returns its final message (T-045). Codex has no system
/// prompt here, so the builder's goes ahead of the message. `codex` is Codex's default model; `codex:<model>` picks one.
pub async fn codex_no_tools(req: &Request, progress: &mut dyn Progress) -> anyhow::Result<Outcome> {
    let jail = Jail::default();
    if !jail.available() {
        bail!("agent sessions need the R1's agent box");
    }
    jail.check_quota()?;
    jail.refresh_dns().await;
    let started = Instant::now();
    let effort = format!("model_reasoning_effort=\"{}\"", req.effort.as_deref().unwrap_or("low"));
    let model = req.model.strip_prefix("codex:").map(str::to_owned);
    // No code-mode host (`features.code_mode_host`), so its tools fail closed; no web search either. The jail is the
    // sandbox, as for an agent's Codex turns (ADR-0003).
    let mut args: Vec<&str> = vec!["exec", "--json", "--skip-git-repo-check", "--dangerously-bypass-approvals-and-sandbox"];
    // No ChatGPT apps either: a build has no one to ask before one acts (ADR-0058).
    args.extend(["-c", &effort, "-c", "tools.web_search=false", "-c", "features.apps=false"]);
    if let Some(m) = &model {
        args.extend(["-m", m]);
    }
    args.push("-");
    let mut child = jail
        .command(CODEX, &args, "/work", &[("NO_COLOR", "1")])?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("start codex in the agent box")?;
    let mut stdin = child.stdin.take().context("stdin")?;
    stdin.write_all(format!("{}\n\n---\n\n{}", req.system_prompt, req.prompt).as_bytes()).await?;
    drop(stdin);
    let stderr = child.stderr.take().context("stderr")?;
    let err_task = tokio::spawn(async move {
        let mut s = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if s.len() < 4000 {
                s.push_str(&l);
                s.push('\n');
            }
        }
        s
    });
    let mut lines = BufReader::new(child.stdout.take().context("stdout")?).lines();
    let (mut reply, mut usage, mut failed) = (String::new(), Value::Null, None::<String>);
    let read = async {
        while let Some(line) = lines.next_line().await? {
            let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
            match (ev["type"].as_str(), ev["item"]["type"].as_str()) {
                (Some("item.completed"), Some("agent_message")) => {
                    if let Some(t) = ev["item"]["text"].as_str() {
                        if !reply.is_empty() {
                            reply.push_str("\n\n");
                        }
                        reply.push_str(t);
                        progress.text(t);
                    }
                }
                (Some("turn.completed"), _) => usage = ev["usage"].clone(),
                (Some("turn.failed" | "error"), _) => {
                    failed = ev["error"]["message"].as_str().or_else(|| ev["message"].as_str()).map(str::to_owned);
                }
                _ => {}
            }
        }
        anyhow::Ok(())
    };
    let timed_out = tokio::time::timeout(req.timeout, read).await.is_err();
    if timed_out {
        let _ = child.kill().await;
    }
    let status = child.wait().await?;
    let stderr_text = err_task.await.unwrap_or_default();
    if timed_out {
        bail!("Codex took longer than {} s", req.timeout.as_secs());
    }
    if let Some(f) = failed {
        bail!("Codex failed: {f}");
    }
    if reply.is_empty() {
        bail!("Codex exited ({status}) without a reply: {}", stderr_text.trim());
    }
    Ok(Outcome {
        text: reply,
        cost_usd: None,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        output_tokens: usage["output_tokens"].as_u64(),
    })
}
