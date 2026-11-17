#!/usr/bin/env bash
# The Rune desktop's gate: format, lints, unit tests, then the scene accessibility and
# translation check (src/checks.rs) against a disposable book, all offline. Replaces the
# Python eval tools in the product gate (AGENTS.md: no Python in the gate).
#
# Usage: packaging/ci/rune-checks.sh --kassiber-bin PATH [--book DATA_ROOT] [--skip-scenes | --scenes-only]
#   --kassiber-bin  the `kassiber` binary the scenes' daemon runs (the root workspace's build)
#   --book          a book data root to copy (e.g. the regtest demo); default: an empty book
#                   created with `kassiber init`. The original is never opened.
#   --skip-scenes   fmt, clippy and unit tests only (no daemon)
#   --scenes-only   the scene check alone (CI runs the other steps separately)
#
# Every write lands in a temporary HOME; offline mode is on and KASSIBER_NO_EGRESS is set, so
# no scene can reach a backend even if a scene pressed a network control.
set -euo pipefail
crate="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
kassiber_bin="${KASSIBER_BIN:-}"
book=""
scenes=1
static=1
while [ "$#" -gt 0 ]; do
  case "$1" in
    --kassiber-bin) kassiber_bin="$2"; shift ;;
    --book) book="$2"; shift ;;
    --skip-scenes) scenes=0 ;;
    --scenes-only) static=0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

cd "$crate"
run() { echo; echo "> $*"; "$@"; }
if [ "$static" -eq 1 ]; then
  run cargo fmt --check
  run cargo clippy --locked --all-targets -- -D warnings
  run cargo clippy --locked --all-targets --no-default-features -- -D warnings
  run cargo test --locked --release
fi

[ "$scenes" -eq 1 ] || exit 0
[ -n "$kassiber_bin" ] && [ -x "$kassiber_bin" ] || { echo "--kassiber-bin is required for the scene check" >&2; exit 2; }
kassiber_bin="$(cd "$(dirname "$kassiber_bin")" && pwd)/$(basename "$kassiber_bin")"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/rune-checks.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home" "$scratch/book" "$scratch/tmp"
# Keep Cargo and rustup where they are before HOME moves (else Cargo refetches every crate).
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export HOME="$scratch/home" TMPDIR="$scratch/tmp/" XDG_CONFIG_HOME="$scratch/home/.config"
export KASSIBER_BIN="$kassiber_bin"
export KASSIBER_DEMO_DATA_ROOT="$scratch/book"
export KASSIBER_DESKTOP_PREFS_FILE="$scratch/desktop-prefs.json"
export KASSIBER_OFFLINE_PREFERENCE_FILE="$scratch/offline.json"
export KASSIBER_AGENT_ACCESS_FILE="$scratch/agent-access.json"
export KASSIBER_UPDATE_PREFERENCE_FILE="$scratch/update.json"
export KASSIBER_UPDATE_CACHE_FILE="$scratch/update-cache.json"
export KASSIBER_DISABLE_UPDATE_CHECK=1 KASSIBER_NO_EGRESS=1
"$kassiber_bin" offline on >/dev/null
if [ -n "$book" ]; then
  cp -R "$book"/. "$scratch/book/"
else
  "$kassiber_bin" --data-root "$scratch/book" init >/dev/null
fi
# Linux runners have no display: Rune's headless runtime needs none (no window is opened).
run cargo test --locked --release checks::scenes_are_accessible_and_translated -- --ignored --nocapture
