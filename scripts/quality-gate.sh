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

run py -m py_compile kassiber/*.py kassiber/secrets/*.py kassiber/backup/*.py

run py -m unittest tests.test_cli_smoke -v
run py -m unittest tests.test_daemon_smoke -v
run py -m unittest tests.test_secrets_smoke -v
run py -m unittest tests.test_review_regressions -v

echo
SMOKE_HOME="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-quality-gate-home.XXXXXX")"
trap 'rm -rf "$SMOKE_HOME"' EXIT
smoke_py() {
  HOME="$SMOKE_HOME" py "$@"
}

echo "> CLI help smoke (isolated HOME: $SMOKE_HOME)"
smoke_py -m kassiber --help >/dev/null
smoke_py -m kassiber --machine status >/dev/null
smoke_py -m kassiber daemon </dev/null >/dev/null
smoke_py -m kassiber backends list >/dev/null
smoke_py -m kassiber wallets kinds >/dev/null
smoke_py -m kassiber wallets sync-btcpay --help >/dev/null
smoke_py -m kassiber profiles create --help >/dev/null
smoke_py -m kassiber metadata records --help >/dev/null
smoke_py -m kassiber attachments list --help >/dev/null
smoke_py -m kassiber journals events --help >/dev/null
smoke_py -m kassiber reports balance-history --help >/dev/null
smoke_py -m kassiber rates --help >/dev/null
smoke_py -m kassiber diagnostics collect --help >/dev/null
smoke_py -m kassiber ai --help >/dev/null
smoke_py -m kassiber ai providers --help >/dev/null
smoke_py -m kassiber ai providers create --help >/dev/null
smoke_py -m kassiber ai chat --help >/dev/null
smoke_py -m kassiber secrets --help >/dev/null
smoke_py -m kassiber secrets init --help >/dev/null
smoke_py -m kassiber secrets change-passphrase --help >/dev/null
smoke_py -m kassiber secrets verify --help >/dev/null
smoke_py -m kassiber secrets status --help >/dev/null
smoke_py -m kassiber secrets migrate-credentials --help >/dev/null
smoke_py -m kassiber backup --help >/dev/null
smoke_py -m kassiber backup export --help >/dev/null
smoke_py -m kassiber backup import --help >/dev/null
smoke_py -m kassiber backends reveal-token --help >/dev/null
smoke_py -m kassiber wallets reveal-descriptor --help >/dev/null

echo
echo "quality gate passed"
