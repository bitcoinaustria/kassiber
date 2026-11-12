"""Python reference vectors for the exchange API normalizers.

Each case feeds one JSON payload, as the fetchers assemble it from provider
pages, to `normalize_kraken_records`, `normalize_coinbase_records` or
`normalize_binance_records` and records the exact record dicts or the error.
`python crates/wallets/scripts/generate_exchange_api_vectors.py [--check]`.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

import kassiber  # noqa: F401,E402 - sets the process decimal context
from kassiber.core.exchange_imports import (  # noqa: E402
    normalize_binance_records,
    normalize_coinbase_records,
    normalize_kraken_records,
)
from kassiber.errors import AppError  # noqa: E402
from tests.rust_port.exchange_api import Binance, Coinbase, Kraken  # noqa: E402

DEST = ROOT / "crates/wallets/tests/vectors/python_exchange_api.json"
VALUES = ["NaN", "-NaN", "sNaN", "NaN12", "Infinity", "-inf", "1_000", "1__0", "in_f", "١٢", " 0.1 ", "1e400",
          "-0", "0", "0.005", "-0.01", "abc", "\ud800", "0.00000000000000000000000000000000000001", "123456789012345678901234567890.5",
          True, False, 0, 1, [], {"a": 1}, 1e300, -0.0, 0.1, 12345678901234567890123456789012345, None, ""]
TIMES = ["abc", 1e20, -1e20, 253402300800, 253402300799.9999999, -62135596800, -62135596801, 9.3e18, 1e17, 6.7e16,
         "NaN", "sNaN", "Infinity", "-Infinity", 0, False, True, "", None, [], "١", "1700000000.5", 1700000000.0000005]
TEXTS = ["", " ", "BTC", "btc", "XBT", "XXBT", "ZEUR", "zeur", "L-BTC", "lbtc", "EUR", "uſd", "ETH", "\ud800", "Zé12",
         0, 1, True, None, [], ["BTC"], {"code": "BTC"}, "A" * 64]


# The rows each field mutation starts from: one accepted row per history,
# plus the one that exercises fallbacks (missing ids, absent status).
ROWS = {"fiat_payments": [0, 2], "deposits": [0, 3], "withdrawals": [0, 1], "dividends": [0, 1]}


def run(exchange, payload):
    try:
        if exchange == "kraken":
            records = normalize_kraken_records(payload["ledgers"], payload["trades"])
        elif exchange == "coinbase":
            records = normalize_coinbase_records(
                [{"currency": code, "transactions": rows} for code, rows in payload])
        else:
            records = normalize_binance_records(payload)
    except AppError as error:
        return {"error": {"code": error.code, "message": str(error), "hint": error.hint,
                          "details": error.details, "retryable": error.retryable}}
    except Exception as error:  # noqa: BLE001 - the CLI's internal_error wrapping
        return {"error": {"code": "internal_error", "message": str(error) or type(error).__name__,
                          "hint": None, "details": None, "retryable": False}}
    return {"records": records}


def kraken_payload(fake):
    return {"ledgers": {"result": {"ledger": fake.ledgers, "count": len(fake.ledgers)}},
            "trades": {"result": {"trades": fake.trades, "count": len(fake.trades)}}}


def cases():
    out = []

    def add(exchange, name, payload):
        out.append({"exchange": exchange, "name": name, "payload": copy.deepcopy(payload)})

    kraken = Kraken()
    base = kraken_payload(kraken)
    add("kraken", "base", base)
    empty = copy.deepcopy(base)
    empty["trades"]["result"]["trades"] = {}
    add("kraken", "no-trades", empty)
    quirk = copy.deepcopy(empty)
    quirk["ledgers"]["result"]["ledger"]["Z"]["refid"] = "result"
    add("kraken", "no-trades-result-refid", quirk)
    no_ledgers = copy.deepcopy(base)
    no_ledgers["ledgers"]["result"]["ledger"] = {}
    add("kraken", "no-ledgers", no_ledgers)
    def kraken_single(row):
        payload = copy.deepcopy(base)
        payload["ledgers"]["result"]["ledger"] = {row: payload["ledgers"]["result"]["ledger"][row]}
        return payload

    for row, fields in [("W", ["amount", "fee", "time", "asset", "type"]), ("Z", ["amount", "fee", "time", "refid"]),
                        ("D", ["ledger_id", "type"])]:
        for field in fields:
            values = TIMES if field == "time" else TEXTS if field in {"asset", "type", "refid", "ledger_id"} else VALUES
            for index, value in enumerate(values):
                payload = kraken_single(row)
                payload["ledgers"]["result"]["ledger"][row][field] = value
                add("kraken", f"ledger-{row}-{field}-{index}", payload)
    for trade, fields in [("Z", ["cost", "fee", "price", "pair"]), ("A", ["cost", "fee", "price"])]:
        for field in fields:
            values = TEXTS + ["XBTZEUR", "XBT/ZUSD", "BTCEUR", "XBTCEUR", "XXBTEUR", "FOOZCHF", "FOO", "X"] \
                if field == "pair" else VALUES
            for index, value in enumerate(values):
                payload = kraken_single(trade)
                payload["trades"]["result"]["trades"][trade][field] = value
                add("kraken", f"trade-{trade}-{field}-{index}", payload)
    for index, kind in enumerate(["Deposit", " withdrawal ", "TRADE", "transfer", "", "ﬀ"]):
        payload = kraken_single("W")
        payload["ledgers"]["result"]["ledger"]["W"]["type"] = kind
        add("kraken", f"type-{index}", payload)
    payload = kraken_single("Z")
    payload["trades"]["result"]["trades"]["Z"] = {}
    add("kraken", "empty-trade", payload)

    coinbase = Coinbase()
    accounts = [["BTC", coinbase.transactions["btc-wallet"]], ["lbtc", coinbase.transactions["l btc/€"]]]
    add("coinbase", "base", accounts)
    add("coinbase", "empty", [])
    for code_index, code in enumerate(TEXTS):
        add("coinbase", f"account-currency-{code_index}", [[code, coinbase.transactions["btc-wallet"][3:4]]])
    for tx_index, tx in enumerate(accounts[0][1]):
        if tx_index in (4, 5):  # filtered by zero amount / non-Bitcoin currency
            continue
        for field in ["type", "id", "created_at", "createdAt", "network", "amount", "native_amount", "buy"]:
            for value_index, value in enumerate(TEXTS + VALUES[:12]):
                single = copy.deepcopy(tx)
                single[field] = value
                add("coinbase", f"tx-{tx_index}-{field}-{value_index}", [["BTC", [single]]])
        for field in ["amount", "native_amount"]:
            for sub in ["amount", "currency"]:
                for value_index, value in enumerate(VALUES if sub == "amount" else TEXTS):
                    single = copy.deepcopy(tx)
                    money = single.get(field)
                    if not isinstance(money, dict):
                        money = single[field] = {"amount": "1", "currency": "EUR"}
                    money[sub] = value
                    add("coinbase", f"tx-{tx_index}-{field}-{sub}-{value_index}", [["BTC", [single]]])
    for value_index, value in enumerate(VALUES):
        single = copy.deepcopy(accounts[0][1][0])
        single["buy"] = {"commission": value}
        add("coinbase", f"commission-{value_index}", [["BTC", [single]]])
    for value_index, value in enumerate(["2", "0", "NaN"]):
        # A sell's commission never raises its fiat value.
        single = copy.deepcopy(accounts[0][1][1])
        single["sell"] = {"commission": value}
        add("coinbase", f"sell-commission-{value_index}", [["BTC", [single]]])

    binance = Binance()
    histories = binance.histories
    base = {"fiat_payments": histories["/sapi/v1/fiat/payments"]["data"],
            "deposits": histories["/sapi/v1/capital/deposit/hisrec"],
            "withdrawals": histories["/sapi/v1/capital/withdraw/history"],
            "dividends": histories["/sapi/v1/asset/assetDividend"]["rows"]}
    add("binance", "base", base)
    for key in base:
        for index, value in enumerate([None, [], {}, "", "rows", 0, 7, 1.5, True, False, {"x": 1}, [1, "a", None]]):
            payload = copy.deepcopy(base)
            payload[key] = value
            add("binance", f"{key}-shape-{index}", payload)
        missing = copy.deepcopy(base)
        del missing[key]
        add("binance", f"{key}-missing", missing)
    fields = {
        "fiat_payments": ["status", "cryptoCurrency", "fiatCurrency", "obtainAmount", "sourceAmount", "price",
                          "createTime", "orderNo", "orderId"],
        "deposits": ["coin", "asset", "status", "amount", "insertTime", "txId", "id"],
        "withdrawals": ["status", "amount", "transactionFee", "fee", "applyTime", "successTime", "id",
                        "withdrawOrderId", "txId"],
        "dividends": ["asset", "coinName", "amount", "profitAmount", "id", "tranId", "time", "divTime", "enInfo",
                      "type"],
    }
    for key, names in fields.items():
        for field in names:
            values = TIMES if field.endswith("Time") or field == "time" else VALUES if "mount" in field or \
                "ee" in field or field == "price" else TEXTS + ["completed", "Completed ", "1", "6", "success",
                                                              "credited", "confirming", "2", "mining"]
            for row in ROWS[key]:
                for index, value in enumerate(values):
                    payload = {key: [copy.deepcopy(base[key][row])]}
                    payload[key][0][field] = value
                    add("binance", f"{key}-{row}-{field}-{index}", payload)
    return out


def digest_raw(result):
    """Keep vectors small: each record's raw_json becomes its SHA-256 prefix."""
    for record in result.get("records", []):
        record["raw_json"] = hashlib.sha256(record["raw_json"].encode("utf-8", "surrogatepass")).hexdigest()[:16]
    return result


def generate():
    vectors = []
    for case in cases():
        payload = json.loads(json.dumps(case["payload"]))
        result = digest_raw(run(case["exchange"], copy.deepcopy(payload)))
        vectors.append({"exchange": case["exchange"], "name": case["name"], "payload": payload,
                        "expected": json.loads(json.dumps(result, default=str))})
    return "[\n" + ",\n".join(json.dumps(v, separators=(",", ":"), ensure_ascii=True) for v in vectors) + "\n]\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = generate()
    if args.check:
        if DEST.read_text() != text:
            raise SystemExit(f"{DEST} is stale; regenerate it")
        return
    DEST.write_text(text)
    vectors = json.loads(text)
    errors = sum("error" in v["expected"] for v in vectors)
    print(f"{len(vectors)} vectors ({errors} errors) -> {DEST.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
