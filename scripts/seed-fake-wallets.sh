#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage: scripts/seed-fake-wallets.sh [--data-root PATH] [--workspace NAME] [--profile NAME]

Creates a local fake Kassiber workspace with three wallets:

  - Demo Cold BTC
  - Demo Hot BTC
  - Demo Liquid

It imports deterministic CSV fixtures, pairs one peg-in and one peg-out, runs
journal processing, and writes the final reports.summary JSON envelope to stdout.
Progress and the selected data root are written to stderr.
USAGE
}

DATA_ROOT=""
WORKSPACE="Fake Wallet Demo"
PROFILE="Generic Swaps"

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
  DATA_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-fake-wallets.XXXXXX")/data"
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
  echo "seed-fake-wallets requires an activated virtualenv, 'uv' on PATH, or a repo-local .venv" >&2
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
  --label "Demo Cold BTC" \
  --kind custom
run_setup wallets create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --label "Demo Hot BTC" \
  --kind custom
run_setup wallets create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --label "Demo Liquid" \
  --kind custom

run_setup wallets import-csv \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --wallet "Demo Cold BTC" \
  --file "$FIXTURES/onchain-cold.csv"
run_setup wallets import-csv \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --wallet "Demo Hot BTC" \
  --file "$FIXTURES/onchain-hot.csv"
run_setup wallets import-csv \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --wallet "Demo Liquid" \
  --file "$FIXTURES/liquid.csv"

run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code swap \
  --label "Swap review"
run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code peg-in \
  --label "Peg-in"
run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code peg-out \
  --label "Peg-out"
run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code self-transfer \
  --label "Self-transfer"
run_setup metadata tags create \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --code spend \
  --label "Spend"

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

tag_record demo-self-transfer-1 self-transfer
note_record demo-self-transfer-1 "Auto-detected cold-to-hot self-transfer"

tag_record demo-peg-in-out-1 swap
tag_record demo-peg-in-out-1 peg-in
note_record demo-peg-in-out-1 "BTC leg of the fake peg-in"
tag_record demo-peg-in-in-1 swap
tag_record demo-peg-in-in-1 peg-in
note_record demo-peg-in-in-1 "LBTC leg of the fake peg-in"

tag_record demo-peg-out-out-1 swap
tag_record demo-peg-out-out-1 peg-out
note_record demo-peg-out-out-1 "LBTC leg of the fake peg-out"
tag_record demo-peg-out-in-1 swap
tag_record demo-peg-out-in-1 peg-out
note_record demo-peg-out-in-1 "BTC leg of the fake peg-out"

tag_record demo-hot-spend-1 spend
note_record demo-hot-spend-1 "Fake on-chain spend after the self-transfer"
tag_record demo-liquid-spend-1 spend
note_record demo-liquid-spend-1 "Fake Liquid spend after the swap path"

run_setup transfers pair \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --tx-out demo-peg-in-out-1 \
  --tx-in demo-peg-in-in-1 \
  --kind peg-in \
  --policy taxable \
  --note "Fake-wallet peg-in pair"
run_setup transfers pair \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE" \
  --tx-out demo-peg-out-out-1 \
  --tx-in demo-peg-out-in-1 \
  --kind peg-out \
  --policy taxable \
  --note "Fake-wallet peg-out pair"

run_setup journals process \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE"

kassiber reports summary \
  --workspace "$WORKSPACE" \
  --profile "$PROFILE"
