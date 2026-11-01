"""Raw history, prevout, output-inventory and verification bytes bind to txid."""

from contextlib import nullcontext

import pytest
from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput
from embit.liquid.transaction import LTransaction, LTransactionInput, LTransactionOutput

from kassiber.core import sync_backends as sb
from kassiber.core.sync import WalletSyncState
from kassiber.errors import AppError


BACKEND = {"name": "fake", "kind": "electrum", "url": "tcp://unused.invalid:1"}


def _coinbase(value=1000):
    return Transaction(vin=[TransactionInput(b"\0" * 32, 0xffffffff, Script(b"\x01\x01"))],
                       vout=[TransactionOutput(value, Script(b"\x51"))])


def _state(chain="bitcoin", **kwargs):
    target = {"script_pubkey": "51", "branch_index": 0, "address_index": 0}
    return WalletSyncState(chain=chain, network="regtest", descriptor_plan=None, policy_asset_id="",
                           targets=[target], tracked_scripts={"51": target},
                           history_cache=kwargs.get("history_cache", {}), checkpoint=kwargs.get("checkpoint", {}))


class Node:
    def __init__(self, txid, raw):
        self.txid, self.raw = txid, raw
        self.calls = []

    def call(self, method, params):
        self.calls.append((method, params))
        if method == "blockchain.headers.subscribe":
            return {"height": 100, "hex": "00" * 80}
        if method == "blockchain.block.header":
            return "00" * 68 + "00f15365" + "00" * 8
        if method == "blockchain.scripthash.subscribe":
            return "status"
        if method == "blockchain.scripthash.get_history":
            return [{"tx_hash": self.txid, "height": 0}]
        if method == "blockchain.scripthash.listunspent":
            return [{"tx_hash": self.txid, "tx_pos": 0, "height": 0, "value": 1000}]
        assert method == "blockchain.transaction.get"
        return self.raw[params[0]]

    def batch_call(self, calls):
        return [self.call(*call) for call in calls]


@pytest.mark.parametrize("path", ["history", "parent", "lazy-parent"])
def test_wallet_sync_rejects_wrong_raw_identity_on_every_fetch_path(monkeypatch, path):
    parent = _coinbase()
    parent_id = parent.txid().hex()
    child = Transaction(vin=[TransactionInput(parent.txid(), 0)], vout=[TransactionOutput(900, Script(b"\x51"))])
    child_id = child.txid().hex()
    requested = child_id if path == "parent" else "11" * 32
    raw = {requested: child.serialize().hex() if path == "parent" else parent.serialize().hex(),
           parent_id: _coinbase(2000).serialize().hex()}
    if path == "lazy-parent":
        requested = parent_id
        raw[requested] = parent.serialize().hex()
        raw["22" * 32] = parent.serialize().hex()
        monkeypatch.setattr(sb, "_normalize_electrum_bitcoin_graph_for_storage",
                            lambda tx, lookup: lookup("22" * 32))
    node = Node(requested, raw)
    monkeypatch.setattr(sb, "_electrum_client_context", lambda backend: nullcontext(node))
    with pytest.raises(AppError) as raised:
        sb.compatibility_electrum_records_for_wallet(BACKEND, _state())
    assert raised.value.code == "invalid_observation"


@pytest.mark.parametrize("reuse_client", [True, False])
def test_onchain_verification_rejects_wrong_transaction_bytes(monkeypatch, reuse_client):
    node = Node("11" * 32, {"11" * 32: _coinbase().serialize().hex()})
    monkeypatch.setattr(sb, "_electrum_client_context", lambda backend: nullcontext(node))
    with pytest.raises(AppError) as raised:
        sb.fetch_transaction_legs(BACKEND, node.txid, chain="bitcoin", client=node if reuse_client else None)
    assert raised.value.code == "invalid_observation"


def test_matching_bitcoin_bytes_accept_case_insensitive_id(monkeypatch):
    tx = _coinbase()
    requested = tx.txid().hex().upper()
    node = Node(requested, {requested: tx.serialize().hex()})
    legs = sb.fetch_transaction_legs(BACKEND, requested, chain="bitcoin", client=node)
    assert legs["outputs"][0]["script"] == "51"


@pytest.mark.parametrize("path", ["history", "parent", "inventory", "verification"])
def test_liquid_raw_identity_is_verified_before_unblinding_or_projection(monkeypatch, path):
    parent = LTransaction(vin=[LTransactionInput(b"\0" * 32, 0xffffffff, Script(b"\x01\x01"))],
                          vout=[LTransactionOutput(b"\x01" * 32, 1000, Script(b"\x51"))])
    parent_id = parent.txid().hex()
    child = LTransaction(vin=[LTransactionInput(parent.txid(), 0)],
                         vout=[LTransactionOutput(b"\x01" * 32, 900, Script(b"\x51"))])
    requested = child.txid().hex() if path == "parent" else "11" * 32
    node = Node(requested, {requested: child.serialize().hex() if path == "parent" else parent.serialize().hex(),
                            parent_id: child.serialize().hex()})
    monkeypatch.setattr(sb, "_electrum_client_context", lambda backend: nullcontext(node))
    with pytest.raises(AppError) as raised:
        if path == "inventory":
            sb.compatibility_electrum_utxos_for_wallet(BACKEND, _state("liquid"))
        elif path == "verification":
            sb.fetch_transaction_legs(BACKEND, requested, chain="liquid", client=node)
        else:
            sb.compatibility_electrum_records_for_wallet(BACKEND, _state("liquid"))
    assert raised.value.code == "invalid_observation"


def test_old_unverified_graph_cache_cannot_bypass_raw_identity_check(monkeypatch):
    requested = "11" * 32
    graph = {"txid": requested, "vin": [], "vout": [{"script_hex": "51", "value_sats": 1000}],
             "total_output_sats": 1000, "_kassiber_electrum_graph": {"kind": "bitcoin_electrum", "version": 1}}
    node = Node(requested, {requested: _coinbase().serialize().hex()})
    monkeypatch.setattr(sb, "_electrum_client_context", lambda backend: nullcontext(node))
    with pytest.raises(AppError) as raised:
        sb.compatibility_electrum_records_for_wallet(BACKEND, _state(history_cache={requested: graph}))
    assert raised.value.code == "invalid_observation"
    assert ("blockchain.transaction.get", [requested]) in node.calls
