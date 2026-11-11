#!/usr/bin/env bash
# A product binary carries no test network tape (crates/net/src/tape.rs) and
# no test SQLite seed (kassiber_store::seed_test_prng). Both exist only in
# test builds (kassiber-net's test-observer, kassiber-store's test-hooks);
# their variable names are the markers. Pass the binary; `--expect-tape` inverts the check for
# a test-harness build, so the marker search itself is proven to work.
set -euo pipefail
expect=absent
if [ "${1:-}" = "--expect-tape" ]; then expect=present; shift; fi
binary="${1:?usage: scripts/check-no-test-tape.sh [--expect-tape] BINARY}"
[ -f "$binary" ] || { echo "no binary at $binary" >&2; exit 2; }
# The tape's hold and write-log switches share its prefix; they are listed
# so a rename cannot drop them from the check.
for marker in KASSIBER_TEST_NET_TAPE KASSIBER_TEST_NET_TAPE_HOLD KASSIBER_TEST_NET_TAPE_WRITES KASSIBER_TEST_SQLITE_SEED; do
  if LC_ALL=C grep -aq "$marker" "$binary"; then found=present; else found=absent; fi
  if [ "$found" != "$expect" ]; then
    echo "$binary: test hook $marker is $found, expected $expect" >&2
    exit 1
  fi
done
echo "$binary: test network tape and SQLite seed $expect, as expected"
