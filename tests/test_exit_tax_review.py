"""Exit-tax review scenarios through journal processing and the pinned real RP2."""

import json
from decimal import Decimal

import pytest
import openpyxl

from kassiber.cli import handlers
from kassiber.core import accounts, custody_review_terms, reports, ui_snapshot, wallets
from kassiber.core.austrian import infer_regime_from_timestamp
from kassiber.core.journal_markers import MARKER_REGIME, parse_marker
from kassiber.db import open_db


BTC = 100_000_000_000
METHODS = ("fifo", "lifo", "hifo", "lofo", "moving_average", "moving_average_at")


@pytest.fixture
def engine_book(tmp_path):
    connections = []

    def build(rows, *, method="fifo", country="at", quotes=(), pair=False, overrides=None):
        conn = open_db(tmp_path / f"engine-{len(connections)}")
        connections.append(conn)
        accounts.create_workspace(conn, "W")
        accounts.create_profile(conn, "W", "P", "EUR", method, country, 365)
        for label in ("Cold", "Hot"):
            wallets.create_wallet(conn, "W", "P", label, "custom")
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        wallet_ids = {r["label"]: r["id"] for r in conn.execute("SELECT id,label FROM wallets")}
        for tid, wallet, at, direction, amount, fee, rate, kind in rows:
            conn.execute(
                """INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,
                    external_id,fingerprint,occurred_at,direction,asset,amount,fee,
                    fiat_currency,fiat_rate,fiat_rate_exact,kind,description,raw_json,created_at)
                    VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
                (tid, profile["workspace_id"], profile["id"], wallet_ids[wallet], tid,
                 "fp-" + tid, at, direction, "BTC", amount, fee, "EUR", rate,
                 str(rate) if rate is not None else None, kind, tid, "{}", at),
            )
        for tid, fields in (overrides or {}).items():
            for key, value in fields.items():
                assert key in {"at_category_override", "taxability_override"}
                conn.execute(f"UPDATE transactions SET {key}=? WHERE id=?", (value, tid))
        for at, rate in quotes:
            conn.execute(
                """INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at)
                    VALUES('BTC-EUR',?,?,'manual',?)""", (at, rate, at),
            )
        conn.commit()
        if pair:
            custody_review_terms.create_pair_review(
                conn, workspace_id=profile["workspace_id"], profile=profile,
                out_row=conn.execute("SELECT * FROM transactions WHERE id='move-out'").fetchone(),
                in_row=conn.execute("SELECT * FROM transactions WHERE id='move-in'").fetchone(),
            )
        handlers.process_journals(conn, "W", "P")
        assert list(conn.execute("SELECT * FROM journal_quarantines")) == []
        return conn

    yield build
    for conn in connections:
        conn.close()


def regime_rows():
    # The review's complete past-departure scenario includes the earned 0.01
    # and the offset-dated 0.05 receipt. The fractional-second edge is separate.
    return [
        ("alt", "Cold", "2020-06-01T00:00:00Z", "inbound", BTC, 0, 8000, "deposit"),
        ("neu1", "Cold", "2022-01-01T00:00:00Z", "inbound", BTC // 2, 0, 40000, "deposit"),
        ("neu2", "Hot", "2023-01-01T00:00:00+05:00", "inbound", 3 * BTC // 10, 0, 15000, "deposit"),
        ("move-out", "Cold", "2023-06-01T00:00:00Z", "outbound", BTC // 5, 10_000_000, 25000, "withdrawal"),
        ("move-in", "Hot", "2023-06-01T00:10:00Z", "inbound", BTC // 5, 0, 25000, "deposit"),
        ("sell", "Hot", "2023-09-01T00:00:00Z", "outbound", BTC // 4, 20_000_000, 24000, "withdrawal"),
        ("earn", "Cold", "2024-01-15T00:00:00Z", "inbound", BTC // 100, 0, 38000, "staking"),
        ("offset", "Hot", "2024-04-01T01:30:00+02:00", "inbound", BTC // 20, 0, 60000, "deposit"),
    ]


@pytest.mark.parametrize("country", ("at", "generic"))
@pytest.mark.parametrize("method", METHODS)
def test_regime_inventory_matches_realized_lots_and_dashboard(engine_book, method, country):
    if country == "generic" and method == "moving_average_at":
        # This is deliberately not an available generic-country method.
        from kassiber.tax_policy import build_tax_policy
        assert method not in build_tax_policy({"tax_country": country}).accounting_methods
        return
    conn = engine_book(regime_rows(), method=method, country=country, pair=True,
                       quotes=[("2024-03-31T12:00:00Z", 65000)])
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(),
                                     departure_date="2024-03-31")
    realized = reports.report_capital_gains(conn, "W", "P", handlers._report_hooks())
    entries = list(conn.execute("SELECT * FROM journal_entries"))
    remaining = {"alt": Decimal(0), "neu": Decimal(0)}
    basis = {"alt": Decimal(0), "neu": Decimal(0)}
    for entry in entries:
        if entry["entry_type"] != "acquisition":
            continue
        regime = infer_regime_from_timestamp(entry["occurred_at"]) if country == "at" else "neu"
        remaining[regime] += entry["quantity"]
        basis[regime] += Decimal(entry["fiat_value_exact"])
    for sale in realized:
        if sale["entry_type"] != "disposal":
            continue
        regime = "alt" if sale.get("at_category", "").startswith("alt") else "neu"
        remaining[regime] -= sale["quantity_msat"]
        basis[regime] -= Decimal(str(sale["cost_basis"]))
    # Fees are intentionally absent from the realized capital-gains report.
    # Verify their stored regime against RP2's actual matched acquisition lots,
    # independently of the marker used by the exit-tax report.
    for entry in entries:
        if entry["entry_type"] not in ("fee", "transfer_fee"):
            continue
        regimes = set()
        for fragment in json.loads(entry["calculation_json"])["fragments"]:
            regime = infer_regime_from_timestamp(fragment["lot"]["occurred_at"]) if country == "at" else "neu"
            regimes.add(regime)
            remaining[regime] -= Decimal(fragment["quantity_msat_exact"])
            basis[regime] -= Decimal(fragment["cost_basis_exact"])
        if country == "at":
            assert regimes == {parse_marker(entry["description"], MARKER_REGIME)}
        assert entry["at_category"] is None  # fee removal is not a taxable disposal
        assert Decimal(entry["gain_loss_exact"]) == 0
    totals = report["totals"]
    assert totals["altQuantitySats"] == remaining["alt"] / 1000
    assert totals["neuQuantitySats"] == remaining["neu"] / 1000
    assert totals["neuCostBasis"] == float(basis["neu"].quantize(Decimal("0.01")))
    if country == "at":
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        tile = ui_snapshot._tax_free_balance_snapshot(conn, profile, {})
        assert tile["taxFreeQuantitySats"] == totals["altQuantitySats"]
        assert tile["taxableQuantitySats"] == totals["neuQuantitySats"]
        if method in ("fifo", "lofo", "moving_average"):
            assert totals["altQuantitySats"] == 74_970_000
            assert totals["neuQuantitySats"] == 86_000_000
            assert totals["neuCostBasis"] == 27880.00
            assert totals["neuGain"] == 28020.00
            assert totals["estimatedTax"] == 7705.50


def test_reviewer_headline_also_accounts_for_the_consumed_alt_fee(engine_book):
    conn = engine_book(regime_rows(), pair=True, quotes=[("2024-03-31T12:00:00Z", 65000)])
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-03-31")
    assert report["totals"]["estimatedTax"] == 7705.50
    # Pin the review's exact EUR 3,782.05 -> EUR 7,703.93 comparison using
    # this real engine's sale and fee basis. The latter is the intermediate
    # category-only correction: it still removes the Alt fee from Neu.
    sale = conn.execute("SELECT * FROM journal_entries WHERE entry_type='disposal'").fetchone()
    fee = conn.execute("SELECT * FROM journal_entries WHERE entry_type='transfer_fee'").fetchone()
    assert sale["at_category"] == "alt_taxfree"
    assert parse_marker(sale["description"], MARKER_REGIME) == "alt"
    assert fee["quantity"] == -10_000_000
    assert Decimal(fee["cost_basis_exact"]) == Decimal("0.80")
    legacy_neu_qty = Decimal("0.86") + Decimal(sale["quantity"] + fee["quantity"]) / BTC
    legacy_neu_basis = Decimal("27880") - Decimal(sale["cost_basis_exact"]) - Decimal(fee["cost_basis_exact"])
    legacy_tax = (Decimal(65000) * legacy_neu_qty - legacy_neu_basis) * Decimal("0.275")
    assert legacy_tax.quantize(Decimal("0.01")) == Decimal("3782.05")
    category_only_tax = ((Decimal("0.8599") * 65000) - Decimal("27879.20")) * Decimal("0.275")
    assert category_only_tax.quantize(Decimal("0.01")) == Decimal("7703.93")


@pytest.mark.parametrize("country", ("at", "generic"))
def test_old_fee_journals_require_one_rebuild_without_rewriting_evidence(engine_book, tmp_path, country):
    from kassiber.db import AT_OUTFLOW_REGIME_SEMANTICS_MIGRATION
    from kassiber.errors import AppError

    conn = engine_book(regime_rows(), country=country, pair=True)
    old_profile = dict(conn.execute("SELECT * FROM profiles").fetchone())
    retained = [tuple(r) for r in conn.execute("SELECT * FROM journal_entries ORDER BY id")]
    conn.execute("DELETE FROM schema_migration_audits WHERE migration_name=?", (AT_OUTFLOW_REGIME_SEMANTICS_MIGRATION,))
    conn.commit()
    conn.close()
    for _ in range(2):
        with open_db(tmp_path / "engine-0") as reopened:
            profile = dict(reopened.execute("SELECT * FROM profiles").fetchone())
            assert profile["journal_input_version"] == old_profile["journal_input_version"] + (country == "at")
            assert [tuple(r) for r in reopened.execute("SELECT * FROM journal_entries ORDER BY id")] == retained
            if country == "at":
                with pytest.raises(AppError, match="fresh journals"):
                    reports.report_exit_tax(reopened, "W", "P", handlers._report_hooks())
            else:
                assert profile["last_processed_at"] == old_profile["last_processed_at"]
        reopened.close()


@pytest.mark.parametrize("quote_source", ("cache", "transaction"))
@pytest.mark.parametrize("last_instant", ("2024-03-31T23:59:59.500000Z", "2024-03-31T23:59:59.999999Z"))
def test_full_departure_day_includes_fractional_inventory_and_quotes(engine_book, quote_source, last_instant):
    rows = [
        ("buy", "Cold", "2022-01-01T00:00:00Z", "inbound", BTC, 0, 30000, "buy"),
        ("last", "Cold", last_instant, "inbound", BTC // 1000, 0, 61000, "buy"),
        ("next", "Cold", "2024-04-01T02:00:00+02:00", "inbound", BTC, 0, 90000, "buy"),
    ]
    quotes = [(last_instant, 65000), ("2024-04-01T00:00:00Z", 95000)] if quote_source == "cache" else []
    conn = engine_book(rows, quotes=quotes)
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-03-31")
    assert report["totals"]["neuQuantitySats"] == 100_100_000
    assert report["walletHoldings"][0]["quantitySats"] == 100_100_000
    assert report["fmvSource"] == [{
        "asset": "BTC", "rate": 65000 if quote_source == "cache" else 61000,
        "pair": "BTC-EUR", "asOf": last_instant, "source": quote_source,
    }]


@pytest.mark.parametrize(("quote_at", "priced"), [
    ("2021-06-01T00:00:00Z", False),
    ("2024-12-30T23:59:59.999999Z", False),  # 24h + 1 microsecond
    ("2024-12-31T00:00:00Z", True),          # exactly 24h
    ("2024-12-31T23:59:59.500000Z", True),
])
def test_transaction_fmv_age_limit_through_real_engine(engine_book, quote_at, priced):
    conn = engine_book([("buy", "Cold", quote_at, "inbound", BTC, 0, 30000, "buy")])
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-12-31")
    quote = report["fmvSource"][0]
    assert quote["source"] == ("transaction" if priced else "missing")
    assert quote["asOf"] == (quote_at if priced else None)
    assert report["status"]["missingFmvAssets"] == ([] if priced else ["BTC"])
    for field in ("neuMarketValue", "neuGain", "taxableGain", "estimatedTax"):
        assert (report["totals"][field] is not None) == priced
    if not priced:
        assert "Incomplete estimate: missing FMV for BTC" in reports.format_exit_tax_lines(report)


@pytest.mark.parametrize("quote_source", ("cache", "transaction"))
def test_handoffs_always_show_fmv_source_and_actual_quote_time(engine_book, tmp_path, quote_source):
    at = "2024-12-31T12:34:56Z"
    conn = engine_book([("buy", "Cold", at, "inbound", BTC, 0, 30000, "buy")],
                       quotes=[(at, 65000)] if quote_source == "cache" else [])
    pdf = tmp_path / "exit.pdf"
    xlsx = tmp_path / "exit.xlsx"
    reports.export_exit_tax_pdf_report(conn, "W", "P", pdf, handlers._report_hooks(), departure_date="2024-12-31")
    reports.export_exit_tax_xlsx_report(conn, "W", "P", xlsx, handlers._report_hooks(), departure_date="2024-12-31")
    pdf_text = pdf.read_bytes().decode("latin1")
    assert f"source: {quote_source}" in pdf_text
    assert f"as of: {at}" in pdf_text
    workbook = openpyxl.load_workbook(xlsx)
    try:
        values = [cell.value for row in workbook["FMV quotes"] for cell in row]
        assert quote_source in values
        assert at in values
    finally:
        workbook.close()


@pytest.mark.parametrize("country", ("at", "generic"))
def test_unpriced_wallet_only_rail_keeps_all_valuation_totals_incomplete(engine_book, country):
    conn = engine_book([
        ("buy", "Cold", "2024-12-31T12:00:00Z", "inbound", BTC, 0, 30000, "buy"),
    ], country=country)
    # Exercise the supported retained wallet-context projection edge over an
    # otherwise real RP2 journal. Modern cross-asset swaps also emit acquisition
    # lots, so they cannot create a wallet-only rail for this mapping test.
    conn.execute("""INSERT INTO journal_entries(id,workspace_id,profile_id,
        transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,created_at)
        SELECT 'rail',workspace_id,profile_id,'buy',id,'2024-12-31T13:00:00Z',
            'transfer_in','LBTC',20000000000,'now' FROM wallets WHERE label='Hot'""")
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-12-31")
    assert any(row["asset"] == "LBTC" and row["marketValue"] is None for row in report["walletHoldings"])
    assert all(lot["asset"] != "LBTC" for lot in report["lots"])
    assert report["status"]["missingFmvAssets"] == ["LBTC"]
    for field in ("neuMarketValue", "altMarketValue", "neuGain", "taxableGain", "estimatedTax"):
        assert report["totals"][field] is None


@pytest.mark.parametrize("format", ("pdf", "xlsx"))
def test_handoff_stamps_current_input_version_and_quotes_without_snapshot(engine_book, tmp_path, format):
    at = "2024-12-31T12:34:56Z"
    conn = engine_book([("buy", "Cold", at, "inbound", BTC, 0, 30000, "buy")], quotes=[(at, 65000)])
    exporter = getattr(reports, f"export_exit_tax_{format}_report")
    versions = []
    for revision in range(2):
        if revision:
            conn.execute("UPDATE profiles SET journal_input_version=journal_input_version+1")
            conn.commit()
            handlers.process_journals(conn, "W", "P")
        version = conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0]
        report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-12-31")
        assert report["journalInputVersion"] == version
        artifact = tmp_path / f"exit-{revision}.{format}"
        exporter(conn, "W", "P", artifact, handlers._report_hooks(), departure_date="2024-12-31")
        if format == "pdf":
            text = artifact.read_bytes().decode("latin1")
            assert f"Journal input version: {version}" in text
            assert "source: cache" in text and f"as of: {at}" in text
        else:
            workbook = openpyxl.load_workbook(artifact)
            try:
                overview = {row[0].value: row[1].value for row in workbook["Overview"]}
                assert overview["Journal input version"] == version
                quotes = [cell.value for row in workbook["FMV quotes"] for cell in row]
                assert at in quotes and "cache" in quotes
            finally:
                workbook.close()
        versions.append(version)
        assert conn.execute("SELECT COUNT(*) FROM filed_report_snapshots").fetchone()[0] == 0
        assert conn.execute("SELECT COUNT(*) FROM filed_report_chain_captures").fetchone()[0] == 0
    assert versions[1] == versions[0] + 1


# Independent review simulator figures; these must not be computed from the
# report's own categories or its consumption projection.
REVIEW_TAX = dict(zip(METHODS, (7706.60, 4264.98, 5985.79, 7706.60, 7706.60, 5340.48)))
REVIEW_GAIN = dict(zip(METHODS, (70756.90, 72509.00, 78766.50, 70756.90, 73273.81)))
DISPOSAL_OVERRIDES = (
    {}, {"at_category_override": "alt_taxfree"},
    {"at_category_override": "neu_gain"}, {"taxability_override": 0},
    {"at_category_override": "alt_taxfree", "taxability_override": 0},
    {"at_category_override": "none"},
)


def reviewer_rows():
    return regime_rows() + [
        ("late", "Cold", "2024-03-31T23:30:00-01:00", "outbound", BTC // 10, 0, 70000, "withdrawal"),
        ("fraction", "Cold", "2024-03-31T23:59:59.500000Z", "inbound", BTC // 1000, 0, 61000, "deposit"),
        ("future-buy", "Hot", "2025-01-01T00:00:00Z", "inbound", BTC, 0, 90000, "deposit"),
        ("future-sell", "Cold", "2025-02-01T00:00:00Z", "outbound", BTC // 2, 0, 95000, "withdrawal"),
    ]


@pytest.mark.parametrize("country", ("at", "generic"))
@pytest.mark.parametrize("method", METHODS)
@pytest.mark.parametrize("override", DISPOSAL_OVERRIDES)
def test_reviewer_method_jurisdiction_override_matrix(engine_book, method, country, override):
    if country == "generic" and method == "moving_average_at":
        from kassiber.tax_policy import build_tax_policy
        assert method not in build_tax_policy({"tax_country": country}).accounting_methods
        return
    conn = engine_book(reviewer_rows(), method=method, country=country, pair=True,
                       overrides={"sell": override}, quotes=[("2024-03-31T12:00:00Z", 65000)])
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-03-31")
    assert report["totals"]["estimatedTax" if country == "at" else "neuGain"] == (
        REVIEW_TAX if country == "at" else REVIEW_GAIN)[method]
    for row in conn.execute("SELECT * FROM journal_entries WHERE entry_type='disposal'"):
        if country == "at":
            regimes = {infer_regime_from_timestamp(f["lot"]["occurred_at"])
                       for f in json.loads(row["calculation_json"])["fragments"]}
            assert regimes == {parse_marker(row["description"], MARKER_REGIME)}
        if row["transaction_id"] == "sell" and country == "at":
            # Overrides still change realized tax treatment.
            category = override.get("at_category_override")
            if category and category != "none":
                assert row["at_category"] == category
            elif category == "none" or override.get("taxability_override") == 0:
                assert row["at_category"] is None


@pytest.mark.parametrize("category", ("neu_gain", "alt_taxfree", "none"))
def test_disposal_override_keeps_separate_consumed_regime_rows(engine_book, category):
    conn = engine_book([
        ("alt", "Cold", "2020-01-01T00:00:00Z", "inbound", BTC // 10, 0, 8000, "buy"),
        ("neu", "Cold", "2022-01-01T00:00:00Z", "inbound", BTC // 2, 0, 40000, "buy"),
        ("sell", "Cold", "2024-12-31T12:00:00Z", "outbound", 3 * BTC // 10, 0, 65000, "sell"),
    ], overrides={"sell": {"at_category_override": category}})
    sales = list(conn.execute("SELECT * FROM journal_entries WHERE entry_type='disposal'"))
    assert len(sales) == 2
    assert {parse_marker(r["description"], MARKER_REGIME):
            (r["quantity"], Decimal(r["cost_basis_exact"])) for r in sales} == {
        "alt": (-BTC // 10, Decimal(800)), "neu": (-BTC // 5, Decimal(8000)),
    }
    assert {r["at_category"] for r in sales} == {None if category == "none" else category}
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-12-31")
    assert report["totals"]["altQuantitySats"] == 0
    assert report["totals"]["neuQuantitySats"] == 30_000_000
    assert report["totals"]["neuCostBasis"] == 12000
    assert report["totals"]["estimatedTax"] == 2062.50


@pytest.mark.parametrize("fees", (False, True))
def test_at_outflow_semantics_upgrade_includes_disposals_without_fees(engine_book, tmp_path, fees):
    AT_OUTFLOW_REGIME_SEMANTICS_MIGRATION = "at-outflow-consumed-regime-v2"
    from kassiber.errors import AppError
    rows = regime_rows()
    if not fees:
        rows = [(*r[:5], 0, *r[6:]) for r in rows]
    conn = engine_book(rows, pair=True)
    retained = [tuple(r) for r in conn.execute("SELECT * FROM journal_entries ORDER BY id")]
    raw = [tuple(r) for r in conn.execute("SELECT * FROM transactions ORDER BY id")]
    old_version = conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0]
    # Both round-6 and round-6b writers stamp version 1. Keep the already
    # applied fee-v1 audit to prove the new migration is independently keyed.
    conn.execute("INSERT OR IGNORE INTO schema_migration_audits VALUES(?, ?, 1, '{}', 'now')",
                 ("at-fee-consumed-regime-v1", "at-fee-consumed-regime-v1"))
    conn.execute("DELETE FROM schema_migration_audits WHERE migration_name=?", (AT_OUTFLOW_REGIME_SEMANTICS_MIGRATION,))
    conn.execute("UPDATE profiles SET required_journal_semantics_version=1,journal_semantics_version=1")
    conn.commit()
    conn.close()
    for _ in range(2):
        with open_db(tmp_path / "engine-0") as reopened:
            p = reopened.execute("SELECT * FROM profiles").fetchone()
            assert p["required_journal_semantics_version"] == 2
            assert p["journal_input_version"] == old_version + 1
            assert [tuple(r) for r in reopened.execute("SELECT * FROM journal_entries ORDER BY id")] == retained
            assert [tuple(r) for r in reopened.execute("SELECT * FROM transactions ORDER BY id")] == raw
            with pytest.raises(AppError, match="fresh journals"):
                reports.report_exit_tax(reopened, "W", "P", handlers._report_hooks())
        reopened.close()
    with open_db(tmp_path / "engine-0") as rebuilt:
        handlers.process_journals(rebuilt, "W", "P")
        assert rebuilt.execute("SELECT journal_semantics_version FROM profiles").fetchone()[0] == 2
        reports.report_exit_tax(rebuilt, "W", "P", handlers._report_hooks())
    rebuilt.close()


@pytest.mark.parametrize("fees", (False, True))
def test_downgrade_and_rebuild_cannot_reuse_outflow_semantics(engine_book, monkeypatch, tmp_path, fees):
    from kassiber.core import custody_journal
    from kassiber.errors import AppError
    rows = regime_rows() if fees else [(*r[:5], 0, *r[6:]) for r in regime_rows()]
    conn = engine_book(rows, pair=True, quotes=[("2024-03-31T12:00:00Z", 65000)])
    initial = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-03-31")
    # A real rebuild using the version-1 writer publication protocol, after a
    # current rebuild and after the migration sentinel is already present.
    # No report, journal builder, or RP2 computation is stubbed.
    with monkeypatch.context() as old_writer:
        old_writer.setattr(custody_journal, "JOURNAL_CALCULATION_SEMANTICS_VERSION", 1)
        conn.execute("UPDATE profiles SET journal_input_version=journal_input_version+1")
        conn.commit()
        handlers.process_journals(conn, "W", "P")
    p = conn.execute("SELECT * FROM profiles").fetchone()
    assert p["journal_semantics_version"] == 1
    assert p["journal_input_version"] == p["last_processed_input_version"]
    conn.close()
    with open_db(tmp_path / "engine-0") as reopened:
        assert not custody_journal.projection_freshness(reopened, p["id"])["is_current"]
        with pytest.raises(AppError, match="fresh journals"):
            reports.report_exit_tax(reopened, "W", "P", handlers._report_hooks(), departure_date="2024-03-31")
        handlers.process_journals(reopened, "W", "P")
        assert custody_journal.projection_freshness(reopened, p["id"])["is_current"]
        corrected = reports.report_exit_tax(reopened, "W", "P", handlers._report_hooks(), departure_date="2024-03-31")
        assert corrected["totals"] == initial["totals"]
    reopened.close()


@pytest.mark.parametrize(("quote_at", "priced"), [
    ("2024-01-01T00:00:00Z", False),
    ("2024-12-30T23:59:59.999999Z", False),
    ("2024-12-31T00:00:00Z", True),
    ("2024-12-31T23:59:59.999999Z", True),
    ("2025-01-01T00:00:00Z", False),
])
def test_cached_fmv_uses_the_same_age_limit(engine_book, quote_at, priced):
    conn = engine_book([("buy", "Cold", "2022-01-01T00:00:00Z", "inbound", BTC, 0, 30000, "buy")],
                       quotes=[(quote_at, 65000)])
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-12-31")
    assert report["fmvSource"][0]["source"] == ("cache" if priced else "missing")
    assert report["status"]["missingFmvAssets"] == ([] if priced else ["BTC"])
    assert report["totals"]["estimatedTax"] == (9625 if priced else None)


@pytest.mark.parametrize(("cached_at", "expected_source", "expected_rate"), [
    ("2024-01-01T00:00:00Z", "transaction", 70000),
    ("2024-12-31T11:00:00Z", "transaction", 70000),
    ("2024-12-31T12:00:00Z", "cache", 65000),
    ("2024-12-31T13:00:00Z", "cache", 65000),
    ("2025-01-01T00:00:00Z", "transaction", 70000),
])
def test_closest_fmv_quote_wins_across_sources(engine_book, cached_at, expected_source, expected_rate):
    conn = engine_book([("buy", "Cold", "2024-12-31T12:00:00Z", "inbound", BTC, 0, 70000, "buy")],
                       quotes=[(cached_at, 65000)])
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-12-31")
    assert report["fmvSource"][0]["source"] == expected_source
    assert report["fmvSource"][0]["rate"] == expected_rate
    assert report["status"]["missingFmvAssets"] == []


def test_cached_fmv_chooses_closest_instant_across_offsets(engine_book):
    conn = engine_book([("buy", "Cold", "2022-01-01T00:00:00Z", "inbound", BTC, 0, 30000, "buy")],
                       quotes=[("2024-12-31T23:30:00+02:00", 1),
                               ("2024-12-31T22:00:00Z", 65000)])
    report = reports.report_exit_tax(conn, "W", "P", handlers._report_hooks(), departure_date="2024-12-31")
    assert report["fmvSource"][0]["rate"] == 65000
    assert report["totals"]["estimatedTax"] == 9625
