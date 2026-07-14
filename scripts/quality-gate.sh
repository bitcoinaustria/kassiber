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

run_in_dir() {
  local dir="$1"
  shift
  echo
  echo "> (cd $dir && $*)"
  (cd "$dir" && "$@")
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

run py -m compileall -q kassiber tests scripts/python_test_shards.py
run py scripts/python_test_shards.py validate

# Pytest collects both unittest.TestCase and native pytest modules.  Keep this
# as the single Python pass; CI partitions the same manifest into parallel
# domain jobs while this developer gate favors deterministic serial execution.
run py -m pytest tests -q --durations=50

echo
if [ ! -d "$ROOT/ui-tauri/node_modules" ]; then
  echo "quality gate requires ui-tauri/node_modules for Vitest; run: pnpm --dir ui-tauri install --frozen-lockfile" >&2
  exit 2
fi
for tool in eslint tsc vitest; do
  if [ ! -x "$ROOT/ui-tauri/node_modules/.bin/$tool" ]; then
    echo "quality gate requires ui-tauri/node_modules/.bin/$tool; run: pnpm --dir ui-tauri install --frozen-lockfile" >&2
    exit 2
  fi
done
run_in_dir ui-tauri ./node_modules/.bin/tsc -b --noEmit
run_in_dir ui-tauri ./node_modules/.bin/eslint .
run_in_dir ui-tauri ./node_modules/.bin/vitest run

echo
echo "quality gate passed"
