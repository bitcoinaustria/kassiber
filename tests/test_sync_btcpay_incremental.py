import json
import unittest
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
    def __init__(self, pages):
        self.pages = pages
        self.urls = []

    def open(self, request, timeout=30):
        del timeout
        url = request.full_url
        self.urls.append(url)
        query = parse_qs(urlsplit(url).query)
        skip = int((query.get("skip") or ["0"])[0])
        return _Response(self.pages.get(skip, []))


class BtcpayIncrementalTest(unittest.TestCase):
    def test_wallet_history_stops_at_unchanged_page_fingerprint(self):
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
        self.assertEqual(len(second_opener.urls), 1)
        self.assertTrue(second_metadata["stopped_by_known_page"])

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

    def test_invoice_provenance_stops_at_unchanged_page_fingerprint(self):
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
        self.assertEqual(len(opener.urls), 2)

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
        self.assertEqual(len(second_opener.urls), 1)
        self.assertTrue(second_metadata["stopped_by_known_page"])

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
        self.assertEqual(len(second_opener.urls), 2)


if __name__ == "__main__":
    unittest.main()
