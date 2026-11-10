"""A Japanese book end to end on the Rust backend (preview country).

The book is built with the CLI as a user would: a Japanese profile, two
wallets, CSV imports, a reviewed move between them, journal processing, and
the full report as PDF, XLSX and CSV in Japanese, the PDF set with the
bundled Noto Sans JP. The figures are worked by hand below
(docs/reference/tax-country-jp.md): the total average per Tokyo year over
every wallet, a move's fee units as revenue and an equal expense, mining
income, and the NTA sheet's rounding; the moving average on the same book;
and the refused 2027 year. Python does not know the country;
tests/test_python_rust_only_countries.py covers its refusal.
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

NOTICE = "この国の税理士による確認はまだ行われていません。申告前に結果を確認してください。"
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
MOVE = "%064x" % 11
HOT = [
    # 2 BTC for 10,000,000 yen and 1 BTC for 6,000,000 yen.
    "2025-01-10T01:00:00Z,buy-1,inbound,BTC,2.00000000,0.00000000,5000000,購入,buy",
    "2025-03-01T01:00:00Z,buy-2,inbound,BTC,1.00000000,0.00000000,6000000,購入,buy",
    # A move of 1 BTC paying 0.0001 BTC worth 600 yen; the amount is what
    # arrives, the fee units leave beside it.
    f"2025-04-01T01:00:00Z,{MOVE},outbound,BTC,1.00000000,0.00010000,6000000,移動,transfer",
    "2025-05-01T01:00:00Z,mine-1,inbound,BTC,0.10000000,0.00000000,7000000,マイニング,mining",
]
COLD = [
    f"2025-04-01T01:00:00Z,{MOVE},inbound,BTC,1.00000000,0.00000000,6000000,移動,transfer",
    "2025-06-01T01:00:00Z,sell-1,outbound,BTC,1.00000000,0.00000000,7000000,売却,sell",
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


def _book(tmp_path, method_args=(), hot=HOT, cold=COLD):
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
    profile = run("profiles", "create", "--workspace", "Main", "--tax-country", "jp", *method_args, "Book")["data"]
    for wallet, rows in (("Hot", hot), ("Cold", cold)):
        run("wallets", "create", *scope, "--label", wallet, "--kind", "custom", "--chain", "bitcoin")
        path = tmp_path / f"{wallet}.csv"
        path.write_text(HEADER + "\n" + "\n".join(rows) + "\n", encoding="utf-8")
        run("wallets", "import-csv", *scope, "--wallet", wallet, "--file", str(path))
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        ids = [
            conn.execute(
                "SELECT t.id FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE w.label=? AND t.external_id=?",
                (wallet, MOVE),
            ).fetchone()
            for wallet in ("Hot", "Cold")
        ]
    if all(ids):
        run("transfers", "pair", *scope, "--tx-out", ids[0][0], "--tx-in", ids[1][0], "--policy", "carrying-value")
    return root, run, scope, profile


def test_profile_defaults_follow_the_registry(tmp_path):
    _, run, scope, profile = _book(tmp_path)
    assert (profile["fiat_currency"], profile["gains_algorithm"], profile["cost_basis_pool_scope"]) == ("JPY", "JP_TOTAL_AVERAGE", "global")
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "jp",
                  "--gains-algorithm", "FIFO", "Other", ok=False)
    assert refused["error"]["code"] == "validation"
    assert refused["error"]["hint"] == "Choose one of: JP_MOVING_AVERAGE, JP_TOTAL_AVERAGE"


def _read(run, scope, tmp_path, name):
    target = tmp_path / "out" / f"{name}.csv"
    run("reports", "export-csv", *scope, "--file", str(target))
    bundle = zipfile.ZipFile(target.with_suffix(".zip"))
    return bundle, lambda member: list(csv.DictReader(io.StringIO(bundle.read(member).decode())))


def test_total_average_journal_and_japanese_report(tmp_path):
    root, run, scope, _ = _book(tmp_path)
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        entries = conn.execute(
            "SELECT je.occurred_at,je.entry_type,je.quantity,je.proceeds_exact,je.cost_basis_exact "
            "FROM journal_entries je WHERE je.entry_type IN ('disposal','transfer_fee','income') ORDER BY je.occurred_at"
        ).fetchall()
    # 2025's total average: (10,000,000 + 6,000,000 + 700,000) ÷ 3.1 BTC.
    # The fee units (0.0001) and the sale (1) each cost that unit.
    assert [tuple(map(str, row)) for row in entries] == [
        ("2025-04-01T01:00:00Z", "transfer_fee", "-10000000", "600.0000", "538.70967741935483870967741935484"),
        ("2025-05-01T01:00:00Z", "income", "10000000000", "700000.0", "0"),
        ("2025-06-01T01:00:00Z", "disposal", "-100000000000", "7000000", "5387096.7741935483870967741935484"),
    ]
    bundle, read = _read(run, scope, tmp_path, "full")
    assert read("summary.csv")[0] == {"section": "preview", "key": "notice", "value": NOTICE, "unit": ""}
    assert json.loads(bundle.read("manifest.json"))["preview_notice"] == NOTICE
    sheet = read("jp_keisansho.csv")
    assert len(sheet) == 1
    row = sheet[0]
    assert (row["tax_year"], row["asset"], row["acquired_qty"], row["acquired_value"]) == ("2025", "BTC", "3.10000000000", "16700000.0")
    # Revenue 7,000,600 (the sale and the fee units), expenses the cost of
    # 1.0001 BTC plus the fee units' 600 yen, rounded up.
    assert (row["revenue_rounded"], row["expenses_rounded"], row["income"]) == ("7000600", "5388236", "1612364")
    figures = read("jp_return.csv")
    assert (figures[0]["revenue_zatsu_sonota"], figures[0]["income_receipts"], figures[0]["zatsu_crypto_total"]) == ("7700600", "700000", "2312364")
    review = [(r["event"], r["code"]) for r in read("jp_review.csv")]
    assert review == [("2025:BTC", "jp_revenue_over_3m_classification")]

    workbook = tmp_path / "out" / "full.xlsx"
    data = run("reports", "export-xlsx", *scope, "--file", str(workbook))["data"]
    assert {"jp_keisansho", "jp_disposals", "jp_income_receipts", "jp_return"} <= set(data["sheets"])
    strings = zipfile.ZipFile(workbook).read("xl/sharedStrings.xml").decode()
    assert NOTICE in strings and "暗号資産の計算書" in strings

    first, second = tmp_path / "out" / "a.pdf", tmp_path / "out" / "b.pdf"
    run("reports", "export-pdf", *scope, "--file", str(first))
    run("reports", "export-pdf", *scope, "--file", str(second))
    pdf = first.read_bytes()
    assert pdf.startswith(b"%PDF-") and pdf == second.read_bytes()
    # The Japanese text is set in the bundled Noto Sans JP, not replaced.
    assert b"NotoSansJP" in pdf


def test_moving_average_rounds_the_book_up(tmp_path):
    root, run, scope, _ = _book(tmp_path, ("--gains-algorithm", "JP_MOVING_AVERAGE"))
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        costs = [row[0] for row in conn.execute(
            "SELECT cost_basis_exact FROM journal_entries WHERE entry_type IN ('disposal','transfer_fee') ORDER BY occurred_at")]
    # After the purchases: 16,000,000 for 3 BTC. The fee leaves 2.9999 BTC
    # at ROUNDUP(5,333,333.33… × 2.9999) = 15,999,467; the mining adds
    # 700,000 (16,699,467 for 3.0999); the sale leaves 2.0999 BTC at
    # ROUNDUP(16,699,467 ÷ 3.0999 × 2.0999) = 11,312,369, so the sale costs
    # 5,387,098.
    assert costs[0] == "533"
    assert costs[1] == "5387098.0"


def test_2027_is_refused_and_airdrops_wait_for_review(tmp_path):
    hot = HOT[:2] + [
        "2025-07-01T01:00:00Z,drop-1,inbound,BTC,0.01000000,0.00000000,7000000,エアドロップ,airdrop",
        "2027-02-01T01:00:00Z,sell-2,outbound,BTC,0.50000000,0.00000000,9000000,売却,sell",
    ]
    root, run, scope, _ = _book(tmp_path, hot=hot, cold=[])
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        held = [
            (reason, json.loads(detail).get("code"))
            for reason, detail in conn.execute("SELECT reason,detail_json FROM journal_quarantines")
        ]
    # The refused receipt (the shared refusal, with Japan's code) leaves the
    # later 2027 sale's lots uncertain.
    assert held == [
        ("acquisition_valuation_unsupported", "jp_airdrop_requires_review"),
        ("basis_provenance_incomplete", None),
    ]
