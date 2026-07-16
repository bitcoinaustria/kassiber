"""Gap suggestions produce review holds, never basis-carrying edges."""

from dataclasses import replace
import hashlib
import unittest

from kassiber.core.custody_evidence import QuantityObservation
from kassiber.core.custody_gap_holds import (
    CustodyGapHoldCompileError,
    compile_gap_candidate_holds,
)
from kassiber.core.custody_gaps import suggest_custody_gap_candidates
from kassiber.core.custody_quantity import CUSTODY_SUSPENSE
from kassiber.core.custody_quantity_runtime import build_canonical_quantity_state
from kassiber.core.custody_tax_projection import compile_finalized_tax_projection


BTC_MSAT = 100_000_000_000


def _row(
    transaction_id: str,
    wallet_id: str,
    direction: str,
    amount_msat: int,
    occurred_at: str,
    *,
    fee_msat: int = 0,
    privacy_boundary: str | None = None,
) -> dict:
    native_txid = hashlib.sha256(transaction_id.encode()).hexdigest()
    return {
        "id": transaction_id,
        "workspace_id": "workspace",
        "profile_id": "profile",
        "wallet_id": wallet_id,
        "wallet_label": wallet_id,
        "wallet_kind": "descriptor",
        "fingerprint": f"fingerprint:{transaction_id}",
        "external_id": native_txid,
        "occurred_at": occurred_at,
        "created_at": occurred_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount_msat,
        "fee": fee_msat,
        "amount_includes_fee": 0,
        "excluded": 0,
        "kind": "",
        "privacy_boundary": privacy_boundary,
        "chain": "bitcoin",
        "network": "main",
        "raw_json": {"txid": native_txid, "chain": "bitcoin", "network": "main"},
    }


def _candidate(rows, source_ids, return_ids):
    expected_sources = tuple(sorted(source_ids))
    expected_returns = tuple(sorted(return_ids))
    return next(
        item
        for item in suggest_custody_gap_candidates(rows)
        if item.source_ids == expected_sources and item.return_ids == expected_returns
    )


def _observations(rows):
    return {row["id"]: QuantityObservation.from_transaction(row) for row in rows}


class CustodyGapHoldCompilerTests(unittest.TestCase):
    def test_ten_to_nine_point_nine_holds_boundaries_without_transfer_edge(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                99 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]
        candidate = _candidate(rows, ["out"], ["return"])

        compilation = compile_gap_candidate_holds(candidate, _observations(rows))

        self.assertEqual(
            [(hold.direction, hold.quantity.amount_msat) for hold in compilation.holds],
            [("outbound", 10 * BTC_MSAT), ("inbound", 99 * BTC_MSAT // 10)],
        )
        self.assertTrue(all(not hasattr(hold, "target") for hold in compilation.holds))
        self.assertEqual(compilation.retained_msat, 99 * BTC_MSAT // 10)
        self.assertEqual(compilation.residual_msat, BTC_MSAT // 10)

    def test_production_projection_blocks_source_and_return_until_review(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="whirlpool",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                99 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]

        state = build_canonical_quantity_state(rows)
        projection = compile_finalized_tax_projection(
            {"id": "profile", "workspace_id": "workspace", "tax_country": "at"},
            rows,
            state,
        )

        self.assertEqual(len(state.gap_holds), 2)
        self.assertTrue(
            all(decision.target is None for decision in state.projection.decisions)
        )
        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in state.projection.decisions],
            [(CUSTODY_SUSPENSE, 10 * BTC_MSAT)],
        )
        self.assertFalse(projection.rows)
        self.assertFalse(projection.intra_pairs)
        self.assertEqual(
            {item["transaction_id"] for item in projection.quarantines},
            {"out", "return"},
        )

    def test_known_network_fee_remains_a_finalized_sibling(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                fee_msat=25_000,
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                99 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]
        state = build_canonical_quantity_state(rows)
        projection = compile_finalized_tax_projection(
            {"id": "profile", "workspace_id": "workspace", "tax_country": "at"},
            rows,
            state,
        )

        self.assertEqual(len(projection.rows), 1)
        self.assertEqual(projection.rows[0]["amount"], 0)
        self.assertEqual(projection.rows[0]["fee"], 25_000)

    def test_nm_candidate_holds_each_boundary_independently(self):
        rows = [
            _row(
                "out-a",
                "wallet-a",
                "outbound",
                6 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "out-b",
                "wallet-a",
                "outbound",
                4 * BTC_MSAT,
                "2020-01-02T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row("return-a", "wallet-c", "inbound", 5 * BTC_MSAT, "2021-01-01T00:00:00Z"),
            _row("return-b", "wallet-c", "inbound", 49 * BTC_MSAT // 10, "2021-01-02T00:00:00Z"),
        ]
        candidate = _candidate(rows, ["out-a", "out-b"], ["return-a", "return-b"])

        holds = compile_gap_candidate_holds(candidate, _observations(rows)).holds

        self.assertEqual({hold.transaction_id for hold in holds}, {row["id"] for row in rows})
        self.assertEqual(len({hold.quantity.observation_hash for hold in holds}), 4)

    def test_search_only_hint_does_not_create_holds(self):
        rows = [
            _row("out", "wallet-a", "outbound", BTC_MSAT, "2020-01-01T00:00:00Z"),
            _row("return", "wallet-c", "inbound", BTC_MSAT, "2021-01-01T00:00:00Z"),
        ]
        candidate = _candidate(rows, ["out"], ["return"])
        self.assertFalse(candidate.promotion_eligible)

        compilation = compile_gap_candidate_holds(candidate, _observations(rows))

        self.assertFalse(compilation.holds)

    def test_stale_candidate_fails_before_returning_any_hold(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row("return", "wallet-c", "inbound", BTC_MSAT, "2021-01-01T00:00:00Z"),
        ]
        candidate = replace(_candidate(rows, ["out"], ["return"]), retained_msat=1)

        with self.assertRaises(CustodyGapHoldCompileError):
            compile_gap_candidate_holds(candidate, _observations(rows))


if __name__ == "__main__":
    unittest.main()
