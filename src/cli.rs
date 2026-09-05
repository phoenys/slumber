use crate::{
    core::{
        AgentKind, DAEMON_PROTOCOL_VERSION, DaemonRequest, JobSubmitRequest, JobSubmitResponse,
        create_private_dir, exit_status_text, job_dir, recent_jobs, socket_path, state_dir,
    },
    daemon,
};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value;
use std::{
    collections::HashMap,
    env,
    fs::{self, OpenOptions},
    io::{IsTerminal, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time::{sleep, timeout},
};

const PROTOCOL_MARKER: &str = "<!-- SLUMBER PROTOCOL -->";
const PROTOCOL: &str = r#"<!-- SLUMBER PROTOCOL -->
You have a background delegator: `slumber run "<command>"`.
RULE:
- If you need immediate output to write your next command -> run it directly.
- If your current work is done and you are ready to hand off execution and stop working until it finishes -> use `slumber run "<command>"` and exit.
"#;

#[derive(Debug, Parser)]
#[command(version, about = "Zero-token task handoff for coding agents")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Delegate a command to the local daemon.
    Run {
        /// Shell command to execute. Quote it as one argument.
        command: String,
        /// Agent session ID; defaults to CODEX_THREAD_ID or CODEX_SESSION_ID.
        #[arg(long)]
        session_id: Option<String>,
        /// Shell command used to resume a non-Codex agent.
        #[arg(long)]
        resume_template: Option<String>,
        /// OpenSSH destination for agentless remote execution.
        #[arg(long, value_name = "DESTINATION")]
        ssh: Option<String>,
        /// Do not open a tmux pane that follows task logs.
        #[arg(long)]
        no_tail: bool,
        /// Delegate without waking an agent (for standalone shell use).
        #[arg(long, conflicts_with_all = ["session_id", "resume_template"])]
        no_resume: bool,
    },
    /// Show running and recently completed jobs.
    Status {
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Print a job's stdout or stderr log.
    Logs {
        job_id: String,
        #[arg(long)]
        err: bool,
    },
    /// Add the Slumber handoff protocol to an agent instructions file.
    Init {
        #[arg(long, conflicts_with = "agent")]
        file: Option<PathBuf>,
        #[arg(long, value_enum)]
        agent: Option<InitAgent>,
    },
    /// Check local prerequisites without submitting a job.
    Doctor,
    /// Retry a completed job's failed wake-up command.
    Retry { job_id: String },
    /// Run in the foreground, or explicitly manage the background daemon.
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum InitAgent {
    Codex,
    Claude,
}

#[derive(Debug, Subcommand)]
enum DaemonAction {
    Start,
    Status,
    Stop,
}

impl Cli {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            Commands::Run {
                command,
                session_id,
                resume_template,
                ssh,
                no_tail,
                no_resume,
            } => {
                run(
                    command,
                    session_id,
                    resume_template,
                    ssh,
                    no_tail,
                    no_resume,
                )
                .await
            }
            Commands::Status { limit } => status(limit),
            Commands::Logs { job_id, err } => logs(&job_id, err),
            Commands::Init { file, agent } => init(file.as_deref(), agent),
            Commands::Doctor => doctor().await,
            Commands::Retry { job_id } => {
                job_dir(&job_id)?;
                ensure_daemon().await?;
                println!(
                    "{}",
                    exchange(&DaemonRequest::Retry { job_id }).await?["message"]
                        .as_str()
                        .unwrap_or_default()
                );
                Ok(())
            }
            Commands::Daemon { action: None } => daemon::serve().await,
            Commands::Daemon {
                action: Some(action),
            } => manage_daemon(action).await,
        }
    }
}

async fn run(
    command: String,
    session_id: Option<String>,
    resume_template: Option<String>,
    ssh_target: Option<String>,
    no_tail: bool,
    no_resume: bool,
) -> Result<()> {
    if command.trim().is_empty() {
        bail!("command must not be empty");
    }

    let session_id = session_id
        .or_else(detected_session_id)
        .filter(|id| !id.trim().is_empty());
    if !no_resume && resume_template.is_none() && session_id.is_none() {
        bail!(
            "no agent session detected; use --session-id, --resume-template, or --no-resume for standalone use"
        );
    }
    let tmux_tail = tmux_tail_enabled(no_tail)?;
    ensure_daemon().await?;
    let protocol_path = state_dir()?.join("slumberd.protocol");
    let protocol = fs::read_to_string(&protocol_path).unwrap_or_default();
    let daemon_pid = fs::read_to_string(state_dir()?.join("slumberd.pid")).unwrap_or_default();
    if protocol.trim() != format!("{DAEMON_PROTOCOL_VERSION}:{}", daemon_pid.trim()) {
        bail!("running daemon uses an incompatible protocol; restart it and retry");
    }
    let cwd = env::current_dir()?.canonicalize()?;
    let agent_kind = if no_resume {
        AgentKind::NoResume
    } else {
        match resume_template {
            Some(resume_template) => AgentKind::GenericCommand { resume_template },
            None => AgentKind::CodeX,
        }
    };
    let request = JobSubmitRequest {
        command,
        cwd,
        env_vars: env::vars().collect::<HashMap<_, _>>(),
        session_id,
        agent_kind,
        ssh_target,
        tmux_pane_id: tmux_tail.then(String::new),
    };
    let response: JobSubmitResponse =
        serde_json::from_value(exchange(&DaemonRequest::Submit(request)).await?)?;

    println!("Submitted {} (process {})", response.job_id, response.pgid);
    println!("stdout: {}", response.stdout_path.display());
    println!("stderr: {}", response.stderr_path.display());
    if let Some(pane_id) = response.tmux_pane_id {
        println!("tail pane: {pane_id}");
    }
    println!("{}", response.message);
    Ok(())
}

fn tmux_tail_enabled(no_tail: bool) -> Result<bool> {
    if no_tail || env::var_os("TMUX").is_none() {
        return Ok(false);
    }
    let config_path = state_dir()?.join("config.toml");
    if !config_path.exists() {
        return Ok(true);
    }
    let config = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    for line in config.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "auto_tmux_tail" {
            return match value.trim() {
                "true" => Ok(true),
                "false" => Ok(false),
                other => bail!("auto_tmux_tail must be true or false, got {other:?}"),
            };
        }
    }
    Ok(true)
}

fn detected_session_id() -> Option<String> {
    env::var("CODEX_THREAD_ID")
        .ok()
        .or_else(|| env::var("CODEX_SESSION_ID").ok())
}

async fn exchange(request: &DaemonRequest) -> Result<Value> {
    let socket = socket_path()?;
    let mut stream = UnixStream::connect(&socket)
        .await
        .with_context(|| format!("connect to daemon at {}", socket.display()))?;
    validate_daemon_identity(&socket, &stream)?;
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    if line.len() > 1024 * 1024 {
        bail!("request exceeds 1 MiB");
    }
    stream.write_all(&line).await?;

    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).await?;
    if response.is_empty() {
        bail!("daemon closed the connection without a response");
    }
    let value: Value = serde_json::from_str(&response).context("parse daemon response")?;
    if let Some(message) = value.get("error").and_then(Value::as_str) {
        bail!("daemon rejected job: {message}");
    }
    Ok(value)
}

fn validate_daemon_identity(socket: &Path, stream: &UnixStream) -> Result<()> {
    let metadata = fs::metadata(socket)
        .with_context(|| format!("inspect daemon socket at {}", socket.display()))?;
    if !metadata.file_type().is_socket() {
        bail!("daemon path is not a Unix socket: {}", socket.display());
    }

    let expected_uid = nix::unistd::geteuid().as_raw();
    let peer_uid = stream
        .peer_cred()
        .context("inspect daemon peer credentials")?
        .uid();
    if metadata.uid() != expected_uid || peer_uid != expected_uid {
        bail!(
            "refusing untrusted daemon at {}: expected UID {expected_uid}, socket UID {}, peer UID {peer_uid}",
            socket.display(),
            metadata.uid()
        );
    }
    Ok(())
}

async fn ensure_daemon() -> Result<()> {
    let socket = socket_path()?;
    if let Ok(stream) = UnixStream::connect(&socket).await {
        validate_daemon_identity(&socket, &stream)?;
        return Ok(());
    }

    let state = state_dir()?;
    create_private_dir(&state)?;
    create_private_dir(socket.parent().context("socket path has no parent")?)?;
    let log_path = state.join("slumberd.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;
    let executable = env::current_exe()?;
    let mut child = Command::new(executable);
    child
        .arg("daemon")
        .current_dir(&state)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    unsafe {
        child.pre_exec(|| {
            nix::unistd::setsid().map_err(std::io::Error::other)?;
            Ok(())
        });
    }
    child.spawn().context("start slumber daemon")?;

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut delay = Duration::from_millis(25);
    while Instant::now() < deadline {
        if exchange(&DaemonRequest::Ping).await.is_ok() {
            return Ok(());
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_millis(250));
    }
    bail!(
        "daemon did not start within 2 seconds; inspect {}",
        log_path.display()
    )
}

async fn manage_daemon(action: DaemonAction) -> Result<()> {
    if matches!(action, DaemonAction::Start) {
        ensure_daemon().await?;
        let response = exchange(&DaemonRequest::Ping).await?;
        println!(
            "Daemon running (protocol {}, active {}).",
            response["protocol"], response["active"]
        );
        return Ok(());
    }
    match UnixStream::connect(socket_path()?).await {
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            println!("Daemon is not running.");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let response = exchange(&match action {
        DaemonAction::Stop => DaemonRequest::Stop,
        _ => DaemonRequest::Ping,
    })
    .await?;
    if matches!(action, DaemonAction::Stop) {
        for _ in 0..100 {
            if !socket_path()?.exists() {
                println!("Daemon stopped.");
                return Ok(());
            }
            sleep(Duration::from_millis(20)).await;
        }
        bail!("daemon acknowledged stop but has not exited yet");
    }
    println!(
        "Daemon running (protocol {}, active {}).",
        response["protocol"], response["active"]
    );
    Ok(())
}

async fn doctor() -> Result<()> {
    println!(
        "Slumber {} — {} / {}",
        env!("CARGO_PKG_VERSION"),
        env::consts::OS,
        env::consts::ARCH
    );
    println!("State: {}", state_dir()?.display());
    println!("Socket: {}", socket_path()?.display());
    for (program, args) in [
        ("sh", vec!["-c", "exit 0"]),
        ("ssh", vec!["-V"]),
        ("tmux", vec!["-V"]),
        ("codex", vec!["queue", "--help"]),
    ] {
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let available = matches!(timeout(Duration::from_secs(5), command.status()).await, Ok(Ok(status)) if status.success());
        println!(
            "{program}: {}",
            if available {
                "available"
            } else {
                "missing or incompatible"
            }
        );
    }
    println!(
        "Codex requires a build with `codex queue` plus a valid logged-in session. SSH and tmux are optional."
    );
    manage_daemon(DaemonAction::Status).await
}

fn status(limit: usize) -> Result<()> {
    let jobs = recent_jobs(limit)?;
    if jobs.is_empty() {
        println!("No jobs.");
        return Ok(());
    }

    println!(
        "{:<30} {:<10} {:<18} {:<10} COMMAND",
        "JOB", "PID", "TARGET", "STATUS"
    );
    for meta in jobs {
        let status = meta
            .exit_status
            .as_ref()
            .map(exit_status_text)
            .unwrap_or_else(|| {
                if meta.resume_error.is_some() {
                    "unknown".to_owned()
                } else {
                    "running".to_owned()
                }
            });
        let pgid = meta
            .pgid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned());
        let target = meta.ssh_target.as_deref().unwrap_or("local");
        println!(
            "{:<30} {:<10} {:<18} {:<10} {}",
            meta.job_id, pgid, target, status, meta.command
        );
        if let Some(error) = meta.resume_error {
            println!("  wake-up: FAILED: {error}");
        } else if meta.resumed_at.is_some() {
            println!("  wake-up: complete");
        }
    }
    Ok(())
}

fn logs(job_id: &str, err: bool) -> Result<()> {
    if job_id.contains('/') || job_id == "." || job_id == ".." {
        bail!("invalid job id");
    }
    let name = if err { "stderr.log" } else { "stdout.log" };
    let meta_path = job_dir(job_id)?.join("meta.json");
    let meta = crate::core::read_meta(&meta_path)?;
    if let Some(target) = meta.ssh_target {
        let remote_command = format!("cat \"$HOME/.slumber/jobs/{job_id}/{name}\"");
        let status = Command::new("ssh")
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
            .arg(target)
            .arg(remote_command)
            .status()
            .context("read remote job log over SSH")?;
        if !status.success() {
            bail!("SSH failed while reading remote job log");
        }
        return Ok(());
    }
    let path = job_dir(job_id)?.join(name);
    let mut file = fs::File::open(&path).with_context(|| format!("read {}", path.display()))?;
    std::io::copy(&mut file, &mut std::io::stdout())?;
    Ok(())
}

fn init(file: Option<&Path>, agent: Option<InitAgent>) -> Result<()> {
    let path = match (file, agent) {
        (Some(path), _) => path.to_owned(),
        (_, Some(InitAgent::Codex)) => PathBuf::from("AGENTS.md"),
        (_, Some(InitAgent::Claude)) => PathBuf::from("CLAUDE.md"),
        _ => {
            if !std::io::stdin().is_terminal() {
                bail!("choose --agent codex, --agent claude, or --file <path>");
            }
            print!("Agent (codex/claude), or instructions file path: ");
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            match input.trim() {
                "codex" => PathBuf::from("AGENTS.md"),
                "claude" => PathBuf::from("CLAUDE.md"),
                "" => bail!("no agent or path selected"),
                path => PathBuf::from(path),
            }
        }
    };
    let existing = fs::read_to_string(&path).unwrap_or_default();
    if existing.contains(PROTOCOL_MARKER) {
        println!("Slumber protocol already present in {}", path.display());
        return Ok(());
    }

    let mut output = OpenOptions::new().create(true).append(true).open(&path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(output)?;
    }
    if !existing.is_empty() {
        writeln!(output)?;
    }
    write!(output, "{PROTOCOL}")?;
    println!("Added Slumber protocol to {}", path.display());
    Ok(())
}
