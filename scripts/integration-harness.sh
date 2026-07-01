#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="${1:-fast}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
RUNNER=()
STARTED_COMPOSE=0
SUDO_DOCKER_ENV=KASSIBER_REGTEST_RPC_USER,KASSIBER_REGTEST_RPC_AUTH,KASSIBER_REGTEST_RPC_PORT,KASSIBER_REGTEST_BITCOIND_IMAGE
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
      KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-}" \
      KASSIBER_REGTEST_RPC_PORT="${KASSIBER_REGTEST_RPC_PORT:-}" \
      KASSIBER_REGTEST_BITCOIND_IMAGE="${KASSIBER_REGTEST_BITCOIND_IMAGE:-}" \
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
  export KASSIBER_REGTEST_CORE_URL="${KASSIBER_REGTEST_CORE_URL:-http://127.0.0.1:${KASSIBER_REGTEST_RPC_PORT}}"
  export KASSIBER_REGTEST_COMPOSE_PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-$(py -c 'import hashlib, os; print("kassiber-regtest-" + hashlib.sha256(os.getcwd().encode()).hexdigest()[:12])')}"

  STARTED_COMPOSE=0
  if [ -z "${KASSIBER_REGTEST_REUSE_CORE:-}" ] && [ "$provided_core_url" -eq 0 ]; then
    docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" -f dev/regtest/compose.bitcoin.yml up -d
    STARTED_COMPOSE=1
  fi

  cleanup() {
    if [ "$STARTED_COMPOSE" -eq 1 ] && [ -z "${KASSIBER_REGTEST_KEEP:-}" ]; then
      docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" -f dev/regtest/compose.bitcoin.yml down -v
    fi
  }
  trap cleanup EXIT

  wait_for_core
  "$@"
}

run_bitcoin_core_smoke() {
  py -m unittest tests.integration.test_live_bitcoin_core_regtest -v
}

run_demo_full() {
  py -m tests.integration.regtest_demo
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
  all)
    run_fast
    run_with_bitcoin_core run_slow_suite
    ;;
  *)
    echo "usage: $0 [fast|bitcoin-core|slow|demo|demo-full|all]" >&2
    exit 2
    ;;
esac
