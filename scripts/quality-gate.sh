#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc

run() {
  echo
  echo "> $*"
  "$@"
}

PYTHON_BIN="python3"
RUNNER=()

# Honor an already-activated virtualenv before falling back to repo-local tooling.
if [ -n "${VIRTUAL_ENV:-}" ] && command -v python3 >/dev/null 2>&1; then
  PYTHON_BIN="$(command -v python3)"
elif command -v uv >/dev/null 2>&1; then
  RUNNER=(uv run)
elif [ -x "$ROOT/.venv/bin/python" ]; then
  PYTHON_BIN="$ROOT/.venv/bin/python"
else
  echo "quality gate requires an activated virtualenv, 'uv' on PATH, or a repo-local .venv at $ROOT/.venv" >&2
  exit 2
fi

py() {
  if [ ${#RUNNER[@]} -gt 0 ]; then
    "${RUNNER[@]}" python "$@"
  else
    "$PYTHON_BIN" "$@"
  fi
}

run py -m py_compile kassiber/*.py

run py -m unittest tests.test_cli_smoke -v
run py -m unittest tests.test_daemon_smoke -v
run py -m unittest tests.test_review_regressions -v

echo
echo "> CLI help smoke"
py -m kassiber --help >/dev/null
py -m kassiber --machine status >/dev/null
py -m kassiber daemon </dev/null >/dev/null
py -m kassiber backends list >/dev/null
py -m kassiber wallets kinds >/dev/null
py -m kassiber wallets sync-btcpay --help >/dev/null
py -m kassiber profiles create --help >/dev/null
py -m kassiber metadata records --help >/dev/null
py -m kassiber attachments list --help >/dev/null
py -m kassiber journals events --help >/dev/null
py -m kassiber reports balance-history --help >/dev/null
py -m kassiber rates --help >/dev/null
py -m kassiber diagnostics collect --help >/dev/null
py -m kassiber ai --help >/dev/null
py -m kassiber ai providers --help >/dev/null
py -m kassiber ai providers create --help >/dev/null
py -m kassiber ai chat --help >/dev/null

echo
echo "quality gate passed"
