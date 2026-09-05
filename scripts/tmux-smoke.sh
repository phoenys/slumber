#!/bin/sh
set -eu
binary=${1:?provide an absolute path to slumber}
qa_dir=$(mktemp -d /tmp/slumber-tmux.XXXXXX)
tmux_name=slumber-qa-$$
cleanup() {
    env SLUMBER_HOME="$qa_dir/state" SLUMBER_SOCKET="$qa_dir/socket" "$binary" daemon stop || return
    tmux -L "$tmux_name" kill-server 2>/dev/null || true
    rm -r -- "$qa_dir"
}
trap cleanup EXIT HUP INT TERM
tmux -L "$tmux_name" -f /dev/null new-session -d -s qa -x 120 -y 40 'sleep 60'
pane=$(tmux -L "$tmux_name" display-message -p '#{pane_id}')
tmux_env=$(tmux -L "$tmux_name" display-message -p '#{socket_path},#{pid},0')
# shellcheck disable=SC2016 # Expanded by the wake-up child.
env SLUMBER_HOME="$qa_dir/state" SLUMBER_SOCKET="$qa_dir/socket" TMUX="$tmux_env" TMUX_PANE="$pane" \
    "$binary" run --resume-template 'printf complete > "$SLUMBER_HOME/complete"' \
    'echo qa-stdout; echo qa-stderr >&2; sleep 3'
[ "$(tmux -L "$tmux_name" list-panes | wc -l | tr -d ' ')" -eq 2 ]
tail_pane=$(tmux -L "$tmux_name" list-panes -F '#{pane_id}' | tail -1)
sleep 1
tmux -L "$tmux_name" capture-pane -p -t "$tail_pane" | grep qa-stdout
tmux -L "$tmux_name" capture-pane -p -t "$tail_pane" | grep qa-stderr
attempt=0
while [ ! -e "$qa_dir/state/complete" ] && [ "$attempt" -lt 100 ]; do
    sleep 0.1
    attempt=$((attempt + 1))
done
[ -e "$qa_dir/state/complete" ]
[ "$(tmux -L "$tmux_name" list-panes | wc -l | tr -d ' ')" -eq 1 ]
printf 'PASS: real tmux panes 1 -> 2 -> 1, both log streams visible.\n'
