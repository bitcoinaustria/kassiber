"""Targeted, explicitly synthetic Mittelherkunft cases for the persistent demo."""
from __future__ import annotations

import argparse
import csv
import json
import os
import tempfile
import sqlite3
import subprocess
import sys
from decimal import Decimal
from pathlib import Path

LABEL = "SYNTHETIC — Strike purchase to own wallet"
DESCRIPTION = (
    "Synthetic regtest demonstration, not evidence of a real exchange purchase. "
    "The attached generated Strike CSV describes the fixture purchase; the wallet "
    "receipt comes from the local Bitcoin Core regtest chain."
)


def purchase_fixture(path: Path) -> dict[str, str]:
    with path.open(newline="", encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle))
    purchases = [row for row in rows if row["Reference"] == "demo-buy"]
    withdrawals = [row for row in rows if row["Reference"] == "demo-withdraw"]
    if len(purchases) != 1 or len(withdrawals) != 1:
        raise ValueError("The source-funds demo requires the complete generated Strike fixture.")
    buy, withdrawal = purchases[0], withdrawals[0]
    if (Decimal(buy["Amount BTC"]) != Decimal("0.01")
            or Decimal(buy["Amount EUR"]) != Decimal("-1000")
            or Decimal(buy["Fee EUR"]) != Decimal("1")
            or Decimal(withdrawal["Amount BTC"]) != Decimal("-0.006")
            or Decimal(withdrawal["Fee BTC"]) != Decimal("0.0001")):
        raise ValueError("Unexpected synthetic purchase or withdrawal amount.")
    if Decimal("0.006") + Decimal(withdrawal["Fee BTC"]) > Decimal(buy["Amount BTC"]):
        raise ValueError("The fixture withdrawal exceeds its source.")
    return {"acquired_at": buy["Date & Time (UTC)"], "txid": withdrawal["Transaction Hash"]}


def seed(home: Path) -> dict:
    home = home.resolve()
    summary = json.loads((home / "demo-summary.json").read_text())["data"]
    if summary["scenario"] != "full-accounting-v1" or Path(summary["data_root"]).resolve() != home / "data":
        raise ValueError("Expected a generated full-accounting regtest demo.")
    csv_path = home / "exports/exchange/strike-complete.csv"
    fixture = purchase_fixture(csv_path)
    receipt_path = home / "source-funds-seed.json"
    if receipt_path.exists():
        previous = json.loads(receipt_path.read_text())
        with sqlite3.connect(f"file:{home / 'data/kassiber.sqlite3'}?mode=ro", uri=True) as conn:
            source_exists = conn.execute("SELECT 1 FROM source_funds_sources WHERE id=?", (previous["source_id"],)).fetchone()
            cases_exist = conn.execute("SELECT count(*) FROM source_funds_cases WHERE id IN (?,?)",
                                       (previous["ready_case_id"], previous["missing_case_id"])).fetchone()[0]
        if not previous.get("synthetic") or not source_exists or cases_exist != 2:
            raise ValueError("Incomplete source-funds demo seed; rebuild the dedicated demo book.")
        # A persistent demo is editable. Never overwrite reviewed user changes on restart.
        return previous
    profile = summary["exchange_cases"]["profile"]["id"]
    workspace = summary["exchange_cases"]["profile"]["workspace_id"]
    scope = ["--workspace", workspace, "--profile", profile]

    def cli(*args: str, target_scope: list[str] | None = None):
        result = subprocess.run(
            [sys.executable, "-m", "kassiber", "--data-root", str(home / "data"), "--machine",
             *args, *(scope if target_scope is None else target_scope)],
            capture_output=True, text=True,
        )
        if result.returncode:
            raise RuntimeError(f"Synthetic source-funds seed command failed: {result.stdout or result.stderr}")
        return json.loads(result.stdout)["data"]

    with sqlite3.connect(f"file:{home / 'data/kassiber.sqlite3'}?mode=ro", uri=True) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute("SELECT id, external_id, direction, amount FROM transactions WHERE profile_id=?", (profile,)).fetchall()
        buy = next(row for row in rows if row["external_id"] == "strike:demo-buy")
        withdrawal = next(row for row in rows if row["external_id"] == fixture["txid"] and row["direction"] == "outbound")
        receipt = next(row for row in rows if row["external_id"] == fixture["txid"] and row["direction"] == "inbound")
        if (buy["amount"], withdrawal["amount"], receipt["amount"]) != (1000000000, 600000000, 600000000):
            raise ValueError("Observed demo amounts do not match the independent CSV fixture.")
        # A real generated receipt whose economic origin has deliberately not been authored.
        unresolved = conn.execute(
            "SELECT t.id, t.profile_id FROM transactions t JOIN profiles p ON p.id=t.profile_id "
            "WHERE p.label='Full Accounting' AND t.direction='inbound' AND t.amount>0 "
            "ORDER BY t.occurred_at, t.id LIMIT 1"
        ).fetchone()
        sources = conn.execute("SELECT id FROM source_funds_sources WHERE profile_id=? AND label=?", (profile, LABEL)).fetchall()
        attachments = conn.execute("SELECT id FROM attachments WHERE transaction_id=? AND label=?", (buy["id"], LABEL)).fetchall()
    if len(sources) > 1 or len(attachments) > 1:
        raise ValueError("Ambiguous duplicate synthetic source fixture.")
    attachment_id = attachments[0]["id"] if attachments else cli(
        "attachments", "add", "--transaction", buy["id"], "--file", str(csv_path), "--label", LABEL,
    )["id"]
    source_id = sources[0]["id"] if sources else cli(
        "source-funds", "sources", "create", "--type", "fiat_purchase", "--label", LABEL,
        "--asset", "BTC", "--amount", "0.01", "--fiat-currency", "EUR", "--fiat-value", "1001",
        "--acquired-at", fixture["acquired_at"], "--description", DESCRIPTION, "--attachment", attachment_id,
    )["id"]
    common = ["--state", "reviewed", "--confidence", "strong", "--method", "synthetic_fixture_review",
              "--explanation", DESCRIPTION, "--attachment", attachment_id]
    cli("source-funds", "links", "create", "--from-source", source_id, "--to-transaction", buy["id"],
        "--type", "manual_source", "--allocation-amount", "0.01", *common)
    cli("source-funds", "links", "create", "--from-transaction", buy["id"], "--to-transaction", withdrawal["id"],
        "--type", "exchange_transfer", "--allocation-amount", "0.006", "--from-amount", "0.006", *common)
    # Explicit fixture review combines the generated CSV reference and the real
    # Core receipt. It remains authored provenance, never native chain authority.
    cli("source-funds", "links", "create", "--from-transaction", withdrawal["id"], "--to-transaction", receipt["id"],
        "--type", "exchange_transfer", "--allocation-amount", "0.006", "--from-amount", "0.006", *common)
    ready = cli("reports", "source-funds", "--target-transaction", receipt["id"])
    if not ready["explain_gates"]["exportable"]:
        raise ValueError("The targeted synthetic proof still has unresolved export gates.")
    ready = cli("reports", "source-funds", "--target-transaction", receipt["id"], "--save-case", "--case-label", LABEL)
    pdf = home / "exports/source-funds-synthetic.pdf"
    cli("reports", "export-source-funds-pdf", "--case", ready["case"]["id"], "--file", str(pdf))
    if unresolved is None:
        raise ValueError("Missing unresolved sibling receipt.")
    missing = cli("reports", "source-funds", "--target-transaction", unresolved["id"], "--save-case",
                  "--case-label", "SYNTHETIC — Origin evidence still missing",
                  target_scope=["--workspace", workspace, "--profile", unresolved["profile_id"]])
    if missing["explain_gates"]["exportable"]:
        raise ValueError("The missing-evidence case unexpectedly became exportable.")
    result = {"synthetic": True, "source_id": source_id, "source_amount_msat": 1000000000,
              "target_amount_msat": 600000000, "target_transaction_id": receipt["id"],
              "profile_id": profile, "ready_case_id": ready["case"]["id"], "pdf": str(pdf),
              "missing_case_id": missing["case"]["id"], "missing_target_transaction_id": unresolved["id"],
              "missing_profile_id": unresolved["profile_id"]}
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=home, prefix=".source-funds-seed-", delete=False) as handle:
            temporary = Path(handle.name)
            handle.write(json.dumps(result, indent=2) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, receipt_path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--demo-home", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(seed(args.demo_home)))


if __name__ == "__main__":
    main()
