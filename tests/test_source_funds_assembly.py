import unittest

from kassiber.core.source_funds_assembly import derive_payment_hash_pairs


def _row(row_id, direction, amount, *, source="core_lightning", wallet="w1"):
    return {
        "id": row_id,
        "wallet_id": wallet,
        "wallet_kind": "core-ln",
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "occurred_at": "2026-01-01T00:00:00Z",
        "payment_hash": "AB" * 32,
        "payment_hash_source": source,
        "kind": "cln_pay" if direction == "outbound" else "cln_invoice",
    }


class SourceFundsPaymentHashTests(unittest.TestCase):
    def test_uses_journal_lightning_hash_gate_and_allows_same_wallet(self):
        outbound = _row("out", "outbound", 1_001_000)
        inbound = _row("in", "inbound", 1_000_000)

        pairs = derive_payment_hash_pairs(
            [outbound, inbound], skip_row=lambda _row: False
        )

        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["allocation_msat"], 1_000_000)

    def test_chain_script_hash_does_not_assert_lightning_lineage(self):
        outbound = _row("out", "outbound", 1_001_000, source="chain_script")
        inbound = _row("in", "inbound", 1_000_000, source="chain_script")
        self.assertEqual(
            derive_payment_hash_pairs(
                [outbound, inbound], skip_row=lambda _row: False
            ),
            [],
        )

    def test_untyped_import_hash_does_not_auto_assert_lineage(self):
        outbound = _row("out", "outbound", 1_001_000, source="import")
        inbound = _row("in", "inbound", 1_000_000, source="import")
        outbound["kind"] = "withdrawal"
        inbound["kind"] = "deposit"
        outbound["wallet_kind"] = "custom"
        inbound["wallet_kind"] = "custom"
        self.assertEqual(
            derive_payment_hash_pairs(
                [outbound, inbound], skip_row=lambda _row: False
            ),
            [],
        )


if __name__ == "__main__":
    unittest.main()
