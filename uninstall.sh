#!/bin/sh
set -eu
fail() { printf 'slumber uninstall: %s\n' "$*" >&2; exit 1; }
install_dir=${SLUMBER_INSTALL_DIR:-"$HOME/.local/bin"}
state_dir=${SLUMBER_HOME:-"$HOME/.slumber"}
purge=false
confirmed=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --purge) purge=true ;;
        --yes) confirmed=true ;;
        --help) printf 'Usage: sh uninstall.sh [--purge [--yes]]\nDefault: remove binary, retain state. --purge permanently deletes local configuration and logs. Remote logs and project instruction files are left for manual removal.\n'; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done
[ "$(id -u)" -ne 0 ] || fail 'run as your normal user, without sudo'
case "$install_dir" in /*) ;; *) fail 'SLUMBER_INSTALL_DIR must be absolute' ;; esac
binary=$install_dir/slumber
[ ! -L "$binary" ] || fail 'binary is a symlink; remove it explicitly'
[ -f "$binary" ] || fail "binary not found at $binary; set SLUMBER_INSTALL_DIR to its directory"
case "$("$binary" --version)" in 'slumber '*) ;; *) fail 'refusing to remove a different program' ;; esac
if "$purge" && [ -e "$state_dir" ]; then
    case "$state_dir" in /*) ;; *) fail 'SLUMBER_HOME must be absolute for --purge' ;; esac
    [ ! -L "$state_dir" ] || fail 'refusing to purge a symlink'
    state_dir=$(cd "$state_dir" && pwd -P)
    user_home=$(cd "$HOME" && pwd -P)
    case "$state_dir" in /|"$user_home"|/tmp|/var|/usr|/usr/local|"$install_dir"|"$(pwd -P)") fail 'refusing to purge a broad directory' ;; esac
    case "$user_home/" in "$state_dir/"*) fail 'refusing to purge an ancestor of your home' ;; esac
    case "$(pwd -P)/" in "$state_dir/"*) fail 'refusing to purge an ancestor of the working directory' ;; esac
    [ ! -e "$state_dir/.git" ] || fail 'refusing to purge a Git checkout'
    [ -f "$state_dir/.slumber-state" ] || fail 'state marker missing; inspect and remove legacy state manually'
    [ ! -L "$state_dir/.slumber-state" ] || fail 'state marker is a symlink'
    [ "$(cat "$state_dir/.slumber-state")" = slumber-state-v1 ] || fail 'invalid state marker'
    if ! "$confirmed"; then
        printf 'Permanently delete configuration, environment snapshots and logs at %s? Type DELETE: ' "$state_dir"
        read -r answer < /dev/tty || fail 'use --yes for noninteractive purge'
        [ "$answer" = DELETE ] || fail 'cancelled'
    fi
fi
"$binary" daemon stop || fail 'jobs or wake-ups are active; wait before uninstalling'
if "$purge" && [ -e "$state_dir" ]; then
    rm -r -- "$state_dir"
    printf 'Permanently deleted local state at %s\n' "$state_dir"
fi
rm -f -- "$binary"
printf 'Removed %s\n' "$binary"
if ! "$purge"; then printf 'Retained local state at %s\n' "$state_dir"; fi
printf 'Remote logs and project instruction files are unchanged.\n'
