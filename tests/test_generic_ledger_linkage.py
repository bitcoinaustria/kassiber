import tempfile
import unittest
from pathlib import Path

from kassiber.importers import load_generic_ledger_records


class GenericLedgerLinkageTest(unittest.TestCase):
    def test_native_template_preserves_swap_linkage_columns(self):
        payment_hash = "AA" * 32
        refund_funding_txid = "bb" * 32
        with tempfile.TemporaryDirectory(prefix="kassiber-ledger-linkage-") as tmp:
            path = Path(tmp) / "ledger.csv"
            path.write_text(
                "Type,Date,Sent Amount,Sent Asset,Tx-ID,Payment Hash,Payment Hash Source,Swap Refund Funding Tx-ID\n"
                f"Withdrawal,2026-04-01,0.00100000,LBTC,boltz-lockup-1,{payment_hash},boltz-regtest,{refund_funding_txid}\n",
                encoding="utf-8",
            )

            records = load_generic_ledger_records(path)

        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["asset"], "LBTC")
        self.assertEqual(record["txid"], "boltz-lockup-1")
        self.assertEqual(record["payment_hash"], payment_hash)
        self.assertEqual(record["payment_hash_source"], "boltz-regtest")
        self.assertEqual(record["swap_refund_funding_txid"], refund_funding_txid)

    def test_byo_columns_preserve_swap_linkage_columns(self):
        payment_hash = "cc" * 32
        with tempfile.TemporaryDirectory(prefix="kassiber-ledger-byo-linkage-") as tmp:
            path = Path(tmp) / "byo.csv"
            path.write_text(
                "Executed At,Direction,Amount,Asset,Reference,Payment Hash,Note\n"
                f"2026-04-01,outbound,0.00100000,LBTC,boltz-lockup-2,{payment_hash},Boltz lockup\n",
                encoding="utf-8",
            )

            records = load_generic_ledger_records(path)

        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["asset"], "LBTC")
        self.assertEqual(record["txid"], "boltz-lockup-2")
        self.assertEqual(record["payment_hash"], payment_hash)
        self.assertEqual(record["payment_hash_source"], "generic_ledger")


if __name__ == "__main__":
    unittest.main()
