#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
EXCHANGE_WITHDRAW_TXID = "4e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766"
COLD_CONSOLIDATION_TXID = "6f1e2d3c4b5a69788776655443322110ffeeddccbbaa00998877665544332211"


def _write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _run_cli(data_root: Path, *args: str) -> dict[str, Any]:
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(
            f"CLI did not return JSON for {args!r}\nstdout={result.stdout}\nstderr={result.stderr}"
        ) from exc
    if result.returncode != 0:
        raise SystemExit(f"CLI failed for {args!r}: {json.dumps(payload, indent=2)[:1200]}")
    return payload["data"]


def _tx_id(data_root: Path, wallet: str, external_id: str) -> str:
    payload = _run_cli(
        data_root,
        "transactions",
        "list",
        "--workspace",
        "Demo",
        "--profile",
        "Austria Demo",
        "--wallet",
        wallet,
        "--limit",
        "20",
    )
    rows = payload.get("transactions") if isinstance(payload, dict) else payload
    matches = [row for row in rows if row["external_id"] == external_id]
    if len(matches) != 1:
        raise SystemExit(f"Expected one transaction {external_id!r} in wallet {wallet!r}, found {len(matches)}")
    return matches[0]["id"]


def build_demo(output_pdf: Path, output_json: Path, data_root: Path | None = None) -> dict[str, Any]:
    data_root = data_root or Path(tempfile.mkdtemp(prefix="kassiber-source-funds-demo-data-"))
    inputs = Path(tempfile.mkdtemp(prefix="kassiber-source-funds-demo-inputs-"))
    evidence = inputs / "fictitious-exchange-statement.txt"
    _write(
        evidence,
        "Fictitious exchange statement for a demo Mittelherkunftsnachweis.\n"
        "Customer: Example Person\n"
        "Purchase: 0.30000000 BTC funded from EUR salary savings.\n",
    )
    csvs = {
        "exchange.csv": (
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2025-03-01T09:00:00Z,{EXCHANGE_WITHDRAW_TXID},outbound,BTC,0.30010000,0.00010000,55000,"
            "Fictitious exchange withdrawal to self custody\n"
        ),
        "cold.csv": (
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2025-03-01T09:30:00Z,{EXCHANGE_WITHDRAW_TXID},inbound,BTC,0.30000000,0,55000,"
            "Received in cold storage\n"
            f"2025-11-06T08:45:00Z,{COLD_CONSOLIDATION_TXID},outbound,BTC,0.15005000,0.00005000,70000,"
            "Reviewed consolidation spend from cold storage\n"
        ),
        "target.csv": (
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2025-11-06T09:10:00Z,{COLD_CONSOLIDATION_TXID},inbound,BTC,0.15000000,0,70000,"
            "Fictitious target broker deposit\n"
        ),
    }
    for name, content in csvs.items():
        _write(inputs / name, content)

    _run_cli(data_root, "init")
    _run_cli(data_root, "workspaces", "create", "Demo")
    _run_cli(
        data_root,
        "profiles",
        "create",
        "--workspace",
        "Demo",
        "--fiat-currency",
        "EUR",
        "--tax-country",
        "at",
        "Austria Demo",
    )
    for wallet, csv_name in (
        ("Example Exchange", "exchange.csv"),
        ("Cold Storage", "cold.csv"),
        ("Target Broker", "target.csv"),
    ):
        _run_cli(
            data_root,
            "wallets",
            "create",
            "--workspace",
            "Demo",
            "--profile",
            "Austria Demo",
            "--label",
            wallet,
            "--kind",
            "custom",
        )
        _run_cli(
            data_root,
            "wallets",
            "import-csv",
            "--workspace",
            "Demo",
            "--profile",
            "Austria Demo",
            "--wallet",
            wallet,
            "--file",
            str(inputs / csv_name),
        )

    exchange_out = _tx_id(data_root, "Example Exchange", EXCHANGE_WITHDRAW_TXID)
    cold_in = _tx_id(data_root, "Cold Storage", EXCHANGE_WITHDRAW_TXID)
    cold_out = _tx_id(data_root, "Cold Storage", COLD_CONSOLIDATION_TXID)
    target_in = _tx_id(data_root, "Target Broker", COLD_CONSOLIDATION_TXID)

    attachment = _run_cli(
        data_root,
        "attachments",
        "add",
        "--workspace",
        "Demo",
        "--profile",
        "Austria Demo",
        "--transaction",
        exchange_out,
        "--file",
        str(evidence),
        "--label",
        "Fictitious EUR purchase statement",
    )
    source = _run_cli(
        data_root,
        "source-funds",
        "sources",
        "create",
        "--workspace",
        "Demo",
        "--profile",
        "Austria Demo",
        "--type",
        "fiat_purchase",
        "--label",
        "Fictitious EUR salary-funded BTC purchase",
        "--asset",
        "BTC",
        "--amount",
        "0.30000000",
        "--fiat-currency",
        "EUR",
        "--fiat-value",
        "16500",
        "--acquired-at",
        "2025-02-20T10:00:00Z",
        "--attachment",
        attachment["id"],
    )
    links = (
        ("--from-source", source["id"], exchange_out, "manual_source", "0.15005000", None),
        ("--from-transaction", exchange_out, cold_in, "self_transfer", "0.15005000", "0.15005000"),
        ("--from-transaction", cold_in, cold_out, "self_transfer", "0.15005000", "0.15005000"),
        ("--from-transaction", cold_out, target_in, "self_transfer", "0.15000000", "0.15005000"),
    )
    for from_arg, from_ref, to_ref, link_type, allocation, from_amount in links:
        args = [
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Demo",
            "--profile",
            "Austria Demo",
            from_arg,
            from_ref,
            "--to-transaction",
            to_ref,
            "--type",
            link_type,
            "--allocation-amount",
            allocation,
            "--allocation-policy",
            "explicit",
        ]
        if from_amount:
            args.extend(["--from-amount", from_amount])
        _run_cli(data_root, *args)

    report = _run_cli(
        data_root,
        "reports",
        "source-funds",
        "--workspace",
        "Demo",
        "--profile",
        "Austria Demo",
        "--target-transaction",
        target_in,
        "--target-amount",
        "0.15000000",
        "--purpose",
        "planned_exchange_sale",
        "--planned-destination",
        "Example Broker Austria",
        "--planned-note",
        "Fictitious AT/EUR source-of-funds demo report.",
        "--reveal-mode",
        "standard",
        "--case-label",
        "Fictitious AT/EUR source-of-funds demo",
        "--save-case",
    )
    output_json.parent.mkdir(parents=True, exist_ok=True)
    output_json.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    export = _run_cli(
        data_root,
        "reports",
        "export-source-funds-pdf",
        "--workspace",
        "Demo",
        "--profile",
        "Austria Demo",
        "--case",
        report["case"]["id"],
        "--file",
        str(output_pdf),
    )
    return {
        "pdf": str(output_pdf),
        "json": str(output_json),
        "data_root": str(data_root),
        "inputs": str(inputs),
        "case_id": report["case"]["id"],
        "snapshot_hash": report["case"]["snapshot_hash"],
        "pages": export["pages"],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate a fictitious AT/EUR source-of-funds PDF demo.")
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("/tmp/kassiber-source-funds-demo.pdf"),
        help="PDF output path.",
    )
    parser.add_argument(
        "--json-output",
        type=Path,
        default=Path("/tmp/kassiber-source-funds-demo.json"),
        help="Saved report snapshot JSON output path.",
    )
    parser.add_argument("--data-root", type=Path, help="Optional Kassiber data root to populate.")
    args = parser.parse_args()
    result = build_demo(args.output, args.json_output, args.data_root)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
