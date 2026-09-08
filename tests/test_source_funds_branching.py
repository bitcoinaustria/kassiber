"""Reviewed provenance must not depend on the order or length of its branches."""

from decimal import Decimal

import pytest

from kassiber import daemon
from kassiber.core import source_funds
from kassiber.db import open_db, set_setting
from tests.test_source_funds_custody_projection import _hooks, _setup


UNIT = 1_000_000_000


def btc(units):
    return str(Decimal(units * UNIT) / Decimal(100_000_000_000))


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path)
    _setup(conn)
    conn.execute("DELETE FROM transactions")
    for tx_id, amount, day in (
        ("root", 100, 2), ("a", 60, 3), ("b", 40, 3),
        ("c", 40, 4), ("target", 100, 5),
    ):
        timestamp = f"2026-01-0{day}T00:00:00Z"
        conn.execute(
            """INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
                direction, asset, amount, fee, fiat_currency, fiat_rate, created_at
            ) VALUES(?, 'ws', 'profile', 'a', ?, ?, 'inbound', 'BTC', ?, 0, 'EUR', 40000, ?)""",
            (tx_id, tx_id, timestamp, amount * UNIT, timestamp),
        )
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "profile")
    conn.commit()
    try:
        yield conn
    finally:
        conn.close()


def link(conn, parent, child, amount, ordinal):
    row = source_funds.create_link(
        conn, None, None, _hooks(), from_transaction_ref=parent,
        to_transaction_ref=child, allocation_amount=btc(amount),
        from_allocation_amount=btc(amount),
    )
    # Avoid same-second UUID ordering hiding the shorter-branch-first case.
    conn.execute(
        "UPDATE source_funds_links SET created_at=? WHERE id=?",
        (f"2026-06-01T00:00:{ordinal:02}Z", row["id"]),
    )
    conn.commit()


def prepare(conn, *, extra_hop, reverse_links=False, root_amount=100):
    source = source_funds.create_source(
        conn, None, None, _hooks(), source_type="fiat_purchase",
        label="Reviewed purchase", amount=btc(root_amount),
        acquired_at="2026-01-01T00:00:00Z",
    )
    source_funds.create_link(
        conn, None, None, _hooks(), from_source_ref=source["id"],
        to_transaction_ref="root", link_type="manual_source",
        allocation_amount=btc(100),
    )
    edges = [("root", "a", 60), ("root", "b", 40), ("a", "target", 60)]
    edges += [("b", "c", 40), ("c", "target", 40)] if extra_hop else [("b", "target", 40)]
    if reverse_links:
        edges.reverse()
    for ordinal, edge in enumerate(edges):
        link(conn, *edge, ordinal)


def preview(conn, **args):
    return daemon._ui_source_funds_payload_from_conn(
        conn, "ui.source_funds.preview", {"target_transaction": "target", **args},
    )


@pytest.mark.parametrize("extra_hop", [False, True])
@pytest.mark.parametrize("reverse_links", [False, True])
def test_shared_ancestor_aggregates_every_reviewed_branch(book, extra_hop, reverse_links):
    prepare(book, extra_hop=extra_hop, reverse_links=reverse_links)
    result = preview(book)
    assert result["explain_gates"]["exportable"], result["findings"]
    assert result["source_mix"][0]["amount_msat"] == 100 * UNIT
    assert result["source_mix"][0]["percent_of_target"] == 100
    root = next(node for node in result["graph"]["nodes"] if node["id"] == "tx:root")
    assert root["required_amount_msat"] == 100 * UNIT


def test_shared_ancestor_still_checks_total_root_capacity(book):
    prepare(book, extra_hop=True, root_amount=90)
    result = preview(book)
    assert "source_overallocation" in {item["code"] for item in result["findings"]}
    assert not result["explain_gates"]["exportable"]


def test_branching_does_not_invent_a_partial_source_selection(book):
    prepare(book, extra_hop=True)
    result = preview(book, target_amount=btc(90))
    assert "ambiguous_allocation" in {item["code"] for item in result["findings"]}
    assert not result["explain_gates"]["exportable"]


def test_longer_branch_still_obeys_disclosure_depth(book):
    prepare(book, extra_hop=True)
    result = preview(book, max_depth=3)
    assert "path_truncated" in {item["code"] for item in result["findings"]}
    assert not result["explain_gates"]["exportable"]
