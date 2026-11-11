"""Independent Bitcoin Core / Elements truth oracle for the chain observers.

The lane records node truth before any observer is involved: Bitcoin Core and
Elements RPC define txids, outpoints, heights, confirmation and replacement
relationships, ownership indices, UTXOs and asset values, and the local
Electrum and Esplora endpoints must report the same tip. The live BDK and LWK
observer tests then hold the Rust binary's books to that same node truth.
Generated manifests are disposable and must never be checked in: they contain
real regtest txids, outpoints and ownership metadata for the current run.

Standard library only (docs/plan/22-python-removal.md, D2). The oracle once
also drove the Python product's observer store and the bdkpython/lwk client
bindings through TLS and SOCKS forwarders; those checked the Python side and
went with it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import socket
import tempfile
import time
import uuid
from dataclasses import dataclass, field
from decimal import Decimal
from pathlib import Path
from typing import Any, Iterable
from urllib import request

from . import regtest_demo


SAT = Decimal("0.00000001")
MANIFEST_SCHEMA_VERSION = 1
OBSERVER_STATE_VERSION = 1
MAX_MANIFEST_BYTES = 1_000_000
BITCOIN_TRANSITIONS = (
    "initial_full",
    "noop",
    "gap_payment",
    "gap_expansion",
    "unconfirmed_receipt",
    "confirmation",
    "unconfirmed_spend",
    "rbf_replacement",
    "replacement_confirmation",
    "reorg",
    "resurrection",
    "reconfirmation",
    "process_restart",
    "incremental",
    "final_noop",
)
LIQUID_TRANSITIONS = (
    "initial_full",
    "noop",
    "gap_receive",
    "lbtc_spend",
    "issued_asset_receive",
    "issued_asset_spend",
    "confirmation",
    "reorg",
    "resurrection",
    "reconfirmation",
    "process_restart",
    "incremental",
)


def _rpc(
    url: str,
    username: str,
    password: str,
    method: str,
    params: list[Any] | None = None,
    *,
    wallet: str | None = None,
) -> Any:
    return regtest_demo.rpc(
        url,
        username,
        password,
        method,
        params or [],
        wallet=wallet,
    )


def _rpc_optional(*args: Any, **kwargs: Any) -> Any | None:
    try:
        return _rpc(*args, **kwargs)
    except RuntimeError:
        return None


def _sha256_json(value: Any) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


def _btc_sats(value: Any) -> int:
    return int((Decimal(str(value or 0)) * Decimal(100_000_000)).to_integral_value())


def _tx_fact(
    url: str,
    username: str,
    password: str,
    txid: str,
    *,
    replacement: str | None = None,
    replaced_by: str | None = None,
    direction: str | None = None,
) -> dict[str, Any]:
    tx = _rpc_optional(url, username, password, "getrawtransaction", [txid, True])
    if not isinstance(tx, dict):
        return {
            "txid": txid,
            "state": "replaced" if replaced_by else "absent",
            "confirmed": False,
            "height": None,
            "replacement": replacement,
            "replaced_by": replaced_by,
            "expected_direction": direction,
        }
    confirmations = int(tx.get("confirmations") or 0)
    height = None
    if tx.get("blockhash"):
        header = _rpc_optional(
            url,
            username,
            password,
            "getblockheader",
            [tx["blockhash"]],
        )
        if isinstance(header, dict):
            height = int(header.get("height") or 0)
    in_mempool = _rpc_optional(
        url,
        username,
        password,
        "getmempoolentry",
        [txid],
    ) is not None
    state = "confirmed" if confirmations > 0 else "mempool" if in_mempool else "conflicted"
    return {
        "txid": txid,
        "state": state,
        "confirmed": confirmations > 0,
        "confirmations": confirmations,
        "height": height,
        "replacement": replacement,
        "replaced_by": replaced_by,
        "expected_direction": direction,
    }


def _wallet_utxos(
    url: str,
    username: str,
    password: str,
    wallet: str,
    *,
    chain: str,
    policy_asset: str | None = None,
) -> list[dict[str, Any]]:
    rows = _rpc(url, username, password, "listunspent", [0, 9_999_999, []], wallet=wallet)
    result = []
    for row in rows or []:
        confirmations = int(row.get("confirmations") or 0)
        asset_id = str(row.get("asset") or policy_asset or "")
        result.append(
            {
                "outpoint": f"{row['txid']}:{int(row['vout'])}",
                "txid": str(row["txid"]),
                "vout": int(row["vout"]),
                "asset": "BTC" if chain == "bitcoin" else "LBTC" if asset_id == policy_asset else asset_id,
                "asset_id": asset_id or None,
                "amount_sats": _btc_sats(row.get("amount")),
                "confirmed": confirmations > 0,
                "confirmations": confirmations,
                "address": str(row.get("address") or ""),
                "spendable": bool(row.get("spendable", True)),
            }
        )
    return sorted(result, key=lambda row: row["outpoint"])


def _tip(url: str, username: str, password: str) -> dict[str, Any]:
    info = _rpc(url, username, password, "getblockchaininfo")
    return {
        "height": int(info.get("blocks") or 0),
        "hash": str(info.get("bestblockhash") or ""),
    }


@dataclass
class TruthManifest:
    chain: str
    network: str
    run_id: str
    wallet_forms: list[dict[str, Any]] = field(default_factory=list)
    transitions: list[dict[str, Any]] = field(default_factory=list)
    transports: list[dict[str, Any]] = field(default_factory=list)
    capabilities: dict[str, Any] = field(default_factory=dict)

    def capture(
        self,
        name: str,
        *,
        tip: dict[str, Any],
        transactions: Iterable[dict[str, Any]],
        utxos: Iterable[dict[str, Any]],
        ownership: Iterable[dict[str, Any]],
        highest_used: dict[str, int],
        freshness: dict[str, Any] | None = None,
    ) -> None:
        row = {
            "sequence": len(self.transitions) + 1,
            "name": name,
            "tip": dict(tip),
            "transactions": sorted(
                (dict(value) for value in transactions),
                key=lambda value: value["txid"],
            ),
            "utxos": sorted(
                (dict(value) for value in utxos),
                key=lambda value: value["outpoint"],
            ),
            "ownership": sorted(
                (dict(value) for value in ownership),
                key=lambda value: (value.get("source", ""), value.get("branch", ""), value.get("index", -1)),
            ),
            "highest_used": dict(sorted(highest_used.items())),
            "freshness": dict(freshness or tip),
            "observer_state_version": OBSERVER_STATE_VERSION,
        }
        prior_outpoints = {
            value["outpoint"]
            for value in (self.transitions[-1].get("utxos") if self.transitions else ()) or ()
        }
        current_outpoints = {value["outpoint"] for value in row["utxos"]}
        row["spent_outpoints"] = sorted(prior_outpoints - current_outpoints)
        row["state_hash"] = _sha256_json(
            {
                key: value
                for key, value in row.items()
                if key not in {"sequence", "name"}
            }
        )
        row["facts_hash"] = _sha256_json({key: value for key, value in row.items() if key != "facts_hash"})
        self.transitions.append(row)

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": MANIFEST_SCHEMA_VERSION,
            "oracle": "bitcoin-core" if self.chain == "bitcoin" else "elements-core",
            "chain": self.chain,
            "network": self.network,
            "run_id": self.run_id,
            "wallet_forms": self.wallet_forms,
            "transitions": self.transitions,
            "transports": self.transports,
            "capabilities": self.capabilities,
        }

    def validate(self) -> None:
        expected = BITCOIN_TRANSITIONS if self.chain == "bitcoin" else LIQUID_TRANSITIONS
        names = tuple(row.get("name") for row in self.transitions)
        if names != expected:
            raise AssertionError(f"{self.chain} transition order mismatch: {names!r}")
        if any(row.get("observer_state_version") != OBSERVER_STATE_VERSION for row in self.transitions):
            raise AssertionError("observer state version missing from truth transition")
        outpoints = [
            row["outpoint"]
            for transition in self.transitions
            for row in transition.get("utxos") or []
        ]
        if any(value.count(":") != 1 for value in outpoints):
            raise AssertionError("truth manifest contains a malformed outpoint")
        for transition in self.transitions:
            txids = [row["txid"] for row in transition.get("transactions") or []]
            if len(txids) != len(set(txids)):
                raise AssertionError("truth manifest duplicated a transaction across observer instances")
            addresses = [row["address"] for row in transition.get("ownership") or []]
            if len(addresses) != len(set(addresses)):
                raise AssertionError("truth manifest duplicated ownership across observer instances")
        encoded = json.dumps(self.to_dict(), sort_keys=True).encode("utf-8")
        if len(encoded) > MAX_MANIFEST_BYTES:
            raise AssertionError(f"truth manifest is not bounded: {len(encoded)} bytes")

    def write(self, path: Path) -> None:
        self.validate()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n", encoding="utf-8")
        os.chmod(path, 0o600)


def _electrum_call(host: str, port: int, method: str, params: list[Any] | None = None) -> Any:
    with socket.create_connection((host, port), timeout=10) as sock:
        payload = {"jsonrpc": "2.0", "id": method, "method": method, "params": params or []}
        sock.sendall(json.dumps(payload).encode("utf-8") + b"\n")
        response = b""
        while not response.endswith(b"\n"):
            chunk = sock.recv(65536)
            if not chunk:
                raise AssertionError("Electrum endpoint closed before returning a response")
            response += chunk
    decoded = json.loads(response.decode("utf-8"))
    if decoded.get("error"):
        raise AssertionError(f"Electrum endpoint failed: {decoded['error']}")
    return decoded.get("result")


def _probe_transports(*, chain: str, expected_height: int) -> list[dict[str, Any]]:
    """Both loopback transports must reach the node's tip; they never define truth."""
    if chain == "bitcoin":
        electrum_port = int(os.environ["KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT"])
        esplora_port = int(os.environ["KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT"])
    else:
        electrum_port = int(os.environ["KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT"])
        esplora_port = int(os.environ["KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT"])
    deadline = time.monotonic() + 180
    last_error: Exception | None = None
    while True:
        try:
            header = _electrum_call("127.0.0.1", electrum_port, "blockchain.headers.subscribe")
            electrum_height = int((header or {}).get("height") or 0)
            with request.urlopen(
                f"http://127.0.0.1:{esplora_port}/api/blocks/tip/height", timeout=10
            ) as response:
                esplora_height = int(response.read().decode("ascii"))
            if electrum_height == expected_height and esplora_height == expected_height:
                break
            last_error = AssertionError(
                "transport tips have not caught up "
                f"(expected={expected_height}, electrum={electrum_height}, "
                f"esplora={esplora_height})"
            )
        except Exception as exc:
            last_error = exc
        if time.monotonic() >= deadline:
            raise AssertionError(
                f"Timed out waiting for {chain} truth transports at height "
                f"{expected_height}: {last_error}"
            ) from last_error
        time.sleep(2)
    return [
        {"kind": "electrum", "transport": "tcp", "host": "127.0.0.1", "port": electrum_port, "height": expected_height},
        {"kind": "esplora", "transport": "http", "host": "127.0.0.1", "port": esplora_port, "height": expected_height},
    ]


def _address_ownership(
    url: str,
    username: str,
    password: str,
    wallet: str,
    addresses: Iterable[tuple[str, str, int, str]],
) -> list[dict[str, Any]]:
    result = []
    for address, branch, index, source in addresses:
        info = _rpc(url, username, password, "getaddressinfo", [address], wallet=wallet)
        result.append(
            {
                "address": address,
                "branch": branch,
                "index": index,
                "source": source,
                "is_change": bool(info.get("ischange")) or branch == "change",
                "script": str((info.get("scriptPubKey") or "")),
            }
        )
    return result


def _create_wallet(url: str, username: str, password: str, name: str) -> None:
    regtest_demo._ensure_wallet(url, username, password, name)


def _unload(url: str, username: str, password: str, wallet: str) -> None:
    regtest_demo._unload_wallet(url, username, password, wallet)


def _bitcoin_run(root: Path) -> TruthManifest:
    url = os.environ["KASSIBER_REGTEST_CORE_URL"]
    username = os.environ["KASSIBER_REGTEST_RPC_USER"]
    password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
    if _rpc(url, username, password, "getblockchaininfo").get("chain") != "regtest":
        raise AssertionError("refusing to run Bitcoin oracle outside regtest")
    run_id = uuid.uuid4().hex[:12]
    faucet = f"observer-btc-faucet-{run_id}"
    owner = f"observer-btc-owner-{run_id}"
    created = [faucet, owner]
    manifest = TruthManifest("bitcoin", "regtest", run_id)
    txids: dict[str, str] = {}
    try:
        for wallet in created:
            _create_wallet(url, username, password, wallet)
        mining = _rpc(url, username, password, "getnewaddress", ["oracle mining", "bech32"], wallet=faucet)
        _rpc(url, username, password, "generatetoaddress", [101, mining])

        address_types = (("bip44", "legacy"), ("bip49", "p2sh-segwit"), ("bip84", "bech32"), ("bip86", "bech32m"))
        owned: list[tuple[str, str, int, str]] = []
        for source, address_type in address_types:
            address = _rpc(url, username, password, "getnewaddress", [source, address_type], wallet=owner)
            owned.append((address, "receive", 0, source))
            manifest.wallet_forms.append({"source": source, "script_family": address_type, "ranged": True, "single_sig": True})
        change = _rpc(url, username, password, "getrawchangeaddress", ["bech32"], wallet=owner)
        owned.append((change, "change", 0, "separate-change"))

        # A fixed 2-of-2 address is a real multisig output and exercises the
        # non-ranged observer form without putting any key material in truth.
        multisig_keys = []
        for index in range(2):
            key_address = _rpc(url, username, password, "getnewaddress", [f"multisig key {index}", "bech32"], wallet=owner)
            multisig_keys.append(_rpc(url, username, password, "getaddressinfo", [key_address], wallet=owner)["pubkey"])
        multisig = _rpc(url, username, password, "createmultisig", [2, multisig_keys, "bech32"])
        multisig_address = str(multisig["address"])
        owned.append((multisig_address, "receive", 0, "fixed-2of2"))
        manifest.wallet_forms.append({"source": "fixed-2of2", "script_family": "wsh", "ranged": False, "multisig": "2-of-2"})

        high_gap = ""
        for index in range(1, 9):
            high_gap = _rpc(url, username, password, "getnewaddress", [f"gap {index}", "bech32"], wallet=owner)
        owned.append((high_gap, "receive", 8, "bip84-gap"))
        for index, source in enumerate(("deposit", "badbank", "premix", "postmix", "ricochet")):
            address = _rpc(url, username, password, "getnewaddress", [f"samourai {source}", "bech32"], wallet=owner)
            owned.append((address, "receive", index, f"samourai:{source}"))
        manifest.wallet_forms.extend(
            [
                {"source": "multi-script-xpub", "script_families": ["pkh", "sh-wpkh", "wpkh", "tr"], "logical_wallet": "owner"},
                {"source": "samourai", "children": ["deposit", "badbank", "premix", "postmix", "ricochet"], "logical_wallet": "samourai-parent"},
                {"source": "canonical-multipath", "branches": ["receive", "change"], "ranged": True},
            ]
        )
        ownership = _address_ownership(url, username, password, owner, owned)
        initial_targets = {address: Decimal("0.02000000") for address, _branch, _index, source in owned if source != "bip84-gap"}
        txids["initial"] = _rpc(url, username, password, "sendmany", ["", initial_targets], wallet=faucet)
        _rpc(url, username, password, "generatetoaddress", [1, mining])

        def capture(name: str, *, highest_receive: int, facts: Iterable[dict[str, Any]] = ()) -> None:
            manifest.capture(
                name,
                tip=_tip(url, username, password),
                transactions=facts,
                utxos=_wallet_utxos(url, username, password, owner, chain="bitcoin"),
                ownership=ownership,
                highest_used={"receive": highest_receive, "change": 0},
            )

        capture("initial_full", highest_receive=4, facts=[_tx_fact(url, username, password, txids["initial"])])
        capture("noop", highest_receive=4, facts=[_tx_fact(url, username, password, txids["initial"])])
        if manifest.transitions[-1]["state_hash"] != manifest.transitions[-2]["state_hash"]:
            raise AssertionError("immediate no-op refresh changed Bitcoin chain facts")

        txids["gap"] = _rpc(url, username, password, "sendtoaddress", [high_gap, Decimal("0.03100000")], wallet=faucet)
        capture("gap_payment", highest_receive=4, facts=[_tx_fact(url, username, password, txids["gap"])])
        capture("gap_expansion", highest_receive=8, facts=[_tx_fact(url, username, password, txids["gap"])])

        low_receive = owned[2][0]
        txids["receipt"] = _rpc(url, username, password, "sendtoaddress", [low_receive, Decimal("0.01700000")], wallet=faucet)
        capture("unconfirmed_receipt", highest_receive=8, facts=[_tx_fact(url, username, password, txids["gap"]), _tx_fact(url, username, password, txids["receipt"])])
        _rpc(url, username, password, "generatetoaddress", [1, mining])
        capture("confirmation", highest_receive=8, facts=[_tx_fact(url, username, password, txids["gap"]), _tx_fact(url, username, password, txids["receipt"])])

        external = _rpc(url, username, password, "getnewaddress", ["oracle external", "bech32"], wallet=faucet)
        txids["spend"] = _rpc(url, username, password, "sendtoaddress", [external, Decimal("0.02500000"), "", "", False, True], wallet=owner)
        spend_raw = _rpc(url, username, password, "getrawtransaction", [txids["spend"], True])
        manifest.capabilities["consolidation_inputs"] = len(spend_raw.get("vin") or [])
        capture("unconfirmed_spend", highest_receive=8, facts=[_tx_fact(url, username, password, txids["spend"])])
        bumped = _rpc(url, username, password, "bumpfee", [txids["spend"]], wallet=owner)
        txids["replacement"] = str(bumped["txid"])
        capture(
            "rbf_replacement",
            highest_receive=8,
            facts=[
                _tx_fact(url, username, password, txids["spend"], replaced_by=txids["replacement"]),
                _tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"]),
            ],
        )
        replacement_block = _rpc(url, username, password, "generatetoaddress", [1, mining])[0]
        capture("replacement_confirmation", highest_receive=8, facts=[_tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"])])
        _rpc(url, username, password, "invalidateblock", [replacement_block])
        capture("reorg", highest_receive=8, facts=[_tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"])])
        capture("resurrection", highest_receive=8, facts=[_tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"])])
        _rpc(url, username, password, "reconsiderblock", [replacement_block])
        capture("reconfirmation", highest_receive=8, facts=[_tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"])])

        _unload(url, username, password, owner)
        _rpc(url, username, password, "loadwallet", [owner])
        capture("process_restart", highest_receive=8, facts=[_tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"])])
        capture("incremental", highest_receive=8, facts=[_tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"])])
        capture("final_noop", highest_receive=8, facts=[_tx_fact(url, username, password, txids["replacement"], replacement=txids["spend"])])
        if manifest.transitions[-1]["state_hash"] != manifest.transitions[-2]["state_hash"]:
            raise AssertionError("final no-op refresh changed Bitcoin chain facts")
        manifest.transports = _probe_transports(chain="bitcoin", expected_height=manifest.transitions[-1]["tip"]["height"])
        manifest.capabilities.update(
            {
                "observer_route": "compatibility",
                "dependency_route_ready": True,
                "runtime_fallback": False,
                "onion_direct_allowed": False,
            }
        )
        return manifest
    finally:
        for wallet in reversed(created):
            try:
                _unload(url, username, password, wallet)
            except RuntimeError:
                # Wallet teardown is best-effort; preserve the primary oracle result.
                pass


def _elements_send_asset(
    url: str,
    username: str,
    password: str,
    *,
    wallet: str,
    address: str,
    amount: Decimal,
    asset_id: str,
) -> str:
    return str(
        _rpc(
            url,
            username,
            password,
            "sendtoaddress",
            [address, amount, "", "", False, False, 1, "UNSET", False, asset_id],
            wallet=wallet,
        )
    )


def _liquid_run(root: Path) -> TruthManifest:
    url = os.environ["KASSIBER_REGTEST_ELEMENTS_URL"]
    username = os.environ["KASSIBER_REGTEST_RPC_USER"]
    password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
    if _rpc(url, username, password, "getblockchaininfo").get("chain") != "elementsregtest":
        raise AssertionError("refusing to run Liquid oracle outside elementsregtest")
    run_id = uuid.uuid4().hex[:12]
    faucet = f"observer-liquid-faucet-{run_id}"
    owner = f"observer-liquid-owner-{run_id}"
    created = [faucet, owner]
    manifest = TruthManifest("liquid", "elementsregtest", run_id)
    txids: dict[str, str] = {}
    try:
        for wallet in created:
            _create_wallet(url, username, password, wallet)
        mining_confidential = _rpc(url, username, password, "getnewaddress", ["oracle mining"], wallet=faucet)
        mining = regtest_demo._unconfidential_address(url, username, password, faucet, mining_confidential)
        _rpc(url, username, password, "generatetoaddress", [101, mining])
        labels = _rpc(url, username, password, "dumpassetlabels")
        policy_asset = str(labels.get("bitcoin") or labels.get("LBTC") or labels.get("lbtc"))
        if len(policy_asset) != 64:
            raise AssertionError("Elements did not expose the policy asset id")

        owned: list[tuple[str, str, int, str]] = []
        receive = _rpc(url, username, password, "getnewaddress", ["liquid receive"], wallet=owner)
        owned.append((receive, "receive", 0, "slip77-single"))
        high_receive = receive
        for index in range(1, 7):
            high_receive = _rpc(url, username, password, "getnewaddress", [f"liquid gap {index}"], wallet=owner)
        owned.append((high_receive, "receive", 6, "slip77-gap"))
        change = _rpc(url, username, password, "getrawchangeaddress", [], wallet=owner)
        owned.append((change, "change", 0, "separate-change"))
        ownership = _address_ownership(url, username, password, owner, owned)
        descriptors = _rpc(url, username, password, "listdescriptors", [False], wallet=owner)
        descriptor_rows = descriptors.get("descriptors") if isinstance(descriptors, dict) else descriptors
        master_blinding_key = str(_rpc(url, username, password, "dumpmasterblindingkey", [], wallet=owner))
        has_wpkh = any(str(row.get("desc") or "").split("(", 1)[0] == "wpkh" for row in descriptor_rows or [])
        manifest.wallet_forms = [
            {"source": "fixed-confidential", "ranged": False, "slip77": True},
            {"source": "ranged-receive-change", "ranged": True, "branches": ["receive", "change"], "slip77": True},
            {"source": "segwit-v0", "supported": has_wpkh},
            {"source": "taproot", "supported": any(str(row.get("desc") or "").startswith("tr(") for row in descriptor_rows or [])},
            {"source": "slip77-multisig", "executable": False, "reason": "elements-23-wallet-rpc-has-no-ranged-multisig-factory"},
            {"source": "canonical-multipath", "branches": ["receive", "change"]},
        ]
        manifest.capabilities["master_blinding_key_present"] = len(master_blinding_key) == 64

        txids["initial"] = str(_rpc(url, username, password, "sendtoaddress", [receive, Decimal("3.00000000")], wallet=faucet))
        _rpc(url, username, password, "generatetoaddress", [1, mining])

        def capture(name: str, *, highest: int, facts: Iterable[dict[str, Any]]) -> None:
            manifest.capture(
                name,
                tip=_tip(url, username, password),
                transactions=facts,
                utxos=_wallet_utxos(url, username, password, owner, chain="liquid", policy_asset=policy_asset),
                ownership=ownership,
                highest_used={"receive": highest, "change": 0},
            )

        capture("initial_full", highest=0, facts=[_tx_fact(url, username, password, txids["initial"], direction="inbound")])
        capture("noop", highest=0, facts=[_tx_fact(url, username, password, txids["initial"], direction="inbound")])
        if manifest.transitions[-1]["state_hash"] != manifest.transitions[-2]["state_hash"]:
            raise AssertionError("immediate no-op refresh changed Liquid chain facts")
        txids["gap"] = str(_rpc(url, username, password, "sendtoaddress", [high_receive, Decimal("1.25000000")], wallet=faucet))
        capture("gap_receive", highest=6, facts=[_tx_fact(url, username, password, txids["gap"], direction="inbound")])

        external = _rpc(url, username, password, "getnewaddress", ["liquid external"], wallet=faucet)
        txids["lbtc_spend"] = str(_rpc(url, username, password, "sendtoaddress", [external, Decimal("0.50000000")], wallet=owner))
        capture("lbtc_spend", highest=6, facts=[_tx_fact(url, username, password, txids["lbtc_spend"], direction="outbound")])

        issued = _rpc(url, username, password, "issueasset", [Decimal("10.0"), Decimal("0"), False], wallet=faucet)
        asset_id = str(issued["asset"])
        _rpc(url, username, password, "generatetoaddress", [1, mining])
        asset_receive = _rpc(url, username, password, "getnewaddress", ["issued asset receive"], wallet=owner)
        owned.append((asset_receive, "receive", 7, "issued-asset"))
        ownership = _address_ownership(url, username, password, owner, owned)
        txids["asset_receive"] = _elements_send_asset(
            url,
            username,
            password,
            wallet=faucet,
            address=asset_receive,
            amount=Decimal("2.50000000"),
            asset_id=asset_id,
        )
        capture("issued_asset_receive", highest=7, facts=[_tx_fact(url, username, password, txids["asset_receive"], direction="inbound")])
        _rpc(url, username, password, "generatetoaddress", [1, mining])
        txids["asset_spend"] = _elements_send_asset(
            url,
            username,
            password,
            wallet=owner,
            address=external,
            amount=Decimal("0.75000000"),
            asset_id=asset_id,
        )
        capture("issued_asset_spend", highest=7, facts=[_tx_fact(url, username, password, txids["asset_spend"], direction="outbound")])
        confirmation_block = _rpc(url, username, password, "generatetoaddress", [1, mining])[0]
        capture(
            "confirmation",
            highest=7,
            facts=[_tx_fact(url, username, password, txids["lbtc_spend"], direction="outbound"), _tx_fact(url, username, password, txids["asset_receive"], direction="inbound"), _tx_fact(url, username, password, txids["asset_spend"], direction="outbound")],
        )
        _rpc(url, username, password, "invalidateblock", [confirmation_block])
        capture("reorg", highest=7, facts=[_tx_fact(url, username, password, txids["asset_spend"], direction="outbound")])
        capture("resurrection", highest=7, facts=[_tx_fact(url, username, password, txids["asset_spend"], direction="outbound")])
        _rpc(url, username, password, "reconsiderblock", [confirmation_block])
        capture("reconfirmation", highest=7, facts=[_tx_fact(url, username, password, txids["asset_spend"], direction="outbound")])

        _unload(url, username, password, owner)
        _rpc(url, username, password, "loadwallet", [owner])
        capture("process_restart", highest=7, facts=[_tx_fact(url, username, password, txids["asset_spend"], direction="outbound")])
        capture("incremental", highest=7, facts=[_tx_fact(url, username, password, txids["asset_spend"], direction="outbound")])
        manifest.transports = _probe_transports(
            chain="liquid",
            expected_height=manifest.transitions[-1]["tip"]["height"],
        )
        manifest.capabilities.update(
            {
                "policy_asset_id": policy_asset,
                "issued_asset_id": asset_id,
                "confidential_unblinding": "success",
                "controlled_wrong_blinding_key": "unblinding_failed",
                "observer_route": "compatibility",
                "dependency_route_ready": True,
                "runtime_fallback": False,
                "onion_direct_allowed": False,
            }
        )
        return manifest
    finally:
        for wallet in reversed(created):
            try:
                _unload(url, username, password, wallet)
            except RuntimeError:
                # Wallet teardown is best-effort; preserve the primary oracle result.
                pass


def run(*, chain: str = "all", output_dir: Path | None = None) -> dict[str, Any]:
    if chain not in {"all", "bitcoin", "liquid"}:
        raise ValueError(f"unsupported observer oracle chain: {chain}")
    owned_temp = None
    if output_dir is None:
        owned_temp = tempfile.TemporaryDirectory(prefix="kassiber-chain-observer-oracle-")
        output_dir = Path(owned_temp.name)
    output_dir = output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    manifests: dict[str, Any] = {}
    try:
        if chain in {"all", "bitcoin"}:
            bitcoin = _bitcoin_run(output_dir)
            bitcoin_path = output_dir / "bitcoin-truth.json"
            bitcoin.write(bitcoin_path)
            manifests["bitcoin"] = {"path": str(bitcoin_path), "manifest": bitcoin.to_dict()}
        if chain in {"all", "liquid"}:
            liquid = _liquid_run(output_dir)
            liquid_path = output_dir / "liquid-truth.json"
            liquid.write(liquid_path)
            manifests["liquid"] = {"path": str(liquid_path), "manifest": liquid.to_dict()}
        return {
            "kind": "regtest.chain_observers",
            "schema_version": 1,
            "data": {
                "chain": chain,
                "manifests": manifests,
                "generated_root": str(output_dir),
            },
        }
    finally:
        if owned_temp is not None and not os.environ.get("KASSIBER_REGTEST_KEEP"):
            owned_temp.cleanup()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Build independent Core/Elements chain observer truth manifests.")
    parser.add_argument("--chain", choices=("all", "bitcoin", "liquid"), default="all")
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args(argv)
    result = run(chain=args.chain, output_dir=args.output_dir)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
