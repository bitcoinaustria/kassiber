#!/usr/bin/env bash
# The packaged-binary smoke: run one packaged Kassiber CLI through init and
# status on plaintext and SQLCipher books, the daemon handshake, the bundled
# Kraken rate import, an Austrian XLSX export, and descriptor derivation.
#
# Release CI runs it against the CLI each matrix leg packages; local packaging
# runs use it on the extracted artifact. Every book lives under --scratch, and
# every machine-wide preference file the binary would otherwise read from the
# default state root is redirected there too, so a smoke never reads or
# changes the account's real Kassiber data.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: scripts/smoke-packaged-cli.sh --cli PATH --scratch DIR
                                     [--expect-commit HEX] [--operator-unlock]

  --cli PATH          The packaged executable (or a launcher forwarding to it).
  --scratch DIR       An empty or missing directory for disposable books.
  --expect-commit HEX `--version` must name this abbreviated commit.
  --operator-unlock   Unlock through the operator broker (macOS and Windows).
                      Linux needs a live logind session for that, so Linux
                      smokes the descriptor-passphrase path instead.
EOF
}

cli=""
scratch=""
expected_commit=""
operator_unlock=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --cli) cli="${2:-}"; shift 2 ;;
    --scratch) scratch="${2:-}"; shift 2 ;;
    --expect-commit) expected_commit="${2:-}"; shift 2 ;;
    --operator-unlock) operator_unlock=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done
[ -n "$cli" ] && [ -n "$scratch" ] || { usage; exit 2; }
[ -x "$cli" ] || { echo "Not an executable: $cli" >&2; exit 2; }
# The packaged binary must not carry test-only code such as the network tape.
"$(dirname "$0")/check-no-test-tape.sh" "$cli"
if [ -e "$scratch" ] && [ -n "$(ls -A "$scratch")" ]; then
  echo "Scratch directory is not empty: $scratch" >&2
  exit 2
fi
mkdir -p "$scratch"
scratch="$(cd "$scratch" && pwd -P)"

# jq ships on GitHub's Linux, macOS and Windows runners and in macOS 15.
command -v jq >/dev/null 2>&1 || { echo "jq is required for JSON assertions" >&2; exit 2; }

# The offline switch, agent access and update preferences are machine-wide
# files under the default state root even with --data-root; XDG_DATA_HOME
# moves that root itself into the scratch directory.
preferences="$scratch/preferences"
mkdir -p "$preferences"
kassiber() {
  env \
    XDG_DATA_HOME="$scratch/xdg-data" \
    KASSIBER_OFFLINE_PREFERENCE_FILE="$preferences/offline-mode.json" \
    KASSIBER_AGENT_ACCESS_FILE="$preferences/agent-access.json" \
    KASSIBER_UPDATE_PREFERENCE_FILE="$preferences/update-check.json" \
    KASSIBER_UPDATE_CACHE_FILE="$preferences/update-cache.json" \
    "$cli" "$@"
}

assert_contains() {
  if ! grep -F "$2" "$1" >/dev/null; then
    echo "Expected $2 in $1; captured output follows:" >&2
    cat "$1" >&2
    return 1
  fi
}
# assert_json_value FILE DOTTED.PATH EXPECTED: the string at PATH equals EXPECTED.
assert_json_value() {
  local actual=""
  actual="$(jq -er --arg path "$2" --arg expected "$3" \
    'getpath($path | split(".")) as $value
     | if $value == $expected then "ok" else ($value | tojson) end' "$1")" || actual=""
  # Windows jq ends lines with CRLF.
  actual="${actual%$'\r'}"
  if [ "$actual" != ok ]; then
    echo "$2: expected \"$3\", got ${actual:-<unreadable JSON>}" >&2
    cat "$1" >&2
    return 1
  fi
}
step() {
  echo "smoke: $*"
}

step "version and help"
kassiber --help >/dev/null
version_text="$(kassiber --version | tr -d '\r')"
echo "$version_text"
if [ -n "$expected_commit" ]; then
  case "$version_text" in
    *"commit $expected_commit"*) ;;
    *) echo "--version does not name commit $expected_commit" >&2; exit 1 ;;
  esac
fi

step "status before and after init (plaintext book)"
status_root="$scratch/status"
status_json="$scratch/status.json"
if kassiber --data-root "$status_root" --machine status > "$status_json"; then
  echo "Fresh status unexpectedly succeeded before kassiber init" >&2
  exit 1
fi
assert_json_value "$status_json" kind error
assert_json_value "$status_json" error.code not_initialized
kassiber --data-root "$status_root" --machine init >/dev/null
kassiber --data-root "$status_root" --machine status > "$status_json"
assert_json_value "$status_json" kind status

step "daemon handshake"
daemon_out="$scratch/daemon.jsonl"
printf '{"request_id":"smoke-1","kind":"daemon.shutdown"}\n' \
  | kassiber --data-root "$scratch/daemon" daemon > "$daemon_out"
assert_contains "$daemon_out" '"kind":"daemon.ready"'
assert_contains "$daemon_out" '"kind":"daemon.shutdown"'

step "bundled Kraken rates"
kraken_out="$scratch/kraken.jsonl"
printf '{"request_id":"kraken-bundled-1","kind":"ui.rates.kraken_csv.import","args":{"use_bundled":true}}\n{"request_id":"shutdown-1","kind":"daemon.shutdown"}\n' \
  | kassiber --data-root "$scratch/kraken" daemon > "$kraken_out"
assert_contains "$kraken_out" '"kind":"ui.rates.kraken_csv.import"'
assert_contains "$kraken_out" '"bundled":true'
assert_contains "$kraken_out" '"pairs":2'
assert_contains "$kraken_out" '"rows":255181'

step "SQLCipher book: encrypt, then open with the passphrase"
operator_root="$scratch/operator"
passphrase='packaged operator smoke passphrase'
kassiber --data-root "$operator_root" --machine init >/dev/null
printf '%s\n' "$passphrase" \
  | kassiber --data-root "$operator_root" --machine secrets init --new-passphrase-fd 0 >/dev/null
# The book on disk must now be ciphertext, not a plaintext SQLite file.
plaintext_books="$(find "$operator_root" -type f -name '*.sqlite3' -exec sh -c \
  'head -c 15 "$1" | grep -q "SQLite format 3" && printf "%s\n" "$1"' _ {} \; )"
if [ -n "$plaintext_books" ]; then
  echo "secrets init left plaintext databases behind:" >&2
  printf '%s\n' "$plaintext_books" >&2
  exit 1
fi
if kassiber --data-root "$operator_root" --machine status </dev/null > "$scratch/locked.json"; then
  echo "An encrypted book opened without its passphrase" >&2
  exit 1
fi
worker_json="$scratch/operator-worker.json"
if [ "$operator_unlock" -eq 1 ]; then
  if ! printf '%s\n' "$passphrase" \
      | kassiber --data-root "$operator_root" --machine operator unlock \
          --passphrase-fd 0 --duration 1m > "$scratch/operator-unlock.json"; then
    cat "$scratch/operator-unlock.json" >&2
    exit 1
  fi
  assert_json_value "$scratch/operator-unlock.json" data.lease unlocked
  kassiber --data-root "$operator_root" --machine status > "$worker_json"
  kassiber --data-root "$operator_root" --machine operator lock >/dev/null
else
  # Hosted Linux runners have no live logind user session. Production
  # Kassiber correctly refuses to retain an in-memory unlock there, so smoke
  # the supported non-broker path. The Linux broker lifecycle is exercised
  # separately by test_operator_integration.py with an owner-only logout
  # sentinel.
  printf '%s\n' "$passphrase" \
    | kassiber --data-root "$operator_root" --machine --db-passphrase-fd 0 status \
    > "$worker_json"
fi
assert_json_value "$worker_json" kind status

step "Austrian moving-average journal and E1kv XLSX"
at_root="$scratch/at"
kassiber --data-root "$at_root" --machine init >/dev/null
kassiber --data-root "$at_root" --machine workspaces create Smoke >/dev/null
kassiber --data-root "$at_root" --machine profiles create --workspace Smoke \
  --tax-country at --gains-algorithm MOVING_AVERAGE_AT Austrian >/dev/null
kassiber --data-root "$at_root" --machine wallets create --workspace Smoke \
  --profile Austrian --label AustrianSmoke --kind address \
  --address bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080 >/dev/null
at_csv="$scratch/at.csv"
cat > "$at_csv" <<'CSV'
date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,packaged-at-buy,inbound,BTC,0.10000000,0,80000,Packaged smoke acquisition
2026-02-01T10:00:00Z,packaged-at-sell,outbound,BTC,0.05000000,0.0001,82000,Packaged smoke disposal
CSV
kassiber --data-root "$at_root" --machine wallets import-csv --workspace Smoke \
  --profile Austrian --wallet AustrianSmoke --file "$at_csv" >/dev/null
kassiber --data-root "$at_root" --machine journals process --workspace Smoke \
  --profile Austrian >/dev/null
xlsx_file="$scratch/at.xlsx"
kassiber --data-root "$at_root" --machine reports export-austrian-e1kv-xlsx \
  --workspace Smoke --profile Austrian --year 2026 --file "$xlsx_file" >/dev/null
test -s "$xlsx_file"

step "descriptor wallet derivation"
descriptor_root="$scratch/descriptor"
kassiber --data-root "$descriptor_root" --machine init >/dev/null
kassiber --data-root "$descriptor_root" --machine workspaces create Smoke >/dev/null
kassiber --data-root "$descriptor_root" --machine profiles create --workspace Smoke \
  --fiat-currency USD --tax-country generic Descriptor >/dev/null
kassiber --data-root "$descriptor_root" --machine wallets create --workspace Smoke \
  --profile Descriptor --label DescriptorSmoke --kind descriptor \
  --descriptor 'wpkh([91790c21/84h/0h/0h]xpub6CpT4xRJntYQuFwkiZfSiK7jEtPM3e9nUfJKQ8ATWkgYJui4WS5HmzjD6C7avtwAFtmEyAusdFFxg2rZMs73SoSVkPQfEaQtrLGox92JNWn/0/*)' \
  --gap-limit 1 >/dev/null
kassiber --data-root "$descriptor_root" --machine wallets derive --workspace Smoke \
  --profile Descriptor --wallet DescriptorSmoke --count 1 >/dev/null

if [ -n "$(ls -A "$scratch/xdg-data" 2>/dev/null)" ]; then
  echo "note: the binary wrote under the default state root: $(ls -A "$scratch/xdg-data")"
fi
echo "Packaged CLI smoke passed: $version_text"
