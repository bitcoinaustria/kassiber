import json
import unittest
from decimal import Decimal

from kassiber.core.austrian import (
    AT_CATEGORY_TO_KENNZAHL,
    REGIME_ALT,
    REGIME_NEU,
    infer_regime_from_timestamp,
    kennzahl_for_disposal_category,
)
from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.errors import AppError
from tests.tax_ledger_generator import finalized, pair, profile, row

MOVING_AVERAGE_AT = profile("at", "moving_average_at")


def _ledger(book_profile, rows, pairs=()):
    return GenericRP2TaxEngine(book_profile).build_ledger_state(
        finalized(book_profile, rows, pairs)
    )


def _entry(state, transaction_id, entry_type):
    [entry] = [
        entry
        for entry in state.entries
        if entry["transaction_id"] == transaction_id
        and entry["entry_type"] == entry_type
    ]
    return entry


def _description(state, transaction_id, entry_type):
    return _entry(state, transaction_id, entry_type)["description"]


class InferRegimeFromTimestampTest(unittest.TestCase):
    def test_altvermoegen_before_cutoff(self):
        self.assertEqual(
            infer_regime_from_timestamp("2021-01-15T12:00:00+01:00"),
            REGIME_ALT,
        )

    def test_neuvermoegen_at_cutoff(self):
        # Europe/Vienna 2021-03-01 00:00:00 — the cutoff itself is the first Neu moment.
        self.assertEqual(
            infer_regime_from_timestamp("2021-03-01T00:00:00+01:00"),
            REGIME_NEU,
        )

    def test_neuvermoegen_modern(self):
        self.assertEqual(
            infer_regime_from_timestamp("2025-12-01T00:00:00Z"),
            REGIME_NEU,
        )

    def test_handles_z_suffix(self):
        self.assertEqual(
            infer_regime_from_timestamp("2020-06-15T00:00:00Z"),
            REGIME_ALT,
        )


class OutboundRegimeTest(unittest.TestCase):
    """The regime an Austrian disposal draws, as its journal description records it.

    The moving-average pool is global, but Alt/Neu availability stays per
    wallet and moves with self-transfers. A post-cutoff disposal is Neu while
    its wallet holds Neu coins (a Wahlrecht election when Alt is there too)
    and Alt once only Alt remains.
    """

    def test_post_cutoff_sale_with_only_alt_inventory_falls_back_to_alt(self):
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-alt", "A", "inbound", 100_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("sell-later", "A", "outbound", 100_000_000, "2025-06-01T00:00:00Z", rate="90000", kind="sell"),
            ],
        )

        sale = _entry(state, "sell-later", "disposal")
        self.assertEqual(sale["description"], "at_regime=alt at_pool=default sell-later via Cold")
        self.assertEqual(sale["at_category"], "alt_taxfree")

    def test_post_cutoff_sale_with_neu_inventory_stays_neu(self):
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-alt", "A", "inbound", 50_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("buy-neu", "A", "inbound", 100_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="buy"),
                row("sell-later", "A", "outbound", 30_000_000, "2025-06-01T00:00:00Z", rate="90000", kind="sell"),
            ],
        )

        self.assertEqual(
            _description(state, "sell-later", "disposal"),
            "at_regime=neu at_regime_basis=wahlrecht at_pool=default sell-later via Cold",
        )

    def test_post_cutoff_sale_from_alt_only_wallet_stays_alt(self):
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-alt", "A", "inbound", 50_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("buy-neu", "B", "inbound", 100_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="buy"),
                row("sell-from-a", "A", "outbound", 30_000_000, "2025-06-01T00:00:00Z", rate="90000", kind="sell"),
            ],
        )

        self.assertEqual(
            _description(state, "sell-from-a", "disposal"),
            "at_regime=alt at_pool=default sell-from-a via Cold",
        )

    def test_post_cutoff_sale_from_transfer_funded_wallet_draws_moved_neu_inventory(self):
        # bitcoinaustria/kassiber#213: Neu acquired in A, moved to B, then sold
        # from B. The move draws A's Neu coins (no Alt leaves A) and carries
        # them to B.
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-alt", "A", "inbound", 50_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("buy-neu", "A", "inbound", 100_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="buy"),
                row("xfer-out", "A", "outbound", 100_000_000, "2024-07-01T00:00:00Z", rate="60000"),
                row("xfer-in", "B", "inbound", 100_000_000, "2024-07-01T00:10:00Z", rate="60000"),
                row("sell-from-b", "B", "outbound", 30_000_000, "2025-06-01T00:00:00Z", rate="90000", kind="sell"),
            ],
            [pair("pair-1", "xfer-out", "xfer-in")],
        )

        for transaction_id, entry_type in (("xfer-out", "transfer_out"), ("xfer-in", "transfer_in")):
            self.assertEqual(
                _description(state, transaction_id, entry_type),
                "at_regime=neu at_alt_out=0 at_alt_in=0 at_pool=default Transfer Cold -> Hot",
            )
        self.assertEqual(
            _description(state, "sell-from-b", "disposal"),
            "at_regime=neu at_pool=default sell-from-b via Hot",
        )

    def test_one_to_many_transfer_carries_regime_to_each_destination_wallet(self):
        # Custody slices the one source into a move per destination, so each
        # move carries its own share of A's Alt coins.
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-alt", "A", "inbound", 100_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("xfer-out", "A", "outbound", 100_000_000, "2025-01-01T00:00:00Z", rate="90000"),
                row("xfer-in-b", "B", "inbound", 40_000_000, "2025-01-01T00:10:00Z", rate="90000"),
                row("xfer-in-c", "C", "inbound", 60_000_000, "2025-01-01T00:10:00Z", rate="90000"),
                row("sell-from-b", "B", "outbound", 10_000_000, "2025-06-01T00:00:00Z", rate="90000", kind="sell"),
                row("sell-from-c", "C", "outbound", 10_000_000, "2025-06-02T00:00:00Z", rate="90000", kind="sell"),
            ],
            [pair("pair-b", "xfer-out", "xfer-in-b"), pair("pair-c", "xfer-out", "xfer-in-c")],
        )

        self.assertEqual(
            _description(state, "xfer-in-b", "transfer_in"),
            "at_regime=alt at_alt_out=40000000 at_alt_in=40000000 at_pool=default Transfer Cold -> Hot",
        )
        self.assertEqual(
            _description(state, "xfer-in-c", "transfer_in"),
            "at_regime=alt at_alt_out=60000000 at_alt_in=60000000 at_pool=default Transfer Cold -> Exchange",
        )
        self.assertEqual(
            _description(state, "sell-from-b", "disposal"),
            "at_regime=alt at_pool=default sell-from-b via Hot",
        )
        self.assertEqual(
            _description(state, "sell-from-c", "disposal"),
            "at_regime=alt at_pool=default sell-from-c via Exchange",
        )

    def test_self_transfer_fee_is_fee_first_when_neu_covers_fee_only(self):
        # A post-cutoff MOVE from a mixed Alt/Neu wallet uses the preferred Neu
        # slice for the taxable miner fee first. Only the remaining Neu quantity
        # carries to the destination; otherwise a later sale from the destination
        # would see Neu inventory that was already consumed by the fee. B's first
        # sale uses up exactly that remainder, so the next one is Alt.
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-alt", "A", "inbound", 100_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("buy-neu", "A", "inbound", 10_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="buy"),
                row("xfer-out", "A", "outbound", 100_000_000, "2025-01-01T00:00:00Z", fee=5_000_000, rate="90000"),
                row("xfer-in", "B", "inbound", 100_000_000, "2025-01-01T00:10:00Z", rate="90000"),
                row("sell-neu-remainder", "B", "outbound", 5_000_000, "2025-06-01T00:00:00Z", rate="95000", kind="sell"),
                row("sell-after-neu-fee", "B", "outbound", 1_000_000, "2025-06-02T00:00:00Z", rate="95000", kind="sell"),
            ],
            [pair("pair-1", "xfer-out", "xfer-in")],
        )

        self.assertEqual(
            _description(state, "xfer-out", "transfer_fee"),
            "at_regime=neu at_pool=default xfer-out via Cold",
        )
        self.assertEqual(
            _description(state, "xfer-in", "transfer_in"),
            "at_regime=neu at_alt_out=95000000 at_alt_in=95000000 at_pool=default Transfer Cold -> Hot",
        )
        self.assertTrue(
            _description(state, "sell-neu-remainder", "disposal").startswith("at_regime=neu ")
        )
        self.assertEqual(
            _description(state, "sell-after-neu-fee", "disposal"),
            "at_regime=alt at_pool=default sell-after-neu-fee via Hot",
        )


class AustrianKennzahlMappingTest(unittest.TestCase):
    def test_kennzahl_mapping_covers_every_kassiber_category(self):
        self.assertEqual(
            set(AT_CATEGORY_TO_KENNZAHL),
            {
                "income_general",
                "income_capital_yield",
                "neu_gain",
                "neu_loss",
                "neu_swap",
                "alt_spekulation",
                "alt_taxfree",
            },
        )

    def test_maps_known_categories(self):
        self.assertEqual(kennzahl_for_disposal_category("income_general"), 172)
        self.assertEqual(kennzahl_for_disposal_category("income_capital_yield"), 172)
        self.assertEqual(kennzahl_for_disposal_category("neu_gain"), 174)
        self.assertEqual(kennzahl_for_disposal_category("neu_loss"), 176)
        self.assertEqual(kennzahl_for_disposal_category("alt_spekulation"), 801)

    def test_returns_none_for_non_reported_categories(self):
        self.assertIsNone(kennzahl_for_disposal_category("neu_swap"))
        self.assertIsNone(kennzahl_for_disposal_category("alt_taxfree"))
        self.assertIsNone(kennzahl_for_disposal_category(None))


class AustrianEventMarkerTest(unittest.TestCase):
    """The Austrian markers an event's journal description carries.

    Austrian descriptions start with the regime (by the cutoff for an
    acquisition) and the single cost-basis pool, which Austria calls
    ``default``; a swap leg adds its link. Generic books carry none.
    """

    def test_at_inbound_event_gets_neu_regime_and_pool(self):
        state = _ledger(
            MOVING_AVERAGE_AT,
            [row("buy-1", "A", "inbound", 100_000_000, "2025-06-01T00:00:00Z", kind="buy")],
        )

        self.assertEqual(
            _description(state, "buy-1", "acquisition"),
            "at_regime=neu at_pool=default buy-1 via Cold",
        )

    def test_at_inbound_event_pre_cutoff_gets_alt_regime(self):
        state = _ledger(
            MOVING_AVERAGE_AT,
            [row("buy-1", "A", "inbound", 100_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy")],
        )

        self.assertEqual(
            _description(state, "buy-1", "acquisition"),
            "at_regime=alt at_pool=default buy-1 via Cold",
        )

    def test_generic_profile_leaves_at_markers_out(self):
        book = profile("generic", "FIFO")
        state = _ledger(
            book,
            [
                row("buy-1", "A", "inbound", 100_000_000, "2025-06-01T00:00:00Z", kind="buy"),
                row("sell-1", "A", "outbound", 10_000_000, "2025-07-01T00:00:00Z", rate="90000", kind="sell"),
            ],
        )

        self.assertEqual(_description(state, "buy-1", "acquisition"), "buy-1 via Cold")
        sale = _entry(state, "sell-1", "disposal")
        self.assertEqual(sale["description"], "sell-1 via Cold")
        self.assertNotIn("at_category", sale)

    def test_at_swap_link_tags_both_legs_and_carries_the_swapped_basis(self):
        # A reviewed carrying-value pair of a Neu BTC sale and a USDT receipt
        # becomes a swap: both legs carry the link, the disposal is a
        # neutral swap, and the USDT lot takes the BTC's basis (0.5 BTC at
        # 40,000) instead of its own market value (300 USDT at 1).
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-btc", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="40000", kind="buy"),
                row("swap-out", "C", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", fee=1_000_000, rate="60000", kind="sell"),
                row("swap-in", "C", "inbound", 30_000_000_000_000, "2024-06-01T00:00:00Z", rate="1", kind="buy", asset="USDT"),
            ],
            [pair("swap-1", "swap-out", "swap-in")],
        )

        swap_out = _entry(state, "swap-out", "disposal")
        self.assertEqual(
            swap_out["description"],
            "at_regime=neu at_pool=default at_swap_link=swap-1 swap-out via Exchange",
        )
        self.assertEqual(swap_out["at_category"], "neu_swap")
        self.assertEqual(swap_out["gain_loss"], 0)
        swap_in = _entry(state, "swap-in", "acquisition")
        self.assertEqual(
            swap_in["description"],
            "at_regime=neu at_pool=default at_swap_link=swap-1 swap-in via Exchange",
        )
        self.assertEqual(swap_in["fiat_value"], Decimal("20000"))

    def test_at_transfer_uses_the_single_pool(self):
        # Transfers carry the one global pool, not the source wallet's, so a
        # later sale from the destination draws the pool the coins were
        # acquired in; nothing fails as a cross-pool move.
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="40000", kind="buy"),
                row("move-out", "A", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", fee=1_000_000, rate="60000"),
                row("move-in", "B", "inbound", 50_000_000_000, "2024-06-01T00:10:00Z", rate="60000"),
            ],
            [pair("pair-1", "move-out", "move-in")],
        )

        self.assertEqual(state.quarantines, [])
        for transaction_id, entry_type in (
            ("move-out", "transfer_fee"),
            ("move-out", "transfer_out"),
            ("move-in", "transfer_in"),
        ):
            self.assertIn(
                "at_pool=default",
                _description(state, transaction_id, entry_type).split(),
            )


class ATCrossAssetValidationTest(unittest.TestCase):
    """The country's cross-asset check runs before any per-asset compute.

    Two chained swaps whose first hop sells 1 msat more than its wallet
    holds: the first swap cannot be carried, so the USDT that funds the
    second swap never arrives, the second swap's link is left with only its
    inbound leg, and the whole build fails with the validation code.
    """

    def test_unpaired_swap_link_surfaces_as_apperror_with_code(self):
        rows = [
            row("btc-buy", "C", "inbound", 10_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
            row("swap1-out", "C", "outbound", 10_000_000_001, "2024-02-01T00:00:00Z", rate="40000"),
            row("swap1-in", "C", "inbound", 400_000_000_000, "2024-02-01T00:00:00Z", rate="1", asset="USDT"),
            row("swap2-out", "C", "outbound", 400_000_000_000, "2024-03-01T00:00:00Z", rate="1", asset="USDT"),
            row("swap2-in", "L", "inbound", 9_000_000_000, "2024-03-01T00:00:00Z", rate="44000", asset="LBTC"),
        ]

        with self.assertRaises(AppError) as raised:
            _ledger(
                MOVING_AVERAGE_AT,
                rows,
                [pair("s1", "swap1-out", "swap1-in"), pair("s2", "swap2-out", "swap2-in")],
            )

        self.assertEqual(raised.exception.code, "rp2_input_validation")
        self.assertIn("Unpaired `at_swap_link=s2`", str(raised.exception))


class CarryPairPhaseOneBlockJournalTest(unittest.TestCase):
    """Phase-1 blocks on carry legs through the real ``journals process`` path.

    Finalized projection rows have synthetic ``custody-tax:``/``direct-payout:``
    ids while quarantines are keyed by the real anchor transaction. The swap
    selection must resolve that anchor; otherwise an over-sold leg is promoted
    to an ``at_swap_link`` and aborts the whole multi-asset report, and a leg's
    real reason (e.g. a missing price) is replaced by a generic fallback.
    """

    NOW = "2025-01-01T00:00:00Z"

    def _book(self, tmp, *, tax_country="at"):
        from pathlib import Path

        from kassiber.db import open_db

        conn = open_db(Path(tmp) / "data")
        self.addCleanup(conn.close)
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws-1', 'Main', ?)",
            (self.NOW,),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('profile-1', 'ws-1', 'Default', 'EUR', ?, 365, ?, ?)
            """,
            (
                tax_country,
                "moving_average_at" if tax_country == "at" else "FIFO",
                self.NOW,
            ),
        )
        conn.execute(
            """
            INSERT INTO accounts(
                id, workspace_id, profile_id, code, label, account_type, asset, created_at
            ) VALUES('acct-1', 'ws-1', 'profile-1', 'treasury', 'Treasury', 'asset', 'BTC', ?)
            """,
            (self.NOW,),
        )
        for wallet_id, label in (("wallet-btc", "Onchain"), ("wallet-liquid", "Liquid")):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
                ) VALUES(?, 'ws-1', 'profile-1', 'acct-1', ?, 'custom', '{}', ?)
                """,
                (wallet_id, label, self.NOW),
            )
        return conn

    def _tx(self, conn, tx_id, wallet_id, direction, asset, amount_msat, occurred_at, rate):
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, raw_json, created_at
            ) VALUES(?, 'ws-1', 'profile-1', ?, ?, ?, ?, ?, ?, ?, 0, 'EUR', ?, NULL, ?, '{}', ?)
            """,
            (
                tx_id, wallet_id, f"ext-{tx_id}", f"fp-{tx_id}", occurred_at,
                direction, asset, amount_msat, rate,
                "buy" if direction == "inbound" and asset == "BTC" else (
                    "deposit" if direction == "inbound" else "withdrawal"
                ),
                self.NOW,
            ),
        )

    def _process(self, conn):
        from kassiber.cli import handlers

        handlers.process_journals(conn, "Main", "Default")
        quarantines = {
            (row["transaction_id"], row["reason"]): json.loads(row["detail_json"])
            for row in conn.execute(
                "SELECT transaction_id, reason, detail_json FROM journal_quarantines"
            )
        }
        entries = [
            (row["transaction_id"], row["entry_type"], row["asset"])
            for row in conn.execute(
                "SELECT transaction_id, entry_type, asset FROM journal_entries"
            )
        ]
        return quarantines, entries

    def _pair(self, conn):
        from kassiber.cli import handlers

        handlers.create_transaction_pair(
            conn, "Main", "Default", "swap-out", "swap-in",
            kind="peg-in", policy="carrying-value",
        )

    def test_oversold_at_swap_leg_quarantines_pair_instead_of_aborting(self):
        import tempfile

        from kassiber.core.austrian import AT_SWAP_QUARANTINE_REASON

        with tempfile.TemporaryDirectory(prefix="kassiber-at-carry-oversell-") as tmp:
            conn = self._book(tmp)
            self._tx(conn, "btc-in", "wallet-btc", "inbound", "BTC", 10_000_000, "2025-05-01T00:00:00Z", 50_000)
            self._tx(conn, "swap-out", "wallet-btc", "outbound", "BTC", 50_000_000, "2025-06-01T00:00:00Z", 50_000)
            self._tx(conn, "swap-in", "wallet-liquid", "inbound", "LBTC", 50_000_000, "2025-06-01T00:01:00Z", 50_000)
            conn.commit()
            self._pair(conn)

            # Before the anchor lookup this raised "RP2 multi-asset tax
            # calculation failed" and no journal was written at all.
            quarantines, entries = self._process(conn)

        self.assertEqual(
            set(quarantines),
            {
                ("swap-out", AT_SWAP_QUARANTINE_REASON),
                ("swap-in", AT_SWAP_QUARANTINE_REASON),
            },
        )
        for detail in quarantines.values():
            self.assertEqual(detail["reason_code"], "insufficient_lots")
        self.assertEqual(entries, [("btc-in", "acquisition", "BTC")])

    def test_oversold_at_direct_payout_quarantines_source_instead_of_aborting(self):
        import tempfile

        from kassiber.cli import handlers
        from kassiber.core.austrian import AT_SWAP_QUARANTINE_REASON

        with tempfile.TemporaryDirectory(prefix="kassiber-at-payout-oversell-") as tmp:
            conn = self._book(tmp)
            self._tx(conn, "btc-in", "wallet-btc", "inbound", "BTC", 10_000_000, "2025-05-01T00:00:00Z", 50_000)
            self._tx(conn, "swap-out", "wallet-btc", "outbound", "BTC", 50_000_000, "2025-06-01T00:00:00Z", 60_000)
            conn.commit()
            handlers.create_direct_swap_payout(
                conn, "Main", "Default", "swap-out",
                payout_asset="USDT", payout_amount="30", payout_fiat_value="30",
                policy="carrying-value", counterparty="exchange",
            )

            quarantines, entries = self._process(conn)

        # One record for the one real source; the synthetic target-asset
        # settlement disposal is withheld with the pair instead of surfacing a
        # second, misleading insufficient-lots block.
        self.assertEqual(set(quarantines), {("swap-out", AT_SWAP_QUARANTINE_REASON)})
        self.assertEqual(
            quarantines[("swap-out", AT_SWAP_QUARANTINE_REASON)]["reason_code"],
            "insufficient_lots",
        )
        self.assertEqual(entries, [("btc-in", "acquisition", "BTC")])

    def test_unpriced_carry_leg_keeps_its_missing_price_reason(self):
        import tempfile

        from kassiber.core.austrian import AT_SWAP_QUARANTINE_REASON
        from kassiber.core.engines.rp2 import GENERIC_BITCOIN_RAIL_QUARANTINE_REASON

        for tax_country, reason in (
            ("at", AT_SWAP_QUARANTINE_REASON),
            ("generic", GENERIC_BITCOIN_RAIL_QUARANTINE_REASON),
        ):
            with self.subTest(tax_country=tax_country):
                with tempfile.TemporaryDirectory(prefix="kassiber-carry-noprice-") as tmp:
                    conn = self._book(tmp, tax_country=tax_country)
                    self._tx(conn, "btc-in", "wallet-btc", "inbound", "BTC", 100_000_000, "2025-05-01T00:00:00Z", 50_000)
                    # No rate and outside any bundled price history.
                    self._tx(conn, "swap-out", "wallet-btc", "outbound", "BTC", 50_000_000, "2031-06-01T00:00:00Z", None)
                    self._tx(conn, "swap-in", "wallet-liquid", "inbound", "LBTC", 50_000_000, "2031-06-01T00:01:00Z", None)
                    conn.commit()
                    self._pair(conn)

                    quarantines, entries = self._process(conn)

                self.assertEqual(
                    set(quarantines), {("swap-out", reason), ("swap-in", reason)}
                )
                for detail in quarantines.values():
                    self.assertEqual(detail["reason_code"], "missing_spot_price")
                self.assertEqual(entries, [("btc-in", "acquisition", "BTC")])


if __name__ == "__main__":
    unittest.main()
