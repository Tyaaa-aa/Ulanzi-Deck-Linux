#!/bin/sh
# Vendor the Lucide icon set (ISC) into crates/ulanzi-core/assets/lucide.
# Usage: tools/fetch-icons.sh
set -eu

LUCIDE_VERSION=1.61.0
repo=$(cd "$(dirname "$0")/.." && pwd)
dest="$repo/crates/ulanzi-core/assets/lucide"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL "https://registry.npmjs.org/lucide-static/-/lucide-static-$LUCIDE_VERSION.tgz" -o "$work/lucide.tgz"
mkdir "$work/x"
tar -xzf "$work/lucide.tgz" -C "$work/x" package/icons package/LICENSE

mkdir -p "$dest"
find "$dest" -name '*.svg' -delete
cp "$work"/x/package/icons/*.svg "$dest/"
cp "$work/x/package/LICENSE" "$dest/LICENSE"
echo "$LUCIDE_VERSION" > "$dest/VERSION"
echo "lucide-static $LUCIDE_VERSION: $(ls "$dest" | grep -c '\.svg$') icons"
