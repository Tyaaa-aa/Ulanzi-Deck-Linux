#!/usr/bin/env bash
# A fake release for trying the update flow without publishing anything. Serves a signed 9.9.9 on
# this computer, signed with a throwaway key that only debug builds can be told to trust.
#
#   scripts/fake-release.sh [FILE] [--tamper]
#
# FILE is the file the fake release offers (default: a small text file). --tamper changes it after
# signing, so the app must refuse it. Then, in another terminal:
#
#   ULANZI_RELEASES_URL=<printed> ULANZI_TEST_RELEASE_KEY=<printed> scripts/sandbox.sh --demo
set -euo pipefail
file=""
tamper=0
for a in "$@"; do
  case "$a" in
    --tamper) tamper=1 ;;
    *) file=$a ;;
  esac
done
version=9.9.9
arch=$(uname -m)
port=8799
dir=$(mktemp -d -t ulanzi-fake-release.XXXXXX)
trap 'rm -rf "$dir"' EXIT
assets=(
  "Ulanzi_Deck-$version-$arch.AppImage"
  "ulanzi-deck-$version-1-$arch.pkg.tar.zst"
  "ulanzi-deck_$version-1_amd64.deb"
  "ulanzi-deck-$version-1.$arch.rpm"
  "ulanzi-deck-$version-$arch.tar.gz"
)
for name in "${assets[@]}"; do
  if [ -n "$file" ]; then cp "$file" "$dir/$name"; else echo "fake release" >"$dir/$name"; fi
done
(cd "$dir" && sha256sum -- "${assets[@]}" >SHA256SUMS)
minisign -G -W -s "$dir/key" -p "$dir/key.pub" -f >/dev/null
minisign -S -s "$dir/key" -m "$dir/SHA256SUMS" -t "file:SHA256SUMS" >/dev/null
if [ "$tamper" = 1 ]; then
  for name in "${assets[@]}"; do echo tampered >>"$dir/$name"; done
fi
python3 -I - "$dir" "$port" "$version" <<'PY'
import json, os, sys
d, port, version = sys.argv[1:]
names = [n for n in sorted(os.listdir(d)) if n not in ("key", "key.pub")]
assets = [{"name": n, "size": os.path.getsize(os.path.join(d, n)),
           "browser_download_url": f"http://127.0.0.1:{port}/{n}"} for n in names]
notes = "### Important\n\nThis is a fake release for testing.\n\n### New\n\n- **Nothing.** It only tests updates.\n"
json.dump({"tag_name": f"v{version}", "name": f"Ulanzi Deck {version}", "draft": False,
           "prerelease": False, "published_at": "2026-11-02T10:00:00Z", "body": notes,
           "assets": assets}, open(os.path.join(d, "latest"), "w"))
PY
echo "ULANZI_RELEASES_URL=http://127.0.0.1:$port/latest"
echo "ULANZI_TEST_RELEASE_KEY=$(tail -1 "$dir/key.pub")"
exec_dir=$dir
cd "$exec_dir"
python3 -I -m http.server "$port" --bind 127.0.0.1
