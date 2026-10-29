#!/usr/bin/env bash
# Screenshots of the plugin screens (Settings → Integrations, the OBS and Home Assistant settings
# pages, the action picker's plugin sections), taken in a throwaway sandbox with a pretend deck, the
# showcase profiles and the OBS and Home Assistant plugins installed. No passwords or tokens are
# set, so both plugins show their "needs a password/token" problems. Nothing of yours is used.
#
#   scripts/plugin-shots.sh OUT_DIR     → OUT_DIR/{integrations,integrations-obs,integrations-ha,action-picker,library,key-editor}.png
set -euo pipefail
[ $# -eq 1 ] || { echo "usage: $0 OUT_DIR" >&2; exit 2; }
out="$1"
cd "$(dirname "$0")/.."
mkdir -p "$out"
out="$(cd "$out" && pwd)"
cargo build -q --bins
# The OBS and Home Assistant plugins live in the plugins repo, next to this one by default.
plugins_repo="${ULANZI_PLUGINS_REPO:-$PWD/../Ulanzi-Deck-Plugins}"
[ -d "$plugins_repo/plugins/obs" ] || { echo "no plugins repo at $plugins_repo (set ULANZI_PLUGINS_REPO)" >&2; exit 1; }
(cd "$plugins_repo" && cargo build -q --release --target wasm32-wasip2 -p ulanzi-plugin-obs -p ulanzi-plugin-ha)
box="$(mktemp -d -t ulanzi-plugin-shots.XXXXXX)"
trap 'rm -rf "$box"' EXIT
cargo run -q -p ulanzi-core --example showcase -- "$box" >/dev/null
# A neutral look: stock icon theme and window buttons.
mkdir -p "$box/config/gtk-4.0"
printf '[Settings]\ngtk-icon-theme-name=Adwaita\ngtk-decoration-layout=:minimize,maximize,close\n' \
  >"$box/config/gtk-4.0/settings.ini"
# Developer Mode on, so the Install from Folder (developers)… row shows.
settings="$box/config/ulanzi-deck/settings.toml"
if grep -q '^developer_mode' "$settings"; then
  sed -i 's/^developer_mode = .*/developer_mode = true/' "$settings"
else
  sed -i '1i developer_mode = true' "$settings"
fi
# Home Assistant gets an address (an unused documentation address), so it stops at the missing token.
printf '\n[plugins.ha]\nenabled = true\n\n[plugins.ha.settings]\nurl = "http://192.0.2.1:8123"\n' >>"$settings"
for p in obs ha; do
  dest="$box/data/ulanzi-deck/plugins/$p"
  mkdir -p "$dest"
  cp "$plugins_repo/plugins/$p/plugin.toml" "$dest/plugin.toml"
  cp "$plugins_repo/target/wasm32-wasip2/release/ulanzi_plugin_$p.wasm" "$dest/plugin.wasm"
  cp -r "$plugins_repo/plugins/$p/assets" "$dest/assets"
done

shot() { # name app-args...
  local name=$1
  shift
  ULANZI_BIN="$PWD/target/debug" timeout 90 scripts/sandbox.sh --demo --dir "$box" -- \
    --screenshot "$out/$name.png" --size 1400x900 --theme light --wait 6000 "$@" \
    >"$out/$name.log" 2>&1 </dev/null || true
  if [ -s "$out/$name.png" ]; then echo "  $name"; else echo "  $name FAILED (see $out/$name.log)"; fi
}

shot integrations --open integrations
shot integrations-obs --open integrations:obs
shot integrations-ha --open integrations:ha
shot action-picker --select key:1 --open action-picker
shot library
shot key-editor --select key:1
