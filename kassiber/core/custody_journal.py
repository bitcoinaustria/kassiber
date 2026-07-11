"""Project effective custody components into the tax-engine input seam.

Raw transaction rows are evidence, never the authored accounting answer.  An
effective custody component atomically replaces every transaction it anchors
with deterministic synthetic rows.  The synthetic rows retain
``journal_transaction_id`` links to real imported rows so journal foreign keys
and transaction-level audit navigation remain intact.

This module is intentionally independent of SQLite and RP2.  Rail adapters can
add evidence to custody components without teaching the tax engine about every
Bitcoin layer; the only journal contract is source-to-sink conservation.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
import json
from typing import Any, Mapping, Sequence

from ..msat import MSAT_PER_BTC
from ..tax_policy import recommended_pair_policy
from .pair_allocation import allocate_fee_msat


OWNED_SINK_ROLES = frozenset({"destination", "retained"})
ATTRIBUTED_SINK_ROLES = frozenset({"fee", "external"})


def _field(row: Mapping[str, Any] | None, key: str, default: Any = None) -> Any:
    if row is None:
        return default
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def _component_anchor_ids(component: Mapping[str, Any]) -> tuple[str, ...]:
    return tuple(
        sorted(
            {
                str(leg.get("anchor_transaction_id") or leg["transaction_id"])
                for leg in component.get("legs", ())
                if (leg.get("anchor_transaction_id") or leg.get("transaction_id"))
                not in (None, "")
            }
        )
    )


@dataclass(frozen=True)
class CustodyComponentBlocker:
    """Typed, component-wide reason no synthetic interpretation was emitted."""

    component_id: str
    code: str
    message: str
    transaction_ids: tuple[str, ...]
    details: Mapping[str, Any]
    quarantine_transaction_ids: tuple[str, ...] | None = None

    def quarantines(self, profile: Mapping[str, Any]) -> list[dict[str, Any]]:
        detail = {
            "component_id": self.component_id,
            "blocker_code": self.code,
            "message": self.message,
            "required_for": "complete_custody_component",
            **dict(self.details),
        }
        return [
            {
                "transaction_id": transaction_id,
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "reason": "custody_component_blocked",
                "detail_json": json.dumps(detail, sort_keys=True),
            }
            for transaction_id in (
                self.transaction_ids
                if self.quarantine_transaction_ids is None
                else self.quarantine_transaction_ids
            )
        ]


@dataclass(frozen=True)
class CustodyJournalProjection:
    rows: tuple[Mapping[str, Any], ...]
    # Raw evidence anchors retained solely as forced-block normalization rows.
    # They never book, but preserve their occurrence times so later lots cannot
    # outrun an unresolved component and acquire false basis provenance.
    blocked_anchor_rows: tuple[Mapping[str, Any], ...]
    manual_pair_records: tuple[Mapping[str, Any], ...]
    quarantines: tuple[dict[str, Any], ...]
    blockers: tuple[CustodyComponentBlocker, ...]
    claimed_transaction_ids: frozenset[str]


class _ProjectionError(Exception):
    def __init__(self, code: str, message: str, **details: Any):
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = details


def _authored_active(component: Mapping[str, Any]) -> bool:
    """Return whether the user-authored lifecycle says this component is live.

    Materialized DB components always carry ``state``.  The fallback preserves
    compatibility with pure-engine callers and older tests that predate the
    authored/effective distinction and supplied only ``effective_state``.
    An explicit non-active authored state always wins.
    """

    authored_state = component.get("state")
    if authored_state is None:
        return component.get("effective_state") == "active"
    return authored_state == "active"


def _effective_active(component: Mapping[str, Any]) -> bool:
    return component.get("effective_state") == "active"


def _inactive_authored_details(component: Mapping[str, Any]) -> dict[str, Any]:
    validation = component.get("validation")
    issues: Sequence[Mapping[str, Any]] = ()
    if isinstance(validation, Mapping):
        raw_issues = validation.get("issues")
        if isinstance(raw_issues, Sequence) and not isinstance(
            raw_issues, (str, bytes, bytearray)
        ):
            issues = [issue for issue in raw_issues if isinstance(issue, Mapping)]
    return {
        "authored_state": component.get("state"),
        "effective_state": component.get("effective_state"),
        "validation_issues": [dict(issue) for issue in issues],
        "resolution": (
            "complete or correct the component evidence and legs, or supersede "
            "the active revision"
        ),
    }


def _validate_anchor_coverage(
    component: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> None:
    anchor_ids = _component_anchor_ids(component)
    if not anchor_ids:
        raise _ProjectionError(
            "custody_component_anchor_missing",
            "an effective custody component has no real imported transaction anchor",
        )
    missing = [transaction_id for transaction_id in anchor_ids if transaction_id not in rows_by_id]
    if missing:
        raise _ProjectionError(
            "custody_component_anchor_unavailable",
            "one or more component anchors are excluded or unavailable to this journal run",
            unavailable_transaction_ids=missing,
        )

    reviewed_by_anchor: dict[str, int] = {transaction_id: 0 for transaction_id in anchor_ids}
    for leg in component.get("legs", ()):
        transaction_id = leg.get("transaction_id")
        if transaction_id not in (None, ""):
            anchor_id = str(transaction_id)
            anchor = rows_by_id.get(anchor_id)
            direction = _field(anchor, "direction")
            role = leg.get("role")
            # An anchored fee/external sink classifies part of an outbound
            # source; it is not a second replacement of the raw row. Count only
            # the direction-bearing leg side, matching the imported row's own
            # economic sign. This lets source=100 and fee=1 share one anchor
            # without falsely claiming reviewed coverage of 101.
            if (
                (direction == "outbound" and role == "source")
                or (direction == "inbound" and role in OWNED_SINK_ROLES)
            ):
                reviewed_by_anchor[anchor_id] += int(leg.get("amount_msat") or 0)

    deltas: list[dict[str, Any]] = []
    for transaction_id in anchor_ids:
        row = rows_by_id[transaction_id]
        expected = int(_field(row, "amount") or 0)
        if _field(row, "direction") == "outbound" and not bool(
            _field(row, "amount_includes_fee")
        ):
            expected += int(_field(row, "fee") or 0)
        reviewed = reviewed_by_anchor[transaction_id]
        if reviewed != expected:
            deltas.append(
                {
                    "transaction_id": transaction_id,
                    "raw_economic_msat": expected,
                    "reviewed_component_msat": reviewed,
                    "reviewed_minus_raw_msat": reviewed - expected,
                }
            )
    if deltas:
        raise _ProjectionError(
            "custody_component_anchor_coverage_mismatch",
            "component legs do not fully replace their imported transaction anchors",
            anchor_coverage=deltas,
        )


def _inferred_allocations(component: Mapping[str, Any]) -> list[dict[str, Any]]:
    explicit = [dict(allocation) for allocation in component.get("allocations", ())]
    if explicit:
        return sorted(explicit, key=lambda item: (int(item.get("ordinal") or 0), str(item.get("id") or "")))

    legs = [leg for leg in component.get("legs", ()) if int(leg.get("amount_msat") or 0) > 0]
    sources = [leg for leg in legs if leg.get("role") == "source"]
    sinks = [leg for leg in legs if leg.get("role") != "source"]
    if str(component.get("conservation_mode") or "quantity") != "quantity":
        if len(sources) != 1 or len(sinks) != 1:
            raise _ProjectionError(
                "custody_component_allocation_required",
                "multi-leg conversion components require explicit source-to-sink allocations",
            )
    if len(sources) == 1:
        source = sources[0]
        conversion = str(component.get("conservation_mode") or "quantity") != "quantity"
        return [
            {
                "id": f"inferred:{index}",
                "ordinal": index,
                "source_leg_id": source["id"],
                "sink_leg_id": sink["id"],
                # Quantity-mode one-to-many flows allocate the sink quantity
                # from their single source.  A reviewed conversion is allowed
                # to have unlike source/sink quantities, but only in the
                # unambiguous 1:1 shape checked above; preserve both authored
                # quantities instead of silently rewriting the source to the
                # destination amount.
                "source_amount_msat": (
                    int(source["amount_msat"])
                    if conversion
                    else int(sink["amount_msat"])
                ),
                "sink_amount_msat": int(sink["amount_msat"]),
            }
            for index, sink in enumerate(sinks)
        ]
    if len(sinks) == 1 and sinks[0].get("role") in OWNED_SINK_ROLES:
        sink = sinks[0]
        return [
            {
                "id": f"inferred:{index}",
                "ordinal": index,
                "source_leg_id": source["id"],
                "sink_leg_id": sink["id"],
                "source_amount_msat": int(source["amount_msat"]),
                "sink_amount_msat": int(source["amount_msat"]),
            }
            for index, source in enumerate(sources)
        ]
    raise _ProjectionError(
        "custody_component_allocation_required",
        "N:M custody components require explicit source-to-sink allocations",
    )


def _anchor_rate(row: Mapping[str, Any]) -> Decimal | None:
    raw = _field(row, "fiat_rate_exact")
    if raw in (None, ""):
        raw = _field(row, "fiat_rate")
    if raw not in (None, ""):
        try:
            rate = Decimal(str(raw))
        except (InvalidOperation, ValueError):
            rate = Decimal("0")
        if rate > 0:
            return rate
    raw_value = _field(row, "fiat_value_exact")
    if raw_value in (None, ""):
        raw_value = _field(row, "fiat_value")
    amount = int(_field(row, "amount") or 0)
    if raw_value not in (None, "") and amount > 0:
        try:
            value = Decimal(str(raw_value))
        except (InvalidOperation, ValueError):
            return None
        if value > 0:
            return value / (Decimal(amount) / Decimal(MSAT_PER_BTC))
    return None


def _best_anchor(
    preferred_ids: Sequence[str | None],
    anchor_ids: Sequence[str],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> tuple[str, Mapping[str, Any]]:
    candidates = [str(value) for value in preferred_ids if value not in (None, "")]
    candidates.extend(anchor_ids)
    for transaction_id in candidates:
        row = rows_by_id.get(transaction_id)
        if row is not None:
            return transaction_id, row
    raise _ProjectionError(
        "custody_component_anchor_missing",
        "no real imported anchor is available for a synthetic journal row",
    )


def _leg_fiat_value(
    profile: Mapping[str, Any],
    leg: Mapping[str, Any],
    amount_msat: int,
) -> Decimal | None:
    """Project an exact authored valuation when its unit is profile fiat.

    Component valuations are also allowed to use future/non-fiat units for
    conservation evidence. Only explicit major (``eur``) and minor
    (``eur-cent`` / ``eur-minor``) units are safe to turn into journal fiat;
    every other unit remains audit evidence and the normal pricing gate applies.
    """

    valuation_amount = leg.get("valuation_amount")
    valuation_unit = str(leg.get("valuation_unit") or "").strip().lower()
    leg_amount = int(leg.get("amount_msat") or 0)
    currency = str(_field(profile, "fiat_currency", "") or "").strip().lower()
    if valuation_amount is None or not valuation_unit or leg_amount <= 0 or not currency:
        return None
    scale: Decimal | None
    if valuation_unit == currency:
        scale = Decimal("1")
    elif valuation_unit in {f"{currency}-cent", f"{currency}-minor"}:
        scale = Decimal("100")
    else:
        return None
    return (
        Decimal(int(valuation_amount))
        * Decimal(amount_msat)
        / Decimal(leg_amount)
        / scale
    )


def _synthetic_row(
    *,
    profile: Mapping[str, Any],
    component: Mapping[str, Any],
    leg: Mapping[str, Any],
    wallet_ref: Mapping[str, Any],
    anchor_id: str,
    anchor_row: Mapping[str, Any],
    anchor_ids: tuple[str, ...],
    row_id: str,
    direction: str,
    amount_msat: int,
    fee_msat: int = 0,
    kind: str = "custody_component_transfer",
) -> dict[str, Any]:
    row = dict(anchor_row)
    occurred_at = leg.get("occurred_at") or _field(anchor_row, "occurred_at")
    if not occurred_at:
        raise _ProjectionError(
            "custody_component_occurred_at_missing",
            "a synthetic custody leg has no occurrence time",
            leg_id=leg.get("id"),
        )
    fiat_value = _leg_fiat_value(profile, leg, amount_msat)
    rate = None
    if fiat_value is not None and amount_msat > 0:
        rate = fiat_value / (Decimal(amount_msat) / Decimal(MSAT_PER_BTC))
    else:
        rate = _anchor_rate(anchor_row)
        if rate is not None and amount_msat > 0:
            fiat_value = rate * (Decimal(amount_msat) / Decimal(MSAT_PER_BTC))
    component_id = str(component["id"])
    row.update(
        {
            "id": row_id,
            "fingerprint": row_id,
            "external_id": f"custody:{component_id}",
            "wallet_id": wallet_ref["id"],
            "wallet_label": wallet_ref["label"],
            "wallet_kind": wallet_ref.get("kind") or "untracked",
            "wallet_account_id": wallet_ref.get("wallet_account_id"),
            "account_code": wallet_ref.get("account_code") or "treasury",
            "account_label": wallet_ref.get("account_label") or "Treasury",
            "config_json": "{}",
            "occurred_at": str(occurred_at),
            "confirmed_at": str(occurred_at),
            "direction": direction,
            "asset": leg["asset"],
            "amount": int(amount_msat),
            "fee": int(fee_msat),
            "amount_includes_fee": 0,
            "fiat_rate": None if rate is None else float(rate),
            "fiat_rate_exact": None if rate is None else format(rate, "f"),
            "fiat_value": None if fiat_value is None else float(fiat_value),
            "fiat_value_exact": None if fiat_value is None else format(fiat_value, "f"),
            "kind": kind,
            "description": component.get("notes") or f"Custody component {component_id}",
            "note": component.get("notes"),
            "counterparty": None,
            "excluded": 0,
            "raw_json": "{}",
            "payment_hash": None,
            "payment_hash_source": None,
            "swap_refund_funding_txid": None,
            "privacy_boundary": None,
            "journal_transaction_id": anchor_id,
            "custody_component_id": component_id,
            "custody_component_anchor_ids": anchor_ids,
            "custody_component_leg_id": str(leg["id"]),
            "custody_component_valuation_unit": leg.get("valuation_unit"),
            "custody_component_valuation_amount": leg.get("valuation_amount"),
            "custody_component_force_block": None,
            "created_at": component.get("activated_at") or component.get("created_at") or str(occurred_at),
        }
    )
    return row


def _wallet(
    leg: Mapping[str, Any],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
) -> Mapping[str, Any]:
    wallet_id = leg.get("wallet_id")
    if wallet_id in (None, ""):
        raise _ProjectionError(
            "custody_component_wallet_missing",
            "owned custody source/destination legs require a wallet",
            leg_id=leg.get("id"),
            role=leg.get("role"),
        )
    wallet = wallet_refs_by_id.get(str(wallet_id))
    if wallet is None:
        raise _ProjectionError(
            "custody_component_wallet_unavailable",
            "a custody leg references a wallet outside the journal profile",
            leg_id=leg.get("id"),
            wallet_id=wallet_id,
        )
    return wallet


def _tax_pair_policy(
    profile: Mapping[str, Any],
    component: Mapping[str, Any],
    source: Mapping[str, Any],
    sink: Mapping[str, Any],
) -> str:
    """Classify a proven custody edge after country-neutral projection."""
    if source["asset"] == sink["asset"]:
        return "carrying-value"
    recommended = recommended_pair_policy(profile, source["asset"], sink["asset"])
    policy = str(component.get("conversion_policy") or recommended)
    if policy == "carrying-value" and recommended != "carrying-value":
        return "taxable"
    return policy


def _project_component(
    profile: Mapping[str, Any],
    component: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
) -> tuple[list[Mapping[str, Any]], list[Mapping[str, Any]]]:
    _validate_anchor_coverage(component, rows_by_id)
    value_only_losses = [
        str(leg["id"])
        for leg in component.get("legs", ())
        if leg.get("role") in ATTRIBUTED_SINK_ROLES
        and int(leg.get("amount_msat") or 0) == 0
        and int(leg.get("valuation_amount") or 0) > 0
    ]
    if value_only_losses:
        raise _ProjectionError(
            "custody_component_value_only_loss_unsupported",
            "a fiat-only fee/external loss needs an explicit priced transaction leg",
            leg_ids=value_only_losses,
        )
    anchor_ids = _component_anchor_ids(component)
    legs_by_id = {str(leg["id"]): leg for leg in component.get("legs", ())}
    allocations = _inferred_allocations(component)
    material = []
    fee_by_source: dict[str, int] = {}
    for allocation in allocations:
        source = legs_by_id.get(str(allocation.get("source_leg_id")))
        sink = legs_by_id.get(str(allocation.get("sink_leg_id")))
        if source is None or sink is None or source.get("role") != "source":
            raise _ProjectionError(
                "custody_component_allocation_invalid",
                "a custody allocation references incompatible legs",
                allocation_id=allocation.get("id"),
            )
        if sink.get("role") == "unresolved":
            raise _ProjectionError(
                "custody_component_unresolved_value",
                "an effective custody component still has unresolved value",
                allocation_id=allocation.get("id"),
            )
        source_amount = int(allocation.get("source_amount_msat") or 0)
        sink_amount = int(allocation.get("sink_amount_msat") or 0)
        if source_amount <= 0 and sink_amount <= 0:
            continue
        if sink.get("role") == "fee":
            if str(component.get("conservation_mode") or "quantity") == "conversion":
                # Conversion fees are carried on the synthetic source row.
                # The validator requires the fee sink to name exactly that
                # source quantity and its proportional valuation; retain the
                # same checks here so a forged/stale effective payload cannot
                # silently project different authored economics.
                if source_amount != sink_amount:
                    raise _ProjectionError(
                        "conversion_fee_quantity_mismatch",
                        "a conversion fee must equal the source quantity allocated to it",
                        allocation_id=allocation.get("id"),
                        source_amount_msat=source_amount,
                        fee_amount_msat=sink_amount,
                    )
                source_valuation = source.get("valuation_amount")
                fee_valuation = sink.get("valuation_amount")
                source_leg_amount = int(source.get("amount_msat") or 0)
                if (
                    source_valuation is not None
                    and fee_valuation is not None
                    and source.get("valuation_unit") == sink.get("valuation_unit")
                    and source_leg_amount > 0
                    and int(fee_valuation) * source_leg_amount
                    != int(source_valuation) * source_amount
                ):
                    raise _ProjectionError(
                        "conversion_fee_valuation_mismatch",
                        "a conversion fee valuation must match its source allocation",
                        allocation_id=allocation.get("id"),
                    )
            fee_by_source[str(source["id"])] = (
                fee_by_source.get(str(source["id"]), 0) + source_amount
            )
            continue
        material.append((allocation, source, sink, source_amount, sink_amount))

    rows: list[Mapping[str, Any]] = []
    pairs: list[Mapping[str, Any]] = []
    by_source: dict[str, list[int]] = {}
    for index, (_allocation, source, _sink, source_amount, _sink_amount) in enumerate(material):
        by_source.setdefault(str(source["id"]), []).append(index)
    fee_parts: dict[int, int] = {}
    for source_id, indexes in by_source.items():
        allocated = allocate_fee_msat(
            fee_by_source.get(source_id, 0),
            [material[index][3] for index in indexes],
        )
        fee_parts.update(zip(indexes, allocated))
    orphan_fee_sources = sorted(set(fee_by_source) - set(by_source))
    if orphan_fee_sources:
        raise _ProjectionError(
            "custody_component_fee_orphaned",
            "a component fee has no owned transfer or external disposal to carry it",
            source_leg_ids=orphan_fee_sources,
        )

    # A custody component is one atomic review decision, but it can describe a
    # chronological route through several wallets.  The tax normalizer's
    # ``group_id`` means "simultaneous N:M flow" and must therefore not be the
    # whole component id: collapsing A -> gap -> B into one group lets the
    # later hop try to spend the gap wallet before the earlier MOVE credits it.
    # Connected allocation edges (shared source or shared sink leg) form one
    # simultaneous stage; distinct transaction-less in/out legs deliberately
    # separate successive stages even when they reference the same wallet.
    parents = list(range(len(material)))

    def find(value: int) -> int:
        while parents[value] != value:
            parents[value] = parents[parents[value]]
            value = parents[value]
        return value

    def union(left: int, right: int) -> None:
        left_root = find(left)
        right_root = find(right)
        if left_root != right_root:
            parents[right_root] = left_root

    indexes_by_leg: dict[str, list[int]] = {}
    for index, (_allocation, source, sink, _source_amount, _sink_amount) in enumerate(material):
        for leg_id in (str(source["id"]), str(sink["id"])):
            indexes_by_leg.setdefault(leg_id, []).append(index)
    for indexes in indexes_by_leg.values():
        for other in indexes[1:]:
            union(indexes[0], other)
    stage_roots = sorted({find(index) for index in range(len(material))})
    stage_ordinal = {root: ordinal for ordinal, root in enumerate(stage_roots)}
    stage_group_by_index = {
        index: f"custody-flow:{component['id']}:{stage_ordinal[find(index)]}"
        for index in range(len(material))
    }

    for index, (allocation, source, sink, source_amount, sink_amount) in enumerate(material):
        source_wallet = _wallet(source, wallet_refs_by_id)
        source_anchor_id, source_anchor = _best_anchor(
            (source.get("transaction_id"), sink.get("transaction_id")),
            anchor_ids,
            rows_by_id,
        )
        out_id = f"custody:{component['id']}:allocation:{int(allocation.get('ordinal') or index)}:out"
        out_row = _synthetic_row(
            profile=profile,
            component=component,
            leg=source,
            wallet_ref=source_wallet,
            anchor_id=source_anchor_id,
            anchor_row=source_anchor,
            anchor_ids=anchor_ids,
            row_id=out_id,
            direction="outbound",
            amount_msat=source_amount,
            fee_msat=fee_parts.get(index, 0),
            kind=(
                "custody_component_external"
                if sink.get("role") == "external"
                else "custody_component_transfer"
            ),
        )
        rows.append(out_row)
        if sink.get("role") == "external":
            continue
        if sink.get("role") not in OWNED_SINK_ROLES:
            raise _ProjectionError(
                "custody_component_sink_role_invalid",
                "a custody allocation has an unsupported sink role",
                role=sink.get("role"),
                allocation_id=allocation.get("id"),
            )
        sink_wallet = _wallet(sink, wallet_refs_by_id)
        sink_anchor_id, sink_anchor = _best_anchor(
            (sink.get("transaction_id"), source.get("transaction_id")),
            anchor_ids,
            rows_by_id,
        )
        in_id = f"custody:{component['id']}:allocation:{int(allocation.get('ordinal') or index)}:in"
        in_row = _synthetic_row(
            profile=profile,
            component=component,
            leg=sink,
            wallet_ref=sink_wallet,
            anchor_id=sink_anchor_id,
            anchor_row=sink_anchor,
            anchor_ids=anchor_ids,
            row_id=in_id,
            direction="inbound",
            amount_msat=sink_amount,
        )
        rows.append(in_row)
        pairs.append(
            {
                "id": f"custody:{component['id']}:pair:{int(allocation.get('ordinal') or index)}",
                "out_transaction_id": out_id,
                "in_transaction_id": in_id,
                "kind": component.get("component_type") or "manual_bridge",
                "policy": _tax_pair_policy(profile, component, source, sink),
                "notes": component.get("notes"),
                "pair_source": "custody_component",
                "component_id": str(component["id"]),
                "group_id": stage_group_by_index[index],
                "created_at": component.get("activated_at") or component.get("created_at"),
                "deleted_at": None,
            }
        )
    return rows, pairs


def project_effective_components(
    profile: Mapping[str, Any],
    rows: Sequence[Mapping[str, Any]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    manual_pair_records: Sequence[Mapping[str, Any]],
    components: Sequence[Mapping[str, Any]],
) -> CustodyJournalProjection:
    """Atomically interpret every authored-active custody component.

    Effective components replace their raw anchors with synthetic custody
    rows.  An authored-active component that is locally incomplete, invalid,
    or conflicting still claims every transaction anchor currently known for
    it, but emits a component-wide quarantine instead.  This fail-closed split
    is essential during row-wise replication: raw evidence must never become
    taxable merely because only half of an active interpretation has arrived.
    """

    rows_by_id = {str(row["id"]): row for row in rows}
    authored_active = [
        component
        for component in components
        if _authored_active(component)
    ]
    claimed_by: dict[str, list[str]] = {}
    for component in authored_active:
        for transaction_id in _component_anchor_ids(component):
            claimed_by.setdefault(transaction_id, []).append(str(component["id"]))
    conflicts = {
        transaction_id: sorted(component_ids)
        for transaction_id, component_ids in claimed_by.items()
        if len(component_ids) > 1
    }
    claimed_ids = frozenset(claimed_by)
    projected_rows: list[Mapping[str, Any]] = [
        row for row in rows if str(row["id"]) not in claimed_ids
    ]
    surviving_pairs: list[Mapping[str, Any]] = [
        record
        for record in manual_pair_records
        if str(record["out_transaction_id"]) not in claimed_ids
        and str(record["in_transaction_id"]) not in claimed_ids
    ]
    blockers: list[CustodyComponentBlocker] = []
    quarantines: list[dict[str, Any]] = []
    for component in sorted(
        authored_active,
        key=lambda item: (str(item.get("activated_at") or ""), str(item["id"])),
    ):
        component_id = str(component["id"])
        anchor_ids = _component_anchor_ids(component)
        conflict_rows = [
            {"transaction_id": transaction_id, "component_ids": conflicts[transaction_id]}
            for transaction_id in anchor_ids
            if transaction_id in conflicts
        ]
        try:
            if conflict_rows:
                raise _ProjectionError(
                    "custody_component_membership_conflict",
                    "a transaction is claimed by more than one authored active custody component",
                    conflicts=conflict_rows,
                )
            if not _effective_active(component):
                raise _ProjectionError(
                    "custody_component_authored_active_invalid",
                    (
                        "an authored active custody component is locally incomplete "
                        "or invalid; correct it or supersede the active revision"
                    ),
                    **_inactive_authored_details(component),
                )
            component_rows, component_pairs = _project_component(
                profile, component, rows_by_id, wallet_refs_by_id
            )
        except _ProjectionError as exc:
            blocker = CustodyComponentBlocker(
                component_id=component_id,
                code=exc.code,
                message=exc.message,
                transaction_ids=anchor_ids,
                details={"component_anchor_ids": anchor_ids, **exc.details},
                quarantine_transaction_ids=tuple(
                    transaction_id
                    for transaction_id in anchor_ids
                    if transaction_id in rows_by_id
                ),
            )
            blockers.append(blocker)
            quarantines.extend(blocker.quarantines(profile))
            continue
        projected_rows.extend(component_rows)
        surviving_pairs.extend(component_pairs)

    projected_rows.sort(
        key=lambda row: (
            str(_field(row, "occurred_at") or ""),
            str(_field(row, "created_at") or ""),
            str(row["id"]),
        )
    )
    blocker_by_component_id = {
        blocker.component_id: blocker for blocker in blockers
    }
    component_by_id = {
        str(component["id"]): component for component in authored_active
    }
    blocked_anchor_rows: list[Mapping[str, Any]] = []
    for transaction_id, claimant_ids in sorted(claimed_by.items()):
        blocked_claimants = sorted(
            component_id
            for component_id in claimant_ids
            if component_id in blocker_by_component_id
        )
        raw_anchor = rows_by_id.get(transaction_id)
        if not blocked_claimants or raw_anchor is None:
            continue
        all_component_anchors = sorted(
            {
                anchor_id
                for component_id in blocked_claimants
                for anchor_id in _component_anchor_ids(
                    component_by_id[component_id]
                )
            }
        )
        blocker_codes = sorted(
            {
                blocker_by_component_id[component_id].code
                for component_id in blocked_claimants
            }
        )
        blocked = dict(raw_anchor)
        blocked.update(
            {
                "journal_transaction_id": transaction_id,
                "custody_component_id": blocked_claimants[0],
                "custody_component_anchor_ids": tuple(all_component_anchors),
                "custody_component_leg_id": None,
                "custody_component_force_block": ",".join(blocker_codes),
            }
        )
        blocked_anchor_rows.append(blocked)
    blocked_anchor_rows.sort(
        key=lambda row: (
            str(_field(row, "occurred_at") or ""),
            str(_field(row, "created_at") or ""),
            str(row["id"]),
        )
    )
    return CustodyJournalProjection(
        rows=tuple(projected_rows),
        blocked_anchor_rows=tuple(blocked_anchor_rows),
        manual_pair_records=tuple(surviving_pairs),
        quarantines=tuple(quarantines),
        blockers=tuple(blockers),
        claimed_transaction_ids=claimed_ids,
    )


def failed_component_ids(
    rows: Sequence[Mapping[str, Any]],
    quarantines: Sequence[Mapping[str, Any]],
) -> set[str]:
    """Return component ids whose synthetic members produced a quarantine."""

    quarantined_anchor_ids = {str(item["transaction_id"]) for item in quarantines}
    failed: set[str] = set()
    for row in rows:
        component_id = _field(row, "custody_component_id")
        if not component_id:
            continue
        anchor_ids = _field(row, "custody_component_anchor_ids") or ()
        if any(str(anchor_id) in quarantined_anchor_ids for anchor_id in anchor_ids):
            failed.add(str(component_id))
    return failed


def force_block_components(
    rows: Sequence[Mapping[str, Any]],
    component_ids: set[str],
    *,
    reason: str,
) -> list[Mapping[str, Any]]:
    """Mark every synthetic row in failed components for normalization-time deferral."""

    result: list[Mapping[str, Any]] = []
    for row in rows:
        if str(_field(row, "custody_component_id") or "") not in component_ids:
            result.append(row)
            continue
        forced = dict(row)
        forced["custody_component_force_block"] = reason
        result.append(forced)
    return result


def component_member_row_ids(
    rows: Sequence[Mapping[str, Any]], component_ids: set[str]
) -> set[str]:
    return {
        str(row["id"])
        for row in rows
        if str(_field(row, "custody_component_id") or "") in component_ids
    }


__all__ = [
    "CustodyComponentBlocker",
    "CustodyJournalProjection",
    "component_member_row_ids",
    "failed_component_ids",
    "force_block_components",
    "project_effective_components",
]
