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

if ! command -v uv >/dev/null 2>&1; then
  echo "quality gate requires uv; run ./scripts/bootstrap-dev-env.sh" >&2
  exit 2
fi

py() {
  uv run --frozen python "$@"
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
