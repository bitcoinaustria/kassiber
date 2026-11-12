"""A Swiss book end to end on the Rust backend (preview country).

The book is built with the CLI as a user would: a Swiss profile, two
wallets, CSV imports, the ESTV year-end values entered as rates, journal
processing, and the full report as PDF, XLSX and CSV. The figures are
worked by hand below (docs/reference/tax-country-ch.md): gains are tax-free
and listed for information, income by category in CHF when received (a
receipt at 23:30 UTC on 31 December belongs to the next Zurich year), the
wealth at 31 December at the ESTV values (unknown without one), salary kept
out of the income, and the KS 36 indicators with and without the user's
inputs. Python does not know the country; tests/test_python_rust_only_countries.py
covers its refusal.
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
HOT = [
    "2023-06-01T09:00:00Z,b0,inbound,BTC,1.00000000,0.00000000,25000,Kauf,buy",
    "2024-01-31T09:00:00Z,b1,inbound,BTC,1.00000000,0.00000000,38000,Kauf,buy",
    "2024-03-31T10:00:00Z,i1,inbound,BTC,0.00200000,0.00000000,60200,Zins,interest",
    "2024-07-30T09:00:00Z,s1,outbound,BTC,0.50000000,0.00000000,60000,Verkauf,sell",
    "2024-08-01T09:00:00Z,m1,inbound,BTC,0.01000000,0.00000000,55000,Mining,mining",
    "2024-12-31T23:30:00Z,i2,inbound,BTC,0.00100000,0.00000000,88000,Zins,interest",
]
COLD = [
    "2024-05-01T09:00:00Z,w1,inbound,BTC,0.05000000,0.00000000,40000,Lohn,wages",
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
    profile = run("profiles", "create", "--workspace", "Main", "--tax-country", "ch", "Buch")["data"]
    for wallet, rows in (("Hot", HOT + list(extra_hot)), ("Cold", COLD)):
        run("wallets", "create", *scope, "--label", wallet, "--kind", "custom", "--chain", "bitcoin")
        path = tmp_path / f"{wallet}.csv"
        path.write_text(HEADER + "\n" + "\n".join(rows) + "\n", encoding="utf-8")
        run("wallets", "import-csv", *scope, "--wallet", wallet, "--file", str(path))
    # The ESTV Kursliste values, entered by hand: none for 2025.
    for day, value in (("2023-12-31T12:00:00+01:00", "36000"), ("2024-12-31T12:00:00+01:00", "84000")):
        run("rates", "set", "BTC-CHF", day, value, "--source", "estv_kursliste")
    return root, run, scope, profile


def _inputs(root, profile_id, inputs):
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        conn.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)",
            (f"tax_country.ch.ks36:{profile_id}", json.dumps(inputs)),
        )


def test_profile_defaults_follow_the_registry(tmp_path):
    _, run, scope, profile = _book(tmp_path)
    assert (profile["fiat_currency"], profile["gains_algorithm"], profile["cost_basis_pool_scope"]) == (
        "CHF", "FIFO", "global")
    details = run("profiles", "get", *scope)["data"]
    assert details["allowed_cost_basis_pool_scopes"] == ["global"]
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "ch",
                  "--gains-algorithm", "HIFO", "Hifo", ok=False)
    assert refused["error"]["code"] == "validation"
    assert refused["error"]["hint"] == "Choose one of: FIFO, MOVING_AVERAGE"


def test_income_wealth_and_ks36(tmp_path):
    root, run, scope, profile = _book(tmp_path)
    run("journals", "process", *scope)

    def bundle(name):
        target = tmp_path / "out" / f"{name}.csv"
        run("reports", "export-csv", *scope, "--file", str(target))
        files = zipfile.ZipFile(target.with_suffix(".zip"))
        return files, lambda table: list(csv.DictReader(io.StringIO(files.read(table).decode())))

    files, read = bundle("first")
    assert read("summary.csv")[0] == {"section": "preview", "key": "notice", "value": NOTICE, "unit": ""}
    assert json.loads(files.read("manifest.json"))["preview_notice"] == NOTICE
    # Wealth at 31 December: Hot 1 + 1 + 0.002 - 0.5 + 0.01 BTC, Cold the
    # salary's 0.05, at the ESTV value; 2025 has none, so it stays unknown.
    wealth = [(r["date"], r["wallet"], r["quantity"], r["unit_value"], r["tax_value"], r["source"], r["flags"])
              for r in read("wertschriften.csv")]
    assert wealth == [
        ("2023-12-31", "Hot", "1.00000000000", "36000", "36000.00000000000", "ESTV-Kursliste", ""),
        ("2024-12-31", "Cold", "0.05000000000", "84000", "4200.00000000000", "ESTV-Kursliste", ""),
        ("2024-12-31", "Hot", "1.51200000000", "84000", "127008.00000000000", "ESTV-Kursliste", ""),
        ("2025-12-31", "Cold", "0.05000000000", "", "", "\u2014", "Wert fehlt"),
        ("2025-12-31", "Hot", "1.51300000000", "", "", "\u2014", "Wert fehlt"),
    ]
    totals = {r["tax_year"]: (r["tax_value"], r["income_b"]) for r in read("wertschriften_total.csv")}
    # Column B holds income from movable assets only; mining goes elsewhere.
    assert totals["2024"] == ("131208.00000000000", "120.400")
    assert totals["2025"][0] == "unvollständig"
    # Interest and mining in 2024; the 23:30 UTC interest of 31 December is
    # Zurich's 1 January 2025. The salary is kept out of the income.
    income = [(r["date"], r["kind"], r["category"], r["value"]) for r in read("ch_income.csv")]
    assert income == [
        ("2024-03-31", "interest", "Ertrag aus beweglichem Vermögen (Art. 20 DBG)", "120.400"),
        ("2024-08-01", "mining", "Mining", "550.00"),
        ("2025-01-01", "interest", "Ertrag aus beweglichem Vermögen (Art. 20 DBG)", "88.000"),
    ]
    assert [(r["kind"], r["value"]) for r in read("ch_outside_income.csv")] == [("Lohn (Lohnausweis)", "2000.00000000")]
    # The 2024 sale takes the 2023 lot (global FIFO), held over six months:
    # a tax-free 17,500 gain. Volume 38,000 + 30,000 against 5 × 36,000.
    disposals = read("ch_disposals.csv")
    assert [(r["acquired_at"], r["held_6_months"], r["gain"]) for r in disposals] == [
        ("2023-06-01", "ja", "17500.0")]
    ks = {r["tax_year"]: r for r in read("ks36.csv")}
    assert (ks["2024"]["c1_holding"], ks["2024"]["c2_volume"], ks["2024"]["volume"], ks["2024"]["volume_limit"]) == (
        "erfüllt", "erfüllt", "68000.00000000", "180000.00000000000")
    assert (ks["2024"]["c3_income"], ks["2024"]["overall"]) == ("nicht beurteilbar", "nicht beurteilbar")

    # With the user's inputs every criterion of 2024 is met.
    _inputs(root, profile["id"], {"net_income_chf": {"2024": "100000"}, "debt_financed_without_cover": False,
                                  "derivatives_only_hedging": True, "canton": "Zürich"})
    files, read = bundle("second")
    ks = {r["tax_year"]: r for r in read("ks36.csv")}
    assert (ks["2024"]["c3_income"], ks["2024"]["half_net_income"], ks["2024"]["overall"]) == (
        "erfüllt", "50000", "Safe Harbour erfüllt")
    assert ks["2025"]["c3_income"] == "nicht beurteilbar"

    workbook = tmp_path / "out" / "full.xlsx"
    data = run("reports", "export-xlsx", *scope, "--file", str(workbook))["data"]
    assert {"wertschriften", "ks36", "ch_income", "ch_disposals"} <= set(data["sheets"])
    assert NOTICE in zipfile.ZipFile(workbook).read("xl/sharedStrings.xml").decode()
    first, second = tmp_path / "out" / "a.pdf", tmp_path / "out" / "b.pdf"
    run("reports", "export-pdf", *scope, "--file", str(first))
    run("reports", "export-pdf", *scope, "--file", str(second))
    assert first.read_bytes().startswith(b"%PDF-")
    assert first.read_bytes() == second.read_bytes()

    # A book marked as a professional trader's is refused.
    _inputs(root, profile["id"], {"professional_trader": True})
    refused = run("reports", "export-pdf", *scope, "--file", str(tmp_path / "out" / "c.pdf"), ok=False)
    assert refused["error"]["code"] == "professional_trading_unsupported"


def test_a_hard_fork_is_held_with_its_code(tmp_path):
    fork = "2024-10-01T09:00:00Z,h1,inbound,BTC,0.00010000,0.00000000,60000,Fork,hardfork"
    root, run, scope, _ = _book(tmp_path, extra_hot=[fork])
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        held = conn.execute(
            "SELECT q.reason,q.detail_json FROM journal_quarantines q JOIN transactions t ON t.id=q.transaction_id "
            "WHERE t.external_id='h1'"
        ).fetchall()
    assert [reason for reason, _ in held] == ["acquisition_valuation_unsupported"]
    assert json.loads(held[0][1])["code"] == "hardfork_treatment_unsupported"
