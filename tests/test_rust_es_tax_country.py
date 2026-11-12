"""A Spanish book end to end on the Rust backend (preview country).

The book is built with the CLI as a user would: a Spanish profile, two
wallets, CSV imports, a reviewed move between them, journal processing, and
the full report as PDF, XLSX and CSV in Spanish. The figures are worked by
hand below (docs/reference/tax-country-es.md): FIFO across every wallet, a
move's fee units as a transmission of their own, a gift's loss that does not
count, lending interest as a capital-mobiliario yield, and a sale at 00:30 on
1 January in Madrid that belongs to the new year. Python does not know the
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

NOTICE = "Aún no revisado por un asesor fiscal de este país: compruebe los resultados antes de presentar la declaración."
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
MOVE = "%064x" % 9
HOT = [
    "2024-01-10T10:00:00Z,buy-1,inbound,BTC,1.00000000,0.00000000,30000,Compra,buy",
    "2024-06-01T10:00:00Z,buy-2,inbound,BTC,0.50000000,0.00000000,60000,Compra,buy",
    # The move's amount is what arrives; its fee units leave beside it.
    f"2024-08-01T10:00:00Z,{MOVE},outbound,BTC,0.40000000,0.00010000,50000,A frío,transfer",
    "2025-02-01T10:00:00Z,sell-1,outbound,BTC,0.50000000,0.00000000,90000,Venta,sell",
    "2025-03-15T10:00:00Z,int-1,inbound,BTC,0.00100000,0.00000000,85000,Intereses,interest",
    "2025-04-01T10:00:00Z,gift-1,outbound,BTC,0.10000000,0.00000000,20000,Regalo,gift",
]
COLD = [
    f"2024-08-01T10:00:00Z,{MOVE},inbound,BTC,0.40000000,0.00000000,50000,Desde caliente,transfer",
    "2025-12-31T23:30:00Z,sell-2,outbound,BTC,0.20000000,0.00000000,100000,Venta,sell",
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


def _book(tmp_path, hot=HOT, cold=COLD):
    root = tmp_path / "data"
    env = _env(tmp_path)

    def run(*args, ok=True):
        result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(root), *args],
                                env=env, capture_output=True, text=True, timeout=300)
        payload = json.loads(result.stdout)
        if ok:
            assert result.returncode == 0 and payload["kind"] != "error", result.stdout + result.stderr
        return payload

    scope = ["--workspace", "Main", "--profile", "Libro"]
    run("init")
    run("workspaces", "create", "Main")
    profile = run("profiles", "create", "--workspace", "Main", "--tax-country", "es", "Libro")["data"]
    for wallet, rows in (("Caliente", hot), ("Frio", cold)):
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
            for wallet in ("Caliente", "Frio")
        ]
    if all(ids):
        run("transfers", "pair", *scope, "--tx-out", ids[0][0], "--tx-in", ids[1][0], "--policy", "carrying-value")
    return root, run, scope, profile


def test_profile_defaults_follow_the_registry(tmp_path):
    _, run, scope, profile = _book(tmp_path)
    assert (profile["fiat_currency"], profile["gains_algorithm"], profile["cost_basis_pool_scope"]) == ("EUR", "FIFO", "global")
    details = run("profiles", "get", *scope)["data"]
    assert details["allowed_cost_basis_pool_scopes"] == ["global"]
    refused = run("profiles", "create", "--workspace", "Main", "--tax-country", "es",
                  "--gains-algorithm", "LIFO", "Otro", ok=False)
    assert refused["error"]["code"] == "validation"
    assert refused["error"]["hint"] == "Choose one of: FIFO"


def test_global_fifo_journal_and_spanish_report(tmp_path):
    root, run, scope, _ = _book(tmp_path)
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        entries = conn.execute(
            "SELECT je.occurred_at,w.label,je.entry_type,je.quantity,je.proceeds_exact,je.cost_basis_exact,"
            "je.gain_loss_exact,je.calculation_json IS NOT NULL FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id "
            "WHERE je.entry_type IN ('disposal','transfer_fee','income') ORDER BY je.occurred_at"
        ).fetchall()
        gift = conn.execute(
            "SELECT je.calculation_json FROM journal_entries je JOIN transactions t ON t.id=je.transaction_id "
            "WHERE t.external_id='gift-1'"
        ).fetchone()[0]
        quarantines = conn.execute("SELECT COUNT(*) FROM journal_quarantines").fetchone()[0]
    assert quarantines == 0
    # The move's fee units are a permuta of their own: 0.0001 at 50,000
    # against FIFO's 30,000 lot. FIFO is global: each sale takes the oldest
    # lot whichever wallet sells. The gift's loss is computed and marked.
    assert [tuple(map(str, row)) for row in entries] == [
        ("2024-08-01T10:00:00Z", "Caliente", "transfer_fee", "-10000000", "5.0000", "3.0000", "2.0000", "0"),
        ("2025-02-01T10:00:00Z", "Caliente", "disposal", "-50000000000", "45000.0", "15000.0", "30000.0", "0"),
        ("2025-03-15T10:00:00Z", "Caliente", "income", "100000000", "85.000", "0", "85.000", "0"),
        ("2025-04-01T10:00:00Z", "Caliente", "disposal", "-10000000000", "2000.0", "3000.0", "-1000.0", "1"),
        ("2025-12-31T23:30:00Z", "Frio", "disposal", "-20000000000", "20000.0", "6000.0", "14000.0", "0"),
    ]
    assert json.loads(gift)["loss_deductible"] is False

    target = tmp_path / "out" / "full.csv"
    run("reports", "export-csv", *scope, "--file", str(target))
    bundle = zipfile.ZipFile(target.with_suffix(".zip"))
    read = lambda name: list(csv.DictReader(io.StringIO(bundle.read(name).decode())))
    assert read("summary.csv")[0] == {"section": "preview", "key": "notice", "value": NOTICE, "unit": ""}
    assert json.loads(bundle.read("manifest.json"))["preview_notice"] == NOTICE
    rows = [(r["tax_year"], r["date"], r["kind"], r["contraprestacion"], r["valor_transmision"],
             r["valor_adquisicion"], r["ganancia"], r["computable"], r["wallet"])
            for r in read("es_transmisiones.csv")]
    assert rows == [
        ("2024", "2024-08-01", "Comisión de red", "Otro bien o servicio", "5.00", "3.00", "2.00", "Sí", "Caliente"),
        ("2025", "2025-02-01", "Venta", "Moneda de curso legal", "45000.00", "15000.00", "30000.00", "Sí", "Caliente"),
        ("2025", "2025-04-01", "Donación", "Sin contraprestación", "2000.00", "3000.00", "-1000.00", "No", "Caliente"),
        # 23:30 UTC on 31 December is 00:30 on 1 January in Madrid.
        ("2026", "2026-01-01", "Venta", "Moneda de curso legal", "20000.00", "6000.00", "14000.00", "Sí", "Frio"),
    ]
    totals = {r["tax_year"]: r for r in read("es_casilla_1804_totals.csv")}
    assert (totals["2025"]["ganancias"], totals["2025"]["perdidas"], totals["2025"]["non_computable_losses"]) == ("30000.00", "0", "1000.00")
    assert totals["2026"]["saldo"] == "14000.00"
    income = read("es_rendimientos.csv")
    assert [(r["kind"], r["casilla"], r["base"], r["value"]) for r in income] == [
        ("interest", "Art. 25.2 (por confirmar)", "Ahorro", "85.000"),
    ]
    ahorro = {r["tax_year"]: r for r in read("es_integracion_ahorro.csv")}
    assert (ahorro["2025"]["rcm"], ahorro["2025"]["base"]) == ("85.00", "30085.00")
    flags = [(r["code"]) for r in read("es_revision.csv")]
    assert flags == ["es_gift_loss_not_computable", "es_lending_casilla_unconfirmed"]

    workbook = tmp_path / "out" / "full.xlsx"
    data = run("reports", "export-xlsx", *scope, "--file", str(workbook))["data"]
    assert {"es_transmisiones", "es_casilla_1804", "es_rendimientos", "es_integracion_ahorro"} <= set(data["sheets"])
    strings = zipfile.ZipFile(workbook).read("xl/sharedStrings.xml").decode()
    assert NOTICE in strings and "Valor de transmisión" in strings

    first, second = tmp_path / "out" / "a.pdf", tmp_path / "out" / "b.pdf"
    run("reports", "export-pdf", *scope, "--file", str(first))
    run("reports", "export-pdf", *scope, "--file", str(second))
    assert first.read_bytes().startswith(b"%PDF-")
    assert first.read_bytes() == second.read_bytes()


def test_unsettled_kinds_are_quarantined_with_spains_codes(tmp_path):
    hot = HOT[:2] + [
        "2025-05-01T10:00:00Z,lost-1,outbound,BTC,0.10000000,0.00000000,,Llaves perdidas,lost",
        "2025-06-01T10:00:00Z,fork-1,inbound,BTC,0.10000000,0.00000000,100,Bifurcación,hardfork",
    ]
    root, run, scope, _ = _book(tmp_path, hot=hot, cold=[])
    run("journals", "process", *scope)
    with sqlite3.connect(root / "kassiber.sqlite3") as conn:
        held = sorted(
            (reason, json.loads(detail)["code"])
            for reason, detail in conn.execute("SELECT reason,detail_json FROM journal_quarantines")
        )
        disposals = conn.execute("SELECT COUNT(*) FROM journal_entries WHERE entry_type IN ('disposal','income')").fetchone()[0]
    # The shared refusals: a receipt held as an unsupported valuation, an
    # outflow as a non-sale disposal, each with Spain's code.
    assert held == [
        ("acquisition_valuation_unsupported", "es_hardfork_requires_review"),
        ("non_sale_disposal_kind", "es_lost_requires_review"),
    ]
    assert disposals == 0
