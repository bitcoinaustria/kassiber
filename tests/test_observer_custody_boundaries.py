from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from kassiber.core.chain_observer import CoveragePoint, ObserverIdentity
from kassiber.core.chain_observer.provenance import (
    persist_chain_observation_provenance,
)
from kassiber.core.custody_evidence import (
    build_canonical_quantity_input,
    enriched_quantity_rows,
)
from kassiber.core.custody_interpreters import compile_custody_interpreters
from kassiber.core.custody_quantity import (
    CUSTODY_CANDIDATE,
    CUSTODY_SUSPENSE,
    EXTERNAL_CONFIRMED,
    EXTERNAL_PRESUMED,
    INTERNAL_VERIFIED,
)
from kassiber.core.custody_quantity_runtime import build_canonical_quantity_state
from kassiber.core.ownership_policy_epochs import (
    record_observer_policy_coverage,
    technical_coverage_snapshot,
)
from kassiber.db import open_db
from kassiber.time_utils import now_iso


BTC_MSAT = 100_000_000_000


class ObserverCustodyBoundaryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="kassiber-observer-custody-")
        self.addCleanup(self.temp.cleanup)
        self.conn = open_db(Path(self.temp.name) / "data")
        self.addCleanup(self.conn.close)
        timestamp = now_iso()
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'WS', ?)",
            (timestamp,),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('profile', 'ws', 'Treasury', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (timestamp,),
        )
        for wallet_id, label, chain, network in (
            ("source", "Source", "bitcoin", "main"),
            ("destination-incomplete", "Incomplete", "bitcoin", "main"),
            ("operative", "Operative", "bitcoin", "main"),
            ("liquid-source", "Liquid source", "liquid", "liquidv1"),
            ("liquid-destination", "Liquid destination", "liquid", "liquidv1"),
        ):
            self.conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json,
                    created_at
                ) VALUES(?, 'ws', 'profile', ?, 'descriptor', ?, ?)
                """,
                (
                    wallet_id,
                    label,
                    json.dumps({"chain": chain, "network": network}),
                    timestamp,
                ),
            )
        self.profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()

    def _insert_transaction(
        self,
        transaction_id: str,
        wallet_id: str,
        *,
        external_id: str,
        occurred_at: str,
        direction: str,
        asset: str,
        amount_msat: int,
        raw: dict[str, object],
        fee_msat: int = 0,
        amount_includes_fee: bool = False,
        privacy_boundary: str | None = None,
    ) -> None:
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint, occurred_at, direction, asset,
                amount, fee, amount_includes_fee, privacy_boundary, raw_json,
                created_at
            ) VALUES(
                ?, 'ws', 'profile', ?, ?, 'txid', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
            )
            """,
            (
                transaction_id,
                wallet_id,
                external_id,
                f"fingerprint:{transaction_id}",
                occurred_at,
                direction,
                asset,
                amount_msat,
                fee_msat,
                int(amount_includes_fee),
                privacy_boundary,
                json.dumps(raw, sort_keys=True),
                now_iso(),
            ),
        )

    def _authorize(
        self,
        transaction_id: str,
        *,
        observer_kind: str,
        chain: str,
        network: str,
    ) -> None:
        transaction = self.conn.execute(
            "SELECT * FROM transactions WHERE id = ?", (transaction_id,)
        ).fetchone()
        wallet = self.conn.execute(
            "SELECT * FROM wallets WHERE id = ?", (transaction["wallet_id"],)
        ).fetchone()
        persisted = persist_chain_observation_provenance(
            self.conn,
            self.profile,
            wallet,
            application_revision=f"apply:{transaction_id}",
            chain=chain,
            network=network,
            entries=(
                {
                    "external_id": transaction["external_id"],
                    "asset": transaction["asset"],
                    "direction": transaction["direction"],
                    "observer_ids": [f"{observer_kind}:structural-source"],
                    "observer_kinds": [observer_kind],
                },
            ),
        )
        self.assertEqual(persisted, 1)

    def _row(self, transaction_id: str) -> dict[str, object]:
        row = self.conn.execute(
            """
            SELECT
                tx.*,
                wallet.label AS wallet_label,
                wallet.kind AS wallet_kind,
                wallet.config_json AS config_json,
                proof.authority_version AS observation_authority_version,
                proof.graph_hash AS observation_graph_hash,
                proof.quantity_hash AS observation_quantity_hash,
                proof.fee_attribution AS observation_fee_attribution
            FROM transactions tx
            JOIN wallets wallet ON wallet.id = tx.wallet_id
            LEFT JOIN chain_observation_provenance proof
              ON proof.transaction_id = tx.id
            WHERE tx.id = ?
            """,
            (transaction_id,),
        ).fetchone()
        return dict(row)

    @staticmethod
    def _native_evidence(
        transaction_id: str,
        *,
        source_wallet_id: str,
        destination_wallet_id: str,
        asset: str,
        occurred_at: str,
        received_msat: int,
        fee_msat: int = 0,
    ) -> dict[str, object]:
        return {
            "out_id": f"owned:{transaction_id}:out",
            "in_id": f"owned:{transaction_id}:in",
            "out_anchor_transaction_id": transaction_id,
            "in_anchor_transaction_id": transaction_id,
            "from_wallet_id": source_wallet_id,
            "to_wallet_id": destination_wallet_id,
            "asset": asset,
            "occurred_at": occurred_at,
            "pairing_source": "ownership_derived",
            "crypto_sent_msat": received_msat + fee_msat,
            "crypto_received_msat": received_msat,
            "crypto_fee_msat": fee_msat,
        }

    def test_imported_observer_text_cannot_create_verified_internal_custody(self):
        txid = "11" * 32
        occurred_at = "2025-01-01T00:00:00Z"
        self._insert_transaction(
            "fake-bdk",
            "source",
            external_id=txid,
            occurred_at=occurred_at,
            direction="outbound",
            asset="BTC",
            amount_msat=1_000,
            raw={
                "txid": txid,
                "chain": "bitcoin",
                "network": "main",
                "observer": "bdk",
                "vin": [],
                "vout": [{"value": 1, "scriptpubkey": "0014" + "22" * 20}],
            },
        )

        state = build_canonical_quantity_state(
            [self._row("fake-bdk")],
            native_evidence=(
                self._native_evidence(
                    "fake-bdk",
                    source_wallet_id="source",
                    destination_wallet_id="destination-incomplete",
                    asset="BTC",
                    occurred_at=occurred_at,
                    received_msat=900,
                ),
            ),
        )

        self.assertNotIn(
            INTERNAL_VERIFIED,
            {decision.state for decision in state.projection.decisions},
        )
        self.assertEqual(
            [
                (decision.state, decision.source.amount_msat)
                for decision in state.projection.decisions
            ],
            [(EXTERNAL_PRESUMED, 1_000)],
        )
        self.assertIn(
            "native_audit_authoritative_observation_missing",
            {issue.reason for issue in state.issues},
        )

    def test_recorded_pair_source_label_cannot_join_different_native_events(self):
        """The audit label is not itself a custody authority token."""

        for transaction_id, wallet_id, txid, direction in (
            ("spoofed-out", "source", "71" * 32, "outbound"),
            ("spoofed-in", "operative", "72" * 32, "inbound"),
        ):
            self._insert_transaction(
                transaction_id,
                wallet_id,
                external_id=txid,
                occurred_at="2025-01-15T00:00:00Z",
                direction=direction,
                asset="BTC",
                amount_msat=1_000,
                raw={"txid": txid, "chain": "bitcoin", "network": "main"},
            )
        rows = [self._row("spoofed-out"), self._row("spoofed-in")]
        safe_rows = enriched_quantity_rows(rows)
        canonical = build_canonical_quantity_input(safe_rows)
        compilation = compile_custody_interpreters(
            safe_rows,
            canonical,
            wallet_refs_by_id={row["wallet_id"]: row for row in rows},
            channel_transfer_pairs=(
                {
                    "out": rows[0],
                    "in": rows[1],
                    "source": "row_matched",
                },
            ),
        )
        state = build_canonical_quantity_state(
            rows,
            interpreter_claims=compilation.claims,
            native_evidence=compilation.native_audits,
            ignored_gap_transaction_ids=("spoofed-out", "spoofed-in"),
        )

        self.assertNotIn(
            INTERNAL_VERIFIED,
            {decision.state for decision in state.projection.decisions},
        )
        self.assertEqual(
            [decision.state for decision in state.projection.decisions],
            [EXTERNAL_PRESUMED],
        )

    def test_source_technical_coverage_cannot_confirm_an_unknown_destination(self):
        identity = ObserverIdentity(
            id="bdk-source",
            workspace_id="ws",
            profile_id="profile",
            logical_wallet_id="source",
            source_wallet_id="source",
            source_key="descriptor:default",
            observer_kind="bdk",
            chain="bitcoin",
            network="main",
            branch_keys=("receive", "change"),
        )
        record_observer_policy_coverage(
            self.conn,
            identity,
            (
                CoveragePoint("receive", scanned_to=100, highest_used=7),
                CoveragePoint("change", scanned_to=100, highest_used=2),
            ),
        )
        txid = "22" * 32
        self._insert_transaction(
            "covered-outbound",
            "source",
            external_id=txid,
            occurred_at="2025-02-01T00:00:00Z",
            direction="outbound",
            asset="BTC",
            amount_msat=2_000,
            raw={
                "txid": txid,
                "chain": "bitcoin",
                "network": "main",
                "observer": "bdk",
                "vin": [],
                "vout": [{"value": 2, "scriptpubkey": "0014" + "33" * 20}],
            },
        )
        self._authorize(
            "covered-outbound",
            observer_kind="bdk",
            chain="bitcoin",
            network="main",
        )

        coverage = technical_coverage_snapshot(self.conn, "profile")
        state = build_canonical_quantity_state([self._row("covered-outbound")])

        self.assertFalse(coverage["ownership_universe_known"])
        self.assertFalse(coverage["coverage_can_clear_custody_gaps"])
        self.assertEqual(
            {wallet["wallet_label"] for wallet in coverage["wallets"]},
            {"Source"},
        )
        self.assertEqual(
            [
                (decision.state, decision.source.amount_msat)
                for decision in state.projection.decisions
            ],
            [(EXTERNAL_PRESUMED, 2_000)],
        )
        self.assertNotIn(
            EXTERNAL_CONFIRMED,
            {decision.state for decision in state.projection.decisions},
        )

    def test_authoritative_missing_whirlpool_hop_stays_reviewable_not_external(self):
        out_txid = "33" * 32
        in_txid = "44" * 32
        self._insert_transaction(
            "whirlpool-out",
            "source",
            external_id=out_txid,
            occurred_at="2024-01-01T00:00:00Z",
            direction="outbound",
            asset="BTC",
            amount_msat=10 * BTC_MSAT,
            privacy_boundary="whirlpool",
            raw={
                "txid": out_txid,
                "chain": "bitcoin",
                "network": "main",
                "observer": "bdk",
                "vin": [],
                "vout": [],
            },
        )
        self._insert_transaction(
            "whirlpool-return",
            "operative",
            external_id=in_txid,
            occurred_at="2025-01-01T00:00:00Z",
            direction="inbound",
            asset="BTC",
            amount_msat=99 * BTC_MSAT // 10,
            raw={
                "txid": in_txid,
                "chain": "bitcoin",
                "network": "main",
                "observer": "bdk",
                "vin": [],
                "vout": [],
            },
        )
        for transaction_id in ("whirlpool-out", "whirlpool-return"):
            self._authorize(
                transaction_id,
                observer_kind="bdk",
                chain="bitcoin",
                network="main",
            )

        state = build_canonical_quantity_state(
            [self._row("whirlpool-out"), self._row("whirlpool-return")]
        )

        states = {decision.state for decision in state.projection.decisions}
        self.assertEqual(states, {CUSTODY_CANDIDATE, CUSTODY_SUSPENSE})
        self.assertNotIn(EXTERNAL_CONFIRMED, states)
        self.assertFalse(
            self.conn.execute(
                "SELECT 1 FROM wallets WHERE id = 'missing-whirlpool'"
            ).fetchone()
        )
        self.assertEqual(
            {issue.state for issue in state.issues},
            {CUSTODY_CANDIDATE, CUSTODY_SUSPENSE},
        )

    def test_duplicate_event_fee_normalization_preserves_closed_authority(self):
        txid = "66" * 32
        raw = {
            "txid": txid,
            "chain": "bitcoin",
            "network": "main",
            "observer": "bdk",
            "vin": [],
            "vout": [],
        }
        for transaction_id, wallet_id, amount_msat in (
            ("source-a", "source", 499),
            ("source-b", "destination-incomplete", 299),
        ):
            self._insert_transaction(
                transaction_id,
                wallet_id,
                external_id=txid,
                occurred_at="2025-02-15T00:00:00Z",
                direction="outbound",
                asset="BTC",
                amount_msat=amount_msat,
                fee_msat=1,
                raw=raw,
            )
            self._authorize(
                transaction_id,
                observer_kind="bdk",
                chain="bitcoin",
                network="main",
            )

        canonical = build_canonical_quantity_input(
            enriched_quantity_rows(
                [self._row("source-a"), self._row("source-b")]
            )
        )
        observations = sorted(
            canonical.observations,
            key=lambda observation: observation.transaction_id,
        )

        self.assertTrue(
            all(item.authoritative_chain_observation for item in observations)
        )
        self.assertEqual(
            [item.wallet_delta_msat for item in observations],
            [-500, -300],
        )
        self.assertEqual(sum(item.fee_msat for item in observations), 1)
        self.assertEqual(sum(item.principal_msat for item in observations), 799)

    def test_fee_inclusive_duplicate_observation_never_inflates_principal(self):
        txid = "67" * 32
        raw = {
            "txid": txid,
            "chain": "bitcoin",
            "network": "main",
            "observer": "bdk",
            "vin": [],
            "vout": [],
        }
        for transaction_id, wallet_id, gross_msat in (
            ("inclusive-a", "source", 500),
            ("inclusive-b", "destination-incomplete", 300),
        ):
            self._insert_transaction(
                transaction_id,
                wallet_id,
                external_id=txid,
                occurred_at="2025-02-16T00:00:00Z",
                direction="outbound",
                asset="BTC",
                amount_msat=gross_msat,
                fee_msat=1,
                amount_includes_fee=True,
                raw=raw,
            )
            self._authorize(
                transaction_id,
                observer_kind="bdk",
                chain="bitcoin",
                network="main",
            )

        canonical = build_canonical_quantity_input(
            enriched_quantity_rows(
                [self._row("inclusive-a"), self._row("inclusive-b")]
            )
        )
        observations = sorted(
            canonical.observations,
            key=lambda observation: observation.transaction_id,
        )

        self.assertEqual(
            [item.wallet_delta_msat for item in observations],
            [-500, -300],
        )
        self.assertEqual(sum(item.fee_msat for item in observations), 1)
        self.assertEqual(sum(item.principal_msat for item in observations), 799)

    def test_authoritative_lwk_implicit_delta_residual_is_suspense_not_fee(self):
        txid = "55" * 32
        occurred_at = "2025-03-01T00:00:00Z"
        outbound_raw = {
            "txid": txid,
            "chain": "liquid",
            "network": "liquidv1",
            "observer": "lwk",
            "component": {
                "net_sats": -1,
                "fee_sats": 0,
                "transaction_fee_sats": 1,
                "fee_attribution": "implicit_wallet_delta",
            },
            "vin": [],
            "vout": [],
        }
        self._insert_transaction(
            "liquid-out",
            "liquid-source",
            external_id=txid,
            occurred_at=occurred_at,
            direction="outbound",
            asset="LBTC",
            amount_msat=1_000,
            amount_includes_fee=True,
            raw=outbound_raw,
        )
        self._insert_transaction(
            "liquid-in",
            "liquid-destination",
            external_id=txid,
            occurred_at=occurred_at,
            direction="inbound",
            asset="LBTC",
            amount_msat=900,
            raw={
                "txid": txid,
                "chain": "liquid",
                "network": "liquidv1",
                "observer": "lwk",
                "component": {
                    "net_sats": 1,
                    "fee_sats": 0,
                    "transaction_fee_sats": 1,
                    "fee_attribution": "exact",
                },
                "vin": [],
                "vout": [],
            },
        )
        for transaction_id in ("liquid-out", "liquid-in"):
            self._authorize(
                transaction_id,
                observer_kind="lwk",
                chain="liquid",
                network="liquidv1",
            )

        rows = [self._row("liquid-out"), self._row("liquid-in")]
        safe_rows = enriched_quantity_rows(rows)
        canonical = build_canonical_quantity_input(safe_rows)
        compilation = compile_custody_interpreters(
            safe_rows,
            canonical,
            wallet_refs_by_id={row["wallet_id"]: row for row in rows},
            manual_pair_records=(
                {
                    "id": "reviewed-liquid-pair",
                    "out_transaction_id": "liquid-out",
                    "in_transaction_id": "liquid-in",
                    "policy": "carrying-value",
                    "kind": "self-transfer",
                    "pair_source": "manual",
                    "out_amount": 900,
                },
            ),
        )

        state = build_canonical_quantity_state(
            rows,
            interpreter_claims=compilation.claims,
            native_evidence=compilation.native_audits,
            interpreter_blockers=compilation.blocking_quarantines,
        )

        suspense = [
            decision
            for decision in state.projection.decisions
            if decision.state == CUSTODY_SUSPENSE
        ]
        self.assertEqual(
            [(decision.source.amount_msat, decision.reason) for decision in suspense],
            [(100, "implicit_wallet_delta_unallocated")],
        )
        self.assertFalse(
            [posting for posting in state.projection.postings if posting.location_kind == "fee"]
        )
        self.assertNotIn(
            EXTERNAL_CONFIRMED,
            {decision.state for decision in state.projection.decisions},
        )

        native_state = build_canonical_quantity_state(
            rows,
            native_evidence=(
                self._native_evidence(
                    "liquid-out",
                    source_wallet_id="liquid-source",
                    destination_wallet_id="liquid-destination",
                    asset="LBTC",
                    occurred_at=occurred_at,
                    received_msat=900,
                ),
            ),
        )
        self.assertEqual(
            [(issue.state, issue.reason) for issue in native_state.issues],
            [(CUSTODY_SUSPENSE, "implicit_wallet_delta_unallocated")],
        )
        self.assertEqual(
            sorted(
                (decision.state, decision.source.amount_msat, decision.reason)
                for decision in native_state.projection.decisions
            ),
            [
                (CUSTODY_SUSPENSE, 100, "implicit_wallet_delta_unallocated"),
                (INTERNAL_VERIFIED, 900, "ownership_derived"),
            ],
        )


if __name__ == "__main__":
    unittest.main()
