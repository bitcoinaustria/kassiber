"""Pure bridge tests from promoted gap candidates to quantity arbitration."""

from dataclasses import replace
import hashlib
import random
import unittest

from kassiber.core.custody_evidence import QuantityObservation
from kassiber.core.custody_gap_claims import (
    CustodyGapClaimCompileError,
    compile_gap_candidate_claims,
)
from kassiber.core.custody_gaps import suggest_custody_gap_candidates
from kassiber.core.custody_quantity import (
    CONFLICTING,
    CUSTODY_CANDIDATE,
    CUSTODY_SUSPENSE,
    project_quantities,
)
from kassiber.core.custody_quantity_runtime import baseline_fallback_claims


BTC_MSAT = 100_000_000_000


def _row(
    transaction_id: str,
    wallet_id: str,
    direction: str,
    amount_msat: int,
    occurred_at: str,
    *,
    fee_msat: int = 0,
    amount_includes_fee: int = 0,
    privacy_boundary: str | None = None,
    network: str = "main",
) -> dict:
    native_txid = hashlib.sha256(transaction_id.encode("utf-8")).hexdigest()
    return {
        "id": transaction_id,
        "profile_id": "profile-one",
        "wallet_id": wallet_id,
        "wallet_label": wallet_id,
        "wallet_kind": "descriptor",
        "fingerprint": f"fingerprint:{transaction_id}",
        "external_id": native_txid,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount_msat,
        "fee": fee_msat,
        "amount_includes_fee": amount_includes_fee,
        "excluded": 0,
        "kind": "",
        "privacy_boundary": privacy_boundary,
        "chain": "bitcoin",
        "network": network,
        "raw_json": {
            "txid": native_txid,
            "chain": "bitcoin",
            "network": network,
        },
    }


def _observations(rows):
    return {
        row["id"]: QuantityObservation.from_transaction(row)
        for row in rows
    }


def _candidate(rows, source_ids, return_ids):
    expected_sources = tuple(sorted(source_ids))
    expected_returns = tuple(sorted(return_ids))
    return next(
        candidate
        for candidate in suggest_custody_gap_candidates(rows)
        if candidate.source_ids == expected_sources
        and candidate.return_ids == expected_returns
    )


class CustodyGapClaimCompilerTests(unittest.TestCase):
    def test_ten_to_nine_point_nine_leaves_point_one_in_suspense(self):
        rows = [
            _row(
                "out",
                "multisig-b",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "operative-c",
                "inbound",
                99 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]
        observations = _observations(rows)
        candidate = _candidate(rows, ["out"], ["return"])

        compilation = compile_gap_candidate_claims(candidate, observations)
        projection = project_quantities(
            list(observations.values()),
            [*baseline_fallback_claims(list(observations.values())), *compilation.claims],
        )

        self.assertEqual(compilation.atomic_bundle_id, f"candidate:{candidate.gap_id}")
        self.assertEqual(
            sum(claim.source.amount_msat for claim in compilation.claims),
            99 * BTC_MSAT // 10,
        )
        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in projection.decisions],
            [
                (CUSTODY_CANDIDATE, 99 * BTC_MSAT // 10),
                (CUSTODY_SUSPENSE, BTC_MSAT // 10),
            ],
        )

    def test_network_fee_is_not_a_candidate_claim_slice(self):
        fee_msat = BTC_MSAT // 10_000
        rows = [
            _row(
                "out",
                "old",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                fee_msat=fee_msat,
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "new",
                "inbound",
                99 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]
        observations = _observations(rows)
        candidate = _candidate(rows, ["out"], ["return"])

        compilation = compile_gap_candidate_claims(candidate, observations)
        projection = project_quantities(list(observations.values()), compilation.claims)

        self.assertEqual(
            sum(claim.source.amount_msat for claim in compilation.claims),
            candidate.retained_msat,
        )
        self.assertTrue(
            all(
                claim.source.end_msat <= observations["out"].principal_msat
                for claim in compilation.claims
            )
        )
        fee_posting = next(
            item for item in projection.postings if item.location_kind == "fee"
        )
        self.assertEqual(fee_posting.amount_msat, fee_msat)

    def test_many_postmix_returns_compile_as_one_atomic_bundle(self):
        rows = [
            _row(
                "out",
                "old",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            )
        ]
        rows.extend(
            _row(
                f"return-{index:02d}",
                "operative",
                "inbound",
                33 * BTC_MSAT // 100,
                f"2021-{index // 3 + 1:02d}-{index % 3 + 1:02d}T00:00:00Z",
            )
            for index in range(30)
        )
        observations = _observations(rows)
        return_ids = [row["id"] for row in rows[1:]]
        candidate = _candidate(rows, ["out"], return_ids)

        compilation = compile_gap_candidate_claims(candidate, observations)

        self.assertEqual(len(compilation.claims), 30)
        self.assertEqual(
            {claim.atomic_bundle_id for claim in compilation.claims},
            {f"candidate:{candidate.gap_id}"},
        )
        self.assertEqual(compilation.retained_msat, 99 * BTC_MSAT // 10)

    def test_excess_return_remains_external_origin(self):
        rows = [
            _row(
                "out",
                "old",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "new",
                "inbound",
                105 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]
        observations = _observations(rows)
        candidate = _candidate(rows, ["out"], ["return"])

        compilation = compile_gap_candidate_claims(candidate, observations)
        projection = project_quantities(list(observations.values()), compilation.claims)

        self.assertEqual(compilation.retained_msat, 10 * BTC_MSAT)
        self.assertEqual(compilation.excess_msat, BTC_MSAT // 2)
        external_origins = [
            item for item in projection.postings if item.location_kind == "external_origin"
        ]
        self.assertEqual([item.amount_msat for item in external_origins], [-BTC_MSAT // 2])

    def test_n_to_one_and_n_to_m_allocations_are_exact(self):
        for return_amounts in (
            [99 * BTC_MSAT // 10],
            [5 * BTC_MSAT, 49 * BTC_MSAT // 10],
        ):
            with self.subTest(return_amounts=return_amounts):
                rows = [
                    _row(
                        "out-a",
                        "old-a",
                        "outbound",
                        6 * BTC_MSAT,
                        "2020-01-01T00:00:00Z",
                        privacy_boundary="coinjoin",
                    ),
                    _row(
                        "out-b",
                        "old-b",
                        "outbound",
                        4 * BTC_MSAT,
                        "2020-01-02T00:00:00Z",
                        privacy_boundary="coinjoin",
                    ),
                ]
                rows.extend(
                    _row(
                        f"return-{index}",
                        "new",
                        "inbound",
                        amount,
                        f"2021-01-{index + 1:02d}T00:00:00Z",
                    )
                    for index, amount in enumerate(return_amounts)
                )
                observations = _observations(rows)
                candidate = _candidate(
                    rows,
                    ["out-a", "out-b"],
                    [row["id"] for row in rows[2:]],
                )

                compilation = compile_gap_candidate_claims(candidate, observations)

                self.assertEqual(
                    sum(claim.source.amount_msat for claim in compilation.claims),
                    99 * BTC_MSAT // 10,
                )
                self.assertEqual(
                    sum(claim.target.amount_msat for claim in compilation.claims),
                    99 * BTC_MSAT // 10,
                )

    def test_competing_promoted_bundles_conflict_in_the_arbiter(self):
        source = _row(
            "out",
            "old",
            "outbound",
            10 * BTC_MSAT,
            "2020-01-01T00:00:00Z",
            privacy_boundary="coinjoin",
        )
        return_a = _row(
            "return-a",
            "new-a",
            "inbound",
            99 * BTC_MSAT // 10,
            "2021-01-01T00:00:00Z",
        )
        return_b = _row(
            "return-b",
            "new-b",
            "inbound",
            99 * BTC_MSAT // 10,
            "2021-02-01T00:00:00Z",
        )
        observations = _observations([source, return_a, return_b])
        candidate_a = _candidate([source, return_a], ["out"], ["return-a"])
        candidate_b = _candidate([source, return_b], ["out"], ["return-b"])
        claims = [
            *compile_gap_candidate_claims(candidate_a, observations).claims,
            *compile_gap_candidate_claims(candidate_b, observations).claims,
        ]

        projection = project_quantities(list(observations.values()), claims)

        self.assertEqual(
            sum(
                item.source.amount_msat
                for item in projection.decisions
                if item.state == CONFLICTING
            ),
            99 * BTC_MSAT // 10,
        )
        self.assertEqual(
            sum(
                item.source.amount_msat
                for item in projection.decisions
                if item.state == CUSTODY_SUSPENSE
            ),
            BTC_MSAT // 10,
        )

    def test_compilation_is_invariant_to_observation_mapping_order(self):
        rows = [
            _row(
                "out",
                "old",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return-a",
                "new",
                "inbound",
                5 * BTC_MSAT,
                "2021-01-01T00:00:00Z",
            ),
            _row(
                "return-b",
                "new",
                "inbound",
                49 * BTC_MSAT // 10,
                "2021-01-02T00:00:00Z",
            ),
        ]
        candidate = _candidate(rows, ["out"], ["return-a", "return-b"])
        observations = _observations(rows)
        items = list(observations.items())
        random.Random(42).shuffle(items)

        left = compile_gap_candidate_claims(candidate, observations)
        right = compile_gap_candidate_claims(candidate, dict(items))

        self.assertEqual(left, right)

    def test_nonpromotion_hint_emits_no_claims(self):
        rows = [
            _row(
                "out", "old", "outbound", 10 * BTC_MSAT, "2020-01-01T00:00:00Z"
            ),
            _row(
                "return",
                "new",
                "inbound",
                99 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]
        candidate = _candidate(rows, ["out"], ["return"])

        compilation = compile_gap_candidate_claims(candidate, {})

        self.assertEqual(compilation.claims, ())
        self.assertIsNone(compilation.atomic_bundle_id)

    def test_missing_invalid_or_mismatched_observation_fails_without_partial_claims(self):
        rows = [
            _row(
                "out",
                "old",
                "outbound",
                10 * BTC_MSAT,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "new",
                "inbound",
                99 * BTC_MSAT // 10,
                "2021-01-01T00:00:00Z",
            ),
        ]
        candidate = _candidate(rows, ["out"], ["return"])
        observations = _observations(rows)

        with self.assertRaises(CustodyGapClaimCompileError) as missing:
            compile_gap_candidate_claims(candidate, {"out": observations["out"]})
        self.assertEqual(missing.exception.code, "custody_gap_claim_compile")

        invalid = replace(observations["return"], quantity_hash="not-a-hash")
        with self.assertRaises(CustodyGapClaimCompileError):
            compile_gap_candidate_claims(
                candidate, {**observations, "return": invalid}
            )

        mismatched = replace(candidate, source_total_msat=candidate.source_total_msat - 1)
        with self.assertRaises(CustodyGapClaimCompileError):
            compile_gap_candidate_claims(mismatched, observations)


if __name__ == "__main__":
    unittest.main()
