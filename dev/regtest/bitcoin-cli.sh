#!/usr/bin/env bash
# Thin wrapper around the demo regtest node's bitcoin-cli (BTCPayServer's
# docker-bitcoin-cli.sh pattern) so developers don't have to remember the
# Compose project or where the credentials live.
#
#   ./dev/regtest/bitcoin-cli.sh getblockchaininfo
#   ./dev/regtest/bitcoin-cli.sh -generate 1
set -euo pipefail

DEMO_HOME="${KASSIBER_REGTEST_DEMO_HOME:-$HOME/.kassiber/regtest-demo}"
MANIFEST="$DEMO_HOME/demo-manifest.json"
COMPOSE_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/compose.bitcoin.yml"

if [ ! -f "$MANIFEST" ]; then
  echo "No demo manifest at $MANIFEST." >&2
  echo "Run ./scripts/integration-harness.sh demo-up first." >&2
  exit 2
fi

manifest_get() {
  python3 -c 'import json, sys; print(json.load(open(sys.argv[1])).get(sys.argv[2]) or "")' "$MANIFEST" "$1"
}

PROJECT="$(manifest_get compose_project)"
PROJECT="${PROJECT:-kassiber-regtest-demo}"
RPC_USER="$(manifest_get rpc_user)"
RPC_PASSWORD="$(manifest_get rpc_password)"

if docker compose version >/dev/null 2>&1; then
  COMPOSE=(docker compose)
else
  COMPOSE=(docker-compose)
fi

exec "${COMPOSE[@]}" -p "$PROJECT" -f "$COMPOSE_FILE" exec -T bitcoind \
  bitcoin-cli -regtest -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" "$@"
