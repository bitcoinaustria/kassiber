"""Frozen pre-snapshot composition at ed665815; test-only parity reference.

Its loader follows the source order of `transaction_order`, as production does.
"""
from __future__ import annotations

from dataclasses import asdict, dataclass, fields, replace
from datetime import datetime, timezone
import json
import sqlite3
import uuid
from typing import Any, Callable, Mapping

from kassiber.errors import AppError
from kassiber.msat import btc_to_msat, dec, msat_to_btc
from kassiber.time_utils import now_iso, parse_iso_datetime_or_none
from kassiber.tax_policy import require_tax_processing_supported
from kassiber.core import custody_authored_migration
from kassiber.core import custody_components
from kassiber.core import custody_gap_reviews
from kassiber.core import custody_interpreters
from kassiber.core import custody_quantity_runtime
from kassiber.core import custody_quantity_store
from kassiber.core import custody_tax_projection
from kassiber.core import custody_filed_reports
from kassiber.core import loans
from kassiber.core import ownership
from kassiber.core import ownership_transfers
from kassiber.core import pricing
from kassiber.core import quarantine_catalog
from kassiber.core import tax_events
from kassiber.core import transfer_matching
from kassiber.core.custody_evidence import build_canonical_quantity_input, enriched_quantity_rows
from kassiber.core.transaction_order import transaction_order_sql
from kassiber.core.engines import KassiberTaxEngine, TaxEngineLedgerInputs
from kassiber.core.lightning import channel_lifecycle
from kassiber.core.repo import resolve_scope


from kassiber.core.custody_journal import (CustodyJournalDecisions, CustodyJournalProjection, custody_build_clock, latest_transaction_rates_for_profile, duplicate_label_warnings, component_integrity_blockers, ownership_review_projection)

class CustodyJournalBuilder:
    """Build one profile's canonical custody and tax journal state.

    ``as_of`` fixes the instant that time-dependent review state (swap
    dismissal expiry) is judged against. Without it each build reads the wall
    clock once, at its start, and uses that one value throughout.
    """

    def __init__(
        self,
        conn,
        profile: Mapping[str, Any],
        *,
        as_of: str | None = None,
    ) -> None:
        if as_of is not None and parse_iso_datetime_or_none(as_of) is None:
            raise ValueError(f"as_of is not an RFC3339 timestamp: {as_of!r}")
        self.conn = conn
        self.profile = profile
        self.profile_id = str(profile["id"])
        self.as_of = as_of

    def _transactions(self):
        rows = self.conn.execute(
            """
            SELECT
                t.*,
                w.label AS wallet_label,
                w.kind AS wallet_kind,
                w.account_id AS wallet_account_id,
                w.config_json AS config_json,
                observation.authority_version AS observation_authority_version,
                observation.observer_kinds_json AS observation_observer_kinds_json,
                observation.graph_hash AS observation_graph_hash,
                observation.quantity_hash AS observation_quantity_hash,
                observation.fee_attribution AS observation_fee_attribution,
                observation.application_revision AS observation_application_revision,
                observation.observed_at AS observation_observed_at,
                COALESCE(a.code, 'treasury') AS account_code,
                COALESCE(a.label, 'Treasury') AS account_label
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
            LEFT JOIN accounts a ON a.id = w.account_id
            LEFT JOIN chain_observation_provenance observation
              ON observation.transaction_id = t.id
            WHERE t.profile_id = ? AND t.excluded = 0
            ORDER BY """ + transaction_order_sql("t") + """
            """,
            (self.profile_id,),
        ).fetchall()

        return [
            {
                **dict(row),
                "_closed_confirmation_observation": ownership_transfers.capture_confirmation_observation(row),
            }
            for row in rows
        ]

    def _wallet_refs(self) -> dict[str, dict[str, Any]]:
        refs: dict[str, dict[str, Any]] = {}
        wallet_rows = self.conn.execute(
            """
            SELECT
                w.id AS id,
                w.label AS label,
                w.kind AS kind,
                w.account_id AS wallet_account_id,
                COALESCE(a.code, 'treasury') AS account_code,
                COALESCE(a.label, 'Treasury') AS account_label
            FROM wallets w
            LEFT JOIN accounts a ON a.id = w.account_id
            WHERE w.profile_id = ?
            """,
            (self.profile_id,),
        ).fetchall()
        for wallet in wallet_rows:
            refs[wallet["id"]] = {
                "id": wallet["id"],
                "label": wallet["label"],
                "kind": wallet["kind"],
                "wallet_account_id": wallet["wallet_account_id"],
                "account_code": wallet["account_code"],
                "account_label": wallet["account_label"],
            }
        return refs

    def _channel_context(self, rows, wallet_refs_by_id):
        records = self.conn.execute(
            """
            SELECT
                r.txid, r.outpoint, r.tag, r.wallet_id, r.channel_id,
                r.amount_msat, r.raw_json AS raw_json,
                w.config_json AS config_json,
                b.chain AS chain, b.network AS network
            FROM lightning_node_records r
            JOIN wallets w ON w.id = r.wallet_id
            LEFT JOIN backends b ON b.name = r.backend_name
            WHERE r.profile_id = ? AND r.record_type = 'channel'
            """,
            (self.profile_id,),
        ).fetchall()
        if not records:
            return {}, []
        channel_rows: list[dict[str, Any]] = []
        for record in records:
            if not record["txid"]:
                continue
            common = {
                "wallet_id": record["wallet_id"],
                "channel_id": record["channel_id"],
                "config_json": record["config_json"],
                "raw_json": record["raw_json"],
                "chain": record["chain"],
                "network": record["network"],
            }
            if record["tag"] == "channel_close":
                channel_rows.append(
                    {
                        **common,
                        "closing_txid": record["txid"],
                        "close_balance_msat": record["amount_msat"],
                    }
                )
            else:
                channel_rows.append(
                    {
                        **common,
                        "funding_txid": record["txid"],
                        "funding_outpoint": record["outpoint"],
                        "funding_amount_msat": record["amount_msat"],
                    }
                )
        roles = channel_lifecycle.channel_role_map(channel_rows, rows)
        pairs = channel_lifecycle.channel_transfer_pairs(
            channel_rows,
            rows,
            wallet_refs_by_id,
        )
        return roles, pairs

    def build_custody_decisions(self) -> CustodyJournalDecisions:
        from kassiber.core.book_network import require_book_accounting
        as_of = self.as_of if self.as_of is not None else custody_build_clock()
        require_book_accounting(self.conn, self.profile_id)
        require_tax_processing_supported(self.profile)
        rows = self._transactions()
        rates = latest_transaction_rates_for_profile(
            self.conn,
            self.profile_id,
        )
        wallet_refs_by_id = self._wallet_refs()
        warnings = duplicate_label_warnings(wallet_refs_by_id)

        owned_index = None
        if any(
            row["observation_authority_version"] is not None
            for row in rows
        ):
            index_wallets = ownership.load_profile_wallets(self.conn, self.profile_id)
            owned_index, ownership_warnings = ownership.build_owned_index(
                self.conn,
                self.profile_id,
                index_wallets,
            )
            warnings.extend(
                {"code": "ownership_index", "message": str(message)}
                for message in ownership_warnings or ()
            )

        loan_legs = self.conn.execute(
            "SELECT transaction_id, role FROM loan_legs "
            "WHERE profile_id = ? AND deleted_at IS NULL",
            (self.profile_id,),
        ).fetchall()
        channel_roles, channel_transfer_pairs = self._channel_context(
            rows,
            wallet_refs_by_id,
        )
        active_components = list(
            custody_components.iter_authored_active_components(
                self.conn,
                profile_id=self.profile_id,
                include_local_evidence=False,
            )
        )
        migration_quarantines = custody_authored_migration.load_migration_quarantines(
            self.conn,
            profile_id=self.profile_id,
        )
        from kassiber.core.import_identity import duplicate_review_quarantines
        migration_quarantines = (*migration_quarantines,
                                *duplicate_review_quarantines(self.conn, self.profile_id))
        component_blockers = component_integrity_blockers(
            self.conn,
            self.profile_id,
            components=active_components,
        )
        evidence_snapshots = custody_quantity_store.load_component_evidence_snapshots(
            self.conn,
            self.profile_id,
        )
        dismissed_fingerprints = custody_gap_reviews.latest_dismissed_fingerprints(
            self.conn,
            self.profile_id,
        )

        ignored_gap_ids: set[str] = set()
        ignored_gap_ids.update(
            str(leg["transaction_id"])
            for leg in loan_legs
            if leg["transaction_id"] not in (None, "")
        )
        ignored_gap_ids.update(str(item) for item in channel_roles)
        for pair in channel_transfer_pairs:
            for side in ("out", "in"):
                row = pair.get(side) or {}
                transaction_id = row.get("journal_transaction_id") or row.get("id")
                if transaction_id not in (None, ""):
                    ignored_gap_ids.add(str(transaction_id))

        component_transaction_ids = tuple(
            sorted(
                {
                    str(leg.get("anchor_transaction_id") or leg.get("transaction_id"))
                    for component in active_components
                    for leg in component.get("legs", ())
                    if leg.get("anchor_transaction_id") or leg.get("transaction_id")
                }
            )
        )
        native_non_event_ids = ownership_transfers.native_zero_delta_ids(rows, owned_index)
        enriched_rows = enriched_quantity_rows(rows, evidence_only=True)
        from kassiber.core.source_overlap import reconcile_receipt_quantities
        canonical_input = build_canonical_quantity_input(
            enriched_rows,
            non_event_transaction_ids=tuple(sorted(native_non_event_ids)),
            receipt_reconciliations=reconcile_receipt_quantities(
                self.conn, self.profile_id, rows, owned_index=owned_index,
            ),
        )
        swap_dismissals = self.conn.execute(
            "SELECT * FROM transaction_pair_dismissals WHERE profile_id = ?",
            (self.profile_id,),
        ).fetchall()
        interpretation = custody_interpreters.compile_custody_interpreters(
            rows,
            canonical_input,
            profile=self.profile,
            swap_dismissals=swap_dismissals,
            wallet_refs_by_id=wallet_refs_by_id,
            owned_index=owned_index,
            channel_transfer_pairs=channel_transfer_pairs,
            channel_roles=channel_roles,
            loan_legs=loan_legs,
            component_transaction_ids=component_transaction_ids,
            components=active_components,
            as_of=as_of,
        )
        if native_non_event_ids:
            interpretation = replace(
                interpretation,
                non_event_transaction_ids=tuple(sorted({
                    *interpretation.non_event_transaction_ids, *native_non_event_ids,
                })),
            )
        if migration_quarantines:
            interpretation = replace(
                interpretation,
                blocked_transaction_ids=tuple(
                    sorted(
                        {
                            *interpretation.blocked_transaction_ids,
                            *(
                                str(item["transaction_id"])
                                for item in migration_quarantines
                            ),
                        }
                    )
                ),
                quarantines=(
                    *interpretation.quarantines,
                    *migration_quarantines,
                ),
            )
        projection_ignored_ids = {
            *ignored_gap_ids,
            *component_transaction_ids,
        }
        observations_by_hash = {
            item.quantity_hash: item for item in canonical_input.observations
        }
        for claim in interpretation.claims:
            # Gap discovery must retain unmatched/suspense sources so the
            # review queue can explain a missing-wallet hypothesis. Only an
            # authoritative target-bearing custody edge suppresses both
            # boundaries from the shared candidate population.
            if claim.target is None:
                continue
            source = observations_by_hash.get(claim.source.observation_hash)
            if source is not None:
                projection_ignored_ids.add(source.transaction_id)
            target = observations_by_hash.get(claim.target.observation_hash)
            if target is not None:
                projection_ignored_ids.add(target.transaction_id)
        projection_ignored_transaction_ids = tuple(
            sorted(projection_ignored_ids)
        )
        runtime_ignored_transaction_ids = tuple(
            sorted(
                {
                    *projection_ignored_ids,
                    *interpretation.blocked_transaction_ids,
                }
            )
        )
        from kassiber.core import custody_gaps

        gap_search_result, _gap_legs = custody_gaps.load_gap_search_result(
            self.conn,
            self.profile_id,
            ignored_transaction_ids=projection_ignored_transaction_ids,
            accounting_ignored_transaction_ids=runtime_ignored_transaction_ids,
        )
        quantity_state = custody_quantity_runtime.build_canonical_quantity_state(
            enriched_rows,
            canonical_input=canonical_input,
            interpreter_claims=interpretation.claims,
            effective_components=[
                component for component in active_components
                if str(component["id"]) not in interpretation.native_reconciled_component_ids
            ],
            native_evidence=interpretation.native_audits,
            interpreter_blockers=interpretation.blocking_quarantines,
            ignored_gap_transaction_ids=runtime_ignored_transaction_ids,
            component_evidence_snapshots=evidence_snapshots,
            dismissed_gap_fingerprints=dismissed_fingerprints,
            gap_search_result=gap_search_result,
        )
        direct_payout_records = list(quantity_state.reviewed_direct_payouts)
        custody_transfers = custody_quantity_runtime.canonical_internal_transfer_rows(
            quantity_state,
            wallet_refs_by_id,
        )
        channel_non_event_ids = tuple(
            sorted(
                str(transaction_id)
                for transaction_id, role in channel_roles.items()
                if str(role) in {loans.CHANNEL_OPEN, loans.CHANNEL_CLOSE}
            )
        )
        return CustodyJournalDecisions(
            rows=rows,
            direct_payout_records=direct_payout_records,
            rates=rates,
            wallet_refs_by_id=wallet_refs_by_id,
            warnings=warnings,
            owned_index=owned_index,
            loan_legs=loan_legs,
            channel_roles=channel_roles,
            active_components=active_components,
            component_blockers=component_blockers,
            interpretation=interpretation,
            quantity_state=quantity_state,
            custody_transfers=custody_transfers,
            channel_non_event_ids=channel_non_event_ids,
            gap_ignored_transaction_ids=projection_ignored_transaction_ids,
            gap_accounting_ignored_transaction_ids=runtime_ignored_transaction_ids,
            as_of=as_of,
        )

    def build_custody_projection(self) -> CustodyJournalProjection:
        decisions = self.build_custody_decisions()
        finalized_projection = custody_tax_projection.compile_finalized_tax_projection(
            self.profile,
            decisions.rows,
            decisions.quantity_state,
            non_event_transaction_ids=(
                *decisions.interpretation.non_event_transaction_ids,
                *decisions.channel_non_event_ids,
            ),
            blocked_transaction_ids=(
                decisions.interpretation.blocked_transaction_ids
            ),
            interpreter_quarantines=decisions.interpretation.quarantines,
            direct_payout_records=decisions.direct_payout_records,
        )
        return CustodyJournalProjection(
            **{
                field.name: getattr(decisions, field.name)
                for field in fields(CustodyJournalDecisions)
            },
            finalized_tax_projection=finalized_projection,
        )

    def build(self) -> dict[str, Any]:
        custody = self.build_custody_projection()
        engine_state = KassiberTaxEngine(self.profile).build_ledger_state(
            TaxEngineLedgerInputs(
                finalized_tax_projection=custody.finalized_tax_projection,
                wallet_refs_by_id=custody.wallet_refs_by_id,
                direct_payout_records=custody.direct_payout_records,
            )
        )
        current_wallet_balances = {
            (str(wallet_id), str(asset)): btc_to_msat(value["quantity"])
            for (wallet_id, _label, _account, asset), value
            in engine_state.wallet_holdings.items()
        }
        known_non_event_reasons = {
            str(leg["transaction_id"]): f"loan_{leg['role']}_non_event"
            for leg in custody.loan_legs
            if leg["transaction_id"] is not None
        }
        known_non_event_reasons.update(
            {
                str(transaction_id): f"lightning_{role}_non_event"
                for transaction_id, role in custody.channel_roles.items()
            }
        )
        for component in custody.active_components:
            validation = component.get("validation") or {}
            if int(validation.get("suspense_msat") or 0) <= 0:
                continue
            for leg in component.get("legs", ()):
                if leg.get("role") == "source" and leg.get("transaction_id"):
                    known_non_event_reasons[str(leg["transaction_id"])] = (
                        "custody_component_residual_suspense"
                    )
        quantity_differences = custody_quantity_runtime.compare_wallet_balances(
            custody.quantity_state,
            current_wallet_balances,
            known_non_event_reasons=known_non_event_reasons,
        )
        ownership_reviews = ownership_review_projection(
            custody.rows,
            custody.owned_index,
            engine_state.quarantines,
            custody.active_components,
        )

        return {
            "entries": engine_state.entries,
            "quarantines": engine_state.quarantines,
            "cross_asset_pairs": engine_state.cross_asset_pairs,
            "direct_swap_payouts": engine_state.direct_swap_payouts,
            "tax_summary": engine_state.tax_summary,
            "account_holdings": engine_state.account_holdings,
            "wallet_holdings": engine_state.wallet_holdings,
            "ownership_review_counts": ownership_reviews["counts"],
            "ownership_review_candidates": ownership_reviews["candidates"],
            "custody_component_blockers": custody.component_blockers,
            "custody_native_reconciliations": [
                asdict(item) for item in custody.interpretation.native_component_reconciliations
            ],
            "custody_quantity": custody.quantity_state,
            "custody_transfers": custody.custody_transfers,
            "quantity_differences": quantity_differences,
            "latest_rates": custody.rates,
            "warnings": custody.warnings,
            "custody_gap_ignored_transaction_ids": (
                custody.gap_ignored_transaction_ids
            ),
            "custody_gap_accounting_ignored_transaction_ids": (
                custody.gap_accounting_ignored_transaction_ids
            ),
        }
