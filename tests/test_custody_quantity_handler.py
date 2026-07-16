import tempfile
import unittest

from kassiber.cli import handlers
from kassiber.core.chain_observer.provenance import (
    persist_chain_observation_provenance,
)
from kassiber.core.custody_components import activate_component, create_component
from kassiber.core import report_context as core_report_context
from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.db import open_db
from kassiber.errors import AppError


BTC = 100_000_000_000
SOURCE_AT = "2024-01-01T00:00:00Z"


def _leg(role, amount, *, transaction_id=None, wallet_id=None, occurred_at=None):
    return {
        "role": role,
        "rail": "untracked" if role == "suspense" else "bitcoin",
        "chain": None if role == "suspense" else "bitcoin",
        "network": "main",
        "asset": "BTC",
        "exposure": "bitcoin",
        "conservation_unit": "msat",
        "amount_msat": amount,
        "transaction_id": transaction_id,
        "wallet_id": wallet_id,
        **({"occurred_at": occurred_at} if occurred_at else {}),
    }


def _seed_transaction(
    conn,
    tx_id,
    wallet_id,
    direction,
    amount,
    occurred_at,
    rate,
):
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id,
            fingerprint, occurred_at, direction, asset, amount, fee,
            fiat_currency, fiat_rate, fiat_rate_exact, raw_json, created_at
        ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, 0,
                 'EUR', ?, ?, '{}', ?)
        """,
        (
            tx_id,
            wallet_id,
            tx_id,
            f"fp-{tx_id}",
            occurred_at,
            direction,
            amount,
            float(rate),
            str(rate),
            occurred_at,
        ),
    )


def _book(root, algorithm, *, tax_country="generic"):
    conn = open_db(root)
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', 'now')"
    )
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            gains_algorithm, created_at
        ) VALUES('profile', 'ws', 'Book', 'EUR', ?, ?, 'now')
        """,
        (tax_country, algorithm),
    )
    for wallet_id in ("a", "c"):
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, 'descriptor',
                     '{"chain":"bitcoin","network":"main"}', 'now')
            """,
            (wallet_id, wallet_id.upper()),
        )
    _seed_transaction(
        conn, "buy-old", "a", "inbound", 6 * BTC,
        "2022-01-01T00:00:00Z", 10_000,
    )
    _seed_transaction(
        conn, "buy-new", "a", "inbound", 4 * BTC,
        "2023-01-01T00:00:00Z", 20_000,
    )
    _seed_transaction(conn, "out", "a", "outbound", 10 * BTC, SOURCE_AT, 30_000)
    _seed_transaction(
        conn, "in", "c", "inbound", 990_000_000_000,
        "2025-01-01T00:00:00Z", 40_000,
    )
    _seed_transaction(
        conn, "later-sale", "c", "outbound", BTC,
        "2026-01-01T00:00:00Z", 50_000,
    )
    return conn


def _prepare_same_txid_roll(conn):
    conn.execute("DELETE FROM transactions WHERE id = 'later-sale'")
    conn.execute("UPDATE transactions SET amount = ? WHERE id = 'in'", (10 * BTC,))
    native_txid = "ab" * 32
    conn.execute(
        """
        UPDATE transactions
        SET external_id = ?, external_id_kind = 'txid', raw_json = ?
        WHERE id IN ('out', 'in')
        """,
        (native_txid, '{"txid":"' + native_txid + '"}'),
    )
    return native_txid


class CustodyQuantityHandlerTests(unittest.TestCase):
    def test_same_txid_rows_remain_external_without_observer_or_review(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            try:
                _prepare_same_txid_roll(conn)

                result = handlers.process_journals(conn, "Books", "Book")

                self.assertEqual(result["custody_quantity"]["differences"], 0)
                self.assertFalse(result["custody_quantity"]["blocked"])
                self.assertEqual(
                    conn.execute(
                        """
                        SELECT COUNT(*) FROM chain_observation_provenance
                        WHERE transaction_id IN ('out', 'in')
                        """
                    ).fetchone()[0],
                    0,
                )
                self.assertEqual(
                    {
                        row["entry_type"]
                        for row in conn.execute(
                            """
                            SELECT entry_type FROM journal_entries
                            WHERE transaction_id IN ('out', 'in')
                            """
                        ).fetchall()
                    },
                    {"acquisition", "disposal"},
                )
            finally:
                conn.close()

    def test_authoritative_same_txid_wallet_roll_compiles_to_internal_quantity(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            try:
                native_txid = _prepare_same_txid_roll(conn)
                profile = conn.execute(
                    "SELECT * FROM profiles WHERE id = 'profile'"
                ).fetchone()
                for wallet_id, direction in (("a", "outbound"), ("c", "inbound")):
                    wallet = conn.execute(
                        "SELECT * FROM wallets WHERE id = ?", (wallet_id,)
                    ).fetchone()
                    persist_chain_observation_provenance(
                        conn,
                        profile,
                        wallet,
                        application_revision=f"test:{wallet_id}",
                        chain="bitcoin",
                        network="main",
                        entries=(
                            {
                                "external_id": native_txid,
                                "asset": "BTC",
                                "direction": direction,
                                "observer_ids": [f"test-observer:{wallet_id}"],
                                "observer_kinds": ["bdk"],
                            },
                        ),
                    )

                result = handlers.process_journals(conn, "Books", "Book")

                self.assertEqual(result["custody_quantity"]["differences"], 0)
                self.assertFalse(result["custody_quantity"]["blocked"])
                self.assertEqual(
                    conn.execute(
                        """
                        SELECT COUNT(*) FROM journal_entries
                        WHERE transaction_id IN ('out', 'in')
                          AND entry_type IN ('transfer_out', 'transfer_in')
                        """
                    ).fetchone()[0],
                    2,
                )
            finally:
                conn.close()

    def test_promoted_gap_is_force_blocked_before_rp2_and_reported(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            try:
                conn.execute(
                    "UPDATE transactions SET privacy_boundary = 'coinjoin' WHERE id = 'out'"
                )
                result = handlers.process_journals(conn, "Books", "Book")

                self.assertTrue(result["custody_quantity"]["blocked"])
                self.assertEqual(
                    conn.execute(
                        """
                        SELECT COUNT(*) FROM journal_entries
                        WHERE transaction_id IN ('out', 'in')
                          AND entry_type IN ('acquisition', 'disposal')
                        """
                    ).fetchone()[0],
                    0,
                )
                self.assertEqual(
                    conn.execute(
                        """
                        SELECT amount_msat FROM journal_quantity_postings
                        WHERE transaction_id = 'out' AND location_kind = 'wallet'
                        """
                    ).fetchone()[0],
                    -10 * BTC,
                )
                conn.execute(
                    "INSERT INTO settings(key, value) VALUES('context_workspace', 'ws')"
                )
                conn.execute(
                    "INSERT INTO settings(key, value) VALUES('context_profile', 'profile')"
                )
                blocker_ids = {
                    item["id"] for item in build_report_blockers_snapshot(conn)["blockers"]
                }
                self.assertIn("custody_quantity_unresolved", blocker_ids)
            finally:
                conn.close()

    def test_component_evidence_quantity_drift_fail_closes_before_rp2(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            try:
                conn.execute("DELETE FROM transactions WHERE id = 'later-sale'")
                conn.execute("UPDATE transactions SET amount = ? WHERE id = 'in'", (10 * BTC,))
                component = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="manual_bridge",
                    evidence_kind="manual_reconstruction",
                    evidence_grade="reviewed",
                    legs=[
                        {
                            **_leg(
                                "source",
                                10 * BTC,
                                transaction_id="out",
                                wallet_id="a",
                                occurred_at=SOURCE_AT,
                            ),
                            "id": "source",
                        },
                        {
                            **_leg(
                                "destination",
                                10 * BTC,
                                transaction_id="in",
                                wallet_id="c",
                                occurred_at="2025-01-01T00:00:00Z",
                            ),
                            "id": "destination",
                        },
                    ],
                    allocations=[],
                )
                activate_component(conn, component["id"])
                conn.execute(
                    "UPDATE transactions SET amount = ? WHERE id = 'in'",
                    (9 * BTC,),
                )

                result = handlers.process_journals(conn, "Books", "Book")

                self.assertTrue(result["custody_quantity"]["blocked"])
                issue = conn.execute(
                    """
                    SELECT issue_type, reason, detail_json
                    FROM journal_quantity_issues
                    WHERE issue_type = 'component_claim_compile_failed'
                      AND reason = 'custody_component_authored_active_invalid'
                    """
                ).fetchone()
                self.assertEqual(
                    tuple(issue)[:2],
                    (
                        "component_claim_compile_failed",
                        "custody_component_authored_active_invalid",
                    ),
                )
                self.assertIn('"evidence_status": "evidence_mismatch"', issue[2])
                self.assertEqual(
                    conn.execute(
                        """
                        SELECT COUNT(*) FROM journal_entries
                        WHERE transaction_id IN ('out', 'in')
                          AND entry_type IN ('transfer_out', 'transfer_in')
                        """
                    ).fetchone()[0],
                    0,
                )
            finally:
                conn.close()

    def test_author_snapshot_is_audit_only_after_commitments_validate(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            try:
                conn.execute("DELETE FROM transactions WHERE id = 'later-sale'")
                conn.execute("UPDATE transactions SET amount = ? WHERE id = 'in'", (10 * BTC,))
                component = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="manual_bridge",
                    evidence_kind="manual_reconstruction",
                    evidence_grade="reviewed",
                    legs=[
                        {
                            **_leg(
                                "source",
                                10 * BTC,
                                transaction_id="out",
                                wallet_id="a",
                                occurred_at=SOURCE_AT,
                            ),
                            "id": "source",
                        },
                        {
                            **_leg(
                                "destination",
                                10 * BTC,
                                transaction_id="in",
                                wallet_id="c",
                                occurred_at="2025-01-01T00:00:00Z",
                            ),
                            "id": "destination",
                        },
                    ],
                    allocations=[],
                )
                activate_component(conn, component["id"])
                # Raw activation evidence stays local to the author. The
                # replicated commitment set remains the journal authority.
                conn.execute(
                    "DELETE FROM custody_authored_evidence_snapshots WHERE subject_id = ?",
                    (component["id"],),
                )

                result = handlers.process_journals(conn, "Books", "Book")

                self.assertFalse(result["custody_quantity"]["blocked"])
                self.assertEqual(
                    conn.execute(
                        """
                        SELECT COUNT(*) FROM journal_entries
                        WHERE transaction_id IN ('out', 'in')
                          AND entry_type IN ('transfer_out', 'transfer_in')
                        """
                    ).fetchone()[0],
                    2,
                )
                self.assertEqual(
                    conn.execute(
                        "SELECT COUNT(*) FROM custody_authored_evidence_snapshots "
                        "WHERE subject_id = ?",
                        (component["id"],),
                    ).fetchone()[0],
                    0,
                )
            finally:
                conn.close()

    def test_observer_lifecycle_enrichment_does_not_retract_reviewed_component(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            try:
                conn.execute("DELETE FROM transactions WHERE id = 'later-sale'")
                conn.execute("UPDATE transactions SET amount = ? WHERE id = 'in'", (10 * BTC,))
                component = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="manual_bridge",
                    evidence_kind="manual_reconstruction",
                    evidence_grade="reviewed",
                    legs=[
                        {
                            **_leg(
                                "source",
                                10 * BTC,
                                transaction_id="out",
                                wallet_id="a",
                                occurred_at=SOURCE_AT,
                            ),
                            "id": "source",
                        },
                        {
                            **_leg(
                                "destination",
                                10 * BTC,
                                transaction_id="in",
                                wallet_id="c",
                                occurred_at="2025-01-01T00:00:00Z",
                            ),
                            "id": "destination",
                        },
                    ],
                    allocations=[],
                )
                activate_component(conn, component["id"])
                conn.execute(
                    """
                    UPDATE transactions
                    SET fingerprint = 'observer-enriched',
                        confirmed_at = '2025-01-01T00:10:00Z',
                        raw_json = '{"graph_version":2,"block_height":900000}'
                    WHERE id = 'in'
                    """
                )

                result = handlers.process_journals(conn, "Books", "Book")

                self.assertFalse(result["custody_quantity"]["blocked"])
                self.assertEqual(
                    conn.execute(
                        """
                        SELECT COUNT(*) FROM journal_entries
                        WHERE transaction_id IN ('out', 'in')
                          AND entry_type IN ('transfer_out', 'transfer_in')
                        """
                    ).fetchone()[0],
                    2,
                )
            finally:
                conn.close()

    def test_residual_component_blocks_later_tax_for_every_generic_algorithm(self):
        for algorithm in ("FIFO", "LIFO", "HIFO", "LOFO", "MOVING_AVERAGE"):
            with self.subTest(algorithm=algorithm), tempfile.TemporaryDirectory() as root:
                conn = _book(root, algorithm)
                try:
                    component = create_component(
                        conn,
                        workspace_id="ws",
                        profile_id="profile",
                        component_type="manual_bridge",
                        evidence_kind="manual_reconstruction",
                        evidence_grade="reviewed",
                        legs=[
                            {
                                **_leg(
                                    "source", 10 * BTC,
                                    transaction_id="out", wallet_id="a",
                                    occurred_at=SOURCE_AT,
                                ),
                                "id": "source",
                            },
                            {
                                **_leg(
                                    "destination", 990_000_000_000,
                                    transaction_id="in", wallet_id="c",
                                    occurred_at="2025-01-01T00:00:00Z",
                                ),
                                "id": "destination",
                            },
                            {
                                **_leg(
                                    "suspense", 10_000_000_000,
                                    occurred_at=SOURCE_AT,
                                ),
                                "id": "suspense",
                            },
                        ],
                        allocations=[
                            {
                                "source_leg_id": "source",
                                "sink_leg_id": "destination",
                                "source_amount_msat": 990_000_000_000,
                                "sink_amount_msat": 990_000_000_000,
                            },
                            {
                                "source_leg_id": "source",
                                "sink_leg_id": "suspense",
                                "source_amount_msat": 10_000_000_000,
                                "sink_amount_msat": 10_000_000_000,
                            },
                        ],
                    )
                    activate_component(conn, component["id"])
                    result = handlers.process_journals(conn, "Books", "Book")

                    self.assertTrue(result["custody_quantity"]["blocked"])
                    self.assertEqual(
                        result["custody_quantity"]["blocked_from"],
                        SOURCE_AT,
                    )
                    transfer_out = conn.execute(
                        """
                        SELECT SUM(-quantity) AS amount
                        FROM journal_entries
                        WHERE profile_id = 'profile' AND transaction_id = 'out'
                          AND entry_type = 'transfer_out'
                        """
                    ).fetchone()["amount"]
                    self.assertEqual(transfer_out, 990_000_000_000)
                    self.assertIsNone(
                        conn.execute(
                            """
                            SELECT 1 FROM journal_entries
                            WHERE transaction_id = 'later-sale'
                              AND entry_type = 'disposal'
                            """
                        ).fetchone()
                    )
                    self.assertIsNotNone(
                        conn.execute(
                            """
                            SELECT 1 FROM journal_quarantines
                            WHERE transaction_id = 'later-sale'
                              AND reason = 'custody_basis_barrier'
                            """
                        ).fetchone()
                    )
                    issue = conn.execute(
                        """
                        SELECT state, amount_msat, blocks_from
                        FROM journal_quantity_issues
                        WHERE profile_id = 'profile'
                        """
                    ).fetchone()
                    self.assertEqual(tuple(issue), ("custody_suspense", 10_000_000_000, SOURCE_AT))
                    self.assertEqual(
                        conn.execute(
                            """
                            SELECT COUNT(*) FROM custody_authored_evidence_snapshots
                            WHERE subject_id = ?
                            """,
                            (component["id"],),
                        ).fetchone()[0],
                        2,
                    )
                    profile = conn.execute(
                        "SELECT * FROM profiles WHERE id = 'profile'"
                    ).fetchone()
                    with self.assertRaises(AppError) as blocked:
                        core_report_context.require_report_context(
                            conn, "ws", "profile", handlers.resolve_scope
                        )
                    self.assertEqual(
                        blocked.exception.code,
                        "custody_quantity_unresolved",
                    )
                finally:
                    conn.close()

    def test_complete_reviewed_component_is_canonical_differential_equal(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            try:
                conn.execute("DELETE FROM transactions WHERE id = 'later-sale'")
                conn.execute("UPDATE transactions SET amount = ? WHERE id = 'in'", (10 * BTC,))
                component = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="manual_bridge",
                    evidence_kind="manual_reconstruction",
                    evidence_grade="reviewed",
                    legs=[
                        {
                            **_leg(
                                "source",
                                10 * BTC,
                                transaction_id="out",
                                wallet_id="a",
                                occurred_at=SOURCE_AT,
                            ),
                            "id": "source",
                        },
                        {
                            **_leg(
                                "destination",
                                10 * BTC,
                                transaction_id="in",
                                wallet_id="c",
                                occurred_at="2025-01-01T00:00:00Z",
                            ),
                            "id": "destination",
                        },
                    ],
                    allocations=[],
                )
                activate_component(conn, component["id"])
                result = handlers.process_journals(conn, "Books", "Book")
                self.assertEqual(result["custody_quantity"]["differences"], 0)
                self.assertFalse(result["custody_quantity"]["blocked"])
            finally:
                conn.close()

    def test_austrian_moving_average_residual_blocks_report_readiness(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "MOVING_AVERAGE_AT", tax_country="at")
            try:
                component = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="manual_bridge",
                    evidence_kind="manual_reconstruction",
                    evidence_grade="reviewed",
                    legs=[
                        {
                            **_leg(
                                "source",
                                10 * BTC,
                                transaction_id="out",
                                wallet_id="a",
                                occurred_at=SOURCE_AT,
                            ),
                            "id": "source",
                        },
                        {
                            **_leg(
                                "destination",
                                990_000_000_000,
                                transaction_id="in",
                                wallet_id="c",
                                occurred_at="2025-01-01T00:00:00Z",
                            ),
                            "id": "destination",
                        },
                        {
                            **_leg(
                                "suspense",
                                10_000_000_000,
                                occurred_at=SOURCE_AT,
                            ),
                            "id": "suspense",
                        },
                    ],
                    allocations=[
                        {
                            "source_leg_id": "source",
                            "sink_leg_id": "destination",
                            "source_amount_msat": 990_000_000_000,
                            "sink_amount_msat": 990_000_000_000,
                        },
                        {
                            "source_leg_id": "source",
                            "sink_leg_id": "suspense",
                            "source_amount_msat": 10_000_000_000,
                            "sink_amount_msat": 10_000_000_000,
                        },
                    ],
                )
                activate_component(conn, component["id"])

                result = handlers.process_journals(conn, "Books", "Book")
                self.assertTrue(result["custody_quantity"]["blocked"])
                self.assertEqual(
                    result["custody_quantity"]["blocked_from"],
                    SOURCE_AT,
                )
                profile = conn.execute(
                    "SELECT * FROM profiles WHERE id = 'profile'"
                ).fetchone()
                with self.assertRaises(AppError) as blocked:
                    core_report_context.require_report_context(
                        conn, "ws", "profile", handlers.resolve_scope
                    )
                self.assertEqual(
                    blocked.exception.code,
                    "custody_quantity_unresolved",
                )
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
