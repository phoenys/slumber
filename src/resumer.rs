use crate::{
    core::{AgentKind, JobMeta, JobSubmitRequest, exit_status_text, job_dir, now_secs},
    tmux,
};
use anyhow::{Context, Result};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    process::Stdio,
};
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
    let duration = meta.finished_at.unwrap_or_else(now_secs) - started_at;
    let payload = build_payload(request, status, duration, stdout_path, stderr_path);
    let directory = job_dir(&meta.job_id)?;
    let payload_path = directory.join("payload.md");
    fs::write(&payload_path, &payload)?;
    fs::set_permissions(&payload_path, fs::Permissions::from_mode(0o600))?;

    match &request.agent_kind {
        AgentKind::CodeX => {
            let Some(session_id) = request.session_id.as_deref() else {
                return Ok(());
            };
            let resume_log = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(directory.join("resume.log"))?;
            let resume_err = resume_log.try_clone()?;
            Command::new("codex")
                .args(["queue", "--thread", session_id, "--message", &payload])
                .current_dir(&request.cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::from(resume_log))
                .stderr(Stdio::from(resume_err))
                .spawn()
                .context("start Codex resumer")?;
        }
        AgentKind::GenericCommand { resume_template } => {
            Command::new("sh")
                .args(["-c", resume_template])
                .current_dir(&request.cwd)
                .env(
                    "SLUMBER_SESSION_ID",
                    request.session_id.as_deref().unwrap_or(""),
                )
                .env("SLUMBER_PAYLOAD", &payload)
                .env("SLUMBER_PAYLOAD_PATH", &payload_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("start generic resumer")?;
        }
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
