"""Freeze Python's Bark (BTC-ARK) row booking for the native port.

Sections: `bark_records` per row (records or the refusal), `payment_keys`
per BTCPay payment id, and `btc_msat` per amount.
"""
import argparse
import json
from decimal import Decimal
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.btcpay import bark  # noqa: E402
from kassiber.errors import AppError  # noqa: E402

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_btcpay_bark.json"
HASH = "b1" * 32
TXID = "a1" * 32


def outcome(func, *args, **kwargs):
    try:
        return {"ok": func(*args, **kwargs)}
    except AppError as error:
        return {"error": {"code": error.code, "message": str(error), "hint": error.hint, "details": error.details}}
    except Exception as error:  # noqa: BLE001 - frozen as the reference behaves
        return {"exception": type(error).__name__}


def row(row_id="r1", kind="ark-receive", amount="0.0001", **extra):
    base = {"id": row_id, "kind": kind, "amount": amount, "timestamp": 1767312000, "status": "Confirmed", "confirmations": 1}
    base.update(extra)
    return {key: value for key, value in base.items() if value is not ...}


ROWS = [
    row(),
    row(fee="0.00000010", kind="lightning-receive", paymentHash=HASH.upper(), labels={"invoice": {"type": "invoice", "ref": "inv-1"}}),
    row(kind="ark-receive", outpoint=f"{TXID.upper()}:3", transactionHash=TXID, movementId=7, comment=" sale ", labels={"invoice": {"ref": "", "id": "inv-2"}, "raw": {"type": "raw", "text": "Shop"}}),
    row(kind="onchain-receive", amount="0.001", fee=None),
    row(kind="ark-send", amount="-0.00001000", fee="0"),
    row(kind="lightning-send", amount="-0.00003020", fee="0.00000020", paymentHash=HASH),
    row(kind="offboard", amount="-0.00000300", fee="0.00000300"),
    row(kind="send-onchain", amount="-0.0002", fee="0.00000150", transactionHash=TXID),
    row(kind="refresh", amount="-0.0000005", fee="0.0000005"),
    row(kind="board", amount="-0.000005", fee="0.000005", transactionHash=TXID),
    row(kind="exit", amount="0", fee=None),
    row(kind="exit", amount="-0.001", fee="0.00000200"),
    row(kind="exit-fee", amount="-0.00000400", fee="0.00000400"),
    row(kind="exit-claim", amount="-0.00000100", fee="0.00000100"),
    row(kind="onchain-internal", amount="-0.00000141", fee="0.00000141"),
    row(kind="onchain-send", amount="-0.0005", fee="0.00000141"),
    row(kind="lightning-receive", amount="0.0001", movementId=True, paymentHash="not-hex"),
    row(kind="ark-receive", amount=0.00012345),
    row(kind="ark-receive", amount=12345),
    row(kind="ark-receive", amount="1_000E-11"),
    row(kind="ark-receive", amount=" 0.0001 "),
    row(kind="ark-receive", amount="１２E-8"),
    row(kind="ark-receive", amount="0.0001000000000000000000000000000000000"),
    # Refusals.
    row(kind="unknown"),
    row(kind="teleport"),
    row(kind=None),
    row(kind=" "),
    row(kind="refresh", amount="0.0001"),
    row(kind="ark-receive", amount="-0.0001"),
    row(kind="board", amount="-0.0001", fee="0.0000001"),
    row(kind="ark-send", amount="-0.0001", fee="0.0002"),
    row(kind="exit", amount="0", fee="0.00000010"),
    row(kind="ark-send", amount="-0.0001", fee="-0.00000001"),
    row(kind="ark-send", amount="-0.000000000001"),
    row(kind="ark-receive", amount="0.000000000015"),
    row(kind="ark-receive", amount="1.0000000000001"),
    row(kind="ark-send", amount="-0.0001", paymentHash=HASH),
    row(kind="ark-send", amount="1e-12"),
    row(kind="ark-send", amount="abc"),
    row(kind="ark-send", amount=True),
    row(kind="ark-send", amount=None),
    row(kind="ark-send", amount=[]),
    row(kind="ark-receive", amount="NaN"),
    row(kind="ark-receive", amount="Infinity"),
    row(kind="ark-receive", amount="21000001"),
    row(kind="ark-receive", amount="1E+9"),
    row(kind="ark-receive", amount="21000000"),
    row(kind="ark-receive", amount="-21000000.00000000001"),
    row(kind="ark-receive", amount="1e-999999"),
    row(kind="ark-receive", amount="0.0001", fee="x"),
    row(kind="ark-receive", timestamp=None),
    row(kind="ark-receive", timestamp="1767312000.9"),
    row(kind="ark-receive", timestamp="later"),
    row(kind="ark-receive", timestamp=True),
    row(row_id=None),
    row(row_id=" "),
    row(row_id=7),
    row(row_id="r\ud800"),
    "not a row",
    [],
]

PAYMENT_IDS = [
    f"{TXID}-0", f"{TXID.upper()}-007", f"{TXID}-", f"{TXID}-x", f"{TXID}-²", HASH, HASH.upper(), " " + HASH + " ",
    "inv-1#ark-movement-3", "inv-1#ark-movement-0003", "6CV5#ark-movement-2-0", "inv#ark-movement-2-01",
    "inv#ark-movement-2-", "inv#ark-movement-2-x", "inv#ark-movement--1", "#ark-movement-3", "inv#ark-movement-", "inv#ark-movement-٣",
    "a#b#ark-movement-4#ark-movement-5", "", None, 7, "abc", f"{TXID[:-1]}-0",
]

AMOUNTS = ["0.000000000015", "0.00000000002", "0.0001", "1e-11", "-0.00000000001", "21000000", "1E+3", "1_000", "-0", "0E-50", "１２.３４", 0.1, 1, True, None, "NaN", " 1 "]


def clean(value):
    if isinstance(value, Decimal):
        return str(value)
    if isinstance(value, dict):
        return {key: clean(item) for key, item in value.items()}
    if isinstance(value, list):
        return [clean(item) for item in value]
    return value


def records(item):
    result = outcome(bark.bark_records, item, store_id="S1")
    if "ok" in result:
        result["ok"] = [clean(record) for record in result["ok"]]
    return result


def render():
    sections = {
        "rows": [{"input": item, "records": records(item)} for item in ROWS],
        "payment_keys": [{"input": value, "keys": bark.payment_keys(value)} for value in PAYMENT_IDS],
        "amounts": [{"input": value, "msat": outcome(bark.btc_msat, value, field="amount", row_id="r")} for value in AMOUNTS],
    }
    return json.dumps(sections, ensure_ascii=True, indent=1) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    rendered = render()
    if parser.parse_args().check:
        if DEST.read_text() != rendered:
            raise SystemExit("Bark vectors differ from Python")
    else:
        DEST.write_text(rendered)
    print(f"Bark: {sum(len(v) for v in json.loads(rendered).values())} reference cases")


if __name__ == "__main__":
    main()
