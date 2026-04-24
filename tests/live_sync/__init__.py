"""Live-sync regtest harness package.

Session-scoped Bitcoin Core + Liquid (Elements) regtest stacks for opt-in
``tests/test_live_sync_*.py`` modules. Containers start once per test module
(``setUpModule``) and each test method gets a fresh Kassiber ``--data-root``
while sharing the chain state. This keeps a full live-sync suite under one
Docker-run cost per chain.

All networking is loopback-only:

* Bitcoin Core RPC binds to ``127.0.0.1`` on an ephemeral host port.
* Liquid Elements RPC and the electrs-liquid Electrum TCP port both bind to
  ``127.0.0.1`` on ephemeral host ports.
* ``rpcallowip`` is scoped to the Docker bridge range rather than
  ``0.0.0.0/0``.
* RPC credentials are randomized per session.

Environment toggles:

* ``KASSIBER_LIVE_SYNC_TESTS=1`` — enable the suite.
* ``KASSIBER_LIVE_SYNC_PULL=1`` — allow Docker to pull missing images.
* ``KASSIBER_REQUIRE_BITCOIN_REGTEST=1`` — fail (instead of skip) if the
  Bitcoin regtest stack cannot start.
* ``KASSIBER_REQUIRE_LIQUID_REGTEST=1`` — fail (instead of skip) if the
  Liquid regtest stack cannot start.
* ``KASSIBER_BITCOIND_IMAGE`` / ``KASSIBER_ELEMENTSD_IMAGE`` /
  ``KASSIBER_ELECTRS_LIQUID_IMAGE`` — override the Docker images.
"""

from __future__ import annotations

import base64
import json
import os
import secrets
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest


ROOT = Path(__file__).resolve().parent.parent.parent


def truthy(value: Any) -> bool:
    return str(value or "").strip().lower() in {"1", "true", "yes", "on"}


LIVE_SYNC_TESTS = truthy(os.environ.get("KASSIBER_LIVE_SYNC_TESTS"))
LIVE_SYNC_PULL = truthy(os.environ.get("KASSIBER_LIVE_SYNC_PULL"))
REQUIRE_BITCOIN_REGTEST = truthy(os.environ.get("KASSIBER_REQUIRE_BITCOIN_REGTEST"))
REQUIRE_LIQUID_REGTEST = truthy(
    os.environ.get("KASSIBER_REQUIRE_LIQUID_REGTEST")
    or os.environ.get("KASSIBER_REQUIRE_LIQUID_LIVE")
)


# Docker's default bridge network. Containers see the host through this range,
# so scoping rpcallowip here is defense-in-depth over the loopback host port
# bind without locking out legitimate container->daemon traffic.
DOCKER_BRIDGE_CIDR = "172.16.0.0/12"


class DockerUnavailable(Exception):
    """Raised when the Docker daemon or a required image is not accessible."""


def docker_run(args: Iterable[str], check: bool = True) -> subprocess.CompletedProcess:
    result = subprocess.run(
        ["docker", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"docker {' '.join(args)} failed with {result.returncode}\n"
            f"stdout={result.stdout!r}\nstderr={result.stderr!r}"
        )
    return result


def docker_daemon_available() -> tuple[bool, str]:
    if not shutil.which("docker"):
        return False, "docker CLI is not installed"
    result = docker_run(["info"], check=False)
    if result.returncode != 0:
        reason = result.stderr.strip() or result.stdout.strip() or "docker daemon is not reachable"
        return False, reason
    return True, ""


def docker_image_available(image: str) -> bool:
    return docker_run(["image", "inspect", image], check=False).returncode == 0


def ensure_image(image: str, allow_pull: bool) -> None:
    if docker_image_available(image):
        return
    if not allow_pull:
        raise DockerUnavailable(
            f"Docker image {image!r} is not present locally; "
            "pre-pull it or set KASSIBER_LIVE_SYNC_PULL=1 to allow a pull"
        )
    docker_run(["pull", image])


def mapped_port(container: str, container_port: int, timeout: float = 20.0) -> int:
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        result = docker_run(["port", container, f"{container_port}/tcp"], check=False)
        last = (result.stdout or result.stderr).strip()
        if result.returncode == 0 and result.stdout.strip():
            endpoint = result.stdout.strip().splitlines()[0]
            return int(endpoint.rsplit(":", 1)[1])
        time.sleep(0.2)
    raise AssertionError(
        f"Docker did not publish {container_port}/tcp for {container}: {last}"
    )


def random_container_name(prefix: str) -> str:
    return f"{prefix}-{os.getpid()}-{secrets.token_hex(4)}"


def capture_docker_logs(container: str, tail: int = 200) -> str:
    if not container:
        return ""
    result = docker_run(["logs", "--tail", str(tail), container], check=False)
    stdout = (result.stdout or "").strip()
    stderr = (result.stderr or "").strip()
    parts = []
    if stdout:
        parts.append(f"[stdout]\n{stdout}")
    if stderr:
        parts.append(f"[stderr]\n{stderr}")
    return "\n".join(parts) if parts else "(no logs)"


class BitcoinRpcError(AssertionError):
    pass


class BitcoinRpc:
    """Minimal JSON-RPC client used by the regtest harness."""

    def __init__(self, url: str, username: str, password: str):
        self.url = url.rstrip("/")
        self.username = username
        self.password = password

    def _url(self, wallet: str | None = None) -> str:
        if not wallet:
            return self.url
        return f"{self.url}/wallet/{urlparse.quote(wallet, safe='')}"

    def call(self, method: str, params: list | None = None, wallet: str | None = None):
        token = f"{self.username}:{self.password}".encode("utf-8")
        payload = json.dumps(
            {
                "jsonrpc": "1.0",
                "id": f"kassiber-live-{method}",
                "method": method,
                "params": [] if params is None else params,
            }
        ).encode("utf-8")
        req = urlrequest.Request(
            self._url(wallet),
            data=payload,
            headers={
                "Authorization": f"Basic {base64.b64encode(token).decode('ascii')}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urlrequest.urlopen(req, timeout=15) as response:
                message = json.loads(response.read().decode("utf-8"))
        except (urlerror.URLError, TimeoutError) as exc:
            raise BitcoinRpcError(str(exc)) from exc
        if message.get("error"):
            error = message["error"]
            raise BitcoinRpcError(
                f"{method} failed ({error.get('code', 'unknown')}): {error.get('message', error)}"
            )
        return message.get("result")


def wait_for_rpc(rpc: BitcoinRpc, probe: str = "getblockchaininfo", timeout: float = 60.0) -> None:
    deadline = time.time() + timeout
    last_error: BitcoinRpcError | None = None
    while time.time() < deadline:
        try:
            rpc.call(probe)
            return
        except BitcoinRpcError as exc:
            last_error = exc
            time.sleep(0.5)
    raise AssertionError(f"RPC probe {probe!r} never succeeded: {last_error}") from last_error


def run_kassiber_json(data_root: Path, *args: str) -> tuple[dict, subprocess.CompletedProcess]:
    """Invoke the Kassiber CLI in machine mode and parse the JSON envelope."""
    cmd = [
        sys.executable,
        "-m",
        "kassiber",
        "--data-root",
        str(data_root),
        "--machine",
        *args,
    ]
    result = subprocess.run(
        cmd,
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(
            f"CLI produced no stdout for {args!r}; returncode={result.returncode}; stderr={result.stderr!r}"
        )
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise AssertionError(f"CLI stdout was not JSON for {args!r}: {stdout[:500]}") from exc
    return payload, result


def assert_kassiber_ok(testcase, payload: dict, result: subprocess.CompletedProcess, kind: str) -> None:
    testcase.assertEqual(
        result.returncode,
        0,
        msg=f"payload={payload!r}; stderr={result.stderr!r}",
    )
    testcase.assertEqual(payload.get("schema_version"), 1)
    testcase.assertEqual(payload.get("kind"), kind)


def url_is_loopback(value: str) -> bool:
    parsed = urlparse.urlsplit(value if "://" in value else f"tcp://{value}")
    host = (parsed.hostname or "").lower()
    return host in {"localhost", "127.0.0.1", "::1"}


@dataclass
class StackStartResult:
    started: bool
    skip_reason: str = ""
