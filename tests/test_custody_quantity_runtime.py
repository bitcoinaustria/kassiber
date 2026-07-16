import json
import sqlite3
import unittest
from unittest.mock import patch

from kassiber.core.custody_evidence import EvidenceSnapshot
from kassiber.core.custody_gap_reviews import candidate_fingerprint
from kassiber.core.custody_gaps import (
    CustodyGapSearchResult,
    suggest_custody_gap_candidates,
)
from kassiber.core.custody_quantity import (
    CUSTODY_SUSPENSE,
    ClaimPriority,
    EXTERNAL_CONFIRMED,
    INTERNAL_REVIEWED,
    INTERNAL_VERIFIED,
    QuantityClaim,
    QuantitySlice,
)
from kassiber.core.custody_quantity_runtime import (
    build_canonical_quantity_state,
    canonical_internal_transfer_rows,
    compare_wallet_balances,
    enriched_quantity_rows,
)
from kassiber.core.custody_tax_projection import compile_finalized_tax_projection
from kassiber.core.custody_quantity_store import (
    baseline_missing_component_evidence,
    blocking_quantity_issues,
    capture_component_evidence,
    custody_decision_rows,
    custody_quantity_readiness_summary,
    persist_authored_evidence_snapshots,
    replace_canonical_quantity_state,
)
from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.db import SCHEMA
from kassiber.errors import AppError
from tests.custody_tax_helpers import authoritative_chain_observation


def _row(
    tx_id,
    wallet_id,
    direction,
    amount,
    occurred_at,
    *,
    fee=0,
    txid=None,
    config=None,
    privacy_boundary=None,
):
    row = {
        "id": tx_id,
        "wallet_id": wallet_id,
        "wallet_label": wallet_id,
        "profile_id": "profile-one",
        "wallet_kind": "descriptor",
        "config_json": json.dumps(config or {"chain": "bitcoin", "network": "main"}),
        "fingerprint": f"fingerprint:{tx_id}",
        "external_id": txid or tx_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "amount_includes_fee": 0,
        "excluded": 0,
        "privacy_boundary": privacy_boundary,
        "raw_json": json.dumps({"txid": txid or tx_id}),
    }
    # An explicit txid opts a fixture into a structurally observed canonical
    # on-chain event. Generic/provider-shaped rows remain non-authoritative.
    return authoritative_chain_observation(row) if txid is not None else row


class CustodyQuantityRuntimeTests(unittest.TestCase):
    @staticmethod
    def _reviewed_claim(rows, *, out_id, in_id, amount):
        preliminary = build_canonical_quantity_state(rows)
        observations = {
            item.transaction_id: item
            for item in preliminary.canonical_input.observations
        }
        source = observations[out_id]
        target = observations[in_id]
        return QuantityClaim(
            claim_id=f"reviewed:{out_id}:{in_id}:{amount}",
            source=QuantitySlice(source.quantity_hash, 0, amount),
            target=QuantitySlice(target.quantity_hash, 0, amount),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_PAIR,
            reason="reviewed_transfer_pair",
            supporting_evidence_hashes=tuple(
                sorted(
                    {
                        source.evidence_detail_hash,
                        target.evidence_detail_hash,
                    }
                )
            ),
        )

    @staticmethod
    def _native_audit(
        *,
        source_id,
        out_id,
        in_id,
        from_wallet,
        to_wallet,
        received,
        fee,
        occurred_at,
        pairing_source="ownership_derived",
    ):
        return {
            "out_id": out_id,
            "in_id": in_id,
            "out_anchor_transaction_id": source_id,
            "in_anchor_transaction_id": source_id,
            "from_wallet_id": from_wallet,
            "to_wallet_id": to_wallet,
            "asset": "BTC",
            "occurred_at": occurred_at,
            "pairing_source": pairing_source,
            "crypto_sent_msat": received + fee,
            "crypto_received_msat": received,
            "crypto_fee_msat": fee,
            "crypto_sent": (received + fee) / 100_000_000_000,
            "crypto_received": received / 100_000_000_000,
            "crypto_fee": fee / 100_000_000_000,
        }

    def test_rowless_owned_output_projects_exact_target_external_residual_and_fee(self):
        occurred_at = "2025-01-01T00:00:00Z"
        txid = "ab" * 32
        rows = [
            _row("acq", "wallet-a", "inbound", 710, "2024-01-01T00:00:00Z"),
            _row(
                "source",
                "wallet-a",
                "outbound",
                700,
                occurred_at,
                fee=10,
                txid=txid,
            ),
        ]
        audit = self._native_audit(
            source_id="source",
            out_id="owned-derive:out:0",
            in_id="owned-derive:in:0",
            from_wallet="wallet-a",
            to_wallet="wallet-b",
            received=500,
            fee=10,
            occurred_at=occurred_at,
        )

        state = build_canonical_quantity_state(rows, native_evidence=[audit])

        self.assertFalse(state.issues)
        self.assertEqual(
            sorted(
                (decision.state, decision.source.amount_msat)
                for decision in state.projection.decisions
            ),
            [("external_presumed", 200), ("internal_verified", 500)],
        )
        self.assertEqual(
            compare_wallet_balances(
                state,
                {("wallet-a", "BTC"): 0, ("wallet-b", "BTC"): 500},
            ),
            (),
        )
        synthetic = [
            item
            for item in state.canonical_input.observations
            if item.transaction_id.startswith("native-owned-in:")
        ]
        self.assertEqual(len(synthetic), 1)
        self.assertEqual(synthetic[0].event_key.native_event_id, txid)
        self.assertEqual(synthetic[0].amount_msat, 500)
        fee_postings = [
            item
            for item in state.projection.postings
            if item.location_kind == "fee"
        ]
        self.assertEqual([item.amount_msat for item in fee_postings], [10])

    def test_failed_rowless_native_proof_never_projects_fallback_disposal(self):
        occurred_at = "2025-01-01T00:00:00Z"
        source = _row(
            "source",
            "wallet-a",
            "outbound",
            1_000,
            occurred_at,
            txid="aa" * 32,
        )
        for key in (
            "observation_authority_version",
            "observation_graph_hash",
            "observation_quantity_hash",
            "observation_fee_attribution",
        ):
            source.pop(key, None)
        later = _row(
            "later",
            "wallet-a",
            "outbound",
            500,
            "2026-01-01T00:00:00Z",
            txid="bb" * 32,
        )
        audit = self._native_audit(
            source_id="source",
            out_id="owned-derive:out:0",
            in_id="owned-derive:in:0",
            from_wallet="wallet-a",
            to_wallet="wallet-b",
            received=1_000,
            fee=0,
            occurred_at=occurred_at,
        )

        state = build_canonical_quantity_state(
            [source, later], native_evidence=[audit]
        )
        projection = compile_finalized_tax_projection(
            {
                "id": "profile-one",
                "workspace_id": "workspace-one",
                "label": "Book",
            },
            [source, later],
            state,
        )

        self.assertFalse(projection.rows)
        self.assertIn(
            ("source", "custody_quantity_unresolved"),
            {
                (item["transaction_id"], item["reason"])
                for item in projection.quarantines
            },
        )
        self.assertIn(
            ("later", "custody_basis_barrier"),
            {
                (item["transaction_id"], item["reason"])
                for item in projection.quarantines
            },
        )
        self.assertFalse(state.tax_eligibility.eligible_decisions)

    def test_authoritative_rowless_native_proof_projects_only_internal_move(self):
        occurred_at = "2025-01-01T00:00:00Z"
        source = _row(
            "source",
            "wallet-a",
            "outbound",
            1_000,
            occurred_at,
            txid="ac" * 32,
        )
        audit = self._native_audit(
            source_id="source",
            out_id="owned-derive:out:0",
            in_id="owned-derive:in:0",
            from_wallet="wallet-a",
            to_wallet="wallet-b",
            received=1_000,
            fee=0,
            occurred_at=occurred_at,
        )

        state = build_canonical_quantity_state([source], native_evidence=[audit])
        projection = compile_finalized_tax_projection(
            {
                "id": "profile-one",
                "workspace_id": "workspace-one",
                "label": "Book",
            },
            [source],
            state,
        )

        self.assertFalse(state.issues)
        self.assertEqual(len(projection.intra_pairs), 1)
        self.assertEqual(
            {row["direction"] for row in projection.rows},
            {"inbound", "outbound"},
        )
        self.assertFalse(projection.quarantines)

    def test_malformed_interpreter_bundle_isolated_as_quantity_issue(self):
        rows = [
            _row("bad-out", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z"),
            _row(
                "good-out",
                "wallet-b",
                "outbound",
                200,
                "2025-02-01T00:00:00Z",
                config={"chain": "bitcoin", "network": "test"},
            ),
            _row(
                "good-in",
                "wallet-c",
                "inbound",
                200,
                "2025-02-01T00:00:00Z",
                config={"chain": "bitcoin", "network": "test"},
            ),
        ]
        baseline = build_canonical_quantity_state(rows)
        observations = {
            item.transaction_id: item
            for item in baseline.canonical_input.observations
        }
        malformed = QuantityClaim(
            claim_id="malformed",
            source=QuantitySlice(
                observations["bad-out"].quantity_hash, 0, 100
            ),
            target=QuantitySlice("missing-observation", 0, 100),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="malformed_interpreter",
            atomic_bundle_id="interpreter:bad",
        )
        valid = QuantityClaim(
            claim_id="valid",
            source=QuantitySlice(
                observations["good-out"].quantity_hash, 0, 200
            ),
            target=QuantitySlice(
                observations["good-in"].quantity_hash, 0, 200
            ),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="valid_interpreter",
            atomic_bundle_id="interpreter:good",
        )

        state = build_canonical_quantity_state(
            rows, interpreter_claims=[malformed, valid]
        )
        decisions = {
            next(
                item.transaction_id
                for item in state.projection.observations
                if item.quantity_hash == decision.source.observation_hash
            ): decision
            for decision in state.projection.decisions
        }

        self.assertEqual(decisions["bad-out"].state, CUSTODY_SUSPENSE)
        self.assertEqual(decisions["good-out"].state, INTERNAL_REVIEWED)
        self.assertIn(
            decisions["good-out"], state.tax_eligibility.eligible_decisions
        )
        compiler_issue = next(
            item
            for item in state.issues
            if item.issue_type == "quantity_claim_bundle_invalid"
        )
        self.assertEqual(compiler_issue.transaction_ids, ("bad-out",))
        self.assertEqual(
            compiler_issue.details["validation_reasons"],
            ["claim_target_invalid"],
        )

    def test_native_audit_reuses_exact_real_inbound_without_double_counting(self):
        occurred_at = "2025-01-01T00:00:00Z"
        txid = "bc" * 32
        rows = [
            _row("acq", "wallet-a", "inbound", 501, "2024-01-01T00:00:00Z"),
            _row(
                "source",
                "wallet-a",
                "outbound",
                500,
                occurred_at,
                fee=1,
                txid=txid,
            ),
            _row("real-in", "wallet-b", "inbound", 500, occurred_at, txid=txid),
        ]
        audit = self._native_audit(
            source_id="source",
            out_id="owned-derive:out:0",
            in_id="real-in",
            from_wallet="wallet-a",
            to_wallet="wallet-b",
            received=500,
            fee=1,
            occurred_at=occurred_at,
        )

        state = build_canonical_quantity_state(rows, native_evidence=[audit])

        target_observations = [
            item
            for item in state.canonical_input.observations
            if item.wallet_id == "wallet-b" and item.direction == "inbound"
        ]
        self.assertEqual([item.transaction_id for item in target_observations], ["real-in"])
        self.assertEqual(
            compare_wallet_balances(
                state,
                {("wallet-a", "BTC"): 0, ("wallet-b", "BTC"): 500},
            ),
            (),
        )

    def test_rowless_native_fanout_and_consolidation_use_aggregate_target_slots(self):
        occurred_at = "2025-01-01T00:00:00Z"
        txid = "cd" * 32
        rows = [
            _row("acq-a", "wallet-a", "inbound", 500, "2024-01-01T00:00:00Z"),
            _row("acq-b", "wallet-b", "inbound", 300, "2024-01-01T00:00:00Z"),
            _row("source-a", "wallet-a", "outbound", 500, occurred_at, txid=txid),
            _row("source-b", "wallet-b", "outbound", 300, occurred_at, txid=txid),
        ]
        audits = [
            self._native_audit(
                source_id="source-a",
                out_id="multi:out:a",
                in_id="multi:in:a",
                from_wallet="wallet-a",
                to_wallet="wallet-c",
                received=500,
                fee=0,
                occurred_at=occurred_at,
                pairing_source="multi_source_consolidation",
            ),
            self._native_audit(
                source_id="source-b",
                out_id="multi:out:b",
                in_id="multi:in:b",
                from_wallet="wallet-b",
                to_wallet="wallet-c",
                received=300,
                fee=0,
                occurred_at=occurred_at,
                pairing_source="multi_source_consolidation",
            ),
        ]

        state = build_canonical_quantity_state(rows, native_evidence=audits)

        targets = [
            item
            for item in state.canonical_input.observations
            if item.wallet_id == "wallet-c" and item.direction == "inbound"
        ]
        self.assertEqual(len(targets), 1)
        self.assertEqual(targets[0].amount_msat, 800)
        self.assertEqual(
            compare_wallet_balances(
                state,
                {
                    ("wallet-a", "BTC"): 0,
                    ("wallet-b", "BTC"): 0,
                    ("wallet-c", "BTC"): 800,
                },
            ),
            (),
        )

    def test_rowless_native_audit_without_real_anchor_fails_closed(self):
        occurred_at = "2025-01-01T00:00:00Z"
        rows = [
            _row(
                "source",
                "wallet-a",
                "outbound",
                500,
                occurred_at,
                txid="de" * 32,
            )
        ]
        audit = self._native_audit(
            source_id="missing",
            out_id="owned-derive:out:0",
            in_id="owned-derive:in:0",
            from_wallet="wallet-a",
            to_wallet="wallet-b",
            received=500,
            fee=0,
            occurred_at=occurred_at,
        )

        state = build_canonical_quantity_state(rows, native_evidence=[audit])

        self.assertTrue(state.report_blocked)
        self.assertIn(
            "native_audit_source_anchor_missing",
            {item.reason for item in state.issues},
        )
        self.assertFalse(
            any(
                item.transaction_id.startswith("native-owned-in:")
                for item in state.canonical_input.observations
            )
        )

    def test_reviewed_transfer_delta_is_an_atomic_fee_not_suspense(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                1_000,
                "2025-01-01T00:00:00Z",
                fee=1,
            ),
            _row("in", "wallet-b", "inbound", 1_000, "2025-01-01T00:01:00Z"),
        ]
        state = build_canonical_quantity_state(
            rows,
            interpreter_claims=[
                self._reviewed_claim(
                    rows,
                    out_id="out",
                    in_id="in",
                    amount=1_000,
                )
            ],
        )

        self.assertEqual(state.issues, ())
        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in state.projection.decisions],
            [(INTERNAL_REVIEWED, 1_000)],
        )
        fee = next(
            item for item in state.projection.postings if item.location_kind == "fee"
        )
        self.assertEqual(fee.amount_msat, 1)
        self.assertEqual(fee.location_id, "network_fee")

    def test_component_allocates_split_self_transfer_and_external_tail(self):
        rows = [
            _row("out", "wallet-a", "outbound", 5_000, "2025-01-01T00:00:00Z"),
            _row("in", "wallet-b", "inbound", 3_000, "2025-01-01T00:01:00Z"),
        ]
        state = build_canonical_quantity_state(
            rows,
            effective_components=[
                {
                    "id": "reviewed-split",
                    "effective_state": "active",
                    "component_type": "manual_bridge",
                    "conservation_mode": "quantity",
                    "legs": [
                        {
                            "id": "source",
                            "role": "source",
                            "transaction_id": "out",
                            "asset": "BTC",
                            "amount_msat": 5_000,
                        },
                        {
                            "id": "owned",
                            "role": "destination",
                            "transaction_id": "in",
                            "asset": "BTC",
                            "amount_msat": 3_000,
                        },
                        {
                            "id": "external",
                            "role": "external",
                            "location_ref": "reviewed-counterparty",
                            "asset": "BTC",
                            "amount_msat": 2_000,
                        },
                    ],
                    "allocations": [
                        {
                            "id": "owned-allocation",
                            "source_leg_id": "source",
                            "sink_leg_id": "owned",
                            "source_amount_msat": 3_000,
                            "sink_amount_msat": 3_000,
                        },
                        {
                            "id": "external-allocation",
                            "source_leg_id": "source",
                            "sink_leg_id": "external",
                            "source_amount_msat": 2_000,
                            "sink_amount_msat": 2_000,
                        },
                    ],
                    "economic_terms": [
                        {
                            "source_leg_id": "source",
                            "target_leg_id": "external",
                            "term_kind": "direct_swap_payout",
                            "legacy_source_id": "payout",
                            "review_kind": "direct-swap-payout",
                            "tax_policy": "taxable",
                            "payout_asset": "BTC",
                            "payout_amount_msat": 2_000,
                        }
                    ],
                }
            ],
        )

        self.assertEqual(state.issues, ())
        self.assertCountEqual(
            [(item.state, item.source.amount_msat) for item in state.projection.decisions],
            [(INTERNAL_REVIEWED, 3_000), (EXTERNAL_CONFIRMED, 2_000)],
        )
        payout = next(
            item
            for item in state.projection.postings
            if item.location_kind == "external"
        )
        self.assertEqual(payout.amount_msat, 2_000)
        self.assertEqual(
            payout.location_id,
            "component:reviewed-split:allocation:external-allocation",
        )

    def test_component_network_fee_covers_wallet_delta_without_consuming_principal(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                5_000,
                "2025-01-01T00:00:00Z",
                fee=1,
            ),
            _row("in", "wallet-b", "inbound", 5_000, "2025-01-01T00:01:00Z"),
        ]
        component = {
            "id": "reviewed-fee",
            "effective_state": "active",
            "component_type": "native_transfer",
            "conservation_mode": "quantity",
            "legs": [
                {
                    "id": "source",
                    "role": "source",
                    "transaction_id": "out",
                    "asset": "BTC",
                    "amount_msat": 5_001,
                },
                {
                    "id": "owned",
                    "role": "destination",
                    "transaction_id": "in",
                    "asset": "BTC",
                    "amount_msat": 5_000,
                },
                {
                    "id": "fee",
                    "role": "fee",
                    "transaction_id": "out",
                    "asset": "BTC",
                    "amount_msat": 1,
                },
            ],
            "allocations": [
                {
                    "id": "owned-allocation",
                    "source_leg_id": "source",
                    "sink_leg_id": "owned",
                    "source_amount_msat": 5_000,
                    "sink_amount_msat": 5_000,
                },
                {
                    "id": "fee-allocation",
                    "source_leg_id": "source",
                    "sink_leg_id": "fee",
                    "source_amount_msat": 1,
                    "sink_amount_msat": 1,
                },
            ],
        }

        state = build_canonical_quantity_state(rows, effective_components=[component])

        self.assertEqual(state.issues, ())
        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in state.projection.decisions],
            [(INTERNAL_REVIEWED, 5_000)],
        )
        fee = next(
            item for item in state.projection.postings if item.location_kind == "fee"
        )
        self.assertEqual(fee.amount_msat, 1)

        component["allocations"][1]["source_amount_msat"] = 2
        component["allocations"][1]["sink_amount_msat"] = 2
        failed = build_canonical_quantity_state(rows, effective_components=[component])
        self.assertIn(
            "component_claim_compile_failed",
            {issue.issue_type for issue in failed.issues},
        )

    def test_promoted_gap_candidate_holds_boundaries_without_transfer_edge(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10_000,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                9_900,
                "2021-01-01T00:00:00Z",
            ),
        ]

        state = build_canonical_quantity_state(rows)

        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in state.projection.decisions],
            [(CUSTODY_SUSPENSE, 10_000)],
        )
        self.assertTrue(all(item.target is None for item in state.projection.decisions))
        self.assertEqual(
            {hold.transaction_id for hold in state.gap_holds},
            {"out", "return"},
        )
        self.assertTrue(
            any(item.issue_type == "custody_gap_review_hold" for item in state.issues)
        )
        self.assertEqual(state.tax_eligibility.blocked_from, "2020-01-01T00:00:00Z")

    def test_only_current_gap_dismissal_suppresses_live_candidate(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10_000,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                9_900,
                "2021-01-01T00:00:00Z",
            ),
        ]
        candidate = next(
            item
            for item in suggest_custody_gap_candidates(
                enriched_quantity_rows(rows)
            )
            if item.promotion_eligible
        )

        current = build_canonical_quantity_state(
            rows,
            dismissed_gap_fingerprints={
                candidate.gap_id: candidate_fingerprint(candidate)
            },
        )
        stale = build_canonical_quantity_state(
            rows,
            dismissed_gap_fingerprints={candidate.gap_id: "0" * 64},
        )

        self.assertFalse(current.issues)
        self.assertFalse(current.gap_candidate_transaction_ids)
        self.assertTrue(stale.issues)
        self.assertEqual(
            stale.gap_candidate_transaction_ids,
            ("out", "return"),
        )

    def test_candidate_population_limit_is_advisory_not_a_basis_fact(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10_000,
                "2020-01-01T00:00:00Z",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                9_900,
                "2021-01-01T00:00:00Z",
            ),
        ]

        with patch(
            "kassiber.core.custody_quantity_runtime.search_custody_gap_candidates",
            return_value=CustodyGapSearchResult(
                candidates=(),
                accounting_candidates=(),
                search_complete=False,
                limit_kind="candidate_population",
                message="candidate population exceeds its hard ceiling",
            ),
        ):
            state = build_canonical_quantity_state(rows)

        self.assertFalse(state.report_blocked)
        self.assertIsNone(state.tax_eligibility.barrier_event_key)
        self.assertFalse(state.gap_candidate_transaction_ids)

    def test_weak_hint_population_limit_does_not_poison_tax_basis(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10_000,
                "2020-01-01T00:00:00Z",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                9_900,
                "2021-01-01T00:00:00Z",
            ),
        ]

        with patch(
            "kassiber.core.custody_quantity_runtime.search_custody_gap_candidates",
            return_value=CustodyGapSearchResult(
                candidates=(),
                accounting_candidates=(),
                search_complete=False,
                limit_kind="candidate_population",
                candidate_count=5_541,
                promotion_eligible_count=0,
                message="weak candidate population exceeds its display ceiling",
            ),
        ):
            state = build_canonical_quantity_state(rows)

        self.assertFalse(state.report_blocked)
        self.assertIsNone(state.tax_eligibility.barrier_event_key)
        self.assertFalse(state.gap_candidate_transaction_ids)

    def test_shared_candidate_does_not_duplicate_interpreter_blocked_slice(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10_000,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                9_900,
                "2021-01-01T00:00:00Z",
            ),
        ]
        candidate = next(
            item
            for item in suggest_custody_gap_candidates(
                enriched_quantity_rows(rows)
            )
            if item.promotion_eligible
        )

        state = build_canonical_quantity_state(
            rows,
            ignored_gap_transaction_ids=("out",),
            gap_search_result=CustodyGapSearchResult(
                candidates=(candidate,),
                accounting_candidates=(candidate,),
                search_complete=True,
                candidate_count=1,
                promotion_eligible_count=1,
            ),
        )

        self.assertFalse(state.report_blocked)
        self.assertFalse(state.gap_candidate_transaction_ids)
        self.assertFalse(state.gap_holds)

    def test_incomplete_structured_search_blocks_only_source_disposal(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10_000,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                9_900,
                "2021-01-01T00:00:00Z",
            ),
        ]
        sampled_candidate = next(
            candidate
            for candidate in suggest_custody_gap_candidates(
                enriched_quantity_rows(rows)
            )
            if candidate.promotion_eligible
        )

        with patch(
            "kassiber.core.custody_quantity_runtime.search_custody_gap_candidates",
            return_value=CustodyGapSearchResult(
                candidates=(sampled_candidate,),
                accounting_candidates=(),
                search_complete=False,
                limit_kind="boundary_worklist",
                blocking_source_ids=("out",),
                candidate_count=1,
                promotion_eligible_count=1,
                message="structured boundary return worklist is incomplete",
            ),
        ):
            state = build_canonical_quantity_state(
                rows,
                dismissed_gap_fingerprints={
                    sampled_candidate.gap_id: candidate_fingerprint(
                        sampled_candidate
                    )
                },
            )

        decisions_by_transaction = {
            next(
                observation.transaction_id
                for observation in state.projection.observations
                if observation.quantity_hash == decision.source.observation_hash
            ): decision
            for decision in state.projection.decisions
        }
        self.assertEqual(decisions_by_transaction["out"].state, CUSTODY_SUSPENSE)
        self.assertEqual(state.gap_candidate_transaction_ids, ("out",))
        self.assertTrue(state.report_blocked)
        self.assertFalse(state.tax_eligibility.eligible_decisions)

        projection = compile_finalized_tax_projection(
            {
                "id": "profile-one",
                "workspace_id": "workspace-one",
                "label": "Book",
            },
            rows,
            state,
        )
        self.assertFalse(projection.rows)
        self.assertIn(
            ("out", "custody_quantity_unresolved"),
            {
                (item["transaction_id"], item["reason"])
                for item in projection.quarantines
            },
        )

    def test_structured_candidates_survive_the_display_population_limit(self):
        rows = [
            _row(
                "out",
                "wallet-a",
                "outbound",
                10_000,
                "2020-01-01T00:00:00Z",
                privacy_boundary="coinjoin",
            ),
            _row(
                "return",
                "wallet-c",
                "inbound",
                9_900,
                "2021-01-01T00:00:00Z",
            ),
        ]
        candidate = next(
            item
            for item in suggest_custody_gap_candidates(enriched_quantity_rows(rows))
            if item.promotion_eligible
        )

        with patch(
            "kassiber.core.custody_quantity_runtime.search_custody_gap_candidates",
            return_value=CustodyGapSearchResult(
                candidates=(),
                accounting_candidates=(candidate,),
                search_complete=False,
                limit_kind="candidate_population",
                candidate_count=501,
                promotion_eligible_count=1,
                message="candidate population exceeds its display ceiling",
            ),
        ):
            state = build_canonical_quantity_state(rows)

        self.assertTrue(state.report_blocked)
        self.assertEqual(state.tax_eligibility.blocked_from, "2020-01-01T00:00:00Z")
        self.assertEqual(
            state.gap_candidate_transaction_ids,
            ("out", "return"),
        )

    def test_known_correct_acquisition_and_payment_match_current_wallet_balance(self):
        rows = [
            _row("acq", "wallet-a", "inbound", 1_000, "2024-01-01T00:00:00Z"),
            _row(
                "payment",
                "wallet-a",
                "outbound",
                100,
                "2025-01-01T00:00:00Z",
                fee=2,
            ),
        ]
        state = build_canonical_quantity_state(rows)
        self.assertEqual(
            compare_wallet_balances(state, {("wallet-a", "BTC"): 898}),
            (),
        )
        self.assertFalse(state.report_blocked)

    def test_known_correct_internal_transfer_matches_both_wallets(self):
        txid = "ab" * 32
        rows = [
            _row("acq", "wallet-a", "inbound", 1_000, "2024-01-01T00:00:00Z"),
            _row(
                "move-out",
                "wallet-a",
                "outbound",
                99,
                "2025-01-01T00:00:00Z",
                fee=1,
                txid=txid,
            ),
            _row(
                "move-in",
                "wallet-b",
                "inbound",
                99,
                "2025-01-01T00:00:00Z",
                txid=txid,
            ),
        ]
        state = build_canonical_quantity_state(rows)
        self.assertEqual(
            compare_wallet_balances(
                state,
                {("wallet-a", "BTC"): 900, ("wallet-b", "BTC"): 99},
            ),
            (),
        )

    def test_current_non_event_difference_requires_a_named_reason(self):
        row = _row(
            "loan-lock",
            "wallet-a",
            "outbound",
            500,
            "2025-01-01T00:00:00Z",
        )
        state = build_canonical_quantity_state([row])
        unnamed = compare_wallet_balances(state, {("wallet-a", "BTC"): 0})
        self.assertIsNone(unnamed[0].reason)
        named = compare_wallet_balances(
            state,
            {("wallet-a", "BTC"): 0},
            known_non_event_reasons={"loan-lock": "loan_collateral_lock_non_event"},
        )
        self.assertEqual(named[0].reason, "loan_collateral_lock_non_event")

    def test_rejected_event_blocks_but_unrelated_quantity_still_projects(self):
        txid = "ab" * 32
        rows = [
            _row(
                "bad-a",
                "wallet-a",
                "outbound",
                100,
                "2025-01-01T00:00:00Z",
                txid=txid,
            ),
            _row(
                "bad-b",
                "wallet-a",
                "outbound",
                101,
                "2025-01-01T00:00:00Z",
                txid=txid,
            ),
            _row("good", "wallet-b", "inbound", 50, "2024-01-01T00:00:00Z"),
        ]
        state = build_canonical_quantity_state(rows)
        self.assertTrue(state.report_blocked)
        self.assertEqual(state.issues[0].issue_type, "canonical_event_rejected")
        self.assertEqual(
            state.tax_eligibility.blocked_from,
            "2025-01-01T00:00:00Z",
        )
        self.assertTrue(
            any(item.location_id == "wallet-b" for item in state.projection.postings)
        )

    def test_unresolved_quantity_never_enters_finalized_projection_and_blocks_later(self):
        rows = [
            _row("early", "wallet-a", "outbound", 10, "2024-01-01T00:00:00Z"),
            _row("gap", "wallet-a", "outbound", 20, "2025-01-01T00:00:00Z"),
            _row("late", "wallet-a", "outbound", 30, "2026-01-01T00:00:00Z"),
        ]
        baseline = build_canonical_quantity_state(rows)
        gap = next(
            item for item in baseline.projection.observations
            if item.transaction_id == "gap"
        )
        suspense = QuantityClaim(
            claim_id="gap-suspense",
            source=QuantitySlice(gap.quantity_hash, 0, 20),
            state=CUSTODY_SUSPENSE,
            priority=ClaimPriority.ACCOUNTING_CONVENTION,
            reason="missing_wallet",
        )
        state = build_canonical_quantity_state(rows, interpreter_claims=[suspense])
        observations = {
            item.quantity_hash: item for item in state.projection.observations
        }
        finalized_ids = {
            observations[item.source.observation_hash].transaction_id
            for item in state.tax_eligibility.eligible_decisions
        }
        self.assertEqual(finalized_ids, {"early"})
        self.assertEqual(
            state.tax_eligibility.blocked_from,
            "2025-01-01T00:00:00Z",
        )
        self.assertTrue(
            all(item.finalized for item in state.tax_eligibility.eligible_decisions)
        )
        self.assertFalse(
            any(
                item.selected_claim_id == "gap-suspense"
                for item in state.tax_eligibility.eligible_decisions
            )
        )
        suspense_decision = next(
            item
            for item in state.projection.decisions
            if item.selected_claim_id == "gap-suspense"
        )
        self.assertIn(
            suspense_decision.source,
            state.tax_eligibility.ineligible_slices,
        )

    def test_exact_custody_transfer_remains_visible_behind_prior_basis_barrier(self):
        rows = [
            _row("gap", "wallet-a", "outbound", 20, "2025-01-01T00:00:00Z"),
            _row("move-out", "wallet-a", "outbound", 30, "2026-01-01T00:00:00Z"),
            _row("move-in", "wallet-b", "inbound", 30, "2026-01-01T00:00:00Z"),
        ]
        baseline = build_canonical_quantity_state(rows)
        observations = {
            item.transaction_id: item for item in baseline.projection.observations
        }
        state = build_canonical_quantity_state(
            rows,
            interpreter_claims=[
                QuantityClaim(
                    claim_id="gap-suspense",
                    source=QuantitySlice(observations["gap"].quantity_hash, 0, 20),
                    state=CUSTODY_SUSPENSE,
                    priority=ClaimPriority.ACCOUNTING_CONVENTION,
                    reason="missing_wallet",
                ),
                QuantityClaim(
                    claim_id="exact-move",
                    source=QuantitySlice(
                        observations["move-out"].quantity_hash, 0, 30
                    ),
                    target=QuantitySlice(
                        observations["move-in"].quantity_hash, 0, 30
                    ),
                    state="internal_verified",
                    priority=ClaimPriority.EXACT_NATIVE_EVENT,
                    reason="recorded_fanout",
                ),
            ],
        )

        self.assertEqual(
            canonical_internal_transfer_rows(
                state,
                {
                    "wallet-a": {"label": "Cold"},
                    "wallet-b": {"label": "Hot"},
                },
            ),
            (
                {
                    "out_transaction_id": "move-out",
                    "in_transaction_id": "move-in",
                    "occurred_at": "2026-01-01T00:00:00Z",
                    "asset": "BTC",
                    "amount_msat": 30,
                    "from_wallet_id": "wallet-a",
                    "from_wallet": "Cold",
                    "to_wallet_id": "wallet-b",
                    "to_wallet": "Hot",
                    "custody_state": "internal_verified",
                    "basis_state": "blocked_by_prior_custody_basis",
                    "evidence_reason": "recorded_fanout",
                    "network": "main",
                    "rail": "bitcoin",
                },
            ),
        )

    def test_basis_barrier_is_scoped_to_profile_exposure_pool(self):
        rows = [
            _row("early", "wallet-a", "outbound", 10, "2024-01-01T00:00:00Z"),
            _row("gap", "wallet-a", "outbound", 20, "2025-01-01T00:00:00Z"),
            _row("late", "wallet-a", "outbound", 30, "2026-01-01T00:00:00Z"),
            _row("liquid-usdt", "wallet-l", "outbound", 40, "2026-01-01T00:00:00Z"),
            _row("other-profile", "wallet-z", "outbound", 50, "2026-01-01T00:00:00Z"),
        ]
        rows[3].update(
            {
                "asset": "USDT",
                "config_json": json.dumps(
                    {"chain": "liquid", "network": "liquidv1"}
                ),
                "raw_json": json.dumps(
                    {"chain": "liquid", "network": "liquidv1"}
                ),
            }
        )
        rows[4]["profile_id"] = "profile-two"
        baseline = build_canonical_quantity_state(rows)
        gap = next(
            item for item in baseline.projection.observations
            if item.transaction_id == "gap"
        )
        state = build_canonical_quantity_state(
            rows,
            interpreter_claims=[
                QuantityClaim(
                    claim_id="gap-suspense",
                    source=QuantitySlice(gap.quantity_hash, 0, 20),
                    state=CUSTODY_SUSPENSE,
                    priority=ClaimPriority.ACCOUNTING_CONVENTION,
                    reason="missing_wallet",
                )
            ],
        )
        observations = {
            item.quantity_hash: item for item in state.projection.observations
        }
        eligible_ids = {
            observations[item.source.observation_hash].transaction_id
            for item in state.tax_eligibility.eligible_decisions
        }

        self.assertEqual(
            eligible_ids,
            {"early", "liquid-usdt", "other-profile"},
        )
        self.assertEqual(len(state.tax_eligibility.pool_barriers), 1)
        unrelated = next(
            item for item in observations.values()
            if item.transaction_id == "liquid-usdt"
        )
        other_profile = next(
            item for item in observations.values()
            if item.transaction_id == "other-profile"
        )
        self.assertIsNone(state.tax_eligibility.barrier_for(unrelated))
        self.assertIsNone(state.tax_eligibility.barrier_for(other_profile))

    def test_component_detail_enrichment_keeps_unchanged_quantity_active(self):
        txid = "dc" * 32
        rows = [
            _row(
                "source",
                "wallet-a",
                "outbound",
                100,
                "2025-01-01T00:00:00Z",
                txid=txid,
            ),
            _row(
                "target",
                "wallet-b",
                "inbound",
                100,
                "2025-01-01T00:00:00Z",
                txid=txid,
            ),
        ]
        original = build_canonical_quantity_state(rows)
        stored = [
            {
                "quantity_hash": snapshot.quantity_hash,
                "detail_hash": snapshot.detail_hash,
                "payload_json": snapshot.payload_json,
            }
            for event in original.canonical_input.events
            for snapshot in event.evidence_snapshots
        ]
        component = {
            "id": "component-detail",
            "effective_state": "active",
            "legs": [
                {"id": "source", "role": "source", "transaction_id": "source"},
                {"id": "target", "role": "destination", "transaction_id": "target"},
            ],
            "allocations": [
                {
                    "id": "all",
                    "source_leg_id": "source",
                    "sink_leg_id": "target",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
        }
        enriched = [dict(row) for row in rows]
        for ordinal, row in enumerate(enriched):
            row["occurred_at"] = "2025-01-01T00:00:01Z"
            row["confirmed_at"] = "2025-01-01T00:10:00Z"
            row["fingerprint"] = f"enriched:{ordinal}"
            row["raw_json"] = json.dumps(
                {
                    "txid": txid,
                    "chain": "bitcoin",
                    "network": "main",
                    "block_height": 900_000,
                    "graph_version": 2,
                }
            )

        state = build_canonical_quantity_state(
            enriched,
            effective_components=[component],
            component_evidence_snapshots={"component-detail": stored},
        )

        self.assertFalse(state.issues)
        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in state.projection.decisions],
            [(INTERNAL_REVIEWED, 100)],
        )

    def test_component_quantity_or_identity_drift_still_fails_closed(self):
        rows = [
            _row("source", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z"),
            _row("target", "wallet-b", "inbound", 100, "2025-01-02T00:00:00Z"),
        ]
        original = build_canonical_quantity_state(rows)
        stored = [
            {
                "quantity_hash": snapshot.quantity_hash,
                "detail_hash": snapshot.detail_hash,
                "payload_json": snapshot.payload_json,
            }
            for event in original.canonical_input.events
            for snapshot in event.evidence_snapshots
        ]
        component = {
            "id": "component-quantity",
            "effective_state": "active",
            "legs": [
                {"id": "source", "role": "source", "transaction_id": "source"},
                {"id": "target", "role": "destination", "transaction_id": "target"},
            ],
            "allocations": [
                {
                    "id": "all",
                    "source_leg_id": "source",
                    "sink_leg_id": "target",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
        }
        changed = [dict(row) for row in rows]
        changed[1]["amount"] = 99

        state = build_canonical_quantity_state(
            changed,
            effective_components=[component],
            component_evidence_snapshots={"component-quantity": stored},
        )

        issue = next(
            item for item in state.issues
            if item.issue_type == "custody_component_evidence_drift"
        )
        self.assertEqual(
            issue.details["compiler_details"]["drift_kind"],
            "evidence_quantity_changed_or_missing",
        )
        self.assertEqual(state.projection.decisions[0].state, CUSTODY_SUSPENSE)

    def test_reviewed_component_projects_retained_target_and_residual_suspense(self):
        rows = [
            _row("source", "wallet-a", "outbound", 1_000, "2025-01-01T00:00:00Z"),
            _row("target", "wallet-c", "inbound", 900, "2026-01-01T00:00:00Z"),
        ]
        component = {
            "id": "component-1",
            "effective_state": "active",
            "legs": [
                {"id": "source-leg", "transaction_id": "source", "role": "source"},
                {"id": "target-leg", "transaction_id": "target", "role": "destination"},
                {"id": "suspense-leg", "transaction_id": None, "role": "suspense"},
            ],
            "allocations": [
                {
                    "id": "retained",
                    "ordinal": 0,
                    "source_leg_id": "source-leg",
                    "sink_leg_id": "target-leg",
                    "source_amount_msat": 900,
                    "sink_amount_msat": 900,
                },
                {
                    "id": "residual",
                    "ordinal": 1,
                    "source_leg_id": "source-leg",
                    "sink_leg_id": "suspense-leg",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                },
            ],
        }
        state = build_canonical_quantity_state(rows, effective_components=[component])
        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in state.projection.decisions],
            [("internal_reviewed", 900), (CUSTODY_SUSPENSE, 100)],
        )
        wallet_balances = {
            (item.location_id, item.asset): item.amount_msat
            for item in state.projection.postings
            if item.location_kind == "wallet"
        }
        self.assertEqual(wallet_balances, {("wallet-a", "BTC"): -1_000, ("wallet-c", "BTC"): 900})
        self.assertEqual(
            sum(
                item.amount_msat
                for item in state.projection.postings
                if item.location_kind == "custody_suspense"
            ),
            100,
        )
        self.assertEqual(len(state.issues), 1)
        self.assertEqual(state.issues[0].state, CUSTODY_SUSPENSE)
        self.assertEqual(state.tax_eligibility.blocked_from, "2025-01-01T00:00:00Z")

    def test_component_compile_failure_conflicts_and_suppresses_fallback(self):
        row = _row("source", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z")
        component = {
            "id": "broken",
            "effective_state": "active",
            "legs": [
                {"id": "source-leg", "transaction_id": "source", "role": "source"},
                {"id": "target-leg", "transaction_id": "missing", "role": "destination"},
            ],
            "allocations": [
                {
                    "id": "all",
                    "source_leg_id": "source-leg",
                    "sink_leg_id": "target-leg",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
        }
        state = build_canonical_quantity_state([row], effective_components=[component])
        self.assertTrue(
            any(
                item.issue_type == "component_claim_compile_failed"
                and item.state == "conflicting"
                for item in state.issues
            )
        )
        self.assertFalse(
            any(
                item.location_kind == "external"
                for item in state.projection.postings
            )
        )
        self.assertEqual(state.projection.decisions[0].state, CUSTODY_SUSPENSE)

    def test_reimported_durable_anchor_cannot_fall_back_after_live_fk_is_lost(self):
        row = _row("source", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z")
        component = {
            "id": "retracted-component",
            "effective_state": "draft",
            "evidence_status": {"status": "anchor_retracted"},
            "legs": [
                {
                    "id": "source-leg",
                    "transaction_id": None,
                    "anchor_transaction_id": "source",
                    "role": "source",
                },
                {
                    "id": "target-leg",
                    "transaction_id": None,
                    "anchor_transaction_id": "missing-target",
                    "role": "destination",
                },
            ],
            "allocations": [],
        }

        state = build_canonical_quantity_state([row], effective_components=[component])

        self.assertEqual(state.projection.decisions[0].state, CUSTODY_SUSPENSE)
        self.assertFalse(
            any(
                posting.location_kind == "external"
                for posting in state.projection.postings
            )
        )
        issue = next(
            item
            for item in state.issues
            if item.issue_type == "component_claim_compile_failed"
        )
        self.assertEqual(issue.transaction_ids, ("missing-target", "source"))

    def test_component_anchor_uses_deduplicated_transaction_alias(self):
        txid = "ab" * 32
        source = _row(
            "source", "wallet-a", "outbound", 100,
            "2025-01-01T00:00:00Z", txid=txid,
        )
        duplicate = dict(source)
        duplicate["id"] = "source-alias"
        duplicate["fingerprint"] = "fingerprint:source-alias"
        target = _row(
            "target", "wallet-c", "inbound", 100,
            "2025-01-01T00:00:00Z", txid=txid,
        )
        component = {
            "id": "component-alias",
            "effective_state": "active",
            "legs": [
                {"id": "source", "transaction_id": "source-alias", "role": "source"},
                {"id": "target", "transaction_id": "target", "role": "destination"},
            ],
            "allocations": [
                {
                    "id": "all",
                    "source_leg_id": "source",
                    "sink_leg_id": "target",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
        }
        state = build_canonical_quantity_state(
            [duplicate, target, source],
            effective_components=[component],
        )
        self.assertFalse(state.issues)
        self.assertEqual(state.projection.decisions[0].state, "internal_reviewed")


class CustodyQuantityStoreTests(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(":memory:")
        self.conn.row_factory = sqlite3.Row
        self.conn.executescript(SCHEMA)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', 'now')"
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, created_at) "
            "VALUES('profile', 'ws', 'Main', 'now')"
        )
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, created_at) "
            "VALUES('wallet-a', 'ws', 'profile', 'A', 'descriptor', 'now')"
        )

    def tearDown(self):
        self.conn.close()

    def _insert_transaction(self, row):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                amount_includes_fee, raw_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'now')
            """,
            (
                row["id"],
                row["wallet_id"],
                row["external_id"],
                row["fingerprint"],
                row["occurred_at"],
                row["direction"],
                row["asset"],
                row["amount"],
                row["fee"],
                row["amount_includes_fee"],
                row["raw_json"],
            ),
        )

    def test_canonical_state_replaces_derived_rows_and_stays_separate(self):
        row = _row("payment", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z")
        self._insert_transaction(row)
        state = build_canonical_quantity_state([row])
        counts = replace_canonical_quantity_state(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            state=state,
            created_at="now",
        )
        self.assertEqual(
            counts,
            {
                "postings": 2,
                "issues": 0,
                "balances": 2,
                "decisions": 0,
                "economic_relations": 0,
            },
        )
        readiness = custody_quantity_readiness_summary(
            self.conn,
            "profile",
            journal_status="current",
        )
        self.assertEqual(
            readiness["presumed_external"],
            {
                "slice_count": 1,
                "transaction_count": 1,
                "by_asset": [
                    {
                        "asset": "BTC",
                        "amount_msat": 100,
                        "slice_count": 1,
                        "transaction_count": 1,
                    }
                ],
                "treatment": "warning_not_blocker",
            },
        )
        self.assertEqual(
            readiness["warnings"][0]["code"], "external_custody_presumed"
        )
        self.assertEqual(blocking_quantity_issues(self.conn, "profile"), [])
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM journal_entries"
            ).fetchone()[0],
            0,
        )

    def test_reviewed_conversion_is_stored_as_an_economic_relation(self):
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, created_at) "
            "VALUES('wallet-b', 'ws', 'profile', 'B', 'descriptor', 'now')"
        )
        source = _row(
            "conversion-out",
            "wallet-a",
            "outbound",
            1_000,
            "2025-01-01T00:00:00Z",
        )
        target = _row(
            "conversion-in",
            "wallet-b",
            "inbound",
            900,
            "2025-01-01T00:01:00Z",
            config={"chain": "liquid", "network": "liquidv1"},
        )
        target["asset"] = "LBTC"
        self._insert_transaction(source)
        self._insert_transaction(target)
        component = {
            "id": "conversion-component",
            "component_type": "swap",
            "conservation_mode": "conversion",
            "conversion_policy": "carrying-value",
            "conversion_reviewed": True,
            "effective_state": "active",
            "legs": [
                {
                    "id": "source-leg",
                    "role": "source",
                    "transaction_id": source["id"],
                    "amount_msat": 1_000,
                },
                {
                    "id": "target-leg",
                    "role": "destination",
                    "transaction_id": target["id"],
                    "amount_msat": 900,
                },
            ],
            "allocations": [
                {
                    "id": "conversion-allocation",
                    "source_leg_id": "source-leg",
                    "sink_leg_id": "target-leg",
                    "source_amount_msat": 1_000,
                    "sink_amount_msat": 900,
                }
            ],
            "economic_terms": [
                {
                    "term_kind": "transaction_pair",
                    "source_leg_id": "source-leg",
                    "target_leg_id": "target-leg",
                    "legacy_source_id": "reviewed-conversion",
                    "review_kind": "peg-in",
                    "tax_policy": "carrying-value",
                    "swap_fee_msat": 25,
                    "swap_fee_kind": "network",
                    "confidence_at_review": "manual",
                    "review_source": "migration",
                }
            ],
        }
        state = build_canonical_quantity_state(
            [source, target],
            effective_components=[component],
        )

        counts = replace_canonical_quantity_state(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            state=state,
            created_at="now",
        )

        self.assertEqual(counts["economic_relations"], 1)
        relation = self.conn.execute(
            "SELECT * FROM journal_custody_economic_relations"
        ).fetchone()
        self.assertEqual(relation["relation_kind"], "conversion")
        self.assertEqual(relation["source_transaction_id"], source["id"])
        self.assertEqual(relation["target_transaction_id"], target["id"])
        self.assertEqual(relation["source_amount_msat"], 1_000)
        self.assertEqual(relation["target_amount_msat"], 900)
        self.assertEqual(relation["basis_state"], "eligible")
        self.assertEqual(relation["component_id"], component["id"])
        self.assertEqual(relation["swap_fee_msat"], 25)
        self.assertEqual(relation["swap_fee_kind"], "network")
        self.assertEqual(relation["confidence_at_review"], "manual")
        self.assertEqual(relation["review_source"], "migration")

    def test_reviewed_move_metadata_is_stored_on_decision(self):
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, created_at) "
            "VALUES('wallet-b', 'ws', 'profile', 'B', 'descriptor', 'now')"
        )
        source = _row("move-out", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z")
        target = _row("move-in", "wallet-b", "inbound", 100, "2025-01-01T00:01:00Z")
        self._insert_transaction(source)
        self._insert_transaction(target)
        component = {
            "id": "move-component",
            "component_type": "coinjoin",
            "conservation_mode": "quantity",
            "effective_state": "active",
            "legs": [
                {
                    "id": "source-leg",
                    "role": "source",
                    "transaction_id": source["id"],
                    "amount_msat": 100,
                },
                {
                    "id": "target-leg",
                    "role": "destination",
                    "transaction_id": target["id"],
                    "amount_msat": 100,
                },
            ],
            "allocations": [
                {
                    "id": "move-allocation",
                    "source_leg_id": "source-leg",
                    "sink_leg_id": "target-leg",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
            "economic_terms": [
                {
                    "term_kind": "transaction_pair",
                    "source_leg_id": "source-leg",
                    "target_leg_id": "target-leg",
                    "legacy_source_id": "reviewed-move",
                    "review_kind": "coinjoin",
                    "tax_policy": "carrying-value",
                    "swap_fee_msat": 5,
                    "swap_fee_kind": "provider",
                }
            ],
        }
        self.conn.execute(
            """
            INSERT INTO custody_components(
                id, lineage_id, workspace_id, profile_id, revision,
                component_type, state, authored_source, notes, created_at
            ) VALUES(?, ?, 'ws', 'profile', 1, ?, 'active', ?, ?, 'now')
            """,
            ("move-component", "move-lineage", "coinjoin", "manual", "reviewed move"),
        )
        self.conn.executemany(
            """
            INSERT INTO custody_component_legs(
                id, component_id, workspace_id, profile_id, ordinal, role,
                rail, chain, network, asset, exposure, conservation_unit,
                amount_msat, transaction_id, anchor_transaction_id, wallet_id,
                created_at
            ) VALUES(?, 'move-component', 'ws', 'profile', ?, ?, 'bitcoin',
                     'bitcoin', 'main', 'BTC', 'bitcoin', 'msat', 100,
                     ?, ?, ?, 'now')
            """,
            [
                ("source-leg", 0, "source", source["id"], source["id"], "wallet-a"),
                ("target-leg", 1, "destination", target["id"], target["id"], "wallet-b"),
            ],
        )
        self.conn.execute(
            """
            INSERT INTO custody_component_economic_terms(
                id, component_id, workspace_id, profile_id, ordinal,
                source_leg_id, target_leg_id, term_kind, legacy_source_id,
                source_row_hash, review_kind, tax_policy, swap_fee_msat,
                swap_fee_kind, confidence_at_review, review_source, created_at
            ) VALUES(?, 'move-component', 'ws', 'profile', 0,
                     'source-leg', 'target-leg', 'transaction_pair', ?, ?, ?, ?,
                     ?, ?, ?, ?, 'now')
            """,
            (
                "move-term",
                "reviewed-move",
                "f" * 64,
                "coinjoin",
                "carrying-value",
                5,
                "provider",
                "manual",
                "manual",
            ),
        )
        state = build_canonical_quantity_state(
            [source, target], effective_components=[component]
        )

        replace_canonical_quantity_state(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            state=state,
            created_at="now",
        )

        decision = self.conn.execute(
            "SELECT review_kind, policy, swap_fee_msat, swap_fee_kind "
            "FROM journal_custody_decisions"
        ).fetchone()
        self.assertEqual(decision["review_kind"], "coinjoin")
        self.assertEqual(decision["policy"], "carrying-value")
        self.assertEqual(decision["swap_fee_msat"], 5)
        self.assertEqual(decision["swap_fee_kind"], "provider")

    def test_missing_blocker_table_fails_closed(self):
        self.conn.execute("DROP TABLE journal_quantity_issues")

        with self.assertRaises(AppError) as unavailable:
            blocking_quantity_issues(self.conn, "profile")

        self.assertEqual(
            unavailable.exception.code,
            "custody_quantity_state_unavailable",
        )
        self.assertEqual(
            unavailable.exception.details["operation"],
            "read_journal_quantity_issues",
        )

    def test_finalized_internal_decisions_are_durable_and_redacted(self):
        self.conn.executemany(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, created_at
            ) VALUES(?, 'ws', 'profile', ?, 'descriptor', 'now')
            """,
            (("wallet-b", "Hot"), ("wallet-c", "Vault")),
        )
        rows = [
            _row(
                "source", "wallet-a", "outbound", 100,
                "2025-01-01T00:00:00Z",
            ),
            _row(
                "target-reviewed", "wallet-b", "inbound", 40,
                "2025-01-02T00:00:00Z",
            ),
            _row(
                "target-verified", "wallet-c", "inbound", 60,
                "2025-01-03T00:00:00Z",
            ),
        ]
        for row in rows:
            self._insert_transaction(row)
        preliminary = build_canonical_quantity_state(rows)
        observations = {
            item.transaction_id: item
            for item in preliminary.canonical_input.observations
        }
        claims = [
            QuantityClaim(
                claim_id="reviewed-edge",
                source=QuantitySlice(observations["source"].quantity_hash, 0, 40),
                target=QuantitySlice(
                    observations["target-reviewed"].quantity_hash, 0, 40
                ),
                state=INTERNAL_REVIEWED,
                priority=ClaimPriority.REVIEWED_PAIR,
                reason="reviewed_gap_bridge",
                atomic_bundle_id="bridge:reviewed",
                component_id="component-reviewed",
            ),
            QuantityClaim(
                claim_id="verified-edge",
                source=QuantitySlice(
                    observations["source"].quantity_hash, 40, 100
                ),
                target=QuantitySlice(
                    observations["target-verified"].quantity_hash, 0, 60
                ),
                state=INTERNAL_VERIFIED,
                priority=ClaimPriority.EXACT_NATIVE_EVENT,
                reason="verified_native_transfer",
                atomic_bundle_id="native:verified",
            ),
        ]
        state = build_canonical_quantity_state(rows, interpreter_claims=claims)

        counts = replace_canonical_quantity_state(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            state=state,
            created_at="rebuilt",
        )

        self.assertEqual(counts["decisions"], 2)
        private_rows = self.conn.execute(
            """
            SELECT source_observation_hash, source_start_msat, source_end_msat,
                   target_observation_hash, target_start_msat, target_end_msat
            FROM journal_custody_decisions
            ORDER BY state
            """
        ).fetchall()
        self.assertEqual(len(private_rows), 2)
        self.assertTrue(
            all(len(row["source_observation_hash"]) == 64 for row in private_rows)
        )
        self.assertEqual(
            {
                row["source_end_msat"] - row["source_start_msat"]
                for row in private_rows
            },
            {40, 60},
        )

        summary = custody_decision_rows(self.conn, "profile", limit=1)
        self.assertEqual(summary["count"], 2)
        self.assertEqual(summary["returned"], 1)
        self.assertTrue(summary["truncated"])
        self.assertIsNotNone(summary["next_cursor"])
        self.assertFalse(summary["observation_commitments_included"])
        record = summary["records"][0]
        self.assertEqual(record["source_transaction_id"], "source")
        self.assertIn(record["custody_state"], {INTERNAL_REVIEWED, INTERNAL_VERIFIED})
        self.assertEqual(record["source_network"], "main")
        self.assertEqual(record["target_network"], "main")
        self.assertEqual(record["source_rail"], "bitcoin")
        self.assertEqual(record["target_rail"], "bitcoin")
        self.assertNotIn("state", record)
        self.assertFalse(any("hash" in key or "slice" in key for key in record))

        older = custody_decision_rows(
            self.conn,
            "profile",
            limit=1,
            cursor=summary["next_cursor"],
        )
        self.assertEqual(older["count"], 2)
        self.assertEqual(older["returned"], 1)
        self.assertFalse(older["truncated"])
        self.assertIsNone(older["next_cursor"])
        self.assertNotEqual(
            older["records"][0]["target_transaction_id"],
            record["target_transaction_id"],
        )

        with self.assertRaises(AppError) as mismatched_cursor:
            custody_decision_rows(
                self.conn,
                "profile",
                transaction_ids=["target-reviewed"],
                cursor=summary["next_cursor"],
            )
        self.assertEqual(mismatched_cursor.exception.code, "validation")

        with self.assertRaises(AppError) as mismatched_profile:
            custody_decision_rows(
                self.conn,
                "another-profile",
                cursor=summary["next_cursor"],
            )
        self.assertEqual(mismatched_profile.exception.code, "validation")

        reviewed = custody_decision_rows(
            self.conn,
            "profile",
            transaction_ids=["target-reviewed"],
        )
        self.assertEqual(reviewed["count"], 1)
        self.assertEqual(reviewed["records"][0]["target_wallet_label"], "Hot")
        self.assertEqual(reviewed["records"][0]["amount_msat"], 40)
        self.assertEqual(
            reviewed["records"][0]["custody_state"], INTERNAL_REVIEWED
        )
        self.assertEqual(
            reviewed["records"][0]["component_id"], "component-reviewed"
        )

        self.conn.execute(
            "UPDATE journal_custody_decisions SET occurred_at = NULL "
            "WHERE component_id = 'component-reviewed'"
        )
        page_before_null = custody_decision_rows(self.conn, "profile", limit=1)
        null_timestamp_page = custody_decision_rows(
            self.conn,
            "profile",
            limit=1,
            cursor=page_before_null["next_cursor"],
        )
        self.assertIsNone(null_timestamp_page["records"][0]["occurred_at"])
        self.assertIsNone(null_timestamp_page["next_cursor"])

        profile_plan = self.conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT d.decision_id, source_wallet.label, target_wallet.label
            FROM journal_custody_decisions d
            LEFT JOIN wallets source_wallet
              ON source_wallet.id = d.source_wallet_id
            LEFT JOIN wallets target_wallet
              ON target_wallet.id = d.target_wallet_id
            WHERE d.profile_id = ?
            ORDER BY d.occurred_at DESC, d.decision_id DESC
            LIMIT ?
            """,
            ("profile", 100),
        ).fetchall()
        profile_plan_text = " ".join(str(row["detail"]) for row in profile_plan)
        self.assertIn(
            "idx_journal_custody_decisions_profile_time", profile_plan_text
        )
        self.assertNotIn("TEMP B-TREE", profile_plan_text)

        next_page_plan = self.conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT d.decision_id, source_wallet.label, target_wallet.label
            FROM journal_custody_decisions d
            LEFT JOIN wallets source_wallet
              ON source_wallet.id = d.source_wallet_id
            LEFT JOIN wallets target_wallet
              ON target_wallet.id = d.target_wallet_id
            WHERE d.profile_id = ?
              AND (d.occurred_at < ? OR d.occurred_at IS NULL
                   OR (d.occurred_at = ? AND d.decision_id < ?))
            ORDER BY d.occurred_at DESC, d.decision_id DESC
            LIMIT ?
            """,
            (
                "profile",
                "2025-01-01T00:00:00Z",
                "2025-01-01T00:00:00Z",
                "f" * 64,
                100,
            ),
        ).fetchall()
        next_page_plan_text = " ".join(
            str(row["detail"]) for row in next_page_plan
        )
        self.assertIn(
            "idx_journal_custody_decisions_profile_time", next_page_plan_text
        )
        self.assertNotIn("TEMP B-TREE", next_page_plan_text)

        decision_indexes = {
            row["name"]
            for row in self.conn.execute(
                "PRAGMA index_list('journal_custody_decisions')"
            ).fetchall()
        }
        self.assertIn("idx_journal_custody_decisions_source", decision_indexes)
        self.assertIn("idx_journal_custody_decisions_target", decision_indexes)

        fallback = build_canonical_quantity_state([rows[0]])
        replacement_counts = replace_canonical_quantity_state(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            state=fallback,
            created_at="replaced",
        )
        self.assertEqual(replacement_counts["decisions"], 0)
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM journal_custody_decisions"
            ).fetchone()[0],
            0,
        )
        self.conn.execute(
            """
            CREATE TRIGGER reject_test_custody_decision
            BEFORE INSERT ON journal_custody_decisions
            BEGIN
                SELECT RAISE(ABORT, 'test_decision_rejected');
            END
            """
        )
        with self.assertRaisesRegex(sqlite3.IntegrityError, "test_decision_rejected"):
            replace_canonical_quantity_state(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                state=state,
                created_at="must-roll-back",
            )
        self.assertEqual(
            {
                row["created_at"]
                for row in self.conn.execute(
                    "SELECT created_at FROM journal_quantity_postings"
                ).fetchall()
            },
            {"replaced"},
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM journal_custody_decisions"
            ).fetchone()[0],
            0,
        )

    def test_readiness_keeps_assets_separate_and_counts_unquantified_issues(self):
        rows = [
            (
                "btc-gap",
                "unresolved_quantity",
                "custody_suspense",
                "BTC",
                100,
            ),
            (
                "lbtc-gap",
                "claim_conflict",
                "conflicting",
                "LBTC",
                30,
            ),
            (
                "unknown-gap",
                "canonical_event_rejected",
                "conflicting",
                None,
                None,
            ),
        ]
        self.conn.executemany(
            """
            INSERT INTO journal_quantity_issues(
                issue_id, workspace_id, profile_id, issue_type, state, asset,
                amount_msat, transaction_ids_json, reason, detail_json,
                blocks_from, created_at
            ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, '[]', 'test', '{}',
                     '2025-01-01T00:00:00Z', 'now')
            """,
            rows,
        )

        summary = custody_quantity_readiness_summary(
            self.conn,
            "profile",
            journal_status="current",
        )

        self.assertEqual(summary["status"], "known_custody_gaps")
        self.assertEqual(summary["quantified_issue_count"], 2)
        self.assertEqual(summary["unquantified_issue_count"], 1)
        self.assertEqual(
            summary["unresolved_by_asset"],
            [
                {"asset": "BTC", "amount_msat": 100, "issue_count": 1},
                {"asset": "LBTC", "amount_msat": 30, "issue_count": 1},
            ],
        )
    def test_authored_evidence_is_explicit_and_immutable(self):
        row = _row("payment", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z")
        snapshot = EvidenceSnapshot.from_transaction(row)
        inserted = persist_authored_evidence_snapshots(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            subject_kind="custody_component",
            subject_id="component-1",
            snapshots=[snapshot],
            created_at="now",
        )
        self.assertEqual(inserted, 1)
        self.assertEqual(
            persist_authored_evidence_snapshots(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                subject_kind="custody_component",
                subject_id="component-1",
                snapshots=[snapshot],
                created_at="later",
            ),
            0,
        )
        with self.assertRaisesRegex(sqlite3.IntegrityError, "immutable"):
            self.conn.execute(
                "UPDATE custody_authored_evidence_snapshots "
                "SET payload_json = '{}'"
            )

    def test_receiver_local_evidence_never_creates_author_baseline(self):
        source = _row(
            "legacy-out", "wallet-a", "outbound", 100,
            "2025-01-01T00:00:00Z",
        )
        target = _row(
            "legacy-in", "wallet-a", "inbound", 100,
            "2025-01-02T00:00:00Z",
        )
        self._insert_transaction(source)
        self._insert_transaction(target)
        component = {
            "id": "legacy-component",
            "workspace_id": "ws",
            "profile_id": "profile",
            "effective_state": "draft",
            "evidence_status": {"status": "commitment_header_missing"},
            "legs": [
                {"id": "source", "role": "source", "transaction_id": "legacy-out"},
                {"id": "target", "role": "destination", "transaction_id": "legacy-in"},
            ],
            "allocations": [
                {
                    "id": "all",
                    "source_leg_id": "source",
                    "sink_leg_id": "target",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
        }

        result = baseline_missing_component_evidence(
            self.conn, [component], created_at="migration"
        )
        state = build_canonical_quantity_state(
            [source, target],
            effective_components=[component],
        )
        repeated = baseline_missing_component_evidence(
            self.conn, [component], created_at="later"
        )

        self.assertEqual(result["baselined_component_ids"], [])
        self.assertEqual(result["blocked"][0]["reason"], "component_not_effective")
        self.assertTrue(state.report_blocked)
        self.assertIn(
            "custody_component_authored_active_invalid",
            {issue.reason for issue in state.issues},
        )
        self.assertEqual(repeated["existing_component_ids"], [])
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_authored_evidence_snapshots"
            ).fetchone()[0],
            0,
        )

    def test_changed_receiver_evidence_still_does_not_create_author_baseline(self):
        source = _row(
            "replicated-out", "wallet-a", "outbound", 100,
            "2025-01-01T00:00:00Z",
        )
        target = _row(
            "replicated-in", "wallet-a", "inbound", 100,
            "2025-01-02T00:00:00Z",
        )
        self._insert_transaction(source)
        self._insert_transaction(target)
        component = {
            "id": "replicated-component",
            "workspace_id": "ws",
            "profile_id": "profile",
            "effective_state": "draft",
            "evidence_status": {"status": "commitment_header_missing"},
            "legs": [
                {"id": "source", "role": "source", "transaction_id": "replicated-out"},
                {"id": "target", "role": "destination", "transaction_id": "replicated-in"},
            ],
            "allocations": [
                {
                    "id": "all",
                    "source_leg_id": "source",
                    "sink_leg_id": "target",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
        }
        baseline_missing_component_evidence(
            self.conn, [component], created_at="reconciliation"
        )
        original_snapshot_count = self.conn.execute(
            "SELECT COUNT(*) FROM custody_authored_evidence_snapshots"
        ).fetchone()[0]
        changed_source = dict(source)
        changed_source["raw_json"] = json.dumps(
            {"txid": "replicated-out", "ownership_graph_version": 2}
        )
        self.conn.execute(
            "UPDATE transactions SET raw_json = ? WHERE id = 'replicated-out'",
            (changed_source["raw_json"],),
        )

        repeated = baseline_missing_component_evidence(
            self.conn, [component], created_at="after-change"
        )
        state = build_canonical_quantity_state(
            [changed_source, target],
            effective_components=[component],
        )

        self.assertEqual(repeated["existing_component_ids"], [])
        self.assertEqual(repeated["blocked"][0]["reason"], "component_not_effective")
        self.assertEqual(
            original_snapshot_count,
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_authored_evidence_snapshots"
            ).fetchone()[0],
        )
        self.assertTrue(state.report_blocked)
        self.assertIn(
            "custody_component_authored_active_invalid",
            {issue.reason for issue in state.issues},
        )

    def test_partial_replay_stays_unbaselined_until_component_is_effective(self):
        source = _row(
            "partial-out", "wallet-a", "outbound", 100,
            "2025-01-01T00:00:00Z",
        )
        target = _row(
            "partial-in", "wallet-a", "inbound", 100,
            "2025-01-02T00:00:00Z",
        )
        self._insert_transaction(source)
        incomplete = {
            "id": "partial-component",
            "workspace_id": "ws",
            "profile_id": "profile",
            "effective_state": "draft",
            "legs": [
                {"id": "source", "role": "source", "transaction_id": "partial-out"},
                {"id": "target", "role": "destination", "transaction_id": "partial-in"},
            ],
            "allocations": [],
        }

        blocked = baseline_missing_component_evidence(
            self.conn, [incomplete], created_at="partial-replay"
        )
        self.assertEqual(blocked["blocked"][0]["reason"], "component_not_effective")
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_authored_evidence_snapshots"
            ).fetchone()[0],
            0,
        )

        self._insert_transaction(target)
        complete = {
            **incomplete,
            "effective_state": "draft",
            "evidence_status": {"status": "commitment_header_missing"},
            "allocations": [
                {
                    "id": "all",
                    "source_leg_id": "source",
                    "sink_leg_id": "target",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                }
            ],
        }
        result = baseline_missing_component_evidence(
            self.conn, [complete], created_at="dependencies-arrived"
        )
        state = build_canonical_quantity_state(
            [source, target],
            effective_components=[complete],
        )

        self.assertEqual(result["baselined_component_ids"], [])
        self.assertEqual(result["blocked"][0]["reason"], "component_not_effective")
        self.assertTrue(state.report_blocked)

    def test_component_evidence_capture_is_not_rewritten_by_journal_refresh(self):
        row = _row("payment", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z")
        self._insert_transaction(row)
        self.conn.execute(
            """
            INSERT INTO custody_components(
                id, lineage_id, workspace_id, profile_id, revision,
                component_type, state, expected_leg_count,
                expected_allocation_count, created_at
            ) VALUES('component-1', 'component-1', 'ws', 'profile', 1,
                     'native_transfer', 'draft', 1, 0, 'now')
            """
        )
        component = {
            "id": "component-1",
            "workspace_id": "ws",
            "profile_id": "profile",
            "legs": [{"transaction_id": "payment"}],
        }
        self.assertEqual(
            capture_component_evidence(self.conn, component, created_at="activation"),
            1,
        )
        captured = self.conn.execute(
            "SELECT detail_hash, payload_json FROM custody_authored_evidence_snapshots"
        ).fetchone()
        self.conn.execute(
            "UPDATE transactions SET raw_json = ? WHERE id = 'payment'",
            (json.dumps({"txid": "payment", "ownership_graph_version": 1}),),
        )
        updated_row = dict(row)
        updated_row["raw_json"] = json.dumps(
            {"txid": "payment", "ownership_graph_version": 1}
        )
        state = build_canonical_quantity_state([updated_row])
        replace_canonical_quantity_state(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            state=state,
            created_at="refresh",
        )
        after = self.conn.execute(
            "SELECT detail_hash, payload_json FROM custody_authored_evidence_snapshots"
        ).fetchall()
        self.assertEqual(len(after), 1)
        self.assertEqual(tuple(after[0]), tuple(captured))

    def test_quantity_issue_blocks_reports_and_appears_in_blocker_snapshot(self):
        row = _row("gap", "wallet-a", "outbound", 100, "2025-01-01T00:00:00Z")
        self._insert_transaction(row)
        baseline = build_canonical_quantity_state([row])
        observation = baseline.projection.observations[0]
        suspense = QuantityClaim(
            claim_id="gap-suspense",
            source=QuantitySlice(observation.quantity_hash, 0, 100),
            state=CUSTODY_SUSPENSE,
            priority=ClaimPriority.ACCOUNTING_CONVENTION,
            reason="missing_wallet",
        )
        state = build_canonical_quantity_state([row], interpreter_claims=[suspense])
        replace_canonical_quantity_state(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            state=state,
            created_at="now",
        )
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES('context_workspace', 'ws')"
        )
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES('context_profile', 'profile')"
        )
        self.conn.execute(
            "UPDATE profiles SET last_processed_at = 'now', "
            "last_processed_tx_count = 1 WHERE id = 'profile'"
        )

        snapshot = build_report_blockers_snapshot(self.conn)
        blocker = next(
            item
            for item in snapshot["blockers"]
            if item["id"] == "custody_quantity_unresolved"
        )
        self.assertEqual(blocker["blocked_from"], "2025-01-01T00:00:00Z")
        self.assertEqual(blocker["unresolved_msat"], 100)
        self.assertEqual(blocker["states"], [CUSTODY_SUSPENSE])
        self.assertEqual(
            blocker["unresolved_by_asset"],
            [{"asset": "BTC", "amount_msat": 100, "issue_count": 1}],
        )
        self.assertEqual(
            snapshot["custody_quantity"]["status"], "known_custody_gaps"
        )
        self.assertEqual(
            snapshot["custody_quantity"]["status_text"],
            "Known custody gaps require review",
        )
        self.assertIn(
            "does not assert that every wallet was imported",
            snapshot["custody_quantity"]["qualification"],
        )

        from kassiber.cli.handlers import resolve_scope
        from kassiber.core.report_context import require_report_context

        profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()
        with self.assertRaises(AppError) as raised:
            require_report_context(self.conn, "ws", "profile", resolve_scope)
        self.assertEqual(raised.exception.code, "custody_quantity_unresolved")


if __name__ == "__main__":
    unittest.main()
