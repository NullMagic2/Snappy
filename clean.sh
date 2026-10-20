#!/usr/bin/env bash
# Module: cleanup. Purpose: remove owned intermediates, retain binaries and source.
# Created: 2026-10-19. Architecture: shares the build.sh lock and output boundary.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$project_root"
exec 9< "$project_root/build.sh"
flock -x 9
if [[ -L Binaries || -L Binaries/.build ]]; then
    echo 'Refusing a symlink build directory.' >&2
    exit 1
fi
if [[ -d Binaries/.build ]]; then
    if [[ ! -f Binaries/.build/.owner ]] || [[ $(< Binaries/.build/.owner) != snappy-owned-build-v1 ]]; then
        echo 'Refusing to remove an unrecognized build directory.' >&2
        exit 1
    fi
    rm -rf --one-file-system -- Binaries/.build
fi
printf '%s\n' 'Cleaned build artifacts. Binaries and source files were preserved.'
