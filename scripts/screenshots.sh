#!/usr/bin/env bash
# Take the README screenshots from the showcase profiles in a throwaway sandbox with a pretend deck.
# Nothing from your own configuration appears: the sandbox has its own config, data and D-Bus.
#
#   scripts/screenshots.sh            → docs/screenshots/*.png
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

shot() { # name theme app-args...
  local name=$1 theme=$2
  shift 2
  ULANZI_BIN="$PWD/target/debug" timeout 60 scripts/sandbox.sh --demo --dir "$box" -- \
    --screenshot "$out/$name-$theme.png" --size 1400x900 --theme "$theme" "$@" >/dev/null 2>&1 </dev/null || true
  [ -s "$out/$name-$theme.png" ] && echo "  $name-$theme" || echo "  $name-$theme FAILED"
}

for t in light dark; do
  shot main "$t"
  shot widgets "$t" --select screen
  shot key "$t" --select key:1
  shot dial "$t" --select dial:1
  shot icons "$t" --select key:1 --open picker:icons
  shot obs "$t" --open integrations
  shot templates "$t" --open templates
  shot onboarding "$t" --open onboarding:welcome
done
cargo run -q -p ulanzi-core --example widgets_sheet -- "$out/widgets-sheet.png" >/dev/null
echo "  widgets-sheet"
