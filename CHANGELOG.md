# Slumber 0.1.0 — release candidate

Initial local/one-SSH-server MVP. Public release is pending the gates in RELEASING.md.

- Hand off shell commands to a private local daemon, capture separate stdout/stderr logs, and deliver completion metadata to Codex through `codex queue` or a custom shell hook.
- Execute on one SSH server without a remote agent installation; recover remote monitoring after a local daemon restart.
- Display both logs in an adaptive detached tmux pane and close it before wake-up.
- Add explicit init selection, standalone `--no-resume`, doctor, daemon start/status/stop and failed-wake-up retry.
- Authenticate IPC peers, validate private directories, write private state atomically, isolate job environments, and clear credentials after successful wake-up.
- Add user-local installation/uninstallation, clean-environment smoke tests, four-platform CI, dependency/secret scanning and a manually invoked draft-release workflow.

Protocol version 5 requires restarting earlier MVP daemons after their jobs finish. Full environment snapshots are temporarily persisted for local and remote jobs; failed wake-ups retain them for retry. Read SECURITY.md before using credential-bearing environments.
