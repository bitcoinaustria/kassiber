#!/usr/bin/env bash
# A dedicated disposable Core31 index oracle; never reuses a user's node/book.
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
export KASSIBER_REGTEST_RPC_USER=kassiber
KASSIBER_REGTEST_RPC_PASSWORD="$(uv run --locked python -c 'import secrets; print(secrets.token_urlsafe(24))')"
export KASSIBER_REGTEST_RPC_PASSWORD
CHAIN_ANALYSIS_CONTAINER="$(docker run --rm -d -p 127.0.0.1::18443 bitcoin/bitcoin:31.0 \
  -regtest=1 -server=1 -txindex=1 -txospenderindex=1 -fallbackfee=0.0002 \
  -rpcbind=0.0.0.0 -rpcallowip=0.0.0.0/0 \
  -rpcuser="$KASSIBER_REGTEST_RPC_USER" -rpcpassword="$KASSIBER_REGTEST_RPC_PASSWORD")"
CHAIN_ANALYSIS_PORT="$(docker port "$CHAIN_ANALYSIS_CONTAINER" 18443/tcp)"
export KASSIBER_REGTEST_CORE_URL="http://${CHAIN_ANALYSIS_PORT}"
export KASSIBER_INTEGRATION=1
uv run --locked python - <<'PY'
import os, time
from tests.integration.test_live_bitcoin_core_regtest import _rpc
for _ in range(100):
    try:
        info = _rpc(os.environ['KASSIBER_REGTEST_CORE_URL'], os.environ['KASSIBER_REGTEST_RPC_USER'], os.environ['KASSIBER_REGTEST_RPC_PASSWORD'], 'getblockchaininfo')
        assert info['chain'] == 'regtest'
        break
    except Exception:
        time.sleep(0.2)
else:
    raise SystemExit('Disposable Core31 did not become ready')
PY
uv run --locked python -m unittest tests.integration.test_live_chain_analysis -v
