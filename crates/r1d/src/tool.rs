//! Android's shell tools (`cmd`, `input`, `am`), run with a deadline.
//!
//! These hand their output to system_server, which writes the answer and then calls back into the
//! tool's process to report the result. When SELinux refuses that call-back (image v0.2.4:
//! `system_server → r1d` binder `call`), the tool hangs forever after its work is done, and an
//! awaited `output()` never returns: requests stalled and stuck `cmd` processes piled up
//! (2026-10-09). So the output is read until the tool exits or the deadline passes, and a tool still
//! there then is killed. A hung tool holds its own pipes open, so that case costs the whole deadline.
//!
//! Run `cmd` itself, not `input` or `am`: those are shell scripts that start `cmd` as a child, and
//! killing the script leaves `cmd` running.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// How long a tool gets: these answer in well under a second when nothing's wrong.
pub const DEADLINE: Duration = Duration::from_secs(4);

pub struct Output {
    pub stdout: String,
    pub stderr: String,
    /// It exited successfully, or it was stopped at the deadline with nothing on stderr (it did
    /// its work and hung on the call-back).
    pub ok: bool,
}

pub async fn run(program: &str, args: &[&str], deadline: Duration) -> anyhow::Result<Output> {
    let (out, err, ok) = run_raw(program, args, deadline).await?;
    Ok(Output { stdout: String::from_utf8_lossy(&out).into_owned(), stderr: String::from_utf8_lossy(&err).into_owned(), ok })
}

/// [`run`] for a tool whose output is binary (`screencap -p`): its stdout as bytes, whatever arrived
/// before it exited or the deadline passed.
pub async fn run_bytes(program: &str, args: &[&str], deadline: Duration) -> anyhow::Result<(Vec<u8>, bool)> {
    let (out, _, ok) = run_raw(program, args, deadline).await?;
    Ok((out, ok))
}

async fn run_raw(program: &str, args: &[&str], deadline: Duration) -> anyhow::Result<(Vec<u8>, Vec<u8>, bool)> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut so = child.stdout.take().expect("piped stdout");
    let mut se = child.stderr.take().expect("piped stderr");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let finished = tokio::time::timeout(deadline, async {
        let _ = tokio::join!(so.read_to_end(&mut out), se.read_to_end(&mut err));
        child.wait().await
    })
    .await;
    let ok = match finished {
        Ok(Ok(status)) => status.success(),
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => {
            let _ = child.kill().await;
            tracing::warn!(program, ?args, "tool hung past its deadline; stopped it");
            err.is_empty()
        }
    };
    Ok((out, err, ok))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_promptly_and_stops_a_hung_tool() {
        let fast = run("sh", &["-c", "echo hi"], DEADLINE).await.unwrap();
        assert!(fast.ok);
        assert_eq!(fast.stdout.trim(), "hi");
        // Did its work, then hung (like `cmd` refused its call-back): stopped at the deadline.
        let hung = run("sh", &["-c", "echo done; sleep 30"], Duration::from_millis(300)).await.unwrap();
        assert!(hung.ok);
        assert_eq!(hung.stdout.trim(), "done");
    }
}
