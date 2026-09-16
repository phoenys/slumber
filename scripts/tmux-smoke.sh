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
# A newer session must not steal the fallback when an agent strips TMUX_PANE.
tmux -L "$tmux_name" new-session -d -s decoy 'sleep 60'
for mode in explicit session-fallback; do
if [ "$mode" = explicit ]; then
    export TMUX_PANE="$pane"
else
    unset TMUX_PANE
fi
# shellcheck disable=SC2016 # Expanded by the wake-up child.
env SLUMBER_HOME="$qa_dir/state" SLUMBER_SOCKET="$qa_dir/socket" TMUX="$tmux_env" QA_MODE="$mode" \
    "$binary" run --resume-template 'printf complete > "$SLUMBER_HOME/$QA_MODE"' \
    'echo qa-stdout; echo qa-stderr >&2; sleep 3'
[ "$(tmux -L "$tmux_name" list-panes -t qa | wc -l | tr -d ' ')" -eq 2 ]
[ "$(tmux -L "$tmux_name" list-panes -t decoy | wc -l | tr -d ' ')" -eq 1 ]
tail_pane=$(tmux -L "$tmux_name" list-panes -t qa -F '#{pane_id}' | tail -1)
sleep 1
tmux -L "$tmux_name" capture-pane -p -t "$tail_pane" | grep qa-stdout
tmux -L "$tmux_name" capture-pane -p -t "$tail_pane" | grep qa-stderr
attempt=0
while [ ! -e "$qa_dir/state/$mode" ] && [ "$attempt" -lt 100 ]; do
    sleep 0.1
    attempt=$((attempt + 1))
done
[ -e "$qa_dir/state/$mode" ]
[ "$(tmux -L "$tmux_name" list-panes -t qa | wc -l | tr -d ' ')" -eq 1 ]
printf 'PASS: %s real tmux panes 1 -> 2 -> 1, both log streams visible.\n' "$mode"
done

# Start a fresh daemon from a real PTY, then destroy the submitting terminal
# and the entire isolated tmux server while its delegated task is still active.
env SLUMBER_HOME="$qa_dir/state" SLUMBER_SOCKET="$qa_dir/socket" "$binary" daemon stop
# shellcheck disable=SC2016 # Positional arguments and state are expanded in the child shells.
source_pane=$(tmux -L "$tmux_name" new-window -d -t qa -P -F '#{pane_id}' \
    env SLUMBER_HOME="$qa_dir/state" SLUMBER_SOCKET="$qa_dir/socket" \
    sh -c '"$1" run --resume-template "$2" "$3" > "$SLUMBER_HOME/terminal-submission" && touch "$SLUMBER_HOME/terminal-accepted"' \
    sh "$binary" 'printf complete > "$SLUMBER_HOME/terminal-complete"' \
    'printf before-close; attempt=0; while [ ! -f "$SLUMBER_HOME/release-task" ] && [ "$attempt" -lt 100 ]; do sleep 0.1; attempt=$((attempt + 1)); done; test -f "$SLUMBER_HOME/release-task" || exit 42; printf after-close; printf terminal-stderr >&2; exit 7')
attempt=0
while [ ! -f "$qa_dir/state/terminal-accepted" ] && [ "$attempt" -lt 100 ]; do
    sleep 0.1
    attempt=$((attempt + 1))
done
[ -f "$qa_dir/state/terminal-accepted" ]
job=$(awk '/^Submitted / {print $2}' "$qa_dir/state/terminal-submission")
grep -q '"exit_status": null' "$qa_dir/state/jobs/$job/meta.json"
attempt=0
while tmux -L "$tmux_name" list-panes -a -F '#{pane_id}' | grep -qx "$source_pane"; do
    [ "$attempt" -lt 100 ]
    sleep 0.1
    attempt=$((attempt + 1))
done
tmux -L "$tmux_name" kill-server
touch "$qa_dir/state/release-task"
attempt=0
while [ ! -f "$qa_dir/state/terminal-complete" ] && [ "$attempt" -lt 100 ]; do
    sleep 0.1
    attempt=$((attempt + 1))
done
[ -f "$qa_dir/state/terminal-complete" ]
[ "$(cat "$qa_dir/state/jobs/$job/stdout.log")" = before-closeafter-close ]
[ "$(cat "$qa_dir/state/jobs/$job/stderr.log")" = terminal-stderr ]
grep -q '"Exited": 7' "$qa_dir/state/jobs/$job/meta.json"
printf 'PASS: task, dual logs, exit 7 and wake-up survive submitting PTY exit and tmux server destruction.\n'
