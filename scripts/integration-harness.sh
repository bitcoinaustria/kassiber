#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="${1:-fast}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
RUNNER=()
STARTED_COMPOSE=0
STARTED_BOLTZ=0
BOLTZ_COMPOSE_FILE=""
BOLTZ_COMPOSE_TEMP=""
SUDO_DOCKER_ENV=COMPOSE_PROFILES,KASSIBER_REGTEST_COMPOSE_PROFILES,KASSIBER_REGTEST_RPC_USER,KASSIBER_REGTEST_RPC_PASSWORD,KASSIBER_REGTEST_RPC_AUTH,KASSIBER_REGTEST_RPC_PORT,KASSIBER_REGTEST_ELEMENTS_RPC_PORT,KASSIBER_REGTEST_BITCOIND_IMAGE,KASSIBER_REGTEST_ELEMENTSD_IMAGE,KASSIBER_REGTEST_FULCRUM_IMAGE,KASSIBER_REGTEST_FRIGATE_IMAGE,KASSIBER_REGTEST_FRIGATE_VERSION,KASSIBER_REGTEST_FRIGATE_TARBALL_SHA256,KASSIBER_REGTEST_BACKEND_STACK_IMAGE,KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT,KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT,KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT,KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT,KASSIBER_REGTEST_FRIGATE_PORT,KASSIBER_REGTEST_CLN_IMAGE,KASSIBER_REGTEST_CLN_MERCHANT_PORT,KASSIBER_REGTEST_CLN_CUSTOMER_PORT,KASSIBER_REGTEST_CLN_SUPPLIER_PORT,KASSIBER_REGTEST_CLN_ROUTER_PORT,KASSIBER_REGTEST_LND_IMAGE,KASSIBER_REGTEST_LND_BACKUP_P2P_PORT,KASSIBER_REGTEST_LND_BACKUP_REST_PORT,KASSIBER_REGTEST_LND_BACKUP_RPC_PORT
SUDO_DOCKER=(sudo -n --preserve-env="$SUDO_DOCKER_ENV" docker)

export PYTHONHASHSEED="${PYTHONHASHSEED:-0}"
export TZ="${TZ:-UTC}"
export LC_ALL="${LC_ALL:-C.UTF-8}"
export LANG="${LANG:-C.UTF-8}"

if [ -n "${VIRTUAL_ENV:-}" ] && command -v python3 >/dev/null 2>&1; then
  PYTHON_BIN="$(command -v python3)"
elif command -v uv >/dev/null 2>&1; then
  RUNNER=(uv run)
elif [ -x "$ROOT/.venv/bin/python" ]; then
  PYTHON_BIN="$ROOT/.venv/bin/python"
fi

py() {
  if [ ${#RUNNER[@]} -gt 0 ]; then
    "${RUNNER[@]}" python "$@"
  else
    "$PYTHON_BIN" "$@"
  fi
}

ensure_python_runtime() {
  if py - <<'PY' >/dev/null 2>&1
import importlib.util
import sys

missing = [
    name
    for name in ("embit", "openpyxl")
    if importlib.util.find_spec(name) is None
]
if missing:
    print(",".join(missing), file=sys.stderr)
    sys.exit(1)
PY
  then
    return 0
  fi
  echo "Kassiber's Python dependencies are not available in this interpreter." >&2
  echo "Run ./scripts/bootstrap-dev-env.sh, activate a prepared virtualenv, or install uv and rerun." >&2
  echo "Current Python: $PYTHON_BIN" >&2
  exit 2
}

run_fast() {
  ensure_python_runtime
  KASSIBER_NO_EGRESS=1 py -m unittest \
    tests.test_regtest_harness \
    tests.test_lightning_business_plan \
    -v
}

rpc_auth() {
  py - <<'PY'
import hashlib
import hmac
import os
import secrets

user = os.environ["KASSIBER_REGTEST_RPC_USER"]
password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
salt = secrets.token_hex(16)
digest = hmac.new(salt.encode("utf-8"), password.encode("utf-8"), hashlib.sha256).hexdigest()
print(f"{user}:{salt}${digest}")
PY
}

docker_compose() {
  if docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose "$@"
  elif "${SUDO_DOCKER[@]}" info >/dev/null 2>&1 && "${SUDO_DOCKER[@]}" compose version >/dev/null 2>&1; then
    "${SUDO_DOCKER[@]}" compose "$@"
  elif docker info >/dev/null 2>&1 && command -v docker-compose >/dev/null 2>&1; then
    docker-compose "$@"
  elif sudo -n docker info >/dev/null 2>&1 && sudo -n docker-compose version >/dev/null 2>&1; then
    sudo -n env \
      KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-}" \
      KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-}" \
      KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-}" \
      KASSIBER_REGTEST_RPC_PORT="${KASSIBER_REGTEST_RPC_PORT:-}" \
      KASSIBER_REGTEST_ELEMENTS_RPC_PORT="${KASSIBER_REGTEST_ELEMENTS_RPC_PORT:-}" \
      KASSIBER_REGTEST_BITCOIND_IMAGE="${KASSIBER_REGTEST_BITCOIND_IMAGE:-}" \
      KASSIBER_REGTEST_ELEMENTSD_IMAGE="${KASSIBER_REGTEST_ELEMENTSD_IMAGE:-}" \
      KASSIBER_REGTEST_FULCRUM_IMAGE="${KASSIBER_REGTEST_FULCRUM_IMAGE:-}" \
      KASSIBER_REGTEST_FRIGATE_IMAGE="${KASSIBER_REGTEST_FRIGATE_IMAGE:-}" \
      KASSIBER_REGTEST_FRIGATE_VERSION="${KASSIBER_REGTEST_FRIGATE_VERSION:-}" \
      KASSIBER_REGTEST_FRIGATE_TARBALL_SHA256="${KASSIBER_REGTEST_FRIGATE_TARBALL_SHA256:-}" \
      KASSIBER_REGTEST_BACKEND_STACK_IMAGE="${KASSIBER_REGTEST_BACKEND_STACK_IMAGE:-}" \
      KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT="${KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT:-}" \
      KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT="${KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT:-}" \
      KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT="${KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT:-}" \
      KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT="${KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT:-}" \
      KASSIBER_REGTEST_CLN_IMAGE="${KASSIBER_REGTEST_CLN_IMAGE:-}" \
      KASSIBER_REGTEST_CLN_MERCHANT_PORT="${KASSIBER_REGTEST_CLN_MERCHANT_PORT:-}" \
      KASSIBER_REGTEST_CLN_CUSTOMER_PORT="${KASSIBER_REGTEST_CLN_CUSTOMER_PORT:-}" \
      KASSIBER_REGTEST_CLN_SUPPLIER_PORT="${KASSIBER_REGTEST_CLN_SUPPLIER_PORT:-}" \
      KASSIBER_REGTEST_CLN_ROUTER_PORT="${KASSIBER_REGTEST_CLN_ROUTER_PORT:-}" \
      KASSIBER_REGTEST_LND_IMAGE="${KASSIBER_REGTEST_LND_IMAGE:-}" \
      KASSIBER_REGTEST_LND_BACKUP_P2P_PORT="${KASSIBER_REGTEST_LND_BACKUP_P2P_PORT:-}" \
      KASSIBER_REGTEST_LND_BACKUP_REST_PORT="${KASSIBER_REGTEST_LND_BACKUP_REST_PORT:-}" \
      KASSIBER_REGTEST_LND_BACKUP_RPC_PORT="${KASSIBER_REGTEST_LND_BACKUP_RPC_PORT:-}" \
      KASSIBER_REGTEST_FRIGATE_PORT="${KASSIBER_REGTEST_FRIGATE_PORT:-}" \
      KASSIBER_REGTEST_COMPOSE_PROFILES="${KASSIBER_REGTEST_COMPOSE_PROFILES:-}" \
      COMPOSE_PROFILES="${COMPOSE_PROFILES:-}" \
      docker-compose "$@"
  else
    echo "Docker Compose is required for the slow regtest lane." >&2
    echo "Install Docker or set KASSIBER_REGTEST_CORE_URL with matching RPC credentials for an already-running regtest node." >&2
    exit 2
  fi
}

configure_lightning_ports() {
  export KASSIBER_REGTEST_CLN_MERCHANT_PORT="${KASSIBER_REGTEST_CLN_MERCHANT_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 1292))}"
  export KASSIBER_REGTEST_CLN_CUSTOMER_PORT="${KASSIBER_REGTEST_CLN_CUSTOMER_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 1293))}"
  export KASSIBER_REGTEST_CLN_SUPPLIER_PORT="${KASSIBER_REGTEST_CLN_SUPPLIER_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 1294))}"
  export KASSIBER_REGTEST_CLN_ROUTER_PORT="${KASSIBER_REGTEST_CLN_ROUTER_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 1295))}"
  export KASSIBER_REGTEST_LND_BACKUP_P2P_PORT="${KASSIBER_REGTEST_LND_BACKUP_P2P_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 1296))}"
  export KASSIBER_REGTEST_LND_BACKUP_REST_PORT="${KASSIBER_REGTEST_LND_BACKUP_REST_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 1297))}"
  export KASSIBER_REGTEST_LND_BACKUP_RPC_PORT="${KASSIBER_REGTEST_LND_BACKUP_RPC_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 1298))}"
}

docker_compose_regtest() {
  local files=(-f dev/regtest/compose.bitcoin.yml)
  if [ -n "${KASSIBER_REGTEST_USE_LIGHTNING_COMPOSE:-}" ]; then
    files+=(-f dev/regtest/compose.lightning.yml)
  fi
  docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" "${files[@]}" "$@"
}

probe_core() {
  local debug="${1:-0}"
  KASSIBER_REGTEST_PROBE_DEBUG="$debug" py - <<'PY'
import base64
import json
import os
import sys
from urllib import error, request

url = os.environ["KASSIBER_REGTEST_CORE_URL"]
user = os.environ["KASSIBER_REGTEST_RPC_USER"]
password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
debug = os.environ.get("KASSIBER_REGTEST_PROBE_DEBUG") == "1"
payload = json.dumps({"jsonrpc": "1.0", "id": "probe", "method": "getblockchaininfo", "params": []}).encode()
req = request.Request(url, data=payload, headers={"Content-Type": "application/json"})
req.add_header("Authorization", "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode())
try:
    with request.urlopen(req, timeout=3) as response:
        body = json.loads(response.read().decode())
except error.HTTPError as exc:
    body_text = exc.read().decode(errors="replace")
    if debug:
        print(f"HTTP {exc.code}: {body_text}", file=sys.stderr)
    sys.exit(1)
except Exception as exc:
    if debug:
        print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
    sys.exit(1)
ok = body.get("result", {}).get("chain") == "regtest"
if debug and not ok:
    print(json.dumps(body, sort_keys=True), file=sys.stderr)
sys.exit(0 if ok else 1)
PY
}

probe_elements() {
  local debug="${1:-0}"
  KASSIBER_REGTEST_PROBE_DEBUG="$debug" py - <<'PY'
import base64
import json
import os
import sys
from urllib import error, request

url = os.environ["KASSIBER_REGTEST_ELEMENTS_URL"]
user = os.environ["KASSIBER_REGTEST_RPC_USER"]
password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
debug = os.environ.get("KASSIBER_REGTEST_PROBE_DEBUG") == "1"
payload = json.dumps({"jsonrpc": "1.0", "id": "probe-elements", "method": "getblockchaininfo", "params": []}).encode()
req = request.Request(url, data=payload, headers={"Content-Type": "application/json"})
req.add_header("Authorization", "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode())
try:
    with request.urlopen(req, timeout=3) as response:
        body = json.loads(response.read().decode())
except error.HTTPError as exc:
    body_text = exc.read().decode(errors="replace")
    if debug:
        print(f"HTTP {exc.code}: {body_text}", file=sys.stderr)
    sys.exit(1)
except Exception as exc:
    if debug:
        print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
    sys.exit(1)
ok = body.get("result", {}).get("chain") == "elementsregtest"
if debug and not ok:
    print(json.dumps(body, sort_keys=True), file=sys.stderr)
sys.exit(0 if ok else 1)
PY
}

wait_for_core() {
  local deadline
  deadline=$((SECONDS + 90))
  until probe_core 0
  do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for bitcoind regtest RPC." >&2
      probe_core 1 || true
      return 1
    fi
    sleep 2
  done
}

wait_for_elements() {
  local deadline
  deadline=$((SECONDS + 90))
  until probe_elements 0
  do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for elementsd regtest RPC." >&2
      probe_elements 1 || true
      return 1
    fi
    sleep 2
  done
}

run_with_bitcoin_core() {
  ensure_python_runtime
  local provided_core_url=0
  if [ -n "${KASSIBER_REGTEST_CORE_URL:-}" ]; then
    provided_core_url=1
  fi

  export KASSIBER_INTEGRATION=1
  export KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-kassiber}"
  export KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-$(py -c 'import secrets; print(secrets.token_urlsafe(24))')}"
  export KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-$(rpc_auth)}"
  if [ -z "${KASSIBER_REGTEST_RPC_PORT:-}" ] \
    && [ -z "${KASSIBER_REGTEST_REUSE_CORE:-}" ] \
    && [ "$provided_core_url" -eq 0 ]; then
    export KASSIBER_REGTEST_RPC_PORT="$(choose_regtest_base_port)"
  else
    export KASSIBER_REGTEST_RPC_PORT="${KASSIBER_REGTEST_RPC_PORT:-18443}"
  fi
  export KASSIBER_REGTEST_ELEMENTS_RPC_PORT="${KASSIBER_REGTEST_ELEMENTS_RPC_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 104))}"
  export KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT="${KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 100))}"
  export KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT="${KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 101))}"
  export KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT="${KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 102))}"
  export KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT="${KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 103))}"
  export KASSIBER_REGTEST_FRIGATE_PORT="${KASSIBER_REGTEST_FRIGATE_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 105))}"
  export KASSIBER_REGTEST_CORE_URL="${KASSIBER_REGTEST_CORE_URL:-http://127.0.0.1:${KASSIBER_REGTEST_RPC_PORT}}"
  export KASSIBER_REGTEST_ELEMENTS_URL="${KASSIBER_REGTEST_ELEMENTS_URL:-http://127.0.0.1:${KASSIBER_REGTEST_ELEMENTS_RPC_PORT}}"
  export KASSIBER_REGTEST_COMPOSE_PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-$(py -c 'import hashlib, os; print("kassiber-regtest-" + hashlib.sha256(os.getcwd().encode()).hexdigest()[:12])')}"
  if [ -n "${KASSIBER_REGTEST_USE_LIGHTNING_COMPOSE:-}" ]; then
    configure_lightning_ports
  fi
  if [ -n "${KASSIBER_REGTEST_COMPOSE_PROFILES:-}" ]; then
    export COMPOSE_PROFILES="$KASSIBER_REGTEST_COMPOSE_PROFILES"
  fi

  STARTED_COMPOSE=0
  cleanup() {
    if [ "$STARTED_COMPOSE" -eq 1 ] && [ -z "${KASSIBER_REGTEST_KEEP:-}" ]; then
      docker_compose_regtest down -v
    fi
  }
  trap cleanup EXIT

  if [ -z "${KASSIBER_REGTEST_REUSE_CORE:-}" ] && [ "$provided_core_url" -eq 0 ]; then
    # Mark before `up` so the EXIT trap also removes a half-created project
    # (network/volume/container) when startup fails, e.g. on a port collision.
    STARTED_COMPOSE=1
    if ! docker_compose_regtest up -d; then
      echo "Failed to start the regtest bitcoind container." >&2
      echo "If port ${KASSIBER_REGTEST_RPC_PORT} is already taken (e.g. by the demo-up node)," >&2
      echo "stop it with './scripts/integration-harness.sh demo-down' or pick another port" >&2
      echo "via KASSIBER_REGTEST_RPC_PORT=18444 before running this lane." >&2
      exit 1
    fi
  fi

  wait_for_core
  if [ "$provided_core_url" -eq 0 ] || [ -n "${KASSIBER_REGTEST_REQUIRE_ELEMENTS:-}" ]; then
    wait_for_elements
  fi
  "$@"
}

run_bitcoin_core_smoke() {
  py -m unittest tests.integration.test_live_bitcoin_core_regtest -v
}

run_bitcoin_electrum_parity_smoke() {
  py -m unittest tests.integration.test_live_bitcoin_electrum_parity -v
}

run_bitcoin_backend_suite() {
  run_bitcoin_core_smoke
  run_bitcoin_electrum_parity_smoke
}

probe_frigate() {
  py - <<'PY'
import json
import os
import socket
import sys

port = int(os.environ.get("KASSIBER_REGTEST_FRIGATE_PORT", "18548"))

def call(sock, ident, method, params=None):
    payload = {"jsonrpc": "2.0", "id": ident, "method": method, "params": params or []}
    sock.sendall(json.dumps(payload).encode("utf-8") + b"\n")
    raw = b""
    while not raw.endswith(b"\n"):
        chunk = sock.recv(65536)
        if not chunk:
            raise RuntimeError("Frigate closed the Electrum connection")
        raw += chunk
    response = json.loads(raw.decode("utf-8"))
    if response.get("error"):
        raise RuntimeError(f"{method} failed: {response['error']}")
    return response.get("result")

try:
    with socket.create_connection(("127.0.0.1", port), timeout=3) as sock:
        call(sock, "version", "server.version", ["Kassiber regtest probe", "1.6"])
        features = call(sock, "features", "server.features")
except Exception as exc:
    print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
    sys.exit(1)

if not isinstance(features, dict) or 0 not in list(features.get("silent_payments") or []):
    print(json.dumps(features, sort_keys=True), file=sys.stderr)
    sys.exit(1)
sys.exit(0)
PY
}

seed_frigate_regtest_tip() {
  py - <<'PY'
import base64
import json
import os
import sys
from urllib import error, parse, request

url = os.environ["KASSIBER_REGTEST_CORE_URL"].rstrip("/")
user = os.environ["KASSIBER_REGTEST_RPC_USER"]
password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
wallet_name = os.environ.get("KASSIBER_REGTEST_FRIGATE_READY_WALLET", "kassiber-frigate-ready")


class RpcError(RuntimeError):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def call(method, params=None, *, wallet=None):
    endpoint = url
    if wallet:
        endpoint += "/wallet/" + parse.quote(wallet, safe="")
    payload = json.dumps({"jsonrpc": "1.0", "id": "frigate-ready", "method": method, "params": params or []}).encode()
    req = request.Request(endpoint, data=payload, headers={"Content-Type": "application/json"})
    req.add_header("Authorization", "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode())
    try:
        with request.urlopen(req, timeout=10) as response:
            body = json.loads(response.read().decode())
    except error.HTTPError as exc:
        body = json.loads(exc.read().decode(errors="replace"))
    if body.get("error"):
        err = body["error"]
        raise RpcError(err.get("code"), err.get("message") or method)
    return body.get("result")


info = call("getblockchaininfo")
if not info.get("initialblockdownload") and int(info.get("blocks") or 0) > 0:
    sys.exit(0)

try:
    call("createwallet", [wallet_name])
except RpcError as exc:
    if exc.code not in {-4, -35}:
        raise
    try:
        call("loadwallet", [wallet_name])
    except RpcError as load_exc:
        if load_exc.code != -35:
            raise

address = call("getnewaddress", ["Frigate readiness", "bech32m"], wallet=wallet_name)
call("generatetoaddress", [1, address])
PY
}

wait_for_frigate() {
  local deadline
  deadline=$((SECONDS + ${KASSIBER_REGTEST_FRIGATE_WAIT_SECONDS:-600}))
  until probe_frigate
  do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for Frigate Electrum Silent Payments support on port ${KASSIBER_REGTEST_FRIGATE_PORT}." >&2
      docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" -f dev/regtest/compose.bitcoin.yml logs --tail=120 frigate || true
      return 1
    fi
    sleep 3
  done
}

run_silent_payments_smoke() {
  seed_frigate_regtest_tip
  wait_for_frigate
  py -m unittest tests.test_silent_payments -v
}

run_demo_full() {
  ensure_python_runtime
  py -m tests.integration.regtest_demo
}

DEMO_HOME="${KASSIBER_REGTEST_DEMO_HOME:-$HOME/.kassiber/regtest-demo}"
DEMO_MANIFEST="$DEMO_HOME/demo-manifest.json"
DEMO_SCENARIO="dev/regtest/scenarios/full_accounting.json"

demo_assert_safe_home() {
  local mode="$1"
  KASSIBER_DEMO_HOME_DIR="$DEMO_HOME" \
  KASSIBER_DEMO_MANIFEST="$DEMO_MANIFEST" \
  KASSIBER_DEMO_PURGE_MODE="$mode" \
    py - <<'PY'
import json
import os
import sys
from pathlib import Path

home = Path(os.environ["KASSIBER_DEMO_HOME_DIR"]).expanduser()
manifest_path = Path(os.environ["KASSIBER_DEMO_MANIFEST"]).expanduser()
mode = os.environ["KASSIBER_DEMO_PURGE_MODE"]
try:
    resolved = home.resolve(strict=False)
    user_home = Path.home().resolve(strict=False)
except OSError as exc:
    print(f"Refusing unsafe demo home {home}: {exc}", file=sys.stderr)
    raise SystemExit(2)

def fail(reason: str) -> None:
    print(f"Refusing to {mode} unsafe demo home {resolved}: {reason}", file=sys.stderr)
    raise SystemExit(2)

def manifest_matches() -> bool:
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, ValueError, json.JSONDecodeError):
        return False
    data_root = manifest.get("data_root")
    export_dir = manifest.get("export_dir")
    try:
        data_root_path = (
            Path(str(data_root)).expanduser().resolve(strict=False)
            if data_root
            else None
        )
        export_dir_path = (
            Path(str(export_dir)).expanduser().resolve(strict=False)
            if export_dir
            else None
        )
        return (
            manifest.get("schema_version") == 1
            and manifest.get("scenario_id") == "full-accounting-v1"
            and data_root_path == resolved / "data"
            and export_dir_path == resolved / "exports"
        )
    except (OSError, ValueError):
        return False

dangerous = {Path("/"), user_home, Path("/tmp"), Path("/var/tmp")}
if resolved in dangerous or resolved.parent == Path("/"):
    fail("path is root, user home, a temp root, or root-level")
if len(resolved.parts) < 4:
    fail("path is too shallow")

manifest_ok = manifest_matches()
if mode == "purge":
    if not manifest_ok:
        fail("missing Kassiber regtest demo manifest")
else:
    default_home = user_home / ".kassiber" / "regtest-demo"
    name = resolved.name.lower()
    dedicated_name = name in {"regtest-demo", "kassiber-regtest-demo"} or name.startswith(
        "kassiber-regtest-demo-"
    )
    if not (resolved == default_home or dedicated_name or manifest_ok):
        fail("path does not look like a Kassiber regtest demo home")
PY
}

demo_manifest_get() {
  KASSIBER_DEMO_MANIFEST="$DEMO_MANIFEST" KASSIBER_DEMO_KEY="$1" py - <<'PY'
import json
import os

try:
    with open(os.environ["KASSIBER_DEMO_MANIFEST"], "r", encoding="utf-8") as handle:
        print(json.load(handle).get(os.environ["KASSIBER_DEMO_KEY"]) or "")
except (OSError, ValueError):
    print("")
PY
}

demo_manifest_url_port() {
  KASSIBER_DEMO_URL="$(demo_manifest_get "$1")" py - <<'PY'
import os
from urllib.parse import urlsplit

value = os.environ.get("KASSIBER_DEMO_URL") or ""
if value:
    try:
        print(urlsplit(value).port or "")
    except ValueError:
        print("")
PY
}

demo_scenario_checksum() {
  py -c 'import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$DEMO_SCENARIO"
}

demo_book_needs_rebuild() {
  local checksum
  local current_checksum
  checksum="$(demo_scenario_checksum)"
  current_checksum="$(demo_manifest_get scenario_checksum)"
  if [ -n "${KASSIBER_REGTEST_DEMO_REBUILD:-}" ] \
    || [ ! -d "$DEMO_HOME/data" ] \
    || [ "$current_checksum" != "$checksum" ]; then
    return 0
  fi
  return 1
}

demo_lightning_enabled() {
  case "${KASSIBER_REGTEST_DEMO_LIGHTNING:-1}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      return 1
      ;;
    *)
      return 0
      ;;
  esac
}

demo_configure_lightning() {
  if demo_lightning_enabled; then
    export KASSIBER_REGTEST_USE_LIGHTNING_COMPOSE=1
    export KASSIBER_REGTEST_DEMO_LIGHTNING_ENABLED=1
    if [ -n "${KASSIBER_REGTEST_RPC_PORT:-}" ]; then
      configure_lightning_ports
    fi
  else
    unset KASSIBER_REGTEST_USE_LIGHTNING_COMPOSE
    export KASSIBER_REGTEST_DEMO_LIGHTNING_ENABLED=0
  fi
}

demo_write_manifest() {
  local checksum="$1"
  KASSIBER_DEMO_MANIFEST="$DEMO_MANIFEST" \
  KASSIBER_DEMO_SCENARIO_ID="full-accounting-v1" \
  KASSIBER_DEMO_SCENARIO_CHECKSUM="$checksum" \
  KASSIBER_DEMO_HOME_DIR="$DEMO_HOME" \
    py - <<'PY'
import datetime
import json
import os
import tempfile

manifest_path = os.environ["KASSIBER_DEMO_MANIFEST"]
home = os.environ["KASSIBER_DEMO_HOME_DIR"]
manifest_dir = os.path.dirname(manifest_path) or "."
manifest = {
    "schema_version": 1,
    "scenario_id": os.environ["KASSIBER_DEMO_SCENARIO_ID"],
    "scenario_checksum": os.environ["KASSIBER_DEMO_SCENARIO_CHECKSUM"],
    "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "data_root": os.path.join(home, "data"),
    "export_dir": os.path.join(home, "exports"),
    "core_url": os.environ["KASSIBER_REGTEST_CORE_URL"],
    "elements_core_url": f"http://127.0.0.1:{os.environ['KASSIBER_REGTEST_ELEMENTS_RPC_PORT']}",
    "bitcoin_electrum_url": f"tcp://127.0.0.1:{os.environ['KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT']}",
    "bitcoin_frigate_url": f"tcp://127.0.0.1:{os.environ['KASSIBER_REGTEST_FRIGATE_PORT']}",
    "bitcoin_mempool_url": f"http://127.0.0.1:{os.environ['KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT']}/api",
    "liquid_electrum_url": f"tcp://127.0.0.1:{os.environ['KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT']}",
    "liquid_mempool_url": f"http://127.0.0.1:{os.environ['KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT']}/api",
    "lightning_enabled": os.environ.get("KASSIBER_REGTEST_DEMO_LIGHTNING_ENABLED") == "1",
    "compose_project": os.environ.get("KASSIBER_REGTEST_COMPOSE_PROJECT", ""),
    "rpc_user": os.environ["KASSIBER_REGTEST_RPC_USER"],
    "rpc_password": os.environ["KASSIBER_REGTEST_RPC_PASSWORD"],
}
if manifest["lightning_enabled"]:
    manifest.update(
        {
            "cln_merchant_port": int(os.environ["KASSIBER_REGTEST_CLN_MERCHANT_PORT"]),
            "cln_customer_port": int(os.environ["KASSIBER_REGTEST_CLN_CUSTOMER_PORT"]),
            "cln_supplier_port": int(os.environ["KASSIBER_REGTEST_CLN_SUPPLIER_PORT"]),
            "cln_router_port": int(os.environ["KASSIBER_REGTEST_CLN_ROUTER_PORT"]),
            "lnd_backup_p2p_port": int(os.environ["KASSIBER_REGTEST_LND_BACKUP_P2P_PORT"]),
            "lnd_backup_rest_port": int(os.environ["KASSIBER_REGTEST_LND_BACKUP_REST_PORT"]),
            "lnd_backup_rpc_port": int(os.environ["KASSIBER_REGTEST_LND_BACKUP_RPC_PORT"]),
            "lnd_backup_url": f"https://127.0.0.1:{os.environ['KASSIBER_REGTEST_LND_BACKUP_REST_PORT']}",
            "lightning_wallet": "cln_merchant",
            "lightning_backend": "cln-merchant",
            "lightning_backup_wallet": "lnd_merchant_backup",
            "lightning_backup_backend": "lnd-merchant-backup",
        }
    )
os.makedirs(home, mode=0o700, exist_ok=True)
os.chmod(home, 0o700)
os.makedirs(manifest_dir, mode=0o700, exist_ok=True)
os.chmod(manifest_dir, 0o700)
tmp_path = None
fd = None
for index in range(100):
    candidate = os.path.join(manifest_dir, f".{os.path.basename(manifest_path)}.{os.getpid()}.{index}.tmp")
    try:
        fd = os.open(candidate, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        continue
    tmp_path = candidate
    break
else:
    raise RuntimeError(f"could not create temporary manifest next to {manifest_path}")

try:
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        fd = None
        json.dump(manifest, handle, indent=2, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(tmp_path, manifest_path)
    os.chmod(manifest_path, 0o600)
    try:
        dir_fd = os.open(manifest_dir, os.O_RDONLY)
    except OSError:
        dir_fd = None
    if dir_fd is not None:
        try:
            os.fsync(dir_fd)
        finally:
            os.close(dir_fd)
finally:
    if fd is not None:
        os.close(fd)
    if tmp_path is not None:
        try:
            os.unlink(tmp_path)
        except FileNotFoundError:
            pass
PY
}

demo_load_rpc_env() {
  if [ -z "${KASSIBER_REGTEST_RPC_PORT:-}" ]; then
    KASSIBER_REGTEST_RPC_PORT="$(demo_manifest_url_port core_url)"
    [ -n "$KASSIBER_REGTEST_RPC_PORT" ] && export KASSIBER_REGTEST_RPC_PORT
  fi
  if [ -z "${KASSIBER_REGTEST_RPC_USER:-}" ]; then
    KASSIBER_REGTEST_RPC_USER="$(demo_manifest_get rpc_user)"
    [ -n "$KASSIBER_REGTEST_RPC_USER" ] && export KASSIBER_REGTEST_RPC_USER
  fi
  if [ -z "${KASSIBER_REGTEST_RPC_PASSWORD:-}" ]; then
    KASSIBER_REGTEST_RPC_PASSWORD="$(demo_manifest_get rpc_password)"
    [ -n "$KASSIBER_REGTEST_RPC_PASSWORD" ] && export KASSIBER_REGTEST_RPC_PASSWORD
  fi
  export KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-}"
  export KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-}"
  export KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-}"
}

demo_refresh_live_rate() {
  if [ ! -d "$DEMO_HOME/data" ]; then
    return 0
  fi
  KASSIBER_DEMO_SCENARIO="$DEMO_SCENARIO" \
  KASSIBER_DEMO_DATA_ROOT="$DEMO_HOME/data" \
    py - <<'PY'
import json
import os
import time

from kassiber.core import rates as core_rates
from kassiber.db import open_db
from kassiber.errors import AppError

scenario_path = os.environ["KASSIBER_DEMO_SCENARIO"]
data_root = os.environ["KASSIBER_DEMO_DATA_ROOT"]
with open(scenario_path, "r", encoding="utf-8") as handle:
    scenario = json.load(handle)
pricing = scenario.get("pricing") or {}
live_env = str(pricing.get("live_source_env") or "KASSIBER_REGTEST_DEMO_LIVE_RATES")
live_source = str(os.environ.get(live_env) or pricing.get("live_source") or "").strip().lower()
pair = pricing.get("pair") or "BTC-EUR"
conn = open_db(data_root)
try:
    if not live_source or live_source in {"0", "false", "no", "off"}:
        for source in core_rates.LIVE_MARKET_RATE_SOURCES:
            conn.execute(
                "DELETE FROM rates_cache WHERE pair = ? AND source = ? AND granularity = 'latest'",
                (pair, source),
            )
        conn.execute(
            "DELETE FROM settings WHERE key = ? AND value = ?",
            (core_rates.MARKET_RATE_PROVIDER_SETTING, core_rates.RATE_SOURCE_MEMPOOL),
        )
        conn.commit()
    else:
        normalized = core_rates.normalize_market_rate_provider(live_source)
        deadline = time.monotonic() + 60
        while True:
            try:
                core_rates.sync_latest_rates(conn, pair=pair, source=normalized, commit=True)
                break
            except AppError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(2)
        conn.execute(
            "DELETE FROM settings WHERE key = ? AND value = ?",
            (core_rates.MARKET_RATE_PROVIDER_SETTING, core_rates.RATE_SOURCE_MEMPOOL),
        )
        conn.commit()
finally:
    conn.close()
PY
}

demo_write_lightning_cli_wrapper() {
  mkdir -p "$DEMO_HOME/lightning"
  local wrapper="$DEMO_HOME/lightning/lightning-cli-merchant-demo.sh"
  KASSIBER_DEMO_LIGHTNING_CLI_WRAPPER="$wrapper" \
  KASSIBER_DEMO_REPO_ROOT="$(pwd)" \
  KASSIBER_DEMO_COMPOSE_PROJECT="$KASSIBER_REGTEST_COMPOSE_PROJECT" \
    py - <<'PY'
import os
from pathlib import Path

wrapper = Path(os.environ["KASSIBER_DEMO_LIGHTNING_CLI_WRAPPER"])
repo_root = os.environ["KASSIBER_DEMO_REPO_ROOT"]
compose_project = os.environ["KASSIBER_DEMO_COMPOSE_PROJECT"]
target = Path(repo_root) / "dev" / "regtest" / "lightning-cli-merchant.sh"
content = f"""#!/usr/bin/env bash
set -euo pipefail
export KASSIBER_REGTEST_COMPOSE_PROJECT="${{KASSIBER_REGTEST_COMPOSE_PROJECT:-{compose_project}}}"
exec {target} "$@"
"""
tmp = wrapper.with_suffix(wrapper.suffix + ".tmp")
tmp.write_text(content, encoding="utf-8")
tmp.chmod(0o700)
os.replace(tmp, wrapper)
wrapper.chmod(0o700)
print(wrapper)
PY
}

demo_lnd_readonly_macaroon_hex() {
  docker_compose_regtest exec -T lnd_merchant_backup \
    cat /root/.lnd/data/chain/bitcoin/regtest/readonly.macaroon \
    | od -An -tx1 -v | tr -d ' \n'
  printf '\n'
}

demo_seed_lightning() {
  if ! demo_lightning_enabled; then
    echo "Lightning demo disabled via KASSIBER_REGTEST_DEMO_LIGHTNING=0."
    return 0
  fi
  demo_configure_lightning
  local merchant_cli
  merchant_cli="$(demo_write_lightning_cli_wrapper)"
  export KASSIBER_LIGHTNING_BUSINESS=1
  export KASSIBER_LIGHTNING_BUSINESS_HOME="${KASSIBER_LIGHTNING_BUSINESS_HOME:-$DEMO_HOME/lightning}"
  export KASSIBER_LIGHTNING_BUSINESS_DATA_ROOT="$DEMO_HOME/data"
  export KASSIBER_LIGHTNING_BUSINESS_WORKSPACE="Regtest Demo"
  export KASSIBER_LIGHTNING_BUSINESS_PROFILE="Full Accounting"
  export KASSIBER_LIGHTNING_BUSINESS_CONNECTION_LABEL="${KASSIBER_LIGHTNING_BUSINESS_CONNECTION_LABEL:-cln_merchant}"
  export KASSIBER_LIGHTNING_BUSINESS_BACKEND_NAME="${KASSIBER_LIGHTNING_BUSINESS_BACKEND_NAME:-cln-merchant}"
  export KASSIBER_LIGHTNING_BUSINESS_EMBEDDED=1
  export KASSIBER_LIGHTNING_BUSINESS_REUSE_BOOK=1
  export KASSIBER_LIGHTNING_BUSINESS_PLAN="${KASSIBER_LIGHTNING_BUSINESS_PLAN:-$KASSIBER_LIGHTNING_BUSINESS_HOME/business-plan.json}"
  export KASSIBER_LIGHTNING_BUSINESS_MERCHANT_CLI="$merchant_cli"
  ./dev/regtest/lightning-business-bootstrap.sh
  export KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_URL="https://127.0.0.1:$KASSIBER_REGTEST_LND_BACKUP_REST_PORT"
  export KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_MACAROON_HEX
  KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_MACAROON_HEX="$(demo_lnd_readonly_macaroon_hex)"
  ./dev/regtest/lightning-business-scenario.sh
  py -m tests.integration.lightning_business_regtest >/dev/null
  # Real lightningd/lnd stamp settle times at wall-clock "now"; spread the
  # imported Lightning history across the on-chain scenario window so the demo
  # ledger shows years of activity instead of a single burst. Demo-only, and it
  # must run AFTER sync but BEFORE journals process so journals re-derive over
  # the backdated timestamps. Node channels/balances stay "now" (live snapshot).
  export KASSIBER_DEMO_BACKDATE_SCENARIO="$DEMO_SCENARIO"
  export KASSIBER_DEMO_BACKDATE_SEED="${KASSIBER_DEMO_BACKDATE_SEED:-0}"
  py -m tests.integration.lightning_demo_backdate >/dev/null
  py -m kassiber \
    --data-root "$DEMO_HOME/data" \
    --machine \
    journals process \
    --workspace "Regtest Demo" \
    --profile "Full Accounting" >/dev/null
}

demo_build_book() {
  local checksum
  local current_checksum
  checksum="$(demo_scenario_checksum)"
  current_checksum="$(demo_manifest_get scenario_checksum)"
  demo_assert_safe_home rebuild
  if [ -z "${KASSIBER_REGTEST_DEMO_REBUILD:-}" ] \
    && [ -d "$DEMO_HOME/data" ] \
    && [ "$current_checksum" = "$checksum" ]; then
    demo_refresh_live_rate
    demo_seed_lightning
    demo_write_manifest "$checksum"
    echo "Reusing existing demo book (scenario unchanged): $DEMO_HOME/data"
    return 0
  fi

  if [ -z "${KASSIBER_REGTEST_REUSE_CORE:-}" ] && [ -z "${KASSIBER_REGTEST_DEMO_CHAIN_RESET_DONE:-}" ]; then
    echo "Resetting the managed demo regtest chain for a backdated rebuild..."
    docker_compose_regtest down -v --remove-orphans
    docker_compose_regtest up -d
    wait_for_core
    wait_for_elements
  fi

  demo_assert_safe_home rebuild
  rm -rf "$DEMO_HOME/data" "$DEMO_HOME/exports" "$DEMO_HOME/imports" "$DEMO_HOME/lightning" \
    "$DEMO_HOME/demo-summary.json" "$DEMO_MANIFEST"
  mkdir -p "$DEMO_HOME"
  echo "Building the demo book (a few minutes of regtest history)..."
  KASSIBER_REGTEST_DEMO_ROOT="$DEMO_HOME" py -m tests.integration.regtest_demo \
    --keep-core-wallets \
    --no-business-tick \
    --json-output "$DEMO_HOME/demo-summary.json" >/dev/null

  demo_refresh_live_rate
  demo_seed_lightning
  demo_write_manifest "$checksum"
}

demo_print_instructions() {
  cat <<EOF

Demo environment is up.
  data root:  $DEMO_HOME/data
  exports:    $DEMO_HOME/exports
  Core RPC:   $KASSIBER_REGTEST_CORE_URL (regtest; credentials in $DEMO_MANIFEST)
  Elements RPC: http://127.0.0.1:$KASSIBER_REGTEST_ELEMENTS_RPC_PORT (elementsregtest; same credentials)
  BTC Electrum: tcp://127.0.0.1:$KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT
  BTC Frigate:  tcp://127.0.0.1:$KASSIBER_REGTEST_FRIGATE_PORT (Silent Payments Electrum)
  BTC mempool:  http://127.0.0.1:$KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT/api
  LBTC Electrum: tcp://127.0.0.1:$KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT
  LBTC mempool:  http://127.0.0.1:$KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT/api
$(if demo_lightning_enabled; then cat <<EOF_LIGHTNING
  CLN merchant: cln_merchant (Core Lightning; port $KASSIBER_REGTEST_CLN_MERCHANT_PORT)
  LND backup:   lnd_merchant_backup (LND; REST https://127.0.0.1:$KASSIBER_REGTEST_LND_BACKUP_REST_PORT)
  CLN peers:    cln_customer, cln_supplier, cln_router
EOF_LIGHTNING
else cat <<EOF_LIGHTNING_DISABLED
  Lightning:    disabled (KASSIBER_REGTEST_DEMO_LIGHTNING=0)
EOF_LIGHTNING_DISABLED
fi)

Next steps:
  cd ui-tauri && pnpm dev:demo                       # dev preview on the real demo book
  uv run python -m kassiber --data-root "$DEMO_HOME/data" reports summary
  ./dev/regtest/bitcoin-cli.sh getblockchaininfo    # poke the regtest node
  ./scripts/integration-harness.sh demo-down         # stop the node (book + chain kept)
  ./scripts/integration-harness.sh demo-down --purge # remove node, chain, and demo book
EOF
}

run_demo_up() {
  # The dev demo stack is machine-global (one demo book per developer), so use
  # a fixed Compose project instead of the per-worktree test project, keep the
  # node running after the script exits, and persist the generated RPC
  # credentials so later demo-up runs still match the book's stored backend.
  export KASSIBER_REGTEST_COMPOSE_PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-kassiber-regtest-demo}"
  export KASSIBER_REGTEST_KEEP=1
  export KASSIBER_REGTEST_COMPOSE_PROFILES="${KASSIBER_REGTEST_COMPOSE_PROFILES:-silent-payments}"
  export COMPOSE_PROFILES="$KASSIBER_REGTEST_COMPOSE_PROFILES"
  export KASSIBER_REGTEST_REQUIRE_ELEMENTS=1
  demo_configure_lightning
  demo_load_rpc_env
  if demo_book_needs_rebuild && [ -z "${KASSIBER_REGTEST_REUSE_CORE:-}" ]; then
    echo "Removing the managed demo regtest chain before rebuilding the backdated book..."
    docker_compose_regtest down -v --remove-orphans
    export KASSIBER_REGTEST_DEMO_CHAIN_RESET_DONE=1
  fi
  run_with_bitcoin_core demo_build_book
  demo_print_instructions
}

run_demo_tick() {
  local count="${1:-1}"
  if [ ! -f "$DEMO_MANIFEST" ] || [ ! -f "$DEMO_HOME/demo-summary.json" ]; then
    echo "No demo book at $DEMO_HOME. Run './scripts/integration-harness.sh demo-up' first." >&2
    exit 2
  fi
  export KASSIBER_INTEGRATION=1
  export KASSIBER_REGTEST_CORE_URL="${KASSIBER_REGTEST_CORE_URL:-$(demo_manifest_get core_url)}"
  export KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-$(demo_manifest_get rpc_user)}"
  export KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-$(demo_manifest_get rpc_password)}"
  if ! probe_core 0; then
    echo "Demo regtest node is not reachable at $KASSIBER_REGTEST_CORE_URL." >&2
    echo "Start it with './scripts/integration-harness.sh demo-up'." >&2
    exit 2
  fi
  py -m tests.integration.regtest_demo \
    --tick \
    --summary "$DEMO_HOME/demo-summary.json" \
    --tick-count "$count"
  echo
  echo "New business activity is confirmed on the demo node."
  echo "Refresh/sync in the app (or run it directly) to import it:"
  echo "  uv run python -m kassiber --data-root \"$DEMO_HOME/data\" wallets sync --all"
}

run_demo_down() {
  local purge="${1:-}"
  local project
  project="$(demo_manifest_get compose_project)"
  project="${KASSIBER_REGTEST_COMPOSE_PROJECT:-${project:-kassiber-regtest-demo}}"
  export KASSIBER_REGTEST_COMPOSE_PROJECT="$project"
  export KASSIBER_REGTEST_COMPOSE_PROFILES="${KASSIBER_REGTEST_COMPOSE_PROFILES:-silent-payments}"
  export COMPOSE_PROFILES="$KASSIBER_REGTEST_COMPOSE_PROFILES"
  demo_load_rpc_env
  if [ "$purge" = "--purge" ]; then
    demo_assert_safe_home purge
    KASSIBER_REGTEST_USE_LIGHTNING_COMPOSE=1 docker_compose_regtest down -v
    rm -rf "$DEMO_HOME"
    echo "Demo node, chain volume, and demo book removed."
  else
    KASSIBER_REGTEST_USE_LIGHTNING_COMPOSE=1 docker_compose_regtest down
    echo "Demo node stopped. Chain volume and demo book kept; 'demo-up' resumes them."
  fi
}

run_bitcoin_core() {
  run_with_bitcoin_core run_bitcoin_backend_suite
}

run_bitcoin_electrum() {
  run_with_bitcoin_core run_bitcoin_electrum_parity_smoke
}

run_regtest_demo_full() {
  export KASSIBER_REGTEST_REQUIRE_ELEMENTS=1
  run_with_bitcoin_core run_demo_full
}

port_is_free() {
  KASSIBER_CANDIDATE_PORT="$1" py - <<'PY'
import os
import socket
import sys

port = int(os.environ["KASSIBER_CANDIDATE_PORT"])
sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
try:
    sock.bind(("127.0.0.1", port))
except OSError:
    sys.exit(1)
finally:
    sock.close()
sys.exit(0)
PY
}

regtest_ports_available() {
  local base="$1"
  local ports=(
    "$base"
    "$((base + 100))"
    "$((base + 101))"
    "$((base + 102))"
    "$((base + 103))"
    "$((base + 104))"
    "$((base + 105))"
  )
  local port
  for port in "${ports[@]}"; do
    if ! port_is_free "$port"; then
      return 1
    fi
  done
  return 0
}

choose_regtest_base_port() {
  local candidate
  for candidate in 18443 19443 20443 21443 22443; do
    if regtest_ports_available "$candidate"; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  echo "No free regtest port family found." >&2
  echo "Set KASSIBER_REGTEST_RPC_PORT to an available base port and rerun." >&2
  exit 2
}

lightning_ports_available() {
  local base="$1"
  if ! regtest_ports_available "$base"; then
    return 1
  fi

  local ports=(
    "$((base + 1292))"
    "$((base + 1293))"
    "$((base + 1294))"
    "$((base + 1295))"
    "$((base + 1296))"
    "$((base + 1297))"
    "$((base + 1298))"
  )
  local port
  for port in "${ports[@]}"; do
    if ! port_is_free "$port"; then
      return 1
    fi
  done
  return 0
}

choose_lightning_base_port() {
  local candidate
  for candidate in 18443 19443 20443 21443 22443; do
    if lightning_ports_available "$candidate"; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  echo "No free regtest port family found for the Lightning business lane." >&2
  echo "Set KASSIBER_REGTEST_RPC_PORT to an available base port and rerun." >&2
  exit 2
}

run_lightning_business() {
  ensure_python_runtime
  export KASSIBER_INTEGRATION=1
  export KASSIBER_LIGHTNING_BUSINESS=1
  export KASSIBER_REGTEST_USE_LIGHTNING_COMPOSE=1
  export KASSIBER_REGTEST_RPC_USER="${KASSIBER_REGTEST_RPC_USER:-kassiber}"
  export KASSIBER_REGTEST_RPC_PASSWORD="${KASSIBER_REGTEST_RPC_PASSWORD:-$(py -c 'import secrets; print(secrets.token_urlsafe(24))')}"
  export KASSIBER_REGTEST_RPC_AUTH="${KASSIBER_REGTEST_RPC_AUTH:-$(rpc_auth)}"
  export KASSIBER_REGTEST_RPC_PORT="${KASSIBER_REGTEST_RPC_PORT:-$(choose_lightning_base_port)}"
  export KASSIBER_REGTEST_ELEMENTS_RPC_PORT="${KASSIBER_REGTEST_ELEMENTS_RPC_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 104))}"
  export KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT="${KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 100))}"
  export KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT="${KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 101))}"
  export KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT="${KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 102))}"
  export KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT="${KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT:-$((KASSIBER_REGTEST_RPC_PORT + 103))}"
  configure_lightning_ports
  export KASSIBER_REGTEST_CORE_URL="${KASSIBER_REGTEST_CORE_URL:-http://127.0.0.1:${KASSIBER_REGTEST_RPC_PORT}}"
  export KASSIBER_REGTEST_COMPOSE_PROJECT="${KASSIBER_REGTEST_COMPOSE_PROJECT:-$(py -c 'import hashlib, os; print("kassiber-regtest-" + hashlib.sha256(os.getcwd().encode()).hexdigest()[:12])')}"
  export KASSIBER_LIGHTNING_BUSINESS_HOME="${KASSIBER_LIGHTNING_BUSINESS_HOME:-${TMPDIR:-/tmp}/kassiber-lightning-business-${KASSIBER_REGTEST_COMPOSE_PROJECT}}"
  export KASSIBER_LIGHTNING_BUSINESS_PLAN="${KASSIBER_LIGHTNING_BUSINESS_PLAN:-${KASSIBER_LIGHTNING_BUSINESS_HOME}/business-plan.json}"
  export KASSIBER_REGTEST_LIGHTNING_SEED="${KASSIBER_REGTEST_LIGHTNING_SEED:-kassiber-lightning-business-v1}"
  export KASSIBER_REGTEST_LIGHTNING_CAPACITY_MULTIPLIER="${KASSIBER_REGTEST_LIGHTNING_CAPACITY_MULTIPLIER:-0.35}"

  cleanup_lightning() {
    if [ "$STARTED_COMPOSE" -eq 1 ] && [ -z "${KASSIBER_REGTEST_KEEP:-}" ]; then
      docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" \
        -f dev/regtest/compose.bitcoin.yml \
        -f dev/regtest/compose.lightning.yml \
        down -v
      rm -rf "$KASSIBER_LIGHTNING_BUSINESS_HOME"
    fi
  }
  trap cleanup_lightning EXIT

  if [ -z "${KASSIBER_REGTEST_KEEP:-}" ] && [ -z "${KASSIBER_REGTEST_LIGHTNING_REUSE:-}" ]; then
    rm -rf "$KASSIBER_LIGHTNING_BUSINESS_HOME"
  fi

  STARTED_COMPOSE=0
  if [ -z "${KASSIBER_REGTEST_LIGHTNING_REUSE:-}" ]; then
    STARTED_COMPOSE=1
    if ! docker_compose -p "$KASSIBER_REGTEST_COMPOSE_PROJECT" \
      -f dev/regtest/compose.bitcoin.yml \
      -f dev/regtest/compose.lightning.yml \
      up -d; then
      echo "Failed to start the regtest Bitcoin + Core Lightning stack." >&2
      echo "If a host port is already taken, set KASSIBER_REGTEST_RPC_PORT or" >&2
      echo "the KASSIBER_REGTEST_CLN_*_PORT variables before rerunning." >&2
      exit 1
    fi
  fi

  wait_for_core
  py dev/regtest/lightning-business-plan.py --output "$KASSIBER_LIGHTNING_BUSINESS_PLAN"
  ./dev/regtest/lightning-business-bootstrap.sh
  export KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_URL="https://127.0.0.1:$KASSIBER_REGTEST_LND_BACKUP_REST_PORT"
  export KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_MACAROON_HEX
  KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_MACAROON_HEX="$(demo_lnd_readonly_macaroon_hex)"
  ./dev/regtest/lightning-business-scenario.sh
  py -m unittest tests.integration.test_live_lightning_business_regtest -v
}

run_silent_payments() {
  export KASSIBER_REGTEST_COMPOSE_PROFILES="${KASSIBER_REGTEST_COMPOSE_PROFILES:-silent-payments}"
  run_with_bitcoin_core run_silent_payments_smoke
}

boltz_regtest_dir() {
  if [ -n "${KASSIBER_BOLTZ_REGTEST_DIR:-}" ]; then
    printf '%s\n' "$KASSIBER_BOLTZ_REGTEST_DIR"
  else
    printf '%s\n' "${XDG_CACHE_HOME:-$HOME/.cache}/kassiber/boltz-regtest"
  fi
}

ensure_boltz_regtest_dir() {
  local dir="$1"
  if [ -x "$dir/start.sh" ] && [ -x "$dir/stop.sh" ]; then
    return 0
  fi
  if [ -n "${KASSIBER_BOLTZ_REGTEST_AUTO_CLONE:-}" ]; then
    mkdir -p "$(dirname "$dir")"
    git clone --depth=1 https://github.com/BoltzExchange/regtest "$dir"
    return 0
  fi
  cat >&2 <<EOF
Boltz's upstream regtest checkout is required for this lane.

Either clone it yourself and point Kassiber at it:
  git clone https://github.com/BoltzExchange/regtest "$dir"
  KASSIBER_BOLTZ_REGTEST_DIR="$dir" ./scripts/integration-harness.sh boltz-liquid

Or let the lane clone it into the cache path above:
  KASSIBER_BOLTZ_REGTEST_AUTO_CLONE=1 ./scripts/integration-harness.sh boltz-liquid
EOF
  exit 2
}

wait_for_boltz_liquid() {
  local deadline
  deadline=$((SECONDS + 180))
  until py -m tests.integration.boltz_liquid_regtest >/dev/null 2>&1
  do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for Boltz Liquid regtest API at ${KASSIBER_BOLTZ_API_URL}." >&2
      py -m tests.integration.boltz_liquid_regtest --json || true
      return 1
    fi
    sleep 3
  done
}

host_port_in_use() {
  KASSIBER_BOLTZ_HOST_PORT="$1" py - <<'PY'
import os
import socket
import sys

port = int(os.environ["KASSIBER_BOLTZ_HOST_PORT"])
sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
try:
    sock.bind(("127.0.0.1", port))
except OSError:
    sys.exit(0)
finally:
    sock.close()
sys.exit(1)
PY
}

boltz_patch_host_ports() {
  local dir="$1"
  local port="${KASSIBER_BOLTZ_BITCOIN_RPC_PORT:-}"
  BOLTZ_COMPOSE_FILE="$dir/docker-compose.yml"
  if [ -z "$port" ] && host_port_in_use 18443; then
    port=19443
  fi
  if [ -z "$port" ] || [ "$port" = "18443" ]; then
    return 0
  fi
  BOLTZ_COMPOSE_TEMP="$(mktemp "${TMPDIR:-/tmp}/kassiber-boltz-compose.XXXXXX.yml")"
  KASSIBER_BOLTZ_REGTEST_DIR="$dir" \
  KASSIBER_BOLTZ_BITCOIN_RPC_PORT="$port" \
  KASSIBER_BOLTZ_COMPOSE_TEMP="$BOLTZ_COMPOSE_TEMP" \
    py - <<'PY'
import os
import re
from pathlib import Path

source = Path(os.environ["KASSIBER_BOLTZ_REGTEST_DIR"]) / "docker-compose.yml"
target = Path(os.environ["KASSIBER_BOLTZ_COMPOSE_TEMP"])
text = source.read_text(encoding="utf-8")
patched = re.sub(
    r"(?m)^(\s*-\s*)(\d+):18443\s*$",
    rf"\g<1>{os.environ['KASSIBER_BOLTZ_BITCOIN_RPC_PORT']}:18443",
    text,
    count=1,
)
target.write_text(patched, encoding="utf-8")
PY
  BOLTZ_COMPOSE_FILE="$BOLTZ_COMPOSE_TEMP"
  echo "Boltz regtest host bitcoind RPC port mapped to $port -> 18443 to avoid local conflicts."
}

boltz_docker_compose() {
  local dir="$1"
  shift
  local uid gid
  uid="$(id -u)"
  gid="$(id -g)"
  if [ -z "$BOLTZ_COMPOSE_FILE" ]; then
    BOLTZ_COMPOSE_FILE="$dir/docker-compose.yml"
  fi
  if docker info >/dev/null 2>&1; then
    env UID="$uid" GID="$gid" docker compose \
      --project-directory "$dir" \
      -f "$BOLTZ_COMPOSE_FILE" \
      "$@"
  elif sudo -n docker info >/dev/null 2>&1; then
    export KASSIBER_BOLTZ_DOCKER_CMD="${KASSIBER_BOLTZ_DOCKER_CMD:-sudo -n docker}"
    sudo -n env UID="$uid" GID="$gid" docker compose \
      --project-directory "$dir" \
      -f "$BOLTZ_COMPOSE_FILE" \
      "$@"
  else
    echo "Docker access is required for the Boltz Liquid lane." >&2
    echo "Add the current user to the docker group or allow passwordless sudo docker." >&2
    exit 2
  fi
}

boltz_regtest_start() {
  local dir="$1"
  boltz_docker_compose "$dir" down --volumes
  boltz_docker_compose "$dir" up --remove-orphans -d
}

boltz_regtest_stop() {
  local dir="$1"
  boltz_docker_compose "$dir" down --volumes --remove-orphans -t 0
}

run_boltz_liquid() {
  local dir
  dir="$(boltz_regtest_dir)"
  export KASSIBER_BOLTZ_API_URL="${KASSIBER_BOLTZ_API_URL:-http://127.0.0.1:9001}"
  export KASSIBER_BOLTZ_WS_URL="${KASSIBER_BOLTZ_WS_URL:-ws://127.0.0.1:9004}"
  export KASSIBER_BOLTZ_REGTEST=1

  cleanup_boltz() {
    if [ "$STARTED_BOLTZ" -eq 1 ] && [ -z "${KASSIBER_BOLTZ_REGTEST_KEEP:-}" ]; then
      boltz_regtest_stop "$dir"
    fi
    if [ -n "$BOLTZ_COMPOSE_TEMP" ]; then
      rm -f "$BOLTZ_COMPOSE_TEMP"
    fi
  }
  trap cleanup_boltz EXIT

  if [ -z "${KASSIBER_BOLTZ_REGTEST_REUSE:-}" ]; then
    ensure_boltz_regtest_dir "$dir"
    boltz_patch_host_ports "$dir"
    STARTED_BOLTZ=1
    boltz_regtest_start "$dir"
  elif ! docker info >/dev/null 2>&1 && sudo -n docker info >/dev/null 2>&1; then
    export KASSIBER_BOLTZ_DOCKER_CMD="${KASSIBER_BOLTZ_DOCKER_CMD:-sudo -n docker}"
  fi

  wait_for_boltz_liquid
  py -m unittest tests.integration.test_boltz_liquid_regtest -v
}

case "$MODE" in
  fast)
    run_fast
    ;;
  bitcoin-core|slow)
    run_bitcoin_core
    ;;
  bitcoin-electrum)
    run_bitcoin_electrum
    ;;
  demo|demo-full)
    run_regtest_demo_full
    ;;
  demo-up)
    run_demo_up
    ;;
  demo-tick)
    run_demo_tick "${2:-1}"
    ;;
  demo-down)
    run_demo_down "${2:-}"
    ;;
  boltz-liquid)
    run_boltz_liquid
    ;;
  lightning-business)
    run_lightning_business
    ;;
  silent-payments)
    run_silent_payments
    ;;
  all)
    run_fast
    # Keep the generated-truth demo on a fresh chain. The backend parity tests
    # mine enough blocks to push the backdated accounting scenario out of range
    # if demo-full reuses the same disposable regtest volume.
    ( run_bitcoin_core )
    ( run_regtest_demo_full )
    ;;
  *)
    echo "usage: $0 [fast|bitcoin-core|bitcoin-electrum|slow|demo|demo-full|demo-up|demo-tick [N]|demo-down [--purge]|boltz-liquid|lightning-business|silent-payments|all]" >&2
    exit 2
    ;;
esac
