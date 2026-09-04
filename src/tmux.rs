use crate::core::JobSubmitRequest;
use anyhow::{Context, Result, bail};
use std::{path::Path, process::Stdio};
use tokio::process::Command;

pub async fn open_log_pane(
    request: &JobSubmitRequest,
    job_id: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<Option<String>> {
    if request.tmux_pane_id.as_deref() != Some("") {
        return Ok(None);
    }
    let tmux_env = request.env_vars.get("TMUX").context("TMUX is not set")?;
    let tail_command = match request.ssh_target.as_deref() {
        Some(target) => {
            let remote_tail = format!(
                "tail -f -n 20 \"$HOME/.slumber/jobs/{job_id}/stdout.log\" \"$HOME/.slumber/jobs/{job_id}/stderr.log\""
            );
            format!(
                "ssh -t -o BatchMode=yes -o ConnectTimeout=10 {} {}",
                shell_quote(target),
                shell_quote(&remote_tail)
            )
        }
        None => format!(
            "tail -f -n 20 {} {}",
            shell_quote(&stdout_path.to_string_lossy()),
            shell_quote(&stderr_path.to_string_lossy())
        ),
    };

    let mut command = Command::new("tmux");
    command
        .args(["split-window", "-h", "-d", "-P", "-F", "#{pane_id}"])
        .env("TMUX", tmux_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(source_pane) = request.env_vars.get("TMUX_PANE") {
        command.args(["-t", source_pane]);
    }
    let output = command.arg(tail_command).output().await?;
    if !output.status.success() {
        bail!(
            "tmux split-window failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let pane_id = String::from_utf8(output.stdout)?;
    let pane_id = pane_id.trim();
    if pane_id.is_empty() {
        bail!("tmux did not return a pane id");
    }
    Ok(Some(pane_id.to_owned()))
}

pub async fn close_log_pane(request: &JobSubmitRequest) {
    let Some(pane_id) = request.tmux_pane_id.as_deref().filter(|id| !id.is_empty()) else {
        return;
    };
    let mut command = Command::new("tmux");
    command
        .args(["kill-pane", "-t", pane_id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(tmux_env) = request.env_vars.get("TMUX") {
        command.env("TMUX", tmux_env);
    }
    let _ = command.status().await;
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
