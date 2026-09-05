from __future__ import annotations

import json

from kassiber.core import accounts as core_accounts
from kassiber.core import wallets as core_wallets
from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.daemon import _create_profile_payload
from kassiber.db import WAGES_ACQUISITION_SEMANTICS_MIGRATION, open_db


NOW = "2026-01-01T00:00:00Z"


def test_wages_semantic_migration_invalidates_current_journal_once(tmp_path):
    data_root = tmp_path / "data"
    conn = open_db(data_root)
    workspace = core_accounts.create_workspace(conn, "Books")
    profile = core_accounts.create_profile(
        conn, workspace["id"], "Main", "EUR", "FIFO", "at", 0
    )
    wallet = core_wallets.create_wallet(
        conn, workspace["id"], profile["id"], "Salary", "custom"
    )
    account_id = conn.execute(
        "SELECT id FROM accounts WHERE profile_id = ? AND code = 'treasury'",
        (profile["id"],),
    ).fetchone()[0]
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, direction, asset, amount, fee, fiat_currency,
            fiat_rate, fiat_value, kind, raw_json, created_at
        ) VALUES(
            'wage', ?, ?, ?, 'wage', 'wage-fingerprint', ?, 'inbound',
            'BTC', 100000000, 0, 'EUR', 40000, 40, 'wages', '{}', ?
        )
        """,
        (workspace["id"], profile["id"], wallet["id"], NOW, NOW),
    )
    for entry_id, entry_type, quantity, kennzahl in (
        ("old-wage-acquisition", "acquisition", 100000000, None),
        ("old-wage-income", "income", 0, 172),
    ):
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                account_id, occurred_at, entry_type, asset, quantity,
                fiat_value, gain_loss, at_category, at_kennzahl, created_at
            ) VALUES(?, ?, ?, 'wage', ?, ?, ?, ?, 'BTC', ?, 40, 40,
                     'income_general', ?, ?)
            """,
            (
                entry_id,
                workspace["id"],
                profile["id"],
                wallet["id"],
                account_id,
                NOW,
                entry_type,
                quantity,
                kennzahl,
                NOW,
            ),
        )
    conn.execute(
        """
        UPDATE profiles
        SET journal_input_version = 7,
            last_processed_input_version = 7,
            last_processed_at = ?,
            last_processed_tx_count = 1,
            ownership_review_counts_json = '{"pairable":1}'
        WHERE id = ?
        """,
        (NOW, profile["id"]),
    )
    # This fixture models a database last opened by the pre-upgrade build.
    conn.execute(
        "DELETE FROM schema_migration_audits WHERE migration_name = ?",
        (WAGES_ACQUISITION_SEMANTICS_MIGRATION,),
    )
    conn.commit()
    conn.close()

    migrated = open_db(data_root)
    freshness = migrated.execute(
        """
        SELECT journal_input_version, last_processed_input_version,
               last_processed_at, last_processed_tx_count,
               ownership_review_counts_json
        FROM profiles WHERE id = ?
        """,
        (profile["id"],),
    ).fetchone()
    assert freshness["journal_input_version"] == 8
    assert freshness["last_processed_input_version"] == 7
    assert freshness["last_processed_at"] is None
    assert freshness["last_processed_tx_count"] == 0
    assert freshness["ownership_review_counts_json"] is None
    assert "journals_stale" in {
        blocker["id"] for blocker in build_report_blockers_snapshot(migrated)["blockers"]
    }
    assert migrated.execute(
        "SELECT COUNT(*) FROM journal_entries WHERE transaction_id = 'wage' "
        "AND at_kennzahl = 172"
    ).fetchone()[0] == 1
    audit = migrated.execute(
        """
        SELECT impact_json FROM schema_migration_audits
        WHERE migration_name = ?
        """,
        (WAGES_ACQUISITION_SEMANTICS_MIGRATION,),
    ).fetchone()
    impact = json.loads(audit["impact_json"])
    assert impact["migration"] == WAGES_ACQUISITION_SEMANTICS_MIGRATION
    assert impact["changes"][0]["affected_profile_count"] == 1
    assert impact["changes"][0]["affected_transaction_count"] == 1
    assert "wage-fingerprint" not in audit["impact_json"]
    migrated.close()

    repeated = open_db(data_root)
    try:
        assert repeated.execute(
            "SELECT journal_input_version FROM profiles WHERE id = ?",
            (profile["id"],),
        ).fetchone()[0] == 8
        assert repeated.execute(
            "SELECT COUNT(*) FROM schema_migration_audits WHERE migration_name = ?",
            (WAGES_ACQUISITION_SEMANTICS_MIGRATION,),
        ).fetchone()[0] == 1
    finally:
        repeated.close()


def test_fresh_migration_marker_prevents_post_upgrade_wage_invalidation(tmp_path):
    data_root = tmp_path / "data"
    conn = open_db(data_root)
    workspace = core_accounts.create_workspace(conn, "Books")
    profile = core_accounts.create_profile(
        conn, workspace["id"], "Main", "EUR", "FIFO", "generic", 365
    )
    wallet = core_wallets.create_wallet(
        conn, workspace["id"], profile["id"], "Salary", "custom"
    )
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
            direction, asset, amount, fee, kind, raw_json, created_at
        ) VALUES('new-wage', ?, ?, ?, 'new-wage-fingerprint', ?, 'inbound',
                 'BTC', 100000000, 0, ' WAGES ', '{}', ?)
        """,
        (workspace["id"], profile["id"], wallet["id"], NOW, NOW),
    )
    conn.execute(
        """
        UPDATE profiles
        SET journal_input_version = 3,
            last_processed_input_version = 3,
            last_processed_at = ?,
            last_processed_tx_count = 1
        WHERE id = ?
        """,
        (NOW, profile["id"]),
    )
    conn.commit()
    assert conn.execute(
        "SELECT COUNT(*) FROM schema_migration_audits WHERE migration_name = ?",
        (WAGES_ACQUISITION_SEMANTICS_MIGRATION,),
    ).fetchone()[0] == 1
    conn.close()

    reopened = open_db(data_root)
    try:
        row = reopened.execute(
            """
            SELECT journal_input_version, last_processed_input_version,
                   last_processed_at, last_processed_tx_count
            FROM profiles WHERE id = ?
            """,
            (profile["id"],),
        ).fetchone()
        assert row["journal_input_version"] == 3
        assert row["last_processed_input_version"] == 3
        assert row["last_processed_at"] == NOW
        assert row["last_processed_tx_count"] == 1
    finally:
        reopened.close()


def test_new_book_country_change_resets_inherited_pool_scope(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        workspace = core_accounts.create_workspace(conn, "Books")
        source = core_accounts.create_profile(
            conn, workspace["id"], "Future scope", "EUR", "FIFO", "at", 0
        )
        # Model a future source-country policy that permits wallet-local pools.
        # The target generic policy remains global-only.
        conn.execute(
            "UPDATE profiles SET cost_basis_pool_scope = 'wallet' WHERE id = ?",
            (source["id"],),
        )
        conn.commit()

        created = _create_profile_payload(
            conn,
            {
                "workspace_id": workspace["id"],
                "label": "Generic target",
                "tax_country": "generic",
                "gains_algorithm": "FIFO",
            },
        )

        assert created["defaults"]["tax_country"] == "generic"
        assert created["defaults"]["cost_basis_pool_scope"] == "global"
    finally:
        conn.close()
