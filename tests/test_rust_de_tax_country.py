"""A German book end to end on the Rust backend (preview country).

The book is built with the CLI as a user would: a German profile, two
wallets, CSV imports, a reviewed move between them, journal processing, and
the full report as PDF, XLSX and CSV. The figures are worked by hand below
(docs/reference/tax-country-de.md): FIFO per wallet, network fee units
disposed of at their value before the sale and their value split over the
sale's lots as Werbungskosten, the one-year period by Berlin date carried
across the move, the 2024 Freigrenzen, and block rewards kept apart. Python
does not know the country; tests/test_python_rust_only_countries.py covers
its refusal.
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

NOTICE = (
    "Noch nicht von einer Steuerberaterin oder einem Steuerberater für dieses Land geprüft"
    " — Ergebnisse vor der Einreichung kontrollieren."
)
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
MOVE = "%064x" % 9
HOT = [
    "2023-01-10T09:00:00Z,b1,inbound,BTC,1.00000000,0.00000000,15000,Kauf,buy",
    "2024-02-01T09:00:00Z,b2,inbound,BTC,1.00000000,0.00000000,40000,Kauf,buy",
    f"2024-03-01T09:00:00Z,{MOVE},outbound,BTC,0.50000000,0.00010000,50000,An Cold,transfer",
    "2024-06-01T09:00:00Z,s1,outbound,BTC,1.00000000,0.00010000,60000,Verkauf,sell",
    "2024-08-01T09:00:00Z,i1,inbound,BTC,0.00100000,0.00000000,60000,Zinsen,interest",
    "2024-09-01T09:00:00Z,m1,inbound,BTC,0.00200000,0.00000000,55000,Mining,mining",
]
COLD = [
    f"2024-03-01T09:00:00Z,{MOVE},inbound,BTC,0.50000000,0.00000000,50000,Von Hot,transfer",
    "2025-04-01T09:00:00Z,s2,outbound,BTC,0.20000000,0.00000000,80000,Verkauf,sell",
]


def _env(tmp_path):
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
    env.update({
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_RUST_TEST_REPORT_TIME": "2026-01-02T03:04:05Z",
        "KASSIBER_TEST_NO_EGRESS": "strict",
        "KASSIBER_DISABLE_UPDATE_CHECK": "1",
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(tmp_path / "offline.json"),
        "XDG_CONFIG_HOME": str(tmp_path / "config"),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "XDG_STATE_HOME": str(tmp_path / "state"),
    })
    return env


def _book(tmp_path, extra_hot=()):
    root = tmp_path / "data"
    env = _env(tmp_path)

    def run(*args, ok=True):
        result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(root), *args],
                                env=env, capture_output=True, text=True, timeout=300)
        payload = json.loads(result.stdout)
        if ok:
            assert result.returncode == 0 and payload["kind"] != "error", result.stdout + result.stderr
        return payload

    scope = ["--workspace", "Main", "--profile", "Buch"]
    run("init")
    run("workspaces", "create", "Main")
    profile = run("profiles", "create", "--workspace", "Main", "--tax-country", "de", "Buch")["data"]
    for wallet, rows in (("Hot", HOT + list(extra_hot)), ("Cold", COLD)):
        run("wallets", "create", *scope, "--label", wallet, "--kind", "custom", "--chain", "bitcoin")
        path = tmp_path / f"{wallet}.csv"
        path.write_text(HEADER + "\n" + "\n".join(rows) + "\n", encoding="utf-8")
        run("wallets", "import-csv", *scope, "--wallet", wallet, "--file", str(path))
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        out_id, in_id = (
            conn.execute(
                "SELECT t.id FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE w.label=? AND t.external_id=?",
                (wallet, MOVE),
            ).fetchone()[0]
            for wallet in ("Hot", "Cold")
        )
    run("transfers", "pair", *scope, "--tx-out", out_id, "--tx-in", in_id, "--policy", "carrying-value")
    return root, run, scope, profile


def test_profile_defaults_follow_the_registry(tmp_path):
    _, run, scope, profile = _book(tmp_path)
    assert (profile["fiat_currency"], profile["gains_algorithm"], profile["cost_basis_pool_scope"]) == (
        "EUR", "FIFO", "wallet")
    details = run("profiles", "get", *scope)["data"]
    assert details["allowed_cost_basis_pool_scopes"] == ["wallet"]
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "de",
                  "--gains-algorithm", "HIFO", "Hifo", ok=False)
    assert refused["error"]["code"] == "validation"
    assert refused["error"]["hint"] == "Choose one of: FIFO, MOVING_AVERAGE_FIFO_PERIOD"
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "de",
                  "--cost-basis-pool-scope", "global", "Global", ok=False)
    assert refused["error"]["code"] == "validation"


def test_wallet_fifo_journal_and_anlage_so(tmp_path):
    root, run, scope, _ = _book(tmp_path)
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        entries = conn.execute(
            "SELECT je.occurred_at,w.label,je.entry_type,je.quantity,je.proceeds_exact,je.cost_basis_exact,"
            "je.gain_loss_exact,je.capital_gains_type FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id "
            "WHERE je.entry_type IN ('disposal','fee','transfer_fee','income') "
            "ORDER BY je.occurred_at,je.entry_type,je.capital_gains_type"
        ).fetchall()
        stored = conn.execute(
            "SELECT calculation_json FROM journal_entries WHERE entry_type='disposal' AND capital_gains_type='short'"
        ).fetchone()[0]
    # The move's fee (0.0001 of the 2023 lot at 50,000) is a disposal at 5.00,
    # outside the year. The sale's fee units (0.0001 at 60,000) go first, then
    # 1 BTC: 0.4998 of the 2023 lot (outside the year) and 0.5002 of the 2024
    # lot (taxable), the fee's 6.00 split by quantity as Werbungskosten. The
    # moved half keeps its 2023 date, so Cold's 2025 sale is outside the year.
    assert [tuple(map(str, row)) for row in entries] == [
        ("2024-03-01T09:00:00Z", "Hot", "transfer_fee", "-10000000", "5.0000", "1.5000", "3.5000", "long"),
        ("2024-06-01T09:00:00Z", "Hot", "disposal", "-49980000000", "29988.0000", "7497.0000", "22488.00120000", "long"),
        ("2024-06-01T09:00:00Z", "Hot", "disposal", "-50020000000", "30012.0000", "20008.0000", "10000.99880000", "short"),
        ("2024-06-01T09:00:00Z", "Hot", "fee", "-10000000", "6.0000", "1.5000", "4.5000", "long"),
        ("2024-08-01T09:00:00Z", "Hot", "income", "100000000", "60.000", "0", "60.000", "short"),
        ("2024-09-01T09:00:00Z", "Hot", "income", "200000000", "110.000", "0", "110.000", "short"),
        ("2025-04-01T09:00:00Z", "Cold", "disposal", "-20000000000", "16000.0", "3000.0", "13000.0", "long"),
    ]
    # The fragment's Werbungskosten are stored with it.
    assert json.loads(stored)["fragments"][0]["expenses_exact"] == "3.00120000"

    target = tmp_path / "out" / "full.csv"
    run("reports", "export-csv", *scope, "--file", str(target))
    bundle = zipfile.ZipFile(target.with_suffix(".zip"))
    read = lambda name: list(csv.DictReader(io.StringIO(bundle.read(name).decode())))
    assert read("summary.csv")[0] == {"section": "preview", "key": "notice", "value": NOTICE, "unit": ""}
    assert json.loads(bundle.read("manifest.json"))["preview_notice"] == NOTICE
    years = {row["tax_year"]: row for row in read("anlage_so.csv")}
    for row in read("anlage_so_leistungen.csv"):
        years[row["tax_year"]].update(row)
    assert set(years) == {"2024", "2025"}
    figures = lambda year, *keys: tuple(years[year][key] for key in keys)
    assert figures("2024", "p23_lines", "p23_proceeds", "p23_cost", "p23_expenses", "p23_net_gain") == (
        "41–47, 54–55", "30012.0000", "20008.0000", "3.00120000", "10000.99880000")
    assert figures("2024", "p23_freigrenze", "p23_result", "p23_amount") == (
        "1000.00", "steuerpflichtig", "10000.99880000")
    # Outside the year: the move's fee, the sale's fee units and the 2023 lot.
    assert years["2024"]["p23_outside"] == "22496.00120000"
    assert figures("2024", "p22_3_lines", "p22_3_total", "p22_3_result", "p22_3_amount", "mining_total") == (
        "10–16", "60.000", "unter der Freigrenze (indikativ)", "0", "110.000")
    assert figures("2025", "p23_lines", "p23_result", "p23_outside", "loss_line") == (
        "43/45–49, 56–57", "keine", "13000.0", "66")
    disposals = read("p23_disposals.csv")
    assert [(r["disposed_at"], r["kind"], r["acquired_at"], r["holding"], r["taxable"], r["expenses"], r["gain"])
            for r in disposals] == [
        ("2024-03-01", "Gebühr", "2023-01-10", "> 1 Jahr", "nein", "0.0000", "3.5000"),
        ("2024-06-01", "Gebühr", "2023-01-10", "> 1 Jahr", "nein", "0.0000", "4.5000"),
        ("2024-06-01", "Verkauf", "2023-01-10", "> 1 Jahr", "nein", "2.99880000", "22488.00120000"),
        ("2024-06-01", "Verkauf", "2024-02-01", "≤ 1 Jahr", "ja", "3.00120000", "10000.99880000"),
        ("2025-04-01", "Verkauf", "2023-01-10", "> 1 Jahr", "nein", "0.0", "13000.0"),
    ]
    income = read("income_sections.csv")
    assert [(r["kind"], r["section"], r["value"]) for r in income] == [
        ("interest", "§ 22 Nr. 3", "60.000"),
        ("mining", "Blockerstellung – Einordnung erforderlich", "110.000"),
    ]
    # Each wallet's holding at the end of every year with figures and the
    # year before; the move's half sits in Cold from 2024.
    holdings = [(r["date"], r["wallet"], r["quantity"]) for r in read("year_end_holdings.csv")]
    assert holdings == [
        ("2023-12-31", "Hot", "1.00000000000"),
        ("2024-12-31", "Cold", "0.50000000000"),
        ("2024-12-31", "Hot", "0.50280000000"),
        ("2025-12-31", "Cold", "0.30000000000"),
        ("2025-12-31", "Hot", "0.50280000000"),
    ]

    workbook = tmp_path / "out" / "full.xlsx"
    data = run("reports", "export-xlsx", *scope, "--file", str(workbook))["data"]
    assert {"anlage_so", "p23_disposals", "income_sections", "year_end_holdings"} <= set(data["sheets"])
    assert NOTICE in zipfile.ZipFile(workbook).read("xl/sharedStrings.xml").decode()

    first, second = tmp_path / "out" / "a.pdf", tmp_path / "out" / "b.pdf"
    run("reports", "export-pdf", *scope, "--file", str(first))
    run("reports", "export-pdf", *scope, "--file", str(second))
    assert first.read_bytes().startswith(b"%PDF-")
    assert first.read_bytes() == second.read_bytes()


def test_an_unclassified_airdrop_is_held_with_its_code(tmp_path):
    airdrop = "2024-10-01T09:00:00Z,a1,inbound,BTC,0.00010000,0.00000000,60000,Airdrop,airdrop"
    root, run, scope, _ = _book(tmp_path, extra_hot=[airdrop])
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        held = conn.execute(
            "SELECT q.reason,q.detail_json FROM journal_quarantines q JOIN transactions t ON t.id=q.transaction_id "
            "WHERE t.external_id='a1'"
        ).fetchall()
    assert [reason for reason, _ in held] == ["acquisition_valuation_unsupported"]
    assert json.loads(held[0][1])["code"] == "airdrop_classification_required"
