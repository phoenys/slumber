#!/bin/sh
# Entry point for the disposable QA container only.
set -eu
mkdir -p /run/sshd
ssh-keygen -A
install -d -m 700 -o tester -g tester /home/tester/.ssh
runuser -u tester -- ssh-keygen -t ed25519 -N '' -f /home/tester/.ssh/id_ed25519
install -m 600 -o tester -g tester /home/tester/.ssh/id_ed25519.pub /home/tester/.ssh/authorized_keys
passwd -d tester
/usr/sbin/sshd
runuser -u tester -- sh -c 'ssh-keyscan -H 127.0.0.1 > /home/tester/.ssh/known_hosts'
runuser -u tester -- env CARGO_HOME=/home/tester/.cargo sh -ec '
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo build --locked --release
    shellcheck install.sh uninstall.sh scripts/*.sh
    sh scripts/smoke.sh /work/target/release/slumber
    sh scripts/tmux-smoke.sh /work/target/release/slumber
    sh scripts/ssh-smoke.sh /work/target/release/slumber tester@127.0.0.1
'
python3 scripts/ipc-security.py /work/target/release/slumber tester
