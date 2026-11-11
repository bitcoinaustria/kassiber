#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

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

if ! command -v cargo >/dev/null 2>&1; then
  echo "quality gate requires a Rust toolchain; run ./scripts/bootstrap-dev-env.sh" >&2
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
# Tests read generated data the lock pins (docs/reference/testing.md).
./scripts/fetch-test-data.sh --check

run cargo fmt --all --check
run cargo clippy --workspace --all-targets --features kassiber-journals/test-harness --locked -- -D warnings
run cargo test --workspace --features kassiber-journals/test-harness --locked
run cargo run --locked -p kassiber-mutation-check -- check
run scripts/check-ledger-features.sh
run scripts/check-cli-archive.sh
# Frozen custody recordings (docs/reference/testing.md#custody-recordings) are
# local; replay them when this machine has them.
if [ -n "${KASSIBER_CUSTODY_REPLAY_DIR:-}" ]; then
  run cargo run --release --locked -p kassiber-custody --example custody-replay -- "$KASSIBER_CUSTODY_REPLAY_DIR"
fi
run cargo test -p kassiber-net --features test-observer --locked
run cargo clippy -p kassiber -p kassiber-daemon -p kassiber-runtime --all-targets --features kassiber/test-harness --locked -- -D warnings
run cargo test -p kassiber --features test-harness --locked
run cargo build -p kassiber --features test-harness --locked
run scripts/check-no-test-tape.sh --expect-tape target/debug/kassiber
# The parity sessions recorded where Rust and Python agreed
# (docs/reference/testing.md#replaying-the-recorded-sessions).
run scripts/replay-sessions.sh

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
