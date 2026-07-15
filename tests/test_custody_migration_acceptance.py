"""Versioned migration acceptance for authored custody and tax history.

This module intentionally uses pytest-style module functions so the quality
gate names it explicitly instead of relying on unittest discovery.
"""

from __future__ import annotations

import json
import io
import sqlite3
from types import SimpleNamespace
from contextlib import redirect_stdout

from kassiber.cli.handlers import process_journals
from kassiber.cli.main import build_parser, dispatch
from kassiber.core import custody_filed_reports
from kassiber.core.custody_components import (
    activate_component,
    create_component,
    get_component,
    reconcile_active_memberships,
    update_component,
)
from kassiber.db import CUSTODY_DURABLE_EVIDENCE_MIGRATION, open_db


LEGACY_FIXTURE_VERSION = "pre-durable-anchor-v1"
NOW = "2026-01-01T00:00:00Z"
def _insert_scope(conn: sqlite3.Connection) -> None:
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
    for wallet_id in ("old", "new"):
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, 'descriptor',
                     '{"chain":"bitcoin","network":"main"}', ?)
            """,
            (wallet_id, wallet_id, NOW),
        )


def test_fresh_database_does_not_emit_a_fake_schema_migration_audit(tmp_path):
    conn = open_db(tmp_path)
    try:
        assert conn.execute(
            "SELECT COUNT(*) FROM schema_migration_audits"
        ).fetchone()[0] == 0
    finally:
        conn.close()


def _insert_tx(
    conn: sqlite3.Connection,
    tx_id: str,
    wallet_id: str,
    direction: str,
    amount_msat: int,
    occurred_at: str,
    *,
    excluded: bool = False,
) -> None:
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, direction, asset, amount, fee, fiat_currency,
            fiat_rate, fiat_rate_exact, fiat_value, fiat_value_exact, kind,
            excluded, raw_json, created_at
        ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, 0, 'EUR',
                 100000, '100000', ?, ?, ?, ?, '{}', ?)
        """,
        (
            tx_id,
            wallet_id,
            tx_id,
            f"fp-{tx_id}",
            occurred_at,
            direction,
            amount_msat,
            float(amount_msat) / 1_000_000,
            str(float(amount_msat) / 1_000_000),
            "buy" if direction == "inbound" else "sell",
            int(excluded),
            occurred_at,
        ),
    )


def _leg(role: str, amount: int, transaction_id: str, wallet_id: str) -> dict:
    return {
        "role": role,
        "rail": "bitcoin",
        "chain": "bitcoin",
        "network": "main",
        "asset": "BTC",
        "exposure": "bitcoin",
        "conservation_unit": "msat",
        "amount_msat": amount,
        "transaction_id": transaction_id,
        "wallet_id": wallet_id,
    }


def _active_component(
    conn: sqlite3.Connection,
    component_id: str,
    out_id: str,
    in_id: str,
    amount: int,
) -> dict:
    component = create_component(
        conn,
        workspace_id="ws",
        profile_id="profile",
        component_id=component_id,
        component_type="manual_bridge",
        evidence_kind="migration_fixture",
        evidence_grade="reviewed",
        legs=[
            {**_leg("source", amount, out_id, "old"), "id": f"{component_id}-s"},
            {
                **_leg("destination", amount, in_id, "new"),
                "id": f"{component_id}-d",
            },
        ],
        allocations=[
            {
                "source_leg_id": f"{component_id}-s",
                "sink_leg_id": f"{component_id}-d",
                "source_amount_msat": amount,
                "sink_amount_msat": amount,
            }
        ],
        created_at=NOW,
    )
    return activate_component(conn, component["id"], activated_at=NOW)


def _insert_replication_state(conn: sqlite3.Connection, conflicted_id: str) -> None:
    competing = update_component(
        conn,
        conflicted_id,
        new_component_id="conflicted-component-peer",
        notes="concurrent replicated revision",
        created_at="2026-01-03T00:00:00Z",
    )
    activate_component(
        conn, competing["id"], activated_at="2026-01-04T00:00:00Z"
    )
    # Reproduce the authored state that can arrive from a concurrent peer: two
    # active revisions in one lineage. Lifecycle columns are mutable by design;
    # economics remain immutable and both revisions stay visible but ineffective.
    conn.execute(
        "UPDATE custody_components SET state = 'active', activated_at = ? "
        "WHERE id = ?",
        ("2026-01-04T00:00:01Z", conflicted_id),
    )
    conn.execute(
        """
        INSERT INTO sync_members(
            id, workspace_id, profile_id, display_name, signing_public_key_b64,
            role, added_hlc, added_at, inviter_member_id, record_signature
        ) VALUES('member', 'ws', 'profile', 'Owner', 'public', 'owner',
                 '1:0:replica', ?, 'member', 'member-signature')
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO sync_devices(
            id, workspace_id, profile_id, member_id, recipient_public_key,
            label, paired_hlc, paired_at, record_signer_member_id,
            record_signature
        ) VALUES('device', 'ws', 'profile', 'member', 'age-public', 'Laptop',
                 '1:0:replica', ?, 'member', 'device-signature')
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO sync_replicas(
            id, workspace_id, profile_id, member_id, device_id, last_seq,
            last_hlc, last_event_hash, created_at
        ) VALUES('replica', 'ws', 'profile', 'member', 'device', 2,
                 '2:0:replica', 'event-hash-2', ?)
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO sync_books(
            profile_id, workspace_id, book_id, enabled, local_member_id,
            local_device_id, local_replica_id, hmac_key_b64, created_at,
            updated_at
        ) VALUES('profile', 'ws', 'book', 1, 'member', 'device', 'replica',
                 'aG1hYy1rZXk=', ?, ?)
        """,
        (NOW, NOW),
    )
    for seq, state in ((1, "superseded"), (2, "active")):
        conn.execute(
            """
            INSERT INTO sync_events(
                id, workspace_id, profile_id, replica_id, replica_seq, hlc,
                author_member_id, event_type, entity_table, entity_key,
                payload_json, context_json, previous_hash, event_hash,
                signature, created_at, applied_at
            ) VALUES(?, 'ws', 'profile', 'replica', ?, ?, 'member', 'row.upsert',
                     'custody_components', ?, ?, '{}', ?, ?, 'signature', ?, ?)
            """,
            (
                f"event-{seq}",
                seq,
                f"{seq}:0:replica",
                json.dumps([conflicted_id], separators=(",", ":")),
                json.dumps({"row": {"state": state}}, separators=(",", ":")),
                None if seq == 1 else "event-hash-1",
                f"event-hash-{seq}",
                NOW,
                NOW,
            ),
        )
    conn.execute(
        """
        INSERT INTO sync_conflicts(
            id, workspace_id, profile_id, entity_table, entity_key, field,
            local_event_id, remote_event_id, local_value_json,
            remote_value_json, status, created_at
        ) VALUES('lifecycle-conflict', 'ws', 'profile', 'custody_components', ?,
                 'state', 'event-1', 'event-2', '"superseded"', '"active"',
                 'resolved', ?)
        """,
        (json.dumps([conflicted_id], separators=(",", ":")), NOW),
    )
    reconcile_active_memberships(conn, profile_id="profile")


def _insert_filed_history(conn: sqlite3.Connection, component_id: str) -> None:
    filed = custody_filed_reports.create_filed_report_snapshot(
        conn,
        workspace_id="ws",
        profile_id="profile",
        snapshot_id="filed-2025",
        report_kind="capital-gains",
        report_state="filed",
        period_start_year=2025,
        period_end_year=2025,
        content_sha256="ab" * 32,
        classification_summary={
            "external_presumed": {"count": 1, "amount_msat": 100}
        },
        gain_summary={
            "fiat_currency": "EUR",
            "gain_loss_exact": "1.00",
            "status": "final",
        },
        created_at=NOW,
    )
    conn.execute(
        """
        INSERT INTO custody_gap_reviews(
            id, workspace_id, profile_id, gap_id, revision,
            candidate_fingerprint, action, component_id, authored_source,
            reason, snapshot_json, created_at
        ) VALUES('review', 'ws', 'profile', 'gap', 1, ?, 'resolved', ?, 'cli',
                 'migration fixture', '{}', ?)
        """,
        ("cd" * 32, component_id, NOW),
    )
    candidate = SimpleNamespace(
        started_at="2025-01-01T00:00:00Z",
        ended_at="2025-12-31T00:00:00Z",
        retained_msat=100,
        residual_msat=0,
        source_fee_msat=0,
    )
    impacts = custody_filed_reports.append_custody_impacts(
        conn,
        workspace_id="ws",
        profile_id="profile",
        component_id=component_id,
        review_id="review",
        gap_id="gap",
        candidate=candidate,
        created_at=NOW,
    )
    assert impacts[0]["filed_report_snapshot_id"] == filed["id"]


def _build_current_fixture(root) -> sqlite3.Connection:
    conn = open_db(root)
    _insert_scope(conn)
    transactions = (
        ("basis", "old", "inbound", 1_000, "2024-01-01T00:00:00Z", False),
        ("active-out", "old", "outbound", 100, "2025-01-01T00:00:00Z", False),
        ("active-in", "new", "inbound", 100, "2025-01-02T00:00:00Z", False),
        ("pair-out", "old", "outbound", 100, "2025-02-01T00:00:00Z", False),
        ("pair-in", "new", "inbound", 100, "2025-02-02T00:00:00Z", False),
        ("payout-out", "old", "outbound", 100, "2025-03-01T00:00:00Z", False),
        ("sale", "new", "outbound", 100, "2025-04-01T00:00:00Z", False),
        ("excluded", "old", "outbound", 50, "2025-05-01T00:00:00Z", True),
        ("conflict-out", "old", "outbound", 100, "2026-01-01T00:00:00Z", False),
        ("conflict-in", "new", "inbound", 100, "2026-01-02T00:00:00Z", False),
    )
    for tx_id, wallet, direction, amount, occurred_at, excluded in transactions:
        _insert_tx(
            conn,
            tx_id,
            wallet,
            direction,
            amount,
            occurred_at,
            excluded=excluded,
        )
    active = _active_component(conn, "active-component", "active-out", "active-in", 100)
    conflicted = _active_component(
        conn, "conflicted-component", "conflict-out", "conflict-in", 100
    )
    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, pair_source, created_at
        ) VALUES('manual-pair', 'ws', 'profile', 'pair-out', 'pair-in',
                 'manual', 'carrying-value', 'migration fixture', 'manual', ?)
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO direct_swap_payouts(
            id, workspace_id, profile_id, out_transaction_id, kind, policy,
            payout_asset, payout_amount, payout_occurred_at, notes, created_at
        ) VALUES('direct-payout', 'ws', 'profile', 'payout-out',
                 'direct-swap-payout', 'carrying-value', 'BTC', 100,
                 '2025-03-02T00:00:00Z', 'migration fixture', ?)
        """,
        (NOW,),
    )
    _insert_replication_state(conn, conflicted["id"])
    _insert_filed_history(conn, active["id"])
    conn.execute(
        "INSERT INTO settings(key, value) VALUES('test_fixture_schema_version', ?)",
        (LEGACY_FIXTURE_VERSION,),
    )
    conn.commit()
    assert get_component(conn, conflicted["id"])["effective_state"] == "draft"
    return conn


def _tax_snapshot(conn: sqlite3.Connection) -> dict:
    result = process_journals(conn, "ws", "profile")
    entries = [
        tuple(row)
        for row in conn.execute(
            """
            SELECT transaction_id, entry_type, asset, quantity,
                   fiat_value_exact, cost_basis_exact, proceeds_exact,
                   gain_loss_exact
            FROM journal_entries
            ORDER BY transaction_id, entry_type, id
            """
        )
    ]
    tax_summary = [
        tuple(row)
        for row in conn.execute(
            """
            SELECT year, asset, transaction_type, capital_gains_type,
                   quantity, proceeds, cost_basis, gain_loss
            FROM journal_tax_summary
            ORDER BY year, asset, transaction_type, capital_gains_type
            """
        )
    ]
    quarantines = [
        tuple(row)
        for row in conn.execute(
            "SELECT transaction_id, reason, detail_json FROM journal_quarantines "
            "ORDER BY transaction_id"
        )
    ]
    return {
        "entries": entries,
        "tax_summary": tax_summary,
        "quarantines": quarantines,
        "custody_blocked": bool(result["custody_quantity"]["blocked"]),
    }


def _downgrade_to_pre_durable_anchor(conn: sqlite3.Connection) -> None:
    db_path = conn.execute("PRAGMA database_list").fetchone()["file"]
    conn.commit()
    conn.close()
    legacy = sqlite3.connect(db_path)
    try:
        legacy.execute("PRAGMA foreign_keys = OFF")
        trigger_names = [
            row[0]
            for row in legacy.execute(
                "SELECT name FROM sqlite_master WHERE type = 'trigger' "
                "AND name LIKE 'trg_custody_%'"
            )
        ]
        for trigger_name in trigger_names:
            legacy.execute(f'DROP TRIGGER "{trigger_name}"')
        legacy.execute("DROP TABLE custody_component_evidence_commitments")
        legacy.execute(
            "ALTER TABLE custody_components DROP COLUMN expected_evidence_count"
        )
        legacy.execute(
            "ALTER TABLE custody_component_legs DROP COLUMN anchor_transaction_id"
        )
        legacy.commit()
    finally:
        legacy.close()


def test_pre_durable_anchor_migration_preserves_authored_and_tax_history(tmp_path):
    conn = _build_current_fixture(tmp_path)
    before_tax = _tax_snapshot(conn)
    before_rows = {
        table: int(
            conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
        )
        for table in (
            "custody_components",
            "transaction_pairs",
            "direct_swap_payouts",
            "custody_gap_reviews",
            "filed_report_snapshots",
            "custody_filed_report_impacts",
            "sync_books",
            "sync_members",
            "sync_devices",
            "sync_replicas",
            "sync_events",
            "sync_conflicts",
        )
    }
    _downgrade_to_pre_durable_anchor(conn)

    migrated = open_db(tmp_path)
    try:
        after_tax = _tax_snapshot(migrated)
        after_rows = {
            table: int(
                migrated.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
            )
            for table in before_rows
        }
        actual_impacts = {
            "durable_transaction_anchors": int(
                migrated.execute(
                    "SELECT COUNT(*) FROM custody_component_legs "
                    "WHERE transaction_id IS NOT NULL "
                    "AND anchor_transaction_id = transaction_id"
                ).fetchone()[0]
            ),
            "payload_free_evidence_commitments": int(
                migrated.execute(
                    "SELECT COUNT(*) FROM custody_component_evidence_commitments"
                ).fetchone()[0]
            ),
        }
        migration_row = migrated.execute(
            """
            SELECT schema_version, impact_json
            FROM schema_migration_audits
            WHERE migration_name = ?
            """,
            (CUSTODY_DURABLE_EVIDENCE_MIGRATION,),
        ).fetchone()
        migration_report = json.loads(migration_row["impact_json"])
        reported_changes = {
            item["name"]: item for item in migration_report["changes"]
        }

        assert before_tax == after_tax
        assert before_rows == after_rows
        assert actual_impacts == {
            "durable_transaction_anchors": 6,
            "payload_free_evidence_commitments": 6,
        }
        assert migration_row["schema_version"] == 1
        assert migration_report["migration"] == CUSTODY_DURABLE_EVIDENCE_MIGRATION
        assert set(actual_impacts) == set(reported_changes)
        assert reported_changes["durable_transaction_anchors"]["before"] == {
            "column_present": False,
            "anchored_leg_count": 0,
        }
        assert reported_changes["durable_transaction_anchors"]["after"] == {
            "column_present": True,
            "anchored_leg_count": 6,
        }
        assert reported_changes["payload_free_evidence_commitments"]["before"] == {
            "header_column_present": False,
            "commitment_count": 0,
        }
        assert reported_changes["payload_free_evidence_commitments"]["after"] == {
            "header_column_present": True,
            "commitment_count": 6,
        }
        assert all(
            1 <= len(change["explanation"]) <= 240
            for change in reported_changes.values()
        )
        assert migrated.execute(
            "SELECT excluded FROM transactions WHERE id = 'excluded'"
        ).fetchone()[0] == 1
        assert get_component(migrated, "active-component")["effective_state"] == "active"
        conflicted = get_component(migrated, "conflicted-component")
        assert conflicted["state"] == "active"
        assert conflicted["effective_state"] == "draft"
        assert "active_lineage_conflict" in {
            issue["code"] for issue in conflicted["validation"]["issues"]
        }
        impact = custody_filed_reports.list_custody_impacts(migrated, "profile")[0]
        assert impact["after_gain_summary"] == {
            "status": "pending_journal_rebuild"
        }
        assert migrated.execute(
            "SELECT value FROM settings WHERE key = 'test_fixture_schema_version'"
        ).fetchone()[0] == LEGACY_FIXTURE_VERSION
    finally:
        migrated.close()


def test_component_supersession_is_monotonic_across_bounded_revision_chain(tmp_path):
    conn = open_db(tmp_path)
    try:
        _insert_scope(conn)
        _insert_tx(
            conn, "out", "old", "outbound", 100, "2025-01-01T00:00:00Z"
        )
        _insert_tx(
            conn, "in", "new", "inbound", 100, "2025-01-02T00:00:00Z"
        )
        current = _active_component(conn, "revision-1", "out", "in", 100)
        chain = [current]

        for revision in range(2, 8):
            draft = update_component(
                conn,
                current["id"],
                new_component_id=f"revision-{revision}",
                notes=f"bounded revision {revision}",
                created_at=f"2026-01-{revision:02d}T00:00:00Z",
            )
            current = activate_component(
                conn,
                draft["id"],
                activated_at=f"2026-02-{revision:02d}T00:00:00Z",
            )
            chain.append(current)

        materialized = [get_component(conn, item["id"]) for item in chain]
        assert [item["revision"] for item in materialized] == list(range(1, 8))
        assert [item["state"] for item in materialized] == [
            "superseded",
            "superseded",
            "superseded",
            "superseded",
            "superseded",
            "superseded",
            "active",
        ]
        for older, newer in zip(materialized, materialized[1:]):
            assert older["superseded_by_component_id"] == newer["id"]
            assert newer["supersedes_component_id"] == older["id"]
        assert materialized[-1]["effective_state"] == "active"
    finally:
        conn.close()


def test_filed_snapshot_cli_create_and_list_is_reachable(tmp_path):
    conn = open_db(tmp_path)
    try:
        _insert_scope(conn)
        commands = (
            (
                [
                    "create",
                    "--report-kind",
                    "capital-gains",
                    "--state",
                    "filed",
                    "--period-start-year",
                    "2025",
                    "--content-sha256",
                    "ab" * 32,
                    "--classification-summary-json",
                    '{"external_presumed":{"count":1,"amount_msat":100}}',
                    "--gain-summary-json",
                    '{"fiat_currency":"EUR","gain_loss_exact":"1.00","status":"final"}',
                ],
                "reports.filed-snapshots.create",
            ),
            (["list"], "reports.filed-snapshots.list"),
        )
        for command, expected_kind in commands:
            args = build_parser().parse_args(
                [
                    "--data-root",
                    str(tmp_path),
                    "--machine",
                    "reports",
                    "filed-snapshots",
                    *command,
                    "--workspace",
                    "Books",
                    "--profile",
                    "Book",
                ]
            )
            output = io.StringIO()
            args.format = "json"
            args.non_interactive = True
            with redirect_stdout(output):
                dispatch(conn, args)
            payload = json.loads(output.getvalue())
            assert payload["kind"] == expected_kind
        assert payload["data"]["snapshots"][0]["content_sha256"] == "ab" * 32
    finally:
        conn.close()
