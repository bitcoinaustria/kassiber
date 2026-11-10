"""Probe a regtest BTCPay Server's Greenfield permissions for Kassiber's keys.

Starts bitcoind and the BTCPay overlay under its own Compose project, seeds an
admin, a store with a generated hot wallet and a few wallet transactions, and
reports, for one BTCPay version:

- `GET /misc/permissions` (the inclusion tree `kassiber/btcpay/permissions.py`
  mirrors);
- which wallet endpoints each permission set can read;
- what the browser authorize page does with every permission Kassiber's
  presets request, including ones the server does not know.

Usage (from the repository root, Docker available to this user):

    python3 -I dev/regtest/btcpay_permission_probe.py up --btcpay 2.4.7
    python3 -I dev/regtest/btcpay_permission_probe.py probe > probe.json
    python3 -I dev/regtest/btcpay_permission_probe.py down

`up` prints the base URL and seeded secrets to the state file; `probe` reads
them. The project name (`--project`) and ports keep the stack apart from the
demo harness. Nothing here touches a Kassiber data root.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import html.parser
import http.cookiejar
import json
import os
import re
import secrets
import subprocess
import sys
import time
from pathlib import Path
from typing import Any
from urllib import error, parse, request

ROOT = Path(__file__).resolve().parents[2]
NBXPLORER = {"2.3": "nicolasdorier/nbxplorer:2.5.29", "2.4": "nicolasdorier/nbxplorer:2.6.21"}
ADMIN = ("probe@example.com", "probe-password-1")
PERMISSION_SETS = {
    "view_store_settings": ["btcpay.store.canviewstoresettings"],
    "view_store_settings_and_wallet": ["btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet"],
    "view_wallet": ["btcpay.store.canviewwallet"],
    "modify_store_settings": ["btcpay.store.canmodifystoresettings"],
}


def _state_file(args) -> Path:
    return Path(args.state).resolve()


def _env(args, state: dict[str, Any]) -> dict[str, str]:
    env = dict(os.environ)
    env.pop("XDG_DATA_HOME", None)
    env.update(
        KASSIBER_REGTEST_RPC_USER=state["rpc_user"],
        KASSIBER_REGTEST_RPC_PASSWORD=state["rpc_password"],
        KASSIBER_REGTEST_RPC_AUTH=state["rpc_auth"],
        KASSIBER_REGTEST_RPC_PORT=str(state["rpc_port"]),
        KASSIBER_REGTEST_BTCPAY_PORT=str(state["btcpay_port"]),
        KASSIBER_REGTEST_BTCPAY_NBXPLORER_PORT=str(state["nbxplorer_port"]),
        KASSIBER_REGTEST_BTCPAY_IMAGE=f"btcpayserver/btcpayserver:{state['btcpay']}",
        KASSIBER_REGTEST_BTCPAY_NBXPLORER_IMAGE=NBXPLORER[".".join(state["btcpay"].split(".")[:2])],
    )
    return env


def _compose(args, state, *command: str) -> None:
    subprocess.run(
        ["docker", "compose", "-p", state["project"], "-f", "dev/regtest/compose.bitcoin.yml",
         "-f", "dev/regtest/compose.btcpay.yml", *command],
        cwd=ROOT, env=_env(args, state), check=True,
    )


def _rpc(state, method: str, params: list[Any] | None = None, wallet: str | None = None) -> Any:
    url = f"http://127.0.0.1:{state['rpc_port']}/" + (f"wallet/{wallet}" if wallet else "")
    body = json.dumps({"jsonrpc": "1.0", "id": "probe", "method": method, "params": params or []}).encode()
    auth = base64.b64encode(f"{state['rpc_user']}:{state['rpc_password']}".encode()).decode()
    req = request.Request(url, data=body, headers={"Authorization": f"Basic {auth}", "Content-Type": "application/json"})
    try:
        # Mining the first 101 blocks can take minutes on a loaded host.
        with request.urlopen(req, timeout=300) as response:
            return json.loads(response.read())["result"]
    except error.HTTPError as exc:
        raise RuntimeError(exc.read().decode(errors="replace")) from exc


def _api(base: str, method: str, path: str, *, body=None, token=None, basic=None, ok=(200,)):
    headers = {"Accept": "application/json"}
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"token {token}"
    if basic:
        headers["Authorization"] = "Basic " + base64.b64encode(f"{basic[0]}:{basic[1]}".encode()).decode()
    req = request.Request(base + path, data=data, headers=headers, method=method)
    try:
        with request.urlopen(req, timeout=60) as response:
            raw = response.read().decode()
            return response.status, (json.loads(raw) if raw.strip() else None)
    except error.HTTPError as exc:
        raw = exc.read().decode(errors="replace")
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            payload = raw[:300]
        return exc.code, payload


def up(args) -> None:
    password = secrets.token_hex(16)
    salt = secrets.token_hex(16)
    digest = hmac.new(salt.encode(), password.encode(), hashlib.sha256).hexdigest()
    state = {
        "project": args.project, "btcpay": args.btcpay, "rpc_user": "probe", "rpc_password": password,
        "rpc_auth": f"probe:{salt}${digest}", "rpc_port": args.rpc_port,
        "btcpay_port": args.btcpay_port, "nbxplorer_port": args.btcpay_port + 1,
    }
    _state_file(args).write_text(json.dumps(state, indent=2))
    _compose(args, state, "up", "-d", "bitcoind", "btcpay-postgres", "btcpay-nbxplorer", "btcpayserver")
    base = f"http://127.0.0.1:{state['btcpay_port']}"
    deadline = time.monotonic() + 300
    while True:
        try:
            if _rpc(state, "getblockchaininfo") is not None:
                break
        except Exception:
            if time.monotonic() > deadline:
                raise
            time.sleep(2)
    if "probe" not in (_rpc(state, "listwallets") or []):
        _rpc(state, "createwallet", ["probe"])
    miner = _rpc(state, "getnewaddress", [], wallet="probe")
    _rpc(state, "generatetoaddress", [101, miner])
    while True:
        try:
            status, _ = _api(base, "GET", "/api/v1/health")
        except OSError:  # refused or timed out while BTCPay starts
            status = None
        if status == 200:
            break
        if time.monotonic() > deadline:
            raise RuntimeError("BTCPay did not start")
        time.sleep(3)
    status, payload = _api(base, "POST", "/api/v1/users", body={"email": ADMIN[0], "password": ADMIN[1], "isAdministrator": True}, ok=(200, 201))
    if status not in (200, 201):
        raise RuntimeError(f"admin: {status} {payload}")
    status, store = _api(base, "POST", "/api/v1/stores", body={"name": "Probe store", "defaultCurrency": "EUR"}, basic=ADMIN)
    store_id = store["id"]
    while True:
        status, payload = _api(base, "POST", f"/api/v1/stores/{store_id}/payment-methods/BTC-CHAIN/wallet/generate",
                               body={"accountNumber": 0, "savePrivateKeys": False, "wordList": "English",
                                     "wordCount": 12, "scriptPubKeyType": "Segwit"}, basic=ADMIN)
        if status == 200:
            break
        if status != 503 or time.monotonic() > deadline:
            raise RuntimeError(f"wallet: {status} {payload}")
        time.sleep(3)
    for amount in ("0.01", "0.02"):
        status, address = _api(base, "GET", f"/api/v1/stores/{store_id}/payment-methods/BTC-CHAIN/wallet/address?forceGenerate=true", basic=ADMIN)
        _rpc(state, "sendtoaddress", [address["address"], amount], wallet="probe")
    _rpc(state, "generatetoaddress", [1, miner])
    state.update(base=base, store_id=store_id, server_info=_api(base, "GET", "/api/v1/server/info", basic=ADMIN)[1])
    _state_file(args).write_text(json.dumps(state, indent=2))
    print(json.dumps({"base": base, "store_id": store_id, "version": (state["server_info"] or {}).get("version")}))


class _Form(html.parser.HTMLParser):
    """The inputs of every form on a page, plus visible permission labels."""

    def __init__(self):
        super().__init__()
        self.forms: list[dict[str, Any]] = []
        self.text: list[str] = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "form":
            self.forms.append({"action": attrs.get("action"), "inputs": []})
        elif tag in {"input", "button"} and self.forms:
            self.forms[-1]["inputs"].append(attrs)

    def handle_data(self, data):
        if data.strip():
            self.text.append(data.strip())


def _session(base: str):
    jar = http.cookiejar.CookieJar()
    opener = request.build_opener(request.HTTPCookieProcessor(jar))

    def fetch(path, data=None):
        body = None if data is None else parse.urlencode(data, doseq=True).encode()
        try:
            with opener.open(request.Request(base + path, data=body), timeout=60) as response:
                return response.status, response.geturl(), response.read().decode(errors="replace")
        except error.HTTPError as exc:
            return exc.code, base + path, exc.read().decode(errors="replace")

    return fetch


def _authorize(state, permissions: list[str], scope_all: bool) -> dict[str, Any]:
    """Drive the browser authorize page: what it shows, and the key it makes."""
    fetch = _session(state["base"])
    _, _, page = fetch("/login")
    form = _Form()
    form.feed(page)
    token = next(i.get("value") for f in form.forms for i in f["inputs"] if i.get("name") == "__RequestVerificationToken")
    # BTCPay 2.4's login form names its method; 2.3 ignores the field.
    fetch("/login", {"Email": ADMIN[0], "Password": ADMIN[1], "Method": "Password",
                     "__RequestVerificationToken": token})
    query = [("permissions", p) for p in permissions] + [
        ("applicationName", "Kassiber probe"), ("strict", "true"), ("selectiveStores", "false" if scope_all else "true")]
    status, url, page = fetch("/api-keys/authorize?" + parse.urlencode(query))
    form = _Form()
    form.feed(page)
    shown = sorted({i.get("value") for f in form.forms for i in f["inputs"]
                    if (i.get("name") or "").endswith(".Permission") and i.get("value")})
    result: dict[str, Any] = {"requested": permissions, "status": status, "page": parse.urlsplit(url).path,
                              "shown_permissions": shown}
    approve = next((f for f in form.forms if any(i.get("value") == "Authorize" for i in f["inputs"])), None)
    if approve is None:
        result["error_text"] = " ".join(form.text)[-400:]
        return result
    fields = [(i["name"], i.get("value", "")) for i in approve["inputs"]
              if i.get("name") and (i.get("type") not in {"checkbox", "submit"} or "checked" in i)]
    if not scope_all and not any(name == "StoreId" for name, _ in fields):
        fields.append(("StoreId", state["store_id"]))
    fields.append(("command", "Authorize"))
    status, url, page = fetch(approve["action"] or "/api-keys/authorize", fields)
    result["approved_status"] = status
    for key in dict.fromkeys(re.findall(r"\b[0-9a-f]{40}\b", page)):
        code, current = _api(state["base"], "GET", "/api/v1/api-keys/current", token=key)
        if code == 200:
            result["granted_permissions"] = sorted((current or {}).get("permissions", []))
            break
    else:
        done = _Form()
        done.feed(page)
        result["approve_text"] = " ".join(done.text)[-300:]
    return result


def probe(args) -> None:
    state = json.loads(_state_file(args).read_text())
    base, store = state["base"], state["store_id"]
    _, tree = _api(base, "GET", "/misc/permissions", basic=ADMIN)
    endpoints = {
        "preview": f"/api/v1/stores/{store}/payment-methods/BTC-CHAIN/wallet/preview?offset=0&count=3",
        "transactions": f"/api/v1/stores/{store}/payment-methods/BTC-CHAIN/wallet/transactions?statusFilter=Confirmed",
        "payment_methods": f"/api/v1/stores/{store}/payment-methods?onlyEnabled=true",
    }
    keys = {}
    for name, permissions in PERMISSION_SETS.items():
        status, created = _api(base, "POST", "/api/v1/api-keys", basic=ADMIN,
                               body={"label": name, "permissions": [f"{p}:{store}" for p in permissions]})
        entry: dict[str, Any] = {"create_status": status}
        if status == 200:
            entry["granted_permissions"] = sorted(created.get("permissions", []))
            token = created["apiKey"]
            for endpoint, path in endpoints.items():
                code, payload = _api(base, "GET", path, token=token)
                entry[endpoint] = code if code != 403 else {"status": 403, "body": payload}
        else:
            entry["error"] = created
        keys[name] = entry
    authorize = {
        "read_only_all_stores": _authorize(state, ["btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet"], True),
        "wallet_history_single_store": _authorize(state, ["btcpay.store.canviewwallet"], False),
    }
    print(json.dumps({"version": (state.get("server_info") or {}).get("version"), "misc_permissions": tree,
                      "keys": keys, "authorize": authorize}, indent=2, sort_keys=True))


def down(args) -> None:
    state = json.loads(_state_file(args).read_text())
    _compose(args, state, "down", "-v", "--remove-orphans")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("command", choices=("up", "probe", "down"))
    parser.add_argument("--btcpay", default="2.4.7")
    parser.add_argument("--project", default="kassiber-btcpay-permission-probe")
    parser.add_argument("--state", default="btcpay-permission-probe.json")
    parser.add_argument("--rpc-port", type=int, default=28443)
    parser.add_argument("--btcpay-port", type=int, default=28549)
    args = parser.parse_args()
    {"up": up, "probe": probe, "down": down}[args.command](args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
