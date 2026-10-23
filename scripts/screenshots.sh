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

per_theme=8
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
  ULANZI_BIN="$PWD/target/debug" timeout 60 scripts/sandbox.sh --demo --dir "$box" -- \
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
  shot obs "$t" --open integrations
  shot templates "$t" --open templates
  shot onboarding "$t" --open onboarding:welcome
  if [ $board = 1 ]; then sb_ok; fi
done
cargo run -q -p ulanzi-core --example widgets_sheet -- "$out/widgets-sheet.png" >/dev/null
echo "  widgets-sheet"
if [ $board = 1 ]; then sb_finish; fi
