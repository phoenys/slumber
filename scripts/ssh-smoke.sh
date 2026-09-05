#!/bin/sh
# Only point this at a disposable SSH account. It creates remote ~/.slumber logs.
set -eu
binary=${1:?provide an absolute path to slumber}
destination=${2:?provide a disposable SSH destination}
qa_dir=$(mktemp -d /tmp/slumber-ssh.XXXXXX)
slumber() { env SLUMBER_HOME="$qa_dir/state" SLUMBER_SOCKET="$qa_dir/socket" "$binary" "$@"; }
cleanup() {
    slumber daemon stop || return
    rm -r -- "$qa_dir"
}
trap cleanup EXIT HUP INT TERM
# shellcheck disable=SC2016 # Expanded by the wake-up child.
submission=$(slumber run --no-tail --ssh "$destination" --resume-template 'printf complete > "$SLUMBER_HOME/complete"' \
    'printf remote-stdout; printf remote-stderr >&2; sleep 1; exit 7')
printf '%s\n' "$submission"
job=$(printf '%s\n' "$submission" | awk '/^Submitted / {print $2}')
attempt=0
while [ ! -e "$qa_dir/state/complete" ] && [ "$attempt" -lt 150 ]; do
    sleep 0.1
    attempt=$((attempt + 1))
done
[ -e "$qa_dir/state/complete" ]
[ "$(slumber logs "$job")" = remote-stdout ]
[ "$(slumber logs "$job" --err)" = remote-stderr ]
grep -q '"Exited": 7' "$qa_dir/state/jobs/$job/meta.json"
printf 'PASS: real OpenSSH handoff, dual logs, exit 7 and local wake-up.\n'
