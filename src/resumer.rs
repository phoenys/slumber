use crate::{
    core::{
        AgentKind, JobMeta, JobSubmitRequest, exit_status_text, job_dir, now_secs,
        write_private_atomic,
    },
    tmux,
};
use anyhow::{Context, Result, bail};
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path, process::Stdio};
use tokio::process::Command;

pub async fn resume(
    request: &JobSubmitRequest,
    meta: &JobMeta,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<()> {
    tmux::close_log_pane(request).await;
    let Some(status) = meta.exit_status.as_ref() else {
        return Ok(());
    };
    let started_at = meta.started_at.unwrap_or(meta.created_at);
    let duration = meta
        .finished_at
        .unwrap_or_else(now_secs)
        .saturating_sub(started_at);
    let payload = build_payload(request, status, duration, stdout_path, stderr_path);
    let directory = job_dir(&meta.job_id)?;
    let payload_path = directory.join("payload.md");
    write_private_atomic(&payload_path, payload.as_bytes())?;
    let mut command = match &request.agent_kind {
        AgentKind::NoResume => return Ok(()),
        AgentKind::CodeX => {
            let session_id = request
                .session_id
                .as_deref()
                .context("missing Codex session ID")?;
            let mut command = Command::new("codex");
            command.args(["queue", "--thread", session_id, "--message", &payload]);
            command
        }
        AgentKind::GenericCommand { resume_template } => {
            let mut command = Command::new("sh");
            command.args(["-c", resume_template]);
            command
        }
    };
    let resume_log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(directory.join("resume.log"))?;
    let resume_err = resume_log.try_clone()?;
    let status = command
        .current_dir(&request.cwd)
        .env_clear()
        .envs(&request.env_vars)
        .env(
            "SLUMBER_SESSION_ID",
            request.session_id.as_deref().unwrap_or(""),
        )
        .env("SLUMBER_PAYLOAD", &payload)
        .env("SLUMBER_PAYLOAD_PATH", &payload_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(resume_log))
        .stderr(Stdio::from(resume_err))
        .status()
        .await
        .context("run wake-up command")?;
    if !status.success() {
        bail!(
            "wake-up command failed ({status}); inspect {}/resume.log",
            directory.display()
        );
    }
    Ok(())
}

fn build_payload(
    request: &JobSubmitRequest,
    status: &crate::core::ExitStatusRecord,
    duration_secs: u64,
    stdout_path: &Path,
    stderr_path: &Path,
) -> String {
    let remote_target = request
        .ssh_target
        .as_deref()
        .map(|target| format!("• Remote Target    : {target}\n"))
        .unwrap_or_default();
    format!(
        "[SLUMBER EVENT: TASK TERMINATED]\n\
         --------------------------------------------------\n\
         • Command          : {}\n\
         • Working Directory: {}\n\
         {}\
         • Exit Status      : {}\n\
         • Duration         : {}\n\
         • Logs:\n\
           - STDOUT Log     : {}\n\
           - STDERR Log     : {}\n\
         --------------------------------------------------\n\
         INSTRUCTION: Inspect logs if needed and decide on next actions.",
        request.command,
        request.cwd.display(),
        remote_target,
        exit_status_text(status),
        format_duration(duration_secs),
        stdout_path.display(),
        stderr_path.display(),
    )
}

fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_compact_durations() {
        assert_eq!(format_duration(7), "7s");
        assert_eq!(format_duration(3_722), "1h 2m 2s");
    }
}
