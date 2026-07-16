from __future__ import annotations

import sqlite3
from unittest.mock import patch

import pytest

from kassiber.core.custody_authored_migration import (
    backfill_legacy_authored_components,
)
from kassiber.core.custody_components import (
    create_component,
    get_component,
    seal_component_economic_terms,
)
from kassiber.db import open_db


NOW = "2026-01-01T00:00:00Z"


def _scope(conn: sqlite3.Connection) -> None:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', ?)",
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            gains_algorithm, created_at
        ) VALUES('profile', 'ws', 'Book', 'EUR', 'generic', 'FIFO', ?)
        """,
        (NOW,),
    )
    for wallet_id, chain in (("btc", "bitcoin"), ("liquid", "liquid")):
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, 'descriptor', ?, ?)
            """,
            (
                wallet_id,
                wallet_id,
                f'{{"chain":"{chain}","network":"main"}}',
                NOW,
            ),
        )


def _tx(
    conn: sqlite3.Connection,
    tx_id: str,
    wallet_id: str,
    direction: str,
    asset: str,
    amount: int,
    occurred_at: str,
    *,
    fee: int = 0,
) -> None:
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, direction, asset, amount, fee, kind, raw_json, created_at
        ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, ?, ?, ?, 'transfer', '{}', ?)
        """,
        (
            tx_id,
            wallet_id,
            tx_id,
            f"fp:{tx_id}",
            occurred_at,
            direction,
            asset,
            amount,
            fee,
            occurred_at,
        ),
    )


def _legacy_rows(conn: sqlite3.Connection) -> None:
    _scope(conn)
    _tx(conn, "pair-out", "btc", "outbound", "BTC", 1_000, NOW, fee=10)
    _tx(
        conn,
        "pair-in",
        "btc",
        "inbound",
        "BTC",
        900,
        "2026-01-02T00:00:00Z",
    )
    _tx(
        conn,
        "payout-out",
        "btc",
        "outbound",
        "BTC",
        2_000,
        "2026-01-03T00:00:00Z",
    )
    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, swap_fee_msat, swap_fee_kind,
            confidence_at_pair, pair_source, out_amount, created_at
        ) VALUES('pair', 'ws', 'profile', 'pair-out', 'pair-in',
                 'chain-swap', 'carrying-value', 'reviewed pair', 7, 'service',
                 'strong', 'manual', 900, ?)
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO direct_swap_payouts(
            id, workspace_id, profile_id, out_transaction_id, kind, policy,
            payout_asset, payout_amount, payout_occurred_at, payout_fiat_value,
            payout_external_id, counterparty, notes, swap_fee_msat,
            swap_fee_kind, out_amount, created_at
        ) VALUES('payout', 'ws', 'profile', 'payout-out',
                 'direct-swap-payout', 'carrying-value', 'LBTC', 1950,
                 '2026-01-04T00:00:00Z', 123.45678901234567, 'settlement-1',
                 'swap desk', 'reviewed payout', -50, 'provider-rebate', 2000, ?)
        """,
        (NOW,),
    )
    conn.commit()


def test_reopen_migrates_legacy_reviews_to_inert_exact_drafts(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    conn.close()

    migrated = open_db(tmp_path)
    try:
        pair_link = migrated.execute(
            "SELECT component_id FROM transaction_pairs WHERE id = 'pair'"
        ).fetchone()[0]
        payout_link = migrated.execute(
            "SELECT component_id FROM direct_swap_payouts WHERE id = 'payout'"
        ).fetchone()[0]
        assert pair_link and payout_link and pair_link != payout_link

        pair = get_component(migrated, pair_link)
        assert pair["state"] == "draft"
        assert pair["authored_source"] == "migration"
        assert pair["component_type"] == "manual_bridge"
        assert [leg["amount_msat"] for leg in pair["legs"]] == [900, 900]
        assert pair["allocations"][0]["source_amount_msat"] == 900
        assert len(pair["economic_terms"]) == 1
        terms = pair["economic_terms"][0]
        assert terms["term_kind"] == "transaction_pair"
        assert terms["legacy_source_id"] == "pair"
        assert terms["review_kind"] == "chain-swap"
        assert terms["tax_policy"] == "carrying-value"
        assert terms["reviewed_source_amount_msat"] == 900
        assert terms["swap_fee_msat"] == 7
        assert terms["swap_fee_kind"] == "service"
        assert terms["confidence_at_review"] == "strong"
        assert terms["review_source"] == "manual"

        payout = get_component(migrated, payout_link)
        assert payout["state"] == "draft"
        assert payout["component_type"] == "swap"
        assert payout["conservation_mode"] == "conversion"
        assert [leg["amount_msat"] for leg in payout["legs"]] == [2000, 1950]
        assert len(payout["economic_terms"]) == 1
        terms = payout["economic_terms"][0]
        assert terms["term_kind"] == "direct_swap_payout"
        assert terms["payout_asset"] == "LBTC"
        assert terms["payout_amount_msat"] == 1950
        assert terms["payout_fiat_value_exact"] == "123.45678901234567"
        assert terms["payout_external_id"] == "settlement-1"
        assert terms["counterparty"] == "swap desk"
        assert terms["swap_fee_msat"] == -50
    finally:
        migrated.close()


def test_backfill_is_idempotent_and_legacy_edits_create_a_revision(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    first = backfill_legacy_authored_components(conn)
    assert first.created == 2
    first_ids = {
        row["id"]: row["component_id"]
        for row in conn.execute(
            "SELECT id, component_id FROM transaction_pairs "
            "UNION ALL SELECT id, component_id FROM direct_swap_payouts"
        )
    }
    assert backfill_legacy_authored_components(conn).unchanged == 2
    assert conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0] == 2

    conn.execute("UPDATE transaction_pairs SET notes = 'revised review' WHERE id = 'pair'")
    revised = backfill_legacy_authored_components(conn)
    assert revised.revised == 1
    new_id = conn.execute(
        "SELECT component_id FROM transaction_pairs WHERE id = 'pair'"
    ).fetchone()[0]
    assert new_id != first_ids["pair"]
    old = get_component(conn, first_ids["pair"])
    new = get_component(conn, new_id)
    assert old["state"] == "superseded"
    assert new["state"] == "draft"
    assert old["lineage_id"] == new["lineage_id"]
    assert new["revision"] == 2
    assert conn.execute(
        "SELECT COUNT(*) FROM custody_component_economic_terms "
        "WHERE legacy_source_id = 'pair'"
    ).fetchone()[0] == 2
    conn.close()


def test_backfill_rolls_back_component_terms_and_links_together(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)

    from kassiber.core import custody_authored_migration as migration

    real_insert = migration._insert_terms
    calls = 0

    def fail_second_insert(*args, **kwargs):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise RuntimeError("fault injection")
        return real_insert(*args, **kwargs)

    with patch.object(migration, "_insert_terms", side_effect=fail_second_insert):
        with pytest.raises(RuntimeError, match="fault injection"):
            backfill_legacy_authored_components(conn)

    assert conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0] == 0
    assert conn.execute(
        "SELECT COUNT(*) FROM custody_component_economic_terms"
    ).fetchone()[0] == 0
    assert conn.execute(
        "SELECT COUNT(*) FROM transaction_pairs WHERE component_id IS NOT NULL"
    ).fetchone()[0] == 0
    assert conn.execute(
        "SELECT COUNT(*) FROM direct_swap_payouts WHERE component_id IS NOT NULL"
    ).fetchone()[0] == 0
    conn.close()


def test_component_aggregate_accepts_multiple_leg_bound_economic_terms(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    backfill_legacy_authored_components(conn)
    component_id = conn.execute(
        "SELECT component_id FROM transaction_pairs WHERE id = 'pair'"
    ).fetchone()[0]
    staged = get_component(conn, component_id)
    aggregate_legs = [
        {
            **{
                key: value
                for key, value in leg.items()
                if key
                in {
                    "role", "rail", "chain", "network", "asset", "exposure",
                    "conservation_unit", "amount_msat", "valuation_unit",
                    "valuation_amount", "occurred_at", "transaction_id",
                    "anchor_transaction_id", "wallet_id", "location_ref", "notes",
                }
            },
            "id": f"multi-leg-{index}",
        }
        for index, leg in enumerate(staged["legs"])
    ]
    aggregate = create_component(
        conn,
        workspace_id="ws",
        profile_id="profile",
        component_id="multi-term-component",
        component_type=staged["component_type"],
        conservation_mode=staged["conservation_mode"],
        legs=aggregate_legs,
        allocations=[
            {
                "source_ordinal": 0,
                "sink_ordinal": 1,
                "source_amount_msat": staged["allocations"][0][
                    "source_amount_msat"
                ],
                "sink_amount_msat": staged["allocations"][0]["sink_amount_msat"],
            }
        ],
        created_at=NOW,
    )
    source_leg_id = aggregate["legs"][0]["id"]
    target_leg_id = aggregate["legs"][1]["id"]
    seal_component_economic_terms(
        conn,
        aggregate["id"],
        [
            {
                "id": "first-term",
                "source_leg_id": source_leg_id,
                "target_leg_id": target_leg_id,
                "term_kind": "transaction_pair",
                "legacy_source_id": "pair",
                "source_row_hash": "ab" * 32,
                "review_kind": "manual",
                "tax_policy": "carrying-value",
                "reviewed_source_amount_msat": 800,
            },
            {
                "id": "second-term",
                "source_leg_id": source_leg_id,
                "target_leg_id": target_leg_id,
                "term_kind": "transaction_pair",
                "legacy_source_id": "pair-2",
                "source_row_hash": "ef" * 32,
                "review_kind": "manual",
                "tax_policy": "carrying-value",
                "reviewed_source_amount_msat": 100,
            },
        ],
    )
    terms = get_component(conn, aggregate["id"])["economic_terms"]
    assert [term["legacy_source_id"] for term in terms] == ["pair", "pair-2"]
    assert get_component(conn, aggregate["id"])["expected_economic_term_count"] == 2
    with pytest.raises(sqlite3.IntegrityError, match="terms_immutable"):
        conn.execute(
            "UPDATE custody_component_economic_terms SET tax_policy = 'sale' "
            "WHERE id = 'second-term'"
        )
    conn.execute(
        "INSERT INTO custody_component_purge_authorizations(profile_id) "
        "VALUES('profile')"
    )
    conn.execute(
        "DELETE FROM custody_component_economic_terms WHERE id = 'second-term'"
    )
    conn.execute(
        "DELETE FROM custody_component_purge_authorizations WHERE profile_id = 'profile'"
    )
    partial = get_component(conn, aggregate["id"])
    assert partial["effective_state"] == "draft"
    assert "component_economic_term_count_mismatch" in {
        issue["code"] for issue in partial["validation"]["issues"]
    }
    conn.close()
