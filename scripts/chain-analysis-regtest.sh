#!/usr/bin/env bash
# A dedicated disposable Core31 consensus/index oracle; never reuses a node/book.
#
# Standard-library Python run with `python3 -I` drives the Rust `kassiber`
# binary (KASSIBER_RUST_BIN, default target/release/kassiber); no uv, no
# virtual environment, no Python product (docs/plan/22-python-removal.md, D2).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
command -v python3 >/dev/null 2>&1 || {
  echo "the chain-analysis lane requires python3 (3.11 or newer, standard library only)" >&2
  exit 2
}
# `py -m MODULE` runs a module from this checkout; `py -` runs stdin with the
# checkout importable. Isolated mode keeps the environment and site out.
py() {
  if [ "${1:-}" = -m ]; then
    local module="$2"
    shift 2
    python3 -I -c 'import runpy, sys
sys.path.insert(0, sys.argv.pop(1))
runpy.run_module(sys.argv.pop(1), run_name="__main__", alter_sys=True)' "$ROOT" "$module" "$@"
  else
    python3 -I -c 'import sys
sys.path.insert(0, sys.argv[1])
exec(compile(sys.stdin.read(), "<stdin>", "exec"))' "$ROOT"
  fi
}

KASSIBER_RUST_BIN="${KASSIBER_RUST_BIN:-$ROOT/target/release/kassiber}"
if [ ! -x "$KASSIBER_RUST_BIN" ]; then
  echo "The chain-analysis lane needs the Rust binary at $KASSIBER_RUST_BIN" >&2
  echo "Build it with 'cargo build -p kassiber --release' or set KASSIBER_RUST_BIN." >&2
  exit 2
fi
KASSIBER_RUST_BIN="$(cd "$(dirname "$KASSIBER_RUST_BIN")" && pwd -P)/$(basename "$KASSIBER_RUST_BIN")"
export KASSIBER_RUST_BIN
export KASSIBER_TEST_CLI="${KASSIBER_TEST_CLI:-$KASSIBER_RUST_BIN}"
# The product reads machine-wide preferences from the account's default state
# root even with --data-root; point them at lane-owned files (as the
# integration harness does) so no read or write reaches the real app data.
mkdir -p "$ROOT/target/integration-harness/preferences"
PREFERENCES="$(cd "$ROOT/target/integration-harness/preferences" && pwd -P)"
chmod 700 "$PREFERENCES"
export KASSIBER_OFFLINE_PREFERENCE_FILE="${KASSIBER_OFFLINE_PREFERENCE_FILE:-$PREFERENCES/offline-mode.json}"
export KASSIBER_AGENT_ACCESS_FILE="${KASSIBER_AGENT_ACCESS_FILE:-$PREFERENCES/agents.json}"
export KASSIBER_UPDATE_PREFERENCE_FILE="${KASSIBER_UPDATE_PREFERENCE_FILE:-$PREFERENCES/updates.json}"
export KASSIBER_UPDATE_CACHE_FILE="${KASSIBER_UPDATE_CACHE_FILE:-$PREFERENCES/update-cache.json}"
export KASSIBER_DISABLE_UPDATE_CHECK=1

docker info >/dev/null
CHAIN_ANALYSIS_CONTAINER=""
cleanup() {
  if [[ -n "$CHAIN_ANALYSIS_CONTAINER" ]]; then
    docker rm -f "$CHAIN_ANALYSIS_CONTAINER" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
export KASSIBER_REGTEST_RPC_USER=kassiber
KASSIBER_REGTEST_RPC_PASSWORD="$(python3 -I -c 'import secrets; print(secrets.token_urlsafe(24))')"
export KASSIBER_REGTEST_RPC_PASSWORD
CHAIN_ANALYSIS_CONTAINER="$(docker run --rm -d --cpus=2 --memory=1g \
  --label at.bitcoinaustria.kassiber.integration=chain-analysis \
  -p 127.0.0.1::18443 bitcoin/bitcoin:31.0 \
  -regtest=1 -server=1 -txindex=1 -txospenderindex=1 -fallbackfee=0.0002 \
  -dbcache=64 -listen=0 -connect=0 -discover=0 -dnsseed=0 \
  -rpcbind=0.0.0.0 -rpcallowip=0.0.0.0/0 \
  -rpcuser="$KASSIBER_REGTEST_RPC_USER" -rpcpassword="$KASSIBER_REGTEST_RPC_PASSWORD")"
CHAIN_ANALYSIS_PORT="$(docker port "$CHAIN_ANALYSIS_CONTAINER" 18443/tcp)"
export KASSIBER_REGTEST_CORE_URL="http://${CHAIN_ANALYSIS_PORT}"
export KASSIBER_INTEGRATION=1
export KASSIBER_DISPOSABLE_CHAIN_ANALYSIS=1
export KASSIBER_REGTEST_RPC_TIMEOUT=10
py - <<'PY'
import os, time
from tests.integration.test_live_chain_analysis import _rpc
deadline = time.monotonic() + 45
while time.monotonic() < deadline:
    try:
        info = _rpc(os.environ['KASSIBER_REGTEST_CORE_URL'], os.environ['KASSIBER_REGTEST_RPC_USER'], os.environ['KASSIBER_REGTEST_RPC_PASSWORD'], 'getblockchaininfo')
        assert info['chain'] == 'regtest'
        break
    except Exception:
        time.sleep(0.2)
else:
    raise SystemExit('Disposable Core31 did not become ready within 45 seconds')
PY
# Mining 101 maturity blocks synchronously can exceed 10s on the capped CI node.
# This timeout applies only to the fixture RPC client, not acquisition budgets.
export KASSIBER_REGTEST_RPC_TIMEOUT=60
py -m unittest tests.integration.test_live_chain_analysis -v
