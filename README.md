# Slumber

Hand off a long shell command, let your coding agent stop working, and send its existing session a compact completion event. Slumber runs a small local daemon, writes stdout/stderr directly to files, and can execute a job over an existing SSH connection without installing Slumber remotely.

**Initial release scope:** one local machine and one SSH server, with Codex wake-up. Multi-server orchestration, clusters/SLURM, Windows, and built-in adapters for other agents are future work. See [release readiness](RELEASING.md) for verification requirements and release gates.

## Install

While the repository is private, download the binary for your platform from [Releases](https://github.com/phoenys/slumber/releases) using an authorized GitHub account, then install from your checkout:

```sh
gh release download v0.1.1 --repo phoenys/slumber --pattern slumber-aarch64-apple-darwin
sh install.sh --from "$PWD/slumber-aarch64-apple-darwin"
```

The example is for Apple Silicon; select the matching asset for another platform. It requires a published release, not a draft. Anonymous installation is available only after the repository and release are public:

```sh
curl -fsSL https://raw.githubusercontent.com/phoenys/slumber/main/install.sh | sh
```

The installer downloads a binary into `~/.local/bin/slumber`, uses no sudo, and never edits shell profiles or dotfiles. If that directory is not in PATH, add it for the current terminal:

```sh
export PATH="$HOME/.local/bin:$PATH"
slumber doctor
```

For a specific release, use `sh -s -- --version v0.1.1` after the pipe. `SLUMBER_INSTALL_DIR` selects another absolute installation directory. Read the script before running it if you prefer; binaries are also available from [GitHub Releases](https://github.com/phoenys/slumber/releases).

If the installer reports **No published binary found**, check that Releases contains a published release with the binary for your platform. Making the repository public or pushing a tag does not publish draft assets. The default `latest` URL requires a stable release; a published prerelease needs `--version <tag>`. Until assets are published, use the checkout build and `--from` instructions below. The installer does not silently install a compiler or build from source.

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

`--agent claude` selects `CLAUDE.md` for the handoff instructions only; it does not configure a tested Claude wake-up adapter. Repeating init is safe: it upgrades the exact original generated contract while preserving surrounding instructions, and leaves customized contracts unchanged with a review notice. Only this explicit init step edits project instruction files.

Here, **foreground** means that the submitted command stays alive until its work finishes, not that it remains attached to your terminal. Slumber owns backgrounding: it starts its background daemon in a new session with redirected standard streams, and its remote wrapper uses `nohup` with redirected streams internally. The agent should not add another detached layer that prevents Slumber from observing the real command's completion.

The generated handoff contract is:

```markdown
<!-- SLUMBER PROTOCOL -->
Use Slumber to run long tasks, not to schedule log checks.
- New task: submit the actual foreground command with `slumber run '<command>'`; do not launch it separately with nohup or `&`. Do short preflight checks directly before submission.
- Remote task: use `slumber run --ssh <host> '<command>'`, not `slumber run 'ssh ...'`.
- Already running: do not relaunch. Monitor a run-specific completion record and propagate its exit code; PID disappearance or log text alone does not establish success. Identify the original task logs separately from monitor output.
- When ready to hand off, submit once. After acceptance, report the job ID and log paths, then end your turn; do not poll or resubmit while waiting for the event.
- On the completion event, inspect the task's actual exit status, logs and results before continuing. Submission or notification success is not task success.
<!-- /SLUMBER PROTOCOL -->
```

Codex must provide `codex queue --thread … --message …`. Run `slumber doctor` to check that capability; the command may not be present in every Codex distribution. A successful capability check does not establish authentication or prove the target session can receive events. Confirm a real wake-up before delegating unattended work.

From an active Codex session:

```sh
slumber run 'cargo test --release'
```

Slumber reads `CODEX_THREAD_ID` or `CODEX_SESSION_ID`, or accepts `--session-id <id>`. With no session or explicit wake-up mode, it fails before starting a task. The agent should hand off only after finishing its current work and stop until the event arrives; Slumber does not terminate the agent itself. An exit code of zero from the queue command confirms queue-command acceptance, not that an agent is running or has processed the event.

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

If an agent launcher preserves `TMUX` but removes `TMUX_PANE`, Slumber uses the active pane in the session identified by `TMUX`. Set `TMUX_PANE` explicitly to select a particular pane when that fallback is ambiguous. Pane creation failures print a warning without rejecting the submitted task; use `slumber logs` even if no viewer opened.

### Terminal independence and its limits

For noninteractive commands submitted successfully to the automatically started daemon or `slumber daemon start`, the submitting terminal and the tmux log viewer are not task owners. Closing them must not terminate the delegated work. A foreground `slumber daemon` is different: it deliberately remains attached to its terminal and must be kept alive.

| Event | Execution and feedback boundary |
| --- | --- |
| Submitting CLI/terminal exits; tmux pane or server closes | Background-daemon tasks must continue. If the agent also exits, completion does not guarantee that it processes the notification. |
| SSH transport disconnects after remote submission is accepted | The detached remote wrapper can continue; monitoring needs working SSH again. A disconnect before acceptance is ambiguous: inspect remote state before retrying submission. |
| Local daemon crashes or the local computer reboots | Local supervision is lost. Remote execution can continue, but monitoring requires daemon recovery and valid credentials; see below. |
| Execution host shuts down, OOM/SIGKILL, application failure, storage failure, or an administrator terminates the login scope/processes | Neither Slumber nor `nohup` guarantees survival or complete logs. Application checkpoints and host/service configuration are separate responsibilities. |

These are scoped lifecycle requirements, not a promise that commands cannot fail. Jobs must not depend on terminal input, an interactive password prompt, or a launcher that returns before its real work finishes. Slumber does not automatically rerun interrupted experiments.

The real tmux smoke test now starts a fresh daemon from a PTY, lets the submitting terminal exit, destroys the isolated tmux server while work is pending, and checks continued output, exit status and wake-up. The remote-wrapper test checks survival after its launch shell exits and after SIGHUP; the SSH smoke test separately exercises a real OpenSSH launch. They do not certify every OS logout policy, terminal application's process-killing behavior, or remote server configuration.

### After a local shutdown or reboot

A local shutdown stops the Slumber daemon, Codex and tmux. A delegated SSH job can continue on the remote server; its logs and exit-code file allow Slumber to discover completion later, even if it finished while the local computer was off. Recovery requires intact local job state, remote job files and working SSH authentication. This does not apply to local jobs, whose process state cannot be recovered.

**The daemon does not start automatically after reboot.** Remote recovery runs when the daemon starts. `slumber status` only reads saved metadata: it neither restarts monitoring nor checks the remote process, so a displayed `running` state may be stale. `slumber daemon status` checks whether the monitor is actually running.

To recover:

1. Keep the original Slumber state directory (including any custom `SLUMBER_HOME`). Do not purge it or resubmit the remote command: that could start duplicate work.
2. Reopen the original Codex conversation and restore any required login. For the CLI, use `codex resume <session-id>` in the original project. tmux is optional; monitoring and wake-up do not require a log pane.
3. From another terminal, start the monitor and inspect its saved results:

   ```sh
   slumber daemon start
   slumber status
   slumber logs <job-id> --err
   ```

4. If status reports a failed wake-up, inspect the job's `resume.log`, correct the cause, then run `slumber retry <job-id>`. This retries notification, not the remote task. If wake-up was already recorded as successful but the agent did not act, retry is not available; inspect the saved `payload.md` and bring the result to the original conversation manually.

Recovery uses the original request environment. After reboot, a saved `SSH_AUTH_SOCK` or another temporary endpoint may no longer exist. Working SSH in a new terminal therefore does not necessarily prove the saved monitor environment works. The current version does not refresh these values automatically; see the private-snapshot precautions above rather than resubmitting an existing job or copying credentials into bug reports.

The old tmux viewer is not recreated during recovery. Old pane IDs can refer to unrelated panes in a new server; current cleanup does not validate the original server identity. Use an ordinary terminal or a differently named tmux server for recovery; do not reuse the old socket path while pending wake-ups can still attempt pane cleanup.

**Not implemented yet:** an explicit `slumber recover` command, automatic offline-monitor warnings, optional start-at-login, safe rebinding of stale transport/pane references, and agent-processing acknowledgements. These are possible improvements, not current guarantees. Slumber does not automatically restart Codex or tmux, and a stopped daemon cannot display a warning until it is started again.

### When logs appear incomplete

The tmux viewer starts at the last 20 lines; this does **not** truncate the files. `slumber logs <job-id>` prints the complete current stdout file, and `--err` reads stderr separately. Neither command follows future writes. SSH logs remain on the remote server and are fetched over SSH, not mirrored locally.

Slumber captures bytes written to stdout/stderr, not data still buffered inside the program. Python normally buffers redirected stdout; use `python3 -u train.py` or `PYTHONUNBUFFERED=1 python3 train.py` for timely output, including inside an SSH command. A SIGKILL/OOM can permanently lose unflushed data. Output redirected by your command to another file is not copied back to Slumber's logs.

Keep delegated work in the foreground, or use shell `wait` for background children. Local process-group cleanup terminates ordinary remaining children when the submitted shell exits; `worker &` by itself is not a complete delegated task. Slumber cannot guarantee all application logs across crashes, buffering, or application-managed redirection.

## Data and safety

State defaults to `~/.slumber`. The socket uses `$XDG_RUNTIME_DIR/slumber/slumber.sock` when set, otherwise `~/.slumber/slumber.sock`. `SLUMBER_HOME` and `SLUMBER_SOCKET` override these paths. Choose absolute, private directories owned by your account, mode `0700`; Slumber refuses unsafe directories instead of changing their permissions.

Job directories are `0700` and private files are `0600`. **Full environment snapshots, including credentials, temporarily reside in `request.json`.** They are cleared after a successful wake-up. Failed wake-ups retain them for retry; logs and commands may independently contain secrets. Do not upload your state directory, include it in public bug reports, or run untrusted templates. Slumber has no telemetry or remote service. See [SECURITY.md](SECURITY.md).

Keep the local daemon running while local jobs execute. Local process state cannot be recovered after a daemon crash/reboot: such tasks are reported as unknown and require manual inspection. The supported stop command refuses active work. Process-group cleanup handles ordinary local descendants, but is not containment for processes that detach themselves; the remote POSIX wrapper does not provide process-tree cleanup. A shell exit in the 129–192 range is reported as a possible signal (137 often means SIGKILL), not proof of OOM. Logs have no automatic size limit or retention policy.

## Upgrade, uninstall and cleanup

Wait for jobs and wake-ups to finish before upgrading. The installer stops an idle existing daemon, then replaces its binary. When migrating from an early MVP daemon without `daemon stop`, finish its jobs and stop that old daemon manually before upgrading. Do not kill it during a local job.

Starting with 0.1.2, update the currently running executable's installation (not another copy in PATH):

```sh
slumber update                         # latest stable release
slumber update --version v0.1.2        # explicit release, including rollback
slumber update --github                # authenticated gh, including private repos
slumber update --from /path/to/slumber  # offline installation
```

The command uses its embedded installer, stages and checks the new binary before stopping an idle daemon, then atomically replaces the installed file. It never bypasses active-task protection, uses sudo, edits your shell configuration or deletes job records. An anonymous HTTP 404 automatically falls back to `gh` if available; `--github` skips the anonymous attempt. Authenticate `gh` with access to this repository; its normal `GH_CONFIG_DIR` setting is honored. Older versions without `update` need one installation using the checkout's `install.sh --from` (or `--github`) first. Package-manager installations should be upgraded through their package manager. Symlinked launchers resolve to the actual executable location.

`daemon status` counts task monitors and wake-up hooks, not just running experiment processes. `status --limit N` limits completed history; unresolved jobs are always shown, even if old. If a successful remote probe finds neither the recorded process nor its exit record, monitoring stops and the result is marked **unknown**, never success. Descendant work could still exist: inspect the remote workload and logs before relaunching anything. SSH/network failures continue monitoring; a reused PID can also keep a monitor waiting. Missing recovery files and invalid exit records are reported explicitly. Unknown/failed records remain for diagnosis and do not themselves block an idle daemon from stopping. `retry` retries a failed wake-up with a known exit status, not an unknown task or the task itself.

```sh
# After scripts are publicly available:
curl -fsSL https://raw.githubusercontent.com/phoenys/slumber/main/uninstall.sh | sh
# Also permanently delete local configuration, credentials and logs (asks for confirmation):
curl -fsSL https://raw.githubusercontent.com/phoenys/slumber/main/uninstall.sh | sh -s -- --purge
```

For automation, use `--purge --yes`. The same scripts can be run from a local checkout. Purge requires the Slumber state marker and refuses broad directories and symlinks; legacy/unmarked state requires manual inspection. It never edits project instruction files or removes remote server logs. You can remove the marked Slumber paragraph from a project's instruction file manually, and remove remote job directories after reviewing their contents.

For development, clean-machine verification, rollback, and the release checklist, see [CONTRIBUTING.md](CONTRIBUTING.md) and [RELEASING.md](RELEASING.md).
