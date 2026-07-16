"""Canonical custody-to-journal composition.

This module owns the production path from imported observations and authored
custody evidence through quantity arbitration, finalized tax inputs, and the
tax engine.  CLI, daemon, reports, and tests must call this seam rather than
reassembling custody interpretation themselves.
"""

from __future__ import annotations

from dataclasses import dataclass, fields, replace
import json
import sqlite3
import uuid
from typing import Any, Mapping

from ..msat import btc_to_msat, dec, msat_to_btc
from ..time_utils import now_iso
from ..tax_policy import require_tax_processing_supported
from . import custody_authored_migration
from . import custody_components
from . import custody_gap_reviews
from . import custody_gaps
from . import custody_interpreters
from . import custody_quantity_runtime
from . import custody_quantity_store
from . import custody_tax_projection
from . import custody_tax_migration
from . import custody_filed_reports
from . import loans
from . import ownership
from . import ownership_transfers
from . import pricing
from . import tax_events
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
    active_components,
) -> dict[str, Any]:
    blocked_reasons = {
        str(quarantine["transaction_id"]): str(quarantine["reason"])
        for quarantine in quarantines
    }
    if not blocked_reasons:
        return {"total": 0, "by_reason": {}}
    active_review_records: list[dict[str, Any]] = []
    for component in active_components:
        legs = {
            str(leg.get("id") or ""): leg
            for leg in component.get("legs", ())
        }
        for allocation in component.get("allocations", ()):
            source = legs.get(str(allocation.get("source_leg_id") or ""), {})
            sink = legs.get(str(allocation.get("sink_leg_id") or ""), {})
            out_id = source.get("anchor_transaction_id") or source.get(
                "transaction_id"
            )
            in_id = sink.get("anchor_transaction_id") or sink.get(
                "transaction_id"
            )
            if out_id in (None, ""):
                continue
            active_review_records.append(
                {
                    "out_transaction_id": str(out_id),
                    "in_transaction_id": (
                        None if in_id in (None, "") else str(in_id)
                    ),
                    "kind": component.get("component_type"),
                    "policy": component.get("conversion_policy"),
                    "deleted_at": None,
                }
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
        migration_quarantines = custody_authored_migration.load_migration_quarantines(
            self.conn,
            profile_id=self.profile_id,
        )
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
        canonical_input = build_canonical_quantity_input(enriched_quantity_rows(rows))
        interpretation = custody_interpreters.compile_custody_interpreters(
            rows,
            canonical_input,
            wallet_refs_by_id=wallet_refs_by_id,
            owned_index=owned_index,
            channel_transfer_pairs=channel_transfer_pairs,
            channel_roles=channel_roles,
            loan_legs=loan_legs,
            component_transaction_ids=component_transaction_ids,
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
                custody.active_components,
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


def store_ledger_state(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    state: Mapping[str, Any],
    *,
    created_at: str | None = None,
) -> dict[str, Any]:
    """Replace every stored projection inside the caller's transaction."""

    profile_id = str(profile["id"])
    workspace_id = str(profile["workspace_id"])
    stored_at = created_at or now_iso()
    conn.execute("DELETE FROM journal_entries WHERE profile_id = ?", (profile_id,))
    conn.execute("DELETE FROM journal_quarantines WHERE profile_id = ?", (profile_id,))
    conn.execute("DELETE FROM journal_tax_summary WHERE profile_id = ?", (profile_id,))
    conn.execute(
        "DELETE FROM journal_account_holdings WHERE profile_id = ?", (profile_id,)
    )
    conn.execute(
        "DELETE FROM journal_wallet_holdings WHERE profile_id = ?", (profile_id,)
    )

    custody_quantity = state.get("custody_quantity")
    quantity_counts = {"postings": 0, "issues": 0, "balances": 0, "decisions": 0}
    if custody_quantity is not None:
        quantity_counts = custody_quantity_store.replace_canonical_quantity_state(
            conn,
            workspace_id=workspace_id,
            profile_id=profile_id,
            state=custody_quantity,
            created_at=stored_at,
        )
    pricing_by_tx = {
        row["id"]: row
        for row in conn.execute(
            "SELECT id, pricing_source_kind, pricing_quality FROM transactions "
            "WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }
    journal_entry_rows = []
    for entry in state["entries"]:
        exact_payload = pricing.journal_exact_payload(entry)
        tx_pricing = pricing_by_tx.get(entry["transaction_id"])
        journal_entry_rows.append(
            (
                entry["id"],
                entry["workspace_id"],
                entry["profile_id"],
                entry["transaction_id"],
                entry["wallet_id"],
                entry["account_id"],
                entry["occurred_at"],
                entry["entry_type"],
                entry["asset"],
                btc_to_msat(entry["quantity"]),
                float(entry["fiat_value"]),
                float(entry["unit_cost"]),
                float(entry["cost_basis"])
                if entry["cost_basis"] is not None
                else None,
                float(entry["proceeds"]) if entry["proceeds"] is not None else None,
                float(entry["gain_loss"])
                if entry["gain_loss"] is not None
                else None,
                exact_payload["fiat_value_exact"],
                exact_payload["unit_cost_exact"],
                exact_payload["cost_basis_exact"],
                exact_payload["proceeds_exact"],
                exact_payload["gain_loss_exact"],
                tx_pricing["pricing_source_kind"] if tx_pricing else None,
                tx_pricing["pricing_quality"] if tx_pricing else None,
                entry["description"],
                entry.get("at_category"),
                entry.get("at_kennzahl"),
                entry.get("capital_gains_type"),
                stored_at,
            )
        )
    conn.executemany(
        """
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
            occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
            cost_basis, proceeds, gain_loss, fiat_value_exact, unit_cost_exact,
            cost_basis_exact, proceeds_exact, gain_loss_exact, pricing_source_kind,
            pricing_quality, description, at_category, at_kennzahl,
            capital_gains_type, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        journal_entry_rows,
    )

    deduped_quarantines = tax_events.dedupe_quarantines(state["quarantines"])
    live_transaction_ids = {
        str(row["id"])
        for row in conn.execute(
            "SELECT id FROM transactions WHERE profile_id = ?", (profile_id,)
        ).fetchall()
    }
    deduped_quarantines = [
        quarantine
        for quarantine in deduped_quarantines
        if str(quarantine["transaction_id"]) in live_transaction_ids
    ]
    conn.executemany(
        """
        INSERT INTO journal_quarantines(
            transaction_id, workspace_id, profile_id, reason, detail_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?)
        """,
        [
            (
                quarantine["transaction_id"],
                quarantine["workspace_id"],
                quarantine["profile_id"],
                quarantine["reason"],
                quarantine["detail_json"],
                stored_at,
            )
            for quarantine in deduped_quarantines
        ],
    )
    conn.executemany(
        """
        INSERT INTO journal_tax_summary(
            id, workspace_id, profile_id, year, asset, transaction_type,
            capital_gains_type, quantity, proceeds, cost_basis, gain_loss, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                str(uuid.uuid4()),
                workspace_id,
                profile_id,
                int(row["year"]),
                row["asset"],
                row["transaction_type"],
                row.get("capital_gains_type"),
                int(row.get("quantity_msat") or btc_to_msat(row["quantity"])),
                float(row["proceeds"]),
                float(row["cost_basis"]),
                float(row["gain_loss"]),
                stored_at,
            )
            for row in state["tax_summary"]
        ],
    )
    conn.executemany(
        """
        INSERT INTO journal_account_holdings(
            id, workspace_id, profile_id, account_id, account_code, account_label,
            asset, quantity, cost_basis, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                str(uuid.uuid4()),
                workspace_id,
                profile_id,
                account_id,
                account_code,
                account_label,
                asset,
                btc_to_msat(value["quantity"]),
                float(value["cost_basis"]),
                stored_at,
            )
            for (account_id, account_code, account_label, asset), value in state[
                "account_holdings"
            ].items()
        ],
    )
    conn.executemany(
        """
        INSERT INTO journal_wallet_holdings(
            id, workspace_id, profile_id, wallet_id, wallet_label, account_code,
            asset, quantity, cost_basis, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                str(uuid.uuid4()),
                workspace_id,
                profile_id,
                wallet_id,
                wallet_label,
                account_code,
                asset,
                btc_to_msat(value["quantity"]),
                float(value["cost_basis"]),
                stored_at,
            )
            for (wallet_id, wallet_label, account_code, asset), value in state[
                "wallet_holdings"
            ].items()
        ],
    )
    tx_count = int(
        conn.execute(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            (profile_id,),
        ).fetchone()[0]
    )
    conn.execute(
        """
        UPDATE profiles
        SET last_processed_at = ?, last_processed_tx_count = ?,
            last_processed_input_version = journal_input_version,
            ownership_review_counts_json = ?
        WHERE id = ?
        """,
        (
            stored_at,
            tx_count,
            json.dumps(state["ownership_review_counts"], sort_keys=True),
            profile_id,
        ),
    )
    custody_tax_migration.finalize_first_rebuild(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        rebuilt_at=stored_at,
    )
    filed_impact_resolutions = []
    if (
        not state.get("custody_component_blockers")
        and not deduped_quarantines
        and not (custody_quantity and custody_quantity.report_blocked)
    ):
        filed_impact_resolutions = custody_filed_reports.resolve_pending_custody_impacts(
            conn,
            workspace_id=workspace_id,
            profile_id=profile_id,
            rebuilt_at=stored_at,
        )
    return {
        "processed_at": stored_at,
        "processed_transactions": tx_count,
        "quarantines": deduped_quarantines,
        "custody_quantity": custody_quantity,
        "quantity_counts": quantity_counts,
        "filed_report_impacts_resolved": len(filed_impact_resolutions),
    }


__all__ = [
    "CustodyJournalBuilder",
    "CustodyJournalDecisions",
    "CustodyJournalProjection",
    "build_ledger_state",
    "store_ledger_state",
    "component_integrity_blockers",
    "duplicate_label_warnings",
    "latest_transaction_rates_for_profile",
    "ownership_review_counts",
]
