"""Regtest stack for the Bark (Ark) BTCPay plugin, to record its real API answers.

Usage (from the repository root, Docker available to this user):

    python3 -I dev/regtest/btcpay_bark_stack.py up --plugin 0.13.0 --state S.json \\
        --plugin-package DIR/BTCPayServer.Plugins.Bark.btcpay
    python3 -I dev/regtest/btcpay_bark_stack.py scenario --state S.json
    python3 -I dev/regtest/btcpay_bark_stack.py record --state S.json --out OUT
    python3 -I dev/regtest/btcpay_bark_stack.py down --state S.json

`up` refuses to start with under 40 GiB of available memory or when one of its
ports (28600-28602 by default, `--base-port`) is taken. It never downloads the
plugin: fetch the package from the BTCPay plugin builder yourself (the URL and
expected sha256 are in PLUGINS below, printed when the package is missing) and
pass it with `--plugin-package`, or put it at
`$KASSIBER_BARK_PLUGIN_CACHE/<version>/BTCPayServer.Plugins.Bark.btcpay`.
`upgrade --plugin 0.13.0` swaps the plugin of a running stack, to see how
invoices from a pre-0.10 plugin (payment method `BARK`) read after upgrading.
The fixtures in tests/fixtures/btcpay_bark/ came from the harness lane
(0.13.0) and, for the old plugin, from `up --plugin 0.9.6 --project
kassiber-bark-old-... --base-port 28610`, `scenario --allow-failures --steps
...`, `record --label 0.9.6`, `upgrade --plugin 0.13.0`, `record --label
0.9.6-upgraded-to-0.13.0`. `scripts/integration-harness.sh btcpay-bark` runs
up, scenario, record and down in one go. A scenario takes about 10 minutes.

The stack: bitcoind, an Esplora electrs, captaind 0.8.1 (Ark server) with its
CLN + hold plugin, an external CLN with a channel to the server's CLN, a
counterparty `bark` CLI wallet, and BTCPay 2.4.7 with the plugin mounted. All of
it runs under its own Compose project on an internal Docker network without
internet access; only a TCP forwarder publishes bitcoind RPC, BTCPay and
Esplora on 127.0.0.1. Nothing here touches a Kassiber data root. The state file
holds the stack's credentials; keep it in a scratch directory. Recorded
fixtures are redacted (API keys, passwords, connection-string secrets).
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import html
import html.parser
import http.cookiejar
import json
import os
import re
import secrets
import shutil
import socket
import stat
import subprocess
import sys
import time
import zipfile
from pathlib import Path
from typing import Any, Callable
from urllib import error, parse, request

ROOT = Path(__file__).resolve().parents[2]
COMPOSE_FILES = ("dev/regtest/compose.bitcoin.yml", "dev/regtest/compose.btcpay.yml", "dev/regtest/compose.bark.yml")
DEFAULT_PROJECT = "kassiber-bark"
BTCPAY_IMAGE = "btcpayserver/btcpayserver:2.4.7"
NBXPLORER_IMAGE = "nicolasdorier/nbxplorer:2.6.21"
# captaind 0.8.1 refuses bitcoind older than v31.
BITCOIND_IMAGE = "bitcoin/bitcoin:31.0"
MIN_AVAILABLE_GIB = 40
BUILDER = "https://btcpaypluginbuilder.blob.core.windows.net/artifacts/bark/{build}/BTCPayServer.Plugins.Bark.btcpay"
PLUGINS = {
    # BTCPay plugin builder builds; both need BTCPay >= 2.4.4.
    "0.13.0": {"build": 16, "sha256": "10494a6bf81604f4462d6c0d8d4ba0b79f05bd9d46add5f5a5a2c420cd5789b9", "bark": "0.8.0"},
    "0.9.6": {"build": 10, "sha256": "125d18916a1a02b6cc8542aeedd9a01f0006d266a5e247cbcd2aa5f369b8e2c4", "bark": "0.7.1"},
}
PLUGIN_NAME = "BTCPayServer.Plugins.Bark"
ARK_SERVER = "http://captaind:3535"
ESPLORA = "http://electrs:3002"
KEYS = {
    # name -> permissions (unscoped: they apply to every store the admin owns)
    "admin": ["unrestricted"],
    "read_only": ["btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet"],
    "settings_only": ["btcpay.store.canviewstoresettings"],
    "modify_settings": ["btcpay.store.canmodifystoresettings"],
    "invoices": ["btcpay.store.canviewinvoices"],
    "payouts": ["btcpay.store.canviewpayouts"],
    "lightning": ["btcpay.store.canviewlightninginvoice", "btcpay.store.canuselightningnode"],
}
SECRET_KEYS = re.compile(r"(api_?key|password|secret|macaroon|rune|mnemonic|cookie|token|seed|privatekey|capability)", re.I)
SECRET_VALUES = re.compile(r"(?i)\b(capability|macaroon|rune|api-token|apikey|password|secret|key)=([^;&\s\"]+)")


# --------------------------------------------------------------------------- utilities


def log(message: str) -> None:
    print(f"[bark-stack {time.strftime('%H:%M:%S')}] {message}", file=sys.stderr, flush=True)


def wait_for(what: str, check: Callable[[], Any], timeout: float = 300, interval: float = 2) -> Any:
    deadline = time.monotonic() + timeout
    last: Exception | None = None
    while True:
        try:
            result = check()
            if result:
                return result
        except Exception as exc:  # noqa: BLE001 - polling until the deadline
            last = exc
        if time.monotonic() > deadline:
            raise RuntimeError(f"timed out waiting for {what}" + (f": {last}" if last else ""))
        time.sleep(interval)


def available_gib() -> float:
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) / 1024 / 1024
    raise RuntimeError("no MemAvailable in /proc/meminfo")


def port_free(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        try:
            sock.bind(("127.0.0.1", port))
        except OSError:
            return False
    return True


def load_state(path: str) -> dict[str, Any]:
    return json.loads(Path(path).read_text())


def save_state(path: str, state: dict[str, Any]) -> None:
    target = Path(path)
    target.write_text(json.dumps(state, indent=2, sort_keys=True))
    target.chmod(0o600)


# --------------------------------------------------------------------------- docker compose


def compose_env(state: dict[str, Any]) -> dict[str, str]:
    env = dict(os.environ)
    env.pop("XDG_DATA_HOME", None)
    env.update(
        KASSIBER_REGTEST_RPC_USER=state["rpc_user"],
        KASSIBER_REGTEST_RPC_PASSWORD=state["rpc_password"],
        KASSIBER_REGTEST_RPC_AUTH=state["rpc_auth"],
        KASSIBER_REGTEST_RPC_PORT=str(state["rpc_port"]),
        KASSIBER_REGTEST_BTCPAY_PORT=str(state["btcpay_port"]),
        KASSIBER_REGTEST_BARK_ESPLORA_PORT=str(state["esplora_port"]),
        KASSIBER_REGTEST_BTCPAY_NBXPLORER_PORT=str(state["esplora_port"] + 1),
        KASSIBER_REGTEST_BITCOIND_IMAGE=BITCOIND_IMAGE,
        KASSIBER_REGTEST_BTCPAY_IMAGE=BTCPAY_IMAGE,
        KASSIBER_REGTEST_BTCPAY_NBXPLORER_IMAGE=NBXPLORER_IMAGE,
        KASSIBER_REGTEST_BARK_PLUGIN_DIR=state["plugin_dir"],
    )
    return env


def compose(state: dict[str, Any], *args: str, capture: bool = False, check: bool = True,
            timeout: float | None = None, input_text: str | None = None) -> subprocess.CompletedProcess:
    command = ["docker", "compose", "-p", state["project"]]
    for file in COMPOSE_FILES:
        command += ["-f", file]
    return subprocess.run(
        command + list(args), cwd=ROOT, env=compose_env(state), check=check, timeout=timeout,
        capture_output=capture, text=True, input=input_text,
    )


def sh(state: dict[str, Any], service: str, *argv: str, timeout: float = 300, check: bool = True) -> str:
    result = compose(state, "exec", "-T", service, *argv, capture=True, check=False, timeout=timeout)
    if check and result.returncode != 0:
        raise RuntimeError(f"{service} {' '.join(argv[:3])}: exit {result.returncode}: "
                           f"{(result.stderr or result.stdout).strip()[-600:]}")
    return result.stdout


def sh_json(state: dict[str, Any], service: str, *argv: str, timeout: float = 300) -> Any:
    out = sh(state, service, *argv, timeout=timeout)
    return json.loads(out) if out.strip() else None


# --------------------------------------------------------------------------- bitcoind / chain


def rpc(state: dict[str, Any], method: str, params: list[Any] | None = None, wallet: str | None = None) -> Any:
    url = f"http://127.0.0.1:{state['rpc_port']}/" + (f"wallet/{wallet}" if wallet else "")
    body = json.dumps({"jsonrpc": "1.0", "id": "bark", "method": method, "params": params or []}).encode()
    auth = base64.b64encode(f"{state['rpc_user']}:{state['rpc_password']}".encode()).decode()
    req = request.Request(url, data=body, headers={"Authorization": f"Basic {auth}", "Content-Type": "application/json"})
    try:
        with request.urlopen(req, timeout=300) as response:
            return json.loads(response.read())["result"]
    except error.HTTPError as exc:
        raise RuntimeError(f"{method}: {exc.read().decode(errors='replace')[:300]}") from exc


def esplora_height(state: dict[str, Any]) -> int:
    with request.urlopen(f"http://127.0.0.1:{state['esplora_port']}/blocks/tip/height", timeout=10) as response:
        return int(response.read())


def cln(state: dict[str, Any], node: str, *args: str, timeout: float = 300) -> Any:
    out = sh(state, node, "lightning-cli", "--network=regtest", *args, timeout=timeout)
    # `pay` prints progress lines starting with "#" before its JSON.
    return json.loads("\n".join(line for line in out.splitlines() if not line.startswith("#")))


def mine(state: dict[str, Any], blocks: int = 1) -> int:
    rpc(state, "generatetoaddress", [blocks, state["miner_address"]])
    height = rpc(state, "getblockcount")
    wait_for("electrs tip", lambda: esplora_height(state) >= height, timeout=180, interval=0.5)
    for node in ("ark-cln", "ext-cln"):
        wait_for(f"{node} tip", lambda n=node: cln(state, n, "getinfo")["blockheight"] >= height, timeout=180, interval=1)
    return height


def captaind_wallet(state: dict[str, Any]) -> dict[str, Any]:
    return sh_json(state, "captaind", "captaind", "rpc", "wallet")["rounds"]


def bark(state: dict[str, Any], *args: str, timeout: float = 300) -> Any:
    out = sh(state, "bark-peer", "bark", "-q", *args, timeout=timeout)
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return out.strip()


# --------------------------------------------------------------------------- BTCPay HTTP


def api(state: dict[str, Any], method: str, path: str, *, body: Any = None, key: str | None = "admin",
        basic: tuple[str, str] | None = None, timeout: float = 120) -> tuple[int, Any]:
    headers = {"Accept": "application/json"}
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    if basic:
        headers["Authorization"] = "Basic " + base64.b64encode(f"{basic[0]}:{basic[1]}".encode()).decode()
    elif key:
        headers["Authorization"] = f"token {state['keys'][key]}"
    req = request.Request(state["base"] + path, data=data, headers=headers, method=method)
    try:
        with request.urlopen(req, timeout=timeout) as response:
            raw = response.read().decode()
            return response.status, (json.loads(raw) if raw.strip() else None)
    except error.HTTPError as exc:
        raw = exc.read().decode(errors="replace")
        try:
            return exc.code, json.loads(raw)
        except json.JSONDecodeError:
            return exc.code, raw[:500]


def api_ok(state: dict[str, Any], method: str, path: str, **kwargs) -> Any:
    status, payload = api(state, method, path, **kwargs)
    if status not in (200, 201):
        raise RuntimeError(f"{method} {path}: {status} {payload}")
    return payload


class Forms(html.parser.HTMLParser):
    """Every form's action and inputs, plus the page's visible text."""

    def __init__(self) -> None:
        super().__init__()
        self.forms: list[dict[str, Any]] = []
        self.text: list[str] = []
        self._skip = 0

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag in ("script", "style"):
            self._skip += 1
        if tag == "form":
            self.forms.append({"action": attrs.get("action") or "", "inputs": []})
        elif tag in {"input", "button", "select", "textarea"} and self.forms:
            attrs["_tag"] = tag
            self.forms[-1]["inputs"].append(attrs)

    def handle_endtag(self, tag):
        if tag in ("script", "style") and self._skip:
            self._skip -= 1

    def handle_data(self, data):
        if data.strip() and not self._skip:
            self.text.append(data.strip())


def parse_page(page: str) -> Forms:
    forms = Forms()
    forms.feed(page)
    return forms


class Session:
    """A cookie session on the BTCPay UI, logged in as the admin."""

    def __init__(self, state: dict[str, Any]):
        self.state = state
        self.jar = http.cookiejar.CookieJar()
        self.opener = request.build_opener(request.HTTPCookieProcessor(self.jar))
        self.token: str | None = None
        _, _, page = self.fetch("/login")
        token = self._token(page)
        status, url, page = self.fetch("/login", {
            "Email": state["admin_email"], "Password": state["admin_password"], "Method": "Password",
            "__RequestVerificationToken": token})
        if "/login" in parse.urlsplit(url).path:
            raise RuntimeError("BTCPay UI login failed: " + " ".join(parse_page(page).text)[-300:])

    @staticmethod
    def _token(page: str) -> str:
        match = re.search(r'name="__RequestVerificationToken"[^>]*value="([^"]+)"', page)
        if not match:
            raise RuntimeError("no antiforgery token on page")
        return match.group(1)

    def fetch(self, path: str, data: Any = None, *, json_body: Any = None, headers: dict[str, str] | None = None,
              timeout: float = 180) -> tuple[int, str, str]:
        body = None
        hdrs = dict(headers or {})
        if json_body is not None:
            body = json.dumps(json_body).encode()
            hdrs["Content-Type"] = "application/json"
        elif data is not None:
            body = parse.urlencode(data, doseq=True).encode()
        req = request.Request(self.state["base"] + path, data=body, headers=hdrs)
        try:
            with self.opener.open(req, timeout=timeout) as response:
                return response.status, response.geturl(), response.read().decode(errors="replace")
        except error.HTTPError as exc:
            return exc.code, self.state["base"] + path, exc.read().decode(errors="replace")

    def page_token(self, path: str) -> str:
        _, _, page = self.fetch(path)
        return self._token(page)

    def submit(self, path: str, action_contains: str, overrides: dict[str, Any], drop: tuple[str, ...] = ()) -> tuple[int, str, str]:
        """GET a page, fill its form whose action contains `action_contains`, POST it."""
        _, _, page = self.fetch(path)
        parsed = parse_page(page)
        # Forms without an action post back to the page itself.
        form = next((f for f in parsed.forms if action_contains in f["action"]
                     or (not f["action"] and any(i.get("name") in overrides for i in f["inputs"]))), None)
        if form is None:
            raise RuntimeError(f"no form {action_contains!r} on {path}: " + " ".join(parsed.text)[-400:])
        fields: list[tuple[str, str]] = []
        for item in form["inputs"]:
            name = item.get("name")
            kind = (item.get("type") or "").lower()
            if not name or name in overrides or name in drop or kind in {"submit", "button"} or item["_tag"] == "button":
                continue
            if kind in {"checkbox", "radio"} and "checked" not in item:
                continue
            if item["_tag"] == "select":
                continue
            fields.append((name, item.get("value", "")))
        for name, value in overrides.items():
            for single in value if isinstance(value, list) else [value]:
                fields.append((name, "true" if single is True else "false" if single is False else str(single)))
        action = html.unescape(form["action"]) or path
        return self.fetch(action, fields)

    def post_json(self, path: str, token_page: str, body: Any) -> tuple[int, Any]:
        token = self.page_token(token_page)
        status, _, raw = self.fetch(path, json_body=body, headers={"RequestVerificationToken": token,
                                                                    "Accept": "application/json"})
        try:
            return status, json.loads(raw)
        except json.JSONDecodeError:
            return status, raw[:300]


def page_messages(page: str) -> list[str]:
    """The status messages (TempData alerts and validation errors) on a page."""
    found = []
    for match in re.finditer(r'<div[^>]*class="[^"]*(alert|validation-summary|text-danger)[^"]*"[^>]*>(.*?)</div>', page, re.S):
        text = re.sub(r"<[^>]+>", " ", match.group(2))
        text = " ".join(html.unescape(text).split())
        if text and "{{" not in text and not text.startswith("BTCPay is expecting you"):
            found.append(text)
    for match in re.finditer(r'<span[^>]*class="[^"]*field-validation-error[^"]*"[^>]*>(.*?)</span>', page, re.S):
        text = " ".join(html.unescape(re.sub(r"<[^>]+>", " ", match.group(1))).split())
        if text:
            found.append(text)
    return found


# --------------------------------------------------------------------------- barkd inside the BTCPay container

BARKD_CALL = r"""
set -e
tok=$(cat "$1")
exec 3<>/dev/tcp/127.0.0.1/$2
body="$5"
printf '%s %s HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer %s\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s' "$3" "$4" "$tok" "${#body}" "$body" >&3
cat <&3
"""


def barkd_location(state: dict[str, Any]) -> tuple[str, int]:
    store = state["store_id"]
    logs = compose(state, "logs", "--no-color", "btcpayserver", capture=True).stdout
    ports = re.findall(rf"bark\.barkd_started store_id={store} outcome=success duration_ms=\d+ port=(\d+)", logs)
    if not ports:
        raise RuntimeError("barkd has not started for the store yet")
    if not state.get("barkd_datadir"):
        found = sh(state, "btcpayserver", "sh", "-c", f"ls -d /datadir/bark/stores/{store} /root/.btcpayserver/*/bark/stores/{store} 2>/dev/null; true")
        state["barkd_datadir"] = found.split()[0]
    return state["barkd_datadir"], int(ports[-1])


def barkd(state: dict[str, Any], method: str, path: str, body: Any = None, timeout: float = 300) -> tuple[int, Any]:
    datadir, port = barkd_location(state)
    payload = "" if body is None else json.dumps(body)
    raw = sh(state, "btcpayserver", "bash", "-c", BARKD_CALL, "barkd", f"{datadir}/auth_token", str(port), method, path,
             payload, timeout=timeout)
    # Text-mode subprocess output has its CRLFs turned into LFs already.
    head, _, rest = raw.replace("\r\n", "\n").partition("\n\n")
    status = int(head.split()[1])
    if "transfer-encoding: chunked" in head.lower():
        decoded, chunks = "", rest
        while chunks:
            size_line, _, chunks = chunks.partition("\n")
            size = int(size_line.strip() or "0", 16)
            if size == 0:
                break
            decoded += chunks[:size]
            chunks = chunks[size + 1:]
        rest = decoded
    try:
        return status, json.loads(rest) if rest.strip() else None
    except json.JSONDecodeError:
        return status, rest[:500]


# --------------------------------------------------------------------------- up


def plugin_package(args) -> Path:
    if args.plugin_package:
        return Path(args.plugin_package).resolve()
    cache = os.environ.get("KASSIBER_BARK_PLUGIN_CACHE")
    if cache:
        return Path(cache).resolve() / args.plugin / f"{PLUGIN_NAME}.btcpay"
    meta = PLUGINS[args.plugin]
    raise SystemExit(f"pass --plugin-package or set KASSIBER_BARK_PLUGIN_CACHE; download it from "
                     f"{BUILDER.format(build=meta['build'])} (sha256 {meta['sha256']})")


def install_plugin(version: str, package: Path, workdir: Path) -> Path:
    """Verify the builder package and unpack it as BTCPAY_PLUGINDIR/<plugin>/."""
    meta = PLUGINS[version]
    if not package.is_file():
        raise SystemExit(f"plugin package {package} missing; download {BUILDER.format(build=meta['build'])}")
    digest = hashlib.sha256(package.read_bytes()).hexdigest()
    if digest != meta["sha256"]:
        raise SystemExit(f"plugin package sha256 {digest} != builder hash {meta['sha256']}")
    plugin_root = workdir / f"plugins-{version}"
    if plugin_root.exists():
        shutil.rmtree(plugin_root)
    target = plugin_root / PLUGIN_NAME
    target.mkdir(parents=True)
    with zipfile.ZipFile(package) as archive:
        archive.extractall(target)  # zipfile strips absolute and parent paths
    for path in [plugin_root, *plugin_root.rglob("*")]:
        mode = path.stat().st_mode
        extra = stat.S_IRGRP | stat.S_IROTH
        if path.is_dir() or "Native" in path.parts:
            extra |= stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        path.chmod(mode | extra)
    return plugin_root


def preflight(state: dict[str, Any]) -> None:
    gib = available_gib()
    if gib < MIN_AVAILABLE_GIB:
        raise SystemExit(f"only {gib:.0f} GiB memory available, need {MIN_AVAILABLE_GIB}")
    taken = [p for p in (state["rpc_port"], state["btcpay_port"], state["esplora_port"]) if not port_free(p)]
    if taken:
        raise SystemExit(f"ports already in use: {taken}")


def up(args) -> None:
    workdir = Path(args.state).resolve().parent / f"{args.project}.d"
    workdir.mkdir(parents=True, exist_ok=True)
    password = secrets.token_hex(16)
    salt = secrets.token_hex(16)
    digest = hmac.new(salt.encode(), password.encode(), hashlib.sha256).hexdigest()
    state: dict[str, Any] = {
        "project": args.project, "plugin": args.plugin, "workdir": str(workdir),
        "rpc_user": "bark", "rpc_password": password, "rpc_auth": f"bark:{salt}${digest}",
        "rpc_port": args.base_port, "btcpay_port": args.base_port + 1, "esplora_port": args.base_port + 2,
        "admin_email": "bark-admin@example.com", "admin_password": secrets.token_urlsafe(18),
        "steps": [], "snapshots": [],
    }
    state["base"] = f"http://127.0.0.1:{state['btcpay_port']}"
    preflight(state)
    state["plugin_dir"] = str(install_plugin(args.plugin, plugin_package(args), workdir))
    save_state(args.state, state)

    log("starting bitcoind")
    compose(state, "up", "-d", "bitcoind", "edge-forward")
    wait_for("bitcoind RPC", lambda: rpc(state, "getblockchaininfo"), timeout=300)
    if "miner" not in (rpc(state, "listwallets") or []):
        rpc(state, "createwallet", ["miner"])
    state["miner_address"] = rpc(state, "getnewaddress", [], wallet="miner")
    rpc(state, "generatetoaddress", [101, state["miner_address"]])
    save_state(args.state, state)

    log("starting electrs, CLN nodes, captaind, BTCPay")
    compose(state, "up", "-d", "electrs", "ark-cln", "ext-cln", "captaind", "bark-peer",
            "btcpay-postgres", "btcpay-nbxplorer", "btcpayserver")
    height = rpc(state, "getblockcount")
    wait_for("electrs", lambda: esplora_height(state) >= height, timeout=300)
    for node in ("ark-cln", "ext-cln"):
        wait_for(node, lambda n=node: cln(state, n, "getinfo")["blockheight"] >= height, timeout=300, interval=3)

    log("funding captaind and opening ext-cln -> ark-cln channel")
    rounds = wait_for("captaind admin RPC", lambda: captaind_wallet(state), timeout=300, interval=3)
    rpc(state, "sendtoaddress", [rounds["address"], 10], wallet="miner")
    ext_address = cln(state, "ext-cln", "newaddr")["bech32"]
    rpc(state, "sendtoaddress", [ext_address, 2], wallet="miner")
    mine(state, 1)
    wait_for("ext-cln funds", lambda: cln(state, "ext-cln", "listfunds")["outputs"], timeout=120)
    ark_id = cln(state, "ark-cln", "getinfo")["id"]
    cln(state, "ext-cln", "connect", f"{ark_id}@ark-cln:9735")
    cln(state, "ext-cln", "fundchannel", f"id={ark_id}", "amount=10000000", "push_msat=4000000000")
    mine(state, 6)
    wait_for("channel normal", lambda: any(ch.get("state") == "CHANNELD_NORMAL"
                                           for ch in cln(state, "ext-cln", "listpeerchannels")["channels"]),
             timeout=180, interval=2)
    wait_for("captaind trusted balance", lambda: captaind_wallet(state)["trusted_balance"] > 0, timeout=120)

    log("waiting for BTCPay")
    wait_for("BTCPay health", lambda: api(state, "GET", "/api/v1/health", key=None)[0] == 200, timeout=600, interval=3)
    admin = (state["admin_email"], state["admin_password"])
    api_ok(state, "POST", "/api/v1/users", key=None,
           body={"email": admin[0], "password": admin[1], "isAdministrator": True})
    state["store_id"] = api_ok(state, "POST", "/api/v1/stores", basic=admin,
                               body={"name": "Bark store", "defaultCurrency": "BTC"})["id"]
    state["plain_store_id"] = api_ok(state, "POST", "/api/v1/stores", basic=admin,
                                     body={"name": "Store without Bark", "defaultCurrency": "BTC"})["id"]
    # Keys before the UI login: a UI login can turn off Greenfield Basic auth for the user.
    state["keys"] = {}
    for name, permissions in KEYS.items():
        created = api_ok(state, "POST", "/api/v1/api-keys", basic=admin, body={"label": name, "permissions": permissions})
        state["keys"][name] = created["apiKey"]
    save_state(args.state, state)

    log("setting up Bark through the plugin UI")
    session = Session(state)
    store = state["store_id"]
    status, url, page = session.submit(f"/stores/{store}/plugins/bark/setup", "/plugins/bark/setup", {
        "PaymentsEnabled": True, "WalletSource": "New",
        "BarkServerAddress": ARK_SERVER, "BitcoinNodeAddress": ESPLORA})
    if "/setup" in parse.urlsplit(url).path:
        raise RuntimeError("Bark setup failed: " + "; ".join(page_messages(page)))
    state["setup"] = {"status": status, "landed_on": parse.urlsplit(url).path, "messages": page_messages(page)}
    wait_for("barkd", lambda: barkd_location(state), timeout=300, interval=3)
    # No Branta lookups (they could not leave the internal network anyway).
    status, url, page = session.submit(f"/stores/{store}/plugins/bark/settings", "/plugins/bark/settings", {
        "PaymentsEnabled": True, "BrantaVerificationEnabled": False, "RefreshOnReceive": False})
    state["settings"] = {"status": status, "messages": page_messages(page)}
    save_state(args.state, state)

    log("creating and funding the counterparty bark wallet")
    sh(state, "bark-peer", "bark", "-q", "create", "--regtest", "--ark", ARK_SERVER, "--esplora", ESPLORA, timeout=300)
    peer_address = bark(state, "onchain", "address")
    peer_address = peer_address["address"] if isinstance(peer_address, dict) else peer_address
    rpc(state, "sendtoaddress", [peer_address, "0.05"], wallet="miner")
    mine(state, 1)
    bark(state, "board", "3000000 sats")
    mine(state, 2)
    wait_for("peer board spendable", lambda: peer_spendable(state) >= 2_000_000, timeout=300, interval=5)
    save_state(args.state, state)
    print(json.dumps({"base": state["base"], "store_id": store, "plugin": args.plugin}))


def peer_spendable(state: dict[str, Any]) -> int:
    balance = bark(state, "balance")
    return int(balance.get("spendable_sat", 0)) if isinstance(balance, dict) else 0


# --------------------------------------------------------------------------- scenario


def wallet_rows(state: dict[str, Any]) -> tuple[int, Any]:
    return api(state, "GET", f"/api/v1/stores/{state['store_id']}/payment-methods/BTC-ARK/wallet/transactions", key="read_only")


def snapshot(state: dict[str, Any], label: str) -> None:
    status, rows = wallet_rows(state)
    state["snapshots"].append({"label": label, "status": status, "body": rows, "height": rpc(state, "getblockcount")})


def count_kind(state: dict[str, Any], kind: str) -> int:
    status, rows = wallet_rows(state)
    return sum(1 for r in rows if r.get("kind") == kind) if status == 200 else -1


def wait_new_row(state: dict[str, Any], kind: str, before: int) -> None:
    if before < 0:  # no wallet API (plugin before 0.10.0): give barkd time to sync
        time.sleep(20)
        return
    wait_for(f"new {kind} row", lambda: count_kind(state, kind) > before, timeout=180, interval=5)


def invoice_settled(state: dict[str, Any], invoice_id: str) -> bool:
    invoice = api_ok(state, "GET", f"/api/v1/stores/{state['store_id']}/invoices/{invoice_id}")
    return invoice["status"] == "Settled"


def create_invoice(state: dict[str, Any], amount: str, order: str, methods: list[str] | None = None) -> dict[str, Any]:
    body: dict[str, Any] = {"amount": amount, "currency": "BTC", "metadata": {"orderId": order}}
    if methods:
        body["checkout"] = {"paymentMethods": methods}
    invoice = api_ok(state, "POST", f"/api/v1/stores/{state['store_id']}/invoices", body=body)
    prompts = api_ok(state, "GET", f"/api/v1/stores/{state['store_id']}/invoices/{invoice['id']}/payment-methods")
    invoice["prompts"] = {p["paymentMethodId"]: p for p in prompts}
    return invoice


def ark_method(invoice: dict[str, Any]) -> dict[str, Any]:
    for method_id in ("BTC-ARK", "BARK"):
        if method_id in invoice["prompts"]:
            return invoice["prompts"][method_id]
    raise RuntimeError(f"invoice has no Ark prompt: {sorted(invoice['prompts'])}")


def store_addresses(state: dict[str, Any], session: Session, amount_sat: int = 0) -> dict[str, Any]:
    store = state["store_id"]
    status, payload = session.post_json(f"/stores/{store}/plugins/bark/receive/addresses",
                                        f"/stores/{store}/plugins/bark/receive", {"amountSat": amount_sat, "arkOnly": False})
    if status != 200:
        raise RuntimeError(f"receive/addresses: {status} {payload}")
    return payload


def store_send(state: dict[str, Any], session: Session, destination: str, amount_sat: int | None, source: str,
               labels: list[str] | None = None) -> list[str]:
    store = state["store_id"]
    overrides: dict[str, Any] = {"Destination": destination, "SendSource": source}
    if amount_sat is not None:
        overrides["AmountSat"] = amount_sat
    if labels:
        overrides["SelectedLabels"] = labels
    _, _, page = session.submit(f"/stores/{store}/plugins/bark/send", "/plugins/bark/send", overrides,
                                drop=("FiatAmount",))
    return page_messages(page)


def vtxos(state: dict[str, Any]) -> list[dict[str, Any]]:
    status, payload = barkd(state, "GET", "/api/v1/wallet/vtxos")
    if status != 200:
        raise RuntimeError(f"barkd vtxos: {status} {payload}")
    return payload


def spendable_vtxos(state: dict[str, Any]) -> list[dict[str, Any]]:
    return [v for v in vtxos(state) if str(v.get("state", "")).lower() in {"spendable", "\"spendable\""}
            or (isinstance(v.get("state"), dict) and v["state"].get("type") == "spendable")]


def vtxo_action(state: dict[str, Any], session: Session, ids: list[str], command: str, address: str | None = None) -> list[str]:
    store = state["store_id"]
    token = session.page_token(f"/stores/{store}/plugins/bark/vtxos")
    fields: list[tuple[str, str]] = [("selectedItems", i) for i in ids] + [("command", command),
                                                                            ("__RequestVerificationToken", token)]
    if address:
        fields.append(("offboardAddress", address))
    _, _, page = session.fetch(f"/stores/{store}/plugins/bark/vtxos/action", fields)
    return page_messages(page)


def ark_balance(state: dict[str, Any]) -> dict[str, Any]:
    status, payload = barkd(state, "GET", "/api/v1/wallet/balance")
    return payload if status == 200 else {}


def scenario(args) -> None:
    state = load_state(args.state)
    session = Session(state)
    store = state["store_id"]
    steps = args.steps.split(",") if args.steps else STEP_ORDER

    def run(name: str, action: Callable[[], dict[str, Any]]) -> None:
        if name not in steps:
            return
        log(f"step {name}")
        started = time.time()
        try:
            result = action() or {}
            outcome = "ok"
        except Exception as exc:  # noqa: BLE001 - every step is best effort and recorded
            result, outcome = {"error": f"{type(exc).__name__}: {exc}"[:800]}, "failed"
            log(f"step {name} failed: {result['error']}")
        state["steps"].append({"step": name, "outcome": outcome, "seconds": round(time.time() - started), **result})
        this_run.append((name, outcome))
        save_state(args.state, state)

    this_run: list[tuple[str, str]] = []

    def onchain_receive():
        addresses = store_addresses(state, session)
        txids = [rpc(state, "sendtoaddress", [addresses["onchainAddress"], amount], wallet="miner")
                 for amount in ("0.02", "0.005")]
        snapshot(state, "onchain-receive in mempool")
        mine(state, 1)
        wait_for("on-chain balance", lambda: (ark_balance(state).get("onchain") or {}) or True, timeout=60)
        return {"address_kind": "bcrt1" if addresses["onchainAddress"].startswith("bcrt1") else "other", "txids": txids,
                "amounts_btc": ["0.02", "0.005"]}

    def board():
        def try_board():
            status, payload = barkd(state, "POST", "/api/v1/boards/board-amount", {"amount_sat": 1_000_000})
            if status != 200:
                raise RuntimeError(f"{status} {payload}")
            return payload
        payload = wait_for("board", try_board, timeout=120, interval=10)
        snapshot(state, "board broadcast, unconfirmed")
        mine(state, 2)
        wait_for("board vtxo spendable", lambda: spendable_vtxos(state), timeout=180, interval=5)
        return {"barkd": payload}

    def ark_receive_invoice():
        invoice = create_invoice(state, "0.002", "ark-invoice")
        destination = ark_method(invoice)["destination"]
        bark(state, "send", destination, "200000 sats")
        wait_for("Ark invoice settled", lambda: invoice_settled(state, invoice["id"]), timeout=180, interval=3)
        state.setdefault("paid_ark_invoices", []).append(invoice["id"])
        return {"invoice": invoice["id"], "methods": sorted(invoice["prompts"]), "ark_destination_prefix": destination[:6]}

    def ark_receive_plain():
        addresses = store_addresses(state, session)
        before = count_kind(state, "ark-receive")
        bark(state, "send", addresses["arkAddress"], "50000 sats")
        wait_new_row(state, "ark-receive", before)
        return {"ark_address_prefix": addresses["arkAddress"][:6], "amount_sat": 50000}

    def lightning_receive_invoice():
        invoice = create_invoice(state, "0.0015", "lightning-on-ark-invoice")
        bolt11 = invoice["prompts"]["BTC-LN"]["destination"]
        paid = cln(state, "ext-cln", "pay", bolt11, timeout=300)
        wait_for("LN invoice settled", lambda: invoice_settled(state, invoice["id"]), timeout=180, interval=3)
        return {"invoice": invoice["id"], "methods": sorted(invoice["prompts"]), "bolt11_prefix": bolt11[:8],
                "payment_hash": paid.get("payment_hash"), "paid_msat": paid.get("amount_sent_msat")}

    def lightning_receive_ln_only():
        invoice = create_invoice(state, "0.0012", "ln-only-invoice", ["BTC-LN"])
        bolt11 = invoice["prompts"]["BTC-LN"]["destination"]
        paid = cln(state, "ext-cln", "pay", bolt11, timeout=300)
        wait_for("LN-only invoice settled", lambda: invoice_settled(state, invoice["id"]), timeout=180, interval=3)
        return {"invoice": invoice["id"], "methods": sorted(invoice["prompts"]), "payment_hash": paid.get("payment_hash")}

    def lightning_receive_plain():
        addresses = store_addresses(state, session, amount_sat=30000)
        before = count_kind(state, "lightning-receive")
        paid = cln(state, "ext-cln", "pay", addresses["bolt11"], timeout=300)
        wait_new_row(state, "lightning-receive", before)
        return {"payment_hash": paid.get("payment_hash"), "amount_sat": 30000}

    def ark_send():
        address = bark(state, "address")
        address = address["address"] if isinstance(address, dict) else address
        messages = store_send(state, session, address, 40000, "ark", labels=["kassiber-ark-send"])
        return {"messages": messages, "amount_sat": 40000}

    def lightning_send():
        label = f"ln-send-{secrets.token_hex(4)}"
        bolt11 = cln(state, "ext-cln", "invoice", "25000000", label, "kassiber lightning send")["bolt11"]
        messages = store_send(state, session, bolt11, None, "ark")
        paid = wait_for("ext-cln invoice paid", lambda: cln(state, "ext-cln", "listinvoices", label)["invoices"][0]["status"] == "paid",
                        timeout=180, interval=3)
        return {"messages": messages, "amount_sat": 25000, "paid": bool(paid)}

    def lightning_api_pay():
        label = f"ln-api-{secrets.token_hex(4)}"
        bolt11 = cln(state, "ext-cln", "invoice", "15000000", label, "greenfield lightning pay")["bolt11"]
        status, payload = api(state, "POST", f"/api/v1/stores/{store}/lightning/BTC/invoices/pay", body={"BOLT11": bolt11})
        return {"status": status, "response": redact(payload)}

    def payout():
        label = f"payout-{secrets.token_hex(4)}"
        bolt11 = cln(state, "ext-cln", "invoice", "12000000", label, "kassiber payout")["bolt11"]
        processor = api(state, "PUT", f"/api/v1/stores/{store}/payout-processors/LightningAutomatedPayoutSenderFactory/BTC-LN",
                        body={"intervalSeconds": 60, "processNewPayoutsInstantly": True, "cancelPayoutAfterFailures": 5})
        created = api_ok(state, "POST", f"/api/v1/stores/{store}/payouts",
                         body={"destination": bolt11, "amount": "0.00012", "payoutMethodId": "BTC-LN", "approved": True})
        done = wait_for("payout completed", lambda: api_ok(state, "GET", f"/api/v1/stores/{store}/payouts/{created['id']}")["state"]
                        in {"Completed", "InProgress"} and cln(state, "ext-cln", "listinvoices", label)["invoices"][0]["status"] == "paid",
                        timeout=240, interval=5)
        return {"processor_status": processor[0], "payout": created["id"], "completed": bool(done)}

    def refund():
        invoice_id = (state.get("paid_ark_invoices") or [None])[0]
        if not invoice_id:
            raise RuntimeError("no paid Ark invoice to refund")
        status, pull = api(state, "POST", f"/api/v1/stores/{store}/invoices/{invoice_id}/refund",
                           body={"refundVariant": "Custom", "customAmount": "0.0001", "customCurrency": "BTC",
                                 "paymentMethod": "BTC-LN", "payoutMethodId": "BTC-LN"})
        if status != 200:
            raise RuntimeError(f"refund: {status} {pull}")
        label = f"refund-{secrets.token_hex(4)}"
        # BTCPay wants a payout BOLT11 to stay valid for 30 days.
        bolt11 = cln(state, "ext-cln", "invoice", "10000000", label, "kassiber refund", str(31 * 86400))["bolt11"]
        claimed = api_ok(state, "POST", f"/api/v1/pull-payments/{pull['id']}/payouts", key=None,
                         body={"destination": bolt11, "payoutMethodId": "BTC-LN"})
        approve = api(state, "POST", f"/api/v1/stores/{store}/payouts/{claimed['id']}", body={"revision": claimed.get("revision", 0)})
        wait_for("refund paid", lambda: cln(state, "ext-cln", "listinvoices", label)["invoices"][0]["status"] == "paid",
                 timeout=240, interval=5)
        return {"invoice": invoice_id, "pull_payment": pull["id"], "payout": claimed["id"], "approve_status": approve[0]}

    def refresh():
        ids = [v["id"] for v in spendable_vtxos(state)][:2]
        if not ids:
            raise RuntimeError("no spendable VTXOs")
        messages = vtxo_action(state, session, ids, "refresh")
        time.sleep(12)
        snapshot(state, "refresh submitted, round unconfirmed")
        mine(state, 1)
        time.sleep(10)
        mine(state, 1)
        return {"vtxos": len(ids), "messages": messages}

    def send_onchain():
        address = rpc(state, "getnewaddress", [], wallet="miner")
        messages = store_send(state, session, address, 30000, "ark", labels=["kassiber-send-onchain"])
        time.sleep(15)
        snapshot(state, "send-onchain before confirmation")
        mine(state, 1)
        time.sleep(10)
        mine(state, 1)
        return {"messages": messages, "amount_sat": 30000}

    def offboard_external():
        candidates = spendable_vtxos(state)
        if not candidates:
            raise RuntimeError("no spendable VTXOs")
        smallest = min(candidates, key=lambda v: int(v.get("amount_sat", 0)))
        messages = vtxo_action(state, session, [smallest["id"]], "offboard", rpc(state, "getnewaddress", [], wallet="miner"))
        time.sleep(20)
        mine(state, 1)
        time.sleep(10)
        mine(state, 1)
        return {"vtxo_amount_sat": smallest.get("amount_sat"), "messages": messages}

    def offboard_own():
        candidates = spendable_vtxos(state)
        if not candidates:
            raise RuntimeError("no spendable VTXOs")
        smallest = min(candidates, key=lambda v: int(v.get("amount_sat", 0)))
        own = store_addresses(state, session)["onchainAddress"]
        messages = vtxo_action(state, session, [smallest["id"]], "offboard", own)
        time.sleep(20)
        mine(state, 1)
        time.sleep(10)
        mine(state, 1)
        return {"vtxo_amount_sat": smallest.get("amount_sat"), "messages": messages}

    def onchain_send():
        address = rpc(state, "getnewaddress", [], wallet="miner")
        messages = store_send(state, session, address, 20000, "onchain", labels=["kassiber-onchain-send"])
        time.sleep(5)
        snapshot(state, "onchain-send in mempool")
        mine(state, 1)
        return {"messages": messages, "amount_sat": 20000}

    def onchain_internal():
        own = store_addresses(state, session)["onchainAddress"]
        messages = store_send(state, session, own, 15000, "onchain")
        time.sleep(5)
        mine(state, 1)
        return {"messages": messages, "amount_sat": 15000}

    def exit_vtxo():
        candidates = spendable_vtxos(state)
        if not candidates:
            raise RuntimeError("no spendable VTXOs")
        smallest = min(candidates, key=lambda v: int(v.get("amount_sat", 0)))
        messages = vtxo_action(state, session, [smallest["id"]], "emergency-exit")
        deadline = time.monotonic() + args.exit_budget
        rounds = 0
        while time.monotonic() < deadline:
            # Jump straight to the claim height once barkd knows it.
            _, exits = barkd(state, "GET", "/api/v1/exits/status/all")
            claimable = [e["state"].get("claimable_height") for e in exits or [] if isinstance(e, dict)
                         and isinstance(e.get("state"), dict) and e["state"].get("claimable_height")]
            height = rpc(state, "getblockcount")
            mine(state, max(6, max(claimable) - height + 2) if claimable else 6)
            rounds += 1
            time.sleep(10)
            status, rows = wallet_rows(state)
            if status == 200 and any(r.get("kind") == "exit-claim" for r in rows):
                break
            if rounds == 3:
                snapshot(state, "exit in progress")
        return {"vtxo_amount_sat": smallest.get("amount_sat"), "messages": messages, "mining_rounds": rounds}

    def settle():
        mine(state, 3)
        time.sleep(20)
        return {}

    actions = {
        "onchain-receive": onchain_receive, "board": board, "ark-receive-invoice": ark_receive_invoice,
        "ark-receive-plain": ark_receive_plain, "lightning-receive-invoice": lightning_receive_invoice,
        "lightning-receive-ln-only": lightning_receive_ln_only, "lightning-receive-plain": lightning_receive_plain,
        "ark-send": ark_send, "lightning-send": lightning_send, "lightning-api-pay": lightning_api_pay,
        "payout": payout, "refund": refund, "refresh": refresh, "send-onchain": send_onchain,
        "offboard-external": offboard_external, "offboard-own": offboard_own, "onchain-send": onchain_send,
        "onchain-internal": onchain_internal, "exit": exit_vtxo, "settle": settle,
    }
    for name in steps:
        run(name, actions[name])
    print(json.dumps([{k: s[k] for k in ("step", "outcome", "seconds")} for s in state["steps"]], indent=2))
    failed = [name for name, outcome in this_run if outcome != "ok" and name not in OPTIONAL_STEPS]
    if failed and not args.allow_failures:
        raise SystemExit(f"scenario steps failed: {', '.join(failed)}")


# Best effort: an exit needs many blocks, and the refund and Lightning API pay
# depend on BTCPay features outside the plugin.
OPTIONAL_STEPS = {"exit", "refund", "lightning-api-pay", "payout"}
STEP_ORDER = [
    "onchain-receive", "board", "ark-receive-invoice", "ark-receive-plain", "lightning-receive-invoice",
    "lightning-receive-ln-only", "lightning-receive-plain", "ark-send", "lightning-send", "lightning-api-pay",
    "payout", "refund", "refresh", "send-onchain", "offboard-external", "offboard-own", "onchain-send",
    "onchain-internal", "exit", "settle",
]


# --------------------------------------------------------------------------- record


def redact(value: Any, key: str = "") -> Any:
    if isinstance(value, dict):
        return {k: redact(v, k) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v, key) for v in value]
    if isinstance(value, str):
        if key and SECRET_KEYS.search(key) and value:
            return "<redacted>"
        return SECRET_VALUES.sub(lambda m: f"{m.group(1)}=<redacted>", value)
    return value


NETWORK_HINTS = [
    ("ark address (testnet HRP tark)", re.compile(r"^tark1")),
    ("ark address (mainnet HRP ark)", re.compile(r"^ark1")),
    ("bolt11 regtest (lnbcrt)", re.compile(r"(?i)^(lightning:)?lnbcrt")),
    ("bolt11 signet/testnet (lntbs/lntb)", re.compile(r"(?i)^(lightning:)?lntbs?\d")),
    ("bolt11 mainnet (lnbc)", re.compile(r"(?i)^(lightning:)?lnbc\d")),
    ("regtest address (bcrt1)", re.compile(r"(?i)^bcrt1")),
    ("BIP21/321 link", re.compile(r"(?i)^bitcoin:")),
    ("connection string", re.compile(r"(?i)type=")),
]


def network_hints(name: str, value: Any, path: str = "$", out: list | None = None) -> list:
    out = [] if out is None else out
    if isinstance(value, dict):
        for k, v in value.items():
            if k.lower() in {"network", "chain", "cryptocode", "connectionstring", "bip21"} and isinstance(v, (str, type(None))):
                out.append({"file": name, "path": f"{path}.{k}", "hint": f"field {k}", "sample": (v or "")[:40]})
            network_hints(name, v, f"{path}.{k}", out)
    elif isinstance(value, list):
        for i, v in enumerate(value):
            network_hints(name, v, f"{path}[{i}]", out)
    elif isinstance(value, str):
        for hint, pattern in NETWORK_HINTS:
            if pattern.search(value) or (hint.startswith("BIP21") and "bitcoin:" in value.lower()):
                out.append({"file": name, "path": path, "hint": hint, "sample": value[:40]})
                break
    return out


def record(args) -> None:
    state = load_state(args.state)
    store, plain = state["store_id"], state["plain_store_id"]
    out = Path(args.out).resolve() / (args.label or state["plugin"])
    out.mkdir(parents=True, exist_ok=True)
    written: dict[str, Any] = {}

    def save(name: str, key: str | None, path: str, method: str = "GET") -> Any:
        status, body = api(state, method, path, key=key)
        payload = {"request": {"method": method, "path": path.replace(store, "{storeId}").replace(plain, "{plainStoreId}"),
                               "key": key, "permissions": KEYS.get(key or "", [])},
                   "status": status, "body": redact(body)}
        (out / f"{name}.json").write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
        written[name] = payload
        return body

    s = f"/api/v1/stores/{store}"
    wallet = f"{s}/payment-methods/BTC-ARK/wallet"
    save("server-info", "read_only", "/api/v1/server/info")
    save("stores", "read_only", "/api/v1/stores")
    save("payment-methods.enabled", "read_only", f"{s}/payment-methods?onlyEnabled=true")
    save("payment-methods.all", "read_only", f"{s}/payment-methods")
    save("payment-methods.include-config.modify-key", "modify_settings", f"{s}/payment-methods?includeConfig=true")
    save("payment-methods.include-config.read-only-key", "read_only", f"{s}/payment-methods?includeConfig=true")
    for method_id in ("BTC-ARK", "BTC-LN", "BARK"):
        save(f"payment-method.{method_id}.include-config.modify-key", "modify_settings", f"{s}/payment-methods/{method_id}?includeConfig=true")
        save(f"payment-method.{method_id}.include-config.read-only-key", "read_only", f"{s}/payment-methods/{method_id}?includeConfig=true")
    save("wallet.overview", "read_only", wallet)
    first = save("wallet.transactions", "read_only", f"{wallet}/transactions")
    second = save("wallet.transactions.second-read", "read_only", f"{wallet}/transactions")
    save("wallet.transactions.confirmed", "read_only", f"{wallet}/transactions?statusFilter=Confirmed")
    save("wallet.transactions.unconfirmed", "read_only", f"{wallet}/transactions?statusFilter=Unconfirmed")
    save("wallet.transactions.label-invoice", "read_only", f"{wallet}/transactions?labelFilter=invoice")
    save("wallet.transactions.page-1", "read_only", f"{wallet}/transactions?skip=0&limit=3")
    save("wallet.transactions.page-2", "read_only", f"{wallet}/transactions?skip=3&limit=3")
    save("wallet.transactions.negative-skip", "read_only", f"{wallet}/transactions?skip=-1")
    save("wallet.transactions.bad-status", "read_only", f"{wallet}/transactions?statusFilter=Bogus")
    save("wallet.transactions.no-wallet-permission", "settings_only", f"{wallet}/transactions")
    save("wallet.overview.no-wallet-permission", "settings_only", wallet)
    save("wallet.transactions.no-bark-store", "read_only", f"/api/v1/stores/{plain}/payment-methods/BTC-ARK/wallet/transactions")
    save("wallet.overview.no-bark-store", "read_only", f"/api/v1/stores/{plain}/payment-methods/BTC-ARK/wallet")
    save("wallet.transactions.BTC-LN", "read_only", f"{s}/payment-methods/BTC-LN/wallet/transactions")
    save("wallet.transactions.BARK", "read_only", f"{s}/payment-methods/BARK/wallet/transactions")
    save("wallet.overview.BARK", "read_only", f"{s}/payment-methods/BARK/wallet")
    save("wallet.transactions.BTC-CHAIN", "read_only", f"{s}/payment-methods/BTC-CHAIN/wallet/transactions")
    invoices = save("invoices", "invoices", f"{s}/invoices?includePaymentMethods=true")
    for invoice in invoices if isinstance(invoices, list) else []:
        save(f"invoice.{invoice['id']}.payment-methods.accounted", "invoices",
             f"{s}/invoices/{invoice['id']}/payment-methods?onlyAccountedPayments=true")
    save("payouts", "payouts", f"{s}/payouts?includeCancelled=true")
    save("lightning.invoices", "lightning", f"{s}/lightning/BTC/invoices")
    save("lightning.payments", "lightning", f"{s}/lightning/BTC/payments?includePending=true")
    save("lightning.info", "lightning", f"{s}/lightning/BTC/info")
    save("lightning.balance", "lightning", f"{s}/lightning/BTC/balance")

    restart: dict[str, Any] = {}
    if not args.no_restart and isinstance(first, list):  # no wallet API to compare before 0.10.0
        log("restarting BTCPay to compare ids")
        compose(state, "restart", "btcpayserver")
        wait_for("BTCPay health", lambda: api(state, "GET", "/api/v1/health", key=None)[0] == 200, timeout=600, interval=3)
        wait_for("wallet API after restart", lambda: wallet_rows(state)[0] == 200, timeout=600, interval=5)
        after = save("wallet.transactions.after-restart", "read_only", f"{wallet}/transactions")
        restart = compare_ids(first, after)
    snapshots_dir = out / "snapshots"
    snapshots_dir.mkdir(exist_ok=True)
    for index, snap in enumerate(state.get("snapshots", [])):
        slug = re.sub(r"[^a-z0-9]+", "-", snap["label"].lower()).strip("-")
        (snapshots_dir / f"{index:02d}-{slug}.json").write_text(json.dumps(redact(snap), indent=2, sort_keys=True) + "\n")
    observations = observe(state, written, compare_ids(first, second), restart)
    (out / "observations.json").write_text(json.dumps(observations, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"out": str(out), "files": len(written), "kinds": observations.get("kinds")}, indent=2))


def compare_ids(before: Any, after: Any) -> dict[str, Any]:
    if not isinstance(before, list) or not isinstance(after, list):
        return {"comparable": False}
    a = {r["id"]: r for r in before}
    b = {r["id"]: r for r in after}
    changed = [i for i in a if i in b and (a[i]["amount"], a[i].get("fee")) != (b[i]["amount"], b[i].get("fee"))]
    return {"comparable": True, "rows_before": len(a), "rows_after": len(b), "missing_after": sorted(set(a) - set(b)),
            "new_after": sorted(set(b) - set(a)), "amount_or_fee_changed": changed,
            "same_order": [r["id"] for r in before] == [r["id"] for r in after][:len(before)]}


def sats(value: Any) -> int | None:
    if value is None:
        return None
    from decimal import Decimal
    return int((Decimal(str(value)) * 100_000_000).to_integral_value())


def observe(state: dict[str, Any], written: dict[str, Any], reread: dict[str, Any], restart: dict[str, Any]) -> dict[str, Any]:
    rows = (written.get("wallet.transactions") or {}).get("body")
    result: dict[str, Any] = {"plugin": state["plugin"], "id_stability_second_read": reread, "id_stability_after_restart": restart,
                              "scenario": [{k: v for k, v in s.items() if k in ("step", "outcome", "error")} for s in state.get("steps", [])]}
    if isinstance(rows, list):
        kinds: dict[str, int] = {}
        shapes: dict[str, list] = {}
        for row in rows:
            kinds[row["kind"]] = kinds.get(row["kind"], 0) + 1
            shapes.setdefault(row["kind"], []).append({
                "amount_sat": sats(row.get("amount")), "fee_sat": sats(row.get("fee")), "status": row.get("status"),
                "confirmations": row.get("confirmations"), "has_transactionHash": row.get("transactionHash") is not None,
                "has_outpoint": row.get("outpoint") is not None, "has_paymentHash": row.get("paymentHash") is not None,
                "has_block": row.get("blockHeight") is not None, "movementId": row.get("movementId"),
                "labels": sorted((row.get("labels") or {}).keys()), "id_shape": re.sub(r"[0-9a-f]{64}", "<txid>", re.sub(r"\d{6,}", "<n>", row["id"]))})
        result["kinds"] = kinds
        result["rows_by_kind"] = shapes
        matches = []
        by_outpoint = {r.get("outpoint"): r for r in rows if r.get("outpoint")}
        by_hash = {r.get("paymentHash"): r for r in rows if r.get("paymentHash")}
        for name, payload in written.items():
            if not name.startswith("invoice.") or not isinstance(payload.get("body"), list):
                continue
            for method in payload["body"]:
                for payment in method.get("payments") or []:
                    pid = payment.get("id", "")
                    row = by_outpoint.get(pid.replace("-", ":")) or by_hash.get(pid)
                    matches.append({"invoice": name.split(".")[1], "paymentMethodId": method.get("paymentMethodId"),
                                    "payment_id_shape": re.sub(r"[0-9a-f]{64}", "<hex64>", pid), "payment_value_sat": sats(payment.get("value")),
                                    "matched_kind": row and row["kind"], "row_amount_sat": row and sats(row.get("amount")),
                                    "row_fee_sat": row and sats(row.get("fee")),
                                    "invoice_label_ref_matches": bool(row and ((row.get("labels") or {}).get("invoice") or {}).get("ref") == name.split(".")[1])})
        result["invoice_payment_matches"] = matches
    hints = []
    for name, payload in written.items():
        network_hints(name, payload.get("body"), "$", hints)
    result["network_hints"] = hints
    return result


# --------------------------------------------------------------------------- upgrade / down


def upgrade(args) -> None:
    state = load_state(args.state)
    workdir = Path(state["workdir"])
    old_dir = state["plugin_dir"]
    state["plugin_dir"] = str(install_plugin(args.plugin, plugin_package(args), workdir))
    state["upgraded_from"] = state["plugin"]
    state["plugin"] = args.plugin
    save_state(args.state, state)
    compose(state, "up", "-d", "--force-recreate", "--no-deps", "btcpayserver")
    wait_for("BTCPay health", lambda: api(state, "GET", "/api/v1/health", key=None)[0] == 200, timeout=600, interval=3)
    wait_for("barkd", lambda: barkd_location(state), timeout=300, interval=3)
    shutil.rmtree(old_dir, ignore_errors=True)
    print(json.dumps({"plugin": args.plugin, "upgraded_from": state["upgraded_from"]}))


def down(args) -> None:
    state = load_state(args.state)
    compose(state, "down", "-v", "--remove-orphans")
    shutil.rmtree(state["workdir"], ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("command", choices=("up", "scenario", "record", "upgrade", "down"))
    parser.add_argument("--state", required=True)
    parser.add_argument("--plugin", default="0.13.0", choices=sorted(PLUGINS))
    parser.add_argument("--plugin-package")
    parser.add_argument("--project", default=DEFAULT_PROJECT)
    parser.add_argument("--base-port", type=int, default=28600)
    parser.add_argument("--out")
    parser.add_argument("--label", help="fixture subdirectory (default: the plugin version)")
    parser.add_argument("--steps", help="comma-separated scenario steps (default: all)")
    parser.add_argument("--exit-budget", type=int, default=600, help="seconds to drive an emergency exit")
    parser.add_argument("--no-restart", action="store_true", help="record without restarting BTCPay")
    parser.add_argument("--allow-failures", action="store_true", help="scenario exits 0 even when steps fail")
    args = parser.parse_args()
    if args.command == "record" and not args.out:
        parser.error("record needs --out")
    {"up": up, "scenario": scenario, "record": record, "upgrade": upgrade, "down": down}[args.command](args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
