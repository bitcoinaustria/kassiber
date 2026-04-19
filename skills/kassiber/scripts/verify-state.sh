#!/usr/bin/env bash
set -euo pipefail

SECTION="all"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --section)
      SECTION="${2:?'--section requires a value: runtime|context|wallets|journals|quarantine|all'}"
      shift 2
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

run_kassiber() {
  if command -v kassiber >/dev/null 2>&1; then
    kassiber "$@"
    return
  fi
  if uv run kassiber status >/dev/null 2>&1; then
    uv run kassiber "$@"
    return
  fi
  if uv run python -m kassiber status >/dev/null 2>&1; then
    uv run python -m kassiber "$@"
    return
  fi
  echo "Unable to find a runnable kassiber command" >&2
  exit 1
}

status_json=$(run_kassiber --machine status)

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for verify-state.sh" >&2
  exit 1
fi

result='{}'
issues='[]'

add_issue() {
  local issue="$1"
  issues=$(jq --arg issue "$issue" '. + [$issue]' <<<"$issues")
}

check_runtime() {
  local version state_root data_root database
  version=$(jq -r '.data.version // ""' <<<"$status_json")
  state_root=$(jq -r '.data.state_root // ""' <<<"$status_json")
  data_root=$(jq -r '.data.data_root // ""' <<<"$status_json")
  database=$(jq -r '.data.database // ""' <<<"$status_json")
  local ok=true
  [[ -n "$version" && -n "$state_root" && -n "$data_root" && -n "$database" ]] || ok=false
  result=$(jq \
    --arg version "$version" \
    --arg state_root "$state_root" \
    --arg data_root "$data_root" \
    --arg database "$database" \
    --argjson ok "$ok" \
    '.runtime = {version: $version, state_root: $state_root, data_root: $data_root, database: $database, ok: $ok}' <<<"$result")
  [[ "$ok" == "true" ]] || add_issue "runtime"
}

check_context() {
  local workspace profile
  workspace=$(jq -r '.data.current_workspace // ""' <<<"$status_json")
  profile=$(jq -r '.data.current_profile // ""' <<<"$status_json")
  local ok=true
  [[ -n "$workspace" && -n "$profile" ]] || ok=false
  result=$(jq \
    --arg workspace "$workspace" \
    --arg profile "$profile" \
    --argjson ok "$ok" \
    '.context = {workspace: $workspace, profile: $profile, ok: $ok}' <<<"$result")
  [[ "$ok" == "true" ]] || add_issue "context"
}

check_wallets() {
  local count
  count=$(jq -r '.data.wallets // 0' <<<"$status_json")
  local ok=true
  [[ "$count" -gt 0 ]] || ok=false
  result=$(jq --argjson count "$count" --argjson ok "$ok" '.wallets = {count: $count, ok: $ok}' <<<"$result")
  [[ "$ok" == "true" ]] || add_issue "wallets"
}

check_journals() {
  local tx_count entry_count
  tx_count=$(jq -r '.data.transactions // 0' <<<"$status_json")
  entry_count=$(jq -r '.data.journal_entries // 0' <<<"$status_json")
  local ok=true
  if [[ "$tx_count" -gt 0 && "$entry_count" -eq 0 ]]; then
    ok=false
  fi
  result=$(jq \
    --argjson transactions "$tx_count" \
    --argjson journal_entries "$entry_count" \
    --argjson ok "$ok" \
    '.journals = {transactions: $transactions, journal_entries: $journal_entries, ok: $ok}' <<<"$result")
  [[ "$ok" == "true" ]] || add_issue "journals"
}

check_quarantine() {
  local count
  count=$(jq -r '.data.quarantines // 0' <<<"$status_json")
  local ok=true
  [[ "$count" -eq 0 ]] || ok=false
  result=$(jq --argjson count "$count" --argjson ok "$ok" '.quarantine = {count: $count, ok: $ok}' <<<"$result")
  [[ "$ok" == "true" ]] || add_issue "quarantine"
}

case "$SECTION" in
  runtime) check_runtime ;;
  context) check_context ;;
  wallets) check_wallets ;;
  journals) check_journals ;;
  quarantine) check_quarantine ;;
  all)
    check_runtime
    check_context
    check_wallets
    check_journals
    check_quarantine
    ;;
  *)
    echo "Unknown section: $SECTION" >&2
    exit 1
    ;;
esac

all_ok=true
[[ "$(jq 'length' <<<"$issues")" -eq 0 ]] || all_ok=false
result=$(jq --argjson all_ok "$all_ok" --argjson issues "$issues" '.summary = {all_ok: $all_ok, issues: $issues}' <<<"$result")
jq . <<<"$result"
