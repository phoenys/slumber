#!/bin/sh
# No Rust, agent login, Python, or pre-existing dotfiles needed.
set -eu
repo=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$repo/target/release/slumber"}
smoke_dir=$(mktemp -d /tmp/slumber-smoke.XXXXXX)
mkdir "$smoke_dir/home"
clean_env() {
    env -i HOME="$smoke_dir/home" PATH=/usr/bin:/bin \
        SLUMBER_HOME="$smoke_dir/state" SLUMBER_SOCKET="$smoke_dir/socket" \
        SLUMBER_INSTALL_DIR="$smoke_dir/bin" "$@"
}
cleanup() {
    if [ -x "$smoke_dir/bin/slumber" ]; then
        if ! clean_env "$smoke_dir/bin/slumber" daemon stop; then
            printf 'Retained active smoke state: %s\n' "$smoke_dir" >&2
            return
        fi
    fi
    rm -r -- "$smoke_dir"
}
trap cleanup EXIT HUP INT TERM
clean_env sh "$repo/install.sh" --from "$binary"
slumber=$smoke_dir/bin/slumber
clean_env "$slumber" --version
clean_env "$slumber" status
clean_env "$slumber" init --agent codex --file "$smoke_dir/bad.md" && exit 1
clean_env "$slumber" init --file "$smoke_dir/AGENTS.md"
clean_env "$slumber" init --file "$smoke_dir/AGENTS.md"
[ "$(grep -c 'SLUMBER PROTOCOL' "$smoke_dir/AGENTS.md")" -eq 1 ]
wait_done() {
    job=$1
    attempt=0
    while [ "$attempt" -lt 100 ]; do
        if ! grep -q '"resumed_at": null' "$smoke_dir/state/jobs/$job/meta.json"; then return; fi
        sleep 0.1
        attempt=$((attempt + 1))
    done
    printf 'Completion timed out: %s\n' "$job" >&2
    exit 1
}
submit() {
    # Expand SLUMBER_HOME in the wake-up child, not this shell.
    # shellcheck disable=SC2016
    clean_env "$slumber" run --no-tail --resume-template 'printf resumed > "$SLUMBER_HOME/resumed"' "$1" |
        awk '/^Submitted / {print $2}'
}
job=$(submit 'printf crash >&2; exit 1')
wait_done "$job"
[ "$(clean_env "$slumber" logs "$job" --err)" = crash ]
grep -q '"Exited": 1' "$smoke_dir/state/jobs/$job/meta.json"
job=$(submit 'kill -KILL $$')
wait_done "$job"
grep -q 'SIGKILL' "$smoke_dir/state/jobs/$job/meta.json"
job=$(submit "awk 'BEGIN { for (i=0; i<100000; i++) print i }'")
wait_done "$job"
[ "$(clean_env "$slumber" logs "$job" | wc -l | tr -d ' ')" -eq 100000 ]
[ "$(cat "$smoke_dir/state/resumed")" = resumed ]
if grep -q '"PATH"' "$smoke_dir/state/jobs/$job/request.json"; then exit 1; fi
clean_env sh "$repo/uninstall.sh"
[ ! -e "$slumber" ]
[ -d "$smoke_dir/state" ]
clean_env sh "$repo/install.sh" --from "$binary"
clean_env sh "$repo/uninstall.sh" --purge --yes
[ ! -e "$slumber" ]
[ ! -e "$smoke_dir/state" ]
[ -z "$(ls -A "$smoke_dir/home")" ]
printf 'PASS: clean environment install, init, failure, signal, 100k lines, wake-up, credential cleanup, uninstall and purge.\n'
