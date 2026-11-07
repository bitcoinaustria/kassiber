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

py() {
  uv run --locked python "$@"
}

run cargo fmt --all --check
run cargo clippy --workspace --all-targets --features kassiber-journals/test-harness --locked -- -D warnings
run cargo test --workspace --features kassiber-journals/test-harness --locked
run py crates/custody/scripts/mutation_check.py --check
run py crates/secrets/scripts/mutation_check.py --check
run py crates/backup/scripts/mutation_check.py --check
run py crates/daemon/scripts/shell_mutation_check.py --check
run py crates/envelope/scripts/mutation_check.py --check
run py crates/custody/scripts/check_ledger_features.py
run cargo test -p kassiber-net --features test-observer --locked
run cargo clippy -p kassiber -p kassiber-daemon -p kassiber-runtime --all-targets --features kassiber/test-harness --locked -- -D warnings
run cargo test -p kassiber --features test-harness --locked
run cargo build -p kassiber --features test-harness --locked
run cargo build -p kassiber-backup --example kassiber-backup-probe --features test-hooks --offline --locked
run cargo build -p kassiber-store --example kassiber-storage-probe --features test-hooks --offline --locked
run cargo build -p kassiber-secrets --example kassiber-secrets-probe --features test-hooks,native-keyring --offline --locked
run cargo build -p kassiber-chain --example client_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example discovery_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example acquisition_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example observer_state_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example bdk_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example records_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example publication_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example sync_fixture --features test-harness --offline --locked
run cargo build --release --offline --locked -p kassiber-journals --example custody-load --example journal-process --features test-harness
export KASSIBER_TEST_CUSTODY_LOADER_BIN="$ROOT/target/release/examples/custody-load"
export KASSIBER_TEST_JOURNAL_PROCESS_BIN="$ROOT/target/release/examples/journal-process"
run cargo build -p kassiber-wallets --example freshness_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example freshness_sync_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example graph_fixture --features test-harness --offline --locked
run cargo build -p kassiber-wallets --example document_fixture --features test-harness --offline --locked
run py crates/wallets/scripts/discovery_mutation_check.py --check
run py crates/wallets/scripts/acquisition_mutation_check.py --check
run py crates/wallets/scripts/observer_mutation_check.py --check
run py crates/wallets/scripts/bdk_mutation_check.py --check
run py crates/wallets/scripts/record_mutation_check.py --check
run py crates/wallets/scripts/sync_mutation_check.py --check
run py crates/wallets/scripts/routing_mutation_check.py --check
run py crates/wallets/scripts/freshness_mutation_check.py --check
run py crates/wallets/scripts/freshness_mutation_check.py --mutations crates/wallets/scripts/freshness_admission_mutations.json --check
run py crates/wallets/scripts/freshness_mutation_check.py --mutations crates/wallets/scripts/freshness_execution_mutations.json --check
run py crates/wallets/scripts/freshness_mutation_check.py --mutations crates/wallets/scripts/freshness_progress_mutations.json --check
run py crates/wallets/scripts/freshness_mutation_check.py --mutations crates/wallets/scripts/freshness_lifecycle_mutations.json --check
run py crates/wallets/scripts/freshness_mutation_check.py --mutations crates/wallets/scripts/freshness_graph_mutations.json --check
run py crates/wallets/scripts/freshness_mutation_check.py --mutations crates/wallets/scripts/freshness_product_mutations.json --check
run py crates/wallets/scripts/freshness_mutation_check.py --mutations crates/wallets/scripts/freshness_journal_mutations.json --check
export KASSIBER_RUST_BIN="$ROOT/target/debug/kassiber"
run py -m compileall -q kassiber tests scripts/python_test_shards.py
run py scripts/python_test_shards.py validate

# Pytest collects both unittest.TestCase and native pytest modules.  Keep this
# as the single Python pass; CI partitions the same manifest into parallel
# domain jobs while this developer gate favors deterministic serial execution.
# The census mode fails it when a module reaches the tax engine but is missing
# from the tax-shadow overlay, which then reruns against the frozen oracle.
run env KASSIBER_TEST_TAX_ENGINE=census KASSIBER_TEST_CUSTODY_ENGINE=census uv run --locked python -m pytest tests -q --durations=50
run py scripts/python_test_shards.py run-shadow -q
run py scripts/python_test_shards.py run-custody-shadow -q

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
