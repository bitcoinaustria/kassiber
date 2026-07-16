import random
import unittest

from kassiber.core.custody_allocations import (
    CustodyAllocationError,
    allocate_msat_fifo,
)
from kassiber.core.custody_evidence import (
    assess_authoritative_chain_observation,
    normalize_boundary_amounts,
)
from kassiber.core.chain_observer.provenance import (
    canonical_graph_hash,
    canonical_observed_quantity_hash,
)

from kassiber.core.custody_quantity import (
    ArbitratedSlice,
    CONFLICTING,
    CUSTODY_CANDIDATE,
    CUSTODY_SUSPENSE,
    ClaimPriority,
    EvidenceSnapshot,
    EXTERNAL_PRESUMED,
    INTERNAL_REVIEWED,
    INTERNAL_VERIFIED,
    QuantityClaim,
    QuantityObservation,
    QuantitySlice,
    _claim_fully_selected,
    _fail_closed_atomic_bundles,
    _selected_claim_totals,
    build_canonical_quantity_input,
    observation_hash,
    project_quantities,
)


_TXID_A = "ab" * 32
_TXID_B = "cd" * 32
_TXID_C = "ef" * 32
_TXID_D = "12" * 32


def _row(
    tx_id,
    wallet_id,
    direction,
    amount_msat,
    *,
    fee_msat=0,
    raw_json=None,
    chain="bitcoin",
    network="main",
):
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "fingerprint": f"fingerprint:{tx_id}",
        "external_id": f"native:{tx_id}",
        "occurred_at": "2026-01-01T00:00:00Z",
        "direction": direction,
        "asset": "BTC",
        "amount": amount_msat,
        "fee": fee_msat,
        "amount_includes_fee": 0,
        "chain": chain,
        "network": network,
        "raw_json": raw_json or {"txid": f"native:{tx_id}"},
    }


def _observation(*args, **kwargs):
    return QuantityObservation.from_transaction(_row(*args, **kwargs))


class CustodyQuantityTests(unittest.TestCase):
    def test_boundary_amount_normalization_covers_fee_conventions(self):
        separate = normalize_boundary_amounts(
            direction="outbound",
            amount_msat=100,
            fee_msat=5,
            amount_includes_fee=False,
        )
        included = normalize_boundary_amounts(
            direction="outbound",
            amount_msat=105,
            fee_msat=5,
            amount_includes_fee=True,
        )
        inbound = normalize_boundary_amounts(
            direction="inbound",
            amount_msat=100,
            fee_msat=5,
            amount_includes_fee=False,
        )

        self.assertEqual(separate.principal_msat, 100)
        self.assertEqual(included.principal_msat, 100)
        self.assertEqual(separate.wallet_movement_msat, 105)
        self.assertEqual(included.wallet_movement_msat, 105)
        self.assertEqual(separate.wallet_delta_msat, -105)
        self.assertEqual(included.wallet_delta_msat, -105)
        self.assertEqual(inbound.principal_msat, 100)
        self.assertEqual(inbound.wallet_movement_msat, 100)
        self.assertEqual(inbound.wallet_delta_msat, 100)

    def test_fifo_nm_allocator_is_exact_and_returns_residuals(self):
        result = allocate_msat_fifo(
            [("source-a", 60), ("source-b", 50)],
            [("sink-x", 30), ("sink-y", 70)],
            amount_msat=95,
        )

        self.assertEqual(
            [
                (
                    cell.source_id,
                    cell.sink_id,
                    cell.amount_msat,
                    cell.source_start_msat,
                    cell.sink_start_msat,
                )
                for cell in result.cells
            ],
            [
                ("source-a", "sink-x", 30, 0, 0),
                ("source-a", "sink-y", 30, 30, 0),
                ("source-b", "sink-y", 35, 0, 30),
            ],
        )
        self.assertEqual(result.allocated_msat, 95)
        self.assertEqual(
            result.source_remaining,
            (("source-a", 0), ("source-b", 15)),
        )
        self.assertEqual(
            result.sink_remaining,
            (("sink-x", 0), ("sink-y", 5)),
        )

    def test_fifo_nm_allocator_fails_before_returning_partial_result(self):
        with self.assertRaises(CustodyAllocationError) as raised:
            allocate_msat_fifo(
                [("source", 5)],
                [("sink", 6)],
                amount_msat=6,
            )
        self.assertEqual(
            raised.exception.code,
            "custody_allocation_insufficient_capacity",
        )

    def test_fifo_nm_allocator_skips_zero_capacity_boundaries(self):
        result = allocate_msat_fifo(
            [("empty-source", 0), ("source", 5)],
            [("empty-sink", 0), ("sink", 5)],
        )
        self.assertEqual(len(result.cells), 1)
        self.assertEqual(result.cells[0].source_id, "source")
        self.assertEqual(result.cells[0].sink_id, "sink")
        self.assertEqual(result.cells[0].amount_msat, 5)

    def test_atomic_bundle_selection_index_matches_legacy_semantics(self):
        source = QuantitySlice("source", 0, 100)
        target = QuantitySlice("target", 0, 100)
        targeted_claim = QuantityClaim(
            claim_id="targeted",
            source=source,
            target=target,
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="reviewed",
        )
        targetless_claim = QuantityClaim(
            claim_id="targetless",
            source=source,
            state=CUSTODY_SUSPENSE,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="reviewed_residual",
        )

        def legacy_fully_selected(claim, decisions):
            selected = [
                decision
                for decision in decisions
                if decision.selected_claim_id == claim.claim_id
            ]
            if (
                sum(item.source.amount_msat for item in selected)
                != claim.source.amount_msat
            ):
                return False
            if claim.target is None:
                return all(item.target is None for item in selected)
            return sum(
                item.target.amount_msat
                for item in selected
                if item.target is not None
            ) == claim.target.amount_msat

        cases = [
            [],
            [
                ArbitratedSlice(
                    source=source,
                    target=target,
                    state=INTERNAL_REVIEWED,
                    reason="reviewed",
                    selected_claim_id="targeted",
                )
            ],
            [
                ArbitratedSlice(
                    source=QuantitySlice("source", 0, 50),
                    target=QuantitySlice("target", 0, 50),
                    state=INTERNAL_REVIEWED,
                    reason="reviewed",
                    selected_claim_id="targeted",
                ),
                ArbitratedSlice(
                    source=QuantitySlice("source", 50, 100),
                    target=QuantitySlice("target", 50, 100),
                    state=INTERNAL_REVIEWED,
                    reason="reviewed",
                    selected_claim_id="targeted",
                ),
            ],
            [
                ArbitratedSlice(
                    source=QuantitySlice("source", 0, 50),
                    target=QuantitySlice("target", 0, 50),
                    state=INTERNAL_REVIEWED,
                    reason="reviewed",
                    selected_claim_id="targeted",
                )
            ],
            [
                ArbitratedSlice(
                    source=source,
                    state=CUSTODY_SUSPENSE,
                    reason="reviewed_residual",
                    selected_claim_id="targetless",
                )
            ],
            [
                ArbitratedSlice(
                    source=QuantitySlice("source", 0, 50),
                    state=CUSTODY_SUSPENSE,
                    reason="reviewed_residual",
                    selected_claim_id="targetless",
                ),
                ArbitratedSlice(
                    source=QuantitySlice("source", 50, 100),
                    state=CUSTODY_SUSPENSE,
                    reason="reviewed_residual",
                    selected_claim_id="targetless",
                ),
            ],
            [
                ArbitratedSlice(
                    source=source,
                    target=target,
                    state=INTERNAL_REVIEWED,
                    reason="unexpected_target",
                    selected_claim_id="targetless",
                )
            ],
        ]
        for claim in (targeted_claim, targetless_claim):
            for decisions in cases:
                with self.subTest(claim=claim.claim_id, decisions=decisions):
                    self.assertEqual(
                        _claim_fully_selected(
                            claim,
                            _selected_claim_totals(decisions),
                        ),
                        legacy_fully_selected(claim, decisions),
                    )

    def test_atomic_bundle_validation_scans_decisions_only_twice(self):
        claim_count = 2_000
        claims = []
        decisions = []
        for index in range(claim_count):
            source = QuantitySlice(f"source-{index}", 0, 1)
            target = QuantitySlice(f"target-{index}", 0, 1)
            claim_id = f"claim-{index}"
            claims.append(
                QuantityClaim(
                    claim_id=claim_id,
                    source=source,
                    target=target,
                    state=INTERNAL_REVIEWED,
                    priority=ClaimPriority.REVIEWED_COMPONENT,
                    reason="reviewed",
                )
            )
            decisions.append(
                ArbitratedSlice(
                    source=source,
                    target=target,
                    state=INTERNAL_REVIEWED,
                    reason="reviewed",
                    selected_claim_id=claim_id,
                )
            )

        class CountingDecisions:
            def __init__(self, items):
                self.items = items
                self.yield_count = 0

            def __iter__(self):
                for item in self.items:
                    self.yield_count += 1
                    yield item

            def __len__(self):
                return len(self.items)

        counted = CountingDecisions(decisions)
        result = _fail_closed_atomic_bundles(counted, claims)

        self.assertEqual(len(result), claim_count)
        self.assertEqual(counted.yield_count, claim_count * 2)

    def test_native_authority_requires_closed_persisted_hash_commitments(self):
        row = _row(
            "observed",
            "wallet-a",
            "outbound",
            10_000,
            raw_json={
                "txid": _TXID_A,
                # User-controlled/imported markers are deliberately inert.
                "observer": "bdk",
                "ownership_graph_version": 1,
            },
        )
        graph_hash = canonical_graph_hash(row["raw_json"])
        missing = assess_authoritative_chain_observation(
            row,
        )
        self.assertFalse(missing.authoritative)
        self.assertEqual(missing.reason, "provenance_missing")

        authoritative_row = {
            **row,
            "observation_authority_version": 1,
            "observation_quantity_hash": canonical_observed_quantity_hash(row),
            "observation_graph_hash": graph_hash,
        }
        matched = assess_authoritative_chain_observation(
            authoritative_row,
        )
        self.assertTrue(matched.authoritative)
        self.assertEqual(matched.reason, "matched")
        observation = QuantityObservation.from_transaction(
            {
                **authoritative_row,
                "observation_fee_attribution": "exact",
            }
        )
        self.assertTrue(observation.authoritative_chain_observation)
        self.assertEqual(observation.fee_attribution, "exact")
        imported = QuantityObservation.from_transaction(row)
        self.assertFalse(imported.authoritative_chain_observation)
        self.assertEqual(imported.fee_attribution, "unknown")

        changed = dict(authoritative_row)
        changed["amount"] = 9_999
        quantity_mismatch = assess_authoritative_chain_observation(
            changed,
        )
        self.assertFalse(quantity_mismatch.authoritative)
        self.assertEqual(quantity_mismatch.reason, "quantity_hash_mismatch")

        graph_mismatch = assess_authoritative_chain_observation(
            {**authoritative_row, "raw_json": {"txid": _TXID_A, "vin": []}},
        )
        self.assertFalse(graph_mismatch.authoritative)
        self.assertEqual(graph_mismatch.reason, "graph_hash_mismatch")

        unsupported = assess_authoritative_chain_observation(
            {**authoritative_row, "observation_authority_version": 0},
        )
        self.assertFalse(unsupported.authoritative)
        self.assertEqual(unsupported.reason, "authority_version_unsupported")

    def test_observation_hash_is_canonical_and_evidence_sensitive(self):
        left = _row(
            "out",
            "wallet-a",
            "outbound",
            10_000,
            raw_json={"vout": [{"n": 1, "value": 10}], "txid": "abc"},
        )
        right = dict(left)
        right["raw_json"] = '{"txid":"abc","vout":[{"value":10,"n":1}]}'
        self.assertEqual(observation_hash(left), observation_hash(right))

        changed = dict(right)
        changed["amount"] = 9_999
        self.assertNotEqual(observation_hash(left), observation_hash(changed))

        enriched = dict(right)
        enriched["raw_json"] = {"txid": "abc", "vout": [], "vin": []}
        self.assertEqual(observation_hash(left), observation_hash(enriched))
        self.assertNotEqual(
            EvidenceSnapshot.from_transaction(left).detail_hash,
            EvidenceSnapshot.from_transaction(enriched).detail_hash,
        )

    def test_canonical_input_deduplicates_repeats_and_keeps_distinct_wallet_legs(self):
        outbound = _row(
            "out",
            "wallet-a",
            "outbound",
            1_000,
            raw_json={"txid": _TXID_A.upper()},
        )
        duplicate = dict(outbound)
        duplicate["id"] = "out-duplicate"
        duplicate["fingerprint"] = "other-import-fingerprint"
        inbound = _row(
            "in",
            "wallet-b",
            "inbound",
            990,
            raw_json={"txid": _TXID_A},
        )

        canonical = build_canonical_quantity_input(
            [duplicate, inbound, outbound]
        )

        self.assertEqual(canonical.rejected_events, ())
        self.assertEqual(len(canonical.events), 1)
        self.assertEqual(canonical.events[0].event_key.native_event_id, _TXID_A)
        self.assertEqual(
            {
                (item.wallet_id, item.direction, item.amount_msat)
                for item in canonical.events[0].legs
            },
            {
                ("wallet-a", "outbound", 1_000),
                ("wallet-b", "inbound", 990),
            },
        )
        self.assertEqual(
            canonical.events[0].source_transaction_ids,
            ("in", "out", "out-duplicate"),
        )
        aliases = dict(canonical.events[0].observation_aliases)
        self.assertEqual(aliases["out"], aliases["out-duplicate"])
        self.assertNotEqual(aliases["out"], aliases["in"])

    def test_provider_ids_are_source_qualified(self):
        left = _row("left", "wallet-a", "inbound", 100)
        right = _row("right", "wallet-b", "inbound", 100)
        for row, source in ((left, "provider-a"), (right, "provider-b")):
            # A provider id may look like a txid. Without typed txid provenance
            # it must remain source-qualified rather than chain-global.
            row["external_id"] = _TXID_C
            row["raw_json"] = {}
            row["source_ref"] = source

        canonical = build_canonical_quantity_input([left, right])

        self.assertEqual(len(canonical.events), 2)
        self.assertEqual(
            {item.event_key.native_namespace for item in canonical.events},
            {"provider-a", "provider-b"},
        )

    def test_contradictory_aggregate_rejects_only_its_event(self):
        bad_one = _row(
            "bad-1",
            "wallet-a",
            "outbound",
            100,
            raw_json={"txid": _TXID_A},
        )
        bad_two = _row(
            "bad-2",
            "wallet-a",
            "outbound",
            101,
            raw_json={"txid": _TXID_A},
        )
        good = _row(
            "good",
            "wallet-b",
            "inbound",
            50,
            raw_json={"txid": _TXID_B},
        )

        canonical = build_canonical_quantity_input([bad_one, good, bad_two])

        self.assertEqual(len(canonical.events), 1)
        self.assertEqual(canonical.events[0].event_key.native_event_id, _TXID_B)
        self.assertEqual(len(canonical.rejected_events), 1)
        self.assertEqual(
            canonical.rejected_events[0].code,
            "canonical_event_leg_contradiction",
        )

    def test_invalid_fee_and_zero_inbound_are_event_local(self):
        fee_overrun = _row("overrun", "wallet-a", "outbound", 10, fee_msat=11)
        fee_overrun["amount_includes_fee"] = 1
        fee_overrun["raw_json"] = {"txid": _TXID_A}
        zero_inbound = _row("zero-in", "wallet-b", "inbound", 0)
        zero_inbound["raw_json"] = {"txid": _TXID_B}
        good = _row("good", "wallet-c", "inbound", 25)
        good["raw_json"] = {"txid": _TXID_C}

        canonical = build_canonical_quantity_input(
            [fee_overrun, zero_inbound, good]
        )

        self.assertEqual(len(canonical.events), 1)
        self.assertEqual(canonical.events[0].event_key.native_event_id, _TXID_C)
        self.assertEqual(
            [item.code for item in canonical.rejected_events],
            [
                "canonical_event_leg_invalid",
                "canonical_event_leg_invalid",
            ],
        )

    def test_inbound_fee_is_evidence_only_not_a_wallet_debit(self):
        inbound = _row("fee-in", "wallet-a", "inbound", 20, fee_msat=1)
        inbound["raw_json"] = {"txid": _TXID_D}
        canonical = build_canonical_quantity_input([inbound])
        self.assertEqual(canonical.rejected_events, ())
        projection = project_quantities(canonical.observations, [])
        self.assertEqual(
            {
                item.location_kind: item.amount_msat
                for item in projection.postings
            },
            {"wallet": 20, "external_origin": -20},
        )

    def test_fee_only_outbound_is_canonical_and_balanced(self):
        fee_only = _row("fee-only", "wallet-a", "outbound", 0, fee_msat=25)
        canonical = build_canonical_quantity_input([fee_only])
        self.assertEqual(len(canonical.events), 1)
        projection = project_quantities(canonical.observations, [])
        self.assertEqual(projection.decisions, ())
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})
        self.assertEqual(
            {
                item.location_kind: item.amount_msat
                for item in projection.postings
            },
            {"wallet": -25, "fee": 25},
        )

    def test_state_priority_matrix_rejects_semantic_mismatches(self):
        source = _observation("out", "wallet-a", "outbound", 100)
        target = _observation("in", "wallet-b", "inbound", 100)
        with self.assertRaisesRegex(ValueError, "cannot use reviewed_component"):
            QuantityClaim(
                claim_id="bad-candidate",
                source=QuantitySlice(source.quantity_hash, 0, 100),
                target=QuantitySlice(target.quantity_hash, 0, 100),
                state=CUSTODY_CANDIDATE,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="invalid",
            )
        reviewed = QuantityClaim(
            claim_id="reviewed-single",
            source=QuantitySlice(source.quantity_hash, 0, 100),
            target=QuantitySlice(target.quantity_hash, 0, 100),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="reviewed",
        )
        self.assertEqual(
            reviewed.effective_bundle_id,
            "single:reviewed-single",
        )

    def test_explicit_suspense_claim_is_selected(self):
        source = _observation("out", "wallet-a", "outbound", 100)
        projection = project_quantities(
            [source],
            [
                QuantityClaim(
                    claim_id="explicit-suspense",
                    source=QuantitySlice(source.quantity_hash, 0, 100),
                    state=CUSTODY_SUSPENSE,
                    priority=ClaimPriority.ACCOUNTING_CONVENTION,
                    reason="known_missing_destination",
                )
            ],
        )
        self.assertEqual(projection.decisions[0].state, CUSTODY_SUSPENSE)
        self.assertEqual(
            projection.decisions[0].selected_claim_id,
            "explicit-suspense",
        )
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_malformed_bundle_suspends_its_source_without_aborting_other_wallets(self):
        bad_source = _observation("bad-out", "wallet-a", "outbound", 100)
        good_source = _observation("good-out", "wallet-b", "outbound", 200)
        good_target = _observation("good-in", "wallet-c", "inbound", 200)
        bad_claim = QuantityClaim(
            claim_id="bad-missing-target",
            source=QuantitySlice(bad_source.quantity_hash, 0, 100),
            target=QuantitySlice("missing-observation", 0, 100),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="malformed_component",
            atomic_bundle_id="component:bad",
        )
        good_claim = QuantityClaim(
            claim_id="good-move",
            source=QuantitySlice(good_source.quantity_hash, 0, 200),
            target=QuantitySlice(good_target.quantity_hash, 0, 200),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="reviewed_component",
            atomic_bundle_id="component:good",
        )

        projection = project_quantities(
            [bad_source, good_source, good_target],
            [bad_claim, good_claim],
        )
        observations = {
            item.quantity_hash: item for item in projection.observations
        }
        decisions = {
            observations[item.source.observation_hash].transaction_id: item
            for item in projection.decisions
        }

        self.assertEqual(decisions["bad-out"].state, CUSTODY_SUSPENSE)
        self.assertEqual(decisions["bad-out"].reason, "malformed_claim_bundle")
        self.assertEqual(decisions["good-out"].state, INTERNAL_REVIEWED)
        self.assertEqual(
            [(item.bundle_id, item.reasons) for item in projection.claim_errors],
            [("component:bad", ("claim_target_invalid",))],
        )
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_reviewed_target_and_residual_suspense_activate_as_one_bundle(self):
        source = _observation("out", "wallet-a", "outbound", 1_000)
        target = _observation("in", "wallet-b", "inbound", 900)
        bundle = [
            QuantityClaim(
                claim_id="retained",
                source=QuantitySlice(source.quantity_hash, 0, 900),
                target=QuantitySlice(target.quantity_hash, 0, 900),
                state=INTERNAL_REVIEWED,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="approved_bridge",
                atomic_bundle_id="component:bridge",
            ),
            QuantityClaim(
                claim_id="residual",
                source=QuantitySlice(source.quantity_hash, 900, 1_000),
                state=CUSTODY_SUSPENSE,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="reviewed_residual_suspense",
                atomic_bundle_id="component:bridge",
            ),
        ]
        projection = project_quantities([source, target], bundle)
        self.assertEqual(
            [(item.state, item.source.amount_msat) for item in projection.decisions],
            [(INTERNAL_REVIEWED, 900), (CUSTODY_SUSPENSE, 100)],
        )
        self.assertEqual(projection.unresolved_msat_by_asset(), {"BTC": 100})
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_split_selected_claim_maps_target_offsets_exactly(self):
        source = _observation("out", "wallet-a", "outbound", 1_000)
        candidate_target = _observation("candidate", "wallet-b", "inbound", 1_200)
        exact_target = _observation("exact", "wallet-c", "inbound", 200)
        candidate = QuantityClaim(
            claim_id="candidate",
            source=QuantitySlice(source.quantity_hash, 0, 1_000),
            target=QuantitySlice(candidate_target.quantity_hash, 100, 1_100),
            state=CUSTODY_CANDIDATE,
            priority=ClaimPriority.HEURISTIC_CANDIDATE,
            reason="candidate",
        )
        exact = QuantityClaim(
            claim_id="exact",
            source=QuantitySlice(source.quantity_hash, 400, 600),
            target=QuantitySlice(exact_target.quantity_hash, 0, 200),
            state=INTERNAL_VERIFIED,
            priority=ClaimPriority.EXACT_NATIVE_EVENT,
            reason="exact",
        )
        projection = project_quantities(
            [source, candidate_target, exact_target],
            [candidate, exact],
        )
        candidate_parts = [
            item for item in projection.decisions if item.selected_claim_id == "candidate"
        ]
        self.assertEqual(
            [
                (item.source.start_msat, item.source.end_msat,
                 item.target.start_msat, item.target.end_msat)
                for item in candidate_parts
            ],
            [(0, 400, 100, 500), (600, 1_000, 700, 1_100)],
        )

    def test_cross_network_claim_isolated_as_source_suspense(self):
        source = _observation(
            "out", "wallet-a", "outbound", 100, network="main"
        )
        target = _observation(
            "in", "wallet-b", "inbound", 100, network="test"
        )
        claim = QuantityClaim(
            claim_id="cross-network",
            source=QuantitySlice(source.quantity_hash, 0, 100),
            target=QuantitySlice(target.quantity_hash, 0, 100),
            state=INTERNAL_VERIFIED,
            priority=ClaimPriority.EXACT_NATIVE_EVENT,
            reason="invalid_scope",
        )
        projection = project_quantities([source, target], [claim])

        self.assertEqual(projection.decisions[0].state, CUSTODY_SUSPENSE)
        self.assertEqual(projection.decisions[0].reason, "malformed_claim_bundle")
        self.assertEqual(
            projection.claim_errors[0].reasons,
            ("claim_domain_incompatible",),
        )

    def test_unclaimed_inbound_has_an_external_origin_counterposting(self):
        inbound = _observation("in", "wallet-a", "inbound", 500)
        projection = project_quantities([inbound], [])
        self.assertEqual(
            {
                item.location_kind: item.amount_msat
                for item in projection.postings
            },
            {"wallet": 500, "external_origin": -500},
        )
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_transitive_destination_overlap_fails_closed_as_one_cluster(self):
        sources = [
            _observation(f"out-{index}", f"wallet-{index}", "outbound", 10)
            for index in range(3)
        ]
        target = _observation("in", "wallet-target", "inbound", 20)
        target_ranges = [(0, 10), (5, 15), (10, 20)]
        claims = [
            QuantityClaim(
                claim_id=f"claim-{index}",
                source=QuantitySlice(source.quantity_hash, 0, 10),
                target=QuantitySlice(target.quantity_hash, start, end),
                state=INTERNAL_REVIEWED,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="transitive_overlap",
            )
            for index, (source, (start, end)) in enumerate(
                zip(sources, target_ranges)
            )
        ]
        projection = project_quantities([*sources, target], claims)
        self.assertTrue(all(item.state == CONFLICTING for item in projection.decisions))
        self.assertTrue(
            all(
                item.contender_claim_ids == ("claim-0", "claim-1", "claim-2")
                for item in projection.decisions
            )
        )
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_flagship_candidate_preserves_wallets_and_only_residual_is_suspense(self):
        source = _observation("out", "multisig-b", "outbound", 10_000)
        return_one = _observation("in-1", "operative-c", "inbound", 6_000)
        return_two = _observation("in-2", "operative-c", "inbound", 3_900)
        claims = [
            QuantityClaim(
                claim_id="fallback-disposal",
                source=QuantitySlice(source.quantity_hash, 0, 10_000),
                state=EXTERNAL_PRESUMED,
                priority=ClaimPriority.PRESUMED_EXTERNAL_FALLBACK,
                reason="unmatched_outflow",
                fallback=True,
            ),
            QuantityClaim(
                claim_id="candidate-1",
                source=QuantitySlice(source.quantity_hash, 0, 6_000),
                target=QuantitySlice(return_one.quantity_hash, 0, 6_000),
                state=CUSTODY_CANDIDATE,
                priority=ClaimPriority.HEURISTIC_CANDIDATE,
                reason="plausible_missing_wallet_bridge",
            ),
            QuantityClaim(
                claim_id="candidate-2",
                source=QuantitySlice(source.quantity_hash, 6_000, 9_900),
                target=QuantitySlice(return_two.quantity_hash, 0, 3_900),
                state=CUSTODY_CANDIDATE,
                priority=ClaimPriority.HEURISTIC_CANDIDATE,
                reason="plausible_missing_wallet_bridge",
            ),
        ]

        projection = project_quantities(
            [source, return_one, return_two],
            claims,
        )

        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})
        self.assertEqual(projection.unresolved_msat_by_asset(), {"BTC": 10_000})
        self.assertEqual(
            sum(
                item.source.amount_msat
                for item in projection.decisions
                if item.state == CUSTODY_SUSPENSE
            ),
            100,
        )
        self.assertFalse(
            any(item.state == EXTERNAL_PRESUMED for item in projection.decisions)
        )
        self.assertFalse(any(item.finalized for item in projection.decisions))
        wallet_postings = {
            item.location_id: item.amount_msat
            for item in projection.postings
            if item.location_kind == "wallet"
        }
        self.assertEqual(wallet_postings["multisig-b"], -10_000)
        self.assertEqual(
            sum(
                item.amount_msat
                for item in projection.postings
                if item.location_kind == "wallet"
                and item.location_id == "operative-c"
            ),
            9_900,
        )
        self.assertEqual(
            sum(
                item.amount_msat
                for item in projection.postings
                if item.location_kind == "custody_suspense"
            ),
            100,
        )

    def test_network_fee_is_separate_and_conserved(self):
        source = _observation(
            "out", "wallet-a", "outbound", 100_000, fee_msat=2_000
        )
        projection = project_quantities(
            [source],
            [
                QuantityClaim(
                    claim_id="presumed",
                    source=QuantitySlice(source.quantity_hash, 0, 100_000),
                    state=EXTERNAL_PRESUMED,
                    priority=ClaimPriority.PRESUMED_EXTERNAL_FALLBACK,
                    reason="ordinary_unmatched_outflow",
                    fallback=True,
                )
            ],
        )
        by_kind = {
            item.location_kind: item.amount_msat for item in projection.postings
        }
        self.assertEqual(by_kind["wallet"], -102_000)
        self.assertEqual(by_kind["external"], 100_000)
        self.assertEqual(by_kind["fee"], 2_000)
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_fee_inclusive_observation_does_not_double_debit_the_fee(self):
        row = _row("out", "wallet-a", "outbound", 100_000, fee_msat=2_000)
        row["amount_includes_fee"] = 1
        source = QuantityObservation.from_transaction(row)
        projection = project_quantities(
            [source],
            [
                QuantityClaim(
                    claim_id="presumed",
                    source=QuantitySlice(source.quantity_hash, 0, 98_000),
                    state=EXTERNAL_PRESUMED,
                    priority=ClaimPriority.PRESUMED_EXTERNAL_FALLBACK,
                    reason="ordinary_unmatched_outflow",
                    fallback=True,
                )
            ],
        )
        by_kind = {
            item.location_kind: item.amount_msat for item in projection.postings
        }
        self.assertEqual(by_kind["wallet"], -100_000)
        self.assertEqual(by_kind["external"], 98_000)
        self.assertEqual(by_kind["fee"], 2_000)
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_stronger_claim_wins_and_equal_priority_overlap_conflicts(self):
        source = _observation("out", "wallet-a", "outbound", 1_000)
        target = _observation("in", "wallet-b", "inbound", 1_000)
        reviewed = QuantityClaim(
            claim_id="reviewed",
            source=QuantitySlice(source.quantity_hash, 0, 1_000),
            target=QuantitySlice(target.quantity_hash, 0, 1_000),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="approved_bridge",
        )
        candidate = QuantityClaim(
            claim_id="candidate",
            source=QuantitySlice(source.quantity_hash, 0, 1_000),
            target=QuantitySlice(target.quantity_hash, 0, 1_000),
            state=CUSTODY_CANDIDATE,
            priority=ClaimPriority.HEURISTIC_CANDIDATE,
            reason="heuristic",
        )
        projection = project_quantities([source, target], [candidate, reviewed])
        self.assertEqual(projection.decisions[0].selected_claim_id, "reviewed")
        self.assertEqual(projection.decisions[0].state, INTERNAL_REVIEWED)

        conflict = QuantityClaim(
            claim_id="reviewed-again",
            source=reviewed.source,
            target=reviewed.target,
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="second_active_revision",
        )
        projection = project_quantities([source, target], [reviewed, conflict])
        self.assertEqual(projection.decisions[0].state, CONFLICTING)
        self.assertEqual(
            projection.decisions[0].contender_claim_ids,
            ("reviewed", "reviewed-again"),
        )
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_two_sources_cannot_consume_one_destination_slice(self):
        source_one = _observation("out-1", "wallet-a", "outbound", 1_000)
        source_two = _observation("out-2", "wallet-b", "outbound", 1_000)
        target = _observation("in", "wallet-c", "inbound", 1_000)
        claims = [
            QuantityClaim(
                claim_id=f"claim-{index}",
                source=QuantitySlice(source.quantity_hash, 0, 1_000),
                target=QuantitySlice(target.quantity_hash, 0, 1_000),
                state=INTERNAL_REVIEWED,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="bad_overlap",
            )
            for index, source in enumerate((source_one, source_two), start=1)
        ]
        projection = project_quantities(
            [source_one, source_two, target],
            claims,
        )
        self.assertEqual(
            [item.state for item in projection.decisions],
            [CONFLICTING, CONFLICTING],
        )
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_one_destination_collision_invalidates_an_atomic_nm_bundle(self):
        source_one = _observation("out-a", "wallet-a", "outbound", 600)
        source_two = _observation("out-b", "wallet-b", "outbound", 400)
        colliding_source = _observation("out-x", "wallet-x", "outbound", 400)
        target_one = _observation("in-c", "wallet-c", "inbound", 600)
        target_two = _observation("in-d", "wallet-d", "inbound", 400)
        bundle_claims = [
            QuantityClaim(
                claim_id="bundle-a-c",
                source=QuantitySlice(source_one.quantity_hash, 0, 600),
                target=QuantitySlice(target_one.quantity_hash, 0, 600),
                state=INTERNAL_REVIEWED,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="reviewed_nm_bridge",
                atomic_bundle_id="component:1",
            ),
            QuantityClaim(
                claim_id="bundle-b-d",
                source=QuantitySlice(source_two.quantity_hash, 0, 400),
                target=QuantitySlice(target_two.quantity_hash, 0, 400),
                state=INTERNAL_REVIEWED,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="reviewed_nm_bridge",
                atomic_bundle_id="component:1",
            ),
        ]
        collision = QuantityClaim(
            claim_id="other-d",
            source=QuantitySlice(colliding_source.quantity_hash, 0, 400),
            target=QuantitySlice(target_two.quantity_hash, 0, 400),
            state=INTERNAL_REVIEWED,
            priority=ClaimPriority.REVIEWED_COMPONENT,
            reason="other_reviewed_claim",
        )
        observations = [
            source_one,
            source_two,
            colliding_source,
            target_one,
            target_two,
        ]

        projection = project_quantities(
            observations,
            [*bundle_claims, collision],
        )

        decisions_by_contender = {
            tuple(item.contender_claim_ids): item for item in projection.decisions
        }
        self.assertTrue(all(item.state == CONFLICTING for item in projection.decisions))
        self.assertTrue(all(item.target is None for item in projection.decisions))
        self.assertIn(
            ("bundle-a-c", "bundle-b-d"),
            decisions_by_contender,
        )
        self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

        reversed_projection = project_quantities(
            list(reversed(observations)),
            [collision, *reversed(bundle_claims)],
        )
        self.assertEqual(projection.decisions, reversed_projection.decisions)
        self.assertEqual(projection.postings, reversed_projection.postings)

    def test_random_partitions_claim_every_source_msat_once(self):
        rng = random.Random(0xC0570D9)
        for case in range(100):
            total = rng.randint(2, 50_000)
            cut_count = rng.randint(0, min(8, total - 1))
            cuts = sorted(rng.sample(range(1, total), cut_count))
            boundaries = [0, *cuts, total]
            source = _observation(
                f"out-{case}", "wallet-a", "outbound", total
            )
            claims = [
                QuantityClaim(
                    claim_id=f"claim-{case}-{index}",
                    source=QuantitySlice(source.quantity_hash, start, end),
                    state=EXTERNAL_PRESUMED,
                    priority=ClaimPriority.PRESUMED_EXTERNAL_FALLBACK,
                    reason="partition",
                    fallback=True,
                )
                for index, (start, end) in enumerate(
                    zip(boundaries, boundaries[1:])
                )
            ]
            projection = project_quantities([source], claims)
            self.assertEqual(
                sum(item.source.amount_msat for item in projection.decisions),
                total,
            )
            self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

    def test_adversarial_random_overlap_priority_and_targets_conserve(self):
        rng = random.Random(0xA4B17)
        for case in range(50):
            sources = [
                _observation(
                    f"random-out-{case}-{index}",
                    f"wallet-{index}",
                    "outbound",
                    100,
                )
                for index in range(4)
            ]
            targets = [
                _observation(
                    f"random-in-{case}-{index}",
                    f"target-{index}",
                    "inbound",
                    200,
                )
                for index in range(2)
            ]
            claims = []
            for index, source in enumerate(sources):
                claims.append(
                    QuantityClaim(
                        claim_id=f"fallback-{case}-{index}",
                        source=QuantitySlice(source.quantity_hash, 0, 100),
                        state=EXTERNAL_PRESUMED,
                        priority=ClaimPriority.PRESUMED_EXTERNAL_FALLBACK,
                        reason="fallback",
                        fallback=True,
                    )
                )
                candidate_start = rng.randint(0, 100)
                claims.append(
                    QuantityClaim(
                        claim_id=f"candidate-{case}-{index}",
                        source=QuantitySlice(source.quantity_hash, 0, 100),
                        target=QuantitySlice(
                            targets[0].quantity_hash,
                            candidate_start,
                            candidate_start + 100,
                        ),
                        state=CUSTODY_CANDIDATE,
                        priority=ClaimPriority.HEURISTIC_CANDIDATE,
                        reason="candidate",
                    )
                )
                exact_start = rng.randint(0, 79)
                exact_end = rng.randint(exact_start + 1, 100)
                target_start = rng.randint(0, 200 - (exact_end - exact_start))
                claims.append(
                    QuantityClaim(
                        claim_id=f"exact-{case}-{index}",
                        source=QuantitySlice(
                            source.quantity_hash,
                            exact_start,
                            exact_end,
                        ),
                        target=QuantitySlice(
                            targets[1].quantity_hash,
                            target_start,
                            target_start + exact_end - exact_start,
                        ),
                        state=INTERNAL_VERIFIED,
                        priority=ClaimPriority.EXACT_NATIVE_EVENT,
                        reason="exact",
                    )
                )
                suspense_start = rng.randint(0, 89)
                suspense_end = rng.randint(suspense_start + 1, 100)
                claims.append(
                    QuantityClaim(
                        claim_id=f"suspense-{case}-{index}",
                        source=QuantitySlice(
                            source.quantity_hash,
                            suspense_start,
                            suspense_end,
                        ),
                        state=CUSTODY_SUSPENSE,
                        priority=ClaimPriority.ACCOUNTING_CONVENTION,
                        reason="explicit_suspense",
                    )
                )

            projection = project_quantities([*sources, *targets], claims)
            by_source = {}
            for decision in projection.decisions:
                by_source.setdefault(decision.source.observation_hash, []).append(
                    decision.source
                )
            for source in sources:
                slices = sorted(by_source[source.quantity_hash])
                self.assertEqual(slices[0].start_msat, 0)
                self.assertEqual(slices[-1].end_msat, 100)
                self.assertTrue(
                    all(
                        left.end_msat == right.start_msat
                        for left, right in zip(slices, slices[1:])
                    )
                )
            selected_targets = sorted(
                item.target
                for item in projection.decisions
                if item.target is not None
            )
            for left, right in zip(selected_targets, selected_targets[1:]):
                if left.observation_hash == right.observation_hash:
                    self.assertLessEqual(left.end_msat, right.start_msat)
            self.assertEqual(projection.totals_by_asset(), {"BTC": 0})

            reversed_projection = project_quantities(
                [*reversed(targets), *reversed(sources)],
                reversed(claims),
            )
            self.assertEqual(projection.decisions, reversed_projection.decisions)
            self.assertEqual(projection.postings, reversed_projection.postings)


if __name__ == "__main__":
    unittest.main()
