#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="${1:-fast}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
RUNNER=()
STARTED_COMPOSE=0
SUDO_DOCKER=(sudo -n --preserve-env=KASSIBER_REGTEST_RPC_USER,KASSIBER_REGTEST_RPC_PASSWORD,KASSIBER_REGTEST_RPC_PORT,KASSIBER_REGTEST_BITCOIND_IMAGE docker)

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

docker_compose() {
  if docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose "$@"
  elif "${SUDO_DOCKER[@]}" info >/dev/null 2>&1 && "${SUDO_DOCKER[@]}" compose version >/dev/null 2>&1; then
    "${SUDO_DOCKER[@]}" compose "$@"
  elif docker info >/dev/null 2>&1 && command -v docker-compose >/dev/null 2>&1; then
    docker-compose "$@"
  elif sudo -n docker info >/dev/null 2>&1 && sudo -n docker-compose version >/dev/null 2>&1; then
    sudo docker-compose "$@"
  else
    echo "Docker Compose is required for the slow regtest lane." >&2
    echo "Install Docker or set KASSIBER_REGTEST_CORE_URL for an already-running regtest node." >&2
    exit 2
  fi
}

wait_for_core() {
  local deadline
  deadline=$((SECONDS + 90))
  until py - <<'PY'
import base64
import json
import os
import sys
from urllib import request

url = os.environ["KASSIBER_REGTEST_CORE_URL"]
user = os.environ["KASSIBER_REGTEST_RPC_USER"]
password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
payload = json.dumps({"jsonrpc": "1.0", "id": "probe", "method": "getblockchaininfo", "params": []}).encode()
req = request.Request(url, data=payload, headers={"Content-Type": "application/json"})
req.add_header("Authorization", "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode())
try:
    with request.urlopen(req, timeout=3) as response:
        body = json.loads(response.read().decode())
except Exception:
    sys.exit(1)
sys.exit(0 if body.get("result", {}).get("chain") == "regtest" else 1)
PY
  do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for bitcoind regtest RPC." >&2
      return 1
    fi
    sleep 2
  done
}

run_bitcoin_core() {
  export KASSIBER_INTEGRATION=1
  export KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-kassiber}"
  export KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-$(py -c 'import secrets; print(secrets.token_urlsafe(24))')}"
  export KASSIBER_REGTEST_RPC_PORT="${KASSIBER_REGTEST_RPC_PORT:-18443}"
  export KASSIBER_REGTEST_CORE_URL="${KASSIBER_REGTEST_CORE_URL:-http://127.0.0.1:${KASSIBER_REGTEST_RPC_PORT}}"

  STARTED_COMPOSE=0
  if [ -z "${KASSIBER_REGTEST_REUSE_CORE:-}" ]; then
    docker_compose -f dev/regtest/compose.bitcoin.yml up -d
    STARTED_COMPOSE=1
  fi

  cleanup() {
    if [ "$STARTED_COMPOSE" -eq 1 ] && [ -z "${KASSIBER_REGTEST_KEEP:-}" ]; then
      docker_compose -f dev/regtest/compose.bitcoin.yml down -v
    fi
  }
  trap cleanup EXIT

  wait_for_core
  py -m unittest tests.integration.test_live_bitcoin_core_regtest -v
}

case "$MODE" in
  fast)
    run_fast
    ;;
  bitcoin-core|slow)
    run_bitcoin_core
    ;;
  all)
    run_fast
    run_bitcoin_core
    ;;
  *)
    echo "usage: $0 [fast|bitcoin-core|slow|all]" >&2
    exit 2
    ;;
esac
