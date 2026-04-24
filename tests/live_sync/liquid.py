"""Session-scoped Liquid (Elements) regtest stack for live-sync tests.

Brings up two containers on a dedicated Docker bridge network:

1. ``elementsd`` running ``-chain=elementsregtest`` with loopback-bound RPC.
2. An electrs-liquid indexer pointed at the daemon, exposing an Electrum TCP
   endpoint on ``127.0.0.1:<ephemeral>``.

Kassiber's Liquid sync path consumes the electrs-liquid Electrum interface
directly, so tests can wire a Kassiber ``electrum`` backend at the host port
and sync descriptor wallets funded via ``elementsd``.

Image choice defaults to the Vulpem/Nigiri builds because those are the most
actively maintained Liquid dev images. Override via env var when the upstream
moves:

* ``KASSIBER_ELEMENTSD_IMAGE``
* ``KASSIBER_ELECTRS_LIQUID_IMAGE``
* ``KASSIBER_ELEMENTSD_EXTRA_ARGS`` — extra CLI args for elementsd
* ``KASSIBER_ELECTRS_LIQUID_EXTRA_ARGS`` — extra CLI args for electrs-liquid
"""

from __future__ import annotations

import json
import os
import secrets
import shlex
import socket
import time
from typing import Optional

from . import (
    BitcoinRpc,
    DOCKER_BRIDGE_CIDR,
    DockerUnavailable,
    capture_docker_logs,
    docker_daemon_available,
    docker_run,
    ensure_image,
    mapped_port,
    random_container_name,
    wait_for_rpc,
)


DEFAULT_ELEMENTSD_IMAGE = "ghcr.io/vulpemventures/elements:23.2.1"
DEFAULT_ELECTRS_LIQUID_IMAGE = "ghcr.io/vulpemventures/electrs-liquid:latest"

# Elements Core defaults for elementsregtest.
ELEMENTSD_RPC_PORT = 18884
ELECTRS_ELECTRUM_PORT = 60401

ELEMENTSREGTEST_NETWORK = "elementsregtest"


def _wait_for_port(host: str, port: int, timeout: float = 60.0) -> None:
    deadline = time.time() + timeout
    last_exc: Exception | None = None
    while time.time() < deadline:
        try:
            with socket.create_connection((host, port), timeout=2):
                return
        except OSError as exc:
            last_exc = exc
            time.sleep(0.5)
    raise AssertionError(f"port {host}:{port} never opened: {last_exc}")


class LiquidRegtestStack:
    """Two-container Liquid regtest: elementsd + electrs-liquid indexer."""

    def __init__(
        self,
        elementsd_image: Optional[str] = None,
        electrs_image: Optional[str] = None,
        allow_pull: bool = False,
    ) -> None:
        self.elementsd_image = (
            elementsd_image
            or os.environ.get("KASSIBER_ELEMENTSD_IMAGE")
            or DEFAULT_ELEMENTSD_IMAGE
        )
        self.electrs_image = (
            electrs_image
            or os.environ.get("KASSIBER_ELECTRS_LIQUID_IMAGE")
            or DEFAULT_ELECTRS_LIQUID_IMAGE
        )
        self.allow_pull = allow_pull

        self.network: Optional[str] = None
        self.elementsd_container: Optional[str] = None
        self.electrs_container: Optional[str] = None

        self.rpc_username = f"kassiber-{secrets.token_hex(4)}"
        self.rpc_password = secrets.token_hex(16)
        self.rpc: Optional[BitcoinRpc] = None

        self.electrum_host = "127.0.0.1"
        self.electrum_port: Optional[int] = None

        self._miner_wallet: Optional[str] = None
        self._miner_address: Optional[str] = None

    # ------------------------------------------------------------------ lifecycle

    def start(self) -> None:
        ok, reason = docker_daemon_available()
        if not ok:
            raise DockerUnavailable(reason)
        ensure_image(self.elementsd_image, self.allow_pull)
        ensure_image(self.electrs_image, self.allow_pull)

        self.network = random_container_name("kassiber-liquid-regtest-net")
        docker_run(["network", "create", self.network])

        self._start_elementsd()
        self._start_electrs()

    def stop(self) -> None:
        for container in (self.electrs_container, self.elementsd_container):
            if container:
                docker_run(["rm", "-f", container], check=False)
        self.electrs_container = None
        self.elementsd_container = None
        if self.network:
            docker_run(["network", "rm", self.network], check=False)
            self.network = None
        self.rpc = None
        self.electrum_port = None
        self._miner_wallet = None
        self._miner_address = None

    def dump_logs(self) -> str:
        parts = []
        if self.elementsd_container:
            parts.append(f"--- elementsd ({self.elementsd_container}) ---\n{capture_docker_logs(self.elementsd_container)}")
        if self.electrs_container:
            parts.append(f"--- electrs-liquid ({self.electrs_container}) ---\n{capture_docker_logs(self.electrs_container)}")
        return "\n\n".join(parts) if parts else "(no containers)"

    # ------------------------------------------------------------------ internals

    def _start_elementsd(self) -> None:
        assert self.network
        self.elementsd_container = random_container_name("kassiber-elementsd-regtest")

        extra = shlex.split(os.environ.get("KASSIBER_ELEMENTSD_EXTRA_ARGS", ""))

        docker_run(
            [
                "run",
                "--rm",
                "-d",
                "--name",
                self.elementsd_container,
                "--network",
                self.network,
                "--network-alias",
                "elementsd",
                "-p",
                f"127.0.0.1::{ELEMENTSD_RPC_PORT}",
                self.elementsd_image,
                "elementsd",
                "-chain=elementsregtest",
                "-server=1",
                "-txindex=1",
                "-validatepegin=0",
                "-initialfreecoins=2100000000000000",
                "-anyonecanspendaremine=1",
                "-con_blocksubsidy=0",
                "-fallbackfee=0.00001",
                f"-rpcuser={self.rpc_username}",
                f"-rpcpassword={self.rpc_password}",
                "-rpcbind=0.0.0.0",
                f"-rpcallowip={DOCKER_BRIDGE_CIDR}",
                f"-rpcport={ELEMENTSD_RPC_PORT}",
                "-printtoconsole=1",
                *extra,
            ]
        )
        port = mapped_port(self.elementsd_container, ELEMENTSD_RPC_PORT)
        self.rpc = BitcoinRpc(
            f"http://127.0.0.1:{port}", self.rpc_username, self.rpc_password
        )
        try:
            wait_for_rpc(self.rpc)
        except AssertionError:
            raise AssertionError(
                f"elementsd never became ready. Recent container logs:\n"
                f"{self.dump_logs()}"
            )

    def _start_electrs(self) -> None:
        assert self.network
        self.electrs_container = random_container_name("kassiber-electrs-liquid")

        extra = shlex.split(os.environ.get("KASSIBER_ELECTRS_LIQUID_EXTRA_ARGS", ""))

        docker_run(
            [
                "run",
                "--rm",
                "-d",
                "--name",
                self.electrs_container,
                "--network",
                self.network,
                "-p",
                f"127.0.0.1::{ELECTRS_ELECTRUM_PORT}",
                self.electrs_image,
                "electrs-liquid",
                f"--network={ELEMENTSREGTEST_NETWORK}",
                f"--daemon-rpc-addr=elementsd:{ELEMENTSD_RPC_PORT}",
                f"--cookie={self.rpc_username}:{self.rpc_password}",
                f"--electrum-rpc-addr=0.0.0.0:{ELECTRS_ELECTRUM_PORT}",
                "--jsonrpc-import",
                *extra,
            ]
        )
        self.electrum_port = mapped_port(self.electrs_container, ELECTRS_ELECTRUM_PORT)
        _wait_for_port(self.electrum_host, self.electrum_port)

    # ------------------------------------------------------------------ chain ops

    def electrum_url(self) -> str:
        assert self.electrum_port, "stack not started"
        return f"tcp://{self.electrum_host}:{self.electrum_port}"

    def ensure_miner(self, wallet_name: str = "miner") -> str:
        assert self.rpc is not None, "stack not started"
        if self._miner_wallet and self._miner_address:
            return self._miner_address
        existing = self.rpc.call("listwallets") or []
        if wallet_name not in existing:
            # ``descriptors=False`` keeps things on the legacy wallet
            # interface we rely on below (``rescanblockchain``, raw
            # sendtoaddress, etc. all work identically).
            self.rpc.call("createwallet", [wallet_name])
        address = self.rpc.call("getnewaddress", [], wallet=wallet_name)
        self.rpc.call("generatetoaddress", [101, address], wallet=wallet_name)
        self._miner_wallet = wallet_name
        self._miner_address = address
        return address

    def mine_blocks(self, count: int = 1) -> list[str]:
        self.ensure_miner()
        assert self._miner_wallet and self._miner_address
        return self.rpc.call(
            "generatetoaddress",
            [count, self._miner_address],
            wallet=self._miner_wallet,
        )

    def policy_asset_id(self) -> str:
        """Return the elementsregtest policy (native) asset id for this run.

        The policy asset is derived from the genesis block, which depends on
        consensus parameters (``initialfreecoins``, ``con_blocksubsidy``,
        etc.). Rather than hardcode an id that only matches one specific
        config, we query elementsd directly via ``dumpassetlabels``.
        """
        assert self.rpc is not None, "stack not started"
        self.ensure_miner()
        labels = self.rpc.call("dumpassetlabels", [], wallet=self._miner_wallet) or {}
        if isinstance(labels, dict) and labels.get("bitcoin"):
            return str(labels["bitcoin"])
        # Fallback: some Elements builds expose the native asset through
        # ``getbalance`` keys instead.
        balance = self.rpc.call("getbalance", [], wallet=self._miner_wallet) or {}
        if isinstance(balance, dict):
            for key in balance:
                if len(key) == 64 and all(c in "0123456789abcdef" for c in key.lower()):
                    return key.lower()
        raise AssertionError(
            f"could not determine elementsregtest policy asset id; labels={labels!r}"
        )

    def send_to(self, address: str, amount_lbtc: float, confirm: bool = True) -> str:
        self.ensure_miner()
        assert self._miner_wallet
        txid = self.rpc.call("sendtoaddress", [address, amount_lbtc], wallet=self._miner_wallet)
        if confirm:
            self.mine_blocks(1)
        return txid

    # ----------------------------------------------------------- descriptor mint

    def mint_blinded_descriptor(self, label: str = "kassiber-test") -> dict:
        """Create a fresh descriptor wallet in elementsd and export its
        external + internal descriptors with a private blinding key so
        Kassiber can consume them as ``--descriptor-file`` /
        ``--change-descriptor-file``.

        Handles both unified ``<0;1>`` descriptors (Bitcoin Core 24+ /
        matching Elements builds) and the legacy split form by looking for
        either shape in ``listdescriptors`` output and splitting the unified
        form into two with fresh checksums via ``getdescriptorinfo``.
        """
        assert self.rpc is not None, "stack not started"
        wallet = f"{label}-{secrets.token_hex(4)}"
        # createwallet args: name, disable_private_keys=False, blank=False,
        # passphrase="", avoid_reuse=False, descriptors=True. Descriptors
        # wallets are the target of this test so we pin that explicitly.
        self.rpc.call(
            "createwallet",
            [wallet, False, False, "", False, True],
        )
        raw = self.rpc.call("listdescriptors", [True], wallet=wallet)
        entries = raw.get("descriptors") if isinstance(raw, dict) else (raw or [])
        if not entries:
            raise AssertionError(
                f"elementsd returned no descriptors for wallet {wallet}: raw={raw!r}"
            )

        def is_blinded_segwit_v0(desc: str) -> bool:
            # ``ct(...)`` wraps a blinding key + script descriptor; ``elwpkh``
            # is the Liquid variant of ``wpkh``. Stick to segwit v0 here
            # because taproot on Liquid is still gated behind dynamic
            # federation checks that are noisier in tests.
            return "ct(" in desc and "elwpkh" in desc

        # Prefer a unified descriptor if elementsd returned one; split it.
        for entry in entries:
            desc = entry.get("desc", "")
            if not is_blinded_segwit_v0(desc):
                continue
            if "<0;1>" in desc or "<0,1>" in desc:
                receive, change = self._split_unified_descriptor(desc, wallet)
                return {
                    "wallet": wallet,
                    "receive_descriptor": receive,
                    "change_descriptor": change,
                }

        # Legacy split form: pick external + internal from the entry list.
        receive: Optional[str] = None
        change: Optional[str] = None
        for entry in entries:
            desc = entry.get("desc", "")
            if not is_blinded_segwit_v0(desc):
                continue
            if bool(entry.get("internal")):
                change = change or desc
            else:
                receive = receive or desc
        if receive and change:
            return {
                "wallet": wallet,
                "receive_descriptor": receive,
                "change_descriptor": change,
            }

        raise AssertionError(
            f"elementsd descriptors missing a ct(slip77..., elwpkh...) pair: "
            f"{json.dumps(entries)}"
        )

    def _split_unified_descriptor(self, desc: str, wallet: str) -> tuple[str, str]:
        assert self.rpc is not None
        body = desc.split("#", 1)[0]
        for marker in ("<0;1>", "<0,1>"):
            if marker in body:
                ext_body = body.replace(marker, "0")
                int_body = body.replace(marker, "1")
                ext_info = self.rpc.call("getdescriptorinfo", [ext_body], wallet=wallet)
                int_info = self.rpc.call("getdescriptorinfo", [int_body], wallet=wallet)
                return ext_info["descriptor"], int_info["descriptor"]
        raise ValueError(f"descriptor is not unified: {desc}")
