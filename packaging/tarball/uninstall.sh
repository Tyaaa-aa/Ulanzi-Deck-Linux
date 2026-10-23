#!/bin/sh
# Remove what install.sh installed. Your profiles in ~/.config/ulanzi-deck and the backups in
# ~/.local/state/ulanzi-deck stay. Add --rules to also remove the deck permission rule.
set -eu
manifest=${XDG_DATA_HOME:-$HOME/.local/share}/ulanzi-deck/installed-files
[ -f "$manifest" ] || { echo "Nothing to remove: $manifest not found." >&2; exit 1; }
sudo=
grep -qx "mode=system" "$manifest" && sudo=sudo
systemctl --user disable --now ulanzi-deckd.service 2>/dev/null || true
grep -v '^mode=' "$manifest" | while read -r f; do
  [ -e "$f" ] && $sudo rm -f -- "$f"
done
rm -f -- "$manifest"
if [ "${1:-}" = --rules ] && [ -e /etc/udev/rules.d/70-ulanzi-deck.rules ]; then
  sudo rm -f /etc/udev/rules.d/70-ulanzi-deck.rules
  sudo udevadm control --reload-rules
fi
systemctl --user daemon-reload
echo "Ulanzi Deck removed. Your profiles are still in ~/.config/ulanzi-deck."
