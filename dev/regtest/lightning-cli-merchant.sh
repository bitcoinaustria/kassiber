#!/usr/bin/env bash
# Merchant-only Core Lightning wrapper for Kassiber's regtest business lane.
#
# This is the only lightning-cli path the Kassiber book stores. The customer,
# supplier, and router nodes are driven by the scenario scripts only; they are
# never created as Kassiber wallets or backends.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-}"
if [ -z "$PROJECT" ]; then
  PROJECT="$(python3 -c 'import hashlib, os; print("kassiber-regtest-" + hashlib.sha256(os.getcwd().encode()).hexdigest()[:12])' 2>/dev/null || true)"
fi
PROJECT="${PROJECT:-kassiber-regtest}"

COMPOSE_FILES=(
  -f "$ROOT/dev/regtest/compose.bitcoin.yml"
  -f "$ROOT/dev/regtest/compose.lightning.yml"
)

docker_compose() {
  if docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose "$@"
  elif sudo -n docker info >/dev/null 2>&1 && sudo -n docker compose version >/dev/null 2>&1; then
    sudo -n --preserve-env=KASSIBER_REGTEST_RPC_USER,KASSIBER_REGTEST_RPC_PASSWORD,KASSIBER_REGTEST_RPC_AUTH,KASSIBER_REGTEST_CLN_IMAGE docker compose "$@"
  elif docker info >/dev/null 2>&1 && command -v docker-compose >/dev/null 2>&1; then
    docker-compose "$@"
  elif sudo -n docker info >/dev/null 2>&1 && sudo -n docker-compose version >/dev/null 2>&1; then
    sudo -n env \
      KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-}" \
      KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-}" \
      KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-}" \
      KASSIBER_REGTEST_CLN_IMAGE="${KASSIBER_REGTEST_CLN_IMAGE:-}" \
      docker-compose "$@"
  else
    echo "Docker Compose is required to call the regtest merchant CLN node." >&2
    exit 2
  fi
}

docker_compose -p "$PROJECT" "${COMPOSE_FILES[@]}" exec -T cln_merchant lightning-cli "$@"
