#!/bin/sh
# Vendor the Lucide (ISC) and Simple Icons (CC0) sets into crates/ulanzi-core/assets/.
# Usage: tools/fetch-icons.sh
set -eu

LUCIDE_VERSION=1.61.0
SIMPLE_ICONS_VERSION=16.35.0
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

# Simple Icons (CC0 data; the logos remain trademarks of their owners).
brands="$repo/crates/ulanzi-core/assets/brands"
curl -fsSL "https://registry.npmjs.org/simple-icons/-/simple-icons-$SIMPLE_ICONS_VERSION.tgz" -o "$work/si.tgz"
mkdir "$work/si"
tar -xzf "$work/si.tgz" -C "$work/si" package/icons package/LICENSE.md
mkdir -p "$brands"
find "$brands" -name '*.svg' -delete
cp "$work"/si/package/icons/*.svg "$brands/"
cp "$work/si/package/LICENSE.md" "$brands/LICENSE.md"
echo "$SIMPLE_ICONS_VERSION" > "$brands/VERSION"
echo "simple-icons $SIMPLE_ICONS_VERSION: $(ls "$brands" | grep -c '\.svg$') icons"
