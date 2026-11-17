#!/usr/bin/env bash
# Captures every scene of scenes.json from the Rune build: `--snap` PNG + `--tree` JSON per scene.
#
#   parity/capture-rune.sh <out-dir> [extra rune args…]     (ONLY=a,b to limit scenes)
#
# Needs CARGO_TARGET_DIR (the built binary), KASSIBER_BIN, KASSIBER_DEMO_DATA_ROOT and the
# isolated KASSIBER_*_FILE preference paths (see parity/run.sh). Frames are taken at t=0.5 s so
# `--font-smoothing` (applied from the second frame) and any settle motion have landed.
set -uo pipefail
EXTRA=("${@:2}")
mkdir -p "$1"; out="$(cd "$1" && pwd)"; shift
mkdir -p "$out"
here="$(cd "$(dirname "$0")" && pwd)"
bin="$CARGO_TARGET_DIR/release/kassiber-desktop"
python3 -I - "$here/scenes.json" "${ONLY:-}" <<'PY' > "$out/.plan"
import json, sys, shlex
import os
cfg = json.load(open(sys.argv[1])); only = [s for s in sys.argv[2].split(",") if s]
extra = os.path.join(os.path.dirname(sys.argv[1]), "scenes.d")
for f in sorted(os.listdir(extra)) if os.path.isdir(extra) else []:
    if f.endswith(".json"):
        cfg["scenes"] += json.load(open(os.path.join(extra, f)))["scenes"]
# Demo-book fixtures: `{fx:name}` / `{fx:name.field}` in scene args (parity/fixtures.json).
fx = json.load(open(os.path.join(os.path.dirname(sys.argv[1]), "fixtures.json")))["fixtures"]
import re
expand = lambda a: re.sub(r"\{fx:([a-z_]+)(?:\.([a-z_]+))?\}", lambda m: fx[m.group(1)][m.group(2) or "id"], a)
for s in cfg["scenes"]:
    if only and s["name"] not in only: continue
    s["rune"]["args"] = [expand(a) for a in s["rune"].get("args", [])]
    w = s.get("width", cfg["width"]); h = s.get("height", cfg["height"])
    # Tauri's captures run at the web's default UI scale (--app-ui-scale 0.9); Rune now defaults to
    # 100 % on the owner's request, so its captures pin 90 % to stay comparable.
    args = ["--scene", s["rune"]["scene"], *s["rune"].get("args", []), "--size", f"{w}x{h}", "--scale", "1", "--app-scale", "0.9"]
    print(s["name"] + "\t" + " ".join(shlex.quote(a) for a in args))
PY
status=0
# GNU timeout where present (Linux, or coreutils' gtimeout); macOS has neither by default.
limit() { if command -v timeout >/dev/null; then timeout 180 "$@"; elif command -v gtimeout >/dev/null; then gtimeout 180 "$@"; else "$@"; fi; }
while IFS=$'\t' read -r name args; do
  eval "set -- $args"
  if (cd /tmp && limit "$bin" "$@" "${EXTRA[@]}" --at 0.5 --snap "$out/$name.png" >/dev/null 2>"$out/$name.log") \
     && (cd /tmp && limit "$bin" "$@" "${EXTRA[@]}" --at 0.5 --tree "$out/$name.tree.json" >/dev/null 2>>"$out/$name.log"); then
    echo "$name ok"
  else
    echo "$name FAILED: $(grep -v '^rune:' "$out/$name.log" | tail -2 | tr '\n' ' ')"; status=1
  fi
done < "$out/.plan"
exit $status
