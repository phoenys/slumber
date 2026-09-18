# Release verification and maintainer runbook

## Scope and distribution

The 0.1 series covers one local machine and one SSH server, with Codex or a trusted custom wake-up hook. Other agents, multi-server orchestration and clusters are outside this release. Cargo registry publication is disabled; distribute the four native GitHub release binaries.

Repository visibility is controlled by the owner. In a private repository, publishing a release does not make its assets public: authorized users must download through GitHub, `slumber update --github` or `gh release download`, then use `install.sh --from` when needed. Preserve the current visibility unless the owner explicitly requests a change. Anonymous installer verification is a separate gate when the repository is public.

The repository was initialized with a clean source snapshot after a privacy review. Do not import prior Git history, old tags, diagnostic dumps or personal development notes. Earlier CI links are not evidence for this repository. Record current evidence in the release notes using the release commit and its Actions run, without private identifiers.

## Required gates

1. Review tracked files, all history being pushed, commit/tag authors and release attachments for credentials and personal identifiers. Use GitHub noreply emails. Keep `docs/`, `AGENTS.md`, local state and environment files untracked. Run Gitleaks with redacted output; a clean secret scan does not replace the identity review.
2. Run formatting, Clippy with warnings denied, locked Rust tests and release build. Repeat the clean-environment installer smoke and real tmux smoke in isolated state, never against active user jobs.
3. Require all seven CI checks on the release code: `Test (x86_64-unknown-linux-gnu)`, `Test (aarch64-unknown-linux-gnu)`, `Test (aarch64-apple-darwin)`, `Test (x86_64-apple-darwin)`, `msrv`, `security`, and `ssh`. These cover native build/install/tmux, Rust 1.89, dependency and history secret scans, workflow lint, real OpenSSH and foreign-UID IPC checks.
4. Verify a real authenticated agent handoff and intended SSH target when changing those integration contracts. The owner has supplied personal MVP hardware evidence. CI validates shell hooks and isolated SSH, not a human login or proof that Codex processed a queued event. Record this distinction rather than claiming CI proves session continuity.
5. Match Cargo.toml, Cargo.lock and the version tag. Push only the intended branch/tag, never `--mirror` or all old local tags. Invoke **Prepare draft release** with the existing tag. Its reusable verification jobs and draft job must all pass.
6. Check that the draft contains exactly the four intended binaries. Download the actual macOS ARM64 asset and run the clean-environment and real tmux smoke tests without replacing a daemon that has active jobs. Inspect distributed binaries for personal paths or identifiers.
7. Publish only after these gates pass. Read back the release tag, target, draft flag and asset list. Verify authenticated installation while private; verify anonymous installation separately if later made public. Publishing authorization does not imply permission to change repository visibility.

Enable Dependabot alerts/security updates. Enable secret scanning, push protection, private vulnerability reporting and required-check branch protection where supported by the repository plan and visibility. Require no second-person approval for a solo maintainer; do not bypass failing checks when protection is unavailable.

## Known limits and rollout

The daemon does not automatically start after reboot. Remote monitoring can recover after manual daemon startup, but local waitpid supervision cannot. Saved SSH-agent endpoints and tmux pane identities may be stale after reboot. Queue acceptance is not proof that an agent processed an event; delivery is not exactly-once. Consult README recovery instructions before retrying, and do not launch duplicate remote work.

Request environments are stored in plaintext with private permissions until successful wake-up; failures retain them for retry. Log growth is unbounded. Same-user processes are trusted, and Slumber is not a sandbox. Read SECURITY.md before running credential-bearing commands. No guarantee covers host reboot, SIGKILL/OOM, storage failure or all host logout policies.

Start with a small group on macOS 15+ and Ubuntu 24.04+, ARM64 and x86_64. Collect redacted doctor output, installation failures and task/wake-up outcomes. There is no telemetry service. Stop rollout on credential exposure, silent lost wake-ups, unreaped ordinary local children or installers changing unrelated files.

## Rollback

Let active tasks and wake-ups finish before stopping the daemon or replacing its binary. Reinstall a known-good artifact using `install.sh --from` (or `--version` when public). If no prior known-good release exists, uninstall while retaining state. Never use `--purge` for routine rollback; it destroys diagnostic evidence and snapshots needed for recovery.

Do not downgrade protocol 5 while jobs are active. Protect any state backup because it may contain secrets. Revoke exposed credentials; reinstalling or deleting a repository does not revoke them or retract existing copies. Review installation and wake-up failures after initial adoption before expanding support.
