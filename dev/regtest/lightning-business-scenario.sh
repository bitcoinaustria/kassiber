#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lightning-common.sh"

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
  local label="expired-quote-001"
  local status
  status="$(invoice_status cln_merchant "$label")"
  if [ "$status" = "paid" ] || [ "$status" = "expired" ]; then
    echo "$label already $status."
    return 0
  fi
  if [ -z "$status" ]; then
    ensure_invoice cln_merchant 99000000 "$label" "Expired wholesale quote 001" 1
  fi
  sleep 2
  if cln cln_customer -k pay bolt11="$(invoice_bolt11 cln_merchant "$label")" maxfeepercent=5 exemptfee=5000 >/dev/null 2>&1; then
    echo "warning: expected $label to be expired, but payment succeeded." >&2
  else
    echo "$label intentionally left expired/unpaid."
  fi
}

main() {
  ensure_paid_invoice cln_merchant cln_customer 150000000 merchant-sale-001 "Kassiber Coffee invoice 001"
  ensure_paid_invoice cln_merchant cln_customer 175000000 merchant-sale-002 "Kassiber Coffee invoice 002"
  ensure_paid_invoice cln_merchant cln_customer 210000000 merchant-sale-003 "Kassiber Coffee invoice 003"

  ensure_paid_invoice cln_supplier cln_merchant 240000000 supplier-restock-001 "Supplier beans restock 001"
  ensure_paid_invoice cln_supplier cln_merchant 260000000 supplier-restock-002 "Supplier equipment lease 002"

  ensure_paid_invoice cln_supplier cln_customer 120000000 routed-customer-supplier-001 "Customer routed supplier payment 001"
  ensure_paid_invoice cln_supplier cln_customer 130000000 routed-customer-supplier-002 "Customer routed supplier payment 002"
  ensure_paid_invoice cln_customer cln_router 80000000 routed-router-customer-001 "Router routed customer payment 001"

  ensure_expired_merchant_quote

  sleep 2
  echo "Lightning business workload is present on the merchant node."
  cln cln_merchant listforwards
}

main "$@"
