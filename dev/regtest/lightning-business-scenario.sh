#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lightning-common.sh"

PLAN_FILE="${KASSIBER_LIGHTNING_BUSINESS_PLAN:-${KASSIBER_LIGHTNING_BUSINESS_HOME:-${TMPDIR:-/tmp}/kassiber-lightning-business}/business-plan.json}"
STATE_FILE="${KASSIBER_LIGHTNING_BUSINESS_STATE:-${PLAN_FILE%.json}.state.json}"
PLAN_ROWS_FILE=""

ensure_plan() {
  python3 "$ROOT/dev/regtest/lightning-business-plan.py" --output "$PLAN_FILE"
  echo "Lightning business plan: $PLAN_FILE"
}

plan_rows() {
  local path="$1"
  shift
  python3 - "$PLAN_FILE" "$path" "$@" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    data = json.load(handle)
for part in sys.argv[2].split("."):
    data = data[part]
keys = sys.argv[3:]
for item in data:
    if isinstance(item, dict):
        print("\t".join(str(item.get(key, "")) for key in keys))
    else:
        print("\t".join(str(item if key == "." else "") for key in keys))
PY
}

plan_value() {
  local path="$1"
  python3 - "$PLAN_FILE" "$path" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    data = json.load(handle)
for part in sys.argv[2].split("."):
    data = data[part]
print(data)
PY
}

plan_hash() {
  plan_value traffic_model.plan_hash
}

load_plan_rows() {
  local path="$1"
  shift
  if [ -n "$PLAN_ROWS_FILE" ] && [ -f "$PLAN_ROWS_FILE" ]; then
    rm -f "$PLAN_ROWS_FILE"
  fi
  PLAN_ROWS_FILE="$(mktemp "${TMPDIR:-/tmp}/kassiber-ln-plan-rows.XXXXXX")"
  plan_rows "$path" "$@" >"$PLAN_ROWS_FILE"
}

cleanup_plan_rows() {
  if [ -n "$PLAN_ROWS_FILE" ] && [ -f "$PLAN_ROWS_FILE" ]; then
    rm -f "$PLAN_ROWS_FILE"
  fi
  PLAN_ROWS_FILE=""
}

state_json_get() {
  local key="$1"
  local fallback="${2:-}"
  if [ ! -f "$STATE_FILE" ]; then
    printf '%s\n' "$fallback"
    return 0
  fi
  python3 - "$STATE_FILE" "$key" "$fallback" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], "r", encoding="utf-8") as handle:
        data = json.load(handle)
except (OSError, ValueError):
    data = {}
value = data
for part in sys.argv[2].split("."):
    if isinstance(value, dict):
        value = value.get(part)
    else:
        value = None
        break
print(sys.argv[3] if value is None else value)
PY
}

ensure_state_for_plan() {
  local expected_hash existing_hash completed_count
  expected_hash="$(plan_hash)"
  mkdir -p "$(dirname "$STATE_FILE")"
  if [ ! -f "$STATE_FILE" ]; then
    python3 - "$STATE_FILE" "$expected_hash" <<'PY'
import json
import os
import sys

path, plan_hash = sys.argv[1:3]
tmp = f"{path}.tmp"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump({"plan_hash": plan_hash, "completed": {}}, handle, indent=2, sort_keys=True)
    handle.write("\n")
os.replace(tmp, path)
PY
    return 0
  fi
  existing_hash="$(state_json_get plan_hash)"
  completed_count="$(python3 - "$STATE_FILE" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], "r", encoding="utf-8") as handle:
        data = json.load(handle)
except (OSError, ValueError):
    data = {}
print(len(data.get("completed") or {}))
PY
)"
  if [ -z "$existing_hash" ] && [ "$completed_count" -eq 0 ]; then
    python3 - "$STATE_FILE" "$expected_hash" <<'PY'
import json
import os
import sys

path, plan_hash = sys.argv[1:3]
try:
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
except (OSError, ValueError):
    data = {}
data["plan_hash"] = plan_hash
data.setdefault("completed", {})
tmp = f"{path}.tmp"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump(data, handle, indent=2, sort_keys=True)
    handle.write("\n")
os.replace(tmp, path)
PY
    return 0
  fi
  if [ "$existing_hash" != "$expected_hash" ]; then
    echo "Lightning business plan changed while reuse state exists." >&2
    echo "State: $STATE_FILE" >&2
    echo "Clear KASSIBER_REGTEST_LIGHTNING_REUSE/KASSIBER_REGTEST_KEEP or remove the state/volumes before changing the seed or traffic knobs." >&2
    return 1
  fi
}

state_done() {
  local label="$1"
  [ -f "$STATE_FILE" ] || return 1
  python3 - "$STATE_FILE" "$label" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    state = json.load(handle)
entry = state.get("completed", {}).get(sys.argv[2])
if not isinstance(entry, dict):
    sys.exit(1)
sys.exit(0 if entry.get("status") == "confirmed" else 1)
PY
}

state_pending() {
  local label="$1"
  [ -f "$STATE_FILE" ] || return 1
  python3 - "$STATE_FILE" "$label" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    state = json.load(handle)
entry = state.get("completed", {}).get(sys.argv[2])
if not isinstance(entry, dict):
    sys.exit(1)
print(entry.get("txid") or "")
sys.exit(0 if entry.get("status") == "pending" and entry.get("txid") else 1)
PY
}

state_mark() {
  local label="$1"
  local status="$2"
  local txid="${3:-}"
  local wallet="${4:-}"
  mkdir -p "$(dirname "$STATE_FILE")"
  python3 - "$STATE_FILE" "$label" "$status" "$txid" "$wallet" "$(plan_hash)" <<'PY'
import json
import os
import sys

path, label, status, txid, wallet, plan_hash = sys.argv[1:7]
try:
    with open(path, "r", encoding="utf-8") as handle:
        state = json.load(handle)
except (OSError, ValueError):
    state = {}
state.setdefault("plan_hash", plan_hash)
completed = state.setdefault("completed", {})
completed[label] = {"status": status, "txid": txid, "wallet": wallet}
tmp = f"{path}.tmp"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump(state, handle, indent=2, sort_keys=True)
    handle.write("\n")
os.replace(tmp, path)
PY
}

state_mark_done() {
  state_mark "$1" confirmed "${2:-}" "${3:-}"
}

state_mark_pending() {
  state_mark "$1" pending "${2:-}" "${3:-}"
}

confirm_pending_state() {
  local label="$1"
  local wallet="$2"
  local confirmations="${3:-1}"
  local txid current
  txid="$(state_pending "$label" 2>/dev/null || true)"
  if [ -z "$txid" ]; then
    return 1
  fi
  ensure_core_wallet "$wallet"
  local tx_json
  if tx_json="$(btc -rpcwallet="$wallet" gettransaction "$txid" 2>/dev/null)"; then
    current="$(printf '%s\n' "$tx_json" | python3 -c 'import json, sys
data = json.load(sys.stdin)
print(int(data.get("confirmations") or 0))')"
  else
    current=0
  fi
  if [ "$current" -lt "$confirmations" ]; then
    mine_to_faucet "$((confirmations - current))"
  fi
  state_mark_done "$label" "$txid" "$wallet"
  echo "$label confirmed from pending state."
  return 0
}

invoice_status_once() {
  local service="$1"
  local label="$2"
  cln "$service" listinvoices "$label" | python3 -c 'import json, sys
try:
    data = json.load(sys.stdin)
except ValueError:
    sys.exit(2)
invoices = data.get("invoices") or []
print((invoices[0].get("status") if invoices else "") or "")'
}

invoice_status() {
  local service="$1"
  local label="$2"
  local attempt status
  for attempt in $(seq 1 20); do
    if status="$(invoice_status_once "$service" "$label" 2>/dev/null)"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done
  invoice_status_once "$service" "$label"
}

invoice_bolt11_once() {
  local service="$1"
  local label="$2"
  cln "$service" listinvoices "$label" | python3 -c 'import json, sys
try:
    data = json.load(sys.stdin)
except ValueError:
    sys.exit(2)
invoices = data.get("invoices") or []
print((invoices[0].get("bolt11") if invoices else "") or "")'
}

invoice_bolt11() {
  local service="$1"
  local label="$2"
  local attempt bolt11
  for attempt in $(seq 1 20); do
    if bolt11="$(invoice_bolt11_once "$service" "$label" 2>/dev/null)" && [ -n "$bolt11" ]; then
      printf '%s\n' "$bolt11"
      return 0
    fi
    sleep 1
  done
  invoice_bolt11_once "$service" "$label"
}

invoice_payment_hash_once() {
  local service="$1"
  local label="$2"
  cln "$service" listinvoices "$label" | python3 -c 'import json, sys
try:
    data = json.load(sys.stdin)
except ValueError:
    sys.exit(2)
invoices = data.get("invoices") or []
print((invoices[0].get("payment_hash") if invoices else "") or "")'
}

invoice_payment_hash() {
  local service="$1"
  local label="$2"
  local attempt payment_hash
  for attempt in $(seq 1 20); do
    if payment_hash="$(invoice_payment_hash_once "$service" "$label" 2>/dev/null)" && [ -n "$payment_hash" ]; then
      printf '%s\n' "$payment_hash"
      return 0
    fi
    sleep 1
  done
  invoice_payment_hash_once "$service" "$label"
}

invoice_matches_plan_once() {
  local service="$1"
  local label="$2"
  local expected_amount_msat="$3"
  local expected_description="$4"
  cln "$service" listinvoices "$label" | python3 -c '
import json
import sys

expected_amount = int(sys.argv[1])
expected_description = sys.argv[2]

def parse_msat(value):
    if value in (None, ""):
        return 0
    if isinstance(value, int):
        return value
    if isinstance(value, dict):
        for key in ("msat", "millisatoshis", "millisatoshi"):
            if key in value:
                return parse_msat(value[key])
        return 0
    text = str(value).strip().lower()
    if text.endswith("msat"):
        return int(float(text[:-4] or "0"))
    if text.endswith("sat"):
        return int(float(text[:-3] or "0") * 1000)
    return int(float(text or "0"))

try:
    data = json.load(sys.stdin)
except ValueError:
    sys.exit(2)
invoices = data.get("invoices") or []
if not invoices:
    sys.exit(2)
invoice = invoices[0]
actual_amount = parse_msat(invoice.get("amount_msat") or invoice.get("amount_received_msat"))
actual_description = str(invoice.get("description") or "")
sys.exit(0 if actual_amount == expected_amount and actual_description == expected_description else 1)
' "$expected_amount_msat" "$expected_description"
}

invoice_matches_plan() {
  local service="$1"
  local label="$2"
  local expected_amount_msat="$3"
  local expected_description="$4"
  local attempt
  for attempt in $(seq 1 20); do
    if invoice_matches_plan_once "$service" "$label" "$expected_amount_msat" "$expected_description" 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  invoice_matches_plan_once "$service" "$label" "$expected_amount_msat" "$expected_description"
}

payment_status_by_hash() {
  local service="$1"
  local payment_hash="$2"
  if [ -z "$payment_hash" ]; then
    return 1
  fi
  cln "$service" listpays | python3 -c 'import json, sys
payment_hash = sys.argv[1]
data = json.load(sys.stdin)
for pay in data.get("pays", []):
    if pay.get("payment_hash") == payment_hash:
        print(pay.get("status") or "")
        sys.exit(0)
sys.exit(1)' "$payment_hash"
}

payment_terminal_status_by_hash() {
  local service="$1"
  local payment_hash="$2"
  local attempt status
  for attempt in $(seq 1 30); do
    status="$(payment_status_by_hash "$service" "$payment_hash" 2>/dev/null || true)"
    case "$status" in
      failed|complete|completed|paid)
        printf '%s\n' "$status"
        return 0
        ;;
    esac
    sleep 1
  done
  printf '%s\n' "$status"
}

ensure_invoice() {
  local service="$1"
  local amount_msat="$2"
  local label="$3"
  local description="$4"
  local expiry="${5:-3600}"
  local status
  status="$(invoice_status "$service" "$label")"
  if [ "$status" = "paid" ]; then
    if ! invoice_matches_plan "$service" "$label" "$amount_msat" "$description"; then
      echo "$service invoice $label was already paid with a different amount or description." >&2
      return 1
    fi
    return 0
  fi
  if [ "$status" = "expired" ]; then
    cln "$service" delinvoice "$label" expired >/dev/null || true
    status=""
  fi
  if [ -n "$status" ] && ! invoice_matches_plan "$service" "$label" "$amount_msat" "$description"; then
    echo "$service invoice $label exists with a different amount or description." >&2
    return 1
  fi
  if [ -z "$status" ]; then
    cln "$service" -k invoice \
      amount_msat="${amount_msat}msat" \
      label="$label" \
      description="$description" \
      expiry="$expiry" >/dev/null
  fi
}

pay_bolt11() {
  local payer="$1"
  local bolt11="$2"
  local label="$3"
  local deadline=$((SECONDS + 90))
  while true; do
    if cln "$payer" -k pay bolt11="$bolt11" maxfeepercent=5 exemptfee=5000 >/dev/null 2>&1; then
      echo "$payer paid $label."
      return 0
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "$payer could not pay $label before timeout." >&2
      cln "$payer" -k pay bolt11="$bolt11" maxfeepercent=5 exemptfee=5000 || true
      return 1
    fi
    sleep 3
  done
}

ensure_paid_invoice() {
  local issuer="$1"
  local payer="$2"
  local amount_msat="$3"
  local label="$4"
  local description="$5"
  ensure_invoice "$issuer" "$amount_msat" "$label" "$description" 3600
  if [ "$(invoice_status "$issuer" "$label")" = "paid" ]; then
    echo "$label already paid."
    return 0
  fi
  pay_bolt11 "$payer" "$(invoice_bolt11 "$issuer" "$label")" "$label"
}

lnd_create_invoice_bolt11() {
  local amount_msat="$1"
  local description="$2"
  local expiry="$3"
  lnd addinvoice \
    --amt_msat="$amount_msat" \
    --memo="$description" \
    --expiry="$expiry" | python3 -c 'import json, sys
data = json.load(sys.stdin)
print(data.get("payment_request") or "")'
}

lnd_pay_bolt11() {
  local bolt11="$1"
  local label="$2"
  local deadline=$((SECONDS + 90))
  while true; do
    if lnd payinvoice --force --pay_req="$bolt11" >/dev/null 2>&1; then
      echo "lnd_merchant_backup paid $label."
      return 0
    fi
    if lnd payinvoice --force "$bolt11" >/dev/null 2>&1; then
      echo "lnd_merchant_backup paid $label."
      return 0
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "lnd_merchant_backup could not pay $label before timeout." >&2
      lnd payinvoice --force --pay_req="$bolt11" || true
      return 1
    fi
    sleep 3
  done
}

ensure_lnd_invoice_paid_by_cln() {
  local payer="$1"
  local amount_msat="$2"
  local label="$3"
  local description="$4"
  local expiry="${5:-3600}"
  local bolt11
  if state_done "$label"; then
    echo "$label already paid."
    return 0
  fi
  bolt11="$(lnd_create_invoice_bolt11 "$amount_msat" "$description" "$expiry")"
  if [ -z "$bolt11" ]; then
    echo "Could not create LND invoice for $label." >&2
    return 1
  fi
  pay_bolt11 "$payer" "$bolt11" "$label"
  state_mark_done "$label"
}

ensure_lnd_paid_cln_invoice() {
  local issuer="$1"
  local amount_msat="$2"
  local label="$3"
  local description="$4"
  local expiry="${5:-3600}"
  ensure_invoice "$issuer" "$amount_msat" "$label" "$description" "$expiry"
  if state_done "$label"; then
    echo "$label already paid."
    return 0
  fi
  if [ "$(invoice_status "$issuer" "$label")" = "paid" ]; then
    state_mark_done "$label"
    echo "$label already paid."
    return 0
  fi
  lnd_pay_bolt11 "$(invoice_bolt11 "$issuer" "$label")" "$label"
  state_mark_done "$label"
}

ensure_expired_merchant_quote() {
  local label="$1"
  local amount_msat="$2"
  local description="$3"
  local expiry="${4:-1}"
  local status
  status="$(invoice_status cln_merchant "$label")"
  if [ "$status" = "paid" ]; then
    echo "$label was paid but should be expired/unpaid." >&2
    return 1
  fi
  if [ "$status" = "expired" ]; then
    if ! invoice_matches_plan cln_merchant "$label" "$amount_msat" "$description"; then
      echo "$label is expired but does not match the current business plan." >&2
      return 1
    fi
    echo "$label already $status."
    return 0
  fi
  if [ -z "$status" ]; then
    ensure_invoice cln_merchant "$amount_msat" "$label" "$description" "$expiry"
  fi
  sleep "$((expiry + 1))"
  if cln cln_customer -k pay bolt11="$(invoice_bolt11 cln_merchant "$label")" maxfeepercent=5 exemptfee=5000 >/dev/null 2>&1; then
    echo "expected $label to be expired, but payment succeeded." >&2
    return 1
  fi
  if [ "$(invoice_status cln_merchant "$label")" != "expired" ]; then
    echo "$label did not reach expired status." >&2
    return 1
  fi
  echo "$label intentionally left expired/unpaid."
}

ensure_failed_payment() {
  local issuer="$1"
  local payer="$2"
  local amount_msat="$3"
  local label="$4"
  local description="$5"
  local expiry="${6:-3600}"
  local payment_hash status

  ensure_invoice "$issuer" "$amount_msat" "$label" "$description" "$expiry"
  payment_hash="$(invoice_payment_hash "$issuer" "$label")"
  status="$(payment_status_by_hash "$payer" "$payment_hash" 2>/dev/null || true)"
  if [ "$status" = "failed" ]; then
    echo "$label already failed as expected."
    return 0
  fi
  if [ "$status" = "complete" ] || [ "$status" = "completed" ] || [ "$status" = "paid" ]; then
    echo "expected $label to fail, but it is already $status." >&2
    return 1
  fi
  if cln "$payer" -k pay \
    bolt11="$(invoice_bolt11 "$issuer" "$label")" \
    retry_for=2 \
    maxfeepercent=5 \
    exemptfee=5000 >/dev/null 2>&1; then
    echo "expected $label to fail, but payment succeeded." >&2
    return 1
  else
    echo "$label intentionally failed due to liquidity limits."
  fi
  status="$(payment_terminal_status_by_hash "$payer" "$payment_hash")"
  if [ "$status" != "failed" ]; then
    echo "$label did not reach failed payment status (status=$status)." >&2
    return 1
  fi
}

ensure_actor_wallet_funds() {
  local wallet="$1"
  local needed_sat="$2"
  local balance_sat address topup_sat
  ensure_core_wallet "$wallet"
  balance_sat="$(wallet_balance_sat "$wallet")"
  if [ "$balance_sat" -ge "$needed_sat" ]; then
    return 1
  fi
  topup_sat=$((needed_sat - balance_sat + 1000000))
  address="$(btc -rpcwallet="$wallet" getnewaddress "kassiber lightning actor funding" bech32)"
  btc -rpcwallet="$FAUCET_WALLET" sendtoaddress \
    "$address" \
    "$(sat_to_btc "$topup_sat")" \
    "fund $wallet" \
    "fund $wallet for lightning business scenario" >/dev/null
  echo "Funded mainchain actor wallet $wallet with $topup_sat sat."
  return 0
}

run_mainchain_topups() {
  local funded=0
  local buffer_sat funding_confirmations
  buffer_sat="$(plan_value mainchain.actor_funding_buffer_sat)"
  funding_confirmations="$(plan_value mainchain.actor_funding_confirmations)"
  ensure_faucet_funds

  load_plan_rows mainchain.topups wallet label amount_sat description confirmations
  while IFS=$'\t' read -r wallet label amount_sat _description _confirmations; do
    [ -n "$wallet" ] || continue
    if state_done "$label"; then
      ensure_core_wallet "$wallet"
      continue
    fi
    if confirm_pending_state "$label" "$wallet" "${_confirmations:-1}"; then
      continue
    fi
    if ensure_actor_wallet_funds "$wallet" "$((amount_sat + buffer_sat))"; then
      funded=1
    fi
  done < "$PLAN_ROWS_FILE"

  if [ "$funded" -eq 1 ]; then
    mine_to_faucet "$funding_confirmations"
  fi

  load_plan_rows mainchain.topups wallet label amount_sat description confirmations
  while IFS=$'\t' read -r wallet label amount_sat description confirmations; do
    [ -n "$wallet" ] || continue
    if state_done "$label"; then
      echo "$label already broadcast."
      continue
    fi
    if confirm_pending_state "$label" "$wallet" "${confirmations:-1}"; then
      continue
    fi
    local address txid
    address="$(cln_new_address cln_merchant)"
    txid="$(btc -rpcwallet="$wallet" sendtoaddress \
      "$address" \
      "$(sat_to_btc "$amount_sat")" \
      "$label" \
      "$description")"
    state_mark_pending "$label" "$txid" "$wallet"
    mine_to_faucet "${confirmations:-1}"
    state_mark_done "$label" "$txid" "$wallet"
    echo "Broadcast $label ($amount_sat sat) to merchant CLN wallet."
  done < "$PLAN_ROWS_FILE"
}

run_mainchain_withdrawals() {
  load_plan_rows mainchain.withdrawals wallet label amount_sat description confirmations
  while IFS=$'\t' read -r wallet label amount_sat description confirmations; do
    [ -n "$wallet" ] || continue
    ensure_core_wallet "$wallet"
    if state_done "$label"; then
      echo "$label already withdrawn."
      continue
    fi
    if confirm_pending_state "$label" "$wallet" "${confirmations:-1}"; then
      continue
    fi
    local address result txid
    address="$(btc -rpcwallet="$wallet" getnewaddress "$label" bech32)"
    result="$(cln cln_merchant withdraw "$address" "${amount_sat}sat")"
    txid="$(printf '%s\n' "$result" | python3 -c 'import json, sys
data = json.load(sys.stdin)
print(data.get("txid") or "")')"
    state_mark_pending "$label" "$txid" "$wallet"
    mine_to_faucet "${confirmations:-1}"
    state_mark_done "$label" "$txid" "$wallet"
    echo "Broadcast $label ($amount_sat sat) from merchant CLN wallet."
  done < "$PLAN_ROWS_FILE"
}

run_lightning_activity() {
  load_plan_rows lightning.merchant_invoices label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_merchant cln_customer "$amount_msat" "$label" "$description"
  done < "$PLAN_ROWS_FILE"

  load_plan_rows lightning.supplier_invoices label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_supplier cln_merchant "$amount_msat" "$label" "$description"
  done < "$PLAN_ROWS_FILE"

  load_plan_rows lightning.routed_customer_supplier label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_supplier cln_customer "$amount_msat" "$label" "$description"
  done < "$PLAN_ROWS_FILE"

  load_plan_rows lightning.routed_router_customer label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_customer cln_router "$amount_msat" "$label" "$description"
  done < "$PLAN_ROWS_FILE"

  load_plan_rows lightning.expired_invoices label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_expired_merchant_quote "$label" "$amount_msat" "$description" "$expiry"
  done < "$PLAN_ROWS_FILE"

  load_plan_rows lightning.failed_payments label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_failed_payment cln_supplier cln_merchant "$amount_msat" "$label" "$description" "$expiry"
  done < "$PLAN_ROWS_FILE"
}

run_lnd_activity() {
  load_plan_rows lightning.lnd_invoices label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_lnd_invoice_paid_by_cln cln_merchant "$amount_msat" "$label" "$description" "$expiry"
  done < "$PLAN_ROWS_FILE"

  load_plan_rows lightning.lnd_payments label amount_msat description expiry
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_lnd_paid_cln_invoice cln_merchant "$amount_msat" "$label" "$description" "$expiry"
  done < "$PLAN_ROWS_FILE"
}

main() {
  trap cleanup_plan_rows EXIT
  ensure_plan
  ensure_state_for_plan
  run_mainchain_topups
  run_lightning_activity
  run_lnd_activity
  run_mainchain_withdrawals

  sleep 2
  echo "Lightning business workload is present on the merchant node."
  cln cln_merchant listforwards
}

main "$@"
