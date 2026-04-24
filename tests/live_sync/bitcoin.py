"""Session-scoped Bitcoin Core regtest stack for live-sync tests."""

from __future__ import annotations

import os
import secrets
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


DEFAULT_BITCOIND_IMAGE = "bitcoin/bitcoin:28.1"


class BitcoinRegtestStack:
    """One bitcoind regtest container, shared across tests in a module.

    Credentials and container name are randomized per session so parallel
    stacks on the same host do not collide and there is no static password
    to misuse outside the regtest context.
    """

    def __init__(
        self,
        image: Optional[str] = None,
        allow_pull: bool = False,
    ) -> None:
        self.image = image or os.environ.get("KASSIBER_BITCOIND_IMAGE", DEFAULT_BITCOIND_IMAGE)
        self.allow_pull = allow_pull
        self.container: Optional[str] = None
        self.username = f"kassiber-{secrets.token_hex(4)}"
        self.password = secrets.token_hex(16)
        self.rpc: Optional[BitcoinRpc] = None
        self._miner_wallet: Optional[str] = None
        self._mining_address: Optional[str] = None

    # ------------------------------------------------------------------ lifecycle

    def start(self) -> None:
        ok, reason = docker_daemon_available()
        if not ok:
            raise DockerUnavailable(reason)
        ensure_image(self.image, self.allow_pull)

        self.container = random_container_name("kassiber-bitcoin-regtest")
        docker_run(
            [
                "run",
                "--rm",
                "-d",
                "--name",
                self.container,
                "-p",
                "127.0.0.1::18443",
                self.image,
                "-regtest=1",
                "-server=1",
                "-txindex=1",
                "-fallbackfee=0.0001",
                f"-rpcuser={self.username}",
                f"-rpcpassword={self.password}",
                "-rpcbind=0.0.0.0",
                f"-rpcallowip={DOCKER_BRIDGE_CIDR}",
                "-printtoconsole=1",
            ]
        )
        port = mapped_port(self.container, 18443)
        self.rpc = BitcoinRpc(f"http://127.0.0.1:{port}", self.username, self.password)
        try:
            wait_for_rpc(self.rpc)
        except AssertionError:
            raise AssertionError(
                f"bitcoind never became ready. Recent container logs:\n"
                f"{self.dump_logs()}"
            )

    def stop(self) -> None:
        if self.container:
            docker_run(["rm", "-f", self.container], check=False)
            self.container = None
            self.rpc = None
            self._miner_wallet = None
            self._mining_address = None

    def dump_logs(self) -> str:
        return capture_docker_logs(self.container or "")

    # ------------------------------------------------------------------ chain ops

    def ensure_miner(self, wallet_name: str = "miner") -> tuple[str, str]:
        """Create the miner wallet and a mining address if not yet present."""
        assert self.rpc is not None, "stack not started"
        if self._miner_wallet and self._mining_address:
            return self._miner_wallet, self._mining_address
        existing = self.rpc.call("listwallets") or []
        if wallet_name not in existing:
            self.rpc.call("createwallet", [wallet_name])
        address = self.rpc.call(
            "getnewaddress",
            ["miner", "bech32"],
            wallet=wallet_name,
        )
        # Coinbase maturity is 100 blocks; mine 101 so the first coinbase is
        # spendable and subsequent mining calls can be single-block.
        self.rpc.call("generatetoaddress", [101, address], wallet=wallet_name)
        self._miner_wallet = wallet_name
        self._mining_address = address
        return wallet_name, address

    def mine_blocks(self, count: int = 1) -> list[str]:
        wallet, address = self.ensure_miner()
        return self.rpc.call("generatetoaddress", [count, address], wallet=wallet)

    def send_to(self, address: str, amount_btc: float, confirm: bool = True) -> str:
        wallet, mining_address = self.ensure_miner()
        txid = self.rpc.call("sendtoaddress", [address, amount_btc], wallet=wallet)
        if confirm:
            self.rpc.call("generatetoaddress", [1, mining_address], wallet=wallet)
        return txid

    def new_watch_address(self) -> str:
        """Mint a fresh mining-wallet address intended to be watched externally."""
        wallet, _ = self.ensure_miner()
        return self.rpc.call("getnewaddress", ["kassiber-watch", "bech32"], wallet=wallet)
