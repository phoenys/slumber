# Slumber

Slumber is a local-first task handoff tool for coding agents. It sends a command to a lightweight local daemon, redirects the command's output directly to files, and resumes the originating agent with exit metadata when the command terminates.

## Build

```sh
cargo build --release
```

Slumber targets POSIX systems. Linux is the primary platform; macOS is supported on a best-effort basis.

## Use

Add the handoff protocol to the current project's agent instructions:

```sh
slumber init
```

Delegate one shell command and then end the agent session:

```sh
slumber run "cargo test --release"
```

`slumber run` uses `CODEX_THREAD_ID` or `CODEX_SESSION_ID` when available. An ID can also be supplied explicitly:

```sh
slumber run --session-id <id> "python train.py"
```

Codex wake-up uses `codex queue` to inject the completion event into the existing session. The external asynchronous wake-up flow has been verified with a detached 10-second task while preserving the working directory and conversation context.

For another agent, provide a shell resume command. Slumber exposes `SLUMBER_SESSION_ID`, `SLUMBER_PAYLOAD`, and `SLUMBER_PAYLOAD_PATH` to it:

```sh
slumber run --resume-template 'my-agent --resume "$SLUMBER_SESSION_ID" -p "$SLUMBER_PAYLOAD"' "make all"
```

Delegate to a remote machine already configured in OpenSSH without installing Slumber there:

```sh
slumber run --ssh gpu-box "cd ~/project && python train.py"
```

The local daemon injects a POSIX `nohup` wrapper over SSH. Remote output and the final exit code are stored under `~/.slumber/jobs/<job-id>/` on the target, while the local daemon polls for completion and wakes the local agent. SSH uses batch mode, so key-based authentication or an equivalent non-interactive OpenSSH configuration is required. If the local daemon restarts, it resumes polling unfinished remote jobs from local metadata.

Inspect recent state and logs:

```sh
slumber status
slumber logs <job-id>
slumber logs <job-id> --err
```

For remote jobs, `slumber logs` reads the selected log over SSH.

When `slumber run` is called inside tmux, it automatically opens a detached pane on the right that follows the last 20 lines of both stdout and stderr. The pane closes before the agent is resumed. Disable it for one task with `--no-tail`, or globally in `~/.slumber/config.toml`:

```toml
auto_tmux_tail = false
```

State lives under `~/.slumber`. The daemon socket uses `$XDG_RUNTIME_DIR/slumber/slumber.sock` when available and otherwise `~/.slumber/slumber.sock`. `SLUMBER_HOME` and `SLUMBER_SOCKET` can override these locations.
