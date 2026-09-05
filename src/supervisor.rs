use crate::{
    core::{
        ExitStatusRecord, JobMeta, JobSubmitRequest, JobSubmitResponse, create_private_dir,
        job_dir, now_secs, read_request, recent_jobs, signal_name, write_meta, write_request,
    },
    resumer, tmux,
};
use anyhow::{Context, Result, bail};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::{Pid, setpgid},
};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    future::Future,
    os::unix::{fs::OpenOptionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::{
    io::AsyncWriteExt,
    process::Command,
    time::{Duration, sleep},
};

static JOB_SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub type Completion = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

pub async fn start(request: JobSubmitRequest) -> Result<(JobSubmitResponse, Completion)> {
    if !request.cwd.is_dir() {
        bail!(
            "working directory does not exist: {}",
            request.cwd.display()
        );
    }
    if request.ssh_target.is_some() {
        start_remote(request).await
    } else {
        start_local(request).await
    }
}

async fn start_local(mut request: JobSubmitRequest) -> Result<(JobSubmitResponse, Completion)> {
    let created_at = now_secs();
    let sequence = JOB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let job_id = format!("job_{created_at}_{}_{}", std::process::id(), sequence);
    let directory = job_dir(&job_id)?;
    create_private_dir(&directory)?;
    let stdout_path = directory.join("stdout.log");
    let stderr_path = directory.join("stderr.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&stdout_path)?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&stderr_path)?;

    let mut meta = JobMeta {
        job_id: job_id.clone(),
        command: request.command.clone(),
        cwd: request.cwd.clone(),
        created_at,
        started_at: None,
        finished_at: None,
        pgid: None,
        exit_status: None,
        ssh_target: None,
        resumed_at: None,
        tmux_pane_id: None,
    };
    write_meta(&meta)?;

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(&request.command)
        .current_dir(&request.cwd)
        .env_clear()
        .envs(&request.env_vars)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    unsafe {
        command.pre_exec(|| {
            setpgid(Pid::from_raw(0), Pid::from_raw(0)).map_err(std::io::Error::other)?;
            Ok(())
        });
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            meta.finished_at = Some(now_secs());
            meta.exit_status = Some(ExitStatusRecord::FailedToStart(error.to_string()));
            write_meta(&meta)?;
            return Err(error).context("start delegated command");
        }
    };
    let pgid = child.id().context("delegated process has no PID")?;
    meta.started_at = Some(now_secs());
    meta.pgid = Some(pgid);
    attach_log_pane(&mut request, &mut meta, &stdout_path, &stderr_path).await;
    write_meta(&meta)?;

    let response = JobSubmitResponse {
        job_id: job_id.clone(),
        pgid,
        stdout_path: stdout_path.clone(),
        stderr_path: stderr_path.clone(),
        message: "Task is running; it is safe for the agent to exit.".to_owned(),
        tmux_pane_id: request.tmux_pane_id.clone(),
    };

    let completion = Box::pin(async move {
        let status = child.wait().await.context("wait for delegated command")?;
        let record = status_record(status);
        let _ = killpg(Pid::from_raw(pgid as i32), Signal::SIGTERM);
        meta.finished_at = Some(now_secs());
        meta.exit_status = Some(record);
        write_meta(&meta)?;
        resumer::resume(&request, &meta, &stdout_path, &stderr_path).await?;
        meta.resumed_at = Some(now_secs());
        write_meta(&meta)
    });
    Ok((response, completion))
}

async fn start_remote(mut request: JobSubmitRequest) -> Result<(JobSubmitResponse, Completion)> {
    let target = request.ssh_target.clone().context("missing SSH target")?;
    validate_ssh_target(&target)?;

    let created_at = now_secs();
    let sequence = JOB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let job_id = format!("job_{created_at}_{}_{}", std::process::id(), sequence);
    let directory = job_dir(&job_id)?;
    create_private_dir(&directory)?;
    let (stdout_path, stderr_path) = remote_log_paths(&target, &job_id);
    let mut meta = JobMeta {
        job_id: job_id.clone(),
        command: request.command.clone(),
        cwd: request.cwd.clone(),
        created_at,
        started_at: None,
        finished_at: None,
        pgid: None,
        exit_status: None,
        ssh_target: Some(target.clone()),
        resumed_at: None,
        tmux_pane_id: None,
    };
    write_meta(&meta)?;
    write_request(&job_id, &request)?;

    let script = remote_wrapper(&job_id, &request.command);
    let remote_pid = match launch_remote(&target, &script, &request.env_vars).await {
        Ok(pid) => pid,
        Err(error) => {
            meta.finished_at = Some(now_secs());
            meta.exit_status = Some(ExitStatusRecord::FailedToStart(format!("{error:#}")));
            meta.resumed_at = Some(now_secs());
            write_meta(&meta)?;
            clear_persisted_environment(&job_id, &request)?;
            return Err(error);
        }
    };
    meta.started_at = Some(now_secs());
    meta.pgid = Some(remote_pid);
    attach_log_pane(&mut request, &mut meta, &stdout_path, &stderr_path).await;
    write_meta(&meta)?;
    write_request(&job_id, &request)?;

    let response = JobSubmitResponse {
        job_id: job_id.clone(),
        pgid: remote_pid,
        stdout_path: stdout_path.clone(),
        stderr_path: stderr_path.clone(),
        message: format!("Remote task is running on {target}; it is safe for the agent to exit."),
        tmux_pane_id: request.tmux_pane_id.clone(),
    };
    let completion = Box::pin(monitor_remote(request, meta, stdout_path, stderr_path));
    Ok((response, completion))
}

async fn attach_log_pane(
    request: &mut JobSubmitRequest,
    meta: &mut JobMeta,
    stdout_path: &Path,
    stderr_path: &Path,
) {
    match tmux::open_log_pane(request, &meta.job_id, stdout_path, stderr_path).await {
        Ok(Some(pane_id)) => {
            request.tmux_pane_id = Some(pane_id.clone());
            meta.tmux_pane_id = Some(pane_id);
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("slumberd: could not open tmux log pane: {error:#}");
            request.tmux_pane_id = None;
        }
    }
}

fn clear_persisted_environment(job_id: &str, request: &JobSubmitRequest) -> Result<()> {
    let mut cleared = request.clone();
    cleared.env_vars.clear();
    write_request(job_id, &cleared)
}

pub fn recover_remote_jobs() -> Result<Vec<Completion>> {
    let mut completions = Vec::new();
    for meta in recent_jobs(usize::MAX)? {
        if meta.ssh_target.is_none() || meta.resumed_at.is_some() {
            continue;
        }
        let request_path = job_dir(&meta.job_id)?.join("request.json");
        let request = match read_request(&request_path) {
            Ok(request) => request,
            Err(error) => {
                eprintln!(
                    "slumberd: cannot recover remote job {}: {error:#}",
                    meta.job_id
                );
                continue;
            }
        };
        let target = request
            .ssh_target
            .as_deref()
            .context("missing SSH target")?;
        let (stdout_path, stderr_path) = remote_log_paths(target, &meta.job_id);
        completions
            .push(Box::pin(monitor_remote(request, meta, stdout_path, stderr_path)) as Completion);
    }
    Ok(completions)
}

async fn launch_remote(
    target: &str,
    script: &str,
    env_vars: &HashMap<String, String>,
) -> Result<u32> {
    let mut child = ssh_command(target, env_vars)
        .args(["sh", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start SSH client")?;
    let mut stdin = child.stdin.take().context("open SSH stdin")?;
    stdin.write_all(script.as_bytes()).await?;
    stdin.shutdown().await?;
    drop(stdin);
    let output = child.wait_with_output().await?;
    if !output.status.success() {
        bail!(
            "SSH could not launch remote wrapper: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let pid = String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()
        .context("remote wrapper returned no PID")?
        .trim()
        .parse()
        .context("remote wrapper returned an invalid PID")?;
    Ok(pid)
}

async fn monitor_remote(
    request: JobSubmitRequest,
    mut meta: JobMeta,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
) -> Result<()> {
    if meta.exit_status.is_some() {
        return resume_remote(request, meta, stdout_path, stderr_path).await;
    }
    let target = request
        .ssh_target
        .as_deref()
        .context("missing SSH target")?;
    let exit_command = format!(
        "cat \"$HOME/.slumber/jobs/{}/exit_code\" 2>/dev/null",
        meta.job_id
    );
    loop {
        match ssh_command(target, &request.env_vars)
            .arg(&exit_command)
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let code: i32 = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse()
                    .context("remote exit_code is invalid")?;
                meta.finished_at = Some(now_secs());
                meta.exit_status = Some(status_record_from_code(code));
                write_meta(&meta)?;
                return resume_remote(request, meta, stdout_path, stderr_path).await;
            }
            _ => sleep(Duration::from_secs(5)).await,
        }
    }
}

async fn resume_remote(
    request: JobSubmitRequest,
    mut meta: JobMeta,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
) -> Result<()> {
    resumer::resume(&request, &meta, &stdout_path, &stderr_path).await?;
    clear_persisted_environment(&meta.job_id, &request)?;
    meta.resumed_at = Some(now_secs());
    write_meta(&meta)
}

fn ssh_command(target: &str, env_vars: &HashMap<String, String>) -> Command {
    let mut command = Command::new("ssh");
    command
        .env_clear()
        .envs(env_vars)
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
        .arg(target);
    command
}

fn validate_ssh_target(target: &str) -> Result<()> {
    if target.is_empty() || target.starts_with('-') || target.chars().any(char::is_whitespace) {
        bail!("invalid SSH destination: {target:?}");
    }
    Ok(())
}

fn remote_log_paths(target: &str, job_id: &str) -> (PathBuf, PathBuf) {
    let base = format!("{target}:~/.slumber/jobs/{job_id}");
    (
        PathBuf::from(format!("{base}/stdout.log")),
        PathBuf::from(format!("{base}/stderr.log")),
    )
}

fn remote_wrapper(job_id: &str, command: &str) -> String {
    let remote_dir = format!("$HOME/.slumber/jobs/{job_id}");
    let worker = format!(
        "sh -c {}\nstatus=$?\nprintf '%s\\n' \"$status\" > \"{remote_dir}/exit_code.tmp\"\nmv \"{remote_dir}/exit_code.tmp\" \"{remote_dir}/exit_code\"\nexit \"$status\"",
        shell_quote(command)
    );
    format!(
        "set -eu\njob_dir=\"{remote_dir}\"\nmkdir -p \"$job_dir\"\nchmod 700 \"$HOME/.slumber\" \"$HOME/.slumber/jobs\" \"$job_dir\"\nrm -f \"$job_dir/exit_code\" \"$job_dir/exit_code.tmp\"\n: > \"$job_dir/stdout.log\"\n: > \"$job_dir/stderr.log\"\nchmod 600 \"$job_dir/stdout.log\" \"$job_dir/stderr.log\"\nnohup sh -c {} > \"$job_dir/stdout.log\" 2> \"$job_dir/stderr.log\" < /dev/null &\nprintf '%s\\n' \"$!\"\n",
        shell_quote(&worker)
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn status_record(status: std::process::ExitStatus) -> ExitStatusRecord {
    if let Some(signal) = status.signal() {
        return ExitStatusRecord::Signaled(signal, signal_name(signal).to_owned());
    }
    status_record_from_code(status.code().unwrap_or(1))
}

fn status_record_from_code(code: i32) -> ExitStatusRecord {
    if (129..=192).contains(&code) {
        let signal = code - 128;
        ExitStatusRecord::Signaled(signal, signal_name(signal).to_owned())
    } else {
        ExitStatusRecord::Exited(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command as StdCommand, thread, time::SystemTime};

    #[test]
    fn records_shell_signal_exit() {
        let status = StdCommand::new("sh")
            .args(["-c", "exit 137"])
            .status()
            .unwrap();
        assert_eq!(
            status_record(status),
            ExitStatusRecord::Signaled(9, "SIGKILL (Potential OOM)".to_owned())
        );
    }

    #[test]
    fn remote_wrapper_detaches_and_records_exit() {
        let root = std::env::temp_dir().join(format!(
            "slumber-wrapper-{}-{:?}",
            std::process::id(),
            SystemTime::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let script = remote_wrapper(
            "job_test",
            "printf \"remote's stdout\"; printf 'remote stderr' >&2; exit 7",
        );
        let launch = StdCommand::new("sh")
            .args(["-c", &script])
            .env("HOME", &root)
            .output()
            .unwrap();
        assert!(launch.status.success());
        let job_dir = root.join(".slumber/jobs/job_test");
        for _ in 0..50 {
            if job_dir.join("exit_code").exists() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert_eq!(
            fs::read_to_string(job_dir.join("exit_code")).unwrap(),
            "7\n"
        );
        assert_eq!(
            fs::read_to_string(job_dir.join("stdout.log")).unwrap(),
            "remote's stdout"
        );
        assert_eq!(
            fs::read_to_string(job_dir.join("stderr.log")).unwrap(),
            "remote stderr"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
