#!/usr/bin/env bash
# Shared helpers for the opt-in Core Lightning regtest business lane.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-}"
if [ -z "$PROJECT" ]; then
  PROJECT="$(cd "$ROOT" && python3 -c 'import hashlib, os; print("kassiber-regtest-" + hashlib.sha256(os.getcwd().encode()).hexdigest()[:12])')"
fi
PROJECT="${PROJECT:-kassiber-regtest}"

COMPOSE_FILES=(
  -f "$ROOT/dev/regtest/compose.bitcoin.yml"
  -f "$ROOT/dev/regtest/compose.lightning.yml"
)

FAUCET_WALLET="${KASSIBER_REGTEST_LIGHTNING_FAUCET_WALLET:-kassiber-ln-faucet}"

docker_compose() {
  if docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose "$@"
  elif sudo -n docker info >/dev/null 2>&1 && sudo -n docker compose version >/dev/null 2>&1; then
    sudo -n --preserve-env=KASSIBER_REGTEST_RPC_USER,KASSIBER_REGTEST_RPC_PASSWORD,KASSIBER_REGTEST_RPC_AUTH,KASSIBER_REGTEST_CLN_IMAGE docker compose "$@"
  elif docker info >/dev/null 2>&1 && command -v docker-compose >/dev/null 2>&1; then
    docker-compose "$@"
  elif sudo -n docker info >/dev/null 2>&1 && sudo -n docker-compose version >/dev/null 2>&1; then
    sudo -n env \
      KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-}" \
      KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-}" \
      KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-}" \
      KASSIBER_REGTEST_CLN_IMAGE="${KASSIBER_REGTEST_CLN_IMAGE:-}" \
      docker-compose "$@"
  else
    echo "Docker Compose is required for the Lightning regtest lane." >&2
    exit 2
  fi
}

compose() {
  docker_compose -p "$PROJECT" "${COMPOSE_FILES[@]}" "$@"
}

btc() {
  compose exec -T bitcoind bitcoin-cli \
    -regtest \
    -rpcuser="${KASSIBER_REGTEST_RPC_USER:?KASSIBER_REGTEST_RPC_USER is required}" \
    -rpcpassword="${KASSIBER_REGTEST_RPC_PASSWORD:?KASSIBER_REGTEST_RPC_PASSWORD is required}" \
    "$@" </dev/null
}

ensure_core_wallet() {
  local wallet="$1"
  if btc -rpcwallet="$wallet" getwalletinfo >/dev/null 2>&1; then
    return 0
  fi
  if btc loadwallet "$wallet" >/dev/null 2>&1; then
    return 0
  fi
  btc createwallet "$wallet" false false "" false true true >/dev/null
}

faucet_balance_ok() {
  local balance="$1"
  python3 -c 'import sys; sys.exit(0 if float(sys.argv[1]) >= 10 else 1)' "$balance"
}

mine_to_faucet() {
  local blocks="$1"
  local address
  address="$(btc -rpcwallet="$FAUCET_WALLET" getnewaddress "kassiber lightning faucet" bech32)"
  btc generatetoaddress "$blocks" "$address" >/dev/null
}

ensure_faucet_wallet() {
  ensure_core_wallet "$FAUCET_WALLET"
}

ensure_faucet_funds() {
  local blocks balance
  ensure_faucet_wallet
  blocks="$(btc getblockcount)"
  if [ "$blocks" -lt 120 ]; then
    mine_to_faucet "$((120 - blocks))"
  fi
  balance="$(btc -rpcwallet="$FAUCET_WALLET" getbalance)"
  if ! faucet_balance_ok "$balance"; then
    mine_to_faucet 120
  fi
}

sat_to_btc() {
  python3 -c 'from decimal import Decimal; import sys
sat = Decimal(int(sys.argv[1]))
print(f"{sat / Decimal(100000000):.8f}")' "$1"
}

wallet_balance_sat() {
  local wallet="$1"
  btc -rpcwallet="$wallet" getbalance | python3 -c 'from decimal import Decimal; import sys
print(int(Decimal(sys.stdin.read().strip() or "0") * Decimal(100000000)))'
}

cln() {
  local service="$1"
  shift
  compose exec -T "$service" lightning-cli --network=regtest "$@" </dev/null
}

json_get() {
  local key="$1"
  python3 -c 'import json, sys
data = json.load(sys.stdin)
value = data
for part in sys.argv[1].split("."):
    if isinstance(value, dict):
        value = value.get(part)
    else:
        value = None
        break
if value is None:
    print("")
elif isinstance(value, bool):
    print("true" if value else "false")
else:
    print(value)' "$key"
}

json_msat_to_sat() {
  python3 -c 'import json, sys
allowed_statuses = set(sys.argv[1:] or ["confirmed", "unconfirmed"])
def parse_msat(value):
    if value in (None, ""):
        return 0
    if isinstance(value, dict):
        for key in ("msat", "millisatoshis", "millisatoshi"):
            if key in value:
                return parse_msat(value[key])
        return 0
    if isinstance(value, int):
        return value
    text = str(value).strip().lower().replace(",", "")
    if text.endswith("msat"):
        return int(float(text[:-4] or "0"))
    if text.endswith("sat"):
        return int(float(text[:-3] or "0") * 1000)
    if text.endswith("btc"):
        return int(float(text[:-3] or "0") * 100_000_000_000)
    return int(float(text or "0"))
data = json.load(sys.stdin)
total = 0
for output in data.get("outputs", []):
    if output.get("reserved_to_block"):
        continue
    status = output.get("status")
    if status not in allowed_statuses:
        continue
    total += parse_msat(output.get("amount_msat"))
print(total // 1000)' "$@"
}

cln_id() {
  cln "$1" getinfo | python3 -c 'import json, sys; print(json.load(sys.stdin).get("id") or "")'
}

cln_alias() {
  cln "$1" getinfo | python3 -c 'import json, sys; print(json.load(sys.stdin).get("alias") or "")'
}

cln_new_address() {
  cln "$1" newaddr bech32 | python3 -c 'import json, sys; data=json.load(sys.stdin); print(data.get("bech32") or data.get("p2tr") or data.get("address") or "")'
}

cln_onchain_sat() {
  cln "$1" listfunds | json_msat_to_sat confirmed
}

cln_any_onchain_sat() {
  cln "$1" listfunds | json_msat_to_sat confirmed unconfirmed
}

cln_has_channel_with_peer() {
  local service="$1"
  local peer_id="$2"
  cln "$service" listpeerchannels | python3 -c 'import json, sys
peer_id = sys.argv[1]
data = json.load(sys.stdin)
for channel in data.get("channels", []):
    if channel.get("peer_id") != peer_id:
        continue
    state = str(channel.get("state") or "").lower()
    if "closing" in state or "closed" in state or "onchain" in state:
        continue
    sys.exit(0)
sys.exit(1)' "$peer_id"
}

cln_has_normal_channel_with_peer() {
  local service="$1"
  local peer_id="$2"
  cln "$service" listpeerchannels | python3 -c 'import json, sys
peer_id = sys.argv[1]
data = json.load(sys.stdin)
for channel in data.get("channels", []):
    if channel.get("peer_id") != peer_id:
        continue
    if str(channel.get("state") or "").lower() == "channeld_normal":
        sys.exit(0)
sys.exit(1)' "$peer_id"
}

cln_channel_count_normal() {
  cln "$1" listpeerchannels | python3 -c 'import json, sys
data = json.load(sys.stdin)
count = 0
for channel in data.get("channels", []):
    if str(channel.get("state") or "").lower() == "channeld_normal":
        count += 1
print(count)'
}
