use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const DAEMON_PROTOCOL_VERSION: &str = "4";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSubmitRequest {
    pub command: String,
    pub cwd: PathBuf,
    pub env_vars: HashMap<String, String>,
    pub session_id: Option<String>,
    pub agent_kind: AgentKind,
    #[serde(default)]
    pub ssh_target: Option<String>,
    #[serde(default)]
    pub tmux_pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentKind {
    CodeX,
    GenericCommand { resume_template: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobSubmitResponse {
    pub job_id: String,
    pub pgid: u32,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub message: String,
    #[serde(default)]
    pub tmux_pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMeta {
    pub job_id: String,
    pub command: String,
    pub cwd: PathBuf,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub pgid: Option<u32>,
    pub exit_status: Option<ExitStatusRecord>,
    #[serde(default)]
    pub ssh_target: Option<String>,
    #[serde(default)]
    pub resumed_at: Option<u64>,
    #[serde(default)]
    pub tmux_pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExitStatusRecord {
    Exited(i32),
    Signaled(i32, String),
    FailedToStart(String),
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn state_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("SLUMBER_HOME") {
        return Ok(PathBuf::from(path));
    }
    let home = env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".slumber"))
}

pub fn socket_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("SLUMBER_SOCKET") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(path).join("slumber/slumber.sock"));
    }
    Ok(state_dir()?.join("slumber.sock"))
}

pub fn jobs_dir() -> Result<PathBuf> {
    Ok(state_dir()?.join("jobs"))
}

pub fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub fn job_dir(job_id: &str) -> Result<PathBuf> {
    Ok(jobs_dir()?.join(job_id))
}

pub fn write_meta(meta: &JobMeta) -> Result<()> {
    let path = job_dir(&meta.job_id)?.join("meta.json");
    let file = File::create(&path).with_context(|| format!("create {}", path.display()))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer_pretty(file, meta).context("serialize job metadata")?;
    Ok(())
}

pub fn write_request(job_id: &str, request: &JobSubmitRequest) -> Result<()> {
    let path = job_dir(job_id)?.join("request.json");
    let file = File::create(&path).with_context(|| format!("create {}", path.display()))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer_pretty(file, request).context("serialize job request")?;
    Ok(())
}

pub fn read_request(path: &Path) -> Result<JobSubmitRequest> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    serde_json::from_reader(file).with_context(|| format!("parse {}", path.display()))
}

pub fn read_meta(path: &Path) -> Result<JobMeta> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    serde_json::from_reader(file).with_context(|| format!("parse {}", path.display()))
}

pub fn recent_jobs(limit: usize) -> Result<Vec<JobMeta>> {
    let jobs = jobs_dir()?;
    if !jobs.exists() {
        return Ok(Vec::new());
    }

    let mut metas = Vec::new();
    for entry in fs::read_dir(jobs)? {
        let path = entry?.path().join("meta.json");
        if path.is_file() {
            match read_meta(&path) {
                Ok(meta) => metas.push(meta),
                Err(error) => eprintln!("slumber: ignoring {}: {error:#}", path.display()),
            }
        }
    }
    metas.sort_by_key(|meta| std::cmp::Reverse(meta.created_at));
    metas.truncate(limit);
    Ok(metas)
}

pub fn exit_status_text(status: &ExitStatusRecord) -> String {
    match status {
        ExitStatusRecord::Exited(code) => format!("exit {code}"),
        ExitStatusRecord::Signaled(signal, name) => {
            format!("{} (signal {signal}: {name})", 128 + signal)
        }
        ExitStatusRecord::FailedToStart(message) => format!("failed to start: {message}"),
    }
}

pub fn signal_name(signal: i32) -> &'static str {
    match signal {
        2 => "SIGINT (Interrupted)",
        9 => "SIGKILL (Potential OOM)",
        11 => "SIGSEGV (Segmentation Fault)",
        15 => "SIGTERM (Terminated)",
        _ => "UNKNOWN_SIGNAL",
    }
}
