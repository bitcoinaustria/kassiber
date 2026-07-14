from __future__ import annotations

import json
import os
import tempfile
import time
import unittest
import uuid
from decimal import Decimal
from pathlib import Path
from urllib import request

from kassiber.db import open_db
from tests.integration.env import skip_unless_integration
from tests.integration.test_live_bitcoin_electrum_parity import (
    _electrum_url,
    _electrum_call,
    _rpc,
    _run,
    _sync,
    _transaction_projection,
    _utxo_projection,
    _wait_for_electrum,
)


def _wait_for_esplora(url: str, txid: str, *, confirmed: bool) -> None:
    # The deterministic regtest Esplora oracle bounds its full-chain script
    # index cache to two seconds. Let that cache expire before using direct tx
    # status as the readiness signal, otherwise a raw-tx fallback can be newer
    # than the script history BDK scans immediately afterward.
    time.sleep(2.1)
    deadline = time.monotonic() + 60
    last = None
    while time.monotonic() < deadline:
        try:
            with request.urlopen(f"{url.rstrip('/')}/tx/{txid}/status", timeout=5) as response:
                last = json.load(response)
            if bool(last.get("confirmed")) is confirmed:
                return
        except Exception as exc:
            last = exc
        time.sleep(1)
    raise AssertionError(f"Esplora did not report {txid} confirmed={confirmed}: {last}")


def _wait_for_electrum_height(url: str, expected: int) -> None:
    deadline = time.monotonic() + 60
    last = None
    while time.monotonic() < deadline:
        try:
            last = _electrum_call(url, "blockchain.headers.subscribe")
            if int((last or {}).get("height") or 0) == expected:
                return
        except Exception as exc:
            last = exc
        time.sleep(1)
    raise AssertionError(f"Electrum did not reach height {expected}: {last}")


def _wait_for_electrum_confirmation(
    url: str,
    txid: str,
    *,
    height: int,
    confirmed: bool,
) -> None:
    """Wait until Electrum's transaction index agrees with its chain tip.

    A readable raw transaction only proves that Fulcrum knows the transaction;
    mempool transactions remain readable while a newly-arrived block is still
    being indexed.  BDK consumes the indexed history/merkle state, so the live
    oracle must wait for that same state before comparing transports.
    """

    deadline = time.monotonic() + 60
    last = None
    while time.monotonic() < deadline:
        try:
            last = _electrum_call(
                url,
                "blockchain.transaction.get_merkle",
                [txid, height],
            )
            if confirmed and int((last or {}).get("block_height") or 0) == height:
                return
        except Exception as exc:
            last = exc
            if not confirmed:
                try:
                    _electrum_call(url, "blockchain.transaction.get", [txid])
                    return
                except Exception as raw_exc:
                    last = raw_exc
        time.sleep(1)
    raise AssertionError(
        f"Electrum did not report {txid} confirmed={confirmed} at height {height}: {last}"
    )


def _create_book(
    root: Path,
    *,
    backend_kind: str,
    backend_url: str,
    receive: str,
    change: str,
) -> None:
    _run(root, "init")
    _run(root, "workspaces", "create", "BDK")
    _run(
        root,
        "profiles",
        "create",
        "Default",
        "--workspace",
        "BDK",
        "--fiat-currency",
        "EUR",
        "--tax-country",
        "generic",
        "--gains-algorithm",
        "FIFO",
    )
    _run(
        root,
        "backends",
        "create",
        f"{backend_kind}-regtest",
        "--kind",
        backend_kind,
        "--url",
        backend_url,
        "--chain",
        "bitcoin",
        "--network",
        "regtest",
        "--timeout",
        "30",
        "--batch-size",
        "25",
    )
    with tempfile.TemporaryDirectory(prefix="kassiber-bdk-descriptors-") as tmp:
        receive_path = Path(tmp) / "receive.txt"
        change_path = Path(tmp) / "change.txt"
        receive_path.write_text(receive + "\n", encoding="utf-8")
        change_path.write_text(change + "\n", encoding="utf-8")
        _run(
            root,
            "wallets",
            "create",
            "--workspace",
            "BDK",
            "--profile",
            "Default",
            "--label",
            "BDK watch",
            "--kind",
            "descriptor",
            "--backend",
            f"{backend_kind}-regtest",
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--descriptor-file",
            str(receive_path),
            "--change-descriptor-file",
            str(change_path),
            "--gap-limit",
            "5",
        )


def _observer_snapshot(root: Path) -> tuple[str, int, int, int]:
    conn = open_db(root)
    try:
        state = conn.execute(
            "SELECT state_json FROM chain_observer_instances WHERE observer_kind = 'bdk'"
        ).fetchone()
        if state is None:
            raise AssertionError("BDK observer state was not persisted")
        tx_count = int(conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0])
        utxo_count = int(conn.execute("SELECT COUNT(*) FROM wallet_utxos WHERE spent_at IS NULL").fetchone()[0])
        coverage_count = int(conn.execute("SELECT COUNT(*) FROM chain_observer_coverage").fetchone()[0])
        return str(state["state_json"]), tx_count, utxo_count, coverage_count
    finally:
        conn.close()


def _transport_projection(root: Path) -> list[dict]:
    rows = _transaction_projection(root)
    for row in rows:
        # `occurred_at` is intentionally the dependency's first observation,
        # which can be either mempool time or block time when two independent
        # indexers converge at different speeds. Confirmation truth remains
        # exact in `confirmed_at`; transport parity must not compare discovery
        # timing even when one backend first sees the transaction confirmed.
        row["occurred_at"] = "<transport-observed>"
    return rows


@skip_unless_integration
class LiveBdkObserverTest(unittest.TestCase):
    def test_esplora_and_electrum_descriptor_routes_match_and_restart(self):
        core_url = os.environ.get("KASSIBER_REGTEST_CORE_URL", "http://127.0.0.1:18443")
        username = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
        password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")
        electrum_url = _electrum_url()
        esplora_url = os.environ.get("KASSIBER_REGTEST_ESPLORA_URL") or (
            "http://127.0.0.1:"
            + os.environ.get("KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT", "18544")
            + "/api"
        )
        run_id = uuid.uuid4().hex[:12]
        faucet = f"kassiber-bdk-faucet-{run_id}"
        owner = f"kassiber-bdk-owner-{run_id}"
        created = [faucet, owner]
        try:
            _rpc(core_url, username, password, "createwallet", [faucet, False, False, "", False, True, True])
            _rpc(core_url, username, password, "createwallet", [owner, False, False, "", False, True, True])
            mining = _rpc(core_url, username, password, "getnewaddress", ["mining", "bech32"], wallet=faucet)
            receive_addresses = [
                _rpc(core_url, username, password, "getnewaddress", [f"receive {index}", "bech32"], wallet=owner)
                for index in range(5)
            ]
            descriptors = _rpc(core_url, username, password, "listdescriptors", [False], wallet=owner)[
                "descriptors"
            ]
            receive = next(
                row["desc"] for row in descriptors if row.get("active") and not row.get("internal") and row["desc"].startswith("wpkh(")
            )
            change = next(
                row["desc"] for row in descriptors if row.get("active") and row.get("internal") and row["desc"].startswith("wpkh(")
            )
            _rpc(core_url, username, password, "generatetoaddress", [101, mining])
            funding = _rpc(
                core_url,
                username,
                password,
                "sendmany",
                ["", {receive_addresses[0]: 0.125}],
                wallet=faucet,
            )
            _rpc(core_url, username, password, "generatetoaddress", [1, mining])
            height = int(_rpc(core_url, username, password, "getblockcount"))
            _wait_for_electrum(electrum_url, min_height=height, txids=[funding])
            _wait_for_electrum_confirmation(
                electrum_url,
                funding,
                height=height,
                confirmed=True,
            )

            with tempfile.TemporaryDirectory(prefix="kassiber-live-bdk-") as tmp:
                roots = {
                    "electrum": Path(tmp) / "electrum",
                    "esplora": Path(tmp) / "esplora",
                }
                _create_book(
                    roots["electrum"],
                    backend_kind="electrum",
                    backend_url=electrum_url,
                    receive=receive,
                    change=change,
                )
                _create_book(
                    roots["esplora"],
                    backend_kind="esplora",
                    backend_url=esplora_url,
                    receive=receive,
                    change=change,
                )
                states = {}
                for kind, root in roots.items():
                    first = _sync(root, "BDK watch")
                    self.assertEqual(first["observer_route"], "bdk")
                    state_before, tx_count, utxo_count, coverage_count = _observer_snapshot(root)
                    self.assertEqual(tx_count, 1, f"{kind}: {first}")
                    self.assertEqual(utxo_count, 1, f"{kind}: {first}")
                    self.assertEqual(coverage_count, 2)
                    payload = json.loads(state_before)
                    self.assertEqual(payload["schema_version"], 1)
                    self.assertIn("bdk_changeset", payload)
                    states[kind] = state_before
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                self.assertEqual(
                    _utxo_projection(roots["electrum"]),
                    _utxo_projection(roots["esplora"]),
                )

                # Exercise gap expansion after initial state exists.
                gap_txid = _rpc(
                    core_url,
                    username,
                    password,
                    "sendtoaddress",
                    [receive_addresses[4], 0.075],
                    wallet=faucet,
                )
                _rpc(core_url, username, password, "generatetoaddress", [1, mining])
                height = int(_rpc(core_url, username, password, "getblockcount"))
                _wait_for_electrum(electrum_url, min_height=height, txids=[gap_txid])
                _wait_for_electrum_confirmation(
                    electrum_url,
                    gap_txid,
                    height=height,
                    confirmed=True,
                )
                for kind, root in roots.items():
                    expanded = _sync(root, "BDK watch")
                    self.assertEqual(expanded["observer_route"], "bdk")
                    _state, tx_count, utxo_count, _coverage_count = _observer_snapshot(root)
                    self.assertEqual(tx_count, 2)
                    self.assertEqual(utxo_count, 2)
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                self.assertEqual(
                    _utxo_projection(roots["electrum"]),
                    _utxo_projection(roots["esplora"]),
                )

                # Outbound observation includes an exact on-chain fee while
                # the transaction is still unconfirmed.
                external = _rpc(
                    core_url,
                    username,
                    password,
                    "getnewaddress",
                    ["BDK external", "bech32"],
                    wallet=faucet,
                )
                spend_txid = _rpc(
                    core_url,
                    username,
                    password,
                    "sendtoaddress",
                    [external, 0.05, "", "", False, True],
                    wallet=owner,
                )
                _wait_for_electrum(electrum_url, min_height=height, txids=[spend_txid])
                _wait_for_esplora(esplora_url, spend_txid, confirmed=False)
                for kind, root in roots.items():
                    try:
                        outcome = _sync(root, "BDK watch")
                    except AssertionError as exc:
                        raise AssertionError(f"{kind} outbound refresh failed: {exc}") from exc
                    self.assertEqual(outcome["observer_route"], "bdk", kind)
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                spend = next(
                    row
                    for row in _transaction_projection(roots["electrum"])
                    if row["external_id"] == spend_txid
                )
                self.assertEqual(spend["direction"], "outbound")
                self.assertIsNone(spend["confirmed_at"])
                self.assertNotEqual(spend["occurred_at"], "1970-01-01T00:00:00Z")
                self.assertGreater(Decimal(str(spend["fee"])), Decimal("0"))

                # RBF replaces the prior canonical tx rather than leaving two
                # competing accounting records.
                bumped = _rpc(core_url, username, password, "bumpfee", [spend_txid], wallet=owner)
                replacement_txid = str(bumped["txid"])
                _wait_for_electrum(electrum_url, min_height=height, txids=[replacement_txid])
                _wait_for_esplora(esplora_url, replacement_txid, confirmed=False)
                for kind, root in roots.items():
                    outcome = _sync(root, "BDK watch")
                    self.assertEqual(outcome["observer_route"], "bdk", kind)
                electrum_rows = _transport_projection(roots["electrum"])
                self.assertEqual(electrum_rows, _transport_projection(roots["esplora"]))
                self.assertNotIn(spend_txid, {row["external_id"] for row in electrum_rows})
                self.assertIn(replacement_txid, {row["external_id"] for row in electrum_rows})

                replacement_block = _rpc(
                    core_url,
                    username,
                    password,
                    "generatetoaddress",
                    [1, mining],
                )[0]
                height = int(_rpc(core_url, username, password, "getblockcount"))
                _wait_for_electrum(electrum_url, min_height=height, txids=[replacement_txid])
                _wait_for_electrum_confirmation(
                    electrum_url,
                    replacement_txid,
                    height=height,
                    confirmed=True,
                )
                _wait_for_esplora(esplora_url, replacement_txid, confirmed=True)
                for root in roots.values():
                    _sync(root, "BDK watch")
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                confirmed = next(
                    row
                    for row in _transaction_projection(roots["electrum"])
                    if row["external_id"] == replacement_txid
                )
                self.assertIsNotNone(confirmed["confirmed_at"])

                # Replace the confirming block with an equal-height empty
                # competitor. A merely shorter backend is indistinguishable
                # from a lagging indexer and must fail retryably; the competing
                # block gives BDK a real hash mismatch to roll back safely.
                _rpc(core_url, username, password, "invalidateblock", [replacement_block])
                alternate_block = _rpc(
                    core_url,
                    username,
                    password,
                    "generateblock",
                    [mining, []],
                )["hash"]
                self.assertNotEqual(alternate_block, replacement_block)
                height = int(_rpc(core_url, username, password, "getblockcount"))
                _wait_for_electrum_height(electrum_url, height)
                _wait_for_electrum_confirmation(
                    electrum_url,
                    replacement_txid,
                    height=height,
                    confirmed=False,
                )
                _wait_for_esplora(esplora_url, replacement_txid, confirmed=False)
                for root in roots.values():
                    _sync(root, "BDK watch")
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                reorged = next(
                    row
                    for row in _transaction_projection(roots["electrum"])
                    if row["external_id"] == replacement_txid
                )
                self.assertIsNone(reorged["confirmed_at"])

                # Mine the resurrected mempool transaction on the competing
                # branch to prove reconfirmation after the rollback.
                _rpc(core_url, username, password, "generatetoaddress", [1, mining])
                height = int(_rpc(core_url, username, password, "getblockcount"))
                _wait_for_electrum_height(electrum_url, height)
                _wait_for_electrum_confirmation(
                    electrum_url,
                    replacement_txid,
                    height=height,
                    confirmed=True,
                )
                _wait_for_esplora(esplora_url, replacement_txid, confirmed=True)
                for root in roots.values():
                    _sync(root, "BDK watch")
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                reconfirmed = next(
                    row
                    for row in _transaction_projection(roots["electrum"])
                    if row["external_id"] == replacement_txid
                )
                self.assertIsNotNone(reconfirmed["confirmed_at"])

                # A real two-input opt-in RBF replacement drops one owned
                # input. BDK must evict the superseded transaction and
                # resurrect that dropped input as an unspent output on both
                # transports; a graph that only appends transactions cannot
                # satisfy this transition.
                funding_a = _rpc(
                    core_url,
                    username,
                    password,
                    "sendtoaddress",
                    [receive_addresses[1], 0.03],
                    wallet=faucet,
                )
                funding_b = _rpc(
                    core_url,
                    username,
                    password,
                    "sendtoaddress",
                    [receive_addresses[2], 0.04],
                    wallet=faucet,
                )
                _rpc(core_url, username, password, "generatetoaddress", [1, mining])
                height = int(_rpc(core_url, username, password, "getblockcount"))
                _wait_for_electrum(
                    electrum_url,
                    min_height=height,
                    txids=[funding_a, funding_b],
                )
                for funding_txid in (funding_a, funding_b):
                    _wait_for_electrum_confirmation(
                        electrum_url,
                        funding_txid,
                        height=height,
                        confirmed=True,
                    )
                _wait_for_esplora(esplora_url, funding_a, confirmed=True)
                _wait_for_esplora(esplora_url, funding_b, confirmed=True)
                for root in roots.values():
                    _sync(root, "BDK watch")
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                self.assertEqual(
                    _utxo_projection(roots["electrum"]),
                    _utxo_projection(roots["esplora"]),
                )
                owned_utxos = _rpc(
                    core_url,
                    username,
                    password,
                    "listunspent",
                    [1, 9_999_999, []],
                    wallet=owner,
                )
                input_a = next(row for row in owned_utxos if row["txid"] == funding_a)
                input_b = next(row for row in owned_utxos if row["txid"] == funding_b)
                eviction_destination = _rpc(
                    core_url,
                    username,
                    password,
                    "getnewaddress",
                    ["BDK eviction destination", "bech32"],
                    wallet=faucet,
                )

                def signed_raw(inputs, amount):
                    raw = _rpc(
                        core_url,
                        username,
                        password,
                        "createrawtransaction",
                        [inputs, {eviction_destination: amount}],
                    )
                    signed = _rpc(
                        core_url,
                        username,
                        password,
                        "signrawtransactionwithwallet",
                        [raw],
                        wallet=owner,
                    )
                    self.assertTrue(signed.get("complete"), signed)
                    return signed["hex"]

                rbf_sequence = 4_294_967_293
                evicted_txid = _rpc(
                    core_url,
                    username,
                    password,
                    "sendrawtransaction",
                    [
                        signed_raw(
                            [
                                {
                                    "txid": input_a["txid"],
                                    "vout": input_a["vout"],
                                    "sequence": rbf_sequence,
                                },
                                {
                                    "txid": input_b["txid"],
                                    "vout": input_b["vout"],
                                    "sequence": rbf_sequence,
                                },
                            ],
                            0.06999,
                        )
                    ],
                )
                _wait_for_electrum(electrum_url, min_height=height, txids=[evicted_txid])
                _wait_for_esplora(esplora_url, evicted_txid, confirmed=False)
                for root in roots.values():
                    _sync(root, "BDK watch")
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                self.assertEqual(
                    _utxo_projection(roots["electrum"]),
                    _utxo_projection(roots["esplora"]),
                )
                spent_b = next(
                    row
                    for row in _utxo_projection(roots["electrum"])
                    if row["txid"] == funding_b and row["vout"] == input_b["vout"]
                )
                self.assertTrue(spent_b["spent"])

                resurrecting_txid = _rpc(
                    core_url,
                    username,
                    password,
                    "sendrawtransaction",
                    [
                        signed_raw(
                            [
                                {
                                    "txid": input_a["txid"],
                                    "vout": input_a["vout"],
                                    "sequence": rbf_sequence,
                                }
                            ],
                            0.02997,
                        )
                    ],
                )
                self.assertNotEqual(resurrecting_txid, evicted_txid)
                _wait_for_electrum(
                    electrum_url,
                    min_height=height,
                    txids=[resurrecting_txid],
                )
                _wait_for_esplora(esplora_url, resurrecting_txid, confirmed=False)
                for root in roots.values():
                    _sync(root, "BDK watch")
                electrum_rows = _transport_projection(roots["electrum"])
                self.assertEqual(
                    electrum_rows,
                    _transport_projection(roots["esplora"]),
                )
                self.assertNotIn(
                    evicted_txid,
                    {row["external_id"] for row in electrum_rows},
                )
                self.assertIn(
                    resurrecting_txid,
                    {row["external_id"] for row in electrum_rows},
                )
                electrum_utxos = _utxo_projection(roots["electrum"])
                self.assertEqual(electrum_utxos, _utxo_projection(roots["esplora"]))
                resurrected_b = next(
                    row
                    for row in electrum_utxos
                    if row["txid"] == funding_b and row["vout"] == input_b["vout"]
                )
                self.assertFalse(resurrected_b["spent"])

                _rpc(core_url, username, password, "generatetoaddress", [1, mining])
                height = int(_rpc(core_url, username, password, "getblockcount"))
                _wait_for_electrum(
                    electrum_url,
                    min_height=height,
                    txids=[resurrecting_txid],
                )
                _wait_for_electrum_confirmation(
                    electrum_url,
                    resurrecting_txid,
                    height=height,
                    confirmed=True,
                )
                _wait_for_esplora(esplora_url, resurrecting_txid, confirmed=True)
                for root in roots.values():
                    _sync(root, "BDK watch")
                self.assertEqual(
                    _transport_projection(roots["electrum"]),
                    _transport_projection(roots["esplora"]),
                )
                self.assertEqual(
                    _utxo_projection(roots["electrum"]),
                    _utxo_projection(roots["esplora"]),
                )

                # New CLI processes load each aggregate through custom
                # Persistence; immediate no-ops keep exact state JSON.
                for kind, root in roots.items():
                    state_before = _observer_snapshot(root)[0]
                    noop = _sync(root, "BDK watch")
                    self.assertEqual(noop["observer_route"], "bdk")
                    state_after, *_ = _observer_snapshot(root)
                    self.assertEqual(state_after, state_before)

                base = Path(tmp)
                banned = list(base.rglob("*.sqlite")) + list(base.rglob("*.db-wal"))
                self.assertEqual(banned, [])
        finally:
            for wallet in reversed(created):
                try:
                    _rpc(core_url, username, password, "unloadwallet", [wallet])
                except Exception:
                    pass


if __name__ == "__main__":
    unittest.main()
