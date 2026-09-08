"""Physical observations must not duplicate quantities or revive stale custody."""

from dataclasses import replace
from decimal import Decimal
import json

import pytest

from kassiber.cli.handlers import _metadata_hooks, _wallet_sync_hooks, process_journals
from kassiber.core.chain_observer.provenance import fee_attribution_from_raw
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.core.metadata import update_transaction_metadata
from kassiber.core.ownership_transfers import detect_pending_onchain_ids
from kassiber.core.sync import sync_wallet_from_backend
from kassiber.core.sync_backends import record_from_bitcoin_esplora_tx
from kassiber.core.wallets import create_wallet, update_wallet
from kassiber.db import open_db
from kassiber.msat import btc_to_msat
from tests import test_rp2_ownership_transfers as fixtures
from tests.custody_tax_helpers import persist_authoritative_chain_observation
from tests.test_source_overlap import (
    ADDR_A, ADDR_B, _descriptor_config, _descriptor_target, _script,
)

SCRIPT_A = _script(ADDR_A)
SCRIPT_B = _script(ADDR_B)


@pytest.fixture
def custody_book(tmp_path):
    conn = open_db(tmp_path)
    helper = fixtures.OwnershipDeriverHandlerTest()
    helper._seed(conn)
    try:
        yield conn, helper
    finally:
        conn.close()


def _profile(conn):
    return conn.execute("SELECT * FROM profiles WHERE id='profile-1'").fetchone()


def _graph(txid, *, inputs, outputs, fee=0, confirmed=True):
    status = {"confirmed": confirmed}
    if confirmed:
        status.update(block_height=100, block_time=1767225600)
    return {
        "txid": txid,
        "chain": "bitcoin",
        "network": "main",
        "fee": fee,
        "status": status,
        "vin": [
            {"txid": previous, "vout": vout,
             "prevout": {"scriptpubkey": script, "value": amount}}
            for previous, vout, script, amount in inputs
        ],
        "vout": [
            {"n": index, "scriptpubkey": script, "value": amount}
            for index, (script, amount) in enumerate(outputs)
        ],
    }


def _source_inventory(conn, helper, txid):
    for wallet, address in (("wallet-a", ADDR_A), ("wallet-b", ADDR_B)):
        conn.execute(
            "UPDATE wallets SET kind='address',config_json=? WHERE id=?",
            (json.dumps({"chain": "bitcoin", "network": "main", "addresses": [address]}), wallet),
        )
    helper._tx(
        conn, tx_id="acq", wallet_id="wallet-a", direction="inbound",
        amount=100010000, external_id="acq", raw_json="{}",
    )
    conn.execute("UPDATE transactions SET occurred_at='2025-01-01T00:00:00Z' WHERE id='acq'")
    for wallet, address, funding, amount in (
        ("wallet-a", ADDR_A, "cc" * 32, 100010000),
        ("wallet-b", ADDR_B, txid, 100000000),
    ):
        helper._utxo(conn, wallet, address, funding, 0)
        conn.execute("UPDATE wallet_utxos SET amount=? WHERE wallet_id=?", (amount, wallet))


def _native_leg(conn, helper, wallet, script, graph, *, observed_at=None):
    record = record_from_bitcoin_esplora_tx(graph, {script: {}}, "bdk")
    transaction_id = wallet + "-leg"
    helper._tx(
        conn, tx_id=transaction_id, wallet_id=wallet,
        direction=record["direction"], amount=btc_to_msat(record["amount"]),
        external_id=graph["txid"], raw_json=record["raw_json"],
        fee=btc_to_msat(record["fee"]),
    )
    conn.execute("UPDATE transactions SET confirmed_at=? WHERE id=?", (record["confirmed_at"], transaction_id))
    persist_authoritative_chain_observation(
        conn, transaction_id, observer_kind="bdk",
        fee_attribution=fee_attribution_from_raw(record["raw_json"]),
    )
    if observed_at:
        conn.execute(
            "UPDATE chain_observation_provenance SET observed_at=? WHERE transaction_id=?",
            (observed_at, transaction_id),
        )


@pytest.mark.parametrize(
    "add_descriptor,refresh_old,conflicting_graph,trim_old,authority,input_coverage",
    [
        (False, False, False, False, "current", "complete"),
        (True, False, False, False, "current", "complete"),
        (True, True, False, False, "current", "complete"),
        (True, False, True, False, "current", "complete"),
        (True, False, False, True, "current", "complete"),
        (True, False, False, False, "missing", "complete"),
        (True, False, False, False, "stale", "complete"),
        (True, False, False, False, "current", "missing_script"),
        (True, False, False, False, "current", "missing_value"),
    ],
)
def test_late_descriptor_reconciles_receipts_without_rewriting_imports(
    custody_book, add_descriptor, refresh_old, conflicting_graph, trim_old,
    authority, input_coverage,
):
    conn, _helper = custody_book
    conn.execute("DELETE FROM wallets")
    config = _descriptor_config(1)
    address_a = _descriptor_target(config, 0).address
    address_b = _descriptor_target(config, 3).address
    graph = _graph(
        "aa" * 32,
        inputs=[("bb" * 32, 0, "0014" + "ff" * 20, 300010)],
        outputs=[(_script(address_a), 100000), (_script(address_b), 200000)],
        fee=10,
    )

    def adapter(_backend, wallet, state):
        observed = json.loads(json.dumps(graph))
        if conflicting_graph and wallet["id"] == "new":
            observed["vout"].append({"n": 2, "scriptpubkey": "0014" + "ee" * 20, "value": 500})
        if input_coverage != "complete":
            field = "scriptpubkey" if input_coverage == "missing_script" else "value"
            for entry in observed["vin"]:
                entry["prevout"].pop(field)
        return [record_from_bitcoin_esplora_tx(observed, state.tracked_scripts, "esplora")], {}

    # Replace only transport. Discovery, overlap filters, record persistence and
    # closed adapter provenance all run through the production sync path.
    hooks = replace(
        _wallet_sync_hooks(), prepare_observer_fetch=None,
        resolve_backend=lambda *_: {"name": "fixture", "kind": "esplora", "url": "https://example.invalid"},
        backend_adapters={"esplora": adapter},
    )
    create_wallet(
        conn, "ws-1", "profile-1", "Old address list", "address", wallet_id="old",
        config={"chain": "bitcoin", "network": "main", "addresses": [address_a, address_b]},
    )
    old = conn.execute("SELECT * FROM wallets WHERE id='old'").fetchone()
    assert sync_wallet_from_backend(conn, {}, _profile(conn), old, hooks)["imported"] == 1
    if add_descriptor:
        create_wallet(conn, "ws-1", "profile-1", "New descriptor", "descriptor", config=config, wallet_id="new")
        new = conn.execute("SELECT * FROM wallets WHERE id='new'").fetchone()
        assert sync_wallet_from_backend(conn, {}, _profile(conn), new, hooks)["imported"] == 1
        if refresh_old:
            sync_wallet_from_backend(conn, {}, _profile(conn), old, hooks)
    if trim_old:
        update_wallet(conn, "Main", "Default", "old", {"config": {"addresses": [address_b]}})
    for row in conn.execute("SELECT id FROM transactions").fetchall():
        update_transaction_metadata(
            conn, "Main", "Default", row["id"], _metadata_hooks(),
            pricing_update={"fiat_rate": "40000", "source_kind": "manual_override", "quality": "exact"},
        )
    assert conn.execute("SELECT COUNT(*) FROM chain_observation_provenance").fetchone()[0] == (2 if add_descriptor else 1)
    if authority == "missing":
        conn.execute("DELETE FROM chain_observation_provenance")
    elif authority == "stale":
        conn.execute("UPDATE chain_observation_provenance SET quantity_hash='stale' WHERE wallet_id='old'")
    conn.commit()
    original_rows = [tuple(row) for row in conn.execute("SELECT id,amount,raw_json FROM transactions ORDER BY id")]
    process_journals(conn, "Main", "Default")
    assert [tuple(row) for row in conn.execute("SELECT id,amount,raw_json FROM transactions ORDER BY id")] == original_rows
    builder = CustodyJournalBuilder(conn, _profile(conn))
    state = builder.build()
    actual = sum(value["quantity"] for value in state["wallet_holdings"].values())
    if conflicting_graph or trim_old or authority != "current" or input_coverage != "complete":
        assert actual == 0
        assert state["quarantines"]
        assert all(
            json.loads(row["detail_json"])["blocker_code"] == "source_overlap_quantity_unresolved"
            for row in state["quarantines"]
        )
        return
    assert actual == Decimal(".003")
    assert not state["quarantines"]
    if add_descriptor:
        assert sorted(row["amount"] for row in conn.execute("SELECT amount FROM transactions")) == [
            100000000, 200000000 if refresh_old else 300000000,
        ]
        projected = builder.build_custody_projection().finalized_tax_projection.rows
        assert {row["wallet_id"]: row["amount"] for row in projected} == {"old": 200000000, "new": 100000000}


@pytest.mark.parametrize("source_present", [False, True])
def test_known_own_input_requires_source_history_in_public_journal(custody_book, source_present):
    conn, helper = custody_book
    txid = "dd" * 32
    _source_inventory(conn, helper, txid)
    graph = _graph(
        txid, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
        outputs=[(SCRIPT_B, 100000)], fee=10,
    )
    if source_present:
        _native_leg(conn, helper, "wallet-a", SCRIPT_A, graph)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.commit()
    process_journals(conn, "Main", "Default")
    state = CustodyJournalBuilder(conn, _profile(conn)).build()
    holds = {(row["transaction_id"], row["reason"]) for row in state["quarantines"]}
    quantity = sum(value["quantity"] for value in state["wallet_holdings"].values())
    assert not any(
        row["transaction_id"] == "wallet-b-leg" and row["entry_type"] == "acquisition"
        for row in state["entries"]
    )
    if source_present:
        assert not holds
        assert quantity == Decimal(".001")
        assert {row["entry_type"] for row in state["entries"] if row["transaction_id"] != "acq"} == {
            "transfer_out", "transfer_in", "transfer_fee",
        }
    else:
        assert holds == {("wallet-b-leg", "ownership_transfer_source_missing")}
        assert quantity == Decimal(".0010001")
        assert conn.execute("SELECT COUNT(*) FROM journal_quarantines WHERE reason='ownership_transfer_source_missing'").fetchone()[0] == 1


@pytest.mark.parametrize(
    "source_confirmed,target_confirmed,source_observed,target_observed,expected_pending",
    [
        (False, False, "2026-01-02T00:00:00Z", "2026-01-01T00:00:00Z", True),
        (False, True, "2026-01-02T00:00:00Z", "2026-01-01T00:00:00Z", True),
        (False, True, "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z", False),
        (False, True, "2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z", True),
        (True, True, "2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z", False),
    ],
)
def test_confirmation_uses_closed_observation_chronology(
    custody_book, source_confirmed, target_confirmed, source_observed,
    target_observed, expected_pending,
):
    conn, helper = custody_book
    txid = "dd" * 32
    _source_inventory(conn, helper, txid)
    for wallet, script, confirmed, observed_at in (
        ("wallet-a", SCRIPT_A, source_confirmed, source_observed),
        ("wallet-b", SCRIPT_B, target_confirmed, target_observed),
    ):
        graph = _graph(
            txid, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
            outputs=[(SCRIPT_B, 100000)], fee=10, confirmed=confirmed,
        )
        _native_leg(conn, helper, wallet, script, graph, observed_at=observed_at)
    conn.commit()
    process_journals(conn, "Main", "Default")
    builder = CustodyJournalBuilder(conn, _profile(conn))
    synthetic = [
        {**row, "id": "owned-derive:" + row["id"], "amount": 1}
        for row in builder._transactions() if row["id"] != "acq"
    ]
    assert bool(detect_pending_onchain_ids(synthetic)) is expected_pending
    state = builder.build()
    entries = [row for row in state["entries"] if row["transaction_id"] != "acq"]
    assert bool(entries) is not expected_pending
    if expected_pending:
        assert {
            row["transaction_id"] for row in state["quarantines"]
            if row["reason"] == "pending_onchain_confirmation"
        } == {"wallet-a-leg", "wallet-b-leg"}
    else:
        assert not state["quarantines"]


@pytest.mark.parametrize(
    "sync_returned_wallet,foreign_fee,own_debit,partial,imported_zero",
    [
        (False, 0, 0, False, False),
        (True, 0, 0, False, False),
        (False, 10, 0, False, False),
        (True, 10, 0, False, False),
        (False, 0, 10, False, False),
        (False, 0, 0, True, False),
        (True, 0, 0, False, True),
    ],
)
def test_returned_own_input_does_not_fund_foreign_receipt(
    custody_book, sync_returned_wallet, foreign_fee, own_debit, partial, imported_zero,
):
    conn, helper = custody_book
    conn.execute("DELETE FROM wallets")
    for wallet, address in (("wallet-a", ADDR_A), ("wallet-b", ADDR_B)):
        create_wallet(
            conn, "ws-1", "profile-1", wallet, "address", wallet_id=wallet,
            config={"chain": "bitcoin", "network": "main", "addresses": [address]},
        )
    helper._tx(
        conn, tx_id="acq", wallet_id="wallet-a", direction="inbound",
        amount=100000000, external_id="acq", raw_json="{}",
    )
    conn.execute("UPDATE transactions SET occurred_at='2020-01-01T00:00:00Z' WHERE id='acq'")
    conn.commit()
    graph = _graph(
        "dd" * 32,
        inputs=[
            ("cc" * 32, 0, SCRIPT_A, 100000),
            ("ee" * 32, 0, "0014" + "ff" * 20, 100000 + foreign_fee),
        ],
        outputs=[(SCRIPT_A, 100000 - own_debit), (SCRIPT_B, 100000 + own_debit)],
        fee=foreign_fee,
    )
    if partial:
        graph["vin"][1]["prevout"].pop("value")

    def adapter(_backend, _wallet, state):
        return [record_from_bitcoin_esplora_tx(graph, state.tracked_scripts, "esplora")], {}

    hooks = replace(
        _wallet_sync_hooks(), prepare_observer_fetch=None,
        resolve_backend=lambda *_: {"name": "fixture", "kind": "esplora", "url": "https://example.invalid"},
        backend_adapters={"esplora": adapter},
    )
    for wallet_id in ["wallet-b"] + (["wallet-a"] if sync_returned_wallet else []):
        wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet_id,)).fetchone()
        assert sync_wallet_from_backend(conn, {}, _profile(conn), wallet, hooks)["imported"] == 1
    for row in conn.execute("SELECT id FROM transactions WHERE id!='acq'").fetchall():
        update_transaction_metadata(
            conn, "Main", "Default", row["id"], _metadata_hooks(),
            pricing_update={"fiat_rate": "40000", "source_kind": "manual_override", "quality": "exact"},
        )
    if imported_zero:
        conn.execute("DELETE FROM chain_observation_provenance WHERE wallet_id='wallet-a'")
        conn.commit()
    original_rows = [tuple(row) for row in conn.execute("SELECT id,amount,fee,raw_json FROM transactions ORDER BY id")]
    process_journals(conn, "Main", "Default")
    state = CustodyJournalBuilder(conn, _profile(conn)).build()
    reasons = {row["reason"] for row in state["quarantines"]}
    if own_debit or partial:
        assert "ownership_transfer_source_missing" in reasons
    elif imported_zero:
        assert "custody_quantity_unresolved" in reasons
        assert all(
            json.loads(row["detail_json"])["blocker_code"] == "canonical_event_leg_invalid"
            for row in state["quarantines"]
        )
    else:
        assert not reasons
        assert sum(value["quantity"] for value in state["wallet_holdings"].values()) == Decimal(".002")
        assert not any(entry["entry_type"] == "fee" for entry in state["entries"])
    assert [tuple(row) for row in conn.execute("SELECT id,amount,fee,raw_json FROM transactions ORDER BY id")] == original_rows
