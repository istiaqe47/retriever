#!/bin/sh
# Signs Retriever's Firefox add-on with your own addons.mozilla.org account, as an unlisted
# add-on (only you get it; Mozilla checks it, usually within minutes). Firefox installs only
# signed add-ons. You need, once:
#   1. a free account at https://addons.mozilla.org
#   2. its API key: https://addons.mozilla.org/developers/addon/api/key/  (JWT issuer and secret)
#   3. Node.js: sudo pacman -S nodejs npm
# Then:  sh sign-firefox.sh <JWT issuer> <JWT secret>
# The signed .xpi lands in ~/Downloads: open it with Firefox (or drop it on about:addons).
set -eu
key=${1:?your JWT issuer}
secret=${2:?your JWT secret}
here=$(cd "$(dirname "$0")" && pwd)
src=$(mktemp -d)
trap 'rm -rf "$src"' EXIT
bsdtar -xf "$here/retriever-firefox.xpi" -C "$src"
npx --yes web-ext sign --channel=unlisted --source-dir "$src" \
    --api-key "$key" --api-secret "$secret" --artifacts-dir "$HOME/Downloads"
echo "Signed: open the .xpi in ~/Downloads with Firefox."
