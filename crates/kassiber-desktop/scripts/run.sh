#!/usr/bin/env bash
# Build and open the desktop app against a Kassiber book.
#
#   crates/kassiber-desktop/scripts/run.sh --data-root /path/to/book/data [rune args…]
#
# Use a disposable copy of a book (see docs/reference/testing.md): the app runs the real
# daemon, and its buttons (refresh, rate refresh, journal processing, book switching, note
# edits) act on that book. Preference files are isolated in a scratch directory so the
# account's default Kassiber state root is not read or written.
#
# Headless instead of a window: add `--snap out.png --size 1440x900 [--scene <name>]`
# (scenes are listed in src/main.rs), or `--script scripts/<file>.json --video out.mp4`.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/../.." && pwd)"

if ! printf '%s\n' "$@" | grep -qx -- '--data-root'; then
  echo "usage: $0 --data-root <book data root> [rune args…]" >&2
  echo "  (pass a disposable copy: the daemon opens and can write that book)" >&2
  exit 2
fi

export KASSIBER_BIN="${KASSIBER_BIN:-$repo/target/release/kassiber}"
if [[ ! -x "$KASSIBER_BIN" ]]; then
  echo "kassiber binary not found at $KASSIBER_BIN; build it with: cargo build --release -p kassiber" >&2
  exit 1
fi

prefs="${KASSIBER_DESKTOP_SCRATCH_PREFS:-${XDG_RUNTIME_DIR:-/tmp}/kassiber-desktop-prefs}"
mkdir -p "$prefs"
export KASSIBER_OFFLINE_PREFERENCE_FILE="${KASSIBER_OFFLINE_PREFERENCE_FILE:-$prefs/offline-mode.json}"
export KASSIBER_AGENT_ACCESS_FILE="${KASSIBER_AGENT_ACCESS_FILE:-$prefs/agent-access.json}"
export KASSIBER_UPDATE_PREFERENCE_FILE="${KASSIBER_UPDATE_PREFERENCE_FILE:-$prefs/update.json}"
export KASSIBER_UPDATE_CACHE_FILE="${KASSIBER_UPDATE_CACHE_FILE:-$prefs/update-cache.json}"
# The desktop's own preferences (theme, window frame, last route, chart periods): kept with the
# scratch preferences too, so a preview never writes into the default Kassiber state root.
export KASSIBER_DESKTOP_PREFS_FILE="${KASSIBER_DESKTOP_PREFS_FILE:-$prefs/desktop-ui.json}"

cargo_cmd=(cargo)
if ! cargo --version >/dev/null 2>&1; then
  cargo_cmd=(mise exec rust@1.99.1 -- cargo)
fi

cd "$here"
"${cargo_cmd[@]}" build --release
target="${CARGO_TARGET_DIR:-$here/target}"
exec "$target/release/kassiber-desktop" "$@"
