#!/usr/bin/env bash
# Record product sessions from a pytest run, once, while the Python reference
# still exists (docs/plan/22-python-removal.md).
#
#   crates/session-replay/record-sessions.sh RECORD_DIR RUN_DIR [pytest args...]
#
# Every Rust executable the suite starts (KASSIBER_RUST_BIN, KASSIBER_TEST_CLI,
# and the cargo examples under target/debug/examples) runs behind
# session-shim. The suite runs in a user and mount namespace with RUN_DIR (an
# empty disposable directory) bound at /mnt, its basetemp and TMPDIR under
# /mnt, so session-replay can recreate the very same paths. Build first:
#   cargo build -p kassiber --features test-harness
#   cargo build -p kassiber-session-replay
# The junit report lands in RECORD_DIR; keep only sessions of passing tests.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
rec="$1"; run="$2"; shift 2
[ -d "$run" ] && [ -z "$(ls -A "$run")" ] || { echo "RUN_DIR must be an empty directory" >&2; exit 2; }
target="$(realpath target)"
shims="$target/shims"
mkdir -p "$shims" "$rec" "$run/bt" "$run/tmp"
python3 -I - "$shims" "$target" <<'PY'
import json, os, re, sys
shims, target = sys.argv[1], sys.argv[2]
names = {"kassiber": f"{target}/debug/kassiber"}
examples = f"{target}/debug/examples"
for name in sorted(os.listdir(examples)) if os.path.isdir(examples) else []:
    path = os.path.join(examples, name)
    # cargo also writes name-<16 hex>; the plain name is the stable one.
    if os.path.isfile(path) and os.access(path, os.X_OK) and not re.search(r"-[0-9a-f]{16}$", name) and "." not in name:
        names[name] = path
json.dump(names, open(os.path.join(shims, "shim-map.json"), "w"), indent=1)
for name in names:
    link = os.path.join(shims, name)
    if os.path.lexists(link):
        os.unlink(link)
    os.symlink(f"{target}/debug/session-shim", link)
PY
export KASSIBER_SESSION_RECORD_DIR="$rec"
export KASSIBER_SESSION_ROOT_PREFIXES="/mnt/bt:/mnt/tmp"
export KASSIBER_SESSION_REPO="$ROOT"
export KASSIBER_RUST_BIN="$shims/kassiber" KASSIBER_HYBRID_RUST_BIN="$shims/kassiber" KASSIBER_TEST_CLI="$shims/kassiber"
export TMPDIR=/mnt/tmp KASSIBER_OFFLINE_PREFERENCE_FILE=/mnt/tmp/offline-preference.json
unset XDG_DATA_HOME
exec unshare -Urm sh -c 'mount --bind "$1" /mnt && u=$2 && g=$3 && shift 3 && exec unshare --user --map-user="$u" --map-group="$g" "$@"' \
  sh "$run" "$(id -u)" "$(id -g)" \
  "$ROOT/.venv/bin/python" -m pytest --basetemp=/mnt/bt -p no:cacheprovider --junitxml="$rec/junit.xml" "$@"
