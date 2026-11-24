#!/bin/sh
# Builds retriever_<version>_<arch>.deb for Debian, Ubuntu and their family (Linux Mint, Pop!_OS,
# Zorin, elementary…). Needs GTK 4.14 and libadwaita 1.5 or newer: Ubuntu 24.04+, Debian 13+.
#
# Once:   sudo apt install build-essential cargo rustc pkg-config cmake clang golang git \
#                          libgtk-4-dev libadwaita-1-dev libarchive-tools dpkg-dev
#         (or a newer Rust from https://rustup.rs if the distribution's is older than 1.85)
# Then:   sh packaging/debian/build-deb.sh          (from the source folder)
# The .deb lands in the source folder: sudo apt install ./retriever_*.deb
set -eu
top=$(cd "$(dirname "$0")/../.." && pwd)
cd "$top"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(dpkg --print-architecture)
target=${CARGO_TARGET_DIR:-$top/target}
export CARGO_TARGET_DIR="$target"

cargo build --release --locked -p retriever -p mdm-nmhost ${CARGO_FLAGS:-}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sh extension/build.sh "$work/extension" >/dev/null

root=$work/root
id=io.github.retriever.Retriever
install -Dm755 "$target/release/retriever" "$root/usr/bin/retriever"
install -Dm755 "$target/release/retriever-host" "$root/usr/bin/retriever-host"
strip --strip-unneeded "$root/usr/bin/retriever" "$root/usr/bin/retriever-host" 2>/dev/null || true
# The browser host's manifests: Firefox, Chrome, Chromium, Edge (the others get theirs in the
# home folder when the app starts). /usr/lib64 isn't Debian's: Firefox reads /usr/lib.
"$target/release/retriever-host" --package-manifests "$root"
rm -rf "$root/usr/lib64"
install -Dm644 "crates/retriever/data/$id.desktop" "$root/usr/share/applications/$id.desktop"
install -Dm644 "crates/retriever/data/$id.metainfo.xml" "$root/usr/share/metainfo/$id.metainfo.xml"
install -Dm644 "crates/retriever/data/$id.svg" "$root/usr/share/icons/hicolor/scalable/apps/$id.svg"
install -Dm644 "crates/retriever/data/$id-symbolic.svg" "$root/usr/share/icons/hicolor/symbolic/apps/$id-symbolic.svg"
install -d "$root/usr/share/retriever/extension"
cp -r "$work/extension/chromium" "$root/usr/share/retriever/extension/chromium"
install -Dm644 "$work/extension/retriever-firefox.xpi" "$root/usr/share/retriever/extension/retriever-firefox.xpi"
install -Dm755 extension/sign-firefox.sh "$root/usr/share/retriever/extension/sign-firefox.sh"
install -Dm644 LICENSE "$root/usr/share/doc/retriever/copyright"

# Depends: the libraries the binaries link to, as dpkg works them out.
mkdir -p "$work/debian"
printf 'Source: retriever\n\nPackage: retriever\nArchitecture: any\n' > "$work/debian/control"
libs=$(cd "$work" && dpkg-shlibdeps -O -e"$root/usr/bin/retriever" -e"$root/usr/bin/retriever-host" 2>/dev/null \
    | sed -n 's/^shlibs:Depends=//p')
size=$(du -sk "$root" | cut -f1)

mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: retriever
Version: $version-1
Architecture: $arch
Maintainer: Retriever contributors <retriever@users.noreply.github.com>
Installed-Size: $size
Depends: $libs
Recommends: yt-dlp, ffmpeg
Suggests: 7zip | p7zip-full
Section: net
Priority: optional
Description: download manager for GNOME
 Retriever downloads files with many connections at once, videos (with
 yt-dlp), torrents and magnet links. It grabs links from pages, extracts
 archives, schedules downloads and limits speed, and takes over the
 browser's downloads with its extension for Firefox and Chromium browsers.
 Built with GTK 4 and libadwaita, it follows the system's light and dark
 style and accent colour.
CONTROL
(cd "$root" && find etc -type f | sed 's|^|/|') > "$root/DEBIAN/conffiles"

out="$top/retriever_${version}-1_${arch}.deb"
dpkg-deb --root-owner-group -Zxz --build "$root" "$out"
echo "built $out"
