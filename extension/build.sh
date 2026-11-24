#!/bin/sh
# Builds the browser extension into $1: chromium/ (to load unpacked) and retriever-firefox.xpi
# (to sign once with sign-firefox.sh, then open in Firefox).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=${1:?where to build}
rm -rf "$out"
mkdir -p "$out/chromium" "$out/firefox"
files="_locales icons background.js page.js video.js popup.html popup.css popup.js popup-ui.js popup-downloads.js help.html"
for f in $files; do
    cp -r "$here/$f" "$out/chromium/"
    cp -r "$here/$f" "$out/firefox/"
done
cp "$here/manifest.json" "$out/chromium/manifest.json"
cp "$here/manifest.firefox.json" "$out/firefox/manifest.json"
# An .xpi is a zip of the add-on's files (bsdtar comes with makepkg's libarchive).
# Its entries named plainly (manifest.json, not ./manifest.json): Firefox finds no manifest in
# a zip whose names start with ./
(cd "$out/firefox" && bsdtar --format zip -cf "$out/retriever-firefox.xpi" $files manifest.json)
echo "built $out/chromium and $out/retriever-firefox.xpi"
