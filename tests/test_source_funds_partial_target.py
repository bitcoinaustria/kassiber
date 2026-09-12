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


def test_unequal_route_states_gross_without_claiming_a_target_share(book):
    """The 0.0099 gross demand is correct; calling it 110% of the target is not."""
    conn, _runtime = book
    prepare(conn)
    conn.execute("UPDATE transactions SET amount=? WHERE id='out'", (1_100_000_000,))
    conn.commit()
    root_link(conn, "out", 1_100_000_000)
    source_funds.create_link(
        conn, "ws", "profile", daemon._source_funds_hooks(), from_transaction_ref="out",
        to_transaction_ref="in", allocation_amount="0.01", from_allocation_amount=btc(1_100_000_000),
    )
    result = report(conn, 900_000_000)
    assert result["explain_gates"]["exportable"] is True

    allocations = result["allocations"]
    assert allocations["target_amount_msat"] == 900_000_000
    # The gross upstream demand is retained exactly, with its own denomination.
    assert allocations["gross_source_requirement"] == [
        {"asset": "BTC", "amount": 0.0099, "amount_msat": 990_000_000}
    ]
    assert allocations["gross_matches_target"] is False
    assert allocations["route_difference_msat"] == 90_000_000

    row = result["source_mix"][0]
    assert row["amount_msat"] == 990_000_000
    assert row["asset"] == "BTC"
    # No unsupported target-composition claim, and nothing clamped to 100.
    assert row["percent_of_target"] is None

    narrative = " ".join(result["narrative"]["paragraphs"])
    assert "110" not in narrative
    assert "Gross upstream requirement: 0.00990000 BTC" in narrative
    assert "not automatically classified as a fee" in narrative
    assert all(edge["share_of_target"] is None for edge in result["simplified_flow"]["edges"])


def test_deflating_hop_never_claims_a_share_above_the_target(book):
    """A per-edge allocation above the target is not a share of it, even when the roots sum exactly."""
    conn, _runtime = book
    prepare(conn)
    conn.execute("UPDATE transactions SET amount=? WHERE id='out'", (1_100_000_000,))
    conn.commit()
    hooks = daemon._source_funds_hooks()
    source = source_funds.create_source(
        conn, "ws", "profile", hooks, source_type="fiat_purchase", label="Reviewed purchase",
        amount=btc(1_000_000_000), acquired_at="2024-01-01T00:00:00Z", fiat_value="1000",
    )
    source_funds.create_link(
        conn, "ws", "profile", hooks, from_source_ref=source["id"], to_transaction_ref="out",
        link_type="manual_source", allocation_amount=btc(1_100_000_000),
        from_allocation_amount=btc(1_000_000_000),
    )
    source_funds.create_link(
        conn, "ws", "profile", hooks, from_transaction_ref="out", to_transaction_ref="in",
        allocation_amount=btc(1_000_000_000), from_allocation_amount=btc(1_100_000_000),
    )
    result = report(conn, 1_000_000_000)
    assert result["explain_gates"]["exportable"] is True
    # Roots sum to the target exactly, so the aggregate gate opens...
    assert result["allocations"]["gross_matches_target"] is True
    assert result["source_mix"][0]["percent_of_target"] == 100
    # ...but the inflated hop still may not be labelled a share of the target.
    shares = [edge["share_of_target"] for edge in result["simplified_flow"]["edges"]]
    assert all(share is None or share <= 100 for share in shares), shares


def test_multi_asset_mix_is_never_summed_into_the_target_denomination(book):
    """A peg-out route funds a BTC target from an L-BTC root; the two must stay apart."""
    conn, _runtime = book
    prepare(conn)
    hooks = daemon._source_funds_hooks()
    conn.execute("UPDATE transactions SET amount=?, asset='LBTC' WHERE id='out'", (600_000_000,))
    conn.commit()
    btc_source = source_funds.create_source(
        conn, "ws", "profile", hooks, source_type="fiat_purchase", label="BTC purchase",
        amount=btc(400_000_000), acquired_at="2024-01-01T00:00:00Z", fiat_value="400",
    )
    source_funds.create_link(
        conn, "ws", "profile", hooks, from_source_ref=btc_source["id"], to_transaction_ref="in",
        link_type="manual_source", allocation_amount=btc(400_000_000),
    )
    lbtc_source = source_funds.create_source(
        conn, "ws", "profile", hooks, source_type="fiat_purchase", label="L-BTC purchase",
        asset="LBTC", amount=btc(600_000_000), acquired_at="2024-01-01T00:00:00Z", fiat_value="600",
    )
    source_funds.create_link(
        conn, "ws", "profile", hooks, from_source_ref=lbtc_source["id"], to_transaction_ref="out",
        link_type="manual_source", allocation_amount=btc(600_000_000),
    )
    source_funds.create_link(
        conn, "ws", "profile", hooks, from_transaction_ref="out", to_transaction_ref="in",
        link_type="peg_out", allocation_amount=btc(600_000_000),
        from_allocation_amount=btc(600_000_000),
    )
    result = report(conn, 1_000_000_000)
    assert result["explain_gates"]["exportable"] is True, result["findings"]
    by_asset = {row["asset"]: row for row in result["source_mix"]}
    assert set(by_asset) == {"BTC", "LBTC"}
    assert by_asset["BTC"]["amount_msat"] == 400_000_000
    assert by_asset["LBTC"]["amount_msat"] == 600_000_000
    # Two rows, one category: the category count must not become a row count.
    assert result["overview"]["source_category_count"] == 1
    # Mixed denominations establish no target composition and draw no single ring.
    assert all(row["percent_of_target"] is None for row in result["source_mix"])
    assert result["allocations"]["gross_matches_target"] is False
    assert result["allocations"]["route_difference_msat"] is None
    from kassiber.core import source_funds_diagram

    assert source_funds_diagram.source_mix_ring_spec(result) is None
