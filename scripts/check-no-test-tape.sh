#!/usr/bin/env bash
# A product binary carries no test network tape (crates/net/src/tape.rs). The
# tape exists only with kassiber-net's test-observer feature; its variable
# name is the marker. Pass the binary; `--expect-tape` inverts the check for
# a test-harness build, so the marker search itself is proven to work.
set -euo pipefail
expect=absent
if [ "${1:-}" = "--expect-tape" ]; then expect=present; shift; fi
binary="${1:?usage: scripts/check-no-test-tape.sh [--expect-tape] BINARY}"
[ -f "$binary" ] || { echo "no binary at $binary" >&2; exit 2; }
if LC_ALL=C grep -aq 'KASSIBER_TEST_NET_TAPE' "$binary"; then found=present; else found=absent; fi
if [ "$found" != "$expect" ]; then
  echo "$binary: test network tape is $found, expected $expect" >&2
  exit 1
fi
echo "$binary: test network tape $found, as expected"
