#!/usr/bin/env bash
# Publish version 2 to GitHub: run this only after the hands-on test (TESTING.md) passed.
#
#   scripts/publish.sh            show what would happen (nothing is sent)
#   scripts/publish.sh --go       push and create the release
#
# What it does, all without force-pushing:
#   1. origin/main must still be the last Python commit (or an ancestor of this branch).
#   2. Pushes this branch to main (a fast-forward: the Python history stays underneath).
#   3. Pushes the python-legacy branch, so version 1 stays easy to find.
#   4. Tags v<version> and creates a GitHub release with the files in dist/ (needs `gh`).
set -euo pipefail
cd "$(dirname "$0")/.."
go=0
[ "${1:-}" = --go ] && go=1
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
branch=$(git rev-parse --abbrev-ref HEAD)
tag="v$version"

say() { printf '%s\n' "$*"; }
do_or_show() { if [ $go = 1 ]; then "$@"; else say "  would run: $*"; fi; }

[ -z "$(git status --porcelain)" ] || { say "Commit or stash your changes first."; exit 1; }
git fetch origin main
if ! git merge-base --is-ancestor origin/main HEAD; then
  say "origin/main has commits this branch doesn't have. Stopping (no force-push)."
  exit 1
fi
ls dist/*.tar.gz >/dev/null 2>&1 || { say "dist/ is empty: run scripts/release.sh first."; exit 1; }
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { say "Tag $tag already exists."; exit 1; }

say "Publishing Ulanzi Deck $version from $branch:"
do_or_show git push origin "refs/heads/$branch:refs/heads/main"
do_or_show git push origin "refs/heads/python-legacy:refs/heads/python-legacy"
do_or_show git tag -a "$tag" -m "Ulanzi Deck $version"
do_or_show git push origin "refs/tags/$tag"
assets=(dist/*.tar.gz dist/*.deb dist/*.rpm dist/*.pkg.tar.zst dist/PKGBUILD dist/SRCINFO dist/SHA256SUMS)
compgen -G "dist/*.AppImage" >/dev/null && assets+=(dist/*.AppImage)
# The repository by its origin URL, so gh needs no default repository set.
repo=$(git remote get-url origin | sed -E 's#(git@github.com:|https://github.com/)##; s#\.git$##')
do_or_show gh release create "$tag" -R "$repo" --title "Ulanzi Deck $version" --notes-file packaging/release-notes.md "${assets[@]}"
[ $go = 1 ] && say "Published." || say "Nothing was sent. Run with --go to publish."
