# Release readiness and maintainer runbook

## Decision and scope

Prepare 0.1.0 as a small initial release for local jobs and one SSH server. The readiness work includes setup, diagnostic errors, credential handling, process/wake-up lifecycle, install/uninstall, support instructions, CI and rollback. Other agents, multi-server orchestration and clusters are separate follow-up releases.

**Current decision: cross-platform candidate verified; public release remains gated on the real-session canary and maintainer actions below.** The candidate is in [draft PR #1](https://github.com/phoenys/slumber/pull/1). No version tag, release, repository visibility change or package-registry publication has been performed. Cargo registry publication is disabled; distribution is through reviewed GitHub release binaries.

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
| Committed secrets | Gitleaks 8.30.0, full Git history, redacted output; remote security job also runs cargo-audit and actionlint | Passed locally and in candidate CI; repeat for release commit |
| Scripts/workflows | ShellCheck 0.11.0 on macOS and 0.10.0 on Linux; actionlint 1.7.12 | Passed |
| Four native binary targets | macOS 15 ARM64/x86_64 and Ubuntu 24.04 ARM64/x86_64; all four Test jobs plus MSRV, security and SSH jobs in [run 33970526372](https://github.com/phoenys/slumber/actions/runs/33970526372) | All 7 passed at `cf289af630efab81647d4bac45bb2b4868cdbdea`; four binary artifacts available |
| Downloaded CI binary | macOS ARM64 artifact from that run, installed and exercised by `scripts/smoke.sh` on macOS 26.5.1 with empty HOME and env -i | Passed, including uninstall/reinstall/purge; no source build used |
| Repository dependency security | Dependabot alerts and automated security updates enabled and read back through the repository API | Enabled, not paused; no open alerts at inspection |
| Codex session continuity | Owner previously passed personal MVP hardware testing; local `codex queue --help` capability verified on 0.153.4 | Repeat short real-session canary with candidate binary before release |
| Public download/install URL | Download URLs and platform mapping prepared; local --from installation verified | Pending public release assets |

The local Docker Hub Rust-image pull failed because of network resolution. Additional Linux evidence was obtained from an already-cached official `golang:1.24` Debian 13 image with a fresh Rust 1.89.0 installation, then the same `container-qa.sh` was run. The four native GitHub jobs above separately validate the intended artifact environments. The committed Dockerfile uses `rust:1.89-bookworm`; the exact Dockerfile build remains unverified locally because of registry access.

Git SSH read/write access has been verified using the repository's designated key and a repository-local HostName override for the owner's intentional fake-IP proxy setup. The candidate is pushed to `release/preflight`; that branch matches the CI push trigger. System DNS, routes, TUN and proxy settings were not changed. The owner's isolated GitHub CLI configuration now provides authorized repository API access without changing the global account. Both branch runs (33970425071 and 33970526372) passed; individual jobs and artifact presence were checked for the latter. RustSec was obtained through its official GitHub source-archive endpoint when its Git transport failed locally.

GitHub's branch-protection API explicitly returned “Upgrade to GitHub Pro or make this repository public to enable this feature.” No paid plan or visibility change was made. Require manual PR/CI review until the owner chooses one of those options. Private vulnerability reporting is a [public-repository feature](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository); enable it as part of the deliberate public transition, not by exposing this candidate early.

## Remaining owner actions, in order

1. Review [draft PR #1](https://github.com/phoenys/slumber/pull/1) and its latest CI results. Keep the repository private. All four Test jobs, MSRV, security and SSH must pass for the commit being merged; earlier green runs are not evidence for later code changes.
2. Download the candidate artifact, install it on a separate user/VM, and follow CONTRIBUTING.md's clean-computer procedure. Run one short Codex handoff in a real session and one on the intended SSH server. Record the exact Codex distribution/version and candidate commit; do not promise support for builds missing `codex queue`. Native CI and the downloaded-artifact smoke are complete, but cannot establish human agent login or actual session continuity.
3. Review SECURITY.md and the intended binary OS minimums. The MVP does not recover live local waitpid state after a crash, guarantee exactly-once wake-up, bound log disk growth, or encrypt credential snapshots. If these limits do not fit the intended initial users, narrow the rollout instead of calling the release generally reliable.
4. Choose whether to upgrade the account for private branch protection or enable protection during the later public transition. Require PRs and these exact checks: `Test (x86_64-unknown-linux-gnu)`, `Test (aarch64-unknown-linux-gnu)`, `Test (aarch64-apple-darwin)`, `Test (x86_64-apple-darwin)`, `msrv`, `security`, `ssh`. Do not require another person's approval unless a second reviewer is available. Dependabot alerts/security updates are already enabled; scheduled dependency PRs start after the configuration reaches the default branch.
5. Once CI and the canary pass, mark the PR ready and merge the candidate. Confirm version and changelog, create/push `v0.1.0` for that commit, and run **Prepare draft release** with that existing tag. The workflow must first reach the default branch to be manually dispatched; it repeats CI and creates a draft with four binaries, never publishing automatically. This draft-release path is prepared but has not yet been executed.
6. Review repository history and the draft assets, then change private → public, enable private vulnerability reporting (and branch protection if deferred), and publish the draft when ready. Verify the unauthenticated one-line installer and uninstaller from a fresh supported user account. Public visibility is a deliberate last action, because making the repository private again cannot retract copies others obtained.

Do not announce all-platform support until the actual platform gates pass. A fresh HOME removes accidental dependencies on personal configuration, but only native runners/new systems establish OS and architecture compatibility. A container adds Linux evidence, but does not validate Intel Macs, desktop sleep behavior, or a human agent login.

## Small rollout and rollback

Start with a small group of users on the declared OS/architecture matrix. Ask for redacted doctor output, job outcome, wake-up success/failure and installation friction. Inspect daemon/resume logs locally; there is no telemetry service. Stop expansion on credential exposure, silent lost wake-ups, unreaped ordinary local children, or install/uninstall modifying unrelated files.

To roll back the binary, let active jobs and wake-up commands finish, stop the daemon, and reinstall a prior known-good artifact with `install.sh --version <tag>` or `--from`. For the first release with no prior artifact, uninstall while retaining state. Do not use `--purge` as a routine rollback: it permanently deletes evidence and credential snapshots needed for recovery. Early MVP daemons without the lifecycle command require a manual stop only after their work finishes.

Protocol 5 and the new metadata should not be downgraded while jobs are active. Back up private state only to a protected location if preserving it is necessary; it can contain secrets. If a token was exposed, revoke/rotate it; reinstalling Slumber does not repair that exposure.

After the first week, review installation failures by platform, failed wake-ups and their causes, resource usage during representative long jobs, and user confusion. Turn evidence into small reviewed PRs. Each future agent/server/cluster adapter must add a real integration gate and a documented compatibility boundary before expanding the public support claim.
