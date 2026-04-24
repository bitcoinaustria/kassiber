#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage: scripts/live-sync-tests.sh [--suite bitcoin|liquid|all]
                                  [--pull-images]
                                  [--require-bitcoin-regtest]
                                  [--require-liquid-regtest]

Runs opt-in live wallet-sync integration tests against local Bitcoin Core and
Liquid (Elements + electrs-liquid) regtest stacks.

  --suite              Which test suite to run. Default: all.
                       - bitcoin : tests.test_live_sync_bitcoin
                       - liquid  : tests.test_live_sync_liquid
                       - all     : both suites
  --pull-images        Allow Docker to pull missing images (KASSIBER_LIVE_SYNC_PULL=1).
  --require-bitcoin-regtest
                       Fail (instead of skip) if Bitcoin regtest cannot start.
  --require-liquid-regtest
                       Fail (instead of skip) if Liquid regtest cannot start.

By default Docker images must already exist locally and unavailable live
services are reported as skipped tests. Both defaults flip when the matching
flags are passed.
USAGE
}

SUITE="all"

while [ $# -gt 0 ]; do
  case "$1" in
    --suite)
      SUITE="${2:?--suite needs a value (bitcoin|liquid|all)}"
      shift
      ;;
    --pull-images)
      export KASSIBER_LIVE_SYNC_PULL=1
      ;;
    --require-bitcoin-regtest)
      export KASSIBER_REQUIRE_BITCOIN_REGTEST=1
      ;;
    --require-liquid-regtest|--require-liquid)
      export KASSIBER_REQUIRE_LIQUID_REGTEST=1
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

case "$SUITE" in
  bitcoin|liquid|all) ;;
  *)
    echo "Unknown --suite value: $SUITE (expected bitcoin, liquid, or all)" >&2
    exit 2
    ;;
esac

export KASSIBER_LIVE_SYNC_TESTS=1
export PYTHONPYCACHEPREFIX="${PYTHONPYCACHEPREFIX:-/tmp/kassiber-pyc}"
export UV_CACHE_DIR="${UV_CACHE_DIR:-/tmp/kassiber-uv-cache}"

PY=()
if [ -n "${VIRTUAL_ENV:-}" ] && command -v python3 >/dev/null 2>&1; then
  PY=(python3)
elif [ -x "$ROOT/.venv/bin/python" ]; then
  PY=("$ROOT/.venv/bin/python")
elif command -v uv >/dev/null 2>&1; then
  PY=(uv run python)
else
  echo "live sync tests require an activated virtualenv, 'uv' on PATH, or a repo-local .venv" >&2
  exit 2
fi

TARGETS=()
case "$SUITE" in
  bitcoin) TARGETS=(tests.test_live_sync_bitcoin) ;;
  liquid)  TARGETS=(tests.test_live_sync_liquid) ;;
  all)     TARGETS=(tests.test_live_sync_bitcoin tests.test_live_sync_liquid) ;;
esac

"${PY[@]}" -m unittest "${TARGETS[@]}" -v
