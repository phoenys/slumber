# Development

The first release supports the local/one-SSH-server workflow. Add other agents, multi-server orchestration or clusters in separate small PRs with explicit acceptance criteria and failure-mode tests. Avoid speculative abstractions, heavy dependencies and redundant tests.

## Local checks

Install Rust 1.89+ and native linker tools. Use the checked-in Cargo.lock:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
sh scripts/smoke.sh "$PWD/target/release/slumber"
sh scripts/tmux-smoke.sh "$PWD/target/release/slumber"  # needs tmux
shellcheck install.sh uninstall.sh scripts/*.sh
cargo audit
```

The clean-environment smoke script uses a temporary HOME and `env -i`, installs from the built binary, runs without an agent login, and tests installation, init, task failure, SIGKILL, 100,000 log lines, generic wake-up, credential cleanup, uninstall and purge. It deletes only its own temporary state after stopping its daemon; on an active-daemon failure, it retains the directory for diagnosis. An isolated environment on a Mac is not a replacement for Linux CI.

With Docker running, this repeats the checks in a fresh Linux container with an unprivileged test user, a real isolated SSH service, and foreign-UID IPC rejection tests:

```sh
docker build -f scripts/Dockerfile.qa -t slumber-qa:local .
docker run --rm slumber-qa:local
```

The build context excludes personal docs, `.git`, credentials and existing build/state directories. The container creates its own SSH keys and never uses your server accounts. Rust images, system packages and crate downloads require network access.

For real SSH integration, use a disposable server/account with non-interactive SSH configured:

```sh
sh scripts/ssh-smoke.sh "$PWD/target/release/slumber" disposable-ssh-host
```

This creates remote `~/.slumber/jobs/` data; the script intentionally leaves it for inspection. The GitHub SSH job uses a temporary runner account. Never point destructive experiments at production jobs.

## Clean computer / VM verification

1. Start a new supported macOS/Linux user or VM. Do not copy your personal dotfiles, Slumber state or API keys.
2. Download the candidate binary artifact for that OS and CPU (or build the candidate checkout). Install it using `sh install.sh --from /absolute/path/to/binary`. No Rust toolchain is needed to run a downloaded binary.
3. Run `slumber doctor`, then the standalone first-run commands in README. Missing Codex, SSH or tmux is expected until those optional integrations are installed.
4. Run the clean smoke script; record OS version, architecture, candidate commit, commands and outcomes.
5. Install a compatible Codex build, authenticate yourself, open a disposable project/session, and perform one short handoff. Verify the completion event reaches that same session and its logs can be inspected. CI never uses a real account or spends model credits for this step.
6. Configure one disposable SSH target and repeat. Disconnect/reconnect the network and verify monitoring resumes. Use tmux to verify both log streams and pane cleanup.
7. Finish all work, uninstall, reinstall, and uninstall with `--purge`. Verify your project and unrelated dotfiles are intact.

## CI and review

CI tests four native platform/architecture combinations, builds downloadable artifacts, checks the minimum Rust version, runs real tmux and Linux OpenSSH smoke tests, and checks dependencies and committed secrets. Use ordinary `pull_request` workflows with read-only permissions; never execute untrusted PR code with publishing credentials. Dependencies are reviewed through Dependabot PRs.

Before merging, require CI and review any change to command execution, credentials, SSH quoting, process lifecycle, state persistence or install/uninstall. The repository owner must configure branch protection in GitHub; workflow YAML cannot enforce it. Use conventional commits with a description of the reason and relevant validation.

Do not commit local `docs/`, `AGENTS.md`, `.dogfood/`, state, logs or credentials. Public user and contributor documentation lives in the tracked root Markdown files.

## Git with an intentional fake-IP proxy

Git SSH authentication is independent of the GitHub CLI/API login. A working repository SSH key does not give `gh` the same account's API access.

If the native resolver stalls but the proxy's current GitHub fake-IP mapping works, use a **repository-local** `core.sshCommand` override with `HostName=<current-fake-IP>`, `HostKeyAlias=github.com`, `IdentitiesOnly=yes`, and your designated key. Retain host-key checking. This avoids changing system DNS, routes or the proxy. Do not commit machine-specific key paths or mappings into shared scripts. Fake-IP assignments can change when the proxy resets; refresh the mapping when necessary. Once normal resolution works, remove the HostName override while preserving any existing repository-specific key selection.
