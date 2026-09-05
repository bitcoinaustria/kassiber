"""Partial disclosures follow a unique complete reviewed route, without rounding."""
import json
from decimal import Decimal

import pytest

from kassiber import daemon
from kassiber.core import source_funds
from tests.test_daemon_review_workflow import book  # noqa: F401


def btc(msat):
    return format(Decimal(msat) / Decimal(100_000_000_000), "f")


def prepare(conn, amount=1_000_000_000):
    conn.execute("UPDATE transactions SET amount=?, fiat_rate=100000", (amount,))
    conn.commit()


def root_link(conn, target="in", amount=1_000_000_000, *, source_amount=None, from_amount=None):
    hooks = daemon._source_funds_hooks()
    source = source_funds.create_source(
        conn, "ws", "profile", hooks, source_type="fiat_purchase", label="Reviewed purchase",
        amount=btc(source_amount if source_amount is not None else amount),
        acquired_at="2024-01-01T00:00:00Z", fiat_value="1000",
    )
    return source_funds.create_link(
        conn, "ws", "profile", hooks, from_source_ref=source["id"],
        to_transaction_ref=target, link_type="manual_source", allocation_amount=btc(amount),
        from_allocation_amount=btc(from_amount) if from_amount is not None else None,
    )


def report(conn, amount, **options):
    return source_funds.build_report(
        conn, "ws", "profile", daemon._source_funds_hooks(),
        target_transaction_ref="in", target_amount=btc(amount), **options,
    )


def test_partial_purchase_projects_graph_source_mix_and_saved_snapshot(book):
    conn, _runtime = book
    prepare(conn)
    link = root_link(conn)
    before = dict(conn.execute("SELECT * FROM source_funds_links WHERE id=?", (link["id"],)).fetchone())
    partial = report(conn, 900_000_000, save_case=True)
    assert partial["explain_gates"]["exportable"] is True
    assert partial["source_mix"][0]["amount_msat"] == 900_000_000
    assert partial["source_mix"][0]["percent_of_target"] == 100
    edge = partial["graph"]["edges"][0]
    assert edge["allocation_amount_msat"] == edge["from_allocation_amount_msat"] == 900_000_000
    saved = json.loads(conn.execute("SELECT snapshot_json FROM source_funds_cases").fetchone()[0])
    assert saved["graph"] == partial["graph"]
    assert saved["source_mix"] == partial["source_mix"]
    assert dict(conn.execute("SELECT * FROM source_funds_links WHERE id=?", (link["id"],)).fetchone()) == before
    assert report(conn, 1_000_000_000)["source_mix"][0]["amount_msat"] == 1_000_000_000


@pytest.mark.parametrize("parent_amount,selected,expected", [(1_100_000_000, 900_000_000, 990_000_000), (1_000_000_001, 900_000_000, None)])
def test_partial_parent_route_preserves_exact_from_amount_ratio(book, parent_amount, selected, expected):
    conn, _runtime = book
    prepare(conn)
    conn.execute("UPDATE transactions SET amount=? WHERE id='out'", (parent_amount,))
    conn.commit()
    root_link(conn, "out", parent_amount)
    source_funds.create_link(
        conn, "ws", "profile", daemon._source_funds_hooks(), from_transaction_ref="out",
        to_transaction_ref="in", allocation_amount="0.01", from_allocation_amount=btc(parent_amount),
    )
    result = report(conn, selected)
    if expected is None:
        assert "ambiguous_allocation" in {item["code"] for item in result["explain_gates"]["blockers"]}
    else:
        assert result["explain_gates"]["exportable"] is True
        assert result["source_mix"][0]["amount_msat"] == expected
        by_to = {edge["to"]: edge for edge in result["graph"]["edges"]}
        assert by_to["tx:in"]["from_allocation_amount_msat"] == expected
        assert by_to["tx:out"]["allocation_amount_msat"] == expected


@pytest.mark.parametrize("shape", ["multiple_sources", "incomplete_route", "insufficient_source"])
def test_partial_target_does_not_invent_source_selection_or_full_route_coverage(book, shape):
    conn, _runtime = book
    prepare(conn)
    if shape == "multiple_sources":
        root_link(conn, amount=500_000_000)
        root_link(conn, amount=500_000_000)
    elif shape == "incomplete_route":
        root_link(conn, amount=800_000_000)
    else:
        root_link(conn, source_amount=800_000_000)
    result = report(conn, 500_000_000)
    assert result["explain_gates"]["exportable"] is False
    assert "ambiguous_allocation" in {item["code"] for item in result["explain_gates"]["blockers"]}


@pytest.mark.parametrize("invalidity", ["stale_custody", "unknown_policy", "unconfirmed_chain", "cross_asset"])
def test_partial_disclosure_preserves_existing_authority_and_asset_gates(book, invalidity):
    conn, _runtime = book
    prepare(conn)
    link = root_link(conn)
    changes = {
        "stale_custody": "method='custody_component'",
        "unknown_policy": "allocation_policy='unknown'",
        "unconfirmed_chain": "uses_chain_observation=1, chain_data_confirmed=0",
        "cross_asset": "from_asset='L-BTC'",
    }
    conn.execute(f"UPDATE source_funds_links SET {changes[invalidity]} WHERE id=?", (link["id"],))
    conn.commit()
    result = report(conn, 900_000_000)
    assert result["explain_gates"]["exportable"] is False
    expected = {
        "stale_custody": "stale_custody_component_lineage",
        "unknown_policy": "ambiguous_allocation",
        "unconfirmed_chain": "unconfirmed_chain_data",
        "cross_asset": "source_asset_mismatch",
    }
    assert expected[invalidity] in {item["code"] for item in result["explain_gates"]["blockers"]}
