#!/usr/bin/env bash
# One-shot: fetch Blinker TTFs into kassiber/ui/resources/fonts/Blinker/.
# The desktop shell's QFontDatabase loader picks them up automatically on next launch.
# License: SIL Open Font License 1.1 — https://github.com/google/fonts/blob/main/ofl/blinker/OFL.txt

set -euo pipefail

dest="$(dirname "$0")/../kassiber/ui/resources/fonts/Blinker"
mkdir -p "$dest"

weights=(Thin ExtraLight Light Regular SemiBold Bold ExtraBold Black)
base="https://raw.githubusercontent.com/google/fonts/main/ofl/blinker"

for w in "${weights[@]}"; do
    out="$dest/Blinker-$w.ttf"
    if [[ -f "$out" ]]; then
        echo "skip $w (exists)"
        continue
    fi
    echo "fetch $w"
    curl -sSL -o "$out" "$base/Blinker-$w.ttf"
done

echo
echo "Done. Files in: $dest"
ls -la "$dest"
