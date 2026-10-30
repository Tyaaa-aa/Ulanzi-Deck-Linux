#!/usr/bin/env bash
# Take the README screenshots from the showcase profiles in a throwaway sandbox with a pretend deck.
# Nothing from your own configuration appears: the sandbox has its own config, data and D-Bus.
#
#   scripts/screenshots.sh            → docs/screenshots/*.png
# Optional live board: set SB_STATE (and SB_LIB) for the status-board plugin.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -q --bins
out=docs/screenshots
mkdir -p "$out"
box="$(mktemp -d -t ulanzi-shots.XXXXXX)"
trap 'rm -rf "$box"' EXIT
cargo run -q -p ulanzi-core --example showcase -- "$box" >/dev/null
# A neutral look: stock icon theme and window buttons.
mkdir -p "$box/config/gtk-4.0"
printf '[Settings]\ngtk-icon-theme-name=Adwaita\ngtk-decoration-layout=:minimize,maximize,close\n' \
  >"$box/config/gtk-4.0/settings.ini"
# The OBS and Home Assistant plugins, from the plugins repo next to this one by default, so plugin
# keys look as they do for a user who installed them.
plugins_repo="${ULANZI_PLUGINS_REPO:-$PWD/../Ulanzi-Deck-Plugins}"
[ -d "$plugins_repo/plugins/obs" ] || { echo "no plugins repo at $plugins_repo (set ULANZI_PLUGINS_REPO)" >&2; exit 1; }
(cd "$plugins_repo" && cargo build -q --release --target wasm32-wasip2 -p ulanzi-plugin-obs -p ulanzi-plugin-ha)
for p in obs ha; do
  dest="$box/data/ulanzi-deck/plugins/$p"
  mkdir -p "$dest"
  cp "$plugins_repo/plugins/$p/plugin.toml" "$dest/plugin.toml"
  cp "$plugins_repo/target/wasm32-wasip2/release/ulanzi_plugin_$p.wasm" "$dest/plugin.wasm"
  cp -r "$plugins_repo/plugins/$p/assets" "$dest/assets"
done
# Home Assistant gets an address (an unused documentation address), so it stops at the missing token.
printf '\n[plugins.ha]\nenabled = true\n\n[plugins.ha.settings]\nurl = "http://192.0.2.1:8123"\n' \
  >>"$box/config/ulanzi-deck/settings.toml"

per_theme=9
board=0
if [ -n "${SB_STATE:-}" ] && [ -f "${SB_LIB:-}" ]; then
  # shellcheck disable=SC1090
  source "$SB_LIB"
  board=1
  sb_init "$SB_STATE" "README screenshots" 2 "pkill -f scripts/screenshots.s[h]"
  sb_cols "Theme" "Shots" "Last"
  sb_item 1 "light" "" ""
  sb_item 2 "dark" "" ""
fi

item=0
done_shots=0
shot() { # name theme app-args...
  local name=$1 theme=$2
  shift 2
  if [ $board = 1 ]; then sb_stage "$name"; fi
  ULANZI_BIN="$PWD/target/debug" timeout 90 scripts/sandbox.sh --demo --dir "$box" -- \
    --screenshot "$out/$name-$theme.png" --size 1400x900 --theme "$theme" "$@" >/dev/null 2>&1 </dev/null || true
  if [ -s "$out/$name-$theme.png" ]; then echo "  $name-$theme"; else echo "  $name-$theme FAILED"; fi
  done_shots=$((done_shots + 1))
  if [ $board = 1 ]; then
    sb_progress "$done_shots"
    sb_cell "$item" 2 "$done_shots/$per_theme"
    sb_cell "$item" 3 "$name"
  fi
}

for t in light dark; do
  item=$((item + 1))
  done_shots=0
  if [ $board = 1 ]; then sb_start "$item" "$t" "$per_theme"; fi
  shot main "$t"
  shot widgets "$t" --select screen
  shot key "$t" --select key:1
  shot dial "$t" --select dial:1
  shot icons "$t" --select key:1 --open picker:icons
  shot plugins "$t" --open plugins
  # The store list comes from the internet: give it time to arrive.
  shot discover "$t" --open discover --wait 30000
  shot templates "$t" --open templates
  shot onboarding "$t" --open onboarding:welcome
  if [ $board = 1 ]; then sb_ok; fi
done
cargo run -q -p ulanzi-core --example widgets_sheet -- "$out/widgets-sheet.png" >/dev/null
echo "  widgets-sheet"
if [ $board = 1 ]; then sb_finish; fi
