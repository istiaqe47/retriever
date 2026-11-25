#!/bin/sh
# Signs Retriever's Firefox add-on with your own addons.mozilla.org account, as an unlisted
# add-on (only you get it; Mozilla checks it, usually within minutes). Firefox installs only
# signed add-ons. You need, once:
#   1. a free account at https://addons.mozilla.org
#   2. its API key: https://addons.mozilla.org/developers/addon/api/key/  (JWT issuer and secret)
#   3. Node.js: sudo pacman -S nodejs npm
# Then:  sh sign-firefox.sh <JWT issuer> <JWT secret>   (again after every Retriever update)
# The signed .xpi lands in ~/Downloads: open it with Firefox (or drop it on about:addons).
set -eu
key=${1:?your JWT issuer}
secret=${2:?your JWT secret}
here=$(cd "$(dirname "$0")" && pwd)
xpi=$here/retriever-firefox.xpi
# Run from elsewhere (downloaded on its own): the installed copy.
[ -f "$xpi" ] || xpi=/usr/share/retriever/extension/retriever-firefox.xpi
[ -f "$xpi" ] || { echo "No retriever-firefox.xpi: install Retriever first." >&2; exit 1; }
src=$(mktemp -d)
trap 'rm -rf "$src"' EXIT
bsdtar -xf "$xpi" -C "$src"
# Mozilla signs each version once: a fourth number (minutes since 1970) makes every signing a
# new version, so this can be run again after each update. The extension compares only the
# first two numbers with the app's, so it never reads as newer than Retriever.
build=$(( $(date +%s) / 60 ))
sed -i -E "s/(\"version\": *\"[0-9]+\.[0-9]+\.[0-9]+)[0-9.]*\"/\1.$build\"/" "$src/manifest.json"
grep -o '"version": *"[^"]*"' "$src/manifest.json"
npx --yes web-ext sign --channel=unlisted --source-dir "$src" \
    --api-key "$key" --api-secret "$secret" --artifacts-dir "$HOME/Downloads"
echo "Signed: open the .xpi in ~/Downloads with Firefox."
