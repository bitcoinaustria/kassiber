"""Tests for deterministic, advisory long-horizon custody-gap matching."""

from dataclasses import asdict
from datetime import datetime, timedelta, timezone
import json
import random
import sqlite3
import time
import unittest
from unittest.mock import patch

from kassiber.core import custody_gaps
from kassiber.core.custody_gaps import (
    CustodyGapSearchLimitError,
    CustodyGapSearchResult,
    build_gap_snapshot,
    suggest_custody_gap_candidates,
)


BTC_MSAT = 100_000_000_000


def _row(
    row_id: str,
    *,
    direction: str,
    amount_btc: float,
    occurred_at: str,
    wallet_id: str,
    wallet_label: str | None = None,
    profile_id: str = "profile-one",
    asset: str = "BTC",
    fee: int = 0,
    amount_includes_fee: int = 0,
    kind: str = "",
    wallet_kind: str = "descriptor",
    privacy_boundary: str | None = None,
    wallet_config_json: str = "{}",
    raw_json: str = "{}",
):
    # Tests use decimal literals whose BTC->msat products are exact integers.
    return {
        "id": row_id,
        "profile_id": profile_id,
        "wallet_id": wallet_id,
        "wallet_label": wallet_label or wallet_id,
        "wallet_kind": wallet_kind,
        "wallet_config_json": wallet_config_json,
        "raw_json": raw_json,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": round(amount_btc * BTC_MSAT),
        "fee": fee,
        "amount_includes_fee": amount_includes_fee,
        "excluded": 0,
        "kind": kind,
        "privacy_boundary": privacy_boundary,
    }


class CustodyGapMatcherTests(unittest.TestCase):
    def test_candidates_never_cross_chain_network_scope(self):
        rows = [
            _row(
                "mainnet-out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="mainnet-wallet",
                kind="samourai_deposit",
                wallet_config_json=json.dumps(
                    {"chain": "bitcoin", "network": "main"}
                ),
            ),
            _row(
                "testnet-return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="testnet-wallet",
                wallet_config_json=json.dumps(
                    {"chain": "bitcoin", "network": "test"}
                ),
            ),
        ]

        self.assertEqual(suggest_custody_gap_candidates(rows), [])

    def test_onchain_and_lightning_observations_never_cross_match(self):
        rows = [
            _row(
                "onchain-out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="mainnet-wallet",
                kind="samourai_deposit",
            ),
            _row(
                "lightning-return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="lightning-node",
                wallet_kind="lnd",
            ),
        ]

        self.assertEqual(suggest_custody_gap_candidates(rows), [])

    def test_lightning_scope_is_explicit_and_stable(self):
        rows = [
            _row(
                "lightning-out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old-node",
                wallet_kind="coreln",
                privacy_boundary="coinjoin",
            ),
            _row(
                "lightning-return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="new-node",
                wallet_kind="lnd",
            ),
        ]

        candidate = suggest_custody_gap_candidates(rows)[0]

        self.assertEqual(candidate.protocol_chain, "lightning")
        self.assertEqual(candidate.network, "main")

    def test_unknown_protocol_scope_fails_closed_without_crashing_matcher(self):
        rows = [
            _row(
                "unknown-out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="future-wallet",
                privacy_boundary="coinjoin",
                wallet_config_json=json.dumps(
                    {"chain": "future-layer", "network": "main"}
                ),
            ),
            _row(
                "bitcoin-return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="bitcoin-wallet",
            ),
        ]

        self.assertEqual(suggest_custody_gap_candidates(rows), [])

    def test_ten_btc_out_and_nine_point_nine_back_a_year_later_is_suggested(self):
        rows = [
            _row(
                "out-a",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="multisig-b",
                wallet_label="Multisig B",
                kind="samourai_deposit",
            ),
            _row(
                "in-c",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="operative-c",
                wallet_label="Operative C",
            ),
        ]

        candidates = suggest_custody_gap_candidates(rows)

        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.source_ids, ("out-a",))
        self.assertEqual(candidate.return_ids, ("in-c",))
        self.assertEqual(candidate.source_total_msat, 10 * BTC_MSAT)
        self.assertEqual(candidate.return_total_msat, 99 * BTC_MSAT // 10)
        self.assertEqual(candidate.retained_msat, 99 * BTC_MSAT // 10)
        self.assertEqual(candidate.residual_msat, BTC_MSAT // 10)
        self.assertEqual(candidate.excess_msat, 0)
        self.assertEqual(candidate.coverage_ppm, 990_000)
        self.assertEqual(candidate.confidence, "strong")
        self.assertIn("long_horizon", candidate.reason_codes)
        self.assertIn("structured_samourai_transaction", candidate.reason_codes)
        self.assertTrue(candidate.promotion_eligible)

    def test_split_returns_form_one_bounded_n_to_m_candidate(self):
        rows = [
            _row(
                "out-a",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old-wallet",
            ),
            _row(
                "return-4",
                direction="inbound",
                amount_btc=4,
                occurred_at="2020-05-01T00:00:00Z",
                wallet_id="new-wallet",
            ),
            _row(
                "return-3",
                direction="inbound",
                amount_btc=3,
                occurred_at="2020-06-01T00:00:00Z",
                wallet_id="new-wallet",
            ),
            _row(
                "return-2.9",
                direction="inbound",
                amount_btc=2.9,
                occurred_at="2020-07-01T00:00:00Z",
                wallet_id="new-wallet",
            ),
        ]

        candidates = suggest_custody_gap_candidates(rows)

        split = next(candidate for candidate in candidates if len(candidate.return_ids) == 3)
        self.assertEqual(split.return_ids, ("return-2.9", "return-3", "return-4"))
        self.assertEqual(split.return_total_msat, 99 * BTC_MSAT // 10)
        self.assertEqual(split.residual_msat, BTC_MSAT // 10)
        self.assertIn("split_return", split.reason_codes)

    def test_competing_return_groups_share_a_visible_conflict_cluster(self):
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            ),
            _row(
                "possible-a",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="candidate-a",
            ),
            _row(
                "possible-b",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-02-01T00:00:00Z",
                wallet_id="candidate-b",
            ),
        ]

        candidates = suggest_custody_gap_candidates(rows)

        self.assertEqual(len(candidates), 2)
        self.assertEqual({candidate.conflict_size for candidate in candidates}, {2})
        self.assertEqual(len({candidate.conflict_set_id for candidate in candidates}), 1)
        self.assertEqual(
            {candidate.return_ids for candidate in candidates},
            {("possible-a",), ("possible-b",)},
        )

    def test_excess_return_never_manufactures_retained_principal(self):
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            ),
            _row(
                "return",
                direction="inbound",
                amount_btc=10.5,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="new",
            ),
        ]

        candidate = suggest_custody_gap_candidates(rows)[0]

        self.assertEqual(candidate.retained_msat, 10 * BTC_MSAT)
        self.assertEqual(candidate.residual_msat, 0)
        self.assertEqual(candidate.excess_msat, BTC_MSAT // 2)
        self.assertIn("return_exceeds_source", candidate.reason_codes)

    def test_source_fee_is_separate_from_principal_residual_and_wallet_debit(self):
        fee_msat = BTC_MSAT // 10_000  # 0.0001 BTC
        candidate = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                    fee=fee_msat,
                ),
                _row(
                    "return",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new",
                ),
            ]
        )[0]

        self.assertEqual(candidate.source_total_msat, 10 * BTC_MSAT)
        self.assertEqual(candidate.retained_msat, 99 * BTC_MSAT // 10)
        self.assertEqual(candidate.residual_msat, BTC_MSAT // 10)
        self.assertEqual(candidate.source_fee_msat, fee_msat)
        self.assertEqual(candidate.source_debit_msat, 10 * BTC_MSAT + fee_msat)

    def test_net_delta_with_explicit_fee_derives_principal_without_double_counting(self):
        fee_msat = BTC_MSAT // 10_000
        candidate = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10.0001,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                    fee=fee_msat,
                    amount_includes_fee=1,
                ),
                _row(
                    "return",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new",
                ),
            ]
        )[0]

        self.assertEqual(candidate.source_total_msat, 10 * BTC_MSAT)
        self.assertEqual(candidate.source_fee_msat, fee_msat)
        self.assertEqual(candidate.source_debit_msat, 10 * BTC_MSAT + fee_msat)
        self.assertEqual(candidate.residual_msat, BTC_MSAT // 10)

    def test_net_delta_without_known_fee_does_not_invent_a_fee_split(self):
        observed_debit = 10 * BTC_MSAT + BTC_MSAT // 10_000
        candidate = suggest_custody_gap_candidates(
            [
                {
                    **_row(
                        "out",
                        direction="outbound",
                        amount_btc=10,
                        occurred_at="2020-01-01T00:00:00Z",
                        wallet_id="old",
                        amount_includes_fee=1,
                    ),
                    "amount": observed_debit,
                },
                _row(
                    "return",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new",
                ),
            ]
        )[0]

        self.assertEqual(candidate.source_total_msat, observed_debit)
        self.assertEqual(candidate.source_fee_msat, 0)
        self.assertEqual(candidate.source_debit_msat, observed_debit)
        self.assertEqual(candidate.residual_msat, observed_debit - 99 * BTC_MSAT // 10)

    def test_results_are_invariant_to_input_permutation(self):
        rows = [
            _row(
                "out-1",
                direction="outbound",
                amount_btc=6,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            ),
            _row(
                "out-2",
                direction="outbound",
                amount_btc=4,
                occurred_at="2020-01-02T00:00:00Z",
                wallet_id="old",
            ),
            _row(
                "in-1",
                direction="inbound",
                amount_btc=5,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="new-a",
            ),
            _row(
                "in-2",
                direction="inbound",
                amount_btc=4.9,
                occurred_at="2021-01-02T00:00:00Z",
                wallet_id="new-b",
            ),
        ]
        expected = [asdict(candidate) for candidate in suggest_custody_gap_candidates(rows)]

        shuffled = list(rows)
        random.Random(42).shuffle(shuffled)
        actual = [asdict(candidate) for candidate in suggest_custody_gap_candidates(shuffled)]

        self.assertEqual(actual, expected)

    def test_candidate_population_ceiling_fails_instead_of_hiding_candidates(self):
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            )
        ]
        rows.extend(
            _row(
                f"in-{index}",
                direction="inbound",
                amount_btc=9.9,
                occurred_at=f"2021-{index + 1:02d}-01T00:00:00Z",
                wallet_id=f"new-{index}",
            )
            for index in range(6)
        )

        with self.assertRaisesRegex(
            CustodyGapSearchLimitError,
            "generated 6 candidates.*configured maximum is 3",
        ):
            suggest_custody_gap_candidates(rows, max_candidates=3)

        candidates = suggest_custody_gap_candidates(rows, max_candidates=6)
        self.assertEqual(len(candidates), 6)
        self.assertTrue(all(candidate.conflict_size == 6 for candidate in candidates))

        with self.assertRaises(CustodyGapSearchLimitError):
            suggest_custody_gap_candidates(rows, max_input_rows=2)

    def test_candidate_ceiling_cannot_hide_a_promotion_eligible_gap(self):
        rows = [
            _row(
                "structured-out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="structured-old",
                privacy_boundary="coinjoin",
            ),
            _row(
                "structured-return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="structured-new",
            ),
            _row(
                "hint-out",
                direction="outbound",
                amount_btc=5,
                occurred_at="2020-02-01T00:00:00Z",
                wallet_id="hint-old",
                profile_id="profile-two",
            ),
            _row(
                "hint-return",
                direction="inbound",
                amount_btc=4.9,
                occurred_at="2021-02-01T00:00:00Z",
                wallet_id="hint-new",
                profile_id="profile-two",
            ),
        ]

        complete = suggest_custody_gap_candidates(rows, max_candidates=2)
        self.assertEqual(len(complete), 2)
        self.assertEqual(sum(candidate.promotion_eligible for candidate in complete), 1)

        with self.assertRaisesRegex(
            CustodyGapSearchLimitError,
            "including 1 promotion-eligible candidates",
        ):
            suggest_custody_gap_candidates(rows, max_candidates=1)

    def test_candidate_ceiling_reports_zero_promotion_eligible_hints(self):
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            )
        ]
        rows.extend(
            _row(
                f"in-{index}",
                direction="inbound",
                amount_btc=9.9,
                occurred_at=f"2021-{index + 1:02d}-01T00:00:00Z",
                wallet_id=f"new-{index}",
            )
            for index in range(3)
        )

        with self.assertRaises(CustodyGapSearchLimitError) as raised:
            suggest_custody_gap_candidates(rows, max_candidates=2)

        self.assertEqual(raised.exception.candidate_count, 3)
        self.assertEqual(raised.exception.promotion_eligible_count, 0)

    def test_separate_profiles_and_assets_never_cross_match(self):
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            ),
            _row(
                "other-profile",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="new",
                profile_id="profile-two",
            ),
            _row(
                "other-asset",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="new",
                asset="LBTC",
            ),
        ]
        self.assertEqual(suggest_custody_gap_candidates(rows), [])

    def test_amount_time_and_wallet_transition_are_nonblocking_search_hints(self):
        candidate = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                ),
                _row(
                    "return",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new",
                ),
            ]
        )[0]

        self.assertFalse(candidate.promotion_eligible)
        self.assertIn("search_hint_only", candidate.reason_codes)

    def test_label_only_fake_samourai_never_becomes_structured_evidence(self):
        candidate = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                    wallet_label="Definitely Samourai Postmix Whirlpool",
                    kind="maybe-samourai-postmix-backup",
                ),
                _row(
                    "return",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new",
                ),
            ]
        )[0]

        self.assertFalse(candidate.promotion_eligible)
        self.assertFalse(
            any(code.startswith("structured_") for code in candidate.reason_codes)
        )

    def test_typed_samourai_policy_metadata_is_a_structured_signal(self):
        candidate = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                    wallet_config_json=json.dumps(
                        {"samourai": {"role": "child", "section": "postmix"}}
                    ),
                ),
                _row(
                    "return",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new",
                ),
            ]
        )[0]

        self.assertTrue(candidate.promotion_eligible)
        self.assertIn("structured_samourai_policy", candidate.reason_codes)

    def test_false_friend_revenue_is_never_promotion_eligible(self):
        candidate = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                    privacy_boundary="coinjoin",
                ),
                _row(
                    "revenue",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new",
                    kind="revenue",
                ),
            ]
        )[0]

        self.assertFalse(candidate.promotion_eligible)
        self.assertIn("structured_external_origin", candidate.reason_codes)
        self.assertIn("promotion_ineligible_external_origin", candidate.reason_codes)

    def test_equal_structured_competitors_are_not_promotion_eligible(self):
        candidates = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                    privacy_boundary="coinjoin",
                ),
                _row(
                    "return-a",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new-a",
                ),
                _row(
                    "return-b",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new-b",
                ),
            ]
        )

        self.assertEqual(len(candidates), 2)
        self.assertEqual({candidate.competitor_score_margin for candidate in candidates}, {0})
        self.assertFalse(any(candidate.promotion_eligible for candidate in candidates))
        self.assertTrue(
            all(
                "competitor_margin_insufficient" in candidate.reason_codes
                for candidate in candidates
            )
        )

    def test_clear_structured_competitor_margin_can_promote_only_the_best(self):
        candidates = suggest_custody_gap_candidates(
            [
                _row(
                    "out",
                    direction="outbound",
                    amount_btc=10,
                    occurred_at="2020-01-01T00:00:00Z",
                    wallet_id="old",
                ),
                _row(
                    "return-supported",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new-a",
                    privacy_boundary="coinjoin",
                    wallet_kind="samourai",
                ),
                _row(
                    "return-weak",
                    direction="inbound",
                    amount_btc=9.9,
                    occurred_at="2021-01-01T00:00:00Z",
                    wallet_id="new-b",
                ),
            ]
        )

        supported = next(
            candidate
            for candidate in candidates
            if candidate.return_ids == ("return-supported",)
        )
        weak = next(
            candidate for candidate in candidates if candidate.return_ids == ("return-weak",)
        )
        self.assertEqual(supported.competitor_score_margin, 80)
        self.assertTrue(supported.promotion_eligible)
        self.assertFalse(weak.promotion_eligible)

    def test_dozen_scale_postmix_fan_in_aggregates_without_micromanagement(self):
        start = datetime(2020, 2, 1, tzinfo=timezone.utc)
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
                privacy_boundary="coinjoin",
            )
        ]
        for index in range(30):
            occurred_at = (start + timedelta(days=12 * index)).isoformat().replace(
                "+00:00", "Z"
            )
            rows.append(
                _row(
                    f"return-{index:02d}",
                    direction="inbound",
                    amount_btc=0.33,
                    occurred_at=occurred_at,
                    wallet_id="operative",
                )
            )

        candidates = suggest_custody_gap_candidates(rows)

        aggregate = next(candidate for candidate in candidates if len(candidate.return_ids) == 30)
        self.assertEqual(aggregate.return_total_msat, 99 * BTC_MSAT // 10)
        self.assertEqual(aggregate.residual_msat, BTC_MSAT // 10)
        self.assertTrue(aggregate.promotion_eligible)

    def test_wallet_era_ceiling_fails_instead_of_silently_sampling(self):
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            )
        ]
        rows.extend(
            _row(
                f"return-{index}",
                direction="inbound",
                amount_btc=1.98,
                occurred_at=f"2021-01-{index + 1:02d}T00:00:00Z",
                wallet_id="new",
            )
            for index in range(5)
        )

        with self.assertRaises(CustodyGapSearchLimitError):
            suggest_custody_gap_candidates(rows, max_aggregate_return_legs=4)

    def test_return_pool_ceiling_fails_instead_of_sampling_history(self):
        rows = [
            _row(
                "out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
            )
        ]
        rows.extend(
            _row(
                f"return-{index}",
                direction="inbound",
                amount_btc=1,
                occurred_at=f"2021-01-{index + 1:02d}T00:00:00Z",
                wallet_id=f"new-{index}",
            )
            for index in range(6)
        )

        with self.assertRaises(CustodyGapSearchLimitError):
            suggest_custody_gap_candidates(rows, max_return_pool=5)

    def test_source_group_ceiling_marks_search_incomplete_instead_of_sampling(self):
        rows = [
            _row(
                f"out-{index}",
                direction="outbound",
                amount_btc=10,
                occurred_at=f"2020-0{index + 1}-01T00:00:00Z",
                wallet_id="old",
            )
            for index in range(4)
        ]
        rows.append(
            _row(
                "return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="new",
            )
        )

        with self.assertRaises(CustodyGapSearchLimitError) as raised:
            suggest_custody_gap_candidates(rows, max_source_groups=2)

        self.assertEqual(raised.exception.limit_kind, "boundary_worklist")
        self.assertTrue(raised.exception.partial_candidates)
        self.assertFalse(raised.exception.search_complete)

    def test_more_than_87_sources_preserve_seeded_structured_candidate(self):
        rows = [
            _row(
                "structured-out",
                direction="outbound",
                amount_btc=10,
                occurred_at="2020-01-01T00:00:00Z",
                wallet_id="old",
                privacy_boundary="coinjoin",
            )
        ]
        rows.extend(
            _row(
                f"ordinary-out-{index:03d}",
                direction="outbound",
                amount_btc=1,
                occurred_at=(
                    datetime(2020, 2, 1, tzinfo=timezone.utc)
                    + timedelta(days=index)
                ).isoformat().replace("+00:00", "Z"),
                wallet_id="old",
            )
            for index in range(100)
        )
        rows.append(
            _row(
                "structured-return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id="new",
            )
        )

        with self.assertRaises(CustodyGapSearchLimitError) as raised:
            suggest_custody_gap_candidates(rows)

        seeded = [
            candidate
            for candidate in raised.exception.partial_candidates
            if candidate.source_ids == ("structured-out",)
            and candidate.return_ids == ("structured-return",)
        ]
        self.assertEqual(len(seeded), 1)
        self.assertFalse(seeded[0].promotion_eligible)
        self.assertIn("search_capacity_incomplete", seeded[0].reason_codes)
        self.assertEqual(raised.exception.blocking_source_ids, ("structured-out",))
        self.assertEqual(raised.exception.limit_kind, "boundary_worklist")

    def test_structured_source_scoring_is_bounded_without_losing_suspense_scope(self):
        rows = [
            _row(
                f"structured-out-{index:03d}",
                direction="outbound",
                amount_btc=10 + index,
                occurred_at=(
                    datetime(2020, 1, 1, tzinfo=timezone.utc)
                    + timedelta(days=index)
                ).isoformat().replace("+00:00", "Z"),
                wallet_id="old",
                privacy_boundary="coinjoin",
            )
            for index in range(300)
        ]
        rows.append(
            _row(
                "return",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2022-01-01T00:00:00Z",
                wallet_id="new",
            )
        )

        with (
            patch(
                "kassiber.core.custody_gaps._indexed_return_pool",
                wraps=custody_gaps._indexed_return_pool,
            ) as indexed_pool,
            self.assertRaises(CustodyGapSearchLimitError) as raised,
        ):
            suggest_custody_gap_candidates(rows, max_input_rows=1)

        self.assertLessEqual(
            indexed_pool.call_count, custody_gaps.DEFAULT_MAX_SOURCE_GROUPS
        )
        self.assertEqual(len(raised.exception.blocking_source_ids), 300)

    def test_wallet_era_result_group_cap_is_honored(self):
        raw_rows = [
            _row(
                f"return-{index}",
                direction="inbound",
                amount_btc=9.9,
                occurred_at="2021-01-01T00:00:00Z",
                wallet_id=f"new-{index}",
            )
            for index in range(3)
        ]
        legs = [
            custody_gaps._normalize_leg(row, set())
            for row in raw_rows
        ]

        groups = custody_gaps._wallet_era_return_groups(
            [leg for leg in legs if leg is not None],
            target=10 * BTC_MSAT,
            min_coverage_ppm=800_000,
            max_excess_ppm=250_000,
            max_legs=256,
            era_gap_seconds=180 * 86_400,
            result_limit=2,
        )

        self.assertEqual(len(groups), 2)

    def test_large_history_runtime_stays_bounded(self):
        rows = [
            _row(
                f"out-{index}",
                direction="outbound",
                amount_btc=1,
                occurred_at=f"2020-01-{index % 28 + 1:02d}T00:00:00Z",
                wallet_id="old",
            )
            for index in range(250)
        ]
        rows.extend(
            _row(
                f"in-{index}",
                direction="inbound",
                amount_btc=0.99,
                occurred_at=f"2021-01-{index % 28 + 1:02d}T00:00:00Z",
                wallet_id=f"new-{index}",
            )
            for index in range(250)
        )

        started = time.monotonic()
        with self.assertRaises(CustodyGapSearchLimitError):
            suggest_custody_gap_candidates(rows)
        elapsed = time.monotonic() - started

        self.assertLess(elapsed, 5.0)


class CustodyGapSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(":memory:")
        self.conn.row_factory = sqlite3.Row
        self.conn.executescript(
            """
            CREATE TABLE wallets (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                label TEXT NOT NULL,
                kind TEXT NOT NULL,
                config_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE profiles (
                id TEXT PRIMARY KEY,
                last_processed_at TEXT,
                last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
                journal_input_version INTEGER NOT NULL DEFAULT 0,
                last_processed_input_version INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE transactions (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                wallet_id TEXT NOT NULL,
                external_id TEXT,
                occurred_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                direction TEXT NOT NULL,
                asset TEXT NOT NULL,
                amount INTEGER NOT NULL,
                fee INTEGER NOT NULL DEFAULT 0,
                amount_includes_fee INTEGER NOT NULL DEFAULT 0,
                excluded INTEGER NOT NULL DEFAULT 0,
                kind TEXT,
                privacy_boundary TEXT,
                raw_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE journal_entries (
                profile_id TEXT NOT NULL,
                transaction_id TEXT NOT NULL,
                entry_type TEXT NOT NULL
            );
            CREATE TABLE journal_quantity_issues (
                issue_id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                issue_type TEXT NOT NULL,
                state TEXT NOT NULL,
                asset TEXT,
                amount_msat INTEGER,
                occurred_at TEXT,
                reason TEXT NOT NULL,
                blocks_from TEXT
            );
            CREATE TABLE custody_gap_candidate_snapshots (
                cache_token TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                version_json TEXT NOT NULL,
                summary_json TEXT NOT NULL,
                gaps_json TEXT NOT NULL
            );
            """
        )
        self.conn.execute("INSERT INTO profiles(id) VALUES ('profile-one')")
        self.conn.executemany(
            "INSERT INTO wallets(id, profile_id, label, kind) VALUES (?, ?, ?, ?)",
            [
                ("old", "profile-one", "Old Multisig", "descriptor"),
                ("new", "profile-one", "Operative", "descriptor"),
            ],
        )
        self.conn.executemany(
            """
            INSERT INTO transactions(
                id, profile_id, wallet_id, occurred_at, created_at, direction,
                asset, amount, fee, amount_includes_fee, excluded, kind
            ) VALUES (?, 'profile-one', ?, ?, ?, ?, 'BTC', ?, 0, 0, 0, ?)
            """,
            [
                (
                    "out",
                    "old",
                    "2020-01-01T00:00:00Z",
                    "2020-01-01T00:00:00Z",
                    "outbound",
                    10 * BTC_MSAT,
                    "samourai_deposit",
                ),
                (
                    "return",
                    "new",
                    "2021-01-01T00:00:00Z",
                    "2021-01-01T00:00:00Z",
                    "inbound",
                    99 * BTC_MSAT // 10,
                    "",
                ),
                (
                    "later-disposal",
                    "new",
                    "2022-01-01T00:00:00Z",
                    "2022-01-01T00:00:00Z",
                    "outbound",
                    BTC_MSAT,
                    "",
                ),
            ],
        )

    def tearDown(self):
        self.conn.close()

    def test_snapshot_is_bounded_privacy_safe_and_reports_downstream_impact(self):
        snapshot = build_gap_snapshot(self.conn, "profile-one")

        self.assertEqual(snapshot["summary"]["needs_review"], 1)
        gap = snapshot["gaps"][0]
        self.assertEqual(gap["status"], "needs_review")
        self.assertEqual(gap["source_wallet_label"], "Old Multisig")
        self.assertEqual(gap["destination_wallet_labels"], ["Operative"])
        self.assertEqual(gap["source_fee_msat"], 0)
        self.assertEqual(gap["source_debit_msat"], 10 * BTC_MSAT)
        self.assertEqual(gap["downstream"]["affected_disposals"], 1)
        self.assertEqual(gap["downstream"]["affected_years"], [2022])
        self.assertNotIn("raw_json", gap)
        self.assertNotIn("address", gap)

    def test_book_above_50k_rows_still_surfaces_structured_boundary(self):
        self.conn.executemany(
            """
            INSERT INTO transactions(
                id, profile_id, wallet_id, occurred_at, created_at, direction,
                asset, amount, fee, amount_includes_fee, excluded, kind
            ) VALUES (?, 'profile-one', 'old',
                      '2019-01-01T00:00:00Z', '2019-01-01T00:00:00Z',
                      'outbound', 'BTC', ?, 0, 0, 0, '')
            """,
            ((f"ordinary-{index:05d}", BTC_MSAT) for index in range(50_001)),
        )

        snapshot = build_gap_snapshot(self.conn, "profile-one")

        self.assertFalse(snapshot["summary"]["search_complete"])
        self.assertEqual(
            snapshot["summary"]["search_limit_kind"], "boundary_worklist"
        )
        self.assertIn(
            {"source_total_msat": 10 * BTC_MSAT, "return_total_msat": 99 * BTC_MSAT // 10},
            [
                {
                    "source_total_msat": gap["source_total_msat"],
                    "return_total_msat": gap["return_total_msat"],
                }
                for gap in snapshot["gaps"]
            ],
        )

    def test_large_book_keeps_high_value_untyped_missing_wallet_hint(self):
        self.conn.executemany(
            """
            INSERT INTO transactions(
                id, profile_id, wallet_id, occurred_at, created_at, direction,
                asset, amount, fee, amount_includes_fee, excluded, kind
            ) VALUES (?, 'profile-one', 'new',
                      '2019-01-01T00:00:00Z', '2019-01-01T00:00:00Z',
                      'inbound', 'BTC', 1, 0, 0, 0, '')
            """,
            ((f"ordinary-{index:05d}",) for index in range(50_001)),
        )
        self.conn.executemany(
            """
            INSERT INTO transactions(
                id, profile_id, wallet_id, occurred_at, created_at, direction,
                asset, amount, fee, amount_includes_fee, excluded, kind
            ) VALUES (?, 'profile-one', ?, ?, ?, ?, 'BTC', ?, 0, 0, 0, '')
            """,
            [
                (
                    "untyped-large-out",
                    "old",
                    "2020-02-01T00:00:00Z",
                    "2020-02-01T00:00:00Z",
                    "outbound",
                    20 * BTC_MSAT,
                ),
                (
                    "untyped-large-return",
                    "new",
                    "2021-02-01T00:00:00Z",
                    "2021-02-01T00:00:00Z",
                    "inbound",
                    198 * BTC_MSAT // 10,
                ),
            ],
        )

        snapshot = build_gap_snapshot(self.conn, "profile-one")

        self.assertFalse(snapshot["summary"]["search_complete"])
        self.assertIn(
            (20 * BTC_MSAT, 198 * BTC_MSAT // 10),
            {
                (gap["source_total_msat"], gap["return_total_msat"])
                for gap in snapshot["gaps"]
            },
        )

    def test_snapshot_marks_competing_candidates_as_conflicting(self):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, profile_id, wallet_id, occurred_at, created_at, direction,
                asset, amount, fee, amount_includes_fee, excluded, kind
            ) VALUES (
                'competing-return', 'profile-one', 'new',
                '2021-02-01T00:00:00Z', '2021-02-01T00:00:00Z', 'inbound',
                'BTC', ?, 0, 0, 0, ''
            )
            """,
            (99 * BTC_MSAT // 10,),
        )

        snapshot = build_gap_snapshot(self.conn, "profile-one")

        self.assertEqual(len(snapshot["gaps"]), 2)
        self.assertEqual({gap["status"] for gap in snapshot["gaps"]}, {"conflicting"})

    def test_snapshot_pages_actionable_candidates_before_reviewed_rows(self):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, profile_id, wallet_id, occurred_at, created_at, direction,
                asset, amount, fee, amount_includes_fee, excluded, kind
            ) VALUES (
                'competing-return', 'profile-one', 'new',
                '2021-02-01T00:00:00Z', '2021-02-01T00:00:00Z', 'inbound',
                'BTC', ?, 0, 0, 0, ''
            )
            """,
            (99 * BTC_MSAT // 10,),
        )
        candidates, _normalized = custody_gaps.load_gap_candidates(
            self.conn, "profile-one"
        )
        actionable_id = candidates[-1].gap_id

        def review_state(_conn, candidate, _review):
            if candidate.gap_id == actionable_id:
                return {"status": "needs_review", "reason": None}
            return {"status": "dismissed", "reason": None}

        with patch(
            "kassiber.core.custody_gap_reviews.review_state",
            side_effect=review_state,
        ):
            snapshot = build_gap_snapshot(self.conn, "profile-one", limit=1)

        self.assertEqual(snapshot["summary"]["needs_review"], 1)
        self.assertEqual(snapshot["summary"]["dismissed"], 1)
        self.assertEqual(snapshot["gaps"][0]["gap_id"], actionable_id)
        self.assertEqual(snapshot["gaps"][0]["status"], "needs_review")

    def test_snapshot_cursor_reaches_every_actionable_gap_after_first_page(self):
        actionable = [
            {
                "gap_id": f"gap-{index:03d}",
                "status": "needs_review",
                "asset": "BTC",
                "residual_msat": BTC_MSAT,
            }
            for index in range(101)
        ]
        reviewed = [
            {
                "gap_id": f"reviewed-{index:03d}",
                "status": "dismissed",
                "asset": "BTC",
                "residual_msat": 0,
            }
            for index in range(5)
        ]
        historical = [*reviewed, *actionable]
        with (
            patch(
                "kassiber.core.custody_gaps.load_gap_search_result",
                return_value=(
                    CustodyGapSearchResult(
                        candidates=(),
                        accounting_candidates=(),
                        search_complete=True,
                    ),
                    [],
                ),
            ) as load_search,
            patch(
                "kassiber.core.custody_gap_reviews.latest_reviews",
                return_value={},
            ),
            patch(
                "kassiber.core.custody_gap_reviews.historical_review_gaps",
                return_value=historical,
            ),
        ):
            self.conn.commit()
            first = build_gap_snapshot(self.conn, "profile-one", limit=100)
            self.assertFalse(self.conn.in_transaction)
            second = build_gap_snapshot(
                self.conn,
                "profile-one",
                limit=100,
                cursor=first["next_cursor"],
            )

        self.assertEqual(first["summary"]["total"], 106)
        self.assertEqual(
            [call.kwargs["limit"] for call in load_search.call_args_list],
            [custody_gaps.DEFAULT_MAX_CANDIDATES],
        )
        self.assertEqual(first["summary"]["needs_review"], 101)
        self.assertRegex(first["next_cursor"], r"^cg1\.[0-9a-f]{24}\.100$")
        self.assertTrue(all(gap["status"] == "needs_review" for gap in first["gaps"]))
        self.assertEqual(second["gaps"][0]["gap_id"], "gap-100")
        self.assertEqual(
            [gap["gap_id"] for gap in second["gaps"][1:]],
            [f"reviewed-{index:03d}" for index in range(5)],
        )
        self.assertIsNone(second["next_cursor"])

    def test_snapshot_cursor_expires_when_journal_input_changes(self):
        historical = [
            {
                "gap_id": f"gap-{index}",
                "status": "needs_review",
                "asset": "BTC",
                "residual_msat": 0,
            }
            for index in range(2)
        ]
        with (
            patch(
                "kassiber.core.custody_gaps.load_gap_search_result",
                return_value=(
                    CustodyGapSearchResult(
                        candidates=(),
                        accounting_candidates=(),
                        search_complete=True,
                    ),
                    [],
                ),
            ),
            patch(
                "kassiber.core.custody_gap_reviews.latest_reviews",
                return_value={},
            ),
            patch(
                "kassiber.core.custody_gap_reviews.historical_review_gaps",
                return_value=historical,
            ),
        ):
            first = build_gap_snapshot(self.conn, "profile-one", limit=1)

        self.conn.execute(
            "UPDATE profiles SET journal_input_version = journal_input_version + 1 "
            "WHERE id = 'profile-one'"
        )
        with self.assertRaisesRegex(ValueError, "cursor expired"):
            build_gap_snapshot(
                self.conn,
                "profile-one",
                limit=1,
                cursor=first["next_cursor"],
            )

    def test_snapshot_rejects_invalid_cursor(self):
        with self.assertRaisesRegex(ValueError, "cursor"):
            build_gap_snapshot(
                self.conn,
                "profile-one",
                cursor="not-an-offset",
            )

    def test_snapshot_reports_capacity_limit_as_advisory_incomplete_search(self):
        candidate = custody_gaps.load_gap_candidates(
            self.conn, "profile-one"
        )[0][0]
        limited = CustodyGapSearchResult(
            candidates=(candidate,),
            accounting_candidates=(),
            search_complete=False,
            message="bounded advisory search reached capacity",
            candidate_count=5_541,
            promotion_eligible_count=12,
            limit_kind="candidate_population",
        )
        with patch(
            "kassiber.core.custody_gaps.load_gap_search_result",
            return_value=(limited, []),
        ):
            snapshot = build_gap_snapshot(self.conn, "profile-one")
            found = custody_gaps.find_gap_candidate(
                self.conn, "profile-one", candidate.gap_id
            )

        self.assertFalse(snapshot["summary"]["search_complete"])
        self.assertEqual(snapshot["summary"]["search_status"], "capacity_limited")
        self.assertEqual(
            snapshot["summary"]["search_limit_kind"], "candidate_population"
        )
        self.assertEqual(snapshot["summary"]["search_candidate_count"], 5_541)
        self.assertEqual(snapshot["gaps"][0]["gap_id"], candidate.gap_id)
        self.assertEqual(found, candidate)
        self.assertFalse(CustodyGapSearchLimitError("capacity").blocking)

    def test_stale_journal_entries_do_not_suppress_candidates(self):
        self.conn.executemany(
            "INSERT INTO journal_entries(profile_id, transaction_id, entry_type) "
            "VALUES ('profile-one', ?, 'transfer_out')",
            [("out",), ("return",)],
        )

        snapshot = build_gap_snapshot(self.conn, "profile-one")

        self.assertEqual(snapshot["summary"]["total"], 1)
        self.assertFalse(snapshot["summary"]["derived_state_current"])

    def test_current_journal_entries_suppress_resolved_candidates(self):
        self.conn.executemany(
            "INSERT INTO journal_entries(profile_id, transaction_id, entry_type) "
            "VALUES ('profile-one', ?, 'transfer_out')",
            [("out",), ("return",)],
        )
        self.conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = '2022-01-02T00:00:00Z',
                last_processed_tx_count = 3,
                journal_input_version = 4,
                last_processed_input_version = 4
            WHERE id = 'profile-one'
            """
        )

        snapshot = build_gap_snapshot(self.conn, "profile-one")

        self.assertEqual(snapshot["summary"]["total"], 0)
        self.assertTrue(snapshot["summary"]["derived_state_current"])
        self.assertEqual(snapshot["gaps"], [])

    def test_candidate_residual_summary_is_separate_per_asset(self):
        self.conn.executemany(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json) "
            "VALUES (?, 'profile-one', ?, 'descriptor', ?)",
            [
                ("liquid-old", "Old Liquid", '{"chain":"liquid","network":"main"}'),
                ("liquid-new", "New Liquid", '{"chain":"liquid","network":"main"}'),
            ],
        )
        self.conn.executemany(
            """
            INSERT INTO transactions(
                id, profile_id, wallet_id, occurred_at, created_at, direction,
                asset, amount, fee, amount_includes_fee, excluded, kind
            ) VALUES (?, 'profile-one', ?, ?, ?, ?, 'LBTC', ?, 0, 0, 0, ?)
            """,
            [
                (
                    "liquid-out",
                    "liquid-old",
                    "2020-03-01T00:00:00Z",
                    "2020-03-01T00:00:00Z",
                    "outbound",
                    2 * BTC_MSAT,
                    "samourai_deposit",
                ),
                (
                    "liquid-return",
                    "liquid-new",
                    "2021-03-01T00:00:00Z",
                    "2021-03-01T00:00:00Z",
                    "inbound",
                    19 * BTC_MSAT // 10,
                    "",
                ),
            ],
        )

        snapshot = build_gap_snapshot(self.conn, "profile-one")

        self.assertEqual(
            snapshot["summary"]["candidate_residual_by_asset"],
            [
                {"asset": "BTC", "amount_msat": BTC_MSAT // 10},
                {"asset": "LBTC", "amount_msat": BTC_MSAT // 10},
            ],
        )
        self.assertEqual(snapshot["summary"]["candidate_residual_msat"], BTC_MSAT // 10)


if __name__ == "__main__":
    unittest.main()
