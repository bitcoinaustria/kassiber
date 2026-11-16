#!/usr/bin/env bash
# Dumps the element tree of every scene and runs the a11y checker over them.
# Usage: tools/a11y-scenes.sh <out-dir> [--lang de]
set -uo pipefail
mkdir -p "$1"; out="$(cd "$1" && pwd)"; shift
mkdir -p "$out"
bin="$CARGO_TARGET_DIR/release/kassiber-desktop"
scenes=(dark light collapsed search search-typed books user-menu refresh-menu notifications privacy-menu theme-menu network alpha locked toasts sync-card settings dev-tools add-connection chart-30d chart-all-log chart-hover chart-gear chart-gear-incoming chart-year chart-controls chart-expanded chart-basis-gap)
for sc in "${scenes[@]}"; do
  (cd /tmp && "$bin" --tree "$out/$sc.json" --size 1440x900 --scale 1 --scene "$sc" "$@" >/dev/null 2>&1) || echo "tree failed: $sc"
done
python3 -I "$(dirname "$0")/a11y-check.py" "$out"/*.json
