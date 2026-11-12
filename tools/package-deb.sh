#!/bin/sh
# Builds target/release/apolline-wall_VERSION_ARCH.deb: the daemon and the app, the systemd
# user unit, the desktop entry, the GNOME Shell extension, the Plasma wallpaper plugin, the
# sample shader and the licenses, for Debian 12 and Ubuntu 22.04 on. It builds the binaries
# first when they aren't there;
# apolline-scene has to sit next to this repository, as the path dependency says.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case $(uname -m) in
    x86_64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) echo "no Debian name for $(uname -m)" >&2; exit 1 ;;
esac
target=${CARGO_TARGET_DIR:-$root/target}
# PLASMA=no leaves out the KDE Plasma plugin, whose PipeWire bindings need PipeWire 1.0.
plasma=${PLASMA:-yes}
[ "$plasma" = no ] && features=--no-default-features || features=
[ -x "$target/release/apolline-wall" ] && [ -x "$target/release/apolline-wallpapers" ] ||
    cargo build --release --locked --workspace $features
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
pkg=$work/apolline-wall
extension=apolline-wall@pisosenie.github.io

install -Dm755 "$target/release/apolline-wall" "$pkg/usr/bin/apolline-wall"
install -Dm755 "$target/release/apolline-wallpapers" "$pkg/usr/bin/apolline-wallpapers"
install -Dm644 packaging/apolline-wall.service "$pkg/usr/lib/systemd/user/apolline-wall.service"
install -Dm644 packaging/apolline-wallpapers.desktop "$pkg/usr/share/applications/apolline-wallpapers.desktop"
install -Dm644 tests/shaders/hills.frag "$pkg/usr/share/apolline-wall/wallpapers/hills.frag"
for file in metadata.json extension.js; do
    install -Dm644 "gnome/$extension/$file" "$pkg/usr/share/gnome-shell/extensions/$extension/$file"
done
plugin=io.github.pisosenie.apolline-wall
[ "$plasma" = no ] || for file in metadata.json contents/ui/main.qml contents/ui/Covered.qml; do
    install -Dm644 "plasma/$plugin/$file" "$pkg/usr/share/plasma/wallpapers/$plugin/$file"
done
install -Dm644 README.md "$pkg/usr/share/doc/apolline-wall/README.md"
{
    echo "apolline-wall is under MIT or Apache-2.0, at your choice; Geist, built into the app, under the OFL."
    for license in LICENSE-MIT LICENSE-APACHE app/assets/fonts/OFL.txt; do
        printf '\n== %s ==\n\n' "$(basename "$license")"
        cat "$license"
    done
} > "$pkg/usr/share/doc/apolline-wall/copyright"

mkdir -p "$pkg/DEBIAN"
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: apolline-wall
Version: $version
Architecture: $arch
Maintainer: Danil Zorin <45851636+Pisosenie@users.noreply.github.com>
Installed-Size: $(du -ks "$pkg/usr" | cut -f1)
Depends: libc6 (>= 2.35), libgcc-s1, libwayland-client0, libegl1, libxkbcommon0, libpipewire-0.3-0 | libpipewire-0.3-0t64
Recommends: libmpv2 | libmpv1, ffmpeg, libvulkan1
Suggests: libx11-6, libx11-xcb1, libxcb1, steamcmd, pulseaudio-utils
Section: x11
Priority: optional
Homepage: https://github.com/Pisosenie/apolline-wall
Description: live wallpapers for any Linux desktop, Wallpaper Engine's included
 Pictures, videos, Shadertoy shaders and Wallpaper Engine's wallpapers on every
 monitor, crossfading and costing nothing while unseen. A daemon draws them on
 wlroots compositors, KDE Plasma (through its wallpaper plugin), GNOME (through
 its Shell extension, or Xwayland) and X11; the Wallpapers app browses your
 folders, Wallpaper Engine's library and the Steam Workshop.
CONTROL
dpkg-deb --root-owner-group --build "$pkg" "$target/release/apolline-wall_${version}_${arch}.deb" > /dev/null
echo "$target/release/apolline-wall_${version}_${arch}.deb"
