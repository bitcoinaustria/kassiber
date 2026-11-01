"""Raw Electrum transactions must match the ID requested for every graph leg."""

from unittest.mock import patch

import pytest
from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput, Witness

from kassiber.core import transaction_graph as graph
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_transaction_graph import _FakeElectrumClient


def transactions():
    parent = Transaction(vin=[TransactionInput(bytes(32), 0xFFFFFFFF)], vout=[TransactionOutput(100000, Script(b"\x51"))])
    current = Transaction(vin=[TransactionInput(parent.txid(), 0)], vout=[TransactionOutput(99000, Script(b"\x51"))])
    other = Transaction(vin=[TransactionInput(parent.txid(), 0)], vout=[TransactionOutput(98000, Script(b"\x51"))])
    return parent, current, other


@pytest.mark.parametrize("mismatch", ["current", "parent"])
def test_electrum_graph_rejects_mismatched_current_and_parent_without_caching(tmp_path, mismatch):
    parent, current, other = transactions()
    responses = {parent.txid().hex(): parent.serialize().hex(), current.txid().hex(): current.serialize().hex()}
    responses[(current if mismatch == "current" else parent).txid().hex()] = other.serialize().hex()
    conn = open_db(tmp_path / "data")
    try:
        with patch.object(graph, "ElectrumClient", _FakeElectrumClient), patch.object(_FakeElectrumClient, "responses", responses):
            with pytest.raises(AppError) as raised:
                graph._fetch_bitcoin_electrum_graph_raw(conn, {"kind": "electrum", "url": "tcp://fixture:50001"}, "bitcoin", "main", current.txid().hex())
        assert raised.value.code == "invalid_observation"
        assert conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0] == 0
    finally:
        conn.close()


def test_electrum_identity_uses_txid_instead_of_witness_hash(tmp_path):
    parent, current, _ = transactions()
    current.vin[0].witness = Witness([b"synthetic witness"])
    responses = {tx.txid().hex(): tx.serialize().hex() for tx in (parent, current)}
    conn = open_db(tmp_path / "data")
    try:
        with patch.object(graph, "ElectrumClient", _FakeElectrumClient), patch.object(_FakeElectrumClient, "responses", responses):
            raw = graph._fetch_bitcoin_electrum_graph_raw(conn, {"kind": "electrum", "url": "tcp://fixture:50001"}, "bitcoin", "main", current.txid().hex())
        assert raw["txid"] == current.txid().hex()
        assert raw["vin"][0]["prevout"]["value"] == 100000
    finally:
        conn.close()


def test_liquid_electrum_graph_rejects_different_transaction_id(tmp_path):
    from tests.test_transaction_graph import _FakeLiquidTx

    requested = "aa" * 32
    decoded = _FakeLiquidTx([], [])
    decoded.txid = lambda: bytes.fromhex("bb" * 32)
    conn = open_db(tmp_path / "data")
    try:
        with patch.object(graph, "ElectrumClient", _FakeElectrumClient), patch.object(
            _FakeElectrumClient, "responses", {requested: "synthetic-liquid-bytes"}
        ), patch.object(graph, "decode_liquid_transaction", return_value=decoded):
            with pytest.raises(AppError) as raised:
                graph._fetch_liquid_electrum_graph_raw(conn, {"kind": "electrum", "url": "tcp://fixture:50001"}, "liquid", "liquidv1", requested)
        assert raised.value.code == "invalid_observation"
        assert conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0] == 0
    finally:
        conn.close()
