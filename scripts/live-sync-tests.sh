#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage: scripts/live-sync-tests.sh [--pull-images] [--require-bitcoin-regtest] [--require-liquid]

Runs opt-in live wallet-sync integration tests.

By default Docker images must already exist locally so this script does not
pull from a registry. Pass --pull-images to allow Docker to fetch missing
images before running the tests.

By default unavailable live services are reported as skipped tests. Use
--require-bitcoin-regtest when you want the Bitcoin Core regtest Docker path to
fail instead of skip if Docker is unavailable or the image is missing. Use
--require-liquid when you have configured a local Liquid backend and want that
path to fail instead of skip.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --pull-images)
      export KASSIBER_LIVE_SYNC_PULL=1
      ;;
    --require-bitcoin-regtest)
      export KASSIBER_REQUIRE_BITCOIN_REGTEST=1
      ;;
    --require-liquid)
      export KASSIBER_REQUIRE_LIQUID_LIVE=1
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

export KASSIBER_LIVE_SYNC_TESTS=1
export PYTHONPYCACHEPREFIX="${PYTHONPYCACHEPREFIX:-/tmp/kassiber-pyc}"
export UV_CACHE_DIR="${UV_CACHE_DIR:-/tmp/kassiber-uv-cache}"

if [ -n "${VIRTUAL_ENV:-}" ] && command -v python3 >/dev/null 2>&1; then
  python3 -m unittest tests.test_live_sync_regtest -v
elif [ -x "$ROOT/.venv/bin/python" ]; then
  "$ROOT/.venv/bin/python" -m unittest tests.test_live_sync_regtest -v
elif command -v uv >/dev/null 2>&1; then
  uv run python -m unittest tests.test_live_sync_regtest -v
else
  echo "live sync tests require an activated virtualenv, 'uv' on PATH, or a repo-local .venv" >&2
  exit 2
fi
