#!/usr/bin/env bash
# Build every release format into dist/:
#   ulanzi-deck-VERSION-ARCH.tar.gz   (install.sh / uninstall.sh inside)
#   ulanzi-deck_VERSION-1_ARCH.deb, ulanzi-deck-VERSION-1.ARCH.rpm
#   ulanzi-deck-VERSION-1-ARCH.pkg.tar.zst (+ PKGBUILD, .SRCINFO), ulanzi-deck-VERSION-source.tar.gz
#   Ulanzi_Deck-VERSION-ARCH.AppImage (when appimagetool is installed)
#   SHA256SUMS
# Needs: cargo-deb, cargo-generate-rpm, makepkg (Arch). Builds from the committed tree (HEAD).
# Optional live board: set SB_STATE to a state directory (status-board plugin).
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(uname -m)
dist=$repo/dist
work=$repo/target/release-work
mkdir -p "$dist"
rm -rf "$work"
mkdir -p "$work"

board=0
sb_lib=${SB_LIB:-}
if [ -n "${SB_STATE:-}" ] && [ -f "$sb_lib" ]; then
  # shellcheck disable=SC1090
  source "$sb_lib"
  board=1
  sb_init "$SB_STATE" "Ulanzi Deck $version release builds" 8 "pkill -f scripts/release.s[h]"
  sb_cols "Format" "File" "Size"
  for i in 1 2 3 4 5 6 7 8; do sb_item "$i" "" "" ""; done
fi
step() { [ $board = 1 ] && { sb_start "$1" "$2" "?"; sb_stage "$3"; sb_cell "$1" 1 "$2"; }; echo "== $2: $3"; }
done_step() { # idx file
  local size=""
  [ -f "$2" ] && size=$(du -h "$2" | cut -f1)
  [ $board = 1 ] && { sb_cell "$1" 2 "$(basename "$2")"; sb_cell "$1" 3 "$size"; sb_ok; }
  echo "   -> $(basename "$2") $size"
}
skip_step() { [ $board = 1 ] && { sb_start "$1" "$2" "?"; sb_cell "$1" 1 "$2"; sb_warn "$3"; }; echo "== $2 skipped: $3"; }
fail() { [ $board = 1 ] && { sb_fail "$1"; sb_finish; }; echo "FAILED: $1" >&2; exit 1; }
trap 'fail "step failed (see the terminal output)"' ERR

# 1. Binaries, and the /usr tree the packages share.
step 1 "Binaries" "cargo build --release"
cargo build --release --locked --bins
rm -rf "$repo/target/pkgroot"
NO_BUILD=1 sh scripts/stage.sh /usr "$repo/target/pkgroot" >/dev/null
done_step 1 target/release/ulanzi-deck

# 2. Tarball with its own installer.
step 2 "Tarball" "packing"
t=$work/ulanzi-deck-$version-$arch
NO_BUILD=1 sh scripts/stage.sh "" "$t" >/dev/null
install -m0755 packaging/tarball/install.sh packaging/tarball/uninstall.sh "$t/"
cp README.md LICENSE "$t/"
tar -C "$work" -czf "$dist/ulanzi-deck-$version-$arch.tar.gz" "ulanzi-deck-$version-$arch"
done_step 2 "$dist/ulanzi-deck-$version-$arch.tar.gz"

# 3. Debian / Ubuntu.
step 3 ".deb" "cargo deb"
deb=$(cargo deb -p ulanzi-deck --no-build --no-strip --output "$dist" 2>/dev/null | tail -1)
done_step 3 "$deb"

# 4. Fedora / openSUSE.
step 4 ".rpm" "cargo generate-rpm"
cargo generate-rpm -p crates/ulanzi-deck -o "$dist" >/dev/null
done_step 4 "$(ls -t "$dist"/*.rpm | head -1)"

# 5. Source tarball (what the PKGBUILD downloads from the release tag).
step 5 "Source" "git archive"
src=$dist/ulanzi-deck-$version-source.tar.gz
git archive --format=tar.gz --prefix="Ulanzi-Deck-Linux-$version/" -o "$src" HEAD
done_step 5 "$src"

# 6. Arch Linux package, built from that source tarball.
if command -v makepkg >/dev/null; then
  step 6 "Arch package" "makepkg (compiles again)"
  a=$work/arch
  mkdir -p "$a"
  sed "s/^pkgver=.*/pkgver=$version/" packaging/arch/PKGBUILD >"$a/PKGBUILD"
  cp "$src" "$a/ulanzi-deck-$version.tar.gz"
  (cd "$a" && makepkg -f --noconfirm --nodeps >makepkg.log 2>&1 && makepkg --printsrcinfo >.SRCINFO)
  cp "$a/.SRCINFO" packaging/arch/.SRCINFO
  cp "$a"/ulanzi-deck-"$version"-*.pkg.tar.zst "$a/PKGBUILD" "$dist/"
  cp "$a/.SRCINFO" "$dist/SRCINFO"
  done_step 6 "$(ls -t "$dist"/*.pkg.tar.zst | head -1)"
else
  skip_step 6 "Arch package" "makepkg not found"
fi

# 7. AppImage (uses the system's GTK 4 and libadwaita).
if command -v appimagetool >/dev/null; then
  step 7 "AppImage" "appimagetool"
  d=$work/AppDir
  NO_BUILD=1 sh scripts/stage.sh /usr "$d" >/dev/null
  cp crates/ulanzi-deck/data/io.github.tyaaa_aa.UlanziDeck.desktop "$d/"
  cp crates/ulanzi-deck/data/icons/io.github.tyaaa_aa.UlanziDeck.svg "$d/"
  install -m0755 packaging/appimage/AppRun "$d/AppRun"
  ARCH=$arch appimagetool -n "$d" "$dist/Ulanzi_Deck-$version-$arch.AppImage" >/dev/null 2>&1
  done_step 7 "$dist/Ulanzi_Deck-$version-$arch.AppImage"
else
  skip_step 7 "AppImage" "appimagetool not installed"
fi

# 8. Checksums.
step 8 "Checksums" "sha256sum"
(cd "$dist" && sha256sum -- *.tar.gz *.deb *.rpm *.pkg.tar.zst *.AppImage 2>/dev/null >SHA256SUMS || true)
# Signed with the release key, which stays on this computer and is never given to CI. The app
# checks this signature before it installs an update. Asks for the key's password.
if [ -f "$HOME/.minisign/ulanzi-deck-release.key" ]; then
  minisign -S -s "$HOME/.minisign/ulanzi-deck-release.key" -m "$dist/SHA256SUMS" -t "file:SHA256SUMS"
else
  echo "WARNING: no release key, so SHA256SUMS is unsigned and scripts/publish.sh will refuse it."
fi
done_step 8 "$dist/SHA256SUMS"
trap - ERR
[ $board = 1 ] && sb_finish
echo "All builds are in dist/"
