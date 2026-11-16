#!/usr/bin/env bash
# Pilot evaluation over every scene in src/main.rs (except ones that write to the book):
# accessibility tree check and missing-translation report, in English and German.
# Usage: tools/pilot-eval.sh <out-dir>     (needs CARGO_TARGET_DIR and a demo data root env)
#   AREA=settings tools/pilot-eval.sh <out>  only that area's scenes (src/settings/**)
set -uo pipefail
mkdir -p "$1"; out="$(cd "$1" && pwd)"
here="$(cd "$(dirname "$0")/.." && pwd)"
bin="$CARGO_TARGET_DIR/release/kassiber-desktop"
limit() { if command -v timeout >/dev/null; then timeout 120 "$@"; elif command -v gtimeout >/dev/null; then gtimeout 120 "$@"; else "$@"; fi; }
# Every file that declares a scene list (`SCENES`), in one area or the whole app.
if [[ -n "${AREA:-}" ]]; then files=$(grep -rl "SCENES" "$here/src/$AREA"); else files=$(grep -rl "SCENES" "$here/src"); fi
# Scene entries: `    ("name", …` lines (main.rs's list and each area's SCENES).
scenes=()
while IFS= read -r sc; do scenes+=("$sc"); done < <(grep -hoE '^    \("[a-z0-9-]+"' $files | sed 's/^    ("//; s/"$//' | grep -vE '^(detail-acq-apply)$' | sort -u)
echo "scenes: ${#scenes[@]}"
for lang in en de; do
  mkdir -p "$out/$lang"
  for sc in "${scenes[@]}"; do
    (cd /tmp && limit "$bin" --tree "$out/$lang/$sc.json" --i18n-report "$out/$lang/$sc.missing" --size 1440x900 --scale 1 --scene "$sc" --lang "$lang" >/dev/null 2>&1) || echo "failed: $lang $sc"
  done
  echo "== a11y ($lang)"
  python3 -I "$here/tools/a11y-check.py" "$out/$lang"/*.json > "$out/a11y-$lang.txt" 2>&1; echo "exit=$? $(wc -l < "$out/a11y-$lang.txt") lines"
  tail -5 "$out/a11y-$lang.txt"
  echo "== missing keys ($lang)"
  cat "$out/$lang"/*.missing 2>/dev/null | sed '/^$/d' | sort | uniq -c | sort -rn > "$out/missing-$lang.txt"; wc -l < "$out/missing-$lang.txt"; head -20 "$out/missing-$lang.txt"
done
