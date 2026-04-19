#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

run() {
  echo
  echo "> $*"
  "$@"
}

PYTHON_BIN="python3"
RUNNER=()
if command -v uv >/dev/null 2>&1; then
  RUNNER=(uv run)
elif [ -x "$ROOT/.venv/bin/python" ]; then
  PYTHON_BIN="$ROOT/.venv/bin/python"
else
  echo "quality gate requires either 'uv' on PATH or a repo-local .venv at $ROOT/.venv" >&2
  exit 2
fi

py() {
  if [ ${#RUNNER[@]} -gt 0 ]; then
    "${RUNNER[@]}" python "$@"
  else
    "$PYTHON_BIN" "$@"
  fi
}

run env PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc bash -lc 'cd "$0" && if command -v uv >/dev/null 2>&1; then uv run python -m py_compile kassiber/*.py kassiber/ui/*.py kassiber/ui/viewmodels/*.py; elif [ -x .venv/bin/python ]; then PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc .venv/bin/python -m py_compile kassiber/*.py kassiber/ui/*.py kassiber/ui/viewmodels/*.py; else PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc python3 -m py_compile kassiber/*.py kassiber/ui/*.py kassiber/ui/viewmodels/*.py; fi' "$ROOT"

run py -m unittest tests.test_cli_smoke -v
run py -m unittest tests.test_review_regressions -v

run bash -lc 'cd "$0" && if command -v uv >/dev/null 2>&1; then uv run python -m kassiber --help >/dev/null; uv run python -m kassiber --machine status >/dev/null; uv run python -m kassiber backends list >/dev/null; uv run python -m kassiber wallets kinds >/dev/null; uv run python -m kassiber profiles create --help >/dev/null; uv run python -m kassiber metadata records --help >/dev/null; uv run python -m kassiber attachments list --help >/dev/null; uv run python -m kassiber journals events --help >/dev/null; uv run python -m kassiber reports balance-history --help >/dev/null; uv run python -m kassiber rates --help >/dev/null; uv run python -m kassiber ui --help >/dev/null; elif [ -x .venv/bin/python ]; then .venv/bin/python -m kassiber --help >/dev/null; .venv/bin/python -m kassiber --machine status >/dev/null; .venv/bin/python -m kassiber backends list >/dev/null; .venv/bin/python -m kassiber wallets kinds >/dev/null; .venv/bin/python -m kassiber profiles create --help >/dev/null; .venv/bin/python -m kassiber metadata records --help >/dev/null; .venv/bin/python -m kassiber attachments list --help >/dev/null; .venv/bin/python -m kassiber journals events --help >/dev/null; .venv/bin/python -m kassiber reports balance-history --help >/dev/null; .venv/bin/python -m kassiber rates --help >/dev/null; .venv/bin/python -m kassiber ui --help >/dev/null; else python3 -m kassiber --help >/dev/null; python3 -m kassiber --machine status >/dev/null; python3 -m kassiber backends list >/dev/null; python3 -m kassiber wallets kinds >/dev/null; python3 -m kassiber profiles create --help >/dev/null; python3 -m kassiber metadata records --help >/dev/null; python3 -m kassiber attachments list --help >/dev/null; python3 -m kassiber journals events --help >/dev/null; python3 -m kassiber reports balance-history --help >/dev/null; python3 -m kassiber rates --help >/dev/null; python3 -m kassiber ui --help >/dev/null; fi' "$ROOT"

echo
echo "quality gate passed"
