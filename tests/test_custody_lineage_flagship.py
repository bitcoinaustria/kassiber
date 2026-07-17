"""Flagship acceptance proof for a long-lived Bitcoin treasury.

The fixture deliberately reads like the history in the custody-lineage plan:
an old acquisition survives multisig rotations, a fully imported Whirlpool
path, a real payment, and a later vault roll.  Variant tests replace only the
Whirlpool segment so their assertions exercise the same source acquisition and
downstream basis consumer instead of four unrelated toy graphs.
"""

from __future__ import annotations

from dataclasses import dataclass
import json
import random
import tempfile
import unittest

from kassiber.cli import handlers
from kassiber.core import (
    custody_components,
    custody_gap_reviews,
    custody_gaps,
    custody_journal,
    reports as core_reports,
)
from kassiber.core.custody_quantity_store import component_native_support_status
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.custody_tax_helpers import persist_authoritative_chain_observation


BTC = 100_000_000_000


def _review_candidate(conn, candidate, *, authored_source="gui"):
    plan = custody_gap_reviews.plan_review(
        conn,
        workspace_id="ws",
        profile_id="profile",
        action="create",
        candidate=candidate,
        authored_source=authored_source,
    )
    return custody_gap_reviews.apply_review(
        conn,
        workspace_id="ws",
        profile_id="profile",
        action="create",
        candidate=candidate,
        expected_input_version=plan["input_version"],
        authored_source=authored_source,
    )


@dataclass(frozen=True)
class _Transaction:
    row_id: str
    wallet_id: str
    direction: str
    amount_msat: int
    occurred_at: str
    fee_msat: int = 0
    txid: str | None = None
    kind: str | None = None
    privacy_boundary: str | None = None
    raw_extra: dict[str, object] | None = None


class _FlagshipTreasury:
    """Small database-backed builder shared by all flagship variants."""

    wallet_labels = {
        "a": "Multisig A",
        "b": "Multisig B",
        "deposit": "Samourai Deposit",
        "premix": "Samourai Premix",
        "postmix": "Samourai Postmix",
        "c": "Operative C",
        "d": "Multisig D",
        "friend-a": "Unrelated Revenue Wallet",
        "friend-b": "Competing Return Wallet",
    }

    def __init__(self, root: str):
        self.conn = open_db(root)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) "
            "VALUES('ws', 'Books', '2015-01-01T00:00:00Z')"
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                gains_algorithm, created_at
            ) VALUES(
                'profile', 'ws', 'OG Treasury', 'EUR', 'generic', 'FIFO',
                '2015-01-01T00:00:00Z'
            )
            """
        )
        for wallet_id, label in self.wallet_labels.items():
            config = {"chain": "bitcoin", "network": "main"}
            if wallet_id in {"deposit", "premix", "postmix"}:
                config["samourai"] = {
                    "role": "child",
                    "section": wallet_id,
                    "group_id": "flagship-whirlpool",
                }
            self.conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json,
                    created_at
                ) VALUES(?, 'ws', 'profile', ?, 'descriptor', ?,
                         '2015-01-01T00:00:00Z')
                """,
                (wallet_id, label, json.dumps(config, sort_keys=True)),
            )

    def close(self) -> None:
        self.conn.close()

    @staticmethod
    def _txid(number: int) -> str:
        return f"{number:064x}"

    @staticmethod
    def acquisition() -> _Transaction:
        # The extra 0.03 BTC funds the five real on-chain fees before C.
        return _Transaction(
            "2015-acquisition",
            "a",
            "inbound",
            10 * BTC + 3 * BTC // 100,
            "2015-06-01T00:00:00Z",
            kind="buy",
        )

    @classmethod
    def exact_move(
        cls,
        stem: str,
        source_wallet: str,
        destination_wallet: str,
        amount_msat: int,
        fee_msat: int,
        occurred_at: str,
        txid_number: int,
        *,
        privacy_boundary: str | None = None,
    ) -> tuple[_Transaction, _Transaction]:
        txid = cls._txid(txid_number)
        return (
            _Transaction(
                f"{stem}-out",
                source_wallet,
                "outbound",
                amount_msat,
                occurred_at,
                fee_msat=fee_msat,
                txid=txid,
                privacy_boundary=privacy_boundary,
            ),
            _Transaction(
                f"{stem}-in",
                destination_wallet,
                "inbound",
                amount_msat,
                occurred_at,
                txid=txid,
                privacy_boundary=privacy_boundary,
            ),
        )

    @classmethod
    def common_prefix(cls) -> list[_Transaction]:
        return [
            cls.acquisition(),
            *cls.exact_move(
                "2018-a-to-b",
                "a",
                "b",
                10 * BTC + 2 * BTC // 100,
                BTC // 100,
                "2018-04-01T00:00:00Z",
                1,
            ),
        ]

    @classmethod
    def complete_history(cls) -> list[_Transaction]:
        rows = cls.common_prefix()
        rows.extend(
            cls.exact_move(
                "2020-b-to-deposit",
                "b",
                "deposit",
                10 * BTC + BTC // 100,
                BTC // 100,
                "2020-01-01T00:00:00Z",
                2,
            )
        )
        rows.extend(
            cls.exact_move(
                "2020-tx0",
                "deposit",
                "premix",
                10 * BTC + BTC // 200,
                BTC // 200,
                "2020-01-02T00:00:00Z",
                3,
                privacy_boundary="coinjoin",
            )
        )
        rows.extend(
            cls.exact_move(
                "2020-mix",
                "premix",
                "postmix",
                10 * BTC + BTC // 1000,
                BTC // 250,
                "2020-02-01T00:00:00Z",
                4,
                privacy_boundary="coinjoin",
            )
        )
        rows.extend(
            cls.exact_move(
                "2022-mixout",
                "postmix",
                "c",
                10 * BTC,
                BTC // 1000,
                "2022-01-01T00:00:00Z",
                5,
                privacy_boundary="coinjoin",
            )
        )
        rows.extend(cls.downstream_history(c_balance_msat=10 * BTC))
        return rows

    @classmethod
    def missing_whirlpool_boundaries(cls) -> list[_Transaction]:
        return [
            *cls.common_prefix(),
            _Transaction(
                "2020-whirlpool-out",
                "b",
                "outbound",
                10 * BTC,
                "2020-01-01T00:00:00Z",
                fee_msat=BTC // 100,
                txid=cls._txid(10),
                kind="samourai_deposit",
                privacy_boundary="coinjoin",
            ),
            _Transaction(
                "2021-return-1",
                "c",
                "inbound",
                6 * BTC,
                "2021-01-01T00:00:00Z",
                txid=cls._txid(11),
            ),
            _Transaction(
                "2021-return-2",
                "c",
                "inbound",
                39 * BTC // 10,
                "2021-02-01T00:00:00Z",
                txid=cls._txid(12),
            ),
            _Transaction(
                "2023-vendor",
                "c",
                "outbound",
                BTC // 2,
                "2023-01-01T00:00:00Z",
                fee_msat=BTC // 1000,
                kind="expense",
            ),
        ]

    @classmethod
    def downstream_history(cls, *, c_balance_msat: int) -> list[_Transaction]:
        vendor_amount = BTC
        vendor_fee = BTC // 1000
        roll_fee = BTC // 1000
        return [
            _Transaction(
                "2023-vendor",
                "c",
                "outbound",
                vendor_amount,
                "2023-01-01T00:00:00Z",
                fee_msat=vendor_fee,
                kind="expense",
            ),
            *cls.exact_move(
                "2024-c-to-d",
                "c",
                "d",
                c_balance_msat - vendor_amount - vendor_fee - roll_fee,
                roll_fee,
                "2024-01-01T00:00:00Z",
                6,
            ),
        ]

    def insert(self, rows: list[_Transaction]) -> None:
        for ordinal, row in enumerate(rows):
            raw = dict(row.raw_extra or {})
            if row.txid is not None:
                raw["txid"] = row.txid
                raw["chain"] = "bitcoin"
                raw["network"] = "main"
            self.conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    external_id_kind, fingerprint, occurred_at, direction,
                    asset, amount, fee, fiat_currency, fiat_rate,
                    fiat_rate_exact, kind, privacy_boundary, raw_json, created_at
                ) VALUES(
                    ?, 'ws', 'profile', ?, ?, ?, ?, ?, ?, 'BTC', ?, ?, 'EUR',
                    20000, '20000', ?, ?, ?, ?
                )
                """,
                (
                    row.row_id,
                    row.wallet_id,
                    row.txid or row.row_id,
                    "txid" if row.txid else None,
                    f"flagship:{row.row_id}",
                    row.occurred_at,
                    row.direction,
                    row.amount_msat,
                    row.fee_msat,
                    row.kind,
                    row.privacy_boundary,
                    json.dumps(raw, sort_keys=True),
                    # Deliberately import-order-specific. The runtime must
                    # still canonicalize by economic/event identity.
                    f"2030-01-01T00:{ordinal:02d}:00Z",
                ),
            )
            if row.txid is not None:
                persist_authoritative_chain_observation(self.conn, row.row_id)
        self.conn.commit()

    def process(self) -> dict[str, object]:
        return handlers.process_journals(self.conn, "Books", "OG Treasury")

    def derived_signature(self) -> tuple[tuple[object, ...], ...]:
        queries = (
            """
            SELECT 'entry', transaction_id, entry_type, wallet_id, asset,
                   quantity, fiat_value_exact, cost_basis_exact,
                   proceeds_exact, gain_loss_exact
            FROM journal_entries WHERE profile_id = 'profile'
            ORDER BY transaction_id, entry_type, wallet_id, quantity
            """,
            """
            SELECT 'posting', COALESCE(transaction_id, ''), location_kind,
                   location_id, asset, amount_msat, state, '', '', ''
            FROM journal_quantity_postings WHERE profile_id = 'profile'
            ORDER BY transaction_id, location_kind, location_id, amount_msat
            """,
            """
            SELECT 'issue', issue_type, state, COALESCE(asset, ''),
                   COALESCE(amount_msat, 0), reason, blocks_from, '', '', ''
            FROM journal_quantity_issues WHERE profile_id = 'profile'
            ORDER BY issue_type, state, amount_msat, reason
            """,
        )
        return tuple(
            tuple(row)
            for query in queries
            for row in self.conn.execute(query).fetchall()
        )


class CustodyLineageFlagshipTests(unittest.TestCase):
    def test_core_builder_runs_the_real_database_pipeline_without_handlers(self):
        with tempfile.TemporaryDirectory() as root:
            book = _FlagshipTreasury(root)
            try:
                history = _FlagshipTreasury.complete_history()
                book.insert(history)
                profile = book.conn.execute(
                    "SELECT * FROM profiles WHERE id = 'profile'"
                ).fetchone()

                core_state = custody_journal.build_ledger_state(book.conn, profile)
                self.assertEqual(
                    len(core_state["custody_quantity"].projection.observations),
                    len(history),
                )
                self.assertEqual(
                    {
                        entry["transaction_id"]
                        for entry in core_state["entries"]
                        if entry["entry_type"] == "transfer_out"
                    },
                    {
                        "2018-a-to-b-out",
                        "2020-b-to-deposit-out",
                        "2020-tx0-out",
                        "2020-mix-out",
                        "2022-mixout-out",
                        "2024-c-to-d-out",
                    },
                )
                self.assertFalse(core_state["custody_quantity"].report_blocked)
            finally:
                book.close()

    def test_complete_policy_history_is_automatic_fee_exact_and_order_invariant(self):
        rows = _FlagshipTreasury.complete_history()
        signatures = []
        for order in (rows, random.Random(20260713).sample(rows, len(rows))):
            with tempfile.TemporaryDirectory() as root:
                book = _FlagshipTreasury(root)
                try:
                    book.insert(list(order))
                    first = book.process()
                    first_signature = book.derived_signature()
                    second = book.process()

                    self.assertFalse(first["custody_quantity"]["blocked"])
                    self.assertEqual(first["custody_quantity"]["differences"], 0)
                    self.assertEqual(first["custody_quantity"]["issues"], 0)
                    self.assertEqual(first_signature, book.derived_signature())
                    self.assertEqual(
                        first["entries_created"], second["entries_created"]
                    )
                    self.assertEqual(
                        book.conn.execute(
                            """
                            SELECT COALESCE(SUM(amount_msat), 0)
                            FROM journal_quantity_postings
                            WHERE profile_id = 'profile'
                              AND location_kind = 'fee'
                            """
                        ).fetchone()[0],
                        32 * BTC // 1000,
                    )
                    self.assertEqual(
                        {
                            row[0]
                            for row in book.conn.execute(
                                """
                                SELECT transaction_id FROM journal_entries
                                WHERE profile_id = 'profile'
                                  AND entry_type = 'transfer_out'
                                """
                            )
                        },
                        {
                            "2018-a-to-b-out",
                            "2020-b-to-deposit-out",
                            "2020-tx0-out",
                            "2020-mix-out",
                            "2022-mixout-out",
                            "2024-c-to-d-out",
                        },
                    )
                    self.assertEqual(
                        book.conn.execute(
                            """
                            SELECT amount_msat FROM journal_quantity_balances
                            WHERE profile_id = 'profile'
                              AND location_kind = 'wallet'
                              AND location_id = 'd' AND asset = 'BTC'
                            """
                        ).fetchone()[0],
                        8_998 * BTC // 1000,
                    )
                    signatures.append(first_signature)
                finally:
                    book.close()
        self.assertEqual(signatures[0], signatures[1])

    def test_missing_whirlpool_review_carries_99_and_keeps_residual_and_sale_blocked(self):
        with tempfile.TemporaryDirectory() as root:
            book = _FlagshipTreasury(root)
            try:
                book.insert(_FlagshipTreasury.missing_whirlpool_boundaries())
                before = book.process()
                candidate = next(
                    item
                    for item in custody_gaps.load_gap_search_result(
                        book.conn, "profile"
                    )[0].candidates
                    if item.source_ids == ("2020-whirlpool-out",)
                    and item.return_ids
                    == ("2021-return-1", "2021-return-2")
                )
                self.assertTrue(candidate.promotion_eligible)
                created = _review_candidate(book.conn, candidate)
                # The later vault roll arrives after review; it must not change
                # the already reviewed missing-history boundary.
                book.insert(
                    _FlagshipTreasury.exact_move(
                        "2024-c-to-d",
                        "c",
                        "d",
                        9_399 * BTC // 1000,
                        BTC // 1000,
                        "2024-01-01T00:00:00Z",
                        6,
                    )
                )
                after = book.process()

                self.assertTrue(before["custody_quantity"]["blocked"])
                self.assertEqual(created["retained_msat"], 99 * BTC // 10)
                self.assertEqual(created["residual_msat"], BTC // 10)
                self.assertTrue(after["custody_quantity"]["blocked"])
                self.assertEqual(
                    after["custody_quantity"]["blocked_from"],
                    "2020-01-01T00:00:00Z",
                )
                self.assertEqual(
                    book.conn.execute(
                        """
                        SELECT COALESCE(SUM(quantity), 0)
                        FROM journal_entries
                        WHERE profile_id = 'profile'
                          AND transaction_id IN (
                              '2021-return-1', '2021-return-2'
                          )
                          AND entry_type = 'transfer_in'
                        """
                    ).fetchone()[0],
                    99 * BTC // 10,
                )
                self.assertEqual(
                    book.conn.execute(
                        """
                        SELECT COALESCE(SUM(-quantity), 0)
                        FROM journal_entries
                        WHERE profile_id = 'profile'
                          AND transaction_id = '2020-whirlpool-out'
                          AND entry_type = 'transfer_fee'
                        """
                    ).fetchone()[0],
                    BTC // 100,
                )
                self.assertEqual(
                    tuple(
                        book.conn.execute(
                            """
                            SELECT amount_msat, state FROM journal_quantity_issues
                            WHERE profile_id = 'profile'
                              AND state = 'custody_suspense'
                            """
                        ).fetchone()
                    ),
                    (BTC // 10, "custody_suspense"),
                )
                self.assertIsNone(
                    book.conn.execute(
                        """
                        SELECT 1 FROM journal_entries
                        WHERE transaction_id = '2023-vendor'
                          AND entry_type = 'disposal'
                        """
                    ).fetchone()
                )
                self.assertIsNotNone(
                    book.conn.execute(
                        """
                        SELECT 1 FROM journal_quarantines
                        WHERE transaction_id = '2023-vendor'
                          AND reason = 'custody_basis_barrier'
                        """
                    ).fetchone()
                )
                self.assertEqual(
                    book.conn.execute(
                        "SELECT revision FROM custody_components WHERE id = ?",
                        (created["component_id"],),
                    ).fetchone()[0],
                    1,
                )
            finally:
                book.close()

    def test_exit_tax_blocks_until_exact_missing_wallet_bridge_carries_basis(self):
        with tempfile.TemporaryDirectory() as root:
            book = _FlagshipTreasury(root)
            try:
                rows = _FlagshipTreasury.missing_whirlpool_boundaries()
                rows = [
                    (
                        _Transaction(
                            "2021-return-2",
                            "c",
                            "inbound",
                            4 * BTC,
                            "2021-02-01T00:00:00Z",
                            txid=_FlagshipTreasury._txid(12),
                        )
                        if row.row_id == "2021-return-2"
                        else row
                    )
                    for row in rows
                ]
                book.insert(rows)
                # If the returns were booked as acquisitions these rates would
                # manufacture EUR 500k of basis. The reviewed bridge must carry
                # the original EUR 20k/BTC lot instead.
                book.conn.execute(
                    "UPDATE transactions SET fiat_rate = 50000, "
                    "fiat_rate_exact = '50000' "
                    "WHERE id IN ('2021-return-1', '2021-return-2')"
                )
                book.conn.execute(
                    "INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at) "
                    "VALUES('BTC-EUR', '2026-06-15T00:00:00Z', 30000, 'manual', "
                    "'2026-06-15T00:00:00Z')"
                )
                book.conn.commit()

                before = book.process()
                self.assertTrue(before["custody_quantity"]["blocked"])
                with self.assertRaises(AppError) as blocked:
                    core_reports.report_exit_tax(
                        book.conn,
                        "Books",
                        "OG Treasury",
                        handlers._report_hooks(),
                        departure_date="2026-06-16",
                        destination="eu_eea",
                    )
                self.assertEqual(blocked.exception.code, "custody_quantity_unresolved")

                candidate = next(
                    item
                    for item in custody_gaps.load_gap_search_result(
                        book.conn, "profile"
                    )[0].candidates
                    if item.source_ids == ("2020-whirlpool-out",)
                    and item.return_ids == ("2021-return-1", "2021-return-2")
                )
                created = _review_candidate(book.conn, candidate)
                self.assertEqual(created["retained_msat"], 10 * BTC)
                self.assertEqual(created["residual_msat"], 0)

                after = book.process()
                self.assertFalse(after["custody_quantity"]["blocked"])
                report = core_reports.report_exit_tax(
                    book.conn,
                    "Books",
                    "OG Treasury",
                    handlers._report_hooks(),
                    departure_date="2026-06-16",
                    destination="eu_eea",
                )
                self.assertEqual(report["totals"]["neuQuantitySats"], 950_900_000)
                self.assertEqual(report["totals"]["neuCostBasis"], 190_180.0)
                self.assertFalse(
                    book.conn.execute(
                        """
                        SELECT 1 FROM journal_entries
                        WHERE transaction_id IN ('2021-return-1', '2021-return-2')
                          AND entry_type = 'acquisition'
                        """
                    ).fetchone()
                )
            finally:
                book.close()


    def test_recovered_policy_evidence_validates_or_blocks_without_rewriting_bridge(self):
        with tempfile.TemporaryDirectory() as root:
            book = _FlagshipTreasury(root)
            try:
                book.insert(_FlagshipTreasury.missing_whirlpool_boundaries())
                book.process()
                candidate = next(
                    item
                    for item in custody_gaps.load_gap_search_result(
                        book.conn, "profile"
                    )[0].candidates
                    if item.source_ids == ("2020-whirlpool-out",)
                    and len(item.return_ids) == 2
                )
                created = _review_candidate(book.conn, candidate)
                component_id = created["component_id"]
                original = book.conn.execute(
                    """
                    SELECT id, lineage_id, revision, state, notes
                    FROM custody_components WHERE id = ?
                    """,
                    (component_id,),
                ).fetchone()
                component = custody_components.get_component(
                    book.conn, component_id, include_local_evidence=False
                )
                self.assertEqual(
                    component["native_support_status"]["status"], "unverified"
                )

                # Recover the source-side Deposit policy first. Exact native
                # identity supports one of the three reviewed boundaries, so
                # the derived status becomes partial without revising the
                # user's component.
                book.insert(
                    [
                        _Transaction(
                            "2020-recovered-deposit-in",
                            "deposit",
                            "inbound",
                            10 * BTC,
                            "2020-01-01T00:00:00Z",
                            txid=_FlagshipTreasury._txid(10),
                            privacy_boundary="coinjoin",
                        )
                    ]
                )
                partial = custody_components.get_component(
                    book.conn, component_id, include_local_evidence=False
                )
                self.assertEqual(
                    partial["native_support_status"]["status"], "partial"
                )
                self.assertEqual(
                    partial["native_support_status"]["supported_boundary_count"],
                    1,
                )

                # The rest of the recovered Deposit/Premix/Postmix path ends
                # in exact counterparts to both original C receipt anchors.
                # The 0.1 BTC difference is represented by actual recovered
                # Postmix spend fees, not an amount-only inference.
                recovered = [
                    *_FlagshipTreasury.exact_move(
                        "2020-recovered-tx0",
                        "deposit",
                        "premix",
                        10 * BTC,
                        0,
                        "2020-01-02T00:00:00Z",
                        20,
                        privacy_boundary="coinjoin",
                    ),
                    *_FlagshipTreasury.exact_move(
                        "2020-recovered-remix",
                        "premix",
                        "postmix",
                        10 * BTC,
                        0,
                        "2020-06-01T00:00:00Z",
                        21,
                        privacy_boundary="coinjoin",
                    ),
                    _Transaction(
                        "2021-recovered-postmix-out-1",
                        "postmix",
                        "outbound",
                        6 * BTC,
                        "2021-01-01T00:00:00Z",
                        fee_msat=BTC // 20,
                        txid=_FlagshipTreasury._txid(11),
                        privacy_boundary="coinjoin",
                    ),
                    _Transaction(
                        "2021-recovered-postmix-out-2",
                        "postmix",
                        "outbound",
                        39 * BTC // 10,
                        "2021-02-01T00:00:00Z",
                        fee_msat=BTC // 20,
                        txid=_FlagshipTreasury._txid(12),
                        privacy_boundary="coinjoin",
                    ),
                ]
                book.insert(recovered)
                corroborated = custody_components.get_component(
                    book.conn, component_id, include_local_evidence=False
                )
                self.assertEqual(corroborated["effective_state"], "active")
                self.assertEqual(
                    corroborated["native_support_status"]["status"],
                    "corroborated",
                )
                self.assertEqual(
                    corroborated["native_support_status"][
                        "supported_boundary_count"
                    ],
                    3,
                )
                self.assertEqual(
                    tuple(
                        book.conn.execute(
                            """
                            SELECT id, lineage_id, revision, state, notes
                            FROM custody_components WHERE id = ?
                            """,
                            (component_id,),
                        ).fetchone()
                    ),
                    tuple(original),
                )
                gap = next(
                    item
                    for item in custody_gaps.build_gap_snapshot(
                        book.conn, "profile", gap_id=candidate.gap_id
                    )["gaps"]
                    if item["gap_id"] == candidate.gap_id
                )
                self.assertEqual(gap["status"], "resolved")
                self.assertEqual(
                    gap["native_support_status"], "corroborated"
                )

                # A recovered same-event quantity contradiction invalidates
                # local use, but never rewrites the authored bridge.
                book.conn.execute(
                    """
                    UPDATE transactions
                    SET amount = ?
                    WHERE id = '2021-recovered-postmix-out-2'
                    """,
                    (4 * BTC,),
                )
                book.conn.commit()
                contradicted_component = custody_components.get_component(
                    book.conn, component_id, include_local_evidence=False
                )
                self.assertEqual(
                    contradicted_component["native_support_status"]["status"],
                    "contradicted",
                )
                self.assertEqual(
                    contradicted_component["effective_state"], "draft"
                )
                contradicted = book.process()

                self.assertTrue(contradicted["custody_quantity"]["blocked"])
                self.assertIsNotNone(
                    book.conn.execute(
                        """
                        SELECT 1 FROM journal_quantity_issues
                        WHERE reason = 'custody_component_authored_active_invalid'
                        """
                    ).fetchone()
                )
                self.assertEqual(
                    tuple(
                        book.conn.execute(
                            """
                            SELECT id, lineage_id, revision, state, notes
                            FROM custody_components WHERE id = ?
                            """,
                            (component_id,),
                        ).fetchone()
                    ),
                    tuple(original),
                )
            finally:
                book.close()

    def test_false_friend_revenue_and_competing_returns_stay_alternatives(self):
        rows = [
            _Transaction(
                "false-source",
                "b",
                "outbound",
                10 * BTC,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _Transaction(
                "false-revenue",
                "friend-a",
                "inbound",
                99 * BTC // 10,
                "2021-01-01T00:00:00Z",
                kind="revenue",
            ),
            _Transaction(
                "false-return-a",
                "c",
                "inbound",
                99 * BTC // 10,
                "2021-02-01T00:00:00Z",
            ),
            _Transaction(
                "false-return-b",
                "friend-b",
                "inbound",
                99 * BTC // 10,
                "2021-02-01T00:00:00Z",
            ),
        ]
        with tempfile.TemporaryDirectory() as root:
            book = _FlagshipTreasury(root)
            try:
                book.insert(rows)
                result = book.process()
                gap_result, _ = custody_gaps.load_gap_search_result(
                    book.conn, "profile"
                )
                candidates = list(gap_result.candidates)
                source_candidates = [
                    item
                    for item in candidates
                    if item.source_ids == ("false-source",)
                ]

                self.assertGreaterEqual(len(source_candidates), 3)
                self.assertTrue(
                    any(
                        item.return_ids == ("false-revenue",)
                        and "structured_external_origin" in item.reason_codes
                        for item in source_candidates
                    )
                )
                self.assertFalse(
                    any(item.promotion_eligible for item in source_candidates)
                )
                self.assertTrue(
                    all(item.conflict_size >= 2 for item in source_candidates)
                )
                self.assertTrue(result["custody_quantity"]["blocked"])
                self.assertEqual(
                    {
                        row[0]
                        for row in book.conn.execute(
                            "SELECT reason FROM journal_quantity_issues"
                        )
                    },
                    {"privacy_hop_unresolved"},
                )
                self.assertEqual(
                    book.conn.execute(
                        "SELECT COUNT(*) FROM custody_components"
                    ).fetchone()[0],
                    0,
                )
                self.assertEqual(
                    book.conn.execute(
                        """
                        SELECT state FROM journal_quantity_postings
                        WHERE transaction_id = 'false-source'
                          AND location_kind = 'external'
                        """
                    ).fetchone()[0],
                    "external_presumed",
                )
            finally:
                book.close()


class CustodyNativeSupportStatusTests(unittest.TestCase):
    """Protocol and quantity boundaries for recovered native support."""

    def setUp(self) -> None:
        self.root = tempfile.TemporaryDirectory()
        self.book = _FlagshipTreasury(self.root.name)

    def tearDown(self) -> None:
        self.book.close()
        self.root.cleanup()

    def _insert_native(
        self,
        row_id: str,
        wallet_id: str,
        direction: str,
        amount_msat: int,
        *,
        txid: str,
        chain: str = "bitcoin",
        network: str = "main",
        asset: str = "BTC",
    ) -> None:
        raw_json = json.dumps(
            {"chain": chain, "network": network, "txid": txid},
            sort_keys=True,
        )
        self.book.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint, occurred_at, direction,
                asset, amount, fee, raw_json, created_at
            ) VALUES(
                ?, 'ws', 'profile', ?, ?, 'txid', ?,
                '2020-01-01T00:00:00Z', ?, ?, ?, 0, ?,
                '2020-01-01T00:00:00Z'
            )
            """,
            (
                row_id,
                wallet_id,
                txid,
                f"native-support:{row_id}",
                direction,
                asset,
                amount_msat,
                raw_json,
            ),
        )
        persist_authoritative_chain_observation(
            self.book.conn,
            row_id,
            observer_kind="lwk" if chain == "liquid" else "bitcoinrpc",
        )
        self.book.conn.commit()

    def _status(self) -> dict[str, object]:
        return component_native_support_status(
            self.book.conn,
            {
                "profile_id": "profile",
                "legs": (
                    {
                        "role": "source",
                        "transaction_id": "anchor",
                        "amount_msat": 100,
                    },
                ),
            },
        )

    def test_cross_network_txid_collision_is_ignored(self):
        txid = _FlagshipTreasury._txid(100)
        self._insert_native("anchor", "b", "outbound", 100, txid=txid)
        self._insert_native(
            "testnet-collision",
            "c",
            "inbound",
            100,
            txid=txid,
            network="test",
        )

        status = self._status()

        self.assertEqual(status["status"], "unverified")
        self.assertEqual(status["contradicted_boundary_count"], 0)

    def test_partial_one_to_many_native_boundary_is_partial_not_contradicted(self):
        txid = _FlagshipTreasury._txid(101)
        self._insert_native("anchor", "b", "outbound", 100, txid=txid)
        self._insert_native("part-a", "c", "inbound", 30, txid=txid)
        self._insert_native("part-b", "d", "inbound", 30, txid=txid)

        status = self._status()

        self.assertEqual(status["status"], "partial")
        self.assertEqual(status["partially_supported_boundary_count"], 1)
        self.assertEqual(status["contradicted_boundary_count"], 0)

    def test_liquid_other_asset_output_is_ignored(self):
        txid = _FlagshipTreasury._txid(102)
        self._insert_native(
            "anchor",
            "b",
            "outbound",
            100,
            txid=txid,
            chain="liquid",
            asset="LBTC",
        )
        self._insert_native(
            "lbtc-return",
            "c",
            "inbound",
            100,
            txid=txid,
            chain="liquid",
            asset="LBTC",
        )
        self._insert_native(
            "other-asset",
            "d",
            "inbound",
            50_000,
            txid=txid,
            chain="liquid",
            asset="USDT",
        )

        status = self._status()

        self.assertEqual(status["status"], "corroborated")
        self.assertEqual(status["supported_boundary_count"], 1)
        self.assertEqual(status["contradicted_boundary_count"], 0)

    def test_same_event_overcoverage_is_contradicted(self):
        txid = _FlagshipTreasury._txid(103)
        self._insert_native("anchor", "b", "outbound", 100, txid=txid)
        self._insert_native("part-a", "c", "inbound", 60, txid=txid)
        self._insert_native("part-b", "d", "inbound", 50, txid=txid)

        status = self._status()

        self.assertEqual(status["status"], "contradicted")
        self.assertEqual(status["contradicted_boundary_count"], 1)
        self.assertFalse(status["usable"])


if __name__ == "__main__":
    unittest.main()
