from __future__ import annotations

import tempfile

from kassiber.cli.handlers import process_journals
from kassiber.core import custody_components, source_funds
from kassiber.core.source_funds import SourceFundsHooks
from kassiber.core.ui_snapshot import (
    build_journal_events_list_snapshot,
    build_journals_snapshot,
    build_journals_transfers_list_snapshot,
    build_transactions_snapshot,
)
from kassiber.db import open_db, set_setting


NOW = "2026-03-01T09:00:00Z"


def _hooks() -> SourceFundsHooks:
    def resolve_scope(conn, workspace_ref, profile_ref):
        return (
            conn.execute("SELECT * FROM workspaces WHERE id = 'ws'").fetchone(),
            conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone(),
        )

    def resolve_transaction(conn, profile_id, ref):
        return conn.execute(
            "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
            (profile_id, ref),
        ).fetchone()

    return SourceFundsHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        format_table=lambda headers, rows, widths, **kwargs: [],
    )


def _setup(conn) -> None:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', ?)",
        (NOW,),
    )
    conn.execute(
        "INSERT INTO profiles(id, workspace_id, label, created_at) "
        "VALUES('profile', 'ws', 'Treasury', ?)",
        (NOW,),
    )
    for wallet_id in ("a", "b", "c"):
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, 'address', '{}', ?)
            """,
            (wallet_id, f"Wallet {wallet_id.upper()}", NOW),
        )
    for tx_id, wallet_id, direction, amount in (
        ("out-a", "a", "outbound", 100),
        ("in-b", "b", "inbound", 40),
        ("in-c", "c", "inbound", 60),
    ):
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, created_at
            ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, 'BTC', ?, 0, 'EUR', 1, 1, ?)
            """,
            (tx_id, wallet_id, f"fp:{tx_id}", NOW, direction, amount, NOW),
        )


def _leg(role: str, amount: int, tx_id: str, wallet_id: str, leg_id: str):
    return {
        "id": leg_id,
        "role": role,
        "rail": "bitcoin",
        "chain": "bitcoin",
        "network": "regtest",
        "asset": "BTC",
        "exposure": "bitcoin",
        "conservation_unit": "msat",
        "amount_msat": amount,
        "occurred_at": NOW,
        "transaction_id": tx_id,
        "wallet_id": wallet_id,
    }


def test_effective_nm_bridge_becomes_reviewed_source_funds_lineage() -> None:
    with tempfile.TemporaryDirectory(prefix="kassiber-sof-custody-") as root:
        conn = open_db(root)
        try:
            _setup(conn)
            component = custody_components.create_component(
                conn,
                workspace_id="ws",
                profile_id="profile",
                component_type="manual_bridge",
                evidence_kind="manual_reconstruction",
                evidence_grade="reviewed",
                legs=[
                    _leg("source", 100, "out-a", "a", "source"),
                    _leg("destination", 40, "in-b", "b", "dest-b"),
                    _leg("destination", 60, "in-c", "c", "dest-c"),
                ],
                allocations=[
                    {
                        "source_leg_id": "source",
                        "sink_leg_id": "dest-b",
                        "source_amount_msat": 40,
                        "sink_amount_msat": 40,
                    },
                    {
                        "source_leg_id": "source",
                        "sink_leg_id": "dest-c",
                        "source_amount_msat": 60,
                        "sink_amount_msat": 60,
                    },
                ],
            )
            activated = custody_components.activate_component(conn, component["id"])
            assert activated["effective_state"] == "active"
            process_journals(conn, "ws", "profile")
            set_setting(conn, "context_workspace", "ws")
            set_setting(conn, "context_profile", "profile")
            conn.commit()

            transfer_snapshot = build_journals_transfers_list_snapshot(
                conn, {"limit": 10}
            )
            assert transfer_snapshot["summary"]["manual_pairs"] == 2
            assert {
                item["in"]["amount_msat"]
                for item in transfer_snapshot["pairs"]
            } == {40, 60}
            transaction_snapshot = build_transactions_snapshot(
                conn, {"limit": 10}
            )
            assert transaction_snapshot["count"] == 2
            assert {
                item["pair"]["type"] for item in transaction_snapshot["txs"]
            } == {"transfer"}

            assembled = source_funds.assemble_history(
                conn,
                None,
                None,
                _hooks(),
                target_transaction_ref="in-b",
            )
            assert assembled["methods"] == {"custody_component": 1}
            link = conn.execute(
                "SELECT * FROM source_funds_links WHERE to_transaction_id = 'in-b'"
            ).fetchone()
            assert link["state"] == "reviewed"
            assert link["allocation_policy"] == "explicit"
            assert link["allocation_amount"] == 40
            assert link["from_allocation_amount"] == 40

            custody_components.supersede_component(conn, component["id"])
            stale_transactions = build_transactions_snapshot(conn, {"limit": 10})
            assert stale_transactions["count"] == 3
            assert all("pair" not in item for item in stale_transactions["txs"])
            stale_transfers = build_journals_transfers_list_snapshot(
                conn, {"limit": 10}
            )
            assert stale_transfers["summary"]["projection_status"] == "stale"
            assert stale_transfers["pairs"] == []
            stale_events = build_journal_events_list_snapshot(conn, {"limit": 20})
            assert all(item["pair"] is None for item in stale_events["events"])
            stale_journals = build_journals_snapshot(conn)
            assert all("pair" not in item for item in stale_journals["recent"])
            report = source_funds.build_report(
                conn,
                None,
                None,
                _hooks(),
                target_transaction_ref="in-b",
            )
            blocker_codes = {
                finding["code"]
                for finding in report["findings"]
                if finding["severity"] == "blocker"
            }
            assert "stale_custody_component_lineage" in blocker_codes
        finally:
            conn.close()
