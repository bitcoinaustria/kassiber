#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="${1:-fast}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
RUNNER=()
STARTED_COMPOSE=0
SUDO_DOCKER_ENV=KASSIBER_REGTEST_RPC_USER,KASSIBER_REGTEST_RPC_PASSWORD,KASSIBER_REGTEST_RPC_AUTH,KASSIBER_REGTEST_RPC_PORT,KASSIBER_REGTEST_ELEMENTS_RPC_PORT,KASSIBER_REGTEST_BITCOIND_IMAGE,KASSIBER_REGTEST_ELEMENTSD_IMAGE,KASSIBER_REGTEST_FULCRUM_IMAGE,KASSIBER_REGTEST_BACKEND_STACK_IMAGE,KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT,KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT,KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT,KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT
SUDO_DOCKER=(sudo -n --preserve-env="$SUDO_DOCKER_ENV" docker)

export PYTHONHASHSEED="${PYTHONHASHSEED:-0}"
export TZ="${TZ:-UTC}"
export LC_ALL="${LC_ALL:-C.UTF-8}"
export LANG="${LANG:-C.UTF-8}"

if [ -n "${VIRTUAL_ENV:-}" ] && command -v python3 >/dev/null 2>&1; then
  PYTHON_BIN="$(command -v python3)"
elif command -v uv >/dev/null 2>&1; then
  RUNNER=(uv run)
elif [ -x "$ROOT/.venv/bin/python" ]; then
  PYTHON_BIN="$ROOT/.venv/bin/python"
fi

py() {
  if [ ${#RUNNER[@]} -gt 0 ]; then
    "${RUNNER[@]}" python "$@"
  else
    "$PYTHON_BIN" "$@"
  fi
}

run_fast() {
  KASSIBER_NO_EGRESS=1 py -m unittest tests.test_regtest_harness -v
}

rpc_auth() {
  py - <<'PY'
import hashlib
import hmac
import os
import secrets

user = os.environ["KASSIBER_REGTEST_RPC_USER"]
password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
salt = secrets.token_hex(16)
digest = hmac.new(salt.encode("utf-8"), password.encode("utf-8"), hashlib.sha256).hexdigest()
print(f"{user}:{salt}${digest}")
PY
}

docker_compose() {
  if docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose "$@"
  elif "${SUDO_DOCKER[@]}" info >/dev/null 2>&1 && "${SUDO_DOCKER[@]}" compose version >/dev/null 2>&1; then
    "${SUDO_DOCKER[@]}" compose "$@"
  elif docker info >/dev/null 2>&1 && command -v docker-compose >/dev/null 2>&1; then
    docker-compose "$@"
  elif sudo -n docker info >/dev/null 2>&1 && sudo -n docker-compose version >/dev/null 2>&1; then
    sudo -n env \
      KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-}" \
      KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-}" \
      KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-}" \
      KASSIBER_REGTEST_RPC_PORT="${KASSIBER_REGTEST_RPC_PORT:-}" \
      KASSIBER_REGTEST_ELEMENTS_RPC_PORT="${KASSIBER_REGTEST_ELEMENTS_RPC_PORT:-}" \
      KASSIBER_REGTEST_BITCOIND_IMAGE="${KASSIBER_REGTEST_BITCOIND_IMAGE:-}" \
      KASSIBER_REGTEST_ELEMENTSD_IMAGE="${KASSIBER_REGTEST_ELEMENTSD_IMAGE:-}" \
      KASSIBER_REGTEST_FULCRUM_IMAGE="${KASSIBER_REGTEST_FULCRUM_IMAGE:-}" \
      KASSIBER_REGTEST_BACKEND_STACK_IMAGE="${KASSIBER_REGTEST_BACKEND_STACK_IMAGE:-}" \
      KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT="${KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT:-}" \
      KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT="${KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT:-}" \
      KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT="${KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT:-}" \
      KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT="${KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT:-}" \
      docker-compose "$@"
  else
    echo "Docker Compose is required for the slow regtest lane." >&2
    echo "Install Docker or set KASSIBER_REGTEST_CORE_URL with matching RPC credentials for an already-running regtest node." >&2
    exit 2
  fi
}

probe_core() {
  local debug="${1:-0}"
  KASSIBER_REGTEST_PROBE_DEBUG="$debug" py - <<'PY'
import base64
import json
import os
import sys
from urllib import error, request

url = os.environ["KASSIBER_REGTEST_CORE_URL"]
user = os.environ["KASSIBER_REGTEST_RPC_USER"]
password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
debug = os.environ.get("KASSIBER_REGTEST_PROBE_DEBUG") == "1"
payload = json.dumps({"jsonrpc": "1.0", "id": "probe", "method": "getblockchaininfo", "params": []}).encode()
req = request.Request(url, data=payload, headers={"Content-Type": "application/json"})
req.add_header("Authorization", "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode())
try:
    with request.urlopen(req, timeout=3) as response:
        body = json.loads(response.read().decode())
except error.HTTPError as exc:
    body_text = exc.read().decode(errors="replace")
    if debug:
        print(f"HTTP {exc.code}: {body_text}", file=sys.stderr)
    sys.exit(1)
except Exception as exc:
    if debug:
        print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
    sys.exit(1)
ok = body.get("result", {}).get("chain") == "regtest"
if debug and not ok:
    print(json.dumps(body, sort_keys=True), file=sys.stderr)
sys.exit(0 if ok else 1)
PY
}

wait_for_core() {
  local deadline
  deadline=$((SECONDS + 90))
  until probe_core 0
  do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for bitcoind regtest RPC." >&2
      probe_core 1 || true
      return 1
    fi
    sleep 2
  done
}

run_with_bitcoin_core() {
  local provided_core_url=0
  if [ -n "${KASSIBER_REGTEST_CORE_URL:-}" ]; then
    provided_core_url=1
  fi

  export KASSIBER_INTEGRATION=1
  export KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-kassiber}"
  export KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-$(py -c 'import secrets; print(secrets.token_urlsafe(24))')}"
  export KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-$(rpc_auth)}"
  export KASSIBER_REGTEST_RPC_PORT="${KASSIBER_REGTEST_RPC_PORT:-18443}"
  export KASSIBER_REGTEST_ELEMENTS_RPC_PORT="${KASSIBER_REGTEST_ELEMENTS_RPC_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 104))}"
  export KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT="${KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 100))}"
  export KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT="${KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 101))}"
  export KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT="${KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 102))}"
  export KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT="${KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 103))}"
  export KASSIBER_REGTEST_CORE_URL="${KASSIBER_REGTEST_CORE_URL:-http://127.0.0.1:${KASSIBER_REGTEST_RPC_PORT}}"
  export KASSIBER_REGTEST_COMPOSE_PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-$(py -c 'import hashlib, os; print("kassiber-regtest-" + hashlib.sha256(os.getcwd().encode()).hexdigest()[:12])')}"

  STARTED_COMPOSE=0
  cleanup() {
    if [ "$STARTED_COMPOSE" -eq 1 ] && [ -z "${KASSIBER_REGTEST_KEEP:-}" ]; then
      docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" -f dev/regtest/compose.bitcoin.yml down -v
    fi
  }
  trap cleanup EXIT

  if [ -z "${KASSIBER_REGTEST_REUSE_CORE:-}" ] && [ "$provided_core_url" -eq 0 ]; then
    # Mark before `up` so the EXIT trap also removes a half-created project
    # (network/volume/container) when startup fails, e.g. on a port collision.
    STARTED_COMPOSE=1
    if ! docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" -f dev/regtest/compose.bitcoin.yml up -d; then
      echo "Failed to start the regtest bitcoind container." >&2
      echo "If port ${KASSIBER_REGTEST_RPC_PORT} is already taken (e.g. by the demo-up node)," >&2
      echo "stop it with './scripts/integration-harness.sh demo-down' or pick another port" >&2
      echo "via KASSIBER_REGTEST_RPC_PORT=18444 before running this lane." >&2
      exit 1
    fi
  fi

  wait_for_core
  "$@"
}

run_bitcoin_core_smoke() {
  py -m unittest tests.integration.test_live_bitcoin_core_regtest -v
}

run_demo_full() {
  py -m tests.integration.regtest_demo
}

DEMO_HOME="${KASSIBER_REGTEST_DEMO_HOME:-$HOME/.kassiber/regtest-demo}"
DEMO_MANIFEST="$DEMO_HOME/demo-manifest.json"
DEMO_SCENARIO="dev/regtest/scenarios/full_accounting.json"

demo_manifest_get() {
  KASSIBER_DEMO_MANIFEST="$DEMO_MANIFEST" KASSIBER_DEMO_KEY="$1" py - <<'PY'
import json
import os

try:
    with open(os.environ["KASSIBER_DEMO_MANIFEST"], "r", encoding="utf-8") as handle:
        print(json.load(handle).get(os.environ["KASSIBER_DEMO_KEY"]) or "")
except (OSError, ValueError):
    print("")
PY
}

demo_scenario_checksum() {
  py -c 'import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$DEMO_SCENARIO"
}

demo_write_manifest() {
  local checksum="$1"
  KASSIBER_DEMO_MANIFEST="$DEMO_MANIFEST" \
  KASSIBER_DEMO_SCENARIO_ID="full-accounting-v1" \
  KASSIBER_DEMO_SCENARIO_CHECKSUM="$checksum" \
  KASSIBER_DEMO_HOME_DIR="$DEMO_HOME" \
    py - <<'PY'
import datetime
import json
import os

manifest_path = os.environ["KASSIBER_DEMO_MANIFEST"]
home = os.environ["KASSIBER_DEMO_HOME_DIR"]
manifest = {
    "schema_version": 1,
    "scenario_id": os.environ["KASSIBER_DEMO_SCENARIO_ID"],
    "scenario_checksum": os.environ["KASSIBER_DEMO_SCENARIO_CHECKSUM"],
    "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "data_root": os.path.join(home, "data"),
    "export_dir": os.path.join(home, "exports"),
    "core_url": os.environ["KASSIBER_REGTEST_CORE_URL"],
    "elements_core_url": f"http://127.0.0.1:{os.environ['KASSIBER_REGTEST_ELEMENTS_RPC_PORT']}",
    "bitcoin_electrum_url": f"tcp://127.0.0.1:{os.environ['KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT']}",
    "bitcoin_mempool_url": f"http://127.0.0.1:{os.environ['KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT']}/api",
    "liquid_electrum_url": f"tcp://127.0.0.1:{os.environ['KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT']}",
    "liquid_mempool_url": f"http://127.0.0.1:{os.environ['KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT']}/api",
    "compose_project": os.environ.get("KASSIBER_REGTEST_COMPOSE_PROJECT", ""),
    "rpc_user": os.environ["KASSIBER_REGTEST_RPC_USER"],
    "rpc_password": os.environ["KASSIBER_REGTEST_RPC_PASSWORD"],
}
with open(manifest_path, "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2, sort_keys=True)
    handle.write("\n")
os.chmod(manifest_path, 0o600)
PY
}

demo_build_book() {
  local checksum
  checksum="$(demo_scenario_checksum)"
  if [ -z "${KASSIBER_REGTEST_DEMO_REBUILD:-}" ] \
    && [ -d "$DEMO_HOME/data" ] \
    && [ "$(demo_manifest_get scenario_checksum)" = "$checksum" ]; then
    demo_write_manifest "$checksum"
    echo "Reusing existing demo book (scenario unchanged): $DEMO_HOME/data"
    return 0
  fi

  rm -rf "$DEMO_HOME/data" "$DEMO_HOME/exports" "$DEMO_HOME/imports" \
    "$DEMO_HOME/demo-summary.json" "$DEMO_MANIFEST"
  mkdir -p "$DEMO_HOME"
  echo "Building the demo book (a few minutes of regtest history)..."
  KASSIBER_REGTEST_DEMO_ROOT="$DEMO_HOME" py -m tests.integration.regtest_demo \
    --keep-core-wallets \
    --json-output "$DEMO_HOME/demo-summary.json" >/dev/null

  demo_write_manifest "$checksum"
}

demo_print_instructions() {
  cat <<EOF

Demo environment is up.
  data root:  $DEMO_HOME/data
  exports:    $DEMO_HOME/exports
  Core RPC:   $KASSIBER_REGTEST_CORE_URL (regtest; credentials in $DEMO_MANIFEST)
  Elements RPC: http://127.0.0.1:$KASSIBER_REGTEST_ELEMENTS_RPC_PORT (elementsregtest; same credentials)
  BTC Electrum: tcp://127.0.0.1:$KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT
  BTC mempool:  http://127.0.0.1:$KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT/api
  LBTC Electrum: tcp://127.0.0.1:$KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT
  LBTC mempool:  http://127.0.0.1:$KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT/api

Next steps:
  cd ui-tauri && pnpm dev:demo                       # dev preview on the real demo book
  uv run python -m kassiber --data-root "$DEMO_HOME/data" reports summary
  ./dev/regtest/bitcoin-cli.sh getblockchaininfo    # poke the regtest node
  ./scripts/integration-harness.sh demo-down         # stop the node (book + chain kept)
  ./scripts/integration-harness.sh demo-down --purge # remove node, chain, and demo book
EOF
}

run_demo_up() {
  # The dev demo stack is machine-global (one demo book per developer), so use
  # a fixed Compose project instead of the per-worktree test project, keep the
  # node running after the script exits, and persist the generated RPC
  # credentials so later demo-up runs still match the book's stored backend.
  export KASSIBER_REGTEST_COMPOSE_PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-kassiber-regtest-demo}"
  export KASSIBER_REGTEST_KEEP=1
  if [ -z "${KASSIBER_REGTEST_RPC_USER:-}" ]; then
    KASSIBER_REGTEST_RPC_USER="$(demo_manifest_get rpc_user)"
    [ -n "$KASSIBER_REGTEST_RPC_USER" ] && export KASSIBER_REGTEST_RPC_USER
  fi
  if [ -z "${KASSIBER_REGTEST_RPC_PASSWORD:-}" ]; then
    KASSIBER_REGTEST_RPC_PASSWORD="$(demo_manifest_get rpc_password)"
    [ -n "$KASSIBER_REGTEST_RPC_PASSWORD" ] && export KASSIBER_REGTEST_RPC_PASSWORD
  fi
  run_with_bitcoin_core demo_build_book
  demo_print_instructions
}

run_demo_tick() {
  local count="${1:-1}"
  if [ ! -f "$DEMO_MANIFEST" ] || [ ! -f "$DEMO_HOME/demo-summary.json" ]; then
    echo "No demo book at $DEMO_HOME. Run './scripts/integration-harness.sh demo-up' first." >&2
    exit 2
  fi
  export KASSIBER_INTEGRATION=1
  export KASSIBER_REGTEST_CORE_URL="${KASSIBER_REGTEST_CORE_URL:-$(demo_manifest_get core_url)}"
  export KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-$(demo_manifest_get rpc_user)}"
  export KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-$(demo_manifest_get rpc_password)}"
  if ! probe_core 0; then
    echo "Demo regtest node is not reachable at $KASSIBER_REGTEST_CORE_URL." >&2
    echo "Start it with './scripts/integration-harness.sh demo-up'." >&2
    exit 2
  fi
  py -m tests.integration.regtest_demo \
    --tick \
    --summary "$DEMO_HOME/demo-summary.json" \
    --tick-count "$count"
  echo
  echo "New business activity is confirmed on the demo node."
  echo "Refresh/sync in the app (or run it directly) to import it:"
  echo "  uv run python -m kassiber --data-root \"$DEMO_HOME/data\" wallets sync --all"
}

run_demo_down() {
  local purge="${1:-}"
  local project
  project="$(demo_manifest_get compose_project)"
  project="${KASSIBER_REGTEST_COMPOSE_PROJECT:-${project:-kassiber-regtest-demo}}"
  if [ "$purge" = "--purge" ]; then
    docker_compose -p "$project" -f dev/regtest/compose.bitcoin.yml down -v
    rm -rf "$DEMO_HOME"
    echo "Demo node, chain volume, and demo book removed."
  else
    docker_compose -p "$project" -f dev/regtest/compose.bitcoin.yml down
    echo "Demo node stopped. Chain volume and demo book kept; 'demo-up' resumes them."
  fi
}

run_slow_suite() {
  run_bitcoin_core_smoke
  run_demo_full
}

run_bitcoin_core() {
  run_with_bitcoin_core run_bitcoin_core_smoke
}

run_regtest_demo_full() {
  run_with_bitcoin_core run_demo_full
}

case "$MODE" in
  fast)
    run_fast
    ;;
  bitcoin-core|slow)
    run_bitcoin_core
    ;;
  demo|demo-full)
    run_regtest_demo_full
    ;;
  demo-up)
    run_demo_up
    ;;
  demo-tick)
    run_demo_tick "${2:-1}"
    ;;
  demo-down)
    run_demo_down "${2:-}"
    ;;
  all)
    run_fast
    run_with_bitcoin_core run_slow_suite
    ;;
  *)
    echo "usage: $0 [fast|bitcoin-core|slow|demo|demo-full|demo-up|demo-tick [N]|demo-down [--purge]|all]" >&2
    exit 2
    ;;
esac
