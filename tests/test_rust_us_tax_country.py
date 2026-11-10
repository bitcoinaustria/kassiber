"""A United States book end to end on the Rust backend (preview country).

The book is built with the CLI as a user would: a US profile, two wallets,
CSV imports, a reviewed move between them, journal processing, and the full
report as PDF, XLSX and CSV. The figures are worked by hand below
(docs/reference/tax-country-us.md): FIFO per wallet, a move's fee leaving
first without a gain, the holding period carried across the move, and the
Form 8949 boxes and Schedule D lines of 2025. Python does not know the
country; tests/test_python_rust_only_countries.py covers its refusal.
"""
from __future__ import annotations

import csv
import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import zipfile

import pytest

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")

NOTICE = "Not yet reviewed by a tax professional for this country — check results before filing."
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
MOVE = "%064x" % 7
HOT = [
    "2024-01-05T15:00:00Z,buy-1,inbound,BTC,0.50000000,0.00000000,40000,Purchase,buy",
    "2024-03-01T15:00:00Z,buy-2,inbound,BTC,0.30000000,0.00000000,60000,Purchase,buy",
    # The move debits 0.4 plus its 0.0001 fee: amounts exclude the fee, so
    # the pair balances and only the fee leaves Hot without reaching Cold.
    f"2024-06-01T12:00:00Z,{MOVE},outbound,BTC,0.40000000,0.00010000,67000,To cold,transfer",
    "2025-02-01T12:00:00Z,sell-1,outbound,BTC,0.10000000,0.00000000,95000,Exchange sale,sell",
    "2025-03-15T12:00:00Z,int-1,inbound,BTC,0.00100000,0.00000000,85000,Interest,interest",
]
COLD = [
    f"2024-06-01T12:00:00Z,{MOVE},inbound,BTC,0.40000000,0.00000000,67000,From hot,transfer",
    "2025-07-01T12:00:00Z,sell-2,outbound,BTC,0.20000000,0.00000000,100000,Exchange sale,sell",
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


def _book(tmp_path, scope_args=()):
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
    profile = run("profiles", "create", "--workspace", "Main", "--tax-country", "us", *scope_args, "Book")["data"]
    for wallet, rows in (("Hot", HOT), ("Cold", COLD)):
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
    assert (profile["fiat_currency"], profile["gains_algorithm"], profile["cost_basis_pool_scope"]) == ("USD", "FIFO", "wallet")
    details = run("profiles", "get", *scope)["data"]
    assert details["allowed_cost_basis_pool_scopes"] == ["global", "wallet"]
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "us",
                  "--gains-algorithm", "MOVING_AVERAGE", "Average", ok=False)
    assert refused["error"]["code"] == "validation"
    assert refused["error"]["hint"] == "Choose one of: FIFO, HIFO, LIFO, LOFO"


def test_per_wallet_journal_and_report_figures(tmp_path):
    root, run, scope, _ = _book(tmp_path)
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        entries = conn.execute(
            "SELECT je.occurred_at,w.label,je.entry_type,je.quantity,je.proceeds_exact,je.cost_basis_exact,"
            "je.gain_loss_exact,je.capital_gains_type FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id "
            "WHERE je.entry_type IN ('disposal','transfer_fee','income') ORDER BY je.occurred_at,je.capital_gains_type"
        ).fetchall()
    # The fee (0.0001 of the 40,000 lot) leaves first, at its basis and no
    # gain; the move takes 0.4 of that lot, so Hot keeps 0.0999 of it and the
    # 60,000 lot. Hot's 2025 sale: 0.0999 long (held since 2024-01-05) and
    # 0.0001 short (2024-03-01). Cold's sale is the moved lot, long-term
    # because its holding period carries over the move.
    assert [tuple(map(str, row)) for row in entries] == [
        ("2024-06-01T12:00:00Z", "Hot", "transfer_fee", "-10000000", "4.0000", "4.0000", "0", "short"),
        ("2025-02-01T12:00:00Z", "Hot", "disposal", "-9990000000", "9490.5000", "3996.0000", "5494.5000", "long"),
        ("2025-02-01T12:00:00Z", "Hot", "disposal", "-10000000", "9.5000", "6.0000", "3.5000", "short"),
        ("2025-03-15T12:00:00Z", "Hot", "income", "100000000", "85.000", "0", "85.000", "short"),
        ("2025-07-01T12:00:00Z", "Cold", "disposal", "-20000000000", "20000.0", "8000.0", "12000.0", "long"),
    ]

    target = tmp_path / "out" / "full.csv"
    run("reports", "export-csv", *scope, "--file", str(target))
    bundle = zipfile.ZipFile(target.with_suffix(".zip"))
    read = lambda name: list(csv.DictReader(io.StringIO(bundle.read(name).decode())))
    assert read("summary.csv")[0] == {"section": "preview", "key": "notice", "value": NOTICE, "unit": ""}
    assert json.loads(bundle.read("manifest.json"))["preview_notice"] == NOTICE
    rows = [(r["tax_year"], r["part"], r["box"], r["date_acquired"], r["date_sold"], r["proceeds"], r["cost_basis"], r["gain_loss"], r["wallet"])
            for r in read("form_8949.csv")]
    assert rows == [
        ("2025", "Part I", "I", "2024-03-01", "2025-02-01", "9.5000", "6.0000", "3.5000", "Hot"),
        ("2025", "Part II", "L", "2024-01-05", "2025-02-01", "9490.5000", "3996.0000", "5494.5000", "Hot"),
        ("2025", "Part II", "L", "2024-01-05", "2025-07-01", "20000.0", "8000.0", "12000.0", "Cold"),
    ]
    schedule = {r["tax_year"]: r for r in read("schedule_d.csv")}
    assert schedule["2025"]["schedule_d_line_3"] == "3.5000"
    assert schedule["2025"]["schedule_d_line_10"] == "17494.5000"
    assert schedule["2025"]["ordinary_income"] == "85.000"
    assert (schedule["2024"]["short_box"], schedule["2024"]["long_box"], schedule["2024"]["network_fee_cost"]) == ("C", "F", "4.0000")
    assert [r["kind"] for r in read("us_income.csv")] == ["interest"]
    # Per-wallet holdings carry each wallet's own lots: Cold 0.2 of the
    # 40,000 lot; Hot 0.2999 of the 60,000 lot and the 85 interest receipt.
    by_wallet = {r["wallet"]: r for r in read("holdings_by_wallet.csv")}
    assert (by_wallet["Cold"]["cost_basis"], by_wallet["Hot"]["cost_basis"]) == ("8000.0", "18079.0")

    workbook = tmp_path / "out" / "full.xlsx"
    data = run("reports", "export-xlsx", *scope, "--file", str(workbook))["data"]
    assert {"form_8949", "schedule_d", "us_income", "us_network_fees"} <= set(data["sheets"])
    assert NOTICE in zipfile.ZipFile(workbook).read("xl/sharedStrings.xml").decode()

    first, second = tmp_path / "out" / "a.pdf", tmp_path / "out" / "b.pdf"
    run("reports", "export-pdf", *scope, "--file", str(first))
    run("reports", "export-pdf", *scope, "--file", str(second))
    assert first.read_bytes().startswith(b"%PDF-")
    assert first.read_bytes() == second.read_bytes()


def test_global_scope_refuses_disposals_from_2025(tmp_path):
    _, run, scope, profile = _book(tmp_path, ("--cost-basis-pool-scope", "global"))
    assert profile["cost_basis_pool_scope"] == "global"
    refused = run("journals", "process", *scope, ok=False)
    assert refused["kind"] == "error"
    assert "per wallet from 2025" in refused["error"]["message"]
