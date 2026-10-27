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
if ! command -v cargo >/dev/null 2>&1; then
  echo "quality gate requires a Rust toolchain for the tax engine; run ./scripts/bootstrap-dev-env.sh" >&2
  exit 2
fi
# CI builds every crate with the toolchain rust-toolchain.toml pins; formatting
# and lints differ between versions, so the gate refuses any other.
RUST_CHANNEL="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml)"
RUSTC_VERSION="$(rustc --version)"
case "$RUSTC_VERSION" in
  "rustc $RUST_CHANNEL "*) ;;
  *)
    echo "quality gate requires Rust $RUST_CHANNEL (rust-toolchain.toml), found: $RUSTC_VERSION" >&2
    echo "install it with: rustup toolchain install $RUST_CHANNEL --profile minimal --component rustfmt --component clippy" >&2
    exit 2
    ;;
esac
export MATURIN_NO_INSTALL_RUST=1
# Tests read generated data the lock pins (docs/reference/testing.md).
./scripts/fetch-test-data.sh --check

py() {
  uv run --locked python "$@"
}

run cargo fmt --all --check
run cargo clippy --workspace --all-targets --locked -- -D warnings
run cargo test --workspace --locked
run py -m compileall -q kassiber tests scripts/python_test_shards.py
run py scripts/python_test_shards.py validate

# Pytest collects both unittest.TestCase and native pytest modules.  Keep this
# as the single Python pass; CI partitions the same manifest into parallel
# domain jobs while this developer gate favors deterministic serial execution.
# The census mode fails it when a module reaches the tax engine but is missing
# from the tax-shadow overlay, which then reruns against the frozen oracle.
run env KASSIBER_TEST_TAX_ENGINE=census uv run --locked python -m pytest tests -q --durations=50
run py scripts/python_test_shards.py run-shadow -q

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
