#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lightning-common.sh"

NODE_FUND_BTC="${KASSIBER_REGTEST_LIGHTNING_NODE_FUND_BTC:-2.0}"
NODE_MIN_ONCHAIN_SAT="${KASSIBER_REGTEST_LIGHTNING_NODE_MIN_ONCHAIN_SAT:-50000000}"
CHANNEL_CAPACITY_SAT="${KASSIBER_REGTEST_LIGHTNING_CHANNEL_CAPACITY_SAT:-5000000}"
CHANNEL_PUSH_MSAT="${KASSIBER_REGTEST_LIGHTNING_CHANNEL_PUSH_MSAT:-2500000000}"

wait_for_cln() {
  local service="$1"
  local deadline=$((SECONDS + 180))
  until cln "$service" getinfo >/dev/null 2>&1
  do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for $service lightning-cli." >&2
      compose logs --tail=120 "$service" >&2 || true
      return 1
    fi
    sleep 2
  done
}

fund_node_if_needed() {
  local service="$1"
  local sat any_sat address
  sat="$(cln_onchain_sat "$service" 2>/dev/null || printf '0')"
  if [ "$sat" -ge "$NODE_MIN_ONCHAIN_SAT" ]; then
    echo "$service already has on-chain CLN funds (${sat} sat)."
    return 1
  fi
  any_sat="$(cln_any_onchain_sat "$service" 2>/dev/null || printf '0')"
  if [ "$any_sat" -ge "$NODE_MIN_ONCHAIN_SAT" ]; then
    echo "$service has unconfirmed CLN funds (${any_sat} sat); mining confirmations."
    return 0
  fi
  address="$(cln_new_address "$service")"
  if [ -z "$address" ]; then
    echo "Could not get a funding address from $service." >&2
    return 2
  fi
  btc -rpcwallet="$FAUCET_WALLET" sendtoaddress "$address" "$NODE_FUND_BTC" >/dev/null
  echo "Funded $service with $NODE_FUND_BTC BTC."
  return 0
}

wait_for_node_funds() {
  local service="$1"
  local min_sat="$2"
  local deadline=$((SECONDS + 240))
  local sat
  while true; do
    sat="$(cln_onchain_sat "$service" 2>/dev/null || printf '0')"
    if [ "$sat" -ge "$min_sat" ]; then
      echo "$service sees $sat sat of on-chain CLN funds."
      return 0
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for $service to see at least $min_sat sat (saw $sat)." >&2
      cln "$service" listfunds || true
      return 1
    fi
    sleep 3
  done
}

peer_connected() {
  local service="$1"
  local peer_id="$2"
  cln "$service" listpeers "$peer_id" | python3 -c 'import json, sys
data = json.load(sys.stdin)
for peer in data.get("peers", []):
    if peer.get("connected"):
        sys.exit(0)
sys.exit(1)'
}

ensure_connected() {
  local from_service="$1"
  local to_service="$2"
  local host="$3"
  local peer_id
  peer_id="$(cln_id "$to_service")"
  if peer_connected "$from_service" "$peer_id"; then
    return 0
  fi
  cln "$from_service" connect "$peer_id@$host:9735" >/dev/null
}

ensure_channel() {
  local from_service="$1"
  local to_service="$2"
  local host="$3"
  local peer_id
  peer_id="$(cln_id "$to_service")"
  ensure_connected "$from_service" "$to_service" "$host"
  if cln_has_normal_channel_with_peer "$from_service" "$peer_id"; then
    echo "$from_service already has a normal channel with $to_service."
    return 1
  fi
  if cln_has_channel_with_peer "$from_service" "$peer_id"; then
    echo "$from_service already has a pending channel with $to_service; mining confirmations."
    return 0
  fi
  local result
  result="$(
  cln "$from_service" -k fundchannel \
    id="$peer_id" \
    amount="${CHANNEL_CAPACITY_SAT}sat" \
    push_msat="${CHANNEL_PUSH_MSAT}msat" \
    announce=true \
      minconf=1
  )"
  if ! printf '%s\n' "$result" | python3 -c 'import json, sys
data = json.load(sys.stdin)
sys.exit(1 if "code" in data else 0)'; then
    printf '%s\n' "$result" >&2
    echo "fundchannel failed for $from_service -> $to_service." >&2
    return 1
  fi
  echo "Opened $CHANNEL_CAPACITY_SAT sat channel $from_service -> $to_service."
  return 0
}

wait_for_normal_channels() {
  local service="$1"
  local expected="$2"
  local deadline=$((SECONDS + 240))
  local count
  while true; do
    count="$(cln_channel_count_normal "$service" 2>/dev/null || printf '0')"
    if [ "$count" -ge "$expected" ]; then
      return 0
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for $service to reach $expected normal channels (saw $count)." >&2
      cln "$service" listpeerchannels || true
      return 1
    fi
    sleep 2
  done
}

main() {
  for service in cln_merchant cln_customer cln_supplier cln_router; do
    wait_for_cln "$service"
  done

  ensure_faucet_wallet
  ensure_faucet_funds

  local funded=0
  for service in cln_merchant cln_customer cln_supplier cln_router; do
    if fund_node_if_needed "$service"; then
      funded=1
    fi
  done
  if [ "$funded" -eq 1 ]; then
    mine_to_faucet 6
  fi
  for service in cln_merchant cln_customer cln_supplier cln_router; do
    wait_for_node_funds "$service" "$NODE_MIN_ONCHAIN_SAT"
  done

  local opened=0
  if ensure_channel cln_customer cln_merchant cln_merchant; then opened=1; fi
  if ensure_channel cln_merchant cln_router cln_router; then opened=1; fi
  if ensure_channel cln_router cln_supplier cln_supplier; then opened=1; fi
  if [ "$opened" -eq 1 ]; then
    mine_to_faucet 6
  fi

  wait_for_normal_channels cln_customer 1
  wait_for_normal_channels cln_merchant 2
  wait_for_normal_channels cln_router 2
  wait_for_normal_channels cln_supplier 1

  for service in cln_customer cln_merchant cln_router cln_supplier; do
    cln "$service" setchannel all 1000 500 >/dev/null || true
  done

  echo "Lightning business topology is ready:"
  for service in cln_customer cln_merchant cln_router cln_supplier; do
    printf '  %s %s %s normal_channels=%s\n' \
      "$service" \
      "$(cln_alias "$service")" \
      "$(cln_id "$service")" \
      "$(cln_channel_count_normal "$service")"
  done
}

main "$@"
