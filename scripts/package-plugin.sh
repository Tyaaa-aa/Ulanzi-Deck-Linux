#!/usr/bin/env bash
# Build a plugin and pack it into dist/plugins/<id>-<version>.ulanziplugin
# (a ZIP with plugin.toml, plugin.wasm and, if the plugin has them, assets/).
#
#   scripts/package-plugin.sh plugins/obs
#
# Prints the path of the package. Needs cargo with the wasm32-wasip2 target and either zip or
# python3. Install the result with: ulanzictl plugin install <package>
set -euo pipefail

if [[ $# -ne 1 || ! -d $1 ]]; then
    echo "usage: $0 <plugin-dir>" >&2
    exit 2
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
plugin="$(cd "$1" && pwd)"
manifest="$plugin/plugin.toml"
[[ -f $manifest ]] || { echo "$plugin has no plugin.toml" >&2; exit 1; }
[[ -f $plugin/Cargo.toml ]] || { echo "$plugin has no Cargo.toml" >&2; exit 1; }

# A top-level string key of plugin.toml (before the first [table]).
manifest_value() {
    sed -n '/^\[/q;s/^'"$1"'[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$manifest" | head -n 1
}
id="$(manifest_value id)"
version="$(manifest_value version)"
[[ -n $id && -n $version ]] || { echo "plugin.toml needs id and version" >&2; exit 1; }
if [[ ! $id =~ ^[A-Za-z0-9._-]+$ || ! $version =~ ^[A-Za-z0-9._+-]+$ ]]; then
    echo "id or version has characters that do not belong in a file name" >&2
    exit 1
fi

# The package name from the plugin's Cargo.toml ([package] name).
crate="$(sed -n '/^\[package\]/,/^\[/{s/^name[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p}' "$plugin/Cargo.toml" | head -n 1)"
[[ -n $crate ]] || { echo "no package name in $plugin/Cargo.toml" >&2; exit 1; }

(cd "$root" && cargo build --release --target wasm32-wasip2 -p "$crate") >&2
wasm="${CARGO_TARGET_DIR:-$root/target}/wasm32-wasip2/release/${crate//-/_}.wasm"
[[ -f $wasm ]] || { echo "the build did not produce $wasm" >&2; exit 1; }

out_dir="$root/dist/plugins"
out="$out_dir/$id-$version.ulanziplugin"
mkdir -p "$out_dir"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp "$manifest" "$stage/plugin.toml"
cp "$wasm" "$stage/plugin.wasm"
entries=(plugin.toml plugin.wasm)
if [[ -d $plugin/assets ]]; then
    cp -r "$plugin/assets" "$stage/assets"
    entries+=(assets)
fi

rm -f "$out"
if command -v zip >/dev/null 2>&1; then
    (cd "$stage" && zip -X -r -q "$out" "${entries[@]}")
elif command -v python3 >/dev/null 2>&1; then
    (cd "$stage" && python3 -m zipfile -c "$out" "${entries[@]}")
else
    echo "this needs zip or python3" >&2
    exit 1
fi
echo "$out"
