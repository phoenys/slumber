# Slumber 0.1.1

Initial distribution of the local/one-SSH-server MVP, including the fixes below. The repository remains private; download assets with an authorized GitHub account. See RELEASING.md for verification and rollback.

- Verify terminal independence with a real submitting PTY and tmux server teardown, and test remote-wrapper survival after launcher exit and SIGHUP. Document Slumber-owned detachment and the limits of terminal, reboot and failure guarantees.
- Clarify the agent contract: launch new tasks through Slumber, use native SSH mode, and monitor existing tasks only through run-specific completion records. Re-running init upgrades the exact original generated contract without replacing customized instructions.
- Restore tmux auto-splitting when an agent preserves `TMUX` but strips `TMUX_PANE`, targeting that session rather than a different client's session. Surface pane failures to the submitting CLI.
- Explain unavailable draft assets and stable/prerelease download URLs in installer errors; cover piped installation and unavailable downloads.
- Expand CLI help with execution, log, environment and wake-up contracts. Clarify application buffering and foreground/background task requirements.
- Extend real tmux checks to missing pane variables and a competing session; verify 100,000 lines on each log stream plus trailing bytes without newlines.
- Update nix and GitHub Actions dependencies, and synchronize the wake-up retry test with task completion instead of a fixed task duration.

# Slumber 0.1.0 — release candidate

Earlier development candidate; superseded by 0.1.1.

- Hand off shell commands to a private local daemon, capture separate stdout/stderr logs, and deliver completion metadata to Codex through `codex queue` or a custom shell hook.
- Execute on one SSH server without a remote agent installation; recover remote monitoring after a local daemon restart.
- Display both logs in an adaptive detached tmux pane and close it before wake-up.
- Add explicit init selection, standalone `--no-resume`, doctor, daemon start/status/stop and failed-wake-up retry.
- Authenticate IPC peers, validate private directories, write private state atomically, isolate job environments, and clear credentials after successful wake-up.
- Add user-local installation/uninstallation, clean-environment smoke tests, four-platform CI, dependency/secret scanning and a manually invoked draft-release workflow.

Protocol version 5 requires restarting earlier MVP daemons after their jobs finish. Full environment snapshots are temporarily persisted for local and remote jobs; failed wake-ups retain them for retry. Read SECURITY.md before using credential-bearing environments.
