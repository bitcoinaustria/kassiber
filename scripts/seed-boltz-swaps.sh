#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage: scripts/seed-boltz-swaps.sh [--data-root PATH] [--workspace NAME] [--profile NAME]

Creates a local fake Kassiber workspace with two wallets:

  - Boltz Demo Hot BTC
  - Boltz Demo Liquid

It imports deterministic CSV fixtures that model a Boltz-style BTC <-> LBTC
chain swap round trip (forward and reverse) including Boltz's typical service
fee spread, pairs both swap directions with `--kind chain-swap --policy
taxable`, runs journal processing, and writes the final `reports.summary`
machine envelope to stdout. Progress and the selected data root are written
to stderr.
USAGE
}

DATA_ROOT=""
WORKSPACE="Boltz Chain-Swap Demo"
PROFILE="Generic Chain Swaps"

while [ $# -gt 0 ]; do
  case "$1" in
    --data-root)
      DATA_ROOT="${2:?--data-root needs a path}"
      shift
      ;;
    --workspace)
      WORKSPACE="${2:?--workspace needs a name}"
      shift
      ;;
    --profile)
      PROFILE="${2:?--profile needs a name}"
      shift
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

if [ -z "$DATA_ROOT" ]; then
  DATA_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-boltz-swaps.XXXXXX")/data"
fi

export PYTHONPYCACHEPREFIX="${PYTHONPYCACHEPREFIX:-/tmp/kassiber-pyc}"
export UV_CACHE_DIR="${UV_CACHE_DIR:-/tmp/kassiber-uv-cache}"

if [ -n "${VIRTUAL_ENV:-}" ] && command -v python3 >/dev/null 2>&1; then
  PY=(python3)
elif [ -x "$ROOT/.venv/bin/python" ]; then
  PY=("$ROOT/.venv/bin/python")
elif command -v uv >/dev/null 2>&1; then
  PY=(uv run python)
else
  echo "seed-boltz-swaps requires an activated virtualenv, 'uv' on PATH, or a repo-local .venv" >&2
  exit 2
fi

kassiber() {
  "${PY[@]}" -m kassiber --data-root "$DATA_ROOT" --machine "$@"
}

run_setup() {
  echo "> kassiber $*" >&2
  kassiber "$@" >&2
}

FIXTURES="$ROOT/tests/fixtures/fake_wallets"

echo "data_root=$DATA_ROOT" >&2
run_setup init
run_setup workspaces create "$WORKSPACE"
run_setup profiles create \
  --workspace "$WORKSPACE" \
  --fiat-currency USD \
  --tax-country generic \
  --gains-algorithm FIFO \
  "$PROFILE"

run_setup wallets create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --label "Boltz Demo Hot BTC" \
  --kind custom
run_setup wallets create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --label "Boltz Demo Liquid" \
  --kind custom

run_setup wallets import-csv \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --wallet "Boltz Demo Hot BTC" \
  --file "$FIXTURES/boltz_onchain.csv"
run_setup wallets import-csv \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --wallet "Boltz Demo Liquid" \
  --file "$FIXTURES/boltz_liquid.csv"

run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code boltz \
  --label "Boltz swap"
run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code chain-swap \
  --label "Chain swap"
run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code service-fee \
  --label "Service fee (spread)"

tag_record() {
  run_setup metadata records tag add \
    --workspace "$WORKSPACE" \
    --profile "$PROFILE" \
    --transaction "$1" \
    --tag "$2"
}

note_record() {
  run_setup metadata records note set \
    --workspace "$WORKSPACE" \
    --profile "$PROFILE" \
    --transaction "$1" \
    --note "$2"
}

for txid in boltz-fwd-btc-out-1 boltz-fwd-lbtc-in-1 boltz-rev-lbtc-out-1 boltz-rev-btc-in-1; do
  tag_record "$txid" boltz
  tag_record "$txid" chain-swap
done
tag_record boltz-fwd-lbtc-in-1 service-fee
tag_record boltz-rev-btc-in-1 service-fee

note_record boltz-fwd-btc-out-1 "BTC leg of forward Boltz chain-swap (user pays Boltz on BTC)"
note_record boltz-fwd-lbtc-in-1 "LBTC leg of forward Boltz chain-swap (user receives LBTC, minus service fee spread)"
note_record boltz-rev-lbtc-out-1 "LBTC leg of reverse Boltz chain-swap (user pays Boltz on Liquid)"
note_record boltz-rev-btc-in-1 "BTC leg of reverse Boltz chain-swap (user receives BTC, minus service fee spread)"

run_setup transfers pair \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --tx-out boltz-fwd-btc-out-1 \
  --tx-in boltz-fwd-lbtc-in-1 \
  --kind chain-swap \
  --policy taxable \
  --note "Forward Boltz chain-swap: BTC -> LBTC"

run_setup transfers pair \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --tx-out boltz-rev-lbtc-out-1 \
  --tx-in boltz-rev-btc-in-1 \
  --kind chain-swap \
  --policy taxable \
  --note "Reverse Boltz chain-swap: LBTC -> BTC"

run_setup journals process \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE"

kassiber reports summary \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE"
