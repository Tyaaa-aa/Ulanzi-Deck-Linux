#!/bin/sh
# Build a release and install it for the current user, for testing. Starts and enables nothing.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
cargo build --release --bins

bin="$HOME/.local/bin"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
dbus="${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
icons="$data/icons/hicolor"
mkdir -p "$bin" "$units" "$dbus" "$data/applications" "$data/metainfo" \
  "$icons/scalable/apps" "$icons/symbolic/apps"
install -m 0755 target/release/ulanzi-deckd target/release/ulanzictl target/release/ulanzi-deck "$bin/"
gui=crates/ulanzi-deck/data
install -m 0644 "$gui/io.github.tyaaa_aa.UlanziDeck.desktop" "$data/applications/"
install -m 0644 "$gui/io.github.tyaaa_aa.UlanziDeck.metainfo.xml" "$data/metainfo/"
install -m 0644 "$gui/icons/io.github.tyaaa_aa.UlanziDeck.svg" "$icons/scalable/apps/"
install -m 0644 "$gui/icons/io.github.tyaaa_aa.UlanziDeck-symbolic.svg" "$icons/symbolic/apps/"
update-desktop-database -q "$data/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$icons" 2>/dev/null || true
install -m 0644 packaging/systemd/ulanzi-deckd.service "$units/"
install -m 0644 packaging/dbus/io.github.tyaaa_aa.UlanziDeck.service "$dbus/"
systemctl --user daemon-reload

cat <<MSG
Installed ulanzi-deck, ulanzi-deckd and ulanzictl to $bin, and the app to the menu.

If the udev rule is not installed yet, run (asks for your password):
  sudo install -m 0644 "$repo/packaging/udev/70-ulanzi-deck.rules" /etc/udev/rules.d/
  sudo udevadm control --reload-rules && sudo udevadm trigger

Start the service:   systemctl --user start ulanzi-deckd
Check it:            ulanzictl status
Start at login:      systemctl --user enable ulanzi-deckd
MSG
