#!/bin/sh
# Lay out an installed tree: binaries, desktop entry, icons, metainfo, MIME type, D-Bus activation,
# systemd user unit, udev rule, GNOME focus extension and docs.
#
#   scripts/stage.sh PREFIX DESTDIR     e.g. scripts/stage.sh /usr target/pkgroot
#
# Builds the release binaries first unless NO_BUILD=1. BIN_DIR says where they are.
set -eu
prefix=$1
dest=$2
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
[ "${NO_BUILD:-0}" = 1 ] || cargo build --release --locked --bins
root="$dest$prefix"
gui=crates/ulanzi-deck/data
id=io.github.tyaaa_aa.UlanziDeck
bin=${BIN_DIR:-target/release}
install -Dm0755 -t "$root/bin" "$bin/ulanzi-deck" "$bin/ulanzi-deckd" "$bin/ulanzictl"
install -Dm0644 "$gui/$id.desktop" "$root/share/applications/$id.desktop"
install -Dm0644 "$gui/$id.metainfo.xml" "$root/share/metainfo/$id.metainfo.xml"
install -Dm0644 "$gui/$id.mime.xml" "$root/share/mime/packages/$id.mime.xml"
install -Dm0644 "$gui/icons/$id.svg" "$root/share/icons/hicolor/scalable/apps/$id.svg"
install -Dm0644 "$gui/icons/$id-symbolic.svg" "$root/share/icons/hicolor/symbolic/apps/$id-symbolic.svg"
install -Dm0644 "$gui/icons/$id-auto-symbolic.svg" "$root/share/icons/hicolor/symbolic/apps/$id-auto-symbolic.svg"
install -Dm0644 packaging/dbus/$id.Service.service "$root/share/dbus-1/services/$id.Service.service"
mkdir -p "$root/lib/systemd/user"
sed "s|@BINDIR@|$prefix/bin|" packaging/systemd/ulanzi-deckd.service >"$root/lib/systemd/user/ulanzi-deckd.service"
chmod 0644 "$root/lib/systemd/user/ulanzi-deckd.service"
install -Dm0644 packaging/udev/70-ulanzi-deck.rules "$root/lib/udev/rules.d/70-ulanzi-deck.rules"
ext=ulanzi-deck-focus@tyaaa-aa.github.io
for f in packaging/gnome-extension/$ext/*; do
  install -Dm0644 "$f" "$root/share/gnome-shell/extensions/$ext/$(basename "$f")"
done
install -Dm0644 README.md "$root/share/doc/ulanzi-deck/README.md"
install -Dm0644 LICENSE "$root/share/licenses/ulanzi-deck/LICENSE"
echo "staged $prefix into $dest"
