#!/usr/bin/env bash
# Module: build. Purpose: reproducible release publication. Created: 2026-10-19.
# Architecture: only Binaries/.build owns disposable intermediates.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$project_root"
exec 9< "$project_root/build.sh"
flock -x 9
if [[ -L Binaries || -L Binaries/.build ]]; then
    echo 'Refusing a symlink build directory.' >&2
    exit 1
fi
for dependency in cargo pkg-config cc; do
    command -v "$dependency" >/dev/null || { echo "Missing build dependency: $dependency" >&2; exit 1; }
done
pkg-config --atleast-version=3.22 gtk+-3.0 || {
    echo 'Install build-essential pkg-config libgtk-3-dev libx11-dev and Rust >=1.88.' >&2
    exit 1
}
mkdir -p Binaries/.build
printf '%s\n' 'snappy-owned-build-v1' > Binaries/.build/.owner
export CARGO_TARGET_DIR="$project_root/Binaries/.build/cargo"
cargo build --locked --release "$@"
install -m 755 "$CARGO_TARGET_DIR/release/snappy" Binaries/.build/snappy.new
mv -f -- Binaries/.build/snappy.new Binaries/snappy
printf 'Built %s\n' "$project_root/Binaries/snappy"
