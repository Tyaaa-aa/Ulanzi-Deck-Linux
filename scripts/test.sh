#!/usr/bin/env bash
# The automated test suite. Touches nothing of yours: the end-to-end part runs on a private D-Bus
# session with a pretend deck and throwaway profiles.
#
#   scripts/test.sh          everything
#   scripts/test.sh --quick  skip the end-to-end smoke test (no display needed)
set -uo pipefail
cd "$(dirname "$0")/.."
quick=0
[ "${1:-}" = --quick ] && quick=1
pass=0
failed=()

run() { # name command...
  local name=$1
  shift
  printf '%-34s ' "$name"
  local log
  log=$(mktemp)
  if "$@" >"$log" 2>&1; then
    echo "ok"
    pass=$((pass + 1))
  else
    echo "FAILED"
    tail -25 "$log" | sed 's/^/    /'
    failed+=("$name")
  fi
  rm -f "$log"
}

skip() { printf '%-34s skipped (%s)\n' "$1" "$2"; }

run "format" cargo fmt --all --check
run "lints (clippy, warnings = errors)" cargo clippy --workspace --all-targets --locked -- -D warnings
run "unit and integration tests" cargo test --workspace --locked
run "shell scripts parse" bash -c 'for f in scripts/*.sh packaging/tarball/*.sh packaging/appimage/AppRun; do sh -n "$f" || exit 1; done'

if command -v appstreamcli >/dev/null; then
  run "metainfo" appstreamcli validate --no-net crates/ulanzi-deck/data/io.github.tyaaa_aa.UlanziDeck.metainfo.xml
else
  skip "metainfo" "appstreamcli not installed"
fi
if command -v desktop-file-validate >/dev/null; then
  run "desktop entry" desktop-file-validate crates/ulanzi-deck/data/io.github.tyaaa_aa.UlanziDeck.desktop
else
  skip "desktop entry" "desktop-file-validate not installed"
fi
run "udev rule names the deck" grep -q 'idVendor}=="2207", ATTRS{idProduct}=="0019"' packaging/udev/70-ulanzi-deck.rules
run "staged install tree" bash -c '
  d=$(mktemp -d); trap "rm -rf $d" EXIT
  cargo build -q --release --locked --bins &&
  NO_BUILD=1 sh scripts/stage.sh /usr "$d" >/dev/null &&
  test -x "$d/usr/bin/ulanzi-deckd" &&
  grep -q "^ExecStart=/usr/bin/ulanzi-deckd$" "$d/usr/lib/systemd/user/ulanzi-deckd.service" &&
  test -f "$d/usr/share/dbus-1/services/io.github.tyaaa_aa.UlanziDeck.Service.service"'

if [ $quick = 1 ]; then
  skip "end-to-end smoke test" "--quick"
elif ! command -v dbus-run-session >/dev/null; then
  skip "end-to-end smoke test" "dbus-run-session not installed"
else
  run "end-to-end: service and CLI" bash -c '
    box=$(mktemp -d); trap "rm -rf $box" EXIT
    cargo build -q --locked --bins || exit 1
    cargo run -q -p ulanzi-core --example showcase -- "$box" >/dev/null || exit 1
    export XDG_CONFIG_HOME=$box/config XDG_STATE_HOME=$box/state XDG_CACHE_HOME=$box/cache XDG_DATA_HOME=$box/data ULANZI_DEMO=1
    dbus-run-session --config-file="$PWD/scripts/private-bus.conf" -- bash -c "
      target/debug/ulanzi-deckd >$box/daemon.log 2>&1 &
      trap \"kill \$! 2>/dev/null\" EXIT
      for i in \$(seq 50); do target/debug/ulanzictl status >/dev/null 2>&1 && break; sleep 0.1; done
      target/debug/ulanzictl status | grep -q \"ready: demo\" &&
      target/debug/ulanzictl profile list | grep -q \"Media Desk\" &&
      target/debug/ulanzictl export $box/all.ulanzideck >/dev/null &&
      test -s $box/all.ulanzideck &&
      target/debug/ulanzictl snapshots >/dev/null
    "'
  if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    run "end-to-end: app renders" bash -c '
      out=$(mktemp -d); trap "rm -rf $out" EXIT
      ULANZI_BIN="$PWD/target/debug" timeout 60 scripts/sandbox.sh --demo --showcase --dir "$out/box" -- \
        --screenshot "$out/shot.png" --size 1200x800 </dev/null >/dev/null 2>&1
      test -s "$out/shot.png"'
  else
    skip "end-to-end: app renders" "no display"
  fi
fi

echo
if [ ${#failed[@]} -eq 0 ]; then
  echo "All $pass checks passed."
else
  echo "${#failed[@]} failed: ${failed[*]}"
  exit 1
fi
