import base64
import json
import tempfile
import unittest
from pathlib import Path
from urllib.parse import parse_qs

from kassiber.core.exchange_imports import (
    fetch_kraken_records,
    normalize_binance_records,
    normalize_coinbase_records,
    normalize_kraken_records,
)
from kassiber.errors import AppError
from kassiber.importers import (
    load_binance_supplemental_csv_records,
    load_ledgerlive_csv_records,
)


class ExchangeImporterTest(unittest.TestCase):
    def test_kraken_api_fetcher_paginates_ledgers_and_trades(self):
        class Response:
            def __init__(self, payload):
                self.payload = payload

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

            def read(self):
                return json.dumps(self.payload).encode("utf-8")

        class Opener:
            def open(self, request, timeout=0):  # noqa: ARG002
                params = parse_qs((request.data or b"").decode("utf-8"))
                offset = params.get("ofs", ["0"])[0]
                if request.full_url.endswith("/0/private/TradesHistory"):
                    trades = {
                        "0": {
                            "T1": {
                                "pair": "XXBTZEUR",
                                "cost": "400.00",
                                "fee": "1.20",
                                "price": "40000.00",
                            }
                        },
                        "1": {
                            "T2": {
                                "pair": "XXBTZEUR",
                                "cost": "420.00",
                                "fee": "1.26",
                                "price": "42000.00",
                            }
                        },
                    }[offset]
                    return Response({"error": [], "result": {"count": 2, "trades": trades}})
                ledgers = {
                    "0": {
                        "L1": {
                            "refid": "T1",
                            "time": "1700000000",
                            "type": "trade",
                            "asset": "XXBT",
                            "amount": "0.01",
                            "fee": "0",
                        }
                    },
                    "1": {
                        "L2": {
                            "refid": "T2",
                            "time": "1700000100",
                            "type": "trade",
                            "asset": "XXBT",
                            "amount": "0.01",
                            "fee": "0",
                        }
                    },
                }[offset]
                return Response({"error": [], "result": {"count": 2, "ledger": ledgers}})

        records = fetch_kraken_records(
            {
                "kind": "kraken",
                "token": "key",
                "auth_header": base64.b64encode(b"secret").decode("ascii"),
                "url": "https://api.kraken.com",
            },
            opener=Opener(),
        )

        self.assertEqual([record["txid"] for record in records], ["kraken:L1", "kraken:L2"])

    def test_ledgerlive_csv_imports_wallet_movement_and_redacts_xpub(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ledger.csv"
            path.write_text(
                "\n".join(
                    [
                        "Operation Date,Currency Ticker,Operation Type,Operation Amount,Operation Fees,Operation Hash,Account Name,Account xpub,Countervalue Ticker,Countervalue at Operation Date",
                        "2024-01-02T03:04:05.000Z,BTC,IN,0.01000000,,tx-in,Bitcoin,xpub-secret,EUR,420.00",
                        "2024-01-03T03:04:05.000Z,ETH,IN,1.0,,eth-tx,Ethereum,xpub-eth,EUR,2000.00",
                        "2024-01-04T03:04:05.000Z,BTC,OUT,-0.00200000,0.00001000,tx-out,Bitcoin,xpub-secret,EUR,80.00",
                    ]
                ),
                encoding="utf-8",
            )

            records = load_ledgerlive_csv_records(path)

        self.assertEqual(len(records), 2)
        inbound = records[0]
        self.assertEqual(inbound["txid"], "tx-in")
        self.assertEqual(inbound["direction"], "inbound")
        self.assertEqual(inbound["asset"], "BTC")
        self.assertNotIn("fiat_value", inbound)
        raw = json.loads(inbound["raw_json"])
        self.assertEqual(raw["Account xpub"], "[redacted]")
        self.assertNotIn("xpub-secret", inbound["raw_json"])
        outbound = records[1]
        self.assertEqual(outbound["direction"], "outbound")
        self.assertEqual(str(outbound["fee"]), "0.00001000")

    def test_binance_supplemental_autoinvest_requires_fiat_quote_for_exact_price(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "binance-auto.csv"
            path.write_text(
                "\n".join(
                    [
                        "timestamp UTC,base asset symbol,quote asset amount + symbol,trading fee (in quote asset),base asset amount + symbol,source of funds",
                        "2024-02-01 10:00:00,BTC,100.00 EUR,1.00 EUR,0.002 BTC,Spot Wallet",
                    ]
                ),
                encoding="utf-8",
            )
            records = load_binance_supplemental_csv_records(path)

            bad = Path(tmp) / "binance-auto-usdt.csv"
            bad.write_text(
                "\n".join(
                    [
                        "timestamp UTC,base asset symbol,quote asset amount + symbol,trading fee (in quote asset),base asset amount + symbol,source of funds",
                        "2024-02-01 10:00:00,BTC,100.00 USDT,1.00 USDT,0.002 BTC,Spot Wallet",
                    ]
                ),
                encoding="utf-8",
            )
            with self.assertRaises(AppError):
                load_binance_supplemental_csv_records(bad)

        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["pricing_source_kind"], "exchange_execution")
        self.assertEqual(record["pricing_quality"], "exact")
        self.assertEqual(record["fiat_currency"], "EUR")
        self.assertEqual(str(record["fiat_value"]), "101.00")
        self.assertEqual(record["pricing_provider"], "Binance")

    def test_kraken_api_normalizer_pairs_btc_trade_with_trade_history(self):
        records = normalize_kraken_records(
            {
                "result": {
                    "ledger": {
                        "L1": {
                            "refid": "T1",
                            "time": "1700000000",
                            "type": "trade",
                            "asset": "XXBT",
                            "amount": "0.01",
                            "fee": "0.00001",
                        }
                    }
                }
            },
            {
                "result": {
                    "trades": {
                        "T1": {
                            "pair": "XXBTZEUR",
                            "cost": "400.00",
                            "fee": "1.20",
                            "price": "40000.00",
                        }
                    }
                }
            },
        )

        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["txid"], "kraken:L1")
        self.assertEqual(record["kind"], "buy")
        self.assertEqual(record["fiat_currency"], "EUR")
        self.assertEqual(str(record["fiat_value"]), "400.00")
        self.assertEqual(record["pricing_method"], "kraken_api")

    def test_coinbase_api_normalizer_keeps_trade_and_transfer_separate(self):
        records = normalize_coinbase_records(
            [
                {
                    "currency": "BTC",
                    "transactions": [
                        {
                            "id": "buy-1",
                            "type": "buy",
                            "created_at": "2024-03-01T00:00:00Z",
                            "amount": {"amount": "0.01", "currency": "BTC"},
                            "native_amount": {"amount": "500.00", "currency": "EUR"},
                        },
                        {
                            "id": "send-1",
                            "type": "send",
                            "created_at": "2024-03-02T00:00:00Z",
                            "amount": {"amount": "-0.002", "currency": "BTC"},
                            "native_amount": {"amount": "-100.00", "currency": "EUR"},
                            "network": {"hash": "a" * 64},
                        },
                    ],
                }
            ]
        )

        self.assertEqual(len(records), 2)
        self.assertEqual(records[0]["pricing_source_kind"], "exchange_execution")
        self.assertEqual(records[0]["kind"], "buy")
        self.assertEqual(records[1]["txid"], "a" * 64)
        self.assertEqual(records[1]["kind"], "withdrawal")
        self.assertIsNone(records[1]["pricing_source_kind"])

    def test_binance_api_normalizer_imports_supported_btc_rows(self):
        records = normalize_binance_records(
            {
                "fiat_payments": [
                    {
                        "orderNo": "order-1",
                        "status": "Completed",
                        "cryptoCurrency": "BTC",
                        "fiatCurrency": "EUR",
                        "obtainAmount": "0.01",
                        "sourceAmount": "450.00",
                        "totalFee": "1.00",
                        "price": "45000.00",
                        "createTime": 1700000000000,
                    }
                ],
                "deposits": [
                    {
                        "coin": "BTC",
                        "amount": "0.02",
                        "txId": "b" * 64,
                        "status": "1",
                        "insertTime": 1700001000000,
                    }
                ],
                "withdrawals": [],
                "dividends": [],
            }
        )

        self.assertEqual(len(records), 2)
        self.assertEqual(records[0]["kind"], "buy")
        self.assertEqual(str(records[0]["fiat_value"]), "450.00")
        self.assertEqual(records[1]["kind"], "deposit")
        self.assertIsNone(records[1]["pricing_source_kind"])


if __name__ == "__main__":
    unittest.main()
