#!/usr/bin/env bash
# A dedicated disposable Core31 consensus/index oracle; never reuses a node/book.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
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
KASSIBER_REGTEST_RPC_PASSWORD="$(uv run --locked python -c 'import secrets; print(secrets.token_urlsafe(24))')"
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
uv run --locked python - <<'PY'
import os, time
from tests.integration.test_live_bitcoin_core_regtest import _rpc
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
uv run --locked python -m unittest tests.integration.test_live_chain_analysis -v
