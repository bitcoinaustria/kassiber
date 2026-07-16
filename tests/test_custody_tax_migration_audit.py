from __future__ import annotations

from decimal import Decimal
import json
import sqlite3
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import process_journals
from kassiber.core import audit_package
from kassiber.core.custody_tax_migration import (
    CUSTODY_TAX_BEHAVIOR_MIGRATION,
    MAX_DETAILED_CHANGES,
    finalize_first_rebuild,
    list_redacted_reports,
)
from kassiber.core.sync_replication.schema_allowlist import (
    NEVER_SYNC_TABLES,
    SYNC_TABLE_MAP,
)
from kassiber.db import open_db


NOW = "2026-07-01T00:00:00Z"
BTC = 100_000_000_000
LOCAL_MIGRATION_TABLES = {
    "custody_tax_migration_baselines",
    "custody_tax_migration_baseline_events",
    "custody_tax_migration_reports",
}


def _scope(conn) -> None:
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
    conn.execute(
        """
        INSERT INTO wallets(
            id, workspace_id, profile_id, label, kind, config_json, created_at
        ) VALUES('wallet', 'ws', 'profile', 'Vault', 'descriptor', ?, ?)
        """,
        (
            json.dumps(
                {
                    "descriptor": "wpkh(MUST_NOT_LEAVE_LOCAL_DB)",
                    "chain": "bitcoin",
                    "network": "main",
                }
            ),
            NOW,
        ),
    )


def _transaction(conn, tx_id: str, direction: str, occurred_at: str) -> None:
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, direction, asset, amount, fee, fiat_currency,
            fiat_rate, fiat_value, kind, raw_json, created_at
        ) VALUES(?, 'ws', 'profile', 'wallet', ?, ?, ?, ?, 'BTC', ?, 0,
                 'EUR', 50000, 50000, ?, ?, ?)
        """,
        (
            tx_id,
            f"external-{tx_id}",
            f"fingerprint-{tx_id}",
            occurred_at,
            direction,
            BTC,
            "sell" if direction == "outbound" else "buy",
            '{"secret":"MUST_NOT_LEAVE_LOCAL_DB"}',
            NOW,
        ),
    )


def _legacy_journal(
    conn,
    tx_id: str,
    entry_type: str,
    occurred_at: str,
    *,
    entry_id: str,
) -> None:
    conn.execute(
        """
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, transaction_id, wallet_id,
            occurred_at, entry_type, asset, quantity, created_at
        ) VALUES(?, 'ws', 'profile', ?, 'wallet', ?, ?, 'BTC', ?, ?)
        """,
        (
            entry_id,
            tx_id,
            occurred_at,
            entry_type,
            -BTC if entry_type == "disposal" else BTC,
            NOW,
        ),
    )


def _entry(
    tx_id: str,
    entry_type: str,
    occurred_at: str,
    entry_id: str,
    *,
    quantity_btc: Decimal = Decimal("1"),
) -> dict:
    incoming = entry_type in {"acquisition", "income", "transfer_in"}
    return {
        "id": entry_id,
        "workspace_id": "ws",
        "profile_id": "profile",
        "transaction_id": tx_id,
        "wallet_id": "wallet",
        "account_id": None,
        "occurred_at": occurred_at,
        "entry_type": entry_type,
        "asset": "BTC",
        "quantity": quantity_btc if incoming else -quantity_btc,
        "fiat_value": Decimal("0"),
        "unit_cost": Decimal("0"),
        "cost_basis": None,
        "proceeds": None,
        "gain_loss": None,
        "description": "bounded migration fixture",
    }


def _state(entries: list[dict], *, custody_quantity=None) -> dict:
    return {
        "entries": entries,
        "quarantines": [],
        "tax_summary": [],
        "account_holdings": {},
        "wallet_holdings": {},
        "ownership_review_counts": {},
        "custody_quantity": custody_quantity,
        "intra_audit": [],
        "cross_asset_pairs": [],
        "quantity_differences": [],
    }


def _run(conn, state, *, replace_quantity=None):
    patches = [
        patch("kassiber.core.custody_journal.build_ledger_state", return_value=state),
        patch(
            "kassiber.cli.handlers.auto_price_transactions_from_rates_cache",
            return_value=0,
        ),
    ]
    if replace_quantity is not None:
        patches.append(
            patch(
                "kassiber.core.custody_quantity_store."
                "replace_canonical_quantity_state",
                side_effect=replace_quantity,
            )
        )
    with patches[0], patches[1]:
        if len(patches) == 3:
            with patches[2]:
                return process_journals(conn, "ws", "profile")
        return process_journals(conn, "ws", "profile")


def test_unchanged_first_rebuild_records_zero_behavioral_changes(tmp_path):
    data_root = tmp_path / "data"
    conn = open_db(data_root)
    _scope(conn)
    _transaction(conn, "out", "outbound", "2021-01-02T00:00:00Z")
    _legacy_journal(
        conn,
        "out",
        "disposal",
        "2021-01-02T00:00:00Z",
        entry_id="legacy-random-id",
    )
    conn.execute(
        """
        INSERT INTO journal_tax_summary(
            id, workspace_id, profile_id, year, asset, transaction_type,
            quantity, created_at
        ) VALUES('legacy-tax', 'ws', 'profile', 2021, 'BTC', 'disposal', ?, ?)
        """,
        (BTC, NOW),
    )
    conn.commit()
    conn.close()

    # Opening/migrating the schema is intentionally lightweight. It must not
    # inspect or classify the legacy tax rows.
    conn = open_db(data_root)
    try:
        assert conn.execute(
            "SELECT COUNT(*) FROM custody_tax_migration_baselines"
        ).fetchone()[0] == 0
        _run(
            conn,
            _state(
                [
                    _entry(
                        "out",
                        "disposal",
                        "2021-01-02T00:00:00Z",
                        "new-random-id",
                    )
                ]
            ),
        )

        report = json.loads(
            conn.execute(
                "SELECT comparison_json FROM custody_tax_migration_reports"
            ).fetchone()[0]
        )
        assert report["migration"] == CUSTODY_TAX_BEHAVIOR_MIGRATION
        assert report["status"] == "complete"
        assert report["summary"] == {
            "baseline_event_count": 1,
            "baseline_journal_entry_count": 1,
            "baseline_tax_summary_count": 1,
            "changed_event_count": 0,
            "changes_truncated": False,
            "detailed_change_count": 0,
            "rebuilt_event_count": 1,
        }
        assert report["changes"] == []

        _run(
            conn,
            _state(
                [
                    _entry(
                        "out",
                        "disposal",
                        "2021-01-02T00:00:00Z",
                        "another-random-id",
                    )
                ]
            ),
        )
        assert conn.execute(
            "SELECT COUNT(*) FROM custody_tax_migration_reports"
        ).fetchone()[0] == 1
        with pytest.raises(
            sqlite3.IntegrityError,
            match="custody_tax_migration_reports_immutable",
        ):
            conn.execute(
                "UPDATE custody_tax_migration_reports SET status = status"
            )
    finally:
        conn.close()


def test_behavioral_baseline_and_report_are_explicitly_local_only():
    assert LOCAL_MIGRATION_TABLES <= NEVER_SYNC_TABLES
    assert LOCAL_MIGRATION_TABLES.isdisjoint(SYNC_TABLE_MAP)


def test_legacy_external_presumptions_are_listed_as_retained_and_suspense(tmp_path):
    data_root = tmp_path / "data"
    conn = open_db(data_root)
    try:
        _scope(conn)
        _transaction(conn, "out", "outbound", "2021-01-02T00:00:00Z")
        _transaction(conn, "return", "inbound", "2021-07-02T00:00:00Z")
        _legacy_journal(
            conn,
            "out",
            "disposal",
            "2021-01-02T00:00:00Z",
            entry_id="legacy-out",
        )
        _legacy_journal(
            conn,
            "return",
            "acquisition",
            "2021-07-02T00:00:00Z",
            entry_id="legacy-return",
        )
        conn.commit()

        custody_quantity = SimpleNamespace(
            report_blocked=True,
            tax_eligibility=SimpleNamespace(blocked_from="2021-01-02T00:00:00Z"),
        )

        def replace_quantity(inner_conn, **_kwargs):
            inner_conn.execute(
                "DELETE FROM journal_quantity_issues WHERE profile_id = 'profile'"
            )
            inner_conn.execute(
                """
                INSERT INTO journal_quantity_issues(
                    issue_id, workspace_id, profile_id, issue_type, state,
                    asset, amount_msat, occurred_at, transaction_ids_json,
                    reason, created_at
                ) VALUES('suspense', 'ws', 'profile', 'unresolved_quantity',
                         'custody_suspense', 'BTC', ?,
                         '2021-01-02T00:00:00Z', '["out"]',
                         'missing_wallet', ?)
                """,
                (BTC // 100, NOW),
            )
            return {"postings": 0, "issues": 1, "balances": 0}

        _run(
            conn,
            _state(
                [
                    _entry(
                        "out",
                        "transfer_out",
                        "2021-01-02T00:00:00Z",
                        "new-out",
                    ),
                    _entry(
                        "return",
                        "transfer_in",
                        "2021-07-02T00:00:00Z",
                        "new-return",
                    ),
                ],
                custody_quantity=custody_quantity,
            ),
            replace_quantity=replace_quantity,
        )

        baseline_event_ids = {
            row["transaction_id"]: row["event_id"]
            for row in conn.execute(
                """
                SELECT transaction_id, event_id
                FROM custody_tax_migration_baseline_events
                """
            ).fetchall()
        }
        redacted = list_redacted_reports(conn, "profile")[0]
        changes = {item["transaction_id"]: item for item in redacted["changes"]}
        assert redacted["summary"]["changed_event_count"] == 2
        assert changes["out"] == {
            "event_id": baseline_event_ids["out"],
            "transaction_id": "out",
            "asset": "BTC",
            "old_classification": "disposal",
            "new_classification": "retained+suspense",
            "old_amounts_msat": {"disposal": str(BTC)},
            "new_amounts_msat": {
                "retained": str(BTC),
                "suspense": str(BTC // 100),
            },
            "old_monetary_exact": {},
            "new_monetary_exact": {
                "retained": {"fiat_value_exact": "0"},
            },
            "reason": "legacy_external_presumption_reclassified_by_custody",
            "affected_years": [2021],
        }
        assert changes["return"]["old_classification"] == "acquisition"
        assert changes["return"]["new_classification"] == "retained"
        assert changes["return"]["reason"] == (
            "legacy_external_presumption_reclassified_by_custody"
        )
        assert list_redacted_reports(
            conn,
            "profile",
            transaction_ids=["return"],
        )[0]["changes"] == [changes["return"]]

        def resolve_scope(inner_conn, _workspace, _profile):
            return (
                inner_conn.execute("SELECT * FROM workspaces WHERE id = 'ws'").fetchone(),
                inner_conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone(),
            )

        def resolve_transaction(inner_conn, profile_id, ref, direction=None):
            del direction
            return inner_conn.execute(
                "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
                (profile_id, ref),
            ).fetchone()

        summary = audit_package.build_evidence_summary(
            conn,
            str(data_root),
            None,
            None,
            audit_package.AuditPackageHooks(
                resolve_scope=resolve_scope,
                resolve_transaction=resolve_transaction,
                now_iso=lambda: NOW,
            ),
        )
        assert summary["summary"]["custody_tax_migration_audit_count"] == 1
        assert summary["custody_tax_migration_audits"][0] == redacted
        serialized = json.dumps(summary)
        assert "MUST_NOT_LEAVE_LOCAL_DB" not in serialized
        assert "wpkh(" not in serialized
        assert "raw_json" not in serialized
    finally:
        conn.close()


def test_same_classification_with_different_exact_quantity_is_reported(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _transaction(conn, "out", "outbound", "2021-01-02T00:00:00Z")
        _legacy_journal(
            conn,
            "out",
            "disposal",
            "2021-01-02T00:00:00Z",
            entry_id="legacy-out",
        )
        conn.commit()

        _run(
            conn,
            _state(
                [
                    _entry(
                        "out",
                        "disposal",
                        "2021-01-02T00:00:00Z",
                        "new-out",
                        quantity_btc=Decimal("0.1"),
                    )
                ]
            ),
        )

        change = list_redacted_reports(conn, "profile")[0]["changes"][0]
        assert change["old_classification"] == "disposal"
        assert change["new_classification"] == "disposal"
        assert change["old_amounts_msat"] == {"disposal": str(BTC)}
        assert change["new_amounts_msat"] == {"disposal": str(BTC // 10)}
        assert change["reason"] == "classification_quantity_changed"
        assert change["affected_years"] == [2021]
    finally:
        conn.close()


def test_exact_monetary_change_is_reported_when_both_sides_provide_it(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _transaction(conn, "out", "outbound", "2021-01-02T00:00:00Z")
        _legacy_journal(
            conn,
            "out",
            "disposal",
            "2021-01-02T00:00:00Z",
            entry_id="legacy-out",
        )
        conn.execute(
            """
            UPDATE journal_entries
            SET fiat_value_exact = '100', cost_basis_exact = '80',
                proceeds_exact = '100', gain_loss_exact = '20'
            WHERE id = 'legacy-out'
            """
        )
        conn.commit()

        rebuilt = _entry(
            "out",
            "disposal",
            "2021-01-02T00:00:00Z",
            "new-out",
        )
        rebuilt.update(
            {
                "fiat_value": Decimal("90"),
                "cost_basis": Decimal("80"),
                "proceeds": Decimal("90"),
                "gain_loss": Decimal("10"),
            }
        )
        _run(conn, _state([rebuilt]))

        change = list_redacted_reports(conn, "profile")[0]["changes"][0]
        assert change["old_classification"] == "disposal"
        assert change["new_classification"] == "disposal"
        assert change["old_amounts_msat"] == change["new_amounts_msat"]
        assert change["old_monetary_exact"]["disposal"]["proceeds_exact"] == "100"
        assert change["new_monetary_exact"]["disposal"]["proceeds_exact"] == "90"
        assert change["reason"] == "classification_monetary_changed"
    finally:
        conn.close()


def test_failed_rebuild_rolls_back_baseline_and_preserves_legacy_rows(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _transaction(conn, "out", "outbound", "2021-01-02T00:00:00Z")
        _legacy_journal(
            conn,
            "out",
            "disposal",
            "2021-01-02T00:00:00Z",
            entry_id="legacy-out",
        )
        conn.commit()

        with (
            patch(
                "kassiber.cli.handlers.auto_price_transactions_from_rates_cache",
                return_value=0,
            ),
            patch(
                "kassiber.core.custody_journal.build_ledger_state",
                side_effect=RuntimeError("new engine failed"),
            ),
            pytest.raises(RuntimeError, match="new engine failed"),
        ):
            process_journals(conn, "ws", "profile")

        assert conn.execute(
            "SELECT COUNT(*) FROM custody_tax_migration_baselines"
        ).fetchone()[0] == 0
        assert conn.execute(
            "SELECT COUNT(*) FROM custody_tax_migration_reports"
        ).fetchone()[0] == 0
        assert conn.execute(
            "SELECT entry_type FROM journal_entries WHERE id = 'legacy-out'"
        ).fetchone()[0] == "disposal"
    finally:
        conn.close()


def test_comparison_detail_is_bounded_without_losing_total_change_count(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        total = MAX_DETAILED_CHANGES + 7
        conn.execute(
            """
            INSERT INTO custody_tax_migration_baselines(
                profile_id, workspace_id, migration_name, schema_version,
                journal_entry_count, tax_summary_count, event_count, captured_at
            ) VALUES('profile', 'ws', ?, 1, ?, 0, ?, ?)
            """,
            (CUSTODY_TAX_BEHAVIOR_MIGRATION, total, total, NOW),
        )
        conn.executemany(
            """
            INSERT INTO custody_tax_migration_baseline_events(
                profile_id, event_id, transaction_id, asset, classification,
                affected_years_json
            ) VALUES('profile', ?, ?, 'BTC', 'disposal', '[2021]')
            """,
            [
                (f"ctm:{index:064x}", f"legacy-{index:04d}")
                for index in range(total)
            ],
        )

        report = finalize_first_rebuild(
            conn,
            workspace_id="ws",
            profile_id="profile",
            rebuilt_at=NOW,
        )
        assert report is not None
        assert report["summary"]["changed_event_count"] == total
        assert report["summary"]["detailed_change_count"] == MAX_DETAILED_CHANGES
        assert report["summary"]["changes_truncated"] is True
        assert len(report["changes"]) == MAX_DETAILED_CHANGES
        assert len(list_redacted_reports(conn, "profile")[0]["changes"]) == (
            MAX_DETAILED_CHANGES
        )
    finally:
        conn.close()


def test_incomplete_baseline_fails_closed_without_appending_report(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        conn.execute(
            """
            INSERT INTO custody_tax_migration_baselines(
                profile_id, workspace_id, migration_name, schema_version,
                journal_entry_count, tax_summary_count, event_count, captured_at
            ) VALUES('profile', 'ws', ?, 1, 1, 0, 1, ?)
            """,
            (CUSTODY_TAX_BEHAVIOR_MIGRATION, NOW),
        )

        with pytest.raises(
            sqlite3.IntegrityError,
            match="custody_tax_migration_baseline_incomplete",
        ):
            finalize_first_rebuild(
                conn,
                workspace_id="ws",
                profile_id="profile",
                rebuilt_at=NOW,
            )
        assert conn.execute(
            "SELECT COUNT(*) FROM custody_tax_migration_reports"
        ).fetchone()[0] == 0
    finally:
        conn.close()
