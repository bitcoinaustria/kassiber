#!/usr/bin/env bash
# Record product sessions from a pytest run, once, while the Python reference
# still exists (docs/plan/22-python-removal.md).
#
#   crates/session-replay/record-sessions.sh RECORD_DIR RUN_DIR [pytest args...]
#
# Every Rust executable the suite starts (KASSIBER_RUST_BIN, KASSIBER_TEST_CLI,
# the probes it names by variable, and the cargo examples under
# target/debug/examples) runs behind session-shim. The suite runs in a user
# and mount namespace with RUN_DIR (an empty disposable directory) bound at
# /mnt, its basetemp and TMPDIR under /mnt, so session-replay can recreate
# the very same paths. Build first, all with kassiber-store/test-hooks so
# every process takes its recorded SQLite seed:
#   cargo build -p kassiber --features test-harness
#   cargo build --examples -p kassiber -p kassiber-wallets -p kassiber-journals ...
#     --features kassiber-store/test-hooks,<each package>/test-harness
#   cargo build -p kassiber-session-replay -p kassiber-clock-shim
# Run it under the fast-io wrapper; recordings never need durable writes.
# The junit report lands in RECORD_DIR; keep only sessions of passing tests.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
rec="$1"; run="$2"; shift 2
[ -d "$run" ] && [ -z "$(ls -A "$run")" ] || { echo "RUN_DIR must be an empty directory" >&2; exit 2; }
target="$(realpath target)"
shims="$target/shims"
mkdir -p "$shims" "$rec" "$run/bt" "$run/tmp"
# Synthetic passphrases: only literals of the checked-in test sources may
# enter a recording (the shim refuses a session that hands over another).
python3 -I - "$ROOT/tests" "$run/passphrase-allowlist.txt" "$rec/fixture-keys.txt" <<'PY'
import ast, pathlib, sys
literals = set()
for path in pathlib.Path(sys.argv[1]).rglob("*.py"):
    for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
        if isinstance(node, ast.Constant) and isinstance(node.value, (str, bytes)):
            value = node.value.decode("utf-8", "replace") if isinstance(node.value, bytes) else node.value
            if 0 < len(value) <= 200 and "\n" not in value:
                try:
                    value.encode("utf-8")
                except UnicodeEncodeError:
                    continue  # a lone surrogate: never a passphrase a process receives
                literals.add(value)
# Rust fixtures key their books themselves (`PRAGMA key='...'`).
import re
fixture_keys = set()
for path in pathlib.Path(sys.argv[1]).parent.joinpath("crates").rglob("*.rs"):
    if "/examples/" in str(path) or "/tests/" in str(path):
        keys = re.findall(r"PRAGMA key\s*=\s*'([^']+)'", path.read_text(encoding="utf-8"))
        literals.update(keys)
        fixture_keys.update(keys)
pathlib.Path(sys.argv[2]).write_text("\n".join(sorted(literals)) + "\n", encoding="utf-8")
# For `session-replay decrypt`, after the run: books such fixtures keyed.
pathlib.Path(sys.argv[3]).write_text("\n".join(sorted(fixture_keys)) + "\n", encoding="utf-8")
PY
# Kept beside the recording for `session-replay decrypt`; never packed.
cp "$run/passphrase-allowlist.txt" "$rec/passphrase-allowlist.txt"
python3 -I - "$shims" "$target" "$rec" "$ROOT" "$run/passphrase-allowlist.txt" <<'PY'
import json, os, re, shutil, sys
shims, target, rec, repo, allowlist = sys.argv[1:6]
# Tests often start children with a minimal environment; the recording
# settings therefore live beside the shims. Paths under /mnt are the same
# inside the recording namespace.
config = {"record_dir": rec, "root_prefixes": ["/mnt/bt", "/mnt/tmp"], "repo": repo,
          "examples_dir": f"{target}/debug/examples", "current_dir": "/mnt/current",
          "passphrase_allowlist": allowlist,
          "clock_shim": f"{target}/debug/libkassiber_clock_shim.so"}
shim = f"{target}/debug/session-shim"
examples = f"{target}/debug/examples"
# Hard links, not symlinks: tests resolve KASSIBER_RUST_BIN and look for
# examples beside it, and check each binary's dep-info for staleness.
def install(directory, names):
    os.makedirs(directory, exist_ok=True)
    json.dump(names, open(os.path.join(directory, "shim-map.json"), "w"), indent=1)
    json.dump(config, open(os.path.join(directory, "shim-config.json"), "w"), indent=1)
    for name in names:
        link = os.path.join(directory, name)
        if os.path.lexists(link):
            os.unlink(link)
        os.link(shim, link)
        shutil.copy(shim + ".d", link + ".d")
names = {}
for name in sorted(os.listdir(examples)) if os.path.isdir(examples) else []:
    path = os.path.join(examples, name)
    # cargo also writes name-<16 hex>; the plain name is the stable one.
    if os.path.isfile(path) and os.access(path, os.X_OK) and not re.search(r"-[0-9a-f]{16}$", name) and "." not in name:
        names[name] = path
install(shims, {"kassiber": f"{target}/debug/kassiber"})
install(os.path.join(shims, "examples"), names)
PY
# Tests that build their own example binaries read the executable path from
# cargo's JSON messages; the wrapper points those paths at shim links too.
real_cargo="$(command -v cargo)"
mkdir -p "$shims/bin"
cat > "$shims/bin/cargo" <<WRAP
#!/usr/bin/env bash
set -o pipefail
"$real_cargo" "\$@" | while IFS= read -r line; do
  case "\$line" in
    *'"executable":"$target/debug/examples/'*)
      name=\$(printf '%s' "\$line" | sed -n 's#.*"executable":"$target/debug/examples/\([^"]*\)".*#\1#p')
      [ -e "$shims/examples/\$name" ] || { ln -f "$target/debug/session-shim" "$shims/examples/\$name" && cp "$target/debug/session-shim.d" "$shims/examples/\$name.d"; }
      printf '%s\n' "\$line" | sed "s#\"executable\":\"$target/debug/examples/#\"executable\":\"$shims/examples/#" ;;
    *) printf '%s\n' "\$line" ;;
  esac
done
WRAP
chmod +x "$shims/bin/cargo"
export PATH="$shims/bin:$PATH"
# Each pytest process notes the test it runs (by pid) for the shims, which
# walk their ancestors to attribute a session to its test.
mkdir -p "$run/current" "$run/plugin"
cat > "$run/plugin/kassiber_session_plugin.py" <<'PLUGIN'
import faulthandler
import os


def pytest_runtest_logstart(nodeid, location):
    with open(os.path.join("/mnt/current", str(os.getpid())), "w") as handle:
        handle.write(nodeid)
    # A test that hangs under recording ends its worker, never the run.
    faulthandler.dump_traceback_later(900, exit=True)


def pytest_runtest_logfinish(nodeid, location):
    faulthandler.cancel_dump_traceback_later()
PLUGIN
export PYTHONPATH="/mnt/plugin${PYTHONPATH:+:$PYTHONPATH}"
export KASSIBER_RUST_BIN="$shims/kassiber" KASSIBER_HYBRID_RUST_BIN="$shims/kassiber" KASSIBER_TEST_CLI="$shims/kassiber"
# The probes the suite names by variable (tests/rust_fixture_binary.py).
ex="$shims/examples"
for pair in ANALYSIS_ANALYTICS_BIN=analysis_analytics_fixture ANALYSIS_DATASETS_BIN=analysis_datasets_fixture \
  ANALYSIS_DURABLE_BIN=analysis_durable_fixture ANALYSIS_ENTROPY_BIN=analysis_entropy_fixture \
  ANALYSIS_INDEX_BIN=analysis_index_fixture ANALYSIS_OPS_BIN=analysis_ops_fixture \
  ANALYSIS_TRAVERSAL_BIN=analysis_traversal_fixture CHAIN_ANALYSIS_READS_BIN=chain_analysis_reads_fixture \
  FEATURES_BIN=analysis_features_fixture PSBT_BIN=psbt_fixture PSBT_PRIVACY_BIN=psbt_privacy_fixture \
  PSBT_TRANSPORT_BIN=psbt_transport_fixture LEGACY_PRIVACY_BIN=legacy_privacy_fixture \
  LIGHTNING_FIXTURE_BIN=lightning_fixture XLSX_SERVICE_BIN=xlsx SOF_MUTATION_SERVICE_BIN=source_funds_mutations \
  SOF_CASE_SERVICE_BIN=source_funds_cases RUST_FRESHNESS_FIXTURE=freshness_fixture \
  RUST_FRESHNESS_SYNC_FIXTURE=freshness_sync_fixture RUST_GRAPH_FIXTURE=graph_fixture \
  DOCUMENT_FIXTURE=document_fixture RUST_AI_BROKER_EXAMPLE=broker_stream \
  TEST_CUSTODY_LOADER_BIN=custody-load TEST_JOURNAL_PROCESS_BIN=journal-process \
  STORE_PROBE_BIN=kassiber-store-probe; do
  [ -e "$ex/${pair#*=}" ] && export "KASSIBER_${pair%%=*}=$ex/${pair#*=}"
done
export TMPDIR=/mnt/tmp KASSIBER_OFFLINE_PREFERENCE_FILE=/mnt/tmp/offline-preference.json
unset XDG_DATA_HOME
exec unshare -Urm sh -c 'mount --bind "$1" /mnt && u=$2 && g=$3 && shift 3 && exec unshare --user --map-user="$u" --map-group="$g" "$@"' \
  sh "$run" "$(id -u)" "$(id -g)" \
  "$ROOT/.venv/bin/python" -m pytest --basetemp=/mnt/bt -p no:cacheprovider -p kassiber_session_plugin --junitxml="$rec/junit.xml" "$@"
