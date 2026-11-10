"""Irish and United Kingdom books end to end on the Rust backend (preview
countries).

Each book is built with the CLI as a user would: a profile of the country,
CSV imports, a reviewed move between two wallets where the rules need one,
journal processing, and the full report as CSV, XLSX and PDF. The figures
are worked by hand below from the specifications
(docs/reference/tax-country-ie.md, docs/reference/tax-country-gb.md); the
country vectors cover each rule on its own. Python does not know either
country; tests/test_python_rust_only_countries.py covers its refusal.
"""
from __future__ import annotations

import csv
import io
import json
import os
import sqlite3
import subprocess
import zipfile

import pytest

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")

NOTICE = "Not yet reviewed by a tax professional for this country — check results before filing."
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
MOVE = "%064x" % 9


def _env(tmp_path):
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
    env.update({
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_RUST_TEST_REPORT_TIME": "2026-11-02T03:04:05Z",
        "KASSIBER_TEST_NO_EGRESS": "strict",
        "KASSIBER_DISABLE_UPDATE_CHECK": "1",
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(tmp_path / "offline.json"),
        "XDG_CONFIG_HOME": str(tmp_path / "config"),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "XDG_STATE_HOME": str(tmp_path / "state"),
    })
    return env


def _book(tmp_path, country, wallets):
    """A book of `country` with `wallets` ({label: csv rows}); returns the
    data root, the CLI runner, the scope arguments and the profile."""
    root = tmp_path / "data"
    env = _env(tmp_path)

    def run(*args, ok=True):
        result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(root), *args],
                                env=env, capture_output=True, text=True, timeout=300)
        payload = json.loads(result.stdout)
        if ok:
            assert result.returncode == 0 and payload["kind"] != "error", result.stdout + result.stderr
        return payload

    scope = ["--workspace", "Main", "--profile", "Book"]
    run("init")
    run("workspaces", "create", "Main")
    profile = run("profiles", "create", "--workspace", "Main", "--tax-country", country, "Book")["data"]
    for wallet, rows in wallets.items():
        run("wallets", "create", *scope, "--label", wallet, "--kind", "custom", "--chain", "bitcoin")
        path = tmp_path / f"{wallet}.csv"
        path.write_text(HEADER + "\n" + "\n".join(rows) + "\n", encoding="utf-8")
        run("wallets", "import-csv", *scope, "--wallet", wallet, "--file", str(path))
    return root, run, scope, profile


def _pair_move(root, run, scope):
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        out_id, in_id = (
            conn.execute(
                "SELECT t.id FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE w.label=? AND t.external_id=?",
                (wallet, MOVE),
            ).fetchone()[0]
            for wallet in ("A", "B")
        )
    run("transfers", "pair", *scope, "--tx-out", out_id, "--tx-in", in_id, "--policy", "carrying-value")


def _bundle(run, scope, tmp_path):
    target = tmp_path / "out" / "full.csv"
    run("reports", "export-csv", *scope, "--file", str(target))
    bundle = zipfile.ZipFile(target.with_suffix(".zip"))
    return bundle, lambda name: list(csv.DictReader(io.StringIO(bundle.read(name).decode())))


def _pdf_repeats(run, scope, tmp_path):
    first, second = tmp_path / "out" / "a.pdf", tmp_path / "out" / "b.pdf"
    run("reports", "export-pdf", *scope, "--file", str(first))
    run("reports", "export-pdf", *scope, "--file", str(second))
    assert first.read_bytes().startswith(b"%PDF-")
    assert first.read_bytes() == second.read_bytes()


# ------------------------------------------------------------------ Ireland

IE_ROWS = [
    "2024-01-10T10:00:00Z,buy-1,inbound,BTC,1.00000000,0.00000000,30000,Purchase,buy",
    "2024-06-10T10:00:00Z,sell-1,outbound,BTC,0.50000000,0.00000000,40000,Sale,sell",
    "2024-12-10T10:00:00Z,sell-2,outbound,BTC,0.20000000,0.00000000,25000,December sale,sell",
    "2025-03-01T10:00:00Z,int-1,inbound,BTC,0.01000000,0.00000000,80000,Lending interest,interest",
    "2026-03-10T10:00:00Z,sell-3,outbound,BTC,0.10000000,0.00000000,60000,Sale,sell",
    # 7 October 2026 in Dublin: the 31 % rate.
    "2026-10-06T23:30:00Z,sell-4,outbound,BTC,0.10000000,0.00000000,100000,Sale,sell",
]


def test_ireland_profile_defaults_follow_the_registry(tmp_path):
    _, run, _, profile = _book(tmp_path, "ie", {"Hot": IE_ROWS[:1]})
    assert (profile["fiat_currency"], profile["gains_algorithm"], profile["cost_basis_pool_scope"]) == ("EUR", "FIFO", "global")
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "ie",
                  "--gains-algorithm", "LIFO", "Other", ok=False)
    assert refused["error"]["code"] == "validation"
    assert refused["error"]["hint"] == "Choose one of: FIFO"


def test_ireland_cg1_lines_payments_and_rates(tmp_path):
    _, run, scope, _ = _book(tmp_path, "ie", {"Hot": IE_ROWS})
    run("journals", "process", *scope)
    _, read = _bundle(run, scope, tmp_path)
    assert read("summary.csv")[0] == {"section": "preview", "key": "notice", "value": NOTICE, "unit": ""}
    disposals = [(r["date"], r["consideration"], r["allowable_cost"], r["gain_loss"], r["period"], r["rate"])
                 for r in read("ie_disposals.csv")]
    # FIFO from the one lot at 30,000/BTC; the December sale pays later; the
    # 23:30 UTC sale is 7 October in Dublin, at 31 %.
    assert disposals == [
        ("2024-06-10", "20000.00", "15000.00", "5000.00", "Initial (by 15 Dec)", "33.00"),
        ("2024-12-10", "5000.00", "6000.00", "-1000.00", "Later (by 31 Jan)", "33.00"),
        ("2026-03-10", "6000.00", "3000.00", "3000.00", "Initial (by 15 Dec)", "33.00"),
        ("2026-10-07", "10000.00", "3000.00", "7000.00", "Initial (by 15 Dec)", "31.00"),
    ]
    lines = {r["tax_year"]: r for r in read("ie_cg1.csv")}
    keys = ("line_1k_disposals", "line_1k_consideration", "line_10", "line_11", "line_14", "line_18", "line_19", "line_23")
    # 2024: gains 5,000 less the December loss and the €1,270 exemption;
    # 2025 has only the interest; 2026's exemption is set against the 33 %
    # gains first. Lines are whole euros.
    assert {year: [row[k] for k in keys] for year, row in lines.items()} == {
        "2024": ["2", "25000", "5000", "1000", "4000", "1270", "2730", "0"],
        "2025": ["0", "0", "0", "0", "0", "0", "0", "0"],
        "2026": ["2", "16000", "10000", "0", "10000", "1270", "8730", "0"],
    }
    payments = {r["tax_year"]: r for r in read("ie_payments.csv")}
    # 2024: 2,730 × 33 % = 900.90; as if the year ended on 30 November,
    # 3,730 × 33 % = 1,230.90 was due on 15 December, so 330.00 was
    # overpaid. 2026: 1,730 × 33 % + 7,000 × 31 % = 2,740.90.
    assert [payments["2024"][k] for k in ("tax_total", "payment_initial", "payment_later", "overpaid_initial")] == [
        "900.90", "1230.90", "0", "330.00"]
    assert payments["2025"]["case_iv_income"] == "800.00"
    assert (payments["2026"]["tax_total"], payments["2026"]["payment_initial"]) == ("2740.90", "2740.90")
    assert [(r["tax_year"], r["line"], r["rate"], r["net_gain"]) for r in read("ie_rate_lines.csv")] == [
        ("2024", "24", "33.00", "2730.00"),
        ("2026", "24", "33.00", "1730.00"),
        ("2026", "24", "31.00", "7000.00"),
    ]
    assert [(r["status"], r["code"]) for r in read("ie_review.csv")] == [("Flag", "ie_rate_31_pending_finance_act")]
    assert [(r["kind"], r["value"]) for r in read("ie_income.csv")] == [("interest", "800.00")]

    workbook = tmp_path / "out" / "full.xlsx"
    data = run("reports", "export-xlsx", *scope, "--file", str(workbook))["data"]
    assert {"ie_cg1", "ie_rate_lines", "ie_payments", "ie_disposals", "ie_income", "ie_review"} <= set(data["sheets"])
    assert NOTICE in zipfile.ZipFile(workbook).read("xl/sharedStrings.xml").decode()
    _pdf_repeats(run, scope, tmp_path)


def test_ireland_holds_receipts_it_does_not_book(tmp_path):
    rows = IE_ROWS[:2] + [
        "2024-07-01T10:00:00Z,drop-1,inbound,BTC,0.01000000,0.00000000,50000,Airdrop,airdrop",
    ]
    root, run, scope, _ = _book(tmp_path, "ie", {"Hot": rows})
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        held = conn.execute("SELECT t.external_id,jq.reason FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id").fetchall()
    assert held == [("drop-1", "acquisition_valuation_unsupported")]
    _, read = _bundle(run, scope, tmp_path)
    assert [(r["status"], r["code"]) for r in read("ie_review.csv")] == [("Held", "ie_airdrop_requires_review")]


# -------------------------------------------------------- United Kingdom

GB_A = [
    "2024-05-01T10:00:00Z,buy-1,inbound,BTC,1.00000000,0.00000000,30000,Purchase,buy",
    "2024-06-01T09:00:00Z,sell-1,outbound,BTC,0.50000000,0.00000000,70000,Sale,sell",
    # Day 30 after the sale: matched with it (the 30-day rule).
    "2024-07-01T09:00:00Z,buy-2,inbound,BTC,0.20000000,0.00000000,60000,Purchase,buy",
    # The move debits 0.5 plus its 0.0001 fee: amounts exclude the fee, as
    # Python books it (with the fee inside, both hold the move unresolved).
    f"2024-08-01T12:00:00Z,{MOVE},outbound,BTC,0.50000000,0.00010000,60000,To B,transfer",
    "2025-05-01T10:00:00Z,mine-1,inbound,BTC,0.01000000,0.00000000,50000,Mining,mining",
]
GB_B = [
    f"2024-08-01T12:00:00Z,{MOVE},inbound,BTC,0.50000000,0.00000000,60000,From A,transfer",
    # 00:30 BST on 6 April: tax year 2025-26.
    "2025-04-05T23:30:00Z,sell-2,outbound,BTC,0.10000000,0.00000000,80000,Sale,sell",
]


def _gb_book(tmp_path):
    root, run, scope, profile = _book(tmp_path, "gb", {"A": GB_A, "B": GB_B})
    _pair_move(root, run, scope)
    return root, run, scope, profile


def test_united_kingdom_profile_defaults_follow_the_registry(tmp_path):
    _, run, _, profile = _book(tmp_path, "gb", {"A": GB_A[:1]})
    assert (profile["fiat_currency"], profile["gains_algorithm"], profile["cost_basis_pool_scope"]) == ("GBP", "UK_SHARE_MATCHING", "global")
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "gb",
                  "--gains-algorithm", "FIFO", "Other", ok=False)
    assert refused["error"]["code"] == "validation"
    assert refused["error"]["hint"] == "Choose one of: UK_SHARE_MATCHING"


def test_united_kingdom_share_matching_and_sa108(tmp_path):
    root, run, scope, _ = _gb_book(tmp_path)
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        entries = conn.execute(
            "SELECT t.external_id,je.entry_type,je.proceeds_exact,je.cost_basis_exact,je.gain_loss_exact FROM journal_entries je "
            "JOIN transactions t ON t.id=je.transaction_id WHERE je.entry_type IN ('disposal','transfer_fee','income') "
            "ORDER BY je.occurred_at"
        ).fetchall()
    # 1 June: 0.2 of 1 July's purchase (day 30, 12,000) and 0.3 of the pool
    # at 30,000/BTC. The move's fee: 0.0001 at 60,000 against the pool's
    # 3.00. 6 April 2025 (00:30 BST): the mining receipt of 1 May is within
    # 30 days, so 0.01 at 500 and 0.09 of the pool (0.6999 at 20,997).
    assert [(e[0][:6], e[1], *(str(round(float(v), 2)) for v in e[2:])) for e in entries] == [
        ("sell-1", "disposal", "35000.0", "21000.0", "14000.0"),
        ("000000", "transfer_fee", "6.0", "3.0", "3.0"),
        ("sell-2", "disposal", "8000.0", "3200.0", "4800.0"),
        ("mine-1", "income", "500.0", "0.0", "500.0"),
    ]
    bundle, read = _bundle(run, scope, tmp_path)
    assert json.loads(bundle.read("manifest.json"))["preview_notice"] == NOTICE
    boxes = {r["tax_year"]: r for r in read("gb_sa108.csv")}
    keys = ("box_disposals", "box_proceeds", "box_costs", "box_gains", "box_losses")
    assert {year: [row[k] for k in keys] for year, row in boxes.items()} == {
        "2024-25": ["2", "35006", "21003", "14003", "0"],
        "2025-26": ["1", "8000", "3200", "4800", "0"],
    }
    years = {r["tax_year"]: r for r in read("gb_years.csv")}
    assert [years["2024-25"][k] for k in ("annual_exempt_amount", "gains_above_aea", "gains_over_aea", "loss_to_claim")] == [
        "3000.00", "11003.0000", "Yes", "No"]
    assert years["2025-26"]["misc_income_box17"] == "500.00"
    assert [(r["tax_year"], r["split_date"], r["gains_before"], r["gains_from"]) for r in read("gb_rate_split.csv")] == [
        ("2024-25", "2024-10-30", "14003.0000", "0")]
    matches = [(r["date"], r["rule"], r["acquired"], r["quantity"], r["pool_quantity_before"]) for r in read("gb_matches.csv")]
    assert matches == [
        ("2024-06-01", "30 days (s106A)", "2024-07-01", "0.20000000000", ""),
        ("2024-06-01", "Section 104 pool (s104)", "", "0.30000000000", "1.00000000000"),
        ("2024-08-01", "Section 104 pool (s104)", "", "0.00010000000", "0.70000000000"),
        ("2025-04-06", "30 days (s106A)", "2025-05-01", "0.01000000000", ""),
        ("2025-04-06", "Section 104 pool (s104)", "", "0.09000000000", "0.69990000000"),
    ]
    pools = [(r["tax_year"], r["quantity"], r["pooled_cost"]) for r in read("gb_pools.csv")]
    assert pools == [("2024-25", "0.69990000000", "20997.0000"), ("2025-26", "0.60990000000", "18297.0000")]
    assert [(r["kind"], r["tax_year"]) for r in read("gb_income.csv")] == [("mining", "2025-26")]

    workbook = tmp_path / "out" / "full.xlsx"
    data = run("reports", "export-xlsx", *scope, "--file", str(workbook))["data"]
    assert {"gb_sa108", "gb_years", "gb_computations", "gb_matches", "gb_pools", "gb_income"} <= set(data["sheets"])
    _pdf_repeats(run, scope, tmp_path)


def test_united_kingdom_withholds_figures_it_would_have_to_guess(tmp_path):
    rows = GB_A[:2] + [
        "2024-07-10T10:00:00Z,gift-1,outbound,BTC,0.01000000,0.00000000,60000,Gift,gift",
    ]
    _, run, scope, _ = _book(tmp_path, "gb", {"A": rows})
    run("journals", "process", *scope)
    bundle, read = _bundle(run, scope, tmp_path)
    assert [r["code"] for r in read("gb_refusals.csv")] == ["gift_recipient_classification_required"]
    assert "gb_sa108.csv" not in bundle.namelist()
