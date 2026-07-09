"""Exit-tax (deemed-disposal) computation tests.

Validates the core invariants of kassiber/core/exit_tax.py against a hand-built
ledger state (no DB / engine round-trip needed): Altbestand is excluded from the
taxed base, Neubestand is valued at FMV and taxed at 27.5%, transfers never touch
the pool, the EU/EEA vs third-country flag drives only collection timing, and the
generic fallback applies no special rate.
"""

import sqlite3
import unittest
from decimal import Decimal

from kassiber.core import exit_tax


def _conn_with_rate(rate=None, *, timestamp="2026-06-15T12:00:00Z", source="manual"):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    # Mirror the columns get_cached_rate_at_or_before selects.
    conn.execute(
        """
        CREATE TABLE rates_cache (
            pair TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            rate REAL NOT NULL,
            rate_exact TEXT,
            source TEXT NOT NULL,
            fetched_at TEXT NOT NULL,
            granularity TEXT,
            method TEXT
        )
        """
    )
    if rate is not None:
        conn.execute(
            "INSERT INTO rates_cache (pair, timestamp, rate, rate_exact, source, fetched_at) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            ("BTC-EUR", timestamp, float(rate), str(rate), source, timestamp),
        )
    return conn


def _profile(tax_country="at"):
    return {
        "id": "p1",
        "workspace_id": "ws1",
        "workspace_label": "Books",
        "label": "Main",
        "tax_country": tax_country,
        "fiat_currency": "EUR",
    }


def _state():
    # 1.0 BTC Altbestand (pre-2021-03-01), 0.5 BTC Neubestand, less a 0.1 BTC
    # Neu disposal, plus a self-transfer that must not change the pool.
    entries = [
        {
            "entry_type": "acquisition",
            "asset": "BTC",
            "occurred_at": "2020-06-01T00:00:00Z",
            "quantity": Decimal("1.0"),
            "fiat_value": Decimal("8000"),
            "cost_basis": None,
        },
        {
            "entry_type": "acquisition",
            "asset": "BTC",
            "occurred_at": "2022-01-01T00:00:00Z",
            "quantity": Decimal("0.5"),
            "fiat_value": Decimal("20000"),
            "cost_basis": None,
        },
        {
            "entry_type": "disposal",
            "asset": "BTC",
            "occurred_at": "2023-03-01T00:00:00Z",
            "quantity": Decimal("-0.1"),
            "cost_basis": Decimal("4000"),
            "proceeds": Decimal("5000"),
            "gain_loss": Decimal("1000"),
            "at_category": "neu_gain",
        },
        {
            "entry_type": "transfer_out",
            "asset": "BTC",
            "occurred_at": "2024-01-01T00:00:00Z",
            "quantity": Decimal("-0.2"),
            "cost_basis": None,
        },
        {
            "entry_type": "transfer_in",
            "asset": "BTC",
            "occurred_at": "2024-01-01T00:00:00Z",
            "quantity": Decimal("0.2"),
            "cost_basis": None,
        },
    ]
    wallet_holdings = {
        ("w1", "Cold", "treasury", "BTC"): {"quantity": Decimal("1.4"), "cost_basis": Decimal("24000")},
    }
    return {
        "entries": entries,
        "wallet_holdings": wallet_holdings,
        "account_holdings": {},
        "quarantines": [],
        "latest_rates": {"BTC": Decimal("60000")},
    }


class ExitTaxComputeTests(unittest.TestCase):
    def test_altbestand_excluded_neubestand_taxed(self):
        conn = _conn_with_rate(Decimal("60000"))
        report = exit_tax.compute_deemed_disposal(
            conn, _profile("at"), _state(), departure_date="2026-06-16", destination="eu_eea"
        )
        totals = report["totals"]
        # Neubestand: 0.4 BTC remaining, basis 16000, market 24000, gain 8000.
        self.assertEqual(totals["neuQuantitySats"], 40_000_000)
        self.assertEqual(totals["neuCostBasis"], 16000.0)
        self.assertEqual(totals["neuMarketValue"], 24000.0)
        self.assertEqual(totals["neuGain"], 8000.0)
        # Altbestand: 1.0 BTC, valued but excluded from the taxed base.
        self.assertEqual(totals["altQuantitySats"], 100_000_000)
        self.assertEqual(totals["altMarketValue"], 60000.0)
        # Tax = 27.5% of the Neu gain only.
        self.assertEqual(totals["taxableGain"], 8000.0)
        self.assertEqual(totals["estimatedTaxRate"], 0.275)
        self.assertEqual(totals["estimatedTax"], 2200.0)

    def test_lots_classification(self):
        conn = _conn_with_rate(Decimal("60000"))
        report = exit_tax.compute_deemed_disposal(
            conn, _profile("at"), _state(), departure_date="2026-06-16"
        )
        by_regime = {lot["regime"]: lot for lot in report["lots"]}
        self.assertTrue(by_regime["neu"]["taxable"])
        self.assertEqual(by_regime["neu"]["category"], "neu_gain")
        self.assertEqual(by_regime["neu"]["kennzahl"], 174)
        self.assertFalse(by_regime["alt"]["taxable"])
        self.assertEqual(by_regime["alt"]["category"], "alt_taxfree")
        self.assertIsNone(by_regime["alt"]["kennzahl"])

    def test_destination_drives_collection_timing_only(self):
        conn = _conn_with_rate(Decimal("60000"))
        eu = exit_tax.compute_deemed_disposal(
            conn, _profile("at"), _state(), destination="eu_eea"
        )
        third = exit_tax.compute_deemed_disposal(
            conn, _profile("at"), _state(), destination="third-country"
        )
        self.assertEqual(eu["totals"]["collectionTiming"], "deferred")
        self.assertEqual(third["totals"]["collectionTiming"], "immediate")
        # Same liability either way — only the timing differs.
        self.assertEqual(eu["totals"]["estimatedTax"], third["totals"]["estimatedTax"])

    def test_transfers_do_not_touch_the_pool(self):
        # Holdings after the self-transfer are unchanged: 1.4 BTC total.
        conn = _conn_with_rate(Decimal("60000"))
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), _state())
        total_sats = report["totals"]["neuQuantitySats"] + report["totals"]["altQuantitySats"]
        self.assertEqual(total_sats, 140_000_000)

    def test_description_regime_marker_overrides_timestamp_for_acquisition(self):
        conn = _conn_with_rate(Decimal("60000"))
        state = {
            "entries": [
                {
                    "entry_type": "acquisition",
                    "asset": "BTC",
                    "occurred_at": "2024-06-01T00:00:00Z",
                    "quantity": Decimal("1.0"),
                    "fiat_value": Decimal("30000"),
                    "cost_basis": None,
                    "description": "at_regime=alt user override",
                },
            ],
            "wallet_holdings": {},
            "account_holdings": {},
            "quarantines": [],
            "latest_rates": {"BTC": Decimal("60000")},
        }

        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), state)
        totals = report["totals"]

        self.assertEqual(totals["altQuantitySats"], 100_000_000)
        self.assertEqual(totals["neuQuantitySats"], 0)

    def test_description_regime_marker_keeps_alt_transfer_fee_in_alt_pool(self):
        conn = _conn_with_rate(Decimal("60000"))
        state = {
            "entries": [
                {
                    "entry_type": "acquisition",
                    "asset": "BTC",
                    "occurred_at": "2020-06-01T00:00:00Z",
                    "quantity": Decimal("1.0"),
                    "fiat_value": Decimal("8000"),
                    "cost_basis": None,
                    "description": "Alt buy",
                },
                {
                    "entry_type": "transfer_fee",
                    "asset": "BTC",
                    "occurred_at": "2024-01-01T00:00:00Z",
                    "quantity": Decimal("-0.001"),
                    "cost_basis": Decimal("8"),
                    "description": "at_regime=alt at_pool=default Transfer Cold -> Hot",
                },
            ],
            "wallet_holdings": {},
            "account_holdings": {},
            "quarantines": [],
            "latest_rates": {"BTC": Decimal("60000")},
        }

        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), state)
        totals = report["totals"]

        self.assertEqual(totals["altQuantitySats"], 99_900_000)
        self.assertEqual(totals["neuQuantitySats"], 0)

    def test_wallet_holdings_listed_without_regime(self):
        conn = _conn_with_rate(Decimal("60000"))
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), _state())
        self.assertEqual(len(report["walletHoldings"]), 1)
        holding = report["walletHoldings"][0]
        self.assertEqual(holding["wallet"], "Cold")
        self.assertEqual(holding["quantitySats"], 140_000_000)
        self.assertEqual(holding["marketValue"], 84000.0)
        self.assertNotIn("regime", holding)

    def test_fmv_fallback_when_rate_cache_empty(self):
        conn = _conn_with_rate(None)  # empty rates_cache -> latest_rates fallback
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), _state())
        self.assertEqual(report["totals"]["neuMarketValue"], 24000.0)
        self.assertTrue(any(s["source"] == "transaction" for s in report["fmvSource"]))

    def test_neu_loss_clamps_taxable_gain_to_zero(self):
        # Single Neubestand lot underwater: FMV 40k < basis 60k.
        conn = _conn_with_rate(Decimal("40000"))
        state = {
            "entries": [
                {
                    "entry_type": "acquisition",
                    "asset": "BTC",
                    "occurred_at": "2022-05-01T00:00:00Z",
                    "quantity": Decimal("1.0"),
                    "fiat_value": Decimal("60000"),
                    "cost_basis": None,
                },
            ],
            "wallet_holdings": {
                ("w1", "Hot", "treasury", "BTC"): {"quantity": Decimal("1.0"), "cost_basis": Decimal("60000")},
            },
            "account_holdings": {},
            "quarantines": [],
            "latest_rates": {"BTC": Decimal("40000")},
        }
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), state)
        totals = report["totals"]
        self.assertEqual(totals["neuGain"], -20000.0)
        self.assertEqual(totals["taxableGain"], 0.0)
        self.assertEqual(totals["estimatedTax"], 0.0)
        neu = next(lot for lot in report["lots"] if lot["regime"] == "neu")
        self.assertEqual(neu["category"], "neu_loss")
        self.assertEqual(neu["kennzahl"], 176)

    def test_missing_rate_leaves_lots_unpriced_and_flags_it(self):
        conn = _conn_with_rate(None)  # empty cache
        state = {**_state(), "latest_rates": {}}  # and no transaction fallback
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), state)
        neu = next(lot for lot in report["lots"] if lot["regime"] == "neu")
        self.assertIsNone(neu["marketValue"])
        self.assertIsNone(neu["gain"])
        self.assertTrue(any(source["source"] == "missing" for source in report["fmvSource"]))
        self.assertTrue(any("No cached rate" in note for note in report["assumptions"]))

    def test_income_surfaces_derived_tokens_assumption(self):
        conn = _conn_with_rate(Decimal("60000"))
        state = {
            **_state(),
            "entries": _state()["entries"]
            + [
                {
                    "entry_type": "income",
                    "asset": "BTC",
                    "occurred_at": "2023-06-01T00:00:00Z",
                    "quantity": Decimal("0.05"),
                    "cost_basis": Decimal("0"),
                    "at_category": "income_general",
                },
            ],
        }
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), state)
        self.assertTrue(any("EXIT-004" in note for note in report["assumptions"]))
        # Income alone must not inflate inventory (no matching acquisition lot).
        baseline = exit_tax.compute_deemed_disposal(conn, _profile("at"), _state())
        self.assertEqual(
            report["totals"]["neuQuantitySats"],
            baseline["totals"]["neuQuantitySats"],
        )
        self.assertEqual(
            report["totals"]["altQuantitySats"],
            baseline["totals"]["altQuantitySats"],
        )

    def test_income_does_not_double_count_matching_acquisition_lot(self):
        # RP2 books earn receipts as acquisition + income with the same quantity.
        # Exit tax must follow holdings and skip the income recognition line.
        conn = _conn_with_rate(Decimal("60000"))
        state = {
            "entries": [
                {
                    "entry_type": "acquisition",
                    "asset": "BTC",
                    "occurred_at": "2024-01-01T00:00:00Z",
                    "quantity": Decimal("0.001"),
                    "fiat_value": Decimal("40"),
                    "cost_basis": None,
                },
                {
                    "entry_type": "acquisition",
                    "asset": "BTC",
                    "occurred_at": "2024-06-01T00:00:00Z",
                    "quantity": Decimal("0.001"),
                    "fiat_value": Decimal("50"),
                    "cost_basis": None,
                },
                {
                    "entry_type": "income",
                    "asset": "BTC",
                    "occurred_at": "2024-06-01T00:00:00Z",
                    "quantity": Decimal("0.001"),
                    "cost_basis": Decimal("0"),
                    "at_category": "income_capital_yield",
                },
            ],
            "wallet_holdings": {
                ("w1", "Hot", "treasury", "BTC"): {
                    "quantity": Decimal("0.002"),
                    "cost_basis": Decimal("90"),
                },
            },
            "account_holdings": {},
            "quarantines": [],
            "latest_rates": {"BTC": Decimal("60000")},
        }
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), state)
        totals = report["totals"]
        self.assertEqual(totals["neuQuantitySats"], 200_000)
        self.assertEqual(totals["neuCostBasis"], 90.0)
        self.assertEqual(totals["neuMarketValue"], 120.0)
        self.assertEqual(totals["neuGain"], 30.0)
        self.assertEqual(totals["estimatedTax"], 8.25)
        self.assertTrue(any("EXIT-004" in note for note in report["assumptions"]))

    def test_historical_departure_ignores_future_cache_rate(self):
        # P1 guard: the cache only has a rate AFTER the departure date; the FMV
        # lookup must NOT use it (no transaction fallback => unpriced).
        conn = _conn_with_rate(Decimal("99999"), timestamp="2026-06-15T12:00:00Z")
        state = {
            "entries": [
                {
                    "entry_type": "acquisition",
                    "asset": "BTC",
                    "occurred_at": "2021-04-01T00:00:00Z",
                    "quantity": Decimal("1.0"),
                    "fiat_value": Decimal("30000"),
                    "cost_basis": None,
                },
            ],
            "wallet_holdings": {
                ("w1", "Hot", "treasury", "BTC"): {"quantity": Decimal("1.0"), "cost_basis": Decimal("30000")},
            },
            "account_holdings": {},
            "quarantines": [],
            "latest_rates": {},  # no transaction fallback either
        }
        report = exit_tax.compute_deemed_disposal(
            conn, _profile("at"), state, departure_date="2021-05-01"
        )
        neu = next(lot for lot in report["lots"] if lot["regime"] == "neu")
        self.assertIsNone(neu["marketValue"])  # not valued at the future 99999 rate
        self.assertTrue(any(source["source"] == "missing" for source in report["fmvSource"]))

    def test_manual_rate_wins_over_later_provider_at_same_timestamp(self):
        # P2 guard: a reviewed manual override must beat a later-fetched provider
        # row at the same timestamp.
        conn = _conn_with_rate(None)
        ts = "2026-06-15T00:00:00Z"
        conn.execute(
            "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) "
            "VALUES('BTC-EUR',?,50000,'50000','manual',?)",
            (ts, "2026-06-15T08:00:00Z"),
        )
        conn.execute(
            "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) "
            "VALUES('BTC-EUR',?,90000,'90000','coinbase',?)",
            (ts, "2026-06-15T09:00:00Z"),  # fetched later than the manual row
        )
        conn.commit()
        report = exit_tax.compute_deemed_disposal(
            conn, _profile("at"), _state(), departure_date="2026-06-16"
        )
        # Neu 0.4 BTC valued at the manual 50000 (=20000), not the provider 90000.
        self.assertEqual(report["totals"]["neuMarketValue"], 20000.0)

    def test_generic_profile_has_no_special_rate(self):
        conn = _conn_with_rate(Decimal("60000"))
        report = exit_tax.compute_deemed_disposal(conn, _profile("generic"), _state())
        self.assertEqual(report["jurisdictionCode"], "generic")
        self.assertIsNone(report["totals"]["estimatedTaxRate"])
        self.assertIsNone(report["totals"]["estimatedTax"])
        # No Altbestand grandfathering: everything pools as one taxable bucket.
        self.assertEqual(report["totals"]["altQuantitySats"], 0)
        self.assertEqual(report["totals"]["neuQuantitySats"], 140_000_000)

    def test_compute_does_not_mutate_state(self):
        conn = _conn_with_rate(Decimal("60000"))
        state = _state()
        before = len(state["entries"])
        exit_tax.compute_deemed_disposal(conn, _profile("at"), state)
        self.assertEqual(len(state["entries"]), before)

    def test_payload_matches_frozen_contract_keys(self):
        # Pins the camelCase contract shared by the daemon, the GUI, and the TS
        # mock fixture. Adding a field here means updating ui-tauri/src/mocks/exitTax.ts.
        conn = _conn_with_rate(Decimal("60000"))
        report = exit_tax.compute_deemed_disposal(conn, _profile("at"), _state())
        self.assertEqual(
            set(report.keys()),
            {
                "workspace", "profile", "jurisdictionCode", "fiatCurrency",
                "departureDate", "destination", "method", "fmvSource", "totals",
                "lots", "walletHoldings", "assumptions", "reviewGate", "status",
            },
        )
        self.assertEqual(
            set(report["totals"].keys()),
            {
                "neuQuantitySats", "neuMarketValue", "neuCostBasis", "neuGain",
                "altQuantitySats", "altMarketValue", "taxableGain",
                "estimatedTaxRate", "estimatedTax", "collectionTiming",
            },
        )
        self.assertEqual(
            set(report["lots"][0].keys()),
            {"asset", "regime", "quantitySats", "marketValue", "costBasis", "gain", "taxable", "category", "kennzahl"},
        )


class _FakeHooks:
    def __init__(self, profile, state):
        self._profile = profile
        self._state = state

    def resolve_scope(self, conn, workspace_ref, profile_ref):
        return ({"label": "Books", "id": "ws1"}, self._profile)

    def require_processed_journals(self, conn, profile):
        return None

    def build_ledger_state(self, conn, profile):
        return self._state


class ExitTaxReportLinesTests(unittest.TestCase):
    def test_plain_lines_render_headline_and_review_gate(self):
        conn = _conn_with_rate(Decimal("60000"))
        hooks = _FakeHooks(_profile("at"), _state())
        lines = exit_tax.build_exit_tax_report_lines(
            conn, None, None, hooks, departure_date="2026-06-16", destination="eu_eea"
        )
        text = "\n".join(lines)
        self.assertIn("Estimated exit tax:", text)
        self.assertIn("2,200.00 EUR", text)
        self.assertIn("Altbestand", text)
        self.assertIn("deferred until you sell", text)
        self.assertIn("not tax advice", text.lower())


class ExitTaxEngineIntegrationTests(unittest.TestCase):
    """End-to-end through the REAL RP2 engine.

    The other tests feed hand-built `state` dicts; these seed an actual book,
    run the real journal processing, and (a) assert the exit-tax headline and
    (b) pin the engine→exit-tax entry contract so a future RP2 change to
    entry_type/at_category fails here instead of silently skewing a tax number.
    """

    def _seed_book(self):
        import shutil
        import tempfile
        from pathlib import Path

        from kassiber.db import open_db
        from kassiber.core import accounts as core_accounts
        from kassiber.core import wallets as core_wallets

        tmp = tempfile.mkdtemp(prefix="kassiber-exit-tax-it-")
        self.addCleanup(shutil.rmtree, tmp, ignore_errors=True)
        data_root = str(Path(tmp) / "data")
        conn = open_db(data_root)
        self.addCleanup(conn.close)

        core_accounts.create_workspace(conn, "MB")
        core_accounts.create_profile(conn, "MB", "Dep", "EUR", "MOVING_AVERAGE_AT", "at", 365)
        core_wallets.create_wallet(
            conn,
            "MB",
            "Dep",
            "Cold",
            "address",
            account_ref="treasury",
            config={"addresses": ["bc1qexampledummyaddressxxxxxxxxxxxxxxxxx"]},
        )

        profile = conn.execute("SELECT * FROM profiles WHERE label = 'Dep'").fetchone()
        wallet = conn.execute("SELECT id FROM wallets WHERE label = 'Cold'").fetchone()

        def tx(tid, occurred_at, amount_msat, rate):
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                    occurred_at, direction, asset, amount, fee, fiat_currency,
                    fiat_rate, fiat_value, kind, description, excluded, raw_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    tid, profile["workspace_id"], profile["id"], wallet["id"], tid,
                    f"fp-{tid}", occurred_at, "inbound", "BTC", amount_msat, 0, "EUR",
                    rate, None, "deposit", tid, 0, "{}", occurred_at,
                ),
            )

        tx("alt-buy", "2020-06-01T00:00:00Z", 100_000_000_000, 8000)  # 1.0 BTC @ 8000 (Alt)
        tx("neu-buy", "2022-01-01T00:00:00Z", 50_000_000_000, 40000)  # 0.5 BTC @ 40000 (Neu)
        conn.execute(
            "INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at) "
            "VALUES('BTC-EUR', '2026-06-15T00:00:00Z', 60000.0, 'manual', '2026-06-15T00:00:00Z')"
        )
        conn.commit()
        return conn, profile

    def test_real_engine_exit_tax_headline(self):
        conn, _ = self._seed_book()
        from kassiber.cli import handlers
        from kassiber.core import reports as core_reports

        handlers.process_journals(conn, "MB", "Dep")
        report = core_reports.report_exit_tax(
            conn, "MB", "Dep", handlers._report_hooks(),
            departure_date="2026-06-16", destination="eu_eea",
        )
        totals = report["totals"]
        # 0.5 BTC Neu @ 60k = 30k market, basis 20k => 10k gain; tax 27.5% = 2750.
        self.assertEqual(report["jurisdictionCode"], "AT")
        self.assertEqual(totals["neuGain"], 10000.0)
        self.assertEqual(totals["taxableGain"], 10000.0)
        self.assertEqual(totals["estimatedTax"], 2750.0)
        self.assertEqual(totals["collectionTiming"], "deferred")
        # 1.0 BTC Altbestand valued but excluded from the taxed base.
        self.assertEqual(totals["altQuantitySats"], 100_000_000)
        self.assertEqual(totals["altMarketValue"], 60000.0)
        by_regime = {lot["regime"]: lot for lot in report["lots"]}
        self.assertEqual(by_regime["neu"]["kennzahl"], 174)
        self.assertFalse(by_regime["alt"]["taxable"])

    def test_engine_entry_types_are_all_recognized(self):
        # Drift guard: the real engine must only emit entry_type / at_category
        # values the exit-tax walk reasons about. A new RP2 type fails here.
        conn, profile = self._seed_book()
        from kassiber.cli import handlers

        state = handlers.build_ledger_state(conn, profile)
        entry_types = {str(e.get("entry_type")) for e in state["entries"]}
        unexpected_entry_types = entry_types - exit_tax.RECOGNIZED_ENTRY_TYPES
        self.assertSetEqual(
            unexpected_entry_types,
            set(),
            msg=f"RP2 emitted unrecognized entry_type(s) {unexpected_entry_types}; "
            "update kassiber/core/exit_tax.py RECOGNIZED_ENTRY_TYPES and the walk.",
        )
        for entry in state["entries"]:
            category = entry.get("at_category")
            if category is not None:
                self.assertTrue(
                    str(category).startswith(("alt", "neu", "income")),
                    msg=f"Unexpected at_category prefix '{category}' — exit-tax regime attribution "
                    "assumes alt/neu/income prefixes.",
                )


if __name__ == "__main__":
    unittest.main()
