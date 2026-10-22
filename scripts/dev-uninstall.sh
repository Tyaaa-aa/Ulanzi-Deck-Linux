#!/bin/sh
# Undo scripts/dev-install.sh. Leaves your profiles in ~/.config/ulanzi-deck alone.
set -eu
systemctl --user disable --now ulanzi-deckd 2>/dev/null || true
data="${XDG_DATA_HOME:-$HOME/.local/share}"
for f in "$HOME/.local/bin/ulanzi-deckd" "$HOME/.local/bin/ulanzictl" "$HOME/.local/bin/ulanzi-deck" \
         "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/ulanzi-deckd.service" \
         "$data/dbus-1/services/io.github.tyaaa_aa.UlanziDeck.service" \
         "$data/applications/io.github.tyaaa_aa.UlanziDeck.desktop" \
         "$data/metainfo/io.github.tyaaa_aa.UlanziDeck.metainfo.xml" \
         "$data/icons/hicolor/scalable/apps/io.github.tyaaa_aa.UlanziDeck.svg" \
         "$data/icons/hicolor/symbolic/apps/io.github.tyaaa_aa.UlanziDeck-symbolic.svg"; do
    [ -e "$f" ] && rm -f -- "$f"
done
systemctl --user daemon-reload
echo "Removed. The udev rule (/etc/udev/rules.d/70-ulanzi-deck.rules) is left in place."
