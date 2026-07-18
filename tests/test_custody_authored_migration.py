from __future__ import annotations

import sqlite3
from unittest.mock import patch

import pytest

from kassiber import db as db_module
from kassiber.cli.handlers import (
    create_direct_swap_payout,
    create_transaction_pair,
    list_direct_swap_payouts,
    list_transaction_pairs,
    list_transactions,
)
from kassiber.core.custody_authored_migration import (
    find_active_review_for_transaction,
    list_active_review_refs,
    refresh_legacy_authored_components,
)
from kassiber.core.custody_components import (
    activate_component,
    create_component,
    get_component,
    iter_authored_active_components,
)
from kassiber.core.custody_journal import (
    CustodyJournalBuilder,
    process_journals as process_custody_journals,
)
from kassiber.db import open_db
from kassiber.errors import AppError


NOW = "2026-01-01T00:00:00Z"


class _StopAfterWriterProbe(Exception):
    pass


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
    for wallet_id, chain in (
        ("btc", "bitcoin"),
        ("btc2", "bitcoin"),
        ("liquid", "liquid"),
    ):
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
        "btc2",
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


def test_reopen_migrates_legacy_reviews_to_active_exact_components(tmp_path):
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
        assert pair["state"] == "active"
        assert pair["effective_state"] == "active"
        assert pair["authored_source"] == "migration"
        assert pair["component_type"] == "manual_bridge"
        assert sorted(
            (leg["role"], leg["amount_msat"]) for leg in pair["legs"]
        ) == [
            ("destination", 900),
            ("fee", 10),
            ("source", 1000),
            ("suspense", 90),
        ]
        assert sum(
            allocation["source_amount_msat"]
            for allocation in pair["allocations"]
        ) == 1000
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
        assert terms["review_notes"] == "reviewed pair"

        payout = get_component(migrated, payout_link)
        assert payout["state"] == "active"
        assert payout["effective_state"] == "active"
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
        assert terms["review_notes"] == "reviewed payout"

        safe_components = {
            component["id"]: component
            for component in iter_authored_active_components(
                migrated,
                profile_id="profile",
                include_local_evidence=False,
            )
        }
        safe_payout = safe_components[payout["id"]]
        assert safe_payout["effective_state"] == "active"
        assert safe_payout["validation"]["activatable"] is True
        assert all("location_ref" not in leg for leg in safe_payout["legs"])

        profile = migrated.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()
        decisions = CustodyJournalBuilder(
            migrated, profile
        ).build_custody_decisions()
        assert [record["id"] for record in decisions.direct_payout_records] == [
            "payout"
        ]
        assert decisions.direct_payout_records[0]["component_id"] == payout["id"]

        listed_payouts = list_direct_swap_payouts(migrated, "ws", "profile")
        assert len(listed_payouts) == 1
        assert listed_payouts[0]["id"] == "payout"
        assert listed_payouts[0]["notes"] == "reviewed payout"
        assert listed_payouts[0]["payout"]["amount_msat"] == 1950

        refs = list_active_review_refs(migrated, profile_id="profile")
        assert {
            (
                row["id"],
                row["term_kind"],
                row["out_asset"],
                row["in_asset"],
            )
            for row in refs
        } == {
            ("pair", "transaction_pair", "BTC", "BTC"),
            ("payout", "direct_swap_payout", "BTC", None),
        }
        assert find_active_review_for_transaction(
            migrated,
            profile_id="profile",
            transaction_id="pair-out",
        ) == {
            "id": "pair",
            "component_id": pair["id"],
            "term_kind": "transaction_pair",
        }
        assert find_active_review_for_transaction(
            migrated,
            profile_id="profile",
            transaction_id="payout-out",
        ) == {
            "id": "payout",
            "component_id": payout["id"],
            "term_kind": "direct_swap_payout",
        }
    finally:
        migrated.close()

    reopened = open_db(tmp_path)
    try:
        assert reopened.execute(
            "SELECT COUNT(*) FROM custody_components"
        ).fetchone()[0] == 2
        assert reopened.execute(
            "SELECT COUNT(*) FROM custody_components WHERE state = 'active'"
        ).fetchone()[0] == 2
    finally:
        reopened.close()


def test_many_to_one_pair_list_uses_allocation_amounts_on_both_sides(tmp_path):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, "out-a", "btc", "outbound", "BTC", 60, NOW)
        _tx(conn, "out-b", "btc", "outbound", "BTC", 40, NOW)
        _tx(conn, "in-shared", "btc2", "inbound", "BTC", 100, NOW)
        component = create_component(
            conn,
            workspace_id="ws",
            profile_id="profile",
            component_id="many-to-one",
            component_type="manual_bridge",
            legs=[
                {
                    "id": "source-a",
                    "role": "source",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "main",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 60,
                    "transaction_id": "out-a",
                    "wallet_id": "btc",
                },
                {
                    "id": "source-b",
                    "role": "source",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "main",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 40,
                    "transaction_id": "out-b",
                    "wallet_id": "btc",
                },
                {
                    "id": "sink",
                    "role": "destination",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "main",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100,
                    "transaction_id": "in-shared",
                    "wallet_id": "btc2",
                },
            ],
            allocations=[
                {
                    "id": "allocation-a",
                    "source_leg_id": "source-a",
                    "sink_leg_id": "sink",
                    "source_amount_msat": 60,
                    "sink_amount_msat": 60,
                },
                {
                    "id": "allocation-b",
                    "source_leg_id": "source-b",
                    "sink_leg_id": "sink",
                    "source_amount_msat": 40,
                    "sink_amount_msat": 40,
                },
            ],
            economic_terms=[
                {
                    "id": "term-a",
                    "source_leg_id": "source-a",
                    "target_leg_id": "sink",
                    "term_kind": "transaction_pair",
                    "legacy_source_id": "pair-a",
                    "source_row_hash": "aa" * 32,
                    "review_kind": "manual",
                    "tax_policy": "carrying-value",
                    "reviewed_source_amount_msat": 60,
                },
                {
                    "id": "term-b",
                    "source_leg_id": "source-b",
                    "target_leg_id": "sink",
                    "term_kind": "transaction_pair",
                    "legacy_source_id": "pair-b",
                    "source_row_hash": "bb" * 32,
                    "review_kind": "manual",
                    "tax_policy": "carrying-value",
                    "reviewed_source_amount_msat": 40,
                },
            ],
            created_at=NOW,
        )
        activate_component(conn, component["id"], activated_at=NOW)

        pairs = sorted(
            list_transaction_pairs(conn, "ws", "profile"),
            key=lambda row: row["id"],
        )

        assert [row["out"]["amount_msat"] for row in pairs] == [60, 40]
        assert [row["in"]["amount_msat"] for row in pairs] == [60, 40]
        assert [row["in"]["full_amount_msat"] for row in pairs] == [100, 100]
        assert all(
            row["component"]
            == {
                "id": "many-to-one",
                "source_count": 2,
                "sink_count": 1,
                "allocation_count": 2,
            }
            for row in pairs
        )
    finally:
        conn.close()


def test_journal_processing_reserves_writer_before_projection_reads(tmp_path):
    owner = open_db(tmp_path)
    contender = None
    try:
        _scope(owner)
        owner.commit()
        contender = open_db(tmp_path)
        contender.execute("PRAGMA busy_timeout = 1")
        phases: list[str] = []

        def probe_writer(_conn, _profile):
            with pytest.raises(sqlite3.OperationalError) as raised:
                contender.execute("BEGIN IMMEDIATE")
            assert raised.value.sqlite_errorname.startswith("SQLITE_BUSY")
            raise _StopAfterWriterProbe

        with pytest.raises(_StopAfterWriterProbe):
            process_custody_journals(
                owner,
                "ws",
                "profile",
                repair_source_overlaps=lambda *_args: None,
                source_overlap_warning=lambda *_args: None,
                auto_price=probe_writer,
                progress_observer=lambda payload: phases.append(payload["phase"]),
            )

        assert phases[:3] == ["writer_wait", "preparing", "repairing"]
        assert "pricing" in phases
        assert not owner.in_transaction
    finally:
        if contender is not None:
            contender.rollback()
            contender.close()
        owner.close()


def test_malformed_legacy_review_becomes_persisted_journal_blocker(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    conn.execute(
        "UPDATE direct_swap_payouts SET out_amount = 3000 WHERE id = 'payout'"
    )
    conn.commit()
    conn.close()

    migrated = open_db(tmp_path)
    try:
        issue = migrated.execute(
            "SELECT * FROM custody_authored_migration_issues "
            "WHERE legacy_source_id = 'payout' AND resolved_at IS NULL"
        ).fetchone()
        assert issue is not None
        assert issue["legacy_table"] == "direct_swap_payouts"

        profile = migrated.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()
        decisions = CustodyJournalBuilder(
            migrated, profile
        ).build_custody_decisions()
        assert "payout-out" in decisions.interpretation.blocked_transaction_ids
        quarantine = next(
            item
            for item in decisions.interpretation.quarantines
            if item["transaction_id"] == "payout-out"
        )
        assert quarantine["reason"] == "custody_authored_migration_incomplete"
        assert decisions.direct_payout_records == []
    finally:
        migrated.close()


def test_deleted_legacy_reviews_migrate_to_component_history(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    conn.execute("UPDATE transaction_pairs SET deleted_at = ?", (NOW,))
    conn.execute("UPDATE direct_swap_payouts SET deleted_at = ?", (NOW,))
    conn.commit()
    conn.close()

    migrated = open_db(tmp_path)
    try:
        assert list_transaction_pairs(migrated, "ws", "profile") == []
        assert list_direct_swap_payouts(migrated, "ws", "profile") == []
        deleted_pairs = list_transaction_pairs(
            migrated, "ws", "profile", include_deleted=True
        )
        deleted_payouts = list_direct_swap_payouts(
            migrated, "ws", "profile", include_deleted=True
        )
        assert [row["id"] for row in deleted_pairs] == ["pair"]
        assert [row["id"] for row in deleted_payouts] == ["payout"]
        assert deleted_pairs[0]["deleted_at"] == NOW
        assert deleted_payouts[0]["deleted_at"] == NOW
        assert migrated.execute(
            "SELECT COUNT(*) FROM custody_components WHERE state = 'active'"
        ).fetchone()[0] == 0
    finally:
        migrated.close()


def test_unchanged_rows_resolve_stale_migration_issues(tmp_path):
    # A durable issue can outlive its cause when a peer migrates the same
    # legacy row and replicates the component. The next refresh must resolve
    # the stale issue on the unchanged path instead of skipping past it.
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    conn.execute(
        "UPDATE direct_swap_payouts SET deleted_at = ? WHERE id = 'payout'",
        (NOW,),
    )
    conn.commit()
    conn.close()

    migrated = open_db(tmp_path)
    for legacy_table, legacy_source_id in (
        ("transaction_pairs", "pair"),
        ("direct_swap_payouts", "payout"),
    ):
        migrated.execute(
            """
            INSERT INTO custody_authored_migration_issues(
                id, workspace_id, profile_id, legacy_table, legacy_source_id,
                issue_code, transaction_ids_json, details_json,
                resolved_at, created_at, updated_at
            ) VALUES(?, 'ws', 'profile', ?, ?, 'custody_legacy_migration_failed',
                     '[]', '{}', NULL, ?, ?)
            """,
            (
                f"stale-issue-{legacy_source_id}",
                legacy_table,
                legacy_source_id,
                NOW,
                NOW,
            ),
        )
    migrated.commit()
    migrated.close()

    refreshed = open_db(tmp_path)
    try:
        open_issues = refreshed.execute(
            "SELECT legacy_source_id FROM custody_authored_migration_issues "
            "WHERE resolved_at IS NULL"
        ).fetchall()
        assert open_issues == []
    finally:
        refreshed.close()


def test_active_component_without_legacy_terms_claims_every_boundary(tmp_path):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, "bridge-out", "btc", "outbound", "BTC", 990, NOW)
    _tx(conn, "bridge-in", "btc", "inbound", "BTC", 990, NOW)
    component = create_component(
        conn,
        workspace_id="ws",
        profile_id="profile",
        component_id="native-bridge",
        component_type="manual_bridge",
        conservation_mode="quantity",
        evidence_kind="manual_reconstruction",
        evidence_grade="reviewed",
        legs=[
            {
                "id": "native-source",
                "role": "source",
                "rail": "bitcoin",
                "chain": "bitcoin",
                "network": "main",
                "asset": "BTC",
                "exposure": "bitcoin",
                "conservation_unit": "msat",
                "amount_msat": 990,
                "occurred_at": NOW,
                "transaction_id": "bridge-out",
                "anchor_transaction_id": "bridge-out",
                "wallet_id": "btc",
            },
            {
                "id": "native-target",
                "role": "destination",
                "rail": "bitcoin",
                "chain": "bitcoin",
                "network": "main",
                "asset": "BTC",
                "exposure": "bitcoin",
                "conservation_unit": "msat",
                "amount_msat": 990,
                "occurred_at": NOW,
                "transaction_id": "bridge-in",
                "anchor_transaction_id": "bridge-in",
                "wallet_id": "btc",
            },
        ],
        allocations=[
            {
                "source_ordinal": 0,
                "sink_ordinal": 1,
                "source_amount_msat": 990,
                "sink_amount_msat": 990,
            }
        ],
        authored_source="user",
        created_at=NOW,
    )
    activate_component(conn, component["id"], activated_at=NOW)

    refs = list_active_review_refs(conn, profile_id="profile")
    assert {
        (row["out_transaction_id"], row["in_transaction_id"])
        for row in refs
    } == {("bridge-out", None), (None, "bridge-in")}
    for transaction_id in ("bridge-out", "bridge-in"):
        review = find_active_review_for_transaction(
            conn,
            profile_id="profile",
            transaction_id=transaction_id,
        )
        assert review["id"] == "native-bridge"
        assert review["component_id"] == "native-bridge"
    with pytest.raises(AppError, match="active custody component"):
        create_transaction_pair(
            conn,
            "ws",
            "profile",
            "bridge-out",
            "bridge-in",
        )
    with pytest.raises(AppError, match="active custody component"):
        create_direct_swap_payout(
            conn,
            "ws",
            "profile",
            "bridge-out",
            payout_asset="BTC",
            payout_amount="0.0000000099",
        )
    conn.close()


def test_transaction_flow_filters_use_only_current_custody_projection(tmp_path):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, "flow-out", "btc", "outbound", "BTC", 990, NOW)
    _tx(conn, "flow-in", "btc", "inbound", "BTC", 990, NOW)
    conn.execute(
        "UPDATE transactions SET kind = 'payment' "
        "WHERE id IN ('flow-out', 'flow-in')"
    )
    conn.execute(
        """
        UPDATE profiles
        SET last_processed_at = ?, journal_input_version = 1,
            last_processed_input_version = 1,
            last_processed_tx_count = 2
        WHERE id = 'profile'
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO journal_custody_decisions(
            decision_id, workspace_id, profile_id,
            source_transaction_id, target_transaction_id,
            source_observation_hash, source_start_msat, source_end_msat,
            target_observation_hash, target_start_msat, target_end_msat,
            source_network, target_network, source_rail, target_rail,
            source_asset, target_asset, state, basis_state, reason,
            created_at
        ) VALUES(
            ?, 'ws', 'profile', 'flow-out', 'flow-in',
            ?, 0, 990, ?, 0, 990,
            'main', 'main', 'bitcoin', 'bitcoin',
            'BTC', 'BTC', 'internal_reviewed', 'eligible',
            'reviewed_component', ?
        )
        """,
        ("a" * 64, "b" * 64, "c" * 64, NOW),
    )

    transfers, _ = list_transactions(conn, "ws", "profile", flow="transfer")
    assert {row["id"] for row in transfers} == {"flow-out", "flow-in"}
    incoming, _ = list_transactions(conn, "ws", "profile", flow="incoming")
    assert incoming == []

    conn.execute(
        "UPDATE profiles SET journal_input_version = 2 WHERE id = 'profile'"
    )
    stale_transfers, _ = list_transactions(
        conn,
        "ws",
        "profile",
        flow="transfer",
    )
    assert stale_transfers == []
    stale_incoming, _ = list_transactions(
        conn,
        "ws",
        "profile",
        flow="incoming",
    )
    assert [row["id"] for row in stale_incoming] == ["flow-in"]
    conn.close()


def test_single_phase_migration_is_idempotent_and_freezes_legacy_edits(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    first = refresh_legacy_authored_components(conn)
    assert first.activated == 2
    first_ids = {
        row["id"]: row["component_id"]
        for row in conn.execute(
            "SELECT id, component_id FROM transaction_pairs "
            "UNION ALL SELECT id, component_id FROM direct_swap_payouts"
        )
    }
    statements: list[str] = []
    conn.set_trace_callback(statements.append)
    try:
        assert refresh_legacy_authored_components(conn).changed is False
    finally:
        conn.set_trace_callback(None)
    assert not [
        statement
        for statement in statements
        if "JOIN transactions out_tx" in statement
    ]
    for table in ("transaction_pairs", "direct_swap_payouts"):
        plan = conn.execute(
            f"EXPLAIN QUERY PLAN SELECT COUNT(*) FROM {table} "
            "WHERE component_id IS NULL"
        ).fetchall()
        assert any("component_pending" in row["detail"] for row in plan)
    assert conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0] == 2
    assert conn.execute(
        "SELECT COUNT(*) FROM custody_components WHERE state = 'draft'"
    ).fetchone()[0] == 0
    with pytest.raises(sqlite3.IntegrityError, match="legacy_custody_review_write_frozen"):
        conn.execute(
            "UPDATE transaction_pairs SET notes = 'revised review' WHERE id = 'pair'"
        )
    assert {
        row["id"]: row["component_id"]
        for row in conn.execute(
            "SELECT id, component_id FROM transaction_pairs "
            "UNION ALL SELECT id, component_id FROM direct_swap_payouts"
        )
    } == first_ids
    conn.close()


def test_connected_fanout_pairs_activate_as_one_atomic_component(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    _tx(
        conn,
        "pair-in-2",
        "btc2",
        "inbound",
        "BTC",
        100,
        "2026-01-02T01:00:00Z",
    )
    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, pair_source, out_amount, created_at
        ) VALUES('pair-2', 'ws', 'profile', 'pair-out', 'pair-in-2',
                 'manual', 'carrying-value', 'fanout remainder', 'manual', 100, ?)
        """,
        (NOW,),
    )
    conn.commit()
    conn.close()

    migrated = open_db(tmp_path)
    try:
        links = {
            row["component_id"]
            for row in migrated.execute(
                "SELECT component_id FROM transaction_pairs "
                "WHERE id IN ('pair', 'pair-2')"
            )
        }
        assert len(links) == 1
        component = get_component(migrated, links.pop())
        assert component["effective_state"] == "active"
        assert len(component["economic_terms"]) == 2
        assert sum(
            allocation["source_amount_msat"]
            for allocation in component["allocations"]
        ) == 1000
        assert {
            leg["anchor_transaction_id"]
            for leg in component["legs"]
            if leg["role"] == "destination"
        } == {"pair-in", "pair-in-2"}
        memberships = migrated.execute(
            "SELECT transaction_id FROM custody_component_transaction_memberships "
            "WHERE component_id = ? ORDER BY transaction_id",
            (component["id"],),
        ).fetchall()
        assert [row["transaction_id"] for row in memberships] == [
            "pair-in",
            "pair-in-2",
            "pair-out",
        ]
        listed = list_transaction_pairs(migrated, "ws", "profile")
        assert [(row["id"], row["notes"]) for row in listed] == [
            ("pair-2", "fanout remainder"),
            ("pair", "reviewed pair"),
        ]
    finally:
        migrated.close()


def test_delayed_legacy_pair_replay_rebuilds_one_atomic_group(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    refresh_legacy_authored_components(conn)
    first_component_id = conn.execute(
        "SELECT component_id FROM transaction_pairs WHERE id = 'pair'"
    ).fetchone()[0]
    _tx(
        conn,
        "pair-in-late",
        "btc2",
        "inbound",
        "BTC",
        100,
        "2026-01-02T02:00:00Z",
    )
    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, pair_source, out_amount, created_at
        ) VALUES(
            'pair-late', 'ws', 'profile', 'pair-out', 'pair-in-late',
            'manual', 'carrying-value', 'delayed signed replay', 'sync', 100, ?
        )
        """,
        (NOW,),
    )

    replay = refresh_legacy_authored_components(conn)

    assert replay.activated == 1
    links = {
        row["component_id"]
        for row in conn.execute(
            "SELECT component_id FROM transaction_pairs "
            "WHERE id IN ('pair', 'pair-late')"
        )
    }
    assert len(links) == 1
    active_id = links.pop()
    assert active_id != first_component_id
    assert get_component(conn, first_component_id)["state"] == "superseded"
    active = get_component(conn, active_id)
    assert active["effective_state"] == "active"
    assert len(active["economic_terms"]) == 2
    assert conn.execute(
        "SELECT COUNT(*) FROM custody_components WHERE state = 'draft'"
    ).fetchone()[0] == 0
    conn.close()


def test_component_native_pair_creation_grows_one_atomic_fanin(tmp_path):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, "fanin-out-a", "btc", "outbound", "BTC", 400, NOW)
    _tx(conn, "fanin-out-b", "btc", "outbound", "BTC", 600, NOW)
    _tx(conn, "fanin-in", "btc2", "inbound", "BTC", 1_000, NOW)

    first = create_transaction_pair(
        conn,
        "ws",
        "profile",
        "fanin-out-a",
        "fanin-in",
        kind="whirlpool",
        policy="carrying-value",
    )
    second = create_transaction_pair(
        conn,
        "ws",
        "profile",
        "fanin-out-b",
        "fanin-in",
        kind="whirlpool",
        policy="carrying-value",
    )

    assert first["component_id"] != second["component_id"]
    assert get_component(conn, first["component_id"])["state"] == "superseded"
    component = get_component(conn, second["component_id"])
    assert component["effective_state"] == "active"
    assert len(component["economic_terms"]) == 2
    assert sum(
        allocation["source_amount_msat"]
        for allocation in component["allocations"]
    ) == 1_000
    assert sum(
        allocation["sink_amount_msat"]
        for allocation in component["allocations"]
    ) == 1_000
    assert {
        leg["anchor_transaction_id"]
        for leg in component["legs"]
        if leg["role"] == "source"
    } == {"fanin-out-a", "fanin-out-b"}
    assert {
        row["component_id"]
        for row in list_transaction_pairs(conn, "ws", "profile")
    } == {second["component_id"]}
    conn.close()


def test_single_phase_migration_rolls_back_components_terms_and_links_together(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)

    from kassiber.core import custody_components as components

    real_insert = components._insert_economic_terms
    calls = 0

    def fail_second_insert(*args, **kwargs):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise RuntimeError("fault injection")
        return real_insert(*args, **kwargs)

    with patch.object(
        components,
        "_insert_economic_terms",
        side_effect=fail_second_insert,
    ):
        with pytest.raises(RuntimeError, match="fault injection"):
            refresh_legacy_authored_components(conn)

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
    refresh_legacy_authored_components(conn)
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
        economic_terms=[
            {
                "id": "first-term",
                "source_ordinal": 0,
                "target_ordinal": 1,
                "term_kind": "transaction_pair",
                "legacy_source_id": "pair",
                "source_row_hash": "ab" * 32,
                "review_kind": "manual",
                "tax_policy": "carrying-value",
                "reviewed_source_amount_msat": 800,
            },
            {
                "id": "second-term",
                "source_ordinal": 0,
                "target_ordinal": 1,
                "term_kind": "transaction_pair",
                "legacy_source_id": "pair-2",
                "source_row_hash": "ef" * 32,
                "review_kind": "manual",
                "tax_policy": "carrying-value",
                "reviewed_source_amount_msat": 100,
            },
        ],
        created_at=NOW,
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


def test_leg_role_rebuild_preserves_existing_economic_term_foreign_keys(tmp_path):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    assert refresh_legacy_authored_components(conn).activated == 2
    before = [
        tuple(row)
        for row in conn.execute(
            "SELECT id, component_id, source_leg_id, target_leg_id "
            "FROM custody_component_economic_terms ORDER BY id"
        )
    ]

    db_module._rebuild_custody_leg_role_schema(conn)

    after = [
        tuple(row)
        for row in conn.execute(
            "SELECT id, component_id, source_leg_id, target_leg_id "
            "FROM custody_component_economic_terms ORDER BY id"
        )
    ]
    assert after == before
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    assert {
        row["table"]
        for row in conn.execute(
            "PRAGMA foreign_key_list(custody_component_economic_terms)"
        )
    } >= {"custody_components", "custody_component_legs"}
    conn.close()


def test_review_notes_schema_migration_backfills_and_refreezes_terms():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE transaction_pairs(
            id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, notes TEXT
        );
        CREATE TABLE direct_swap_payouts(
            id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, notes TEXT
        );
        CREATE TABLE custody_component_economic_terms(
            id TEXT PRIMARY KEY,
            profile_id TEXT NOT NULL,
            term_kind TEXT NOT NULL,
            legacy_source_id TEXT NOT NULL
        );
        CREATE TRIGGER trg_custody_component_terms_immutable
        BEFORE UPDATE ON custody_component_economic_terms
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_terms_immutable');
        END;
        INSERT INTO transaction_pairs VALUES('pair', 'profile', 'pair note');
        INSERT INTO direct_swap_payouts
        VALUES('payout', 'profile', 'payout note');
        INSERT INTO custody_component_economic_terms
        VALUES('pair-term', 'profile', 'transaction_pair', 'pair');
        INSERT INTO custody_component_economic_terms
        VALUES('payout-term', 'profile', 'direct_swap_payout', 'payout');
        """
    )

    assert db_module._ensure_custody_economic_term_review_notes(conn) is True
    assert [
        row["review_notes"]
        for row in conn.execute(
            "SELECT review_notes FROM custody_component_economic_terms ORDER BY id"
        )
    ] == ["pair note", "payout note"]
    assert db_module._ensure_custody_economic_term_review_notes(conn) is False
    with pytest.raises(sqlite3.IntegrityError, match="terms_immutable"):
        conn.execute(
            "UPDATE custody_component_economic_terms "
            "SET review_notes = 'changed' WHERE id = 'pair-term'"
        )
    conn.close()
