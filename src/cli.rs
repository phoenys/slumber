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
const LEGACY_PROTOCOL: &str = r#"<!-- SLUMBER PROTOCOL -->
You have a background delegator: `slumber run "<command>"`.
RULE:
- If you need immediate output to write your next command -> run it directly.
- If your current work is done and you are ready to hand off execution and stop working until it finishes -> use `slumber run "<command>"` and exit.
"#;
const PROTOCOL: &str = r#"<!-- SLUMBER PROTOCOL -->
Use Slumber to run long tasks, not to schedule log checks.
- New task: submit the actual foreground command with `slumber run '<command>'`; do not launch it separately with nohup or `&`. Do short preflight checks directly before submission.
- Remote task: use `slumber run --ssh <host> '<command>'`, not `slumber run 'ssh ...'`.
- Already running: do not relaunch. Monitor a run-specific completion record and propagate its exit code; PID disappearance or log text alone does not establish success. Identify the original task logs separately from monitor output.
- When ready to hand off, submit once. After acceptance, report the job ID and log paths, then end your turn; do not poll or resubmit while waiting for the event.
- On the completion event, inspect the task's actual exit status, logs and results before continuing. Submission or notification success is not task success.
<!-- /SLUMBER PROTOCOL -->
"#;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Run a shell task in the background and notify an existing agent session",
    after_help = "AGENT HANDOFF:\n  Use run only when your current work is done; after successful submission,\n  stop working and wait for the completion event. Slumber does not stop the agent.\n  Submission success is not task success or successful wake-up. Check status.\n\nEXAMPLES:\n  slumber run 'cargo test --release'\n  slumber run --ssh gpu-box 'cd ~/project && python3 -u train.py'\n  slumber run --no-resume 'echo hello; sleep 2'\n  slumber status\n  slumber logs <job-id> --err\n\nDEFAULTS AND LIMITS:\n  Codex wake-up requires CODEX_THREAD_ID or CODEX_SESSION_ID (or --session-id),\n  a compatible codex queue command, and a logged-in session. Otherwise choose\n  --no-resume or a trusted --resume-template. Run `slumber run --help` for details.\n  Full stdout/stderr are written to files, not included in the completion event.\n  Keep the local daemon alive, including for SSH monitoring. State defaults to\n  ~/.slumber; request environments may contain credentials and are stored privately\n  until successful wake-up. Run `slumber doctor` to inspect prerequisites."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Submit one noninteractive shell command locally or over SSH; return a job ID.
    #[command(
        after_help = "EXECUTION:\n  The command is one quoted argument to sh -c, not your interactive login shell.\n  Local jobs use the submitting working directory and environment; stdin is closed.\n  SSH jobs use the remote login environment: put cd, environment activation, and\n  Python -u / PYTHONUNBUFFERED=1 inside the remote command when needed.\n  Keep work in the foreground, or explicitly wait for background children.\n\nOUTPUT AND WAKE-UP:\n  run returns after submission; status reports task and wake-up outcomes separately.\n  Logs are uncropped stdout/stderr files. Child buffering can delay writes; a crash\n  can lose unflushed output. The tmux viewer shows only the last 20 lines then follows.\n  Inside tmux, a log pane opens by default and closes before wake-up; --no-tail\n  disables only that viewer, never log capture.\n  --resume-template is trusted shell code, executed locally with the submitting\n  environment plus SLUMBER_SESSION_ID, SLUMBER_PAYLOAD, SLUMBER_PAYLOAD_PATH.\n  Failed hooks retain the environment for `slumber retry <job-id>`. Retry does not\n  rerun the original task. Completion delivery is not exactly-once."
    )]
    Run {
        /// Shell command to execute. Quote it as one argument.
        command: String,
        /// Agent session ID; defaults to CODEX_THREAD_ID or CODEX_SESSION_ID.
        #[arg(long)]
        session_id: Option<String>,
        /// Trusted local wake-up shell command, replacing the default Codex hook.
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
        /// Maximum number of completed jobs; unresolved jobs are always shown.
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Print the full current stdout log (or --err); fetch SSH logs remotely. Does not follow.
    Logs {
        /// Job ID printed by run or status.
        job_id: String,
        /// Read stderr instead of stdout; the streams are stored separately.
        #[arg(long)]
        err: bool,
    },
    /// Add the Slumber handoff protocol to an agent instructions file.
    Init {
        /// Append to this path, preserving existing content; conflicts with --agent.
        #[arg(long, conflicts_with = "agent")]
        file: Option<PathBuf>,
        /// Select AGENTS.md or CLAUDE.md; this does not install a wake-up adapter.
        #[arg(long, value_enum)]
        agent: Option<InitAgent>,
    },
    /// Check local prerequisites without submitting a job.
    Doctor,
    /// Install the latest release (or a selected version) beside the current binary.
    Update {
        /// Release tag; defaults to the latest stable release.
        #[arg(long, conflicts_with = "from")]
        version: Option<String>,
        /// Install an already downloaded binary, without network access.
        #[arg(long, conflicts_with = "github")]
        from: Option<PathBuf>,
        /// Download using authenticated gh (for private repositories).
        #[arg(long)]
        github: bool,
    },
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
    /// Start in the background if not already running.
    Start,
    /// Query the daemon without starting it.
    Status,
    /// Stop an idle daemon; refuse while jobs or wake-up hooks are active.
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
            Commands::Update {
                version,
                from,
                github,
            } => update(version, from, github),
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
    if let Some(warning) = response.tail_warning {
        eprintln!(
            "Warning: {warning}. Task submission succeeded; use `slumber logs {}` to inspect output.",
            response.job_id
        );
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
    let mut completed = 0;
    let jobs: Vec<_> = recent_jobs(usize::MAX)?
        .into_iter()
        .filter(|meta| {
            if meta.resumed_at.is_none() {
                true
            } else {
                completed += 1;
                completed <= limit
            }
        })
        .collect();
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
            if meta.exit_status.is_none() {
                println!("  monitoring: STOPPED: {error}");
            } else {
                println!("  wake-up: FAILED: {error}");
            }
        } else if meta.resumed_at.is_some() {
            println!("  wake-up: complete");
        } else if meta.exit_status.is_some() {
            println!("  wake-up: pending (saved state; query daemon status for live activity)");
        }
    }
    Ok(())
}

fn update(version: Option<String>, from: Option<PathBuf>, github: bool) -> Result<()> {
    let executable = env::current_exe()?;
    if executable.file_name().and_then(|name| name.to_str()) != Some("slumber") {
        bail!("self-update requires a binary named slumber; use install.sh --from instead");
    }
    let directory = executable
        .parent()
        .context("binary has no parent directory")?;
    let mut command = Command::new("sh");
    command
        .args(["-s", "--"])
        .env("SLUMBER_INSTALL_DIR", directory)
        .env_remove("SLUMBER_VERSION")
        .stdin(Stdio::piped());
    if let Some(version) = version {
        command.args(["--version", &version]);
    }
    if let Some(from) = from {
        command.arg("--from").arg(from);
    }
    if github {
        command.arg("--github");
    }
    let mut child = command.spawn().context("start embedded installer")?;
    let written = child
        .stdin
        .take()
        .context("open installer input")?
        .write_all(include_bytes!("../install.sh"));
    let status = child.wait()?;
    if !status.success() {
        bail!("update failed; see the installer error above");
    }
    written?;
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
    if existing.contains(LEGACY_PROTOCOL) {
        fs::write(&path, existing.replacen(LEGACY_PROTOCOL, PROTOCOL, 1))
            .with_context(|| format!("update Slumber protocol in {}", path.display()))?;
        println!("Updated Slumber protocol in {}", path.display());
        return Ok(());
    }
    if existing.contains(PROTOCOL_MARKER) {
        if existing.contains(PROTOCOL) {
            println!("Slumber protocol already present in {}", path.display());
        } else {
            println!(
                "Existing Slumber protocol in {} differs from the current contract; left unchanged. Review and replace it manually using the README contract.",
                path.display()
            );
        }
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
