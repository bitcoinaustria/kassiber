#!/usr/bin/env bash
# Replay the recorded product sessions against this checkout's binary and
# examples (docs/plan/22-python-removal.md#session-replay). The recordings are
# locked test data (scripts/fetch-test-data.sh); every session must print the
# same output and leave the same state as when it was recorded.
#
#   scripts/replay-sessions.sh [session-replay replay options, e.g. --only TEXT, --part K/N]
#   scripts/replay-sessions.sh build          build the binaries a replay runs
#   scripts/replay-sessions.sh pack FILE      tar them (the examples the recordings name)
#   scripts/replay-sessions.sh unpack FILE    restore them into target/debug
#
# A replay builds first unless KASSIBER_REPLAY_PREBUILT=1 (CI's parts unpack
# the binaries one job built). Replays run in a user and mount namespace at
# the recorded paths, so the host must allow unprivileged user namespaces
# (see ci.yml's replay jobs).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
archives="tests/data/sessions"
[ -f "$archives/sessions.tar.gz" ] || { echo "no recordings at $archives; run scripts/fetch-test-data.sh" >&2; exit 2; }

build() {
cargo build --locked -p kassiber --features test-harness
# Every example a recording names, built as the recorder built them: with
# kassiber-store/test-hooks, so each process takes its recorded SQLite seed.
cargo build --locked --examples \
  -p kassiber -p kassiber-ai -p kassiber-backup -p kassiber-chain -p kassiber-custody \
  -p kassiber-journals -p kassiber-operator -p kassiber-reports -p kassiber-secrets \
  -p kassiber-store -p kassiber-tax-core -p kassiber-transfers -p kassiber-wallets \
  --features kassiber/test-harness,kassiber-ai/test-harness,kassiber-backup/test-hooks,kassiber-chain/test-harness,kassiber-journals/test-harness,kassiber-operator/test-harness,kassiber-reports/test-harness,kassiber-secrets/test-hooks,kassiber-secrets/native-keyring,kassiber-store/test-hooks,kassiber-transfers/test-harness,kassiber-wallets/test-harness
cargo build --locked -p kassiber-session-replay --bins
# Its own command: --bins would apply to the shim too, and its cdylib (the
# clock a replay preloads) would never be built.
cargo build --locked -p kassiber-clock-shim
local target
target="$(cd target && pwd -P)"
for built in kassiber session-replay libkassiber_clock_shim.so; do
  [ -f "$target/debug/$built" ] || { echo "build left no target/debug/$built" >&2; exit 1; }
done
}

case "${1:-}" in
  build)
    build
    exit 0
    ;;
  pack)
    out="${2:?pack FILE}"
    target="$(cd target && pwd -P)"
    names="$(mktemp -d "$target/replay-pack.XXXXXX")"
    tar -xzf "$archives/sessions.tar.gz" -C "$names"
    # Every example a recording runs, plus the binary, the replay tool and the
    # clock shim it preloads.
    files=(debug/kassiber debug/session-replay debug/libkassiber_clock_shim.so)
    while IFS= read -r name; do
      [ "$name" = kassiber ] && continue
      [ -x "$target/debug/examples/$name" ] || { echo "no example $name; run $0 build" >&2; exit 1; }
      files+=("debug/examples/$name")
    done < <("$target/debug/session-replay" list "$names" | cut -f2 | sort -u)
    case "$names" in "$target"/replay-pack.*) rm -rf -- "${names:?}" ;; esac
    tar -C "$target" -cf "$out" "${files[@]}"
    exit 0
    ;;
  unpack)
    mkdir -p target
    tar -C target -xf "${2:?unpack FILE}"
    exit 0
    ;;
esac

[ "${KASSIBER_REPLAY_PREBUILT:-}" = 1 ] || build

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
