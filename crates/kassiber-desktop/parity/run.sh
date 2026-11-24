#!/usr/bin/env bash
# Tauri vs Rune parity run: captures both apps on identical copies of the demo book and diffs them.
#
#   parity/run.sh [out-dir]          (ONLY=scene,scene to limit; SKIP_TAURI=1 / SKIP_RUNE=1 to reuse)
#
# Setup the first time (see parity/README.md): the demo book copies and env files under
# $PARITY_HOME (default: ~/rune-parity), a production web bundle in $PARITY_HOME/dist, and
# PARITY_TAURI_CHECKOUT naming a checkout of commit 1c8065d4b, the last one with ui-tauri/.
# RUNE_ENV may name an extra env file to source before the Rune capture (e.g. a toolchain setup).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
PARITY_HOME="${PARITY_HOME:-$HOME/rune-parity}"
out="${1:-$PARITY_HOME/run-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$out"; out="$(cd "$out" && pwd)"
node_cmd=(node); command -v mise >/dev/null && node_cmd=(mise exec node@24 -- node)
if [[ -z "${SKIP_TAURI:-}" ]]; then
  : "${PARITY_TAURI_CHECKOUT:?set PARITY_TAURI_CHECKOUT to a checkout of commit 1c8065d4b (or SKIP_TAURI=1)}"
  export PARITY_TAURI_CHECKOUT
  ( set -a; source "$PARITY_HOME/env-tauri.sh"; set +a
    PARITY_DIST="$PARITY_HOME/dist" "${node_cmd[@]}" "$here/capture-tauri.mjs" "$out/tauri" )
fi
if [[ -z "${SKIP_RUNE:-}" ]]; then
  ( if [[ -n "${RUNE_ENV:-}" ]]; then source "$RUNE_ENV"; fi
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/../target}"
    set -a; source "$PARITY_HOME/env-rune.sh"; set +a
    "$here/capture-rune.sh" "$out/rune" ${RUNE_ARGS:-} ) || true
fi
python3 -I "$here/diff.py" "$out/tauri" "$out/rune" "$out"
echo "report: $out/report.md"
