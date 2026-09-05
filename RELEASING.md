# Release readiness and maintainer runbook

## Decision and scope

Prepare 0.1.0 as a small initial release for local jobs and one SSH server. The readiness work includes setup, diagnostic errors, credential handling, process/wake-up lifecycle, install/uninstall, support instructions, CI and rollback. Other agents, multi-server orchestration and clusters are separate follow-up releases.

**Current decision: local candidate prepared; public release remains gated on GitHub runner results and maintainer actions below.** No public tag, release, repository visibility change or package-registry publication has been performed. Cargo registry publication is disabled; distribution is through reviewed GitHub release binaries.

## Evidence collected on 2026-09-05

| Gate | Actual evidence | Result |
| --- | --- | --- |
| macOS ARM64 | macOS 26.5.1; Rust 1.95.0; formatting, Clippy with warnings denied, 3 unit + 6 integration tests, release build | Passed |
| Minimum compiler | Rust 1.89.0, all-targets locked check on macOS ARM64; full Linux checks also used 1.89.0 | Passed |
| Fresh local environment | `scripts/smoke.sh`: empty HOME, env -i, binary install, explicit init, exit 1, SIGKILL, 100k log lines, generic wake-up, credential cleanup, uninstall/reinstall/purge | Passed on macOS and Linux ARM64 |
| Real terminal integration | `scripts/tmux-smoke.sh`: 1 → 2 → 1 panes, visible stdout and stderr; integration tests also exercise tall-pane orientation and failed pane teardown | Passed on macOS and Linux ARM64 |
| Linux runtime | Disposable Debian 13 ARM64 container, unprivileged tester account, Rust checks/tests/build, ShellCheck 0.10.0 | Passed |
| Real SSH | Disposable OpenSSH server/account in that Linux container; remote exit 7, both logs, local wake-up | Passed |
| Credential boundaries | Restarted-daemon snapshot test, failed-wake-up retry/cleanup, private file/directory permissions, shared-directory rejection; root-run IPC test independently rejects foreign socket owner and foreign server UID without transmitting a byte | Passed |
| Known dependency vulnerabilities | cargo-audit 0.22.2, current RustSec source archive (1239 advisories), 44 lockfile dependencies, with yanked checks | Passed; no reported vulnerabilities |
| Committed secrets | Gitleaks 8.30.0, full Git history, redacted output | Passed at preparation; rerun for release commit in CI |
| Scripts/workflows | ShellCheck 0.11.0 on macOS and 0.10.0 on Linux; actionlint 1.7.12 | Passed |
| Four official binary targets | macOS 15 ARM64/x86_64 and Ubuntu 24.04 ARM64/x86_64 GitHub jobs configured | Pending remote CI |
| Codex session continuity | Owner previously passed personal MVP hardware testing; local `codex queue --help` capability verified on 0.153.4 | Repeat short real-session canary with candidate binary before release |
| Public download/install URL | Download URLs and platform mapping prepared; local --from installation verified | Pending public release assets |

The local Docker Hub Rust-image pull failed because of network resolution. Linux evidence was obtained from an already-cached official `golang:1.24` Debian 13 image with a fresh Rust 1.89.0 installation, then the same `container-qa.sh` was run. This validates the Linux code path, not the exact four GitHub artifact environments. The committed Dockerfile uses `rust:1.89-bookworm` and is the normal reproducible recipe once registry access is available.

Native GitHub API/SSH access failed from this environment. The configured remote is `git@github.com:phoenys/slumber.git`; the current GitHub CLI account is `Erican-Ji`, and an authenticated REST repository lookup returned 404. Confirm the intended repository and grant this account access (or authenticate the intended account). No source was pushed and no remote CI result is claimed. RustSec was obtained through its official GitHub source-archive endpoint when its Git transport failed.

## Remaining owner actions, in order

1. Restore access to the intended private repository. Confirm `git remote -v`, `gh repo view`, and `git ls-remote origin` work. Review the prepared commits, push a release-preparation branch, and open a PR. The CI workflow runs on `release/**` branches and PRs; keep the repository private during this stage.
2. Enable Actions if required by repository/billing policy. Require all four Test jobs, MSRV, security and SSH jobs to pass. These jobs upload native binary artifacts. Review any failure rather than disabling its gate. GitHub's supported [runner labels and architectures](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) are the basis for the matrix.
3. Configure branch protection/rulesets to require a PR and the checks above. Enable Dependabot alerts/security updates and private vulnerability reporting. These are repository settings, not effects of committing workflow YAML.
4. Download the candidate artifact, install it on a separate user/VM, and follow CONTRIBUTING.md's clean-computer procedure. Run one short Codex handoff in a real session and one on the intended SSH server. Record the exact Codex distribution/version and candidate commit; do not promise support for builds missing `codex queue`.
5. Review SECURITY.md and the intended binary OS minimums. The MVP does not recover live local waitpid state after a crash, guarantee exactly-once wake-up, bound log disk growth, or encrypt credential snapshots. If these limits do not fit the intended initial users, narrow the rollout instead of calling the release generally reliable.
6. Once CI and the canary pass, merge the candidate. Confirm version and changelog, create/push `v0.1.0` for that commit, and run **Prepare draft release** with that existing tag. The workflow repeats CI and creates a draft with four binaries; it never publishes automatically.
7. Review repository history and the draft assets, then change private → public and publish the draft when ready. Verify the unauthenticated one-line installer and uninstaller from a fresh supported user account. Public visibility is a deliberate last action, because making the repository private again cannot retract copies others obtained.

Do not announce all-platform support until the actual platform gates pass. A fresh HOME removes accidental dependencies on personal configuration, but only native runners/new systems establish OS and architecture compatibility. A container adds Linux evidence, but does not validate Intel Macs, desktop sleep behavior, or a human agent login.

## Small rollout and rollback

Start with a small group of users on the declared OS/architecture matrix. Ask for redacted doctor output, job outcome, wake-up success/failure and installation friction. Inspect daemon/resume logs locally; there is no telemetry service. Stop expansion on credential exposure, silent lost wake-ups, unreaped ordinary local children, or install/uninstall modifying unrelated files.

To roll back the binary, let active jobs and wake-up commands finish, stop the daemon, and reinstall a prior known-good artifact with `install.sh --version <tag>` or `--from`. For the first release with no prior artifact, uninstall while retaining state. Do not use `--purge` as a routine rollback: it permanently deletes evidence and credential snapshots needed for recovery. Early MVP daemons without the lifecycle command require a manual stop only after their work finishes.

Protocol 5 and the new metadata should not be downgraded while jobs are active. Back up private state only to a protected location if preserving it is necessary; it can contain secrets. If a token was exposed, revoke/rotate it; reinstalling Slumber does not repair that exposure.

After the first week, review installation failures by platform, failed wake-ups and their causes, resource usage during representative long jobs, and user confusion. Turn evidence into small reviewed PRs. Each future agent/server/cluster adapter must add a real integration gate and a documented compatibility boundary before expanding the public support claim.
