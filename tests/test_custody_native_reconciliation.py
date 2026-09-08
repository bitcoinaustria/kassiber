"""Reviewed custody remains usable when complete native history is recovered."""

from decimal import Decimal
from datetime import datetime, timedelta, timezone
import json
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import custody_components, custody_gaps, custody_journal, review_workflow
from kassiber.core.custody_native_reconciliation import reconcile_native_components
from kassiber.core.chain_observer.provenance import fee_attribution_from_raw
from kassiber.core.sync_backends import record_from_bitcoin_esplora_tx
from kassiber.msat import btc_to_msat
from tests.custody_tax_helpers import persist_authoritative_chain_observation
from tests.test_custody_lineage_flagship import (
    BTC, _FlagshipTreasury, _Transaction, _review_candidate,
)


@pytest.fixture
def book(tmp_path):
    treasury = _FlagshipTreasury(tmp_path)
    yield treasury
    treasury.close()


def _state(book):
    profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    return custody_journal.CustodyJournalBuilder(book.conn, profile).build()


def _reconciled(book):
    profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    return custody_journal.CustodyJournalBuilder(book.conn, profile).build_custody_decisions().interpretation.native_reconciled_component_ids


def _authored_rows(conn):
    return {
        table: [tuple(row) for row in conn.execute(f"SELECT * FROM {table} ORDER BY id")]
        for table in (
            "custody_components", "custody_component_legs", "custody_component_allocations",
            "custody_component_evidence_commitments",
        )
    }


def test_reviewed_two_source_native_consolidation_preserves_each_allocation(book):
    txid = _FlagshipTreasury._txid(123)
    at = "2021-01-01T00:00:00Z"
    book.insert([
        _Transaction("fund-a", "a", "inbound", BTC, "2020-01-01T00:00:00Z", kind="buy"),
        _Transaction("fund-b", "b", "inbound", BTC, "2020-01-01T00:00:00Z", kind="buy"),
        _Transaction("out-a", "a", "outbound", BTC, at, txid=txid),
        _Transaction("out-b", "b", "outbound", BTC, at, txid=txid),
        _Transaction("in-c", "c", "inbound", 2 * BTC, at, txid=txid),
    ])
    scripts = {wallet: "0014" + byte * 20 for wallet, byte in (("a", "11"), ("b", "22"), ("c", "33"))}
    graph = {
        "txid": txid, "chain": "bitcoin", "network": "main", "fee": 0,
        "status": {"confirmed": True, "block_height": 100, "block_time": 1609459200},
        "vin": [
            {"txid": _FlagshipTreasury._txid(120 + i), "vout": 0,
             "prevout": {"scriptpubkey": scripts[wallet], "value": 100_000_000}}
            for i, wallet in enumerate(("a", "b"))
        ],
        "vout": [{"n": 0, "scriptpubkey": scripts["c"], "value": 200_000_000}],
    }
    for wallet, row_id in (("a", "out-a"), ("b", "out-b"), ("c", "in-c")):
        record = record_from_bitcoin_esplora_tx(graph, {scripts[wallet]: {}}, "bdk")
        book.conn.execute(
            "UPDATE transactions SET direction=?, amount=?, fee=?, raw_json=?, confirmed_at=? WHERE id=?",
            (record["direction"], btc_to_msat(record["amount"]), btc_to_msat(record["fee"]),
             record["raw_json"], record["confirmed_at"], row_id),
        )
        persist_authoritative_chain_observation(
            book.conn, row_id, observer_kind="bdk", fee_attribution=fee_attribution_from_raw(record["raw_json"]),
        )
        prev_txid = _FlagshipTreasury._txid(120 if wallet == "a" else 121) if wallet != "c" else txid
        book.conn.execute(
            """INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, chain, network, asset,
                amount, txid, vout, outpoint, confirmation_status, script_pubkey,
                first_seen_at, last_seen_at
            ) VALUES(?, 'ws', 'profile', ?, 'bitcoin', 'main', 'BTC', ?, ?, 0, ?, 'confirmed', ?, ?, ?)""",
            ("utxo-" + wallet, wallet, BTC if wallet != "c" else 2 * BTC,
             prev_txid, prev_txid + ":0", scripts[wallet], at, at),
        )
    book.conn.commit()
    profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    artifact = review_workflow.plan_review(
        book.conn, profile, expected_input_version=profile["journal_input_version"],
        hooks=review_workflow.ReviewHooks(metadata=_metadata_hooks()),
        operations=[{"type": "custody_component", "request": {"action": "create", "activate": True, "components": [{
            "component_type": "manual_bridge", "evidence_kind": "manual_claim", "evidence_grade": "reviewed",
            "change_reason": "Reviewed two-source consolidation",
            "legs": [
                {"id": "a", "role": "source", "transaction": "out-a", "amount_msat": BTC},
                {"id": "b", "role": "source", "transaction": "out-b", "amount_msat": BTC},
                {"id": "c", "role": "destination", "transaction": "in-c", "amount_msat": 2 * BTC},
            ],
            "allocations": [
                {"source_leg_id": source, "sink_leg_id": "c", "source_amount_msat": BTC, "sink_amount_msat": BTC}
                for source in ("a", "b")
            ],
        }]}}],
    )
    receipt = review_workflow.apply_review(
        book.conn, profile, artifact=artifact, idempotency_key="two-source-consolidation",
        hooks=review_workflow.ReviewHooks(metadata=_metadata_hooks()),
    )
    assert artifact["after"]["report_ready"], artifact["after"]
    assert receipt["verification"]["quarantine_count"] == 0
    component = custody_components.list_components(book.conn, profile_id="profile")[0]
    assert component["effective_state"] == "active"
    state = _state(book)
    assert not state["quarantines"]
    holdings = [(key[0], value["quantity"], value["cost_basis"]) for key, value in state["wallet_holdings"].items()]
    assert holdings == [("c", Decimal("2"), Decimal("40000"))]


def test_recovered_middle_wallet_replaces_only_effective_route(book):
    book.insert([
        _Transaction("fund-a", "a", "inbound", BTC, "2020-01-01T00:00:00Z", kind="buy"),
        _Transaction("out-a", "a", "outbound", BTC, "2021-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(1), privacy_boundary="coinjoin"),
        _Transaction("in-c", "c", "inbound", BTC, "2022-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(2), privacy_boundary="coinjoin"),
    ])
    candidate = custody_gaps.load_gap_search_result(book.conn, "profile")[0].candidates[0]
    created = _review_candidate(book.conn, candidate)
    assert not _state(book)["quarantines"]
    authored = _authored_rows(book.conn)
    book.insert([
        _Transaction("in-b", "b", "inbound", BTC, "2021-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(1), privacy_boundary="coinjoin"),
        _Transaction("out-b", "b", "outbound", BTC, "2022-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(2), privacy_boundary="coinjoin"),
    ])
    state = _state(book)
    assert not state["quarantines"]
    assert {(entry["transaction_id"], entry["entry_type"]) for entry in state["entries"]} == {
        ("fund-a", "acquisition"), ("out-a", "transfer_out"), ("in-b", "transfer_in"),
        ("out-b", "transfer_out"), ("in-c", "transfer_in"),
    }
    assert [(key[0], value["quantity"], value["cost_basis"]) for key, value in state["wallet_holdings"].items()] == [
        ("c", Decimal("1"), Decimal("20000")),
    ]
    assert _authored_rows(book.conn) == authored
    assert custody_components.get_component(book.conn, created["component_id"])["state"] == "active"
    assert _reconciled(book) == (created["component_id"],)
    certificate, = state["custody_native_reconciliations"]
    assert certificate["component_id"] == created["component_id"]
    assert certificate["component_revision"] == 1
    assert set(certificate["transaction_ids"]) == {"out-a", "in-b", "out-b", "in-c"}
    assert len(certificate["native_claim_ids"]) == 2
    assert {amount for _claim_id, amount in certificate["native_claim_amounts"]} == {BTC}
    assert certificate["allocations"] == ({
        "source_transaction_id": "out-a", "target_transaction_id": "in-c",
        "asset": "BTC", "amount_msat": BTC, "destination_kind": "wallet",
    },)


@pytest.mark.parametrize("missing", ["outbound", "excluded", "unverified", "extra_activity", "uncovered_fee", "pending"])
def test_incomplete_or_competing_native_history_does_not_replace_review(book, missing):
    book.insert([
        _Transaction("fund-a", "a", "inbound", BTC, "2020-01-01T00:00:00Z", kind="buy"),
        _Transaction("out-a", "a", "outbound", BTC, "2021-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(1), privacy_boundary="coinjoin"),
        _Transaction("in-c", "c", "inbound", BTC, "2022-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(2), privacy_boundary="coinjoin"),
    ])
    candidate = custody_gaps.load_gap_search_result(book.conn, "profile")[0].candidates[0]
    created = _review_candidate(book.conn, candidate)
    authored = _authored_rows(book.conn)
    recovered = [
        _Transaction("in-b", "b", "inbound", BTC, "2021-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(1), privacy_boundary="coinjoin"),
    ]
    if missing != "outbound":
        recovered.append(_Transaction(
            "out-b", "b", "outbound", BTC, "2022-01-01T00:00:00Z",
            fee_msat=1000 if missing == "uncovered_fee" else 0,
            txid=_FlagshipTreasury._txid(2), privacy_boundary="coinjoin",
        ))
    if missing == "extra_activity":
        recovered.append(_Transaction("other-funds", "b", "inbound", BTC, "2021-06-01T00:00:00Z", kind="buy"))
    book.insert(recovered)
    if missing == "excluded":
        book.conn.execute("UPDATE transactions SET excluded=1 WHERE id='in-b'")
    elif missing == "unverified":
        book.conn.execute("DELETE FROM chain_observation_provenance WHERE transaction_id='in-b'")
    elif missing == "pending":
        for row_id in ("in-b", "out-a"):
            raw = json.loads(book.conn.execute("SELECT raw_json FROM transactions WHERE id=?", (row_id,)).fetchone()[0])
            raw["status"] = {"confirmed": False}
            book.conn.execute("UPDATE transactions SET raw_json=? WHERE id=?", (json.dumps(raw), row_id))
            persist_authoritative_chain_observation(book.conn, row_id)
    book.conn.commit()
    assert not _reconciled(book)
    assert custody_components.get_component(book.conn, created["component_id"])["state"] == "active"
    assert _authored_rows(book.conn) == authored


@pytest.mark.parametrize("fee_delta", [0, -1000, 1000])
def test_recovered_branching_route_preserves_reviewed_fees_and_return_allocations(book, fee_delta):
    book.insert(_FlagshipTreasury.missing_whirlpool_boundaries())
    candidate = next(
        item for item in custody_gaps.load_gap_search_result(book.conn, "profile")[0].candidates
        if item.source_ids == ("2020-whirlpool-out",) and len(item.return_ids) == 2
    )
    created = _review_candidate(book.conn, candidate)
    authored = _authored_rows(book.conn)
    book.insert([
        _Transaction("deposit-in", "deposit", "inbound", 10 * BTC, "2020-01-01T00:00:00Z", txid=_FlagshipTreasury._txid(10), privacy_boundary="coinjoin"),
        *_FlagshipTreasury.exact_move("tx0", "deposit", "premix", 10 * BTC, 0, "2020-01-02T00:00:00Z", 20, privacy_boundary="coinjoin"),
        *_FlagshipTreasury.exact_move("remix", "premix", "postmix", 10 * BTC, 0, "2020-06-01T00:00:00Z", 21, privacy_boundary="coinjoin"),
        _Transaction("postmix-out-1", "postmix", "outbound", 6 * BTC, "2021-01-01T00:00:00Z", fee_msat=BTC // 20, txid=_FlagshipTreasury._txid(11), privacy_boundary="coinjoin"),
        _Transaction("postmix-out-2", "postmix", "outbound", 39 * BTC // 10, "2021-02-01T00:00:00Z", fee_msat=BTC // 20 + fee_delta, txid=_FlagshipTreasury._txid(12), privacy_boundary="coinjoin"),
    ])
    if fee_delta:
        assert not _reconciled(book)
        assert _authored_rows(book.conn) == authored
        return
    assert _reconciled(book) == (created["component_id"],)
    state = _state(book)
    assert not state["quarantines"]
    route_fees = [entry for entry in state["entries"] if entry["transaction_id"] in {"postmix-out-1", "postmix-out-2"} and entry["entry_type"] == "transfer_fee"]
    assert sum(abs(entry["quantity"]) for entry in route_fees) == Decimal("0.1")
    assert _authored_rows(book.conn) == authored
    # Removing a previously recovered step retracts only the derived choice.
    book.conn.execute("UPDATE transactions SET excluded=1 WHERE id='remix-in'")
    book.conn.commit()
    assert not _reconciled(book)
    assert _authored_rows(book.conn) == authored


@pytest.mark.parametrize("reverse_allocations", [False, True])
def test_recovery_checks_each_source_to_return_allocation_not_only_net_balance(book, reverse_allocations):
    dates = ["2021-01-01T00:00:00Z", "2021-02-01T00:00:00Z", "2022-01-01T00:00:00Z", "2022-02-01T00:00:00Z"]
    book.insert([
        _Transaction("fund-a", "a", "inbound", BTC, "2020-01-01T00:00:00Z", kind="buy"),
        _Transaction("fund-b", "b", "inbound", BTC, "2020-01-01T00:00:00Z", kind="buy"),
        *[
            _Transaction(row_id, wallet, direction, BTC, dates[i], txid=_FlagshipTreasury._txid(101 + i), privacy_boundary="coinjoin")
            for i, (row_id, wallet, direction) in enumerate((
                ("out-a", "a", "outbound"), ("out-b", "b", "outbound"),
                ("in-c", "c", "inbound"), ("in-d", "d", "inbound"),
            ))
        ],
    ])
    profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    allocations = [("a", "d"), ("b", "c")] if reverse_allocations else [("a", "c"), ("b", "d")]
    artifact = review_workflow.plan_review(
        book.conn, profile, expected_input_version=profile["journal_input_version"],
        hooks=review_workflow.ReviewHooks(metadata=_metadata_hooks()),
        operations=[{"type": "custody_component", "request": {"action": "create", "activate": True, "components": [{
            "component_type": "manual_bridge", "evidence_kind": "manual_claim", "evidence_grade": "reviewed",
            "change_reason": "Reviewed per-source return allocations",
            "legs": [
                {"id": wallet, "role": "source" if wallet in {"a", "b"} else "destination",
                 "transaction": ("out-" if wallet in {"a", "b"} else "in-") + wallet, "amount_msat": BTC}
                for wallet in ("a", "b", "c", "d")
            ],
            "allocations": [
                {"source_leg_id": source, "sink_leg_id": target, "source_amount_msat": BTC, "sink_amount_msat": BTC}
                for source, target in allocations
            ],
        }]}}],
    )
    review_workflow.apply_review(
        book.conn, profile, artifact=artifact, idempotency_key="two-route-review",
        hooks=review_workflow.ReviewHooks(metadata=_metadata_hooks()),
    )
    assert artifact["after"]["report_ready"]
    authored = _authored_rows(book.conn)
    book.insert([
        _Transaction("middle-" + str(i), "friend-a", "inbound" if i < 2 else "outbound", BTC,
                     dates[i], txid=_FlagshipTreasury._txid(101 + i), privacy_boundary="coinjoin")
        for i in range(4)
    ])
    assert bool(_reconciled(book)) is not reverse_allocations
    assert _authored_rows(book.conn) == authored
    if not reverse_allocations:
        state = _state(book)
        assert not state["quarantines"]
        assert sum(value["quantity"] for value in state["wallet_holdings"].values()) == Decimal("2")
        assert sum(value["cost_basis"] for value in state["wallet_holdings"].values()) == Decimal("40000")


@pytest.mark.parametrize("reverse_ids", [False, True])
@pytest.mark.parametrize("return_wallet", ["c", "a"])
def test_same_block_recovery_uses_observed_parent_order(book, reverse_ids, return_wallet):
    first = ("ff" if reverse_ids else "11") * 32
    second = ("11" if reverse_ids else "ff") * 32
    at = "2021-01-01T00:00:00Z"
    scripts = {wallet: "0014" + byte * 20 for wallet, byte in (("a", "aa"), ("b", "bb"), ("c", "cc"))}

    def graph(txid, previous, source, target):
        return {
            "txid": txid, "chain": "bitcoin", "network": "main", "fee": 0,
            "status": {"confirmed": True, "block_height": 100, "block_time": 1609459200},
            "vin": [{"txid": previous, "vout": 0, "prevout": {"scriptpubkey": scripts[source], "value": 100_000_000}}],
            "vout": [{"n": 0, "scriptpubkey": scripts[target], "value": 100_000_000}],
        }

    first_graph = graph(first, "88" * 32, "a", "b")
    second_graph = graph(second, first, "b", return_wallet)
    book.insert([
        _Transaction("fund-a", "a", "inbound", BTC, "2020-01-01T00:00:00Z", kind="buy"),
        _Transaction("out-a", "a", "outbound", BTC, at, txid=first, privacy_boundary="coinjoin", raw_extra=first_graph),
        _Transaction("in-c", return_wallet, "inbound", BTC, at, txid=second, privacy_boundary="coinjoin", raw_extra=second_graph),
    ])
    profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    hooks = review_workflow.ReviewHooks(metadata=_metadata_hooks())
    artifact = review_workflow.plan_review(
        book.conn, profile, expected_input_version=profile["journal_input_version"], hooks=hooks,
        operations=[{"type": "custody_component", "request": {"action": "create", "activate": True, "components": [{
            "component_type": "manual_bridge", "evidence_kind": "manual_claim", "evidence_grade": "reviewed",
            "change_reason": "Reviewed source and return before middle wallet recovery",
            "legs": [
                {"id": "a", "role": "source", "transaction": "out-a", "amount_msat": BTC},
                {"id": "c", "role": "destination", "transaction": "in-c", "amount_msat": BTC},
            ],
            "allocations": [{"source_leg_id": "a", "sink_leg_id": "c", "source_amount_msat": BTC, "sink_amount_msat": BTC}],
        }]}}],
    )
    review_workflow.apply_review(book.conn, profile, artifact=artifact, idempotency_key="same-block-review", hooks=hooks)
    authored = _authored_rows(book.conn)
    book.insert([
        _Transaction("in-b", "b", "inbound", BTC, at, txid=first, privacy_boundary="coinjoin", raw_extra=first_graph),
        _Transaction("out-b", "b", "outbound", BTC, at, txid=second, privacy_boundary="coinjoin", raw_extra=second_graph),
    ])
    assert _reconciled(book)
    assert not _state(book)["quarantines"]
    assert _authored_rows(book.conn) == authored


@pytest.mark.parametrize("recovered", [False, True])
def test_disjoint_reviewed_routes_limit_observation_reads(book, recovered):
    count = 64
    rows = [_Transaction("fund-a", "a", "inbound", count * BTC, "2019-01-01T00:00:00Z", kind="buy")]
    for index in range(count):
        start = datetime(2020, 1, 1, tzinfo=timezone.utc) + timedelta(days=3 * index)
        end = start + timedelta(days=1) if recovered else start
        rows.extend([
            _Transaction(f"out-{index}", "a", "outbound", BTC, start.isoformat(), txid=book._txid(2 * index + 1)),
            _Transaction(f"in-{index}", "c", "inbound", BTC, end.isoformat(), txid=book._txid(2 * index + 2 if recovered else 2 * index + 1)),
        ])
        if recovered:
            rows.extend([
                _Transaction(f"middle-in-{index}", "b", "inbound", BTC, start.isoformat(), txid=book._txid(2 * index + 1)),
                _Transaction(f"middle-out-{index}", "b", "outbound", BTC, end.isoformat(), txid=book._txid(2 * index + 2)),
            ])
    book.insert(rows)
    for index in range(count):
        common_leg = {"rail": "bitcoin", "chain": "bitcoin", "network": "main", "asset": "BTC", "exposure": "bitcoin", "conservation_unit": "msat"}
        component = custody_components.create_component(
            book.conn, workspace_id="ws", profile_id="profile", component_type="manual_bridge",
            evidence_kind="manual_claim", evidence_grade="reviewed", change_reason="Reviewed independent route",
            legs=[
                {**common_leg, "id": f"source-{index}", "role": "source", "wallet_id": "a", "transaction_id": f"out-{index}", "amount_msat": BTC},
                {**common_leg, "id": f"target-{index}", "role": "destination", "wallet_id": "c", "transaction_id": f"in-{index}", "amount_msat": BTC},
            ],
            allocations=[{"source_leg_id": f"source-{index}", "sink_leg_id": f"target-{index}", "source_amount_msat": BTC, "sink_amount_msat": BTC}],
        )
        custody_components.activate_component(book.conn, component["id"])
    book.conn.commit()
    reads = [0]

    class CountObservationReads:
        def __init__(self, observation):
            self.observation = observation

        def __getattr__(self, name):
            if name in {"occurred_at", "wallet_id", "direction"}:
                reads[0] += 1
            return getattr(self.observation, name)

    def measured(components, observations, claims, **kwargs):
        expected = reconcile_native_components(components, observations, claims, **kwargs)
        counted = {key: CountObservationReads(item) for key, item in observations.items()}
        actual = reconcile_native_components(components, counted, claims, **kwargs)
        assert actual == expected
        return actual

    profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    with patch("kassiber.core.custody_interpreters.reconcile_native_components", side_effect=measured):
        decisions = custody_journal.CustodyJournalBuilder(book.conn, profile).build_custody_decisions()
    assert len(decisions.interpretation.native_reconciled_component_ids) == (count if recovered else 0)
    assert reads[0] <= 60 * count, f"{reads[0]} observation reads for {count} disjoint routes"
