#!/bin/sh
# Undo scripts/dev-install.sh. Leaves your profiles in ~/.config/ulanzi-deck alone.
set -eu
systemctl --user disable --now ulanzi-deckd 2>/dev/null || true
for f in "$HOME/.local/bin/ulanzi-deckd" "$HOME/.local/bin/ulanzictl" \
         "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/ulanzi-deckd.service" \
         "${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services/io.github.tyaaa_aa.UlanziDeck.service"; do
    [ -e "$f" ] && rm -f -- "$f"
done
systemctl --user daemon-reload
echo "Removed. The udev rule (/etc/udev/rules.d/70-ulanzi-deck.rules) is left in place."
