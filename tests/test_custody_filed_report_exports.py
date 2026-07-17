from __future__ import annotations

import hashlib
import sqlite3
from unittest.mock import patch

import pytest

from kassiber.cli import handlers
from kassiber.core import custody_filed_reports, reports
from kassiber.db import open_db
from kassiber.errors import AppError


def _scope(conn: sqlite3.Connection) -> None:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', 'now')"
    )
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country, created_at
        ) VALUES('profile', 'ws', 'Book', 'EUR', 'generic', 'now')
        """
    )
    conn.execute(
        """
        INSERT INTO wallets(
            id, workspace_id, profile_id, label, kind, config_json, created_at
        ) VALUES('wallet', 'ws', 'profile', 'Vault', 'manual', '{}', 'now')
        """
    )
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, direction, asset, amount, fee, raw_json, created_at
        ) VALUES('sale', 'ws', 'profile', 'wallet', 'sale', 'sale-fp',
                 '2025-06-01T00:00:00Z', 'outbound', 'BTC', 100000, 0, '{}', 'now')
        """
    )
    conn.execute(
        """
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, transaction_id, wallet_id,
            occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
            cost_basis, proceeds, gain_loss, cost_basis_exact, proceeds_exact,
            gain_loss_exact, created_at
        ) VALUES('entry', 'ws', 'profile', 'sale', 'wallet',
                 '2025-06-01T00:00:00Z', 'disposal', 'BTC', 100000,
                 120, 0, 100, 120, 20, '100.00', '120.00', '20.00', 'now')
        """
    )
    conn.commit()


def test_bundle_hash_rejects_duplicate_basenames_and_is_order_independent(
    tmp_path,
):
    first_dir = tmp_path / "first"
    second_dir = tmp_path / "second"
    first_dir.mkdir()
    second_dir.mkdir()
    first = first_dir / "report.csv"
    second = second_dir / "report.csv"
    first.write_bytes(b"first")
    second.write_bytes(b"second")

    for paths in ((first, second), (second, first)):
        with pytest.raises(AppError) as raised:
            custody_filed_reports.artifact_content_sha256(paths)
        assert raised.value.code == "filed_report_snapshot_validation"
        assert raised.value.details == {"field": "paths"}

    uniquely_named = second_dir / "details.csv"
    uniquely_named.write_bytes(second.read_bytes())
    assert custody_filed_reports.artifact_content_sha256(
        (first, uniquely_named)
    ) == custody_filed_reports.artifact_content_sha256((uniquely_named, first))


def test_completed_full_report_export_registers_computed_saved_snapshot(tmp_path):
    conn = open_db(tmp_path / "book")
    try:
        _scope(conn)
        context = {
            "workspace": {"id": "ws", "label": "Books"},
            "profile": {
                "id": "profile",
                "workspace_id": "ws",
                "label": "Book",
            },
            "wallet": {"id": "wallet", "label": "Vault"},
            "title": "Kassiber Report - Book",
        }
        sections = [
            {
                "sheet_name": "Overview",
                "title": "Overview",
                "headers": ("field", "value"),
                "rows": ({"field": "gain", "value": "20.00"},),
            }
        ]
        artifact = tmp_path / "report.csv"
        with (
            patch("kassiber.core.reports._build_full_report_context", return_value=context),
            patch("kassiber.core.reports._generic_report_section_specs", return_value=sections),
            patch(
                "kassiber.core.reports._generic_report_csv_rows",
                return_value=(("field", "value"), ("gain", "20.00")),
            ),
        ):
            result = reports.export_csv_report(
                conn, "ws", "profile", artifact, object()
            )

        snapshot = result["report_snapshot"]
        assert snapshot["report_state"] == "saved"
        assert snapshot["report_kind"] == "full-report.csv"
        assert snapshot["period_start_year"] == 2025
        assert snapshot["period_end_year"] == 2025
        assert snapshot["content_sha256"] == hashlib.sha256(
            artifact.read_bytes()
        ).hexdigest()
        assert snapshot["classification_summary"] == {
            "disposal": {"count": 1, "amount_msat": 100000}
        }
        assert snapshot["gain_summary"] == {
            "fiat_currency": "EUR",
            "proceeds_exact": "120.00",
            "cost_basis_exact": "100.00",
            "gain_loss_exact": "20.00",
            "status": "final",
        }
        assert snapshot["report_scope"] == {"wallet_ids": ["wallet"]}

        with (
            patch("kassiber.core.reports._build_full_report_context", return_value=context),
            patch("kassiber.core.reports._generic_report_section_specs", return_value=sections),
            patch(
                "kassiber.core.reports._generic_report_csv_rows",
                return_value=(("field", "value"), ("gain", "20.00")),
            ),
        ):
            repeated = reports.export_csv_report(
                conn, "ws", "profile", artifact, object()
            )
        assert repeated["report_snapshot"]["id"] != snapshot["id"]
        assert repeated["report_snapshot"]["content_sha256"] == snapshot["content_sha256"]
        assert conn.execute("SELECT COUNT(*) FROM filed_report_snapshots").fetchone()[0] == 2
    finally:
        conn.close()


def test_standalone_transaction_exports_are_evidence_not_saved_reports(tmp_path):
    conn = open_db(tmp_path / "book")
    try:
        _scope(conn)
        context = {
            "workspace": {"id": "ws", "label": "Books"},
            "profile": {
                "id": "profile",
                "workspace_id": "ws",
                "label": "Book",
            },
            "wallet": None,
            "title": "Kassiber Transactions - Book",
            "spec": {
                "sheet_name": "Transactions",
                "title": "Transactions",
                "headers": reports.TRANSACTIONS_EXPORT_HEADERS,
                "rows": (),
            },
        }
        csv_path = tmp_path / "transactions.csv"
        xlsx_path = tmp_path / "transactions.xlsx"

        with patch(
            "kassiber.core.reports._transactions_export_context",
            return_value=context,
        ):
            csv_result = reports.export_transactions_csv_report(
                conn, "ws", "profile", csv_path, object()
            )
            xlsx_result = reports.export_transactions_xlsx_report(
                conn, "ws", "profile", xlsx_path, object()
            )

        assert csv_path.is_file()
        assert xlsx_path.is_file()
        assert "report_snapshot" not in csv_result
        assert "report_snapshot" not in xlsx_result
        assert (
            conn.execute("SELECT COUNT(*) FROM filed_report_snapshots").fetchone()[0]
            == 0
        )
    finally:
        conn.close()


def test_transaction_exports_fail_before_writing_when_custody_is_unresolved(
    tmp_path,
):
    conn = open_db(tmp_path / "book")
    try:
        _scope(conn)
        conn.execute(
            """
            INSERT INTO journal_quantity_issues(
                issue_id, workspace_id, profile_id, issue_type, state,
                asset, amount_msat, occurred_at, transaction_ids_json,
                reason, detail_json, blocks_from, created_at
            ) VALUES(
                'issue', 'ws', 'profile', 'custody_gap', 'custody_suspense',
                'BTC', 100000, '2025-06-01T00:00:00Z', '["sale"]',
                'missing_wallet', '{}', '2025-06-01T00:00:00Z', 'now'
            )
            """
        )
        conn.commit()

        for suffix, exporter in (
            ("csv", reports.export_transactions_csv_report),
            ("xlsx", reports.export_transactions_xlsx_report),
        ):
            artifact = tmp_path / f"transactions.{suffix}"
            with pytest.raises(AppError) as blocked:
                exporter(
                    conn,
                    "ws",
                    "profile",
                    artifact,
                    handlers._report_hooks(),
                )
            assert blocked.value.code == "custody_quantity_unresolved"
            assert not artifact.exists()
    finally:
        conn.close()


def test_pending_impact_gets_one_immutable_exact_post_rebuild_resolution(tmp_path):
    conn = open_db(tmp_path / "book")
    try:
        _scope(conn)
        snapshot = custody_filed_reports.create_filed_report_snapshot(
            conn,
            workspace_id="ws",
            profile_id="profile",
            report_kind="capital-gains",
            report_state="filed",
            period_start_year=2025,
            period_end_year=2025,
            content_sha256="ab" * 32,
            classification_summary={
                "acquisition": {"count": 1, "amount_msat": 100000}
            },
            gain_summary={
                "fiat_currency": "EUR",
                "proceeds_exact": "0",
                "cost_basis_exact": "0",
                "gain_loss_exact": "0",
                "status": "final",
            },
            report_scope={
                "wallet_ids": ["wallet"],
                "occurred_at_start": "2025-01-01T00:00:00Z",
                "occurred_at_end": "2025-12-31T23:59:59Z",
            },
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES('other-wallet', 'ws', 'profile', 'Other', 'manual', '{}', 'now')
            """
        )
        for transaction_id, wallet_id, occurred_at in (
            ("outside-wallet", "other-wallet", "2025-06-02T00:00:00Z"),
            ("outside-time", "wallet", "2026-01-01T00:00:00Z"),
        ):
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    fingerprint, occurred_at, direction, asset, amount, fee,
                    raw_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, 'outbound', 'BTC',
                         900000, 0, '{}', 'now')
                """,
                (
                    transaction_id,
                    wallet_id,
                    transaction_id,
                    f"{transaction_id}-fp",
                    occurred_at,
                ),
            )
            conn.execute(
                """
                INSERT INTO journal_entries(
                    id, workspace_id, profile_id, transaction_id, wallet_id,
                    occurred_at, entry_type, asset, quantity, fiat_value,
                    unit_cost, cost_basis, proceeds, gain_loss, cost_basis_exact,
                    proceeds_exact, gain_loss_exact, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, ?, 'disposal', 'BTC', 900000,
                         900, 0, 800, 900, 100, '800', '900', '100', 'now')
                """,
                (f"{transaction_id}-entry", transaction_id, wallet_id, occurred_at),
            )
        conn.execute(
            """
            INSERT INTO custody_filed_report_impacts(
                id, workspace_id, profile_id, filed_report_snapshot_id,
                component_id, review_id, gap_id, affected_period_start_year,
                affected_period_end_year, before_classification_summary_json,
                after_classification_summary_json, before_gain_summary_json,
                after_gain_summary_json, amendment_warning, created_at
            ) VALUES('impact', 'ws', 'profile', ?, 'component', 'review', 'gap',
                     2025, 2025, ?, '{}', ?, '{"status":"pending_journal_rebuild"}',
                     'Review amendment', 'now')
            """,
            (
                snapshot["id"],
                '{"acquisition":{"amount_msat":100000,"count":1}}',
                '{"cost_basis_exact":"0","fiat_currency":"EUR",'
                '"gain_loss_exact":"0","proceeds_exact":"0","status":"final"}',
            ),
        )
        conn.commit()

        created = custody_filed_reports.resolve_pending_custody_impacts(
            conn,
            workspace_id="ws",
            profile_id="profile",
            rebuilt_at="2026-01-02T00:00:00Z",
        )
        assert len(created) == 1
        resolution = created[0]
        assert resolution["amendment_status"] == "review_required"
        assert resolution["classification_changed"] is True
        assert resolution["gain_changed"] is True
        assert resolution["after_classification_summary"] == {
            "disposal": {"count": 1, "amount_msat": 100000}
        }
        assert resolution["after_gain_summary"]["gain_loss_exact"] == "20.00"
        assert custody_filed_reports.resolve_pending_custody_impacts(
            conn,
            workspace_id="ws",
            profile_id="profile",
            rebuilt_at="2026-01-03T00:00:00Z",
        ) == []

        impact = custody_filed_reports.list_custody_impacts(conn, "profile")[0]
        assert impact["after_gain_summary"] == {"status": "pending_journal_rebuild"}
        assert impact["resolution"] == resolution
        with conn:
            try:
                conn.execute(
                    "UPDATE custody_filed_report_impact_resolutions "
                    "SET amendment_status = 'no_change' WHERE id = ?",
                    (resolution["id"],),
                )
            except sqlite3.IntegrityError:
                pass
            else:
                raise AssertionError("impact resolution must be immutable")
    finally:
        conn.close()
