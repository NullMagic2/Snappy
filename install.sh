#!/usr/bin/env bash
# Module: desktop_install. Purpose: user-local binary, launcher and dock icon installation.
# Created: 2026-10-19. Architecture: the executable owns shared desktop identity setup.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
for argument in "$@"; do
    case "$argument" in
        --autostart|--no-default) ;;
        *) echo 'Usage: ./install.sh [--autostart] [--no-default]' >&2; exit 2 ;;
    esac
done
if [[ ${EUID} -eq 0 ]]; then
    echo 'Run install.sh as your desktop user, without sudo.' >&2
    exit 1
fi
[[ -x "$project_root/Binaries/snappy" ]] || { echo 'Run ./build.sh first.' >&2; exit 1; }
install -Dm755 "$project_root/Binaries/snappy" "$HOME/.local/bin/snappy"
"$HOME/.local/bin/snappy" --install "$@"
