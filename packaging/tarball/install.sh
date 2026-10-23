#!/bin/sh
# Install Ulanzi Deck from this folder.
#
#   ./install.sh            for you only, into ~/.local (no password, except for the deck permission rule)
#   ./install.sh --system   for everyone, into /usr/local (asks for your password)
#   ./install.sh --no-udev  skip the deck permission rule (if it is already installed)
#
# Your profiles live in ~/.config/ulanzi-deck and are never touched. Undo with ./uninstall.sh.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mode=user
udev=1
for a in "$@"; do
  case "$a" in
    --system) mode=system ;;
    --no-udev) udev=0 ;;
    -h | --help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

if [ "$mode" = system ]; then
  prefix=/usr/local
  data=/usr/local/share
  units=/usr/local/lib/systemd/user
  sudo=sudo
else
  prefix=$HOME/.local
  data=${XDG_DATA_HOME:-$HOME/.local/share}
  units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
  sudo=
fi
manifest_dir=${XDG_DATA_HOME:-$HOME/.local/share}/ulanzi-deck
manifest=$manifest_dir/installed-files
mkdir -p "$manifest_dir"
: >"$manifest.new"

put() { # source dest mode
  $sudo install -Dm"$3" "$1" "$2"
  echo "$2" >>"$manifest.new"
}

echo "Installing Ulanzi Deck into $prefix"
for b in ulanzi-deck ulanzi-deckd ulanzictl; do put "$here/bin/$b" "$prefix/bin/$b" 0755; done
(cd "$here/share" && find . -type f) | while read -r f; do
  f=${f#./}
  put "$here/share/$f" "$data/$f" 0644
done
tmp=$(mktemp)
sed "s|^ExecStart=.*|ExecStart=$prefix/bin/ulanzi-deckd|" "$here/lib/systemd/user/ulanzi-deckd.service" >"$tmp"
put "$tmp" "$units/ulanzi-deckd.service" 0644
rm -f "$tmp"
echo "mode=$mode" >>"$manifest.new"
mv "$manifest.new" "$manifest"

if [ "$udev" = 1 ]; then
  if [ -e /etc/udev/rules.d/70-ulanzi-deck.rules ] || [ -e /usr/lib/udev/rules.d/70-ulanzi-deck.rules ]; then
    echo "The deck permission rule is already installed."
  else
    echo "The deck needs a permission rule so your user can talk to it (asks for your password once):"
    sudo install -Dm0644 "$here/lib/udev/rules.d/70-ulanzi-deck.rules" /etc/udev/rules.d/70-ulanzi-deck.rules
    sudo udevadm control --reload-rules && sudo udevadm trigger
    echo "If the deck was plugged in, unplug it and plug it in again."
  fi
fi

update-mime-database "$data/mime" >/dev/null 2>&1 || true
update-desktop-database -q "$data/applications" >/dev/null 2>&1 || true
gtk-update-icon-cache -q -t "$data/icons/hicolor" >/dev/null 2>&1 || true
systemctl --user daemon-reload
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
systemctl --user enable --now ulanzi-deckd.service
case ":$PATH:" in *":$prefix/bin:"*) ;; *) echo "Note: $prefix/bin is not on your PATH (the menu entry works anyway)." ;; esac
echo "Done. Open “Ulanzi Deck” from your app menu."
