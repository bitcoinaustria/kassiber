#!/usr/bin/env bash
# Build, pack and unpack the Rust binaries the Python parity suite runs.
#
#   scripts/build-rust-test-binaries.sh build          build them into target/
#   scripts/build-rust-test-binaries.sh pack FILE      tar them, with their cargo dep-info
#   scripts/build-rust-test-binaries.sh unpack FILE    restore them into target/
#
# CI builds once and hands the tarball to every Python and shadow lane:
# KASSIBER_RUST_BIN=target/debug/kassiber, its examples beside it in
# target/debug/examples, and the release journal probes the custody lanes
# name (KASSIBER_TEST_CUSTODY_LOADER_BIN, KASSIBER_TEST_JOURNAL_PROCESS_BIN).
# The dep-info files travel along so tests/rust_fixture_binary.py can still
# refuse a stale binary; unpack marks the binaries newer than the checkout
# they were built from, since a fresh checkout's files are newer than any
# earlier build. Only use unpack on a tarball built from this same commit.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# <package> <example> <features>; the binary itself is built first.
EXAMPLES='
kassiber-backup kassiber-backup-probe test-hooks
kassiber-store kassiber-storage-probe test-hooks
kassiber-secrets kassiber-secrets-probe test-hooks,native-keyring
kassiber-chain client_fixture test-harness
kassiber-wallets discovery_fixture test-harness
kassiber-wallets acquisition_fixture test-harness
kassiber-wallets observer_state_fixture test-harness
kassiber-wallets bdk_fixture test-harness
kassiber-wallets lwk_fixture test-harness
kassiber-wallets records_fixture test-harness
kassiber-wallets publication_fixture test-harness
kassiber-wallets sync_fixture test-harness
kassiber-wallets freshness_fixture test-harness
kassiber-wallets freshness_sync_fixture test-harness
kassiber-wallets graph_fixture test-harness
kassiber-wallets document_fixture test-harness
kassiber-wallets exchange_fixture test-harness
'
PROBES='custody-load journal-process'

outputs() {
  printf '%s\n' target/debug/kassiber target/debug/kassiber.d
  printf '%s\n' "$EXAMPLES" | while read -r _package example _features; do
    [ -n "$example" ] || continue
    printf 'target/debug/examples/%s\ntarget/debug/examples/%s.d\n' "$example" "$example"
  done
  for probe in $PROBES; do
    printf 'target/release/examples/%s\ntarget/release/examples/%s.d\n' "$probe" "$probe"
  done
}

case "${1:-}" in
  build)
    cargo build --release --locked -p kassiber-journals --features test-harness \
      $(for probe in $PROBES; do printf -- '--example %s ' "$probe"; done)
    cargo build --locked -p kassiber --features test-harness
    printf '%s\n' "$EXAMPLES" | while read -r package example features; do
      [ -n "$example" ] || continue
      cargo build --locked -p "$package" --example "$example" --features "$features"
    done
    outputs | while read -r path; do
      [ -f "$path" ] || { echo "missing build output: $path" >&2; exit 1; }
    done
    ;;
  pack)
    file=${2:?usage: $0 pack FILE}
    outputs | tar -cf "$file" -T -
    ;;
  unpack)
    file=${2:?usage: $0 unpack FILE}
    tar -xf "$file"
    # Newer than every source and Cargo.lock in this checkout.
    outputs | grep -v '\.d$' | xargs touch
    ;;
  *)
    sed -n '2,16p' "$0" >&2
    exit 2
    ;;
esac
