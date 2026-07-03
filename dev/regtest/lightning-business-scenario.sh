#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lightning-common.sh"

PLAN_FILE="${KASSIBER_LIGHTNING_BUSINESS_PLAN:-${KASSIBER_LIGHTNING_BUSINESS_HOME:-${TMPDIR:-/tmp}/kassiber-lightning-business}/business-plan.json}"
STATE_FILE="${KASSIBER_LIGHTNING_BUSINESS_STATE:-${PLAN_FILE%.json}.state.json}"

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

state_done() {
  local label="$1"
  [ -f "$STATE_FILE" ] || return 1
  python3 - "$STATE_FILE" "$label" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    state = json.load(handle)
sys.exit(0 if sys.argv[2] in state.get("completed", {}) else 1)
PY
}

state_mark_done() {
  local label="$1"
  local txid="${2:-}"
  mkdir -p "$(dirname "$STATE_FILE")"
  python3 - "$STATE_FILE" "$label" "$txid" <<'PY'
import json
import os
import sys

path, label, txid = sys.argv[1:4]
try:
    with open(path, "r", encoding="utf-8") as handle:
        state = json.load(handle)
except (OSError, ValueError):
    state = {}
completed = state.setdefault("completed", {})
completed[label] = {"txid": txid}
tmp = f"{path}.tmp"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump(state, handle, indent=2, sort_keys=True)
    handle.write("\n")
os.replace(tmp, path)
PY
}

invoice_status() {
  local service="$1"
  local label="$2"
  cln "$service" listinvoices "$label" | python3 -c 'import json, sys
data = json.load(sys.stdin)
invoices = data.get("invoices") or []
print((invoices[0].get("status") if invoices else "") or "")'
}

invoice_bolt11() {
  local service="$1"
  local label="$2"
  cln "$service" listinvoices "$label" | python3 -c 'import json, sys
data = json.load(sys.stdin)
invoices = data.get("invoices") or []
print((invoices[0].get("bolt11") if invoices else "") or "")'
}

invoice_payment_hash() {
  local service="$1"
  local label="$2"
  cln "$service" listinvoices "$label" | python3 -c 'import json, sys
data = json.load(sys.stdin)
invoices = data.get("invoices") or []
print((invoices[0].get("payment_hash") if invoices else "") or "")'
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

ensure_invoice() {
  local service="$1"
  local amount_msat="$2"
  local label="$3"
  local description="$4"
  local expiry="${5:-3600}"
  local status
  status="$(invoice_status "$service" "$label")"
  if [ "$status" = "paid" ]; then
    return 0
  fi
  if [ "$status" = "expired" ]; then
    cln "$service" delinvoice "$label" expired >/dev/null || true
    status=""
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

ensure_expired_merchant_quote() {
  local label="$1"
  local amount_msat="$2"
  local description="$3"
  local expiry="${4:-1}"
  local status
  status="$(invoice_status cln_merchant "$label")"
  if [ "$status" = "paid" ] || [ "$status" = "expired" ]; then
    echo "$label already $status."
    return 0
  fi
  if [ -z "$status" ]; then
    ensure_invoice cln_merchant "$amount_msat" "$label" "$description" "$expiry"
  fi
  sleep "$((expiry + 1))"
  if cln cln_customer -k pay bolt11="$(invoice_bolt11 cln_merchant "$label")" maxfeepercent=5 exemptfee=5000 >/dev/null 2>&1; then
    echo "warning: expected $label to be expired, but payment succeeded." >&2
  else
    echo "$label intentionally left expired/unpaid."
  fi
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
    echo "warning: expected $label to fail, but it is already $status." >&2
    return 0
  fi
  if cln "$payer" -k pay \
    bolt11="$(invoice_bolt11 "$issuer" "$label")" \
    retry_for=2 \
    maxfeepercent=5 \
    exemptfee=5000 >/dev/null 2>&1; then
    echo "warning: expected $label to fail, but payment succeeded." >&2
  else
    echo "$label intentionally failed due to liquidity limits."
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

  while IFS=$'\t' read -r wallet label amount_sat _description _confirmations; do
    [ -n "$wallet" ] || continue
    if state_done "$label"; then
      ensure_core_wallet "$wallet"
      continue
    fi
    if ensure_actor_wallet_funds "$wallet" "$((amount_sat + buffer_sat))"; then
      funded=1
    fi
  done < <(plan_rows mainchain.topups wallet label amount_sat description confirmations)

  if [ "$funded" -eq 1 ]; then
    mine_to_faucet "$funding_confirmations"
  fi

  while IFS=$'\t' read -r wallet label amount_sat description confirmations; do
    [ -n "$wallet" ] || continue
    if state_done "$label"; then
      echo "$label already broadcast."
      continue
    fi
    local address txid
    address="$(cln_new_address cln_merchant)"
    txid="$(btc -rpcwallet="$wallet" sendtoaddress \
      "$address" \
      "$(sat_to_btc "$amount_sat")" \
      "$label" \
      "$description")"
    state_mark_done "$label" "$txid"
    mine_to_faucet "${confirmations:-1}"
    echo "Broadcast $label ($amount_sat sat) to merchant CLN wallet."
  done < <(plan_rows mainchain.topups wallet label amount_sat description confirmations)
}

run_mainchain_withdrawals() {
  while IFS=$'\t' read -r wallet label amount_sat description confirmations; do
    [ -n "$wallet" ] || continue
    ensure_core_wallet "$wallet"
    if state_done "$label"; then
      echo "$label already withdrawn."
      continue
    fi
    local address result txid
    address="$(btc -rpcwallet="$wallet" getnewaddress "$label" bech32)"
    result="$(cln cln_merchant withdraw "$address" "${amount_sat}sat")"
    txid="$(printf '%s\n' "$result" | python3 -c 'import json, sys
data = json.load(sys.stdin)
print(data.get("txid") or "")')"
    state_mark_done "$label" "$txid"
    mine_to_faucet "${confirmations:-1}"
    echo "Broadcast $label ($amount_sat sat) from merchant CLN wallet."
  done < <(plan_rows mainchain.withdrawals wallet label amount_sat description confirmations)
}

run_lightning_activity() {
  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_merchant cln_customer "$amount_msat" "$label" "$description"
  done < <(plan_rows lightning.merchant_invoices label amount_msat description expiry)

  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_supplier cln_merchant "$amount_msat" "$label" "$description"
  done < <(plan_rows lightning.supplier_invoices label amount_msat description expiry)

  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_supplier cln_customer "$amount_msat" "$label" "$description"
  done < <(plan_rows lightning.routed_customer_supplier label amount_msat description expiry)

  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_paid_invoice cln_customer cln_router "$amount_msat" "$label" "$description"
  done < <(plan_rows lightning.routed_router_customer label amount_msat description expiry)

  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_expired_merchant_quote "$label" "$amount_msat" "$description" "$expiry"
  done < <(plan_rows lightning.expired_invoices label amount_msat description expiry)

  while IFS=$'\t' read -r label amount_msat description expiry; do
    [ -n "$label" ] || continue
    ensure_failed_payment cln_supplier cln_merchant "$amount_msat" "$label" "$description" "$expiry"
  done < <(plan_rows lightning.failed_payments label amount_msat description expiry)
}

main() {
  ensure_plan
  run_mainchain_topups
  run_lightning_activity
  run_mainchain_withdrawals

  sleep 2
  echo "Lightning business workload is present on the merchant node."
  cln cln_merchant listforwards
}

main "$@"
