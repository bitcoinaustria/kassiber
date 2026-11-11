#!/usr/bin/env bash
# Replay the recorded product sessions against this checkout's binary and
# examples (docs/plan/22-python-removal.md#session-replay). The recordings are
# locked test data (scripts/fetch-test-data.sh); every session must print the
# same output and leave the same state as when it was recorded.
#
#   scripts/replay-sessions.sh [session-replay replay options, e.g. --only TEXT]
#
# Replays run in a user and mount namespace at the recorded paths, so the
# host must allow unprivileged user namespaces (see ci.yml's replay job).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
archives="tests/data/sessions"
[ -f "$archives/sessions.tar.gz" ] || { echo "no recordings at $archives; run scripts/fetch-test-data.sh" >&2; exit 2; }

cargo build --locked -p kassiber --features test-harness
# Every example a recording names, built as the recorder built them: with
# kassiber-store/test-hooks, so each process takes its recorded SQLite seed.
cargo build --locked --examples \
  -p kassiber -p kassiber-ai -p kassiber-backup -p kassiber-chain -p kassiber-custody \
  -p kassiber-journals -p kassiber-operator -p kassiber-reports -p kassiber-secrets \
  -p kassiber-store -p kassiber-tax-core -p kassiber-transfers -p kassiber-wallets \
  --features kassiber/test-harness,kassiber-ai/test-harness,kassiber-backup/test-hooks,kassiber-chain/test-harness,kassiber-journals/test-harness,kassiber-operator/test-harness,kassiber-reports/test-harness,kassiber-secrets/test-hooks,kassiber-secrets/native-keyring,kassiber-store/test-hooks,kassiber-transfers/test-harness,kassiber-wallets/test-harness
cargo build --locked -p kassiber-session-replay --bins -p kassiber-clock-shim

target="$(cd target && pwd -P)"
data="$target/session-replay-data"
case "$data" in */session-replay-data) rm -rf -- "$data" ;; esac
mkdir -p "$data"
"$target/debug/session-replay" extract "$archives" "$data"
report="$target/session-replay-report.tsv"
jobs="${KASSIBER_REPLAY_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"
env -u XDG_DATA_HOME "$target/debug/session-replay" replay "$data" \
  --bin "kassiber=$target/debug/kassiber" --examples "$target/debug/examples" \
  --jobs "$jobs" --report "$report" "$@"
