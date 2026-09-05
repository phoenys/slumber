# Slumber

Hand off a long shell command, let your coding agent stop working, and send its existing session a compact completion event. Slumber runs a small local daemon, writes stdout/stderr directly to files, and can execute a job over an existing SSH connection without installing Slumber remotely.

**Release candidate scope:** one local machine and one SSH server, with Codex wake-up. Multi-server orchestration, clusters/SLURM, Windows, and built-in adapters for other agents are future work. See [release readiness](RELEASING.md) for actual verification status and release gates.

## Install

After the first public release is published:

```sh
curl -fsSL https://raw.githubusercontent.com/phoenys/slumber/main/install.sh | sh
```

The installer downloads a binary into `~/.local/bin/slumber`, uses no sudo, and never edits shell profiles or dotfiles. If that directory is not in PATH, add it for the current terminal:

```sh
export PATH="$HOME/.local/bin:$PATH"
slumber doctor
```

For a specific release, use `sh -s -- --version v0.1.0` after the pipe. `SLUMBER_INSTALL_DIR` selects another absolute installation directory. Read the script before running it if you prefer; binaries are also available from [GitHub Releases](https://github.com/phoenys/slumber/releases).

The intended binary matrix is macOS 15+ and Ubuntu 24.04+ (glibc 2.39+), each on Apple Silicon/ARM64 and x86_64. Older Linux distributions and musl/Alpine are not binary compatibility targets. CI must pass on all four before publishing a release. macOS binaries are currently unsigned and not notarized; follow your organization's software trust policy.

Before a binary release, build from a checkout with Rust 1.89 or later and a native C linker (Xcode command-line tools on macOS; build-essential on Ubuntu):

```sh
cargo build --release --locked
sh install.sh --from "$PWD/target/release/slumber"
```

## First run — no agent account required

```sh
slumber run --no-resume 'echo hello; echo diagnostic >&2; sleep 2'
slumber status
slumber logs <job-id>
slumber logs <job-id> --err
slumber daemon stop
```

Use the job ID printed by `run`. `--no-resume` explicitly disables wake-up. A running job automatically starts the daemon; it does not require a system service.

## Connect your agent

In your project, choose the instruction file explicitly, or run `slumber init` in a terminal for a prompt:

```sh
slumber init --agent codex           # appends to AGENTS.md
slumber init --file my-instructions.md
```

`--agent claude` selects `CLAUDE.md` for the handoff instructions only; it does not configure a tested Claude wake-up adapter. Existing contents are preserved and repeating init is safe. Only this explicit init step edits project instruction files.

Codex must provide `codex queue --thread … --message …`. Run `slumber doctor` to check that capability; the command may not be present in every Codex distribution. A successful capability check does not establish authentication or prove the target session can receive events. Confirm a real wake-up before delegating unattended work.

From an active Codex session:

```sh
slumber run 'cargo test --release'
```

Slumber reads `CODEX_THREAD_ID` or `CODEX_SESSION_ID`, or accepts `--session-id <id>`. With no session or explicit wake-up mode, it fails before starting a task. The agent should hand off only after finishing its current work and stop until the event arrives; Slumber does not terminate the agent itself. An exit code of zero from the queue command confirms command delivery, not the agent's subsequent reasoning or actions.

A custom integration can use `--resume-template`. It receives `SLUMBER_SESSION_ID`, `SLUMBER_PAYLOAD`, and `SLUMBER_PAYLOAD_PATH` in addition to the submitting environment. Templates are trusted shell code, and Slumber waits for them to exit:

```sh
slumber run --resume-template 'printf "%s\n" "$SLUMBER_PAYLOAD" >> "$HOME/task-events.log"' 'make all'
```

## One SSH server

First establish key-based authentication and verify the host key interactively:

```sh
ssh gpu-box true
slumber run --ssh gpu-box 'cd ~/project && python train.py'
```

SSH batch mode must succeed without a password prompt. The remote account needs POSIX `sh`, `nohup`, and standard file utilities. Commands run in the remote login environment, so specify `cd`, virtual-environment activation, or executable paths in the command itself. Local environment variables are not copied to the remote shell.

Remote stdout, stderr and exit status live under `~/.slumber/jobs/<job-id>/` on the server. The local daemon checks for completion every five seconds, then invokes the local agent. Remote monitoring resumes after a daemon restart using the original request environment. Host-key errors, expired credentials, or network failure delay monitoring until resolved; SSH keepalives detect dead connections. Avoid repeating a submission after an ambiguous network error without inspecting the remote job directory: the remote job may already have started.

## Logs, failures and daemon lifecycle

```sh
slumber status                     # task outcome and wake-up errors
slumber logs <job-id> --err
slumber retry <job-id>              # retry a completed task's failed wake-up
slumber doctor
slumber daemon status
slumber daemon start                # explicitly start in the background
slumber daemon stop                 # refuse while jobs/wake-ups are active
slumber daemon                     # run in this terminal, in the foreground
```

Wake-up failures are recorded in `meta.json`, `resume.log` and the daemon log. Inspect the error, correct the cause, and retry. Retry uses the original environment; changed credentials require a new submission or a deliberately updated private snapshot. Automatic replay after a crash can deliver a duplicate event; exactly-once delivery is not guaranteed.

Inside tmux, Slumber finds the submitting pane using `TMUX_PANE`. It compares the pane's character dimensions: a wide pane splits left/right, a tall pane splits top/bottom. The detached pane follows the last 20 lines of **both** stdout and stderr and closes before wake-up. `--no-tail` disables it for one job. In `~/.slumber/config.toml`, this disables it globally:

```toml
auto_tmux_tail = false
```

This configuration currently supports only this boolean setting. Outside tmux, tasks run silently in the background.

## Data and safety

State defaults to `~/.slumber`. The socket uses `$XDG_RUNTIME_DIR/slumber/slumber.sock` when set, otherwise `~/.slumber/slumber.sock`. `SLUMBER_HOME` and `SLUMBER_SOCKET` override these paths. Choose absolute, private directories owned by your account, mode `0700`; Slumber refuses unsafe directories instead of changing their permissions.

Job directories are `0700` and private files are `0600`. **Full environment snapshots, including credentials, temporarily reside in `request.json`.** They are cleared after a successful wake-up. Failed wake-ups retain them for retry; logs and commands may independently contain secrets. Do not upload your state directory, include it in public bug reports, or run untrusted templates. Slumber has no telemetry or remote service. See [SECURITY.md](SECURITY.md).

Keep the local daemon running while local jobs execute. Local process state cannot be recovered after a daemon crash/reboot: such tasks are reported as unknown and require manual inspection. The supported stop command refuses active work. Process-group cleanup handles ordinary local descendants, but is not containment for processes that detach themselves; the remote POSIX wrapper does not provide process-tree cleanup. A shell exit in the 129–192 range is reported as a possible signal (137 often means SIGKILL), not proof of OOM. Logs have no automatic size limit or retention policy.

## Upgrade, uninstall and cleanup

Wait for jobs and wake-ups to finish before upgrading. The installer stops an idle existing daemon, then replaces its binary. When migrating from an early MVP daemon without `daemon stop`, finish its jobs and stop that old daemon manually before upgrading. Do not kill it during a local job.

```sh
# After scripts are publicly available:
curl -fsSL https://raw.githubusercontent.com/phoenys/slumber/main/uninstall.sh | sh
# Also permanently delete local configuration, credentials and logs (asks for confirmation):
curl -fsSL https://raw.githubusercontent.com/phoenys/slumber/main/uninstall.sh | sh -s -- --purge
```

For automation, use `--purge --yes`. The same scripts can be run from a local checkout. Purge requires the Slumber state marker and refuses broad directories and symlinks; legacy/unmarked state requires manual inspection. It never edits project instruction files or removes remote server logs. You can remove the marked Slumber paragraph from a project's instruction file manually, and remove remote job directories after reviewing their contents.

For development, clean-machine verification, rollback, and the release checklist, see [CONTRIBUTING.md](CONTRIBUTING.md) and [RELEASING.md](RELEASING.md).
