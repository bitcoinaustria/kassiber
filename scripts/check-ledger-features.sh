#!/usr/bin/env bash
# Prove custody's unified serde_json features preserve every ledger output
# byte: the ledger contract example prints the same bytes with and without
# the features kassiber-custody turns on for the whole build.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
run() {
  cargo run -p kassiber-tax-core --example ledger_feature_contract --locked --quiet "$@"
}
baseline=$(run | sha256sum)
count=$(run | wc -l)
unified=$(run --features serde_json/arbitrary_precision,serde_json/float_roundtrip | sha256sum)
if [ "$baseline" != "$unified" ]; then
  echo "serde_json features change ledger bytes" >&2
  exit 1
fi
if [ "$count" -lt 100 ]; then
  echo "the ledger contract printed only $count outputs" >&2
  exit 1
fi
echo "ledger: $count outputs byte-identical with and without custody serde_json features"
