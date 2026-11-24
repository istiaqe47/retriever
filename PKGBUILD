# Build and install:  makepkg -si   (run in this folder)
pkgname=retriever
pkgver=1.0.0
pkgrel=1
pkgdesc="A download manager for GNOME (GTK 4 / libadwaita): files, videos, torrents, browser integration"
arch=('x86_64' 'aarch64')
license=('GPL-3.0-or-later')
depends=('gtk4' 'libadwaita' 'glib2' 'gcc-libs' 'glibc' 'hicolor-icon-theme' 'yt-dlp' 'ffmpeg')
optdepends=('deno: all of YouTube'"'"'s qualities (yt-dlp needs a JavaScript runtime there)'
            '7zip: extracting 7z and RAR archives')
# The original app's packages install the same browser host and names.
conflicts=('mdm-gtk' 'downpour')
# Its earlier names: upgrading from them keeps the download list (moved at the first start).
replaces=('mdm-gtk' 'downpour')
# cmake, clang and go: BoringSSL, for the browser-like connection that gets past bot checks.
makedepends=('cargo' 'pkgconf' 'cmake' 'clang' 'go' 'git')
options=('!lto' '!debug')
source=()
sha256sums=()

_src() { echo "$startdir"; }

prepare() {
    cd "$(_src)"
    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$(_src)"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR="$srcdir/target"
    cargo build --release --frozen -p retriever -p mdm-nmhost
    # The browser extension: unpacked for Chromium browsers, an .xpi for Firefox (to sign).
    sh extension/build.sh "$srcdir/extension"
}

check() {
    cd "$(_src)"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR="$srcdir/target"
    cargo test --release --frozen -p retriever
}

package() {
    cd "$(_src)"
    local id=io.github.retriever.Retriever
    install -Dm755 "$srcdir/target/release/retriever" "$pkgdir/usr/bin/retriever"
    # The browser extension's helper (its native messaging host).
    install -Dm755 "$srcdir/target/release/retriever-host" "$pkgdir/usr/bin/retriever-host"
    # Its manifests for Firefox, Chrome, Chromium and Edge (Brave, Vivaldi, Zen, LibreWolf…
    # get theirs in the home folder when the app starts).
    "$srcdir/target/release/retriever-host" --package-manifests "$pkgdir"
    # /usr/lib64 is /usr/lib on Arch: its copy would conflict with the filesystem package.
    rm -rf "$pkgdir/usr/lib64"
    install -Dm644 "crates/retriever/data/$id.desktop" "$pkgdir/usr/share/applications/$id.desktop"
    install -Dm644 "crates/retriever/data/$id.metainfo.xml" "$pkgdir/usr/share/metainfo/$id.metainfo.xml"
    install -Dm644 "crates/retriever/data/$id.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/$id.svg"
    install -Dm644 "crates/retriever/data/$id-symbolic.svg" "$pkgdir/usr/share/icons/hicolor/symbolic/apps/$id-symbolic.svg"
    # The extension, and the script that signs its Firefox copy with your addons.mozilla.org key.
    install -d "$pkgdir/usr/share/retriever/extension"
    cp -r "$srcdir/extension/chromium" "$pkgdir/usr/share/retriever/extension/chromium"
    install -Dm644 "$srcdir/extension/retriever-firefox.xpi" "$pkgdir/usr/share/retriever/extension/retriever-firefox.xpi"
    install -Dm755 extension/sign-firefox.sh "$pkgdir/usr/share/retriever/extension/sign-firefox.sh"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
