#!/usr/bin/env bash
# Record the scenes of the demo video from the showcase profiles, in a throwaway sandbox with a
# pretend deck. Each scene starts from a fresh sandbox. Nothing from your own configuration
# appears. Needs ffmpeg.
#
#   scripts/demo-reel.sh [OUT_DIR] [SCENE...]     → OUT_DIR/<scene>.mp4 (default dist/demo)
#
# Scenes: keys widgets library autoswitch plugins (all when none are named).
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-dist/demo}"
shift || true
scenes=("$@")
[ ${#scenes[@]} -gt 0 ] || scenes=(keys widgets library autoswitch plugins)
command -v ffmpeg >/dev/null || { echo "demo-reel needs ffmpeg" >&2; exit 1; }
cargo build -q --bins
mkdir -p "$out"
out="$(cd "$out" && pwd)"
plugins_repo="${ULANZI_PLUGINS_REPO:-$PWD/../Ulanzi-Deck-Plugins}"
[ -d "$plugins_repo/plugins/obs" ] || { echo "no plugins repo at $plugins_repo (set ULANZI_PLUGINS_REPO)" >&2; exit 1; }
(cd "$plugins_repo" && cargo build -q --release --target wasm32-wasip2 -p ulanzi-plugin-obs -p ulanzi-plugin-ha)
theme="${DEMO_THEME:-dark}"

for scene in "${scenes[@]}"; do
  box="$(mktemp -d -t ulanzi-reel.XXXXXX)"
  cargo run -q -p ulanzi-core --example showcase -- "$box" >/dev/null
  mkdir -p "$box/config/gtk-4.0"
  printf '[Settings]\ngtk-icon-theme-name=Adwaita\ngtk-decoration-layout=:minimize,maximize,close\n' \
    >"$box/config/gtk-4.0/settings.ini"
  for p in obs ha; do
    dest="$box/data/ulanzi-deck/plugins/$p"
    mkdir -p "$dest"
    cp "$plugins_repo/plugins/$p/plugin.toml" "$dest/plugin.toml"
    cp "$plugins_repo/target/wasm32-wasip2/release/ulanzi_plugin_$p.wasm" "$dest/plugin.wasm"
    cp -r "$plugins_repo/plugins/$p/assets" "$dest/assets"
  done
  printf '\n[plugins.ha]\nenabled = true\n\n[plugins.ha.settings]\nurl = "http://192.0.2.1:8123"\n' \
    >>"$box/config/ulanzi-deck/settings.toml"
  rm -f "$out/$scene.mp4"
  ULANZI_BIN="$PWD/target/debug" timeout 180 scripts/sandbox.sh --demo --dir "$box" -- \
    --screenshot /dev/null --size 1720x968 --theme "$theme" --wait 4000 \
    --demo-reel "$out/$scene.mp4" --scene "$scene" </dev/null 2>&1 | grep -E "demo-reel" || true
  rm -rf "$box"
  if [ -s "$out/$scene.mp4" ]; then echo "  $scene"; else echo "  $scene FAILED"; fi
done
