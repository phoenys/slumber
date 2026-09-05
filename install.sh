#!/bin/sh
# Installs one binary. Never edits shell profiles or requires root.
set -eu

fail() { printf 'slumber install: %s\n' "$*" >&2; exit 1; }
install_dir=${SLUMBER_INSTALL_DIR:-"$HOME/.local/bin"}
version=${SLUMBER_VERSION:-latest}
source_binary=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --from) [ "$#" -ge 2 ] || fail '--from needs a local binary'; source_binary=$2; shift 2 ;;
        --version) [ "$#" -ge 2 ] || fail '--version needs a tag'; version=$2; shift 2 ;;
        --help) printf 'Usage: sh install.sh [--version vX.Y.Z] [--from /path/to/slumber]\nSLUMBER_INSTALL_DIR defaults to ~/.local/bin. No shell profiles are modified.\n'; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done
case "$install_dir" in /*) ;; *) fail 'SLUMBER_INSTALL_DIR must be absolute' ;; esac
[ "$(id -u)" -ne 0 ] || fail 'run as your normal user, without sudo'
case "$version" in latest|v[0-9]*) ;; *) fail 'version must be latest or a v-prefixed release tag' ;; esac
case "$version" in *[!a-zA-Z0-9.-]*) fail 'invalid version tag' ;; esac
case "$(uname -s)/$(uname -m)" in
    Darwin/arm64) target=aarch64-apple-darwin ;;
    Darwin/x86_64) target=x86_64-apple-darwin ;;
    Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    *) fail 'supported platforms: macOS and Linux, arm64 or x86_64' ;;
esac
mkdir -p "$install_dir"
[ ! -L "$install_dir/slumber" ] || fail 'destination is a symlink; remove it explicitly before installing'
install_tmp=$(mktemp "$install_dir/.slumber-install.XXXXXX")
trap 'rm -f "$install_tmp"' EXIT HUP INT TERM
if [ -n "$source_binary" ]; then
    [ -f "$source_binary" ] || fail 'local binary does not exist'
    cp "$source_binary" "$install_tmp"
else
    command -v curl >/dev/null 2>&1 || fail 'curl is required'
    if [ "$version" = latest ]; then
        url="https://github.com/phoenys/slumber/releases/latest/download/slumber-$target"
    else
        url="https://github.com/phoenys/slumber/releases/download/$version/slumber-$target"
    fi
    curl --fail --show-error --location --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 180 "$url" -o "$install_tmp" || fail 'download failed; check that the repository and release are public'
fi
chmod 755 "$install_tmp"
"$install_tmp" --version || fail 'binary cannot run on this system (check architecture and Linux glibc version)'
if [ -e "$install_dir/slumber" ]; then
    case "$("$install_dir/slumber" --version)" in 'slumber '*) ;; *) fail 'refusing to replace a different program named slumber' ;; esac
    "$install_dir/slumber" daemon stop || fail 'stop existing jobs and daemon before upgrading'
fi
mv -f "$install_tmp" "$install_dir/slumber"
printf 'Installed %s/slumber\n' "$install_dir"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) printf 'Add this directory to PATH for your shell: %s\n' "$install_dir" ;;
esac
resolved=$(command -v slumber || true)
if [ -n "$resolved" ] && [ "$resolved" != "$install_dir/slumber" ]; then
    printf 'Another slumber takes precedence in PATH: %s\nUse %s/slumber or adjust PATH.\n' "$resolved" "$install_dir"
fi
printf 'Next: slumber doctor\nNo shell profiles or agent instruction files were changed.\n'
