"""Canonical custody-to-journal composition.

This module owns the production path from imported observations and authored
custody evidence through quantity arbitration, finalized tax inputs, and the
tax engine.  CLI, daemon, reports, and tests must call this seam rather than
reassembling custody interpretation themselves.
"""

from __future__ import annotations

from dataclasses import dataclass, fields, replace
import sqlite3
from typing import Any, Mapping

from ..msat import btc_to_msat, dec, msat_to_btc
from ..tax_policy import require_tax_processing_supported
from . import custody_components
from . import custody_gap_reviews
from . import custody_gaps
from . import custody_interpreters
from . import custody_quantity_runtime
from . import custody_quantity_store
from . import custody_tax_projection
from . import loans
from . import ownership
from . import ownership_transfers
from .custody_evidence import build_canonical_quantity_input, enriched_quantity_rows
from .engines import TaxEngineLedgerInputs, build_tax_engine
from .lightning import channel_lifecycle


def latest_transaction_rates_for_profile(conn, profile_id: str) -> dict[str, Any]:
    """Return the latest usable transaction-derived rate for each asset."""

    try:
        rows = conn.execute(
            """
            SELECT asset, fiat_rate, fiat_value, fiat_rate_exact,
                   fiat_value_exact, amount
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            ORDER BY occurred_at DESC, created_at DESC
            """,
            (profile_id,),
        ).fetchall()
    except sqlite3.OperationalError:
        rows = conn.execute(
            """
            SELECT asset, fiat_rate, fiat_value, amount
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            ORDER BY occurred_at DESC, created_at DESC
            """,
            (profile_id,),
        ).fetchall()
    rates: dict[str, Any] = {}
    for row in rows:
        asset = row["asset"]
        if asset in rates:
            continue
        rate = row["fiat_rate_exact"] if "fiat_rate_exact" in row.keys() else None
        value = (
            row["fiat_value_exact"] if "fiat_value_exact" in row.keys() else None
        )
        rate_dec = dec(rate) if rate is not None else None
        value_dec = dec(value) if value is not None else None
        if rate_dec is None and row["fiat_rate"] is not None:
            rate_dec = dec(row["fiat_rate"])
        if value_dec is None and row["fiat_value"] is not None:
            value_dec = dec(row["fiat_value"])
        if rate_dec is not None:
            rates[asset] = rate_dec
        elif value_dec is not None and row["amount"]:
            rates[asset] = value_dec / msat_to_btc(row["amount"])
    return rates


def duplicate_label_warnings(
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
) -> list[dict[str, Any]]:
    ids_by_label: dict[str, list[str]] = {}
    for ref in wallet_refs_by_id.values():
        label = ref.get("label")
        if label:
            ids_by_label.setdefault(str(label), []).append(str(ref.get("id")))
    return [
        {
            "code": "duplicate_wallet_label",
            "label": label,
            "wallet_ids": sorted(ids),
            "message": (
                f"{len(ids)} wallets share the label '{label}'. Reports key "
                "holdings by wallet label, so their balances merge and a "
                "derived self-transfer can be attributed to the wrong wallet. "
                "Rename them to be unique."
            ),
        }
        for label, ids in sorted(ids_by_label.items())
        if len(ids) > 1
    ]


def component_integrity_blockers(
    conn,
    profile_id: str,
    *,
    components: list[Mapping[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    if components is None:
        components = list(
            custody_components.iter_authored_active_components(
                conn,
                profile_id=profile_id,
                include_local_evidence=False,
            )
        )
    return [
        {
            "component_id": component["id"],
            "lineage_id": component["lineage_id"],
            "revision": component["revision"],
            "issue_codes": sorted(
                {
                    str(issue.get("code") or "unknown")
                    for issue in component["validation"]["issues"]
                }
            ),
        }
        for component in components
        if component["effective_state"] != "active"
    ]


def ownership_review_counts(
    rows,
    owned_index,
    quarantines,
    manual_pair_records,
    direct_payout_records,
) -> dict[str, Any]:
    blocked_reasons = {
        str(quarantine["transaction_id"]): str(quarantine["reason"])
        for quarantine in quarantines
    }
    if not blocked_reasons:
        return {"total": 0, "by_reason": {}}
    active_review_records = [*manual_pair_records]
    active_review_records.extend(
        {
            "out_transaction_id": record["out_transaction_id"],
            "in_transaction_id": None,
            "kind": record["kind"],
            "policy": record["policy"],
            "deleted_at": record["deleted_at"],
        }
        for record in direct_payout_records
    )
    proofs = ownership_transfers.derive_ownership_review_proofs(
        rows,
        index=owned_index or ownership.OwnedIndex(),
        blocked_reasons_by_row_id=blocked_reasons,
        active_pair_records=active_review_records,
    )
    by_reason: dict[str, int] = {}
    for proof in proofs:
        by_reason[proof.reason] = by_reason.get(proof.reason, 0) + 1
    return {"total": len(proofs), "by_reason": dict(sorted(by_reason.items()))}


@dataclass(frozen=True)
class CustodyJournalDecisions:
    """Canonical custody decisions, issues, barriers, and lineage inputs."""

    rows: Any
    manual_pair_records: Any
    direct_payout_records: Any
    rates: Any
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]]
    warnings: list[dict[str, Any]]
    owned_index: Any
    loan_legs: Any
    channel_roles: Mapping[str, str]
    active_components: list[Mapping[str, Any]]
    component_blockers: list[dict[str, Any]]
    interpretation: Any
    quantity_state: Any
    custody_transfers: Any
    channel_non_event_ids: tuple[str, ...]


@dataclass(frozen=True)
class CustodyJournalProjection(CustodyJournalDecisions):
    """Custody decisions plus the finalized input consumed by RP2."""

    finalized_tax_projection: Any


class CustodyJournalBuilder:
    """Build one profile's canonical custody and tax journal state."""

    def __init__(self, conn, profile: Mapping[str, Any]) -> None:
        self.conn = conn
        self.profile = profile
        self.profile_id = str(profile["id"])

    def _transactions(self):
        return self.conn.execute(
            """
            SELECT
                t.*,
                w.label AS wallet_label,
                w.kind AS wallet_kind,
                w.account_id AS wallet_account_id,
                w.config_json AS config_json,
                observation.authority_version AS observation_authority_version,
                observation.graph_hash AS observation_graph_hash,
                observation.quantity_hash AS observation_quantity_hash,
                observation.fee_attribution AS observation_fee_attribution,
                observation.application_revision AS observation_application_revision,
                COALESCE(a.code, 'treasury') AS account_code,
                COALESCE(a.label, 'Treasury') AS account_label
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
            LEFT JOIN accounts a ON a.id = w.account_id
            LEFT JOIN chain_observation_provenance observation
              ON observation.transaction_id = t.id
            WHERE t.profile_id = ? AND t.excluded = 0
            ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
            """,
            (self.profile_id,),
        ).fetchall()

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
        require_tax_processing_supported(self.profile)
        rows = self._transactions()
        manual_pair_records = self.conn.execute(
            "SELECT * FROM transaction_pairs "
            "WHERE profile_id = ? AND deleted_at IS NULL",
            (self.profile_id,),
        ).fetchall()
        direct_payout_records = self.conn.execute(
            """
            SELECT p.*, t.asset AS out_asset, t.amount AS out_amount_msat
            FROM direct_swap_payouts p
            JOIN transactions t ON t.id = p.out_transaction_id
            WHERE p.profile_id = ? AND p.deleted_at IS NULL
            """,
            (self.profile_id,),
        ).fetchall()
        rates = latest_transaction_rates_for_profile(
            self.conn,
            self.profile_id,
        )
        wallet_refs_by_id = self._wallet_refs()
        warnings = duplicate_label_warnings(wallet_refs_by_id)

        owned_index = None
        if any(
            row["direction"] == "outbound"
            and (row["raw_json"] or "").find('"vout"') != -1
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
        effective_component_ids = {
            str(component["id"])
            for component in active_components
            if component.get("effective_state") == "active"
        }
        manual_pair_records = [
            record
            for record in manual_pair_records
            if record["component_id"] not in effective_component_ids
        ]
        direct_payout_records = [
            record
            for record in direct_payout_records
            if record["component_id"] not in effective_component_ids
        ]
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

        ignored_gap_ids = {
            str(record[key])
            for record in manual_pair_records
            for key in ("out_transaction_id", "in_transaction_id")
            if record[key] not in (None, "")
        }
        ignored_gap_ids.update(
            str(record["out_transaction_id"])
            for record in direct_payout_records
            if record["out_transaction_id"] not in (None, "")
        )
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
        canonical_input = build_canonical_quantity_input(enriched_quantity_rows(rows))
        interpretation = custody_interpreters.compile_custody_interpreters(
            rows,
            canonical_input,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=manual_pair_records,
            owned_index=owned_index,
            channel_transfer_pairs=channel_transfer_pairs,
            channel_roles=channel_roles,
            loan_legs=loan_legs,
            direct_payout_records=direct_payout_records,
            component_transaction_ids=component_transaction_ids,
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
        gap_search_result, _gap_legs = custody_gaps.load_gap_search_result(
            self.conn,
            self.profile_id,
            ignored_transaction_ids=projection_ignored_transaction_ids,
            accounting_ignored_transaction_ids=runtime_ignored_transaction_ids,
            producer_kind="journal",
        )
        quantity_state = custody_quantity_runtime.build_canonical_quantity_state(
            rows,
            interpreter_claims=interpretation.claims,
            effective_components=active_components,
            native_evidence=interpretation.native_audits,
            direct_payout_records=direct_payout_records,
            interpreter_blockers=interpretation.blocking_quarantines,
            ignored_gap_transaction_ids=runtime_ignored_transaction_ids,
            component_evidence_snapshots=evidence_snapshots,
            dismissed_gap_fingerprints=dismissed_fingerprints,
            gap_search_result=gap_search_result,
        )
        component_conversion_ids = {
            str(item.get("pair_id") or "")
            for item in quantity_state.reviewed_conversion_pairs
        }
        compatibility_conversions = tuple(
            item
            for item in interpretation.cross_asset_pairs
            if str(item.get("pair_id") or "") not in component_conversion_ids
        )
        if compatibility_conversions:
            # During the bounded migration window, the builder remains the
            # sole compatibility interpreter. Persist its reviewed conversion
            # result into the same projection as component-native relations so
            # no report/UI consumer needs to reopen transaction_pairs.
            quantity_state = replace(
                quantity_state,
                reviewed_conversion_pairs=(
                    *quantity_state.reviewed_conversion_pairs,
                    *compatibility_conversions,
                ),
            )
        direct_payout_records = [
            *direct_payout_records,
            *quantity_state.reviewed_direct_payouts,
        ]
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
            manual_pair_records=manual_pair_records,
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
            direct_payout_conflict_transaction_ids=(
                decisions.interpretation.direct_payout_conflict_transaction_ids
            ),
            interpreter_quarantines=decisions.interpretation.quarantines,
            direct_payout_records=decisions.direct_payout_records,
            reviewed_cross_asset_pairs=decisions.interpretation.cross_asset_pairs,
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
        engine_state = build_tax_engine(self.profile).build_ledger_state(
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

        return {
            "entries": engine_state.entries,
            "quarantines": engine_state.quarantines,
            "intra_audit": engine_state.intra_audit,
            "cross_asset_pairs": engine_state.cross_asset_pairs,
            "direct_swap_payouts": engine_state.direct_swap_payouts,
            "tax_summary": engine_state.tax_summary,
            "account_holdings": engine_state.account_holdings,
            "wallet_holdings": engine_state.wallet_holdings,
            "ownership_review_counts": ownership_review_counts(
                custody.rows,
                custody.owned_index,
                engine_state.quarantines,
                custody.manual_pair_records,
                custody.direct_payout_records,
            ),
            "custody_component_blockers": custody.component_blockers,
            "custody_quantity": custody.quantity_state,
            "custody_transfers": custody.custody_transfers,
            "quantity_differences": quantity_differences,
            "latest_rates": custody.rates,
            "warnings": custody.warnings,
        }


def build_ledger_state(conn, profile: Mapping[str, Any]) -> dict[str, Any]:
    """Build canonical journal state through the single custody service seam."""

    return CustodyJournalBuilder(conn, profile).build()


__all__ = [
    "CustodyJournalBuilder",
    "CustodyJournalDecisions",
    "CustodyJournalProjection",
    "build_ledger_state",
    "component_integrity_blockers",
    "duplicate_label_warnings",
    "latest_transaction_rates_for_profile",
    "ownership_review_counts",
]
