use crate::{
    core::{
        DAEMON_PROTOCOL_VERSION, JobSubmitRequest, create_private_dir, jobs_dir, socket_path,
        state_dir,
    },
    supervisor,
};
use anyhow::{Context, Result};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};

pub async fn serve(detached: bool) -> Result<()> {
    let socket = socket_path()?;
    let parent = socket.parent().context("socket path has no parent")?;
    create_private_dir(parent)?;
    create_private_dir(&state_dir()?)?;
    create_private_dir(&jobs_dir()?)?;

    if socket.exists() {
        if UnixStream::connect(&socket).await.is_ok() {
            if !detached {
                eprintln!("slumber daemon is already running at {}", socket.display());
            }
            return Ok(());
        }
        fs::remove_file(&socket)?;
    }

    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("bind daemon socket at {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let daemon_pid = std::process::id().to_string();
    fs::write(state_dir()?.join("slumberd.pid"), &daemon_pid)?;
    fs::write(
        state_dir()?.join("slumberd.protocol"),
        format!("{DAEMON_PROTOCOL_VERSION}:{daemon_pid}"),
    )?;
    if !detached {
        eprintln!("slumber daemon listening at {}", socket.display());
    }

    for completion in supervisor::recover_remote_jobs()? {
        tokio::spawn(async move {
            if let Err(error) = completion.await {
                eprintln!("slumberd remote recovery: {error:#}");
            }
        });
    }

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream).await {
                eprintln!("slumberd: {error:#}");
            }
        });
    }
}

async fn handle_connection(stream: UnixStream) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut line = String::new();
    BufReader::new(reader).read_line(&mut line).await?;
    if line.trim().is_empty() {
        return Ok(());
    }

    let request: JobSubmitRequest = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(error) => {
            let response = json!({"error": format!("invalid request: {error}")});
            writer
                .write_all(serde_json::to_string(&response)?.as_bytes())
                .await?;
            writer.write_all(b"\n").await?;
            return Ok(());
        }
    };

    match supervisor::start(request).await {
        Ok((response, completion)) => {
            writer
                .write_all(serde_json::to_string(&response)?.as_bytes())
                .await?;
            writer.write_all(b"\n").await?;
            tokio::spawn(async move {
                if let Err(error) = completion.await {
                    eprintln!("slumberd supervisor: {error:#}");
                }
            });
        }
        Err(error) => {
            let response = json!({"error": format!("{error:#}")});
            writer
                .write_all(serde_json::to_string(&response)?.as_bytes())
                .await?;
            writer.write_all(b"\n").await?;
        }
    }
    Ok(())
}
