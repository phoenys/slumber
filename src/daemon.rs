use crate::{
    core::{
        DAEMON_PROTOCOL_VERSION, DaemonRequest, create_private_dir, jobs_dir, socket_path,
        state_dir, write_private_atomic,
    },
    supervisor,
};
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Notify,
    time::{Duration, timeout},
};

#[derive(Default)]
struct State {
    active: AtomicUsize,
    stopping: AtomicBool,
    stop: Notify,
}

pub async fn serve() -> Result<()> {
    let socket = socket_path()?;
    let state_dir = state_dir()?;
    create_private_dir(&state_dir)?;
    create_private_dir(&jobs_dir()?)?;
    create_private_dir(socket.parent().context("socket path has no parent")?)?;
    // The OS releases this startup/recovery lock even after a crash.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(state_dir.join("daemon.lock"))?;
    lock.try_lock()
        .context("another daemon is running for this state directory")?;
    write_private_atomic(&state_dir.join(".slumber-state"), b"slumber-state-v1\n")?;
    if let Ok(metadata) = fs::symlink_metadata(&socket) {
        if !metadata.file_type().is_socket() {
            bail!("refusing to remove non-socket {}", socket.display());
        }
        if UnixStream::connect(&socket).await.is_ok() {
            bail!("another daemon is listening at {}", socket.display());
        }
        fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("bind daemon socket at {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let pid = std::process::id().to_string();
    write_private_atomic(&state_dir.join("slumberd.pid"), pid.as_bytes())?;
    write_private_atomic(
        &state_dir.join("slumberd.protocol"),
        format!("{DAEMON_PROTOCOL_VERSION}:{pid}").as_bytes(),
    )?;
    eprintln!("slumber daemon listening at {}", socket.display());
    let state = Arc::new(State::default());
    for completion in supervisor::recover_jobs()? {
        spawn_completion(completion, state.clone());
    }
    loop {
        tokio::select! {
            _ = state.stop.notified() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, state).await {
                        eprintln!("slumberd: {error:#}");
                    }
                });
            }
        }
    }
    fs::remove_file(&socket)?;
    fs::remove_file(state_dir.join("slumberd.pid"))?;
    fs::remove_file(state_dir.join("slumberd.protocol"))?;
    Ok(())
}

fn spawn_completion(completion: supervisor::Completion, state: Arc<State>) {
    state.active.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        if let Err(error) = completion.await {
            eprintln!("slumberd supervisor: {error:#}");
        }
        state.active.fetch_sub(1, Ordering::SeqCst);
    });
}

async fn handle_connection(stream: UnixStream, state: Arc<State>) -> Result<()> {
    if stream.peer_cred()?.uid() != nix::unistd::geteuid().as_raw() {
        bail!("refusing client with a different UID");
    }
    let (reader, mut writer) = stream.into_split();
    let mut line = String::new();
    timeout(
        Duration::from_secs(5),
        BufReader::new(reader)
            .take(1024 * 1024 + 1)
            .read_line(&mut line),
    )
    .await??;
    if line.is_empty() {
        return Ok(());
    }
    if line.len() > 1024 * 1024 || !line.ends_with('\n') {
        bail!("request exceeds 1 MiB or is incomplete");
    }
    let request = serde_json::from_str::<DaemonRequest>(&line);
    let mut stop = false;
    let response = match request {
        Err(_) => json!({"error": "invalid daemon request"}),
        Ok(DaemonRequest::Ping) => {
            json!({"protocol": DAEMON_PROTOCOL_VERSION, "active": state.active.load(Ordering::SeqCst)})
        }
        Ok(DaemonRequest::Stop) => {
            if state.active.load(Ordering::SeqCst) != 0 {
                json!({"error": "jobs or wake-up commands are still active; wait for them to finish before stopping"})
            } else {
                state.stopping.store(true, Ordering::SeqCst);
                stop = true;
                json!({"message": "Daemon stopped."})
            }
        }
        Ok(_) if state.stopping.load(Ordering::SeqCst) => json!({"error": "daemon is stopping"}),
        Ok(DaemonRequest::Submit(request)) => {
            state.active.fetch_add(1, Ordering::SeqCst);
            let started = supervisor::start(request).await;
            let response = match started {
                Ok((response, completion)) => {
                    // Supervise even when the submitting client disconnects.
                    spawn_completion(completion, state.clone());
                    serde_json::to_value(response)?
                }
                Err(error) => json!({"error": format!("{error:#}")}),
            };
            state.active.fetch_sub(1, Ordering::SeqCst);
            response
        }
        Ok(DaemonRequest::Retry { job_id }) => match supervisor::retry_resume(&job_id) {
            Ok(completion) => {
                spawn_completion(completion, state.clone());
                json!({"message": "Wake-up retry started; inspect slumber status and resume.log."})
            }
            Err(error) => json!({"error": format!("{error:#}")}),
        },
    };
    let written = writer.write_all(format!("{response}\n").as_bytes()).await;
    if stop {
        state.stop.notify_one();
    }
    written?;
    Ok(())
}
