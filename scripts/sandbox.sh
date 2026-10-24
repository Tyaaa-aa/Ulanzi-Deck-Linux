#!/usr/bin/env bash
# Run Ulanzi Deck in a throwaway sandbox: its own profiles, settings and backups, never yours.
#
#   scripts/sandbox.sh                 your real deck; your service pauses and comes back after
#   scripts/sandbox.sh --demo          a pretend deck (no hardware), on a private D-Bus session
#   scripts/sandbox.sh --showcase      start from the showcase profiles instead of a fresh setup
#   scripts/sandbox.sh --dir DIR       keep the sandbox in DIR (default: a new temporary folder)
#   scripts/sandbox.sh ... -- ARGS     extra arguments for the app (e.g. --screenshot out.png)
#
# Binaries: the installed ones when on PATH, else target/debug (override with ULANZI_BIN=dir).
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
demo=0 showcase=0 dir="" inner=0
app_args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --demo) demo=1 ;;
    --showcase) showcase=1 ;;
    --dir) dir="$2"; shift ;;
    --inner) inner=1 ;;
    --) shift; app_args=("$@"); break ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

if [ -n "${ULANZI_BIN:-}" ]; then
  bin="$ULANZI_BIN"
elif command -v ulanzi-deckd >/dev/null && command -v ulanzi-deck >/dev/null; then
  bin="$(dirname "$(command -v ulanzi-deckd)")"
else
  bin="$here/target/debug"
fi
for b in ulanzi-deckd ulanzi-deck; do
  [ -x "$bin/$b" ] || { echo "missing $bin/$b (build with: cargo build)" >&2; exit 1; }
done

if [ "$inner" = 0 ]; then
  if [ -z "$dir" ]; then dir="$(mktemp -d -t ulanzi-sandbox.XXXXXX)"; fi
  mkdir -p "$dir"
  dir="$(cd "$dir" && pwd)"
  if [ "$showcase" = 1 ] && [ ! -d "$dir/config/ulanzi-deck/profiles" ]; then
    if command -v cargo >/dev/null && [ -f "$here/Cargo.toml" ]; then
      (cd "$here" && cargo run -q -p ulanzi-core --example showcase -- "$dir")
    else
      echo "--showcase needs the source tree (cargo)" >&2; exit 1
    fi
  fi
  echo "Sandbox: $dir"
  if [ "$demo" = 1 ]; then
    # Everything the private bus could start sees only the sandbox, never your own folders, and
    # the bus has no service directories at all, so your wallet and desktop portals cannot be
    # started on it (they once were, and they stayed running).
    export ULANZI_OBS_CONFIG_HOME="${ULANZI_OBS_CONFIG_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}}"
    export XDG_CONFIG_HOME="$dir/config" XDG_STATE_HOME="$dir/state" XDG_CACHE_HOME="$dir/cache" \
      XDG_DATA_HOME="$dir/data"
    mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"
    exec dbus-run-session --config-file="$here/scripts/private-bus.conf" -- \
      "$0" --inner --demo --dir "$dir" -- "${app_args[@]}"
  fi
  if busctl --user status io.github.tyaaa_aa.UlanziDeck >/dev/null 2>&1; then
    echo "Please close the Ulanzi Deck window first (it would open your own profiles)." >&2
    exit 1
  fi
  # Real deck: pause the user's service (it holds the deck) and bring it back afterwards.
  was_active="$(systemctl --user is-active ulanzi-deckd.service 2>/dev/null || true)"
  if [ "$was_active" = active ]; then
    systemctl --user stop ulanzi-deckd.service
    trap 'echo "Starting your own service again"; systemctl --user start ulanzi-deckd.service' EXIT
  fi
  "$0" --inner --dir "$dir" -- "${app_args[@]}" || true
  exit 0
fi

# Inside: everything below sees only the sandbox.
# OBS keeps its own settings in your real config folder; "Find OBS Settings" should read them there.
export ULANZI_OBS_CONFIG_HOME="${ULANZI_OBS_CONFIG_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}}"
export XDG_CONFIG_HOME="$dir/config" XDG_STATE_HOME="$dir/state" XDG_CACHE_HOME="$dir/cache" \
  XDG_DATA_HOME="$dir/data"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"
# Keep the desktop's look (icon theme, fonts): only GTK's own settings come along.
if [ -f "$HOME/.config/gtk-4.0/settings.ini" ] && [ ! -e "$XDG_CONFIG_HOME/gtk-4.0/settings.ini" ]; then
  mkdir -p "$XDG_CONFIG_HOME/gtk-4.0"
  cp "$HOME/.config/gtk-4.0/settings.ini" "$XDG_CONFIG_HOME/gtk-4.0/"
fi
if [ "$demo" = 1 ]; then
  export ULANZI_DEMO=1
  # The private bus would start fresh desktop portals with default settings; use GTK's own.
  export GDK_DEBUG=no-portals GTK_A11Y=none
fi

if busctl --user status io.github.tyaaa_aa.UlanziDeck.Service >/dev/null 2>&1; then
  echo "Another ulanzi-deckd is still running; stop it first (systemctl --user stop ulanzi-deckd)." >&2
  exit 1
fi
"$bin/ulanzi-deckd" >"$dir/daemon.out" 2>&1 &
daemon=$!
trap 'kill "$daemon" 2>/dev/null; wait "$daemon" 2>/dev/null || true' EXIT
for _ in $(seq 50); do
  busctl --user status io.github.tyaaa_aa.UlanziDeck.Service >/dev/null 2>&1 && break
  sleep 0.1
done
"$bin/ulanzi-deck" "${app_args[@]}"
