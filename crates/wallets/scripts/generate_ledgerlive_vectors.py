#!/usr/bin/env python3
"""Freeze Python Ledger Live parsing and normalized transaction fields."""
import argparse
import csv
import io
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import load_import_records
from kassiber.msat import btc_to_msat
from generate_exchange_vectors import clean

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_ledgerlive_imports.json"
BASE = {
    "Operation Date": "2025-01-02T01:02:03+01:00",
    "Currency Ticker": "BTC",
    "Operation Type": "OUT",
    "Operation Amount": "-0.00200000",
    "Operation Fees": "0.00001000",
    "Operation Hash": "ledger-out",
    "Account Name": "Bitcoin",
    "Account xpub": "xpub-synthetic-secret",
    "Account xpub fingerprint": "synthetic-fingerprint",
    "xpub": "xpub-synthetic-other",
    "Countervalue Ticker": "EUR",
    "Countervalue at Operation Date": "100.00",
}


def inputs():
    header = list(BASE)
    cases = {"empty": "", "header-only": ",".join(header) + "\n",
             "blank-header": "\nx\n", "utf8": b"\xff",
             "utf16": ",".join(header).encode("utf-16")}

    def add(name, changes=None, *, columns=None, rows=None, delimiter=","):
        columns = header if columns is None else columns
        stream = io.StringIO(newline="")
        writer = csv.writer(stream, delimiter=delimiter)
        writer.writerow(columns)
        for row in rows if rows is not None else [{**BASE, **(changes or {})}]:
            writer.writerow([row.get(key, "") for key in columns])
        cases[name] = stream.getvalue()

    add("base")
    hashless = {**BASE, "Operation Hash": ""}
    add("hashless-repeat", rows=[hashless, hashless, {**hashless, "Operation Type": "IN", "Operation Amount": "0.002"}, hashless])
    cases["bom"] = "\ufeff" + cases["base"]
    cases["double-bom"] = "\ufeff" + cases["bom"]
    cases["bom-invalid-utf8"] = b"\xef\xbb\xbf\xff"
    add("tab", delimiter="\t")
    add("semicolon", delimiter=";")
    for key in header:
        add("missing-" + key, columns=[k for k in header if k != key])
        for blank in ("", " ", "\t"):
            add("blank-" + key + repr(blank), {key: blank})
    for name, transform in (
        ("lower", str.lower), ("upper", str.upper),
        ("spaces", lambda k: " \t" + k + "  "),
        ("unicode", lambda k: k.replace(" ", "\u00a0\t")),
        ("casefold", lambda k: k.replace("s", "ſ").replace("fi", "ﬁ")),
    ):
        add("headers-" + name, columns=[transform(k) for k in header],
            rows=[{transform(k): v for k, v in BASE.items()}])
    for key in ("Operation Amount", "Operation Fees"):
        for value in ("0", "-0", "-1", "1e-12", "0.000000000005",
                      "0.000000000015", "0.0000000000049",
                      "1.23456789012345678901234567890123456", "($1,234.50)",
                      " 0.002 LBTC ", "１２.３４", "١_٢e-٣", "NaN", "-NaN42",
                      "sNaN", "Infinity", "-Infinity", "bad", "1e309"):
            add("number-" + key + value, {key: value})
    for value in ("IN", "receive", "RECEIVED", "OUT", "send", "SENT", " ſent ", "unknown", " "):
        add("operation-" + value, {"Operation Type": value})
    for value in ("BTC", "btc", " XBT ", "XXBT", "LBTC", "L-BTC", "ETH", "USD", ""):
        add("asset-" + value, {"Currency Ticker": value})
    for stamp in ("2025-01-02", "20250102", "2025-W01-4",
                  "2025-01-02T01:02:03.123456Z", "2025-01-02T00:00 Z",
                  "2025-01-02T01:02:03-04:30", "invalid", ""):
        add("date-" + stamp, {"Operation Date": stamp})
    for amount in ("", "0", "-0"):
        for fee in ("0", "-0.00001", "NaN", "sNaN", "Infinity", "bad"):
            add("zero-amount-" + amount + fee, {"Operation Amount": amount, "Operation Fees": fee})
    for amount in ("0.0000000000000000001", "1.23456789012345678901234567890123456789", "-0.00200000", "1E+3"):
        add("generated-id-" + amount, {"Operation Hash": "", "Operation Amount": amount})
    for extra in (["OPERATION HASH", "operation hash"], ["operation hash", "OPERATION HASH"],
                  ["Account xpub", " Account xpub "], [" Account xpub ", "Account xpub"]):
        add("collision-" + repr(extra), columns=header + extra,
            changes={extra[0]: "first", extra[1]: "last"})
    add("quoted", {"Account Name": 'note, "quoted"\r\nnext'})
    add("inbound-bad-fee", {"Operation Type": "IN", "Operation Fees": "bad"})
    add("signaling-nan-before-fee", {"Operation Amount": "sNaN", "Operation Fees": "bad"})
    add("quiet-nan-after-fee", {"Operation Amount": "NaN", "Operation Fees": "bad"})
    add("nonbitcoin-invalid", {"Currency Ticker": "ETH", "Operation Type": "bad", "Operation Amount": "bad"})
    add("zero-fee-only", {"Operation Amount": "0", "Operation Fees": "0.00001", "Operation Date": "invalid"})
    add("ignored-pricing", {"Countervalue Ticker": "USD", "Countervalue at Operation Date": "NaN"})
    add("skipped-row-index", rows=[{**BASE, "Currency Ticker": "ETH"}, {**BASE, "Operation Hash": ""}])
    add("repeat", rows=[BASE, BASE, {**BASE, "Account Name": "updated"}])
    add("failed-batch", rows=[BASE, {**BASE, "Operation Date": "invalid"}])
    cases["short-row"] = ",".join(header) + "\n2025-01-02,BTC,OUT,1\n"
    cases["extra-unheaded"] = cases["base"].rstrip() + ",discarded\n"
    cases["field-limit"] = cases["base"].rstrip() + "," + "x" * 131073 + "\n"
    from tests.test_cli_smoke import _LEDGERLIVE_CSV
    cases["smoke"] = _LEDGERLIVE_CSV
    return cases


def vectors():
    cases = []
    with tempfile.TemporaryDirectory(prefix="kassiber-ledgerlive-vectors-") as root:
        path = Path(root) / "input.csv"
        for name, content in inputs().items():
            data = content.encode() if isinstance(content, str) else content
            path.write_bytes(data)
            case = {"name": "ledgerlive:" + name, "format": "ledgerlive_csv",
                    **({"text": content} if isinstance(content, str) else {"bytes": list(data)})}
            try:
                records = []
                for row in load_import_records(str(path), "ledgerlive_csv"):
                    record = normalize_import_record(row, "file:ledgerlive_csv")
                    fingerprint = make_transaction_fingerprint(
                        "wallet", record["external_id"], record["occurred_at"], record["direction"],
                        record["asset"], record["amount"], record["fee"],
                    )
                    record["amount"] = btc_to_msat(record["amount"])
                    record["fee"] = btc_to_msat(record["fee"])
                    if any(not -(1 << 63) <= record[k] < (1 << 63) for k in ("amount", "fee")):
                        raise OverflowError("Python int too large to convert to SQLite INTEGER")
                    for key in ("exchange_evidence_match_by_economics",
                                "exchange_evidence_match_time_tolerance_seconds", "match_existing_ignore_fee"):
                        record.pop(key)
                    record["amount_includes_fee"] = int(record["amount_includes_fee"])
                    records.append({"fields": record, "fingerprint": fingerprint})
                case["records"] = clean(records)
            except Exception as exc:
                case["error"] = {"code": exc.code if isinstance(exc, AppError) else "internal_error",
                                 "message": str(exc), "hint": exc.hint if isinstance(exc, AppError) else None}
            cases.append(case)
    return cases


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), ensure_ascii=True, indent=2) + "\n"
    if args.check:
        if DEST.read_text() != text:
            raise SystemExit("Ledger Live import vectors drifted")
    else:
        DEST.write_text(text)
