import base64
import tempfile
import unittest
from pathlib import Path

from kassiber.daemon import _ledger_preview_payload


def _preview(args):
    return _ledger_preview_payload(args)


class LedgerPreviewSecurityTests(unittest.TestCase):
    def test_daemon_preview_rejects_caller_supplied_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            secret = Path(tmp) / "secret.env"
            secret.write_text("API_TOKEN=super_secret_value\n", encoding="utf-8")

            with self.assertRaises(Exception) as raised:
                _preview({"source_file": str(secret), "limit": 5})

        self.assertIn("source_bytes_base64 is required", str(raised.exception))
        self.assertNotIn("super_secret_value", str(raised.exception))

    def test_daemon_preview_accepts_uploaded_ledger_bytes(self):
        csv_text = (
            "Date,Type,Received Asset,Received Amount,Fiat Currency,Fiat Value,Note\n"
            "2026-01-01,Deposit,BTC,0.01,EUR,500,selected file\n"
        )

        response = _preview({
            "filename": "ledger.csv",
            "source_bytes_base64": base64.b64encode(csv_text.encode()).decode(),
            "limit": 5,
        })

        self.assertEqual(response["mapped"], 1)
        self.assertEqual(response["preview"][0]["description"], "selected file")

    def test_daemon_preview_accepts_uploaded_tsv_ledger_bytes(self):
        tsv_text = (
            "Date\tType\tReceived Asset\tReceived Amount\tFiat Currency\tFiat Value\tNote\n"
            "2026-01-01\tDeposit\tBTC\t0.01\tEUR\t500\tselected tsv\n"
        )

        response = _preview({
            "filename": "ledger.tsv",
            "source_bytes_base64": base64.b64encode(tsv_text.encode()).decode(),
            "limit": 5,
        })

        self.assertEqual(response["mapped"], 1)
        self.assertEqual(response["preview"][0]["description"], "selected tsv")


if __name__ == "__main__":
    unittest.main()
