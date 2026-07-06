import json
import unittest
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

from kassiber.sync_btcpay import fetch_btcpay_invoice_provenance, fetch_btcpay_records


class _Response:
    def __init__(self, payload):
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False

    def read(self):
        return json.dumps(self.payload).encode("utf-8")


class _Opener:
    def __init__(self, pages, payment_methods=None):
        self.pages = pages
        self.payment_methods = payment_methods or {}
        self.urls = []

    def open(self, request, timeout=30):
        del timeout
        url = request.full_url
        self.urls.append(url)
        parts = [part for part in urlsplit(url).path.split("/") if part]
        if "invoices" in parts and "payment-methods" in parts:
            invoice_id = parts[parts.index("invoices") + 1]
            return _Response(self.payment_methods.get(invoice_id, []))
        query = parse_qs(urlsplit(url).query)
        skip = int((query.get("skip") or ["0"])[0])
        return _Response(self.pages.get(skip, []))


class BtcpayIncrementalTest(unittest.TestCase):
    def _wallet_page(self, index, *, comment=None, labels=None):
        row = {
            "transactionHash": f"{index:064x}",
            "timestamp": 1_700_000_000 - index,
            "amount": "0.01",
            "confirmations": 6,
        }
        if comment is not None:
            row["comment"] = comment
        if labels is not None:
            row["labels"] = labels
        return [row]

    def _invoice_page(self, index, *, order_id=None):
        return [
            {
                "id": f"invoice-{index}",
                "status": "Settled",
                "metadata": {"orderId": order_id or f"order-{index}"},
                "payments": [],
            }
        ]

    def test_wallet_history_default_opener_uses_backend_proxy(self):
        backend = {
            "name": "btcpay",
            "kind": "btcpay",
            "url": "https://btcpay.example",
            "token": "secret",
            "tor_proxy": "socks5h://127.0.0.1:9050",
        }
        opener = _Opener({0: []})
        with patch(
            "kassiber.sync_btcpay.build_proxy_opener",
            return_value=opener,
        ) as build_opener:
            records = fetch_btcpay_records(backend, "store", page_size=1)
        self.assertEqual(records, [])
        build_opener.assert_called_once_with(
            "socks5h://127.0.0.1:9050",
            source_label="BTCPay",
        )
        self.assertEqual(len(opener.urls), 1)

    def test_wallet_history_skips_unchanged_page_and_continues_to_end(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        page0 = [
            {
                "transactionHash": "aa" * 32,
                "timestamp": 1_700_000_000,
                "amount": "0.01",
                "confirmations": 6,
            }
        ]
        opener = _Opener({0: page0, 1: []})
        metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=opener,
            metadata=metadata,
        )
        self.assertEqual(len(records), 1)
        self.assertEqual(len(opener.urls), 2)

        second_opener = _Opener({0: page0, 1: []})
        second_metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=second_opener,
            checkpoint={"btcpay_pages": metadata["btcpay_pages"]},
            metadata=second_metadata,
        )
        self.assertEqual(records, [])
        self.assertEqual(len(second_opener.urls), 2)
        self.assertEqual(second_metadata["pages_fetched"], 2)
        self.assertTrue(second_metadata["stopped_by_known_page"])

    def test_wallet_history_stops_after_bounded_unchanged_window(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        pages = {index: self._wallet_page(index) for index in range(12)}
        metadata = {}
        fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=_Opener(pages),
            metadata=metadata,
        )

        second_opener = _Opener(pages)
        second_metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=second_opener,
            checkpoint={
                "btcpay_pages": metadata["btcpay_pages"],
                "btcpay_pagination": metadata["btcpay_pagination"],
            },
            metadata=second_metadata,
        )

        self.assertEqual(records, [])
        self.assertLess(len(second_opener.urls), len(pages))
        self.assertEqual(second_metadata["stop_reason"], "unchanged_page_window")
        self.assertEqual(second_metadata["deep_audit"]["start_skip"], 5)
        self.assertEqual(second_metadata["btcpay_pagination"]["next_deep_audit_skip"], 6)

    def test_wallet_history_reimports_when_metadata_changes_with_same_id(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        txid = "aa" * 32
        page0 = [
            {
                "transactionHash": txid,
                "timestamp": 1_700_000_000,
                "amount": "0.01",
                "confirmations": 6,
                "comment": "Original note",
                "labels": ["old"],
            }
        ]
        metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=_Opener({0: page0, 1: []}),
            metadata=metadata,
        )
        self.assertEqual(len(records), 1)

        updated_page0 = [
            {
                **page0[0],
                "comment": "Corrected note",
                "labels": ["new"],
            }
        ]
        second_opener = _Opener({0: updated_page0, 1: []})
        second_metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=second_opener,
            checkpoint={"btcpay_pages": metadata["btcpay_pages"]},
            metadata=second_metadata,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["_btcpay_comment"], "Corrected note")
        self.assertEqual(records[0]["_btcpay_labels"], ["new"])
        self.assertEqual(len(second_opener.urls), 2)

    def test_wallet_history_reimports_older_changed_page_after_newer_unchanged(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        page0 = [
            {
                "transactionHash": "aa" * 32,
                "timestamp": 1_700_000_100,
                "amount": "0.01",
                "confirmations": 6,
                "comment": "Newest stable",
            }
        ]
        page1 = [
            {
                "transactionHash": "bb" * 32,
                "timestamp": 1_700_000_000,
                "amount": "0.02",
                "confirmations": 6,
                "comment": "Older original",
                "labels": ["old"],
            }
        ]
        metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=_Opener({0: page0, 1: page1, 2: []}),
            metadata=metadata,
        )
        self.assertEqual(len(records), 2)

        updated_page1 = [
            {
                **page1[0],
                "comment": "Older corrected",
                "labels": ["corrected"],
            }
        ]
        second_opener = _Opener({0: page0, 1: updated_page1, 2: []})
        second_metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=second_opener,
            checkpoint={"btcpay_pages": metadata["btcpay_pages"]},
            metadata=second_metadata,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["_btcpay_comment"], "Older corrected")
        self.assertEqual(records[0]["_btcpay_labels"], ["corrected"])
        self.assertEqual(len(second_opener.urls), 3)
        self.assertTrue(second_metadata["stopped_by_known_page"])

    def test_wallet_history_deep_audit_finds_older_comment_and_label_edit(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        pages = {
            index: self._wallet_page(index, comment=f"original-{index}", labels=["old"])
            for index in range(12)
        }
        metadata = {}
        fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=_Opener(pages),
            metadata=metadata,
        )

        updated_pages = dict(pages)
        updated_pages[7] = self._wallet_page(
            7,
            comment="deep corrected",
            labels=["deep", "corrected"],
        )
        second_metadata = {}
        records = fetch_btcpay_records(
            backend,
            "store",
            page_size=1,
            opener=_Opener(updated_pages),
            checkpoint={
                "btcpay_pages": metadata["btcpay_pages"],
                "btcpay_pagination": {
                    **metadata["btcpay_pagination"],
                    "next_deep_audit_skip": 7,
                },
            },
            metadata=second_metadata,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["_btcpay_comment"], "deep corrected")
        self.assertEqual(records[0]["_btcpay_labels"], ["deep", "corrected"])
        self.assertEqual(second_metadata["stop_reason"], "unchanged_page_window")
        self.assertEqual(second_metadata["deep_audit"]["start_skip"], 7)

    def test_invoice_provenance_skips_unchanged_page_and_continues_to_end(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        page0 = [{"id": "invoice-1", "status": "Settled", "payments": []}]
        opener = _Opener({0: page0, 1: []})
        metadata = {}
        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=opener,
            metadata=metadata,
        )
        self.assertEqual(len(records), 1)
        self.assertEqual(len(opener.urls), 3)
        self.assertEqual(parse_qs(urlsplit(opener.urls[0]).query).get("includePaymentMethods"), ["true"])

        second_opener = _Opener({0: page0, 1: []})
        second_metadata = {}
        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=second_opener,
            checkpoint={"btcpay_invoice_pages": metadata["btcpay_invoice_pages"]},
            metadata=second_metadata,
        )
        self.assertEqual(records, [])
        self.assertEqual(len(second_opener.urls), 3)
        self.assertEqual(second_metadata["pages_fetched"], 2)
        self.assertTrue(second_metadata["stopped_by_known_page"])

    def test_invoice_provenance_hydrates_payment_methods_when_invoice_page_omits_payments(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        txid = "ab" * 32
        page0 = [
            {
                "id": "invoice-1",
                "status": "Settled",
                "currency": "EUR",
                "amount": "42.00",
                "metadata": {"paymentRequestId": "membership-2026", "orderUrl": "https://shop.example/pr"},
                "payments": [],
            }
        ]
        opener = _Opener(
            {0: page0, 1: []},
            payment_methods={
                "invoice-1": [
                    {
                        "paymentMethodId": "BTC-CHAIN",
                        "rate": "50000.00",
                        "destination": "bcrt1qexample",
                        "payments": [
                            {
                                "id": f"{txid}-0",
                                "value": "0.00084000",
                                "receivedDate": 1_700_000_000,
                            }
                        ],
                    }
                ]
            },
        )

        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=opener,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["origin_kind"], "payment_request")
        self.assertEqual(records[0]["origin_url"], "https://shop.example/pr")
        self.assertEqual(records[0]["payments"][0]["payment_id"], f"{txid}-0")
        self.assertEqual(records[0]["payments"][0]["txid"], txid)
        self.assertEqual(records[0]["payments"][0]["payment_method_id"], "BTC-CHAIN")
        self.assertEqual(records[0]["payments"][0]["rate"], "50000.00")
        self.assertEqual(records[0]["payments"][0]["destination"], "bcrt1qexample")
        self.assertEqual(records[0]["payments"][0]["invoice_currency"], "EUR")
        self.assertEqual(records[0]["payments"][0]["invoice_amount"], "42.00")
        payment_method_urls = [url for url in opener.urls if "/payment-methods" in url]
        self.assertEqual(len(payment_method_urls), 1)

    def test_invoice_provenance_reimports_when_metadata_changes_with_same_id(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        page0 = [
            {
                "id": "invoice-1",
                "status": "Settled",
                "metadata": {"orderId": "order-1"},
                "payments": [],
            }
        ]
        metadata = {}
        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=_Opener({0: page0, 1: []}),
            metadata=metadata,
        )
        self.assertEqual(len(records), 1)

        updated_page0 = [
            {
                **page0[0],
                "metadata": {"orderId": "order-2", "orderUrl": "https://shop.example/orders/2"},
            }
        ]
        second_opener = _Opener({0: updated_page0, 1: []})
        second_metadata = {}
        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=second_opener,
            checkpoint={"btcpay_invoice_pages": metadata["btcpay_invoice_pages"]},
            metadata=second_metadata,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["order_id"], "order-2")
        self.assertEqual(records[0]["order_url"], "https://shop.example/orders/2")
        self.assertEqual(len(second_opener.urls), 3)

    def test_invoice_provenance_reimports_older_changed_page_after_newer_unchanged(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        page0 = [
            {
                "id": "invoice-new",
                "status": "Settled",
                "metadata": {"orderId": "order-new"},
                "payments": [],
            }
        ]
        page1 = [
            {
                "id": "invoice-old",
                "status": "Settled",
                "metadata": {"orderId": "order-old"},
                "payments": [],
            }
        ]
        metadata = {}
        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=_Opener({0: page0, 1: page1, 2: []}),
            metadata=metadata,
        )
        self.assertEqual(len(records), 2)

        updated_page1 = [
            {
                **page1[0],
                "metadata": {"orderId": "order-old-corrected"},
            }
        ]
        second_opener = _Opener({0: page0, 1: updated_page1, 2: []})
        second_metadata = {}
        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=second_opener,
            checkpoint={"btcpay_invoice_pages": metadata["btcpay_invoice_pages"]},
            metadata=second_metadata,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["order_id"], "order-old-corrected")
        self.assertEqual(len(second_opener.urls), 5)
        self.assertTrue(second_metadata["stopped_by_known_page"])

    def test_invoice_provenance_deep_audit_finds_older_metadata_edit(self):
        backend = {"name": "btcpay", "kind": "btcpay", "url": "https://btcpay.example", "token": "secret"}
        pages = {index: self._invoice_page(index) for index in range(12)}
        metadata = {}
        fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=_Opener(pages),
            metadata=metadata,
        )

        updated_pages = dict(pages)
        updated_pages[8] = self._invoice_page(8, order_id="deep-order-corrected")
        second_metadata = {}
        records = fetch_btcpay_invoice_provenance(
            backend,
            "store",
            page_size=1,
            opener=_Opener(updated_pages),
            checkpoint={
                "btcpay_invoice_pages": metadata["btcpay_invoice_pages"],
                "btcpay_invoice_pagination": {
                    **metadata["btcpay_invoice_pagination"],
                    "next_deep_audit_skip": 8,
                },
            },
            metadata=second_metadata,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["order_id"], "deep-order-corrected")
        self.assertEqual(second_metadata["stop_reason"], "unchanged_page_window")
        self.assertEqual(second_metadata["deep_audit"]["start_skip"], 8)


if __name__ == "__main__":
    unittest.main()
