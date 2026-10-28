"""A Bitcoin coinbase input is not a previous transaction to fetch."""
import json
from decimal import Decimal
from unittest.mock import patch

from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput

from kassiber.core import sync_backends as reference
from kassiber.core.sync import WalletSyncState


def test_electrum_coinbase_graph_does_not_lookup_the_null_outpoint():
    tx = {"vin": [{"txid": "0" * 64, "vout": 0xffffffff}],
          "vout": [{"script_hex": "51", "value_sats": 500_000_000}]}

    def forbidden(_):
        raise AssertionError("coinbase null outpoint must never be fetched")

    reference._normalize_electrum_bitcoin_graph_for_storage(tx, forbidden)
    assert tx["vout"][0]["value"] == 500_000_000
    assert "prevout" not in tx["vin"][0]
    ordinary = {"vin": [{"txid": "0" * 64, "vout": 0}], "vout": []}
    reference._normalize_electrum_bitcoin_graph_for_storage(
        ordinary, lambda _: {"vout": [{"script_hex": "51", "value_sats": 7}]})
    assert ordinary["vin"][0]["prevout"] == {"scriptpubkey": "51", "value": 7}


def test_electrum_coinbase_fetch_and_record_keep_the_observed_receipt():
    tx = Transaction(vin=[TransactionInput(b"\x00" * 32, 0xffffffff, Script(b"\x01\x01"))],
                     vout=[TransactionOutput(500_000_000, Script(b"\x51"))])
    txid = tx.txid().hex()
    target = {"script_pubkey": "51", "branch_index": 0, "address_index": 0}
    state = WalletSyncState(chain="bitcoin", network="regtest", descriptor_plan=None,
                           policy_asset_id="", targets=[target], tracked_scripts={"51": target},
                           history_cache={}, checkpoint={})
    calls = []

    class Client:
        def __init__(self, _):
            pass

        def __enter__(self):
            return self

        def __exit__(self, *_):
            return False

        def batch_call(self, requests):
            results = []
            for method, params in requests:
                calls.append((method, params))
                if method == "blockchain.scripthash.subscribe":
                    result = "status"
                elif method == "blockchain.scripthash.get_history":
                    result = [{"tx_hash": txid, "height": 100}]
                elif method == "blockchain.transaction.get":
                    assert params == [txid], "coinbase null outpoint must never be fetched"
                    result = tx.serialize().hex()
                else:
                    assert method == "blockchain.block.header"
                    result = (b"\x00" * 68 + (1_700_000_000).to_bytes(4, "little") + b"\x00" * 8).hex()
                results.append(result)
            return results

    with patch.object(reference, "ElectrumClient", Client):
        records, meta = reference.compatibility_electrum_records_for_wallet(
            {"name": "fixture", "kind": "electrum", "url": "tcp://unused.invalid:1"}, state)
    assert len(records) == 1
    assert records[0]["direction"] == "inbound"
    assert records[0]["amount"] == Decimal("5")
    assert records[0]["fee"] == Decimal("0")
    assert "prevout" not in json.loads(records[0]["raw_json"])["vin"][0]
    assert meta["transactions_fetched"] == 1
    assert sum(method == "blockchain.transaction.get" for method, _ in calls) == 1
