#!/usr/bin/env bash
# Module: package. Purpose: publish amd64 Debian packages for Ubuntu, Linux Mint and Debian.
# Created: 2026-10-19. Architecture: shares build locking and owned scratch output.
# Usage: ./package.sh [ubuntu|mint|debian|all]   (default: all)
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$project_root"
release=24
case "${1:-all}" in
    all) targets='ubuntu mint debian' ;;
    ubuntu|mint|debian) targets=$1 ;;
    *) echo 'Usage: ./package.sh [ubuntu|mint|debian|all]' >&2; exit 1 ;;
esac
exec 9< "$project_root/build.sh"
flock -x 9
if [[ -L Binaries || -L Binaries/.build ]]; then
    echo 'Refusing a symlink build directory.' >&2
    exit 1
fi
for dependency in dpkg-deb ffmpeg readelf objdump; do
    command -v "$dependency" >/dev/null || { echo "Missing package tool: $dependency" >&2; exit 1; }
done
[[ -x Binaries/snappy ]] || { echo 'Run ./build.sh first.' >&2; exit 1; }
elf_header=$(LC_ALL=C readelf -h Binaries/snappy)
[[ $elf_header == *X86-64* ]] || {
    echo 'These packages target amd64; build an x86-64 executable first.' >&2
    exit 1
}
# The declared libc requirement is read from the executable, never assumed.
glibc_required=$(LC_ALL=C objdump -T Binaries/snappy | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -t. -k1,1n -k2,2n -u | tail -1)
[[ $glibc_required =~ ^[0-9]+\.[0-9]+$ ]] || { echo 'Cannot read the glibc requirement.' >&2; exit 1; }
version=$(Binaries/snappy --version)
version=${version#Snappy }
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Invalid executable version.' >&2; exit 1; }
mkdir -p Binaries/.build
printf '%s\n' 'snappy-owned-build-v1' > Binaries/.build/.owner
stage=$(mktemp -d "$project_root/Binaries/.build/deb.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/DEBIAN" "$stage/usr/share/doc/snappy-capture"
install -Dm755 Binaries/snappy "$stage/usr/bin/snappy"
install -Dm755 packaging/launch "$stage/usr/lib/snappy/launch"
install -Dm644 packaging/io.github.snappy.Capture.desktop "$stage/usr/share/applications/io.github.snappy.Capture.desktop"
install -Dm644 packaging/snappy-setup.desktop "$stage/etc/xdg/autostart/snappy-setup.desktop"
install -Dm644 packaging/copyright "$stage/usr/share/doc/snappy-capture/copyright"
icon_dir=$stage/usr/share/icons/hicolor/512x512/apps
mkdir -p "$icon_dir"
ffmpeg -nostdin -v error -i assets/snappy.png -vf scale=512:512 -frames:v 1 "$icon_dir/io.github.snappy.Capture.png"
extension_dir=$stage/usr/share/gnome-shell/extensions/snappy@snappy.local
install -Dm644 integration/snappy@snappy.local/metadata.json "$extension_dir/metadata.json"
install -Dm644 integration/snappy@snappy.local/extension.js "$extension_dir/extension.js"
install -m755 packaging/postinst "$stage/DEBIAN/postinst"
installed_size=$(du -sk "$stage" | cut -f1)
# Ubuntu 24.04, Linux Mint 22 (Ubuntu 24.04 based) and Debian 13 share the time64
# library names; they differ in their glibc baseline and preferred portal backend.
for target in $targets; do
    case "$target" in
        ubuntu)
            baseline=2.39; minimum='Ubuntu 24.04 or newer'
            portals='xdg-desktop-portal-gnome | xdg-desktop-portal-gtk | xdg-desktop-portal-xapp' ;;
        mint)
            baseline=2.39; minimum='Linux Mint 22 or newer'
            portals='xdg-desktop-portal-xapp | xdg-desktop-portal-gtk | xdg-desktop-portal-gnome' ;;
        debian)
            baseline=2.41; minimum='Debian 13 or newer'
            portals='xdg-desktop-portal-gnome | xdg-desktop-portal-gtk | xdg-desktop-portal-xapp' ;;
    esac
    if dpkg --compare-versions "$glibc_required" gt "$baseline"; then
        echo "The executable needs glibc $glibc_required, newer than $minimum ships ($baseline); rebuild on an older host." >&2
        exit 1
    fi
    package_version="${version}-${release}${target}1"
    cat > "$stage/DEBIAN/control" <<CONTROL
Package: snappy-capture
Version: $package_version
Section: graphics
Priority: optional
Architecture: amd64
Maintainer: Snappy maintainers <snappy@localhost>
Installed-Size: $installed_size
Depends: libc6 (>= $glibc_required), libgcc-s1, libgtk-3-0t64 (>= 3.22), libgdk-pixbuf-2.0-0, libatk1.0-0t64, libcairo2, libglib2.0-0t64, libx11-6, ffmpeg, yaru-theme-gtk, dconf-gsettings-backend, gstreamer1.0-tools, gstreamer1.0-pipewire, gstreamer1.0-plugins-base, gstreamer1.0-plugins-good, gstreamer1.0-libav, xdg-desktop-portal
Recommends: $portals, pipewire-pulse | pulseaudio, gstreamer1.0-plugins-bad, va-driver-all | mesa-va-drivers
Description: Snappy screenshot and desktop video capture
 A GTK 3 screen capture tool with on-screen area selection, desktop audio,
 screenshot formats and video export presets. Supports X11 and Wayland.
 Installation configures Print Screen for the invoking desktop user on GNOME
 and Cinnamon, with first-launch and login fallbacks and a reversible backup.
 Built for $minimum on amd64.
CONTROL
    output="Binaries/snappy-capture_${package_version}_amd64.deb"
    dpkg-deb --root-owner-group -Zxz --build "$stage" "$stage.deb"
    mv -f -- "$stage.deb" "$output"
    printf 'Built %s/%s\n' "$project_root" "$output"
done
