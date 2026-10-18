"""The ledger response reader (schema version 2, plan 20 phase 4).

The engine answers a ``ledger`` request with exact values
(``tax-engine/core/src/ledger/response.rs``). This module renders them into
the shapes the phase-3 adapter returned (``TaxEngineLedgerResult``), and owns
everything about those shapes that is not tax logic (design §1.1, §2): the
legacy floats, ``json.dumps`` of quarantine details, wallet references looked
up by label, the two-level holdings accumulation, and the pass-through
projection metadata.

A quarantine's detail is typed: the engine names the template that built it
(``detail_kind``) and this reader adds the row facts that template read, by
kind and never by reason, since one reason can come from two templates
(``DETAIL_ENRICHMENT``). A float the adapter wrote arrives as
``{"float": "<canonical>"}``.

A failure names its stage; ``errors.ledger_error`` raises what the adapter
raised there.

Journal entries come in the adapter's order and with its keys
(``_append_rp2_journal_entries``): the engine gives each entry's type, row,
wallet label, exact values, and description, and a realized entry's
fragments with their event's and lot's values; this reader adds the uuid,
the profile's ids, the wallet's and account's ids, the transaction id and
time of the entry's row, the Austrian Kennzahl of its category, and the
``calculation`` metadata, whose fragments carry the row facts of
``explanation.gain_fragment``. A transfer leg is rendered from its audit
row, with the adapter's float quantities.
"""

from __future__ import annotations

import json
import uuid
from collections import defaultdict
from decimal import Decimal
from typing import Any, Callable, Mapping, Optional, Sequence

from ...errors import AppError
from ...msat import btc_to_msat, dec
from .. import pricing
from ..austrian import kennzahl_for_disposal_category
from ..privacy_hops import privacy_hop_evidence_from_row
from ..tax_events import build_tax_quarantine
from .base import TaxEngineLedgerInputs, TaxEngineLedgerResult
from .errors import EngineContractError, ledger_error
from .ledger_request import SCHEMA_VERSION, decode_decimal, pair_group_id, timestamp_fields

# The engine identity each realized entry's calculation metadata records
# (the name the phase-3 adapter gave the engine).
CALCULATION_ENGINE = "kassiber_tax"

# The row fields a fragment's event and lot copy into ``pricing``
# (``explanation._source``).
FRAGMENT_PRICING_FIELDS = (
    "fiat_currency",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_source_kind",
    "pricing_quality",
    "fiat_price_source",
    "pricing_timestamp",
    "pricing_provider",
    "pricing_pair",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
)

# The row fields ``_pricing_review_detail`` copied into its detail.
PRICING_REVIEW_FIELDS = (
    "pricing_quality",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_granularity",
    "pricing_method",
)


def _get(row: Any, key: str, default: Any = None) -> Any:
    if row is None:
        return default
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def _pricing_review_facts(row: Mapping[str, Any]) -> dict[str, Any]:
    return {field: _get(row, field) for field in PRICING_REVIEW_FIELDS}


def _privacy_hop_facts(row: Mapping[str, Any]) -> dict[str, Any]:
    return dict(privacy_hop_evidence_from_row(row) or {})


def _hold_facts(row: Mapping[str, Any]) -> dict[str, Any]:
    return {"external_id": str(_get(row, "external_id") or "")}


# What the reader adds to a quarantine's detail, by the template that built
# it (design §3.7). The pricing review's ``required_for`` and its wallet,
# asset, and direction come from the engine.
DETAIL_ENRICHMENT: dict[str, Callable[[Mapping[str, Any]], dict[str, Any]]] = {
    "pricing_review": _pricing_review_facts,
    "privacy_hop": _privacy_hop_facts,
    "hold": _hold_facts,
    "partner_leg": lambda _row: {},
    "plain": lambda _row: {},
}


def loads(text: str) -> dict[str, Any]:
    """Parse a response document."""
    document = json.loads(text)
    if document.get("schema_version") != SCHEMA_VERSION:
        raise EngineContractError("RequestError", f"unexpected engine schema_version in {text[:80]!r}")
    return document


def raise_for_error(response: Mapping[str, Any]) -> None:
    """Raise the failure a response reports, as the adapter raised it."""
    if response.get("ok") is not True:
        raise ledger_error(response.get("error") or {})


def detail_value(value: Any) -> Any:
    """A typed detail value as the adapter wrote it."""
    if isinstance(value, dict):
        if set(value) == {"float"}:
            return float(decode_decimal(value["float"]))
        return {key: detail_value(item) for key, item in value.items()}
    if isinstance(value, list):
        return [detail_value(item) for item in value]
    return value


def render_quarantine(
    profile: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
    quarantine: Mapping[str, Any],
) -> dict[str, Any]:
    """One engine quarantine as ``build_tax_quarantine`` wrote it."""
    row = rows_by_id[str(quarantine["row"])]
    enrich = DETAIL_ENRICHMENT.get(str(quarantine["detail_kind"]))
    if enrich is None:
        raise EngineContractError("RequestError", f"unknown quarantine detail kind {quarantine['detail_kind']!r}")
    detail = {key: detail_value(value) for key, value in quarantine["detail"].items()}
    detail.update(enrich(row))
    return build_tax_quarantine(profile, row, str(quarantine["reason"]), detail)


def render_audit(inputs: TaxEngineLedgerInputs, audit: Mapping[str, Any]) -> dict[str, Any]:
    """One transfer-audit row as the adapter wrote it, with its key order.

    The engine gives the move's quantities, price, Austrian regime, flows,
    and pool; the ids, anchors, wallets, time, external id, pairing source,
    and atomic group come from the move and its legs.
    """
    projection = inputs.finalized_tax_projection
    pair = projection.intra_pairs[int(audit["pair"])]
    out_row, in_row = pair["out"], pair["in"]
    from_wallet = inputs.wallet_refs_by_id[out_row["wallet_id"]]
    to_wallet = inputs.wallet_refs_by_id[in_row["wallet_id"]]
    spot_price = audit["spot_price"]
    row = {
        "out_id": out_row["id"],
        "in_id": in_row["id"],
        "out_anchor_transaction_id": str(_get(out_row, "journal_transaction_id") or out_row["id"]),
        "in_anchor_transaction_id": str(_get(in_row, "journal_transaction_id") or in_row["id"]),
        "from_wallet_id": from_wallet["id"],
        "from_wallet_label": from_wallet["label"],
        "to_wallet_id": to_wallet["id"],
        "to_wallet_label": to_wallet["label"],
        "asset": str(_get(out_row, "asset")),
        "occurred_at": out_row["occurred_at"],
        "external_id": out_row["external_id"],
        "crypto_sent": float(decode_decimal(audit["sent"])),
        "crypto_received": float(decode_decimal(audit["received"])),
        "crypto_fee": float(decode_decimal(audit["fee"])),
        "crypto_sent_msat": int(audit["sent_msat"]),
        "crypto_received_msat": int(audit["received_msat"]),
        "crypto_fee_msat": int(audit["fee_msat"]),
        "spot_price": float(decode_decimal(spot_price)) if spot_price is not None else 0.0,
        "pairing_source": pair.get("source"),
    }
    if audit["at_regime"]:
        row["at_regime"] = audit["at_regime"]
    flows = audit["regime_flows"]
    if flows:
        row["regime_flows"] = {"out": dict(flows["out"]), "in": dict(flows["in"])}
    if audit["at_pool"]:
        row["at_pool"] = audit["at_pool"]
    group_id = pair_group_id(pair)
    if group_id:
        row["transfer_group_id"] = group_id
    return row


def carried_row(row: Mapping[str, Any], value: str, rate: Optional[str]) -> dict[str, Any]:
    """``row`` with the value and rate the generic carry gave it (fixed strings)."""
    return {
        **dict(row),
        "fiat_rate": float(rate) if rate is not None else None,
        "fiat_value": value,
        "fiat_rate_exact": rate,
        "fiat_value_exact": value,
        "fiat_price_source": pricing.legacy_source_for(pricing.SOURCE_MANUAL_OVERRIDE),
        "pricing_source_kind": pricing.SOURCE_MANUAL_OVERRIDE,
        "pricing_quality": pricing.QUALITY_EXACT,
        "pricing_provider": None,
        "pricing_pair": None,
        "pricing_granularity": None,
        "pricing_method": "carrying_value",
    }


def rows_after_carry(
    rows: Sequence[Mapping[str, Any]],
    carried: Sequence[Mapping[str, Any]],
) -> dict[str, Mapping[str, Any]]:
    """The projection rows by id, as the engine saw them after the generic carry."""
    rows_by_id: dict[str, Mapping[str, Any]] = {str(row["id"]): row for row in rows}
    for item in carried:
        row_id = str(item["row"])
        rows_by_id[row_id] = carried_row(rows_by_id[row_id], item["value"], item.get("rate"))
    return rows_by_id


def render_tax_summary(row: Mapping[str, Any]) -> dict[str, Any]:
    """One tax-summary row, with the adapter's floats."""
    quantity = decode_decimal(row["quantity"])
    return {
        "year": int(row["year"]),
        "asset": row["asset"],
        "transaction_type": row["transaction_type"],
        "capital_gains_type": "long" if row["long_term"] else "short",
        "quantity": float(quantity),
        "quantity_msat": btc_to_msat(quantity),
        "proceeds": float(decode_decimal(row["proceeds"])),
        "cost_basis": float(decode_decimal(row["cost_basis"])),
        "gain_loss": float(decode_decimal(row["gain_loss"])),
    }


def _holdings_slot() -> dict[str, Decimal]:
    return {"quantity": Decimal("0"), "cost_basis": Decimal("0")}


def accumulate_holdings(
    holdings: Sequence[Mapping[str, Any]],
    wallet_refs_by_label: Mapping[str, Mapping[str, Any]],
) -> tuple[dict[tuple[Any, ...], dict[str, Decimal]], dict[tuple[Any, ...], dict[str, Decimal]]]:
    """Account and wallet holdings, summed as the adapter summed them.

    Each asset's balances are summed into that asset's totals first, and the
    asset totals into the book's, both from ``Decimal("0")``, so every
    addition is the adapter's own. A balance whose wallet label has no
    reference is skipped.
    """
    account_holdings: defaultdict[tuple[Any, ...], dict[str, Decimal]] = defaultdict(_holdings_slot)
    wallet_holdings: defaultdict[tuple[Any, ...], dict[str, Decimal]] = defaultdict(_holdings_slot)
    by_asset: dict[str, list[Mapping[str, Any]]] = {}
    for holding in holdings:
        by_asset.setdefault(str(holding["asset"]), []).append(holding)
    for asset, balances in by_asset.items():
        asset_accounts: defaultdict[tuple[Any, ...], dict[str, Decimal]] = defaultdict(_holdings_slot)
        asset_wallets: defaultdict[tuple[Any, ...], dict[str, Decimal]] = defaultdict(_holdings_slot)
        for balance in balances:
            wallet = wallet_refs_by_label.get(balance["wallet"])
            if wallet is None:
                continue
            quantity = decode_decimal(balance["quantity"])
            cost_basis = decode_decimal(balance["cost_basis"])
            account_key = (wallet["wallet_account_id"], wallet["account_code"], wallet["account_label"], asset)
            wallet_key = (wallet["id"], wallet["label"], wallet["account_code"], asset)
            asset_accounts[account_key]["quantity"] += quantity
            asset_accounts[account_key]["cost_basis"] += cost_basis
            asset_wallets[wallet_key]["quantity"] += quantity
            asset_wallets[wallet_key]["cost_basis"] += cost_basis
        for key, value in asset_accounts.items():
            account_holdings[key]["quantity"] += value["quantity"]
            account_holdings[key]["cost_basis"] += value["cost_basis"]
        for key, value in asset_wallets.items():
            wallet_holdings[key]["quantity"] += value["quantity"]
            wallet_holdings[key]["cost_basis"] += value["cost_basis"]
    return dict(account_holdings), dict(wallet_holdings)


def wallet_refs_by_label(wallet_refs_by_id: Mapping[str, Mapping[str, Any]]) -> dict[str, Mapping[str, Any]]:
    """Wallet references by label; when two share a label, the last wins."""
    return {str(item["label"]): item for item in wallet_refs_by_id.values() if item.get("label") not in (None, "")}


def direct_payout_compat_records(
    records: Sequence[Mapping[str, Any]],
    projected_rows: Sequence[Mapping[str, Any]],
) -> list[dict[str, Any]]:
    """Keep the direct-payout storage/report adapter after pre-tax cutover.

    Payout rows are compatibility metadata, not a source of tax events.  The
    finalized projection already selected the exact source slice; this adapter
    only preserves the established response/persistence shape.
    """

    projected_by_anchor: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for row in projected_rows:
        anchor = str(_get(row, "journal_transaction_id") or "")
        if anchor and _get(row, "direction") == "outbound":
            projected_by_anchor[anchor].append(row)
    result: list[dict[str, Any]] = []
    for record in records:
        payout_id = str(_get(record, "id") or "")
        out_id = str(_get(record, "out_transaction_id") or "")
        if not payout_id or not out_id:
            continue
        reviewed_amount = (
            _get(record, "out_amount")
            or _get(record, "out_amount_msat")
        )
        candidates = projected_by_anchor.get(out_id, [])
        reviewed_asset = str(_get(record, "out_asset") or "")
        if reviewed_asset:
            candidates = [
                row
                for row in candidates
                if str(_get(row, "asset") or "") == reviewed_asset
            ]
        if reviewed_amount not in (None, ""):
            candidates = [
                row
                for row in candidates
                if int(_get(row, "amount") or 0) == int(reviewed_amount)
            ]
        projected_source = candidates[0] if len(candidates) == 1 else {}
        result.append(
            {
                "payout_id": payout_id,
                "kind": _get(record, "kind"),
                "policy": _get(record, "policy"),
                "out_id": out_id,
                "out_asset": (
                    _get(record, "out_asset")
                    or _get(projected_source, "asset")
                ),
                "out_amount_msat": int(
                    _get(record, "out_amount")
                    or _get(record, "out_amount_msat")
                    or _get(projected_source, "amount")
                    or 0
                ),
                "payout_asset": _get(record, "payout_asset"),
                "payout_amount_msat": int(_get(record, "payout_amount") or 0),
                "payout_occurred_at": _get(record, "payout_occurred_at"),
                "payout_external_id": _get(record, "payout_external_id"),
                "counterparty": _get(record, "counterparty"),
                "swap_fee_msat": int(_get(record, "swap_fee_msat") or 0),
                "swap_fee_kind": _get(record, "swap_fee_kind"),
            }
        )
    return result


def empty_result(inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
    """The ledger of a projection without rows: its own quarantines and metadata."""
    projection = inputs.finalized_tax_projection
    return TaxEngineLedgerResult(
        entries=[],
        quarantines=list(projection.quarantines),
        intra_audit=[],
        cross_asset_pairs=list(projection.cross_asset_pairs),
        direct_swap_payouts=direct_payout_compat_records(inputs.direct_payout_records, projection.rows),
        tax_summary=[],
        account_holdings={},
        wallet_holdings={},
    )


def _profile_str(profile: Mapping[str, Any], key: str) -> str:
    if hasattr(profile, "keys") and key in profile.keys():
        value = profile[key]
        return "" if value is None else str(value).strip()
    return ""


def _journal_transaction_id(row: Optional[Mapping[str, Any]], fallback: str) -> str:
    return str(_get(row, "journal_transaction_id", fallback))


def _exact(value: Optional[Decimal]) -> Optional[str]:
    return None if value is None else format(value, "f")


def _optional_decimal(text: Optional[str]) -> Optional[Decimal]:
    return None if text is None else decode_decimal(text)


class _EntryReader:
    """Renders the engine's journal entries as the adapter wrote them."""

    def __init__(
        self,
        profile: Mapping[str, Any],
        rows_by_id: Mapping[str, Mapping[str, Any]],
        refs_by_label: Mapping[str, Mapping[str, Any]],
        audit: Sequence[Mapping[str, Any]],
        rendered_audit: Sequence[Mapping[str, Any]],
    ) -> None:
        self.profile = profile
        self.rows_by_id = rows_by_id
        self.refs_by_label = refs_by_label
        self.audit = audit
        self.rendered_audit = rendered_audit
        self.method = _profile_str(profile, "gains_algorithm")
        self._iso: dict[Any, str] = {}

    def wallet(self, label: str) -> Mapping[str, Any]:
        ref = self.refs_by_label.get(label)
        if ref is None:
            raise AppError(f"RP2 emitted transaction for unknown wallet '{label}'", code="internal")
        return ref

    def _head(self, wallet: Mapping[str, Any], transaction_id: str, occurred_at: Any) -> dict[str, Any]:
        return {
            "id": str(uuid.uuid4()),
            "workspace_id": self.profile["workspace_id"],
            "profile_id": self.profile["id"],
            "transaction_id": transaction_id,
            "wallet_id": wallet["id"],
            "account_id": wallet["wallet_account_id"],
            "occurred_at": occurred_at,
        }

    def iso(self, occurred_at: Any) -> str:
        """``timestamp.isoformat()`` of the entry an engine row id stands for."""
        if occurred_at not in self._iso:
            self._iso[occurred_at] = timestamp_fields(occurred_at)["iso"]
        return self._iso[occurred_at]

    def source(self, asset: str, facts: Optional[Mapping[str, Any]]) -> Optional[dict[str, Any]]:
        """A fragment's event or lot (``explanation._source``)."""
        if facts is None:
            return None
        row_id = facts["row"]
        row = self.rows_by_id.get(row_id)
        source = dict(row) if row is not None else {}
        crypto_fee = decode_decimal(facts["crypto_fee"])
        return {
            "transaction_id": source.get("journal_transaction_id") or source.get("id"),
            "engine_event_id": row_id,
            "occurred_at": self.iso(source.get("occurred_at")),
            "asset": asset,
            "spot_price_exact": _exact(decode_decimal(facts["spot_price"])),
            "fiat_fee_exact": _exact(decode_decimal(facts["fiat_fee"])),
            "crypto_fee_msat": btc_to_msat(crypto_fee),
            "crypto_fee_msat_exact": str(btc_to_msat(crypto_fee)),
            "pricing": {key: source.get(key) for key in FRAGMENT_PRICING_FIELDS},
        }

    def fragment(self, asset: str, fragment: Mapping[str, Any]) -> dict[str, Any]:
        """One gain/loss fragment (``explanation.gain_fragment``)."""
        crypto_amount = decode_decimal(fragment["crypto_amount"])
        override = _optional_decimal(fragment["unit_basis_override"])
        lot = fragment["lot"]
        crypto_in = decode_decimal(lot["crypto_in"]) if lot else None
        return {
            "quantity_msat": btc_to_msat(crypto_amount),
            "quantity_msat_exact": str(btc_to_msat(crypto_amount)),
            "cost_basis_exact": _exact(decode_decimal(fragment["cost_basis"])),
            "proceeds_exact": _exact(decode_decimal(fragment["proceeds"])),
            "gain_loss_exact": _exact(decode_decimal(fragment["gain_loss"])),
            "unit_basis_override_exact": _exact(override),
            "basis_authority": "engine_unit_override" if override is not None else "acquired_lot" if lot else "income",
            "acquisition_basis_exact": _exact(decode_decimal(lot["basis"])) if lot else None,
            "acquisition_quantity_msat": btc_to_msat(crypto_in) if lot else None,
            "acquisition_quantity_msat_exact": str(btc_to_msat(crypto_in)) if lot else None,
            "event": self.source(asset, fragment["event"]),
            "lot": self.source(asset, lot),
        }

    def acquisition(self, item: Mapping[str, Any]) -> dict[str, Any]:
        row_id = item["row"]
        row = self.rows_by_id.get(row_id)
        wallet = self.wallet(item["wallet"])
        fiat_in_with_fee = decode_decimal(item["fiat_value"])
        return {
            **self._head(wallet, _journal_transaction_id(row, row_id), row["occurred_at"]),
            "entry_type": "acquisition",
            "asset": item["asset"],
            "quantity": decode_decimal(item["quantity"]),
            "fiat_value": fiat_in_with_fee,
            "unit_cost": decode_decimal(item["unit_cost"]),
            "cost_basis": None,
            "proceeds": None,
            "gain_loss": None,
            "description": item["description"],
        }

    def realized(self, item: Mapping[str, Any]) -> dict[str, Any]:
        row_id = item["row"]
        row = self.rows_by_id.get(row_id)
        wallet = self.wallet(item["wallet"])
        proceeds = decode_decimal(item["proceeds"])
        entry = {
            **self._head(wallet, _journal_transaction_id(row, row_id), row["occurred_at"]),
            "entry_type": item["entry_type"],
            "asset": item["asset"],
            "quantity": decode_decimal(item["quantity"]),
            "fiat_value": proceeds,
            "unit_cost": Decimal("0"),
            "cost_basis": decode_decimal(item["cost_basis"]),
            "proceeds": proceeds,
            "gain_loss": decode_decimal(item["gain_loss"]),
            "description": item["description"],
        }
        category = item["at_category"]
        if category is not None:
            entry["at_category"] = category
            entry["at_kennzahl"] = kennzahl_for_disposal_category(category)
        entry["capital_gains_type"] = item["capital_gains_type"]
        entry["calculation"] = {
            "schema_version": 1,
            "engine": CALCULATION_ENGINE,
            "method": self.method,
            "fragments": [self.fragment(item["asset"], fragment) for fragment in item["fragments"]],
        }
        return entry

    def transfer_leg(self, item: Mapping[str, Any]) -> dict[str, Any]:
        """One leg of a booked move, from its audit row and its legs' rows."""
        index = int(item["audit"])
        audit = self.rendered_audit[index]
        outbound = item["entry_type"] == "transfer_out"
        # The adapter indexes the legs' wallets directly (a ``KeyError`` for
        # an unknown label).
        wallet = self.refs_by_label[audit["from_wallet_label"] if outbound else audit["to_wallet_label"]]
        row_id = audit["out_id"] if outbound else audit["in_id"]
        quantity = dec(audit["crypto_sent"] if outbound else audit["crypto_received"])
        return {
            **self._head(wallet, _journal_transaction_id(self.rows_by_id.get(row_id), row_id), audit["occurred_at"]),
            "entry_type": item["entry_type"],
            "asset": audit["asset"],
            "quantity": -quantity if outbound else quantity,
            "fiat_value": Decimal("0"),
            "unit_cost": Decimal("0"),
            "cost_basis": None,
            "proceeds": None,
            "gain_loss": None,
            "description": self.audit[index]["description"],
        }

    def entry(self, item: Mapping[str, Any]) -> dict[str, Any]:
        entry_type = item["entry_type"]
        if entry_type == "acquisition":
            return self.acquisition(item)
        if entry_type in ("transfer_out", "transfer_in"):
            return self.transfer_leg(item)
        return self.realized(item)


def read_ledger(
    profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    response: Mapping[str, Any],
) -> TaxEngineLedgerResult:
    """The ledger a ``ledger`` response describes, or the failure it reports."""
    raise_for_error(response)
    projection = inputs.finalized_tax_projection
    rows_by_id = rows_after_carry(projection.rows, response["carried_rows"])
    refs_by_label = wallet_refs_by_label(inputs.wallet_refs_by_id)
    intra_audit = [render_audit(inputs, item) for item in response["intra_audit"]]
    reader = _EntryReader(profile, rows_by_id, refs_by_label, response["intra_audit"], intra_audit)
    entries = [reader.entry(item) for item in response["entries"]]
    account_holdings, wallet_holdings = accumulate_holdings(response["holdings"], refs_by_label)
    return TaxEngineLedgerResult(
        entries=entries,
        quarantines=[
            *projection.quarantines,
            *(render_quarantine(profile, rows_by_id, item) for item in response["quarantines"]),
        ],
        intra_audit=intra_audit,
        cross_asset_pairs=list(projection.cross_asset_pairs),
        direct_swap_payouts=direct_payout_compat_records(inputs.direct_payout_records, projection.rows),
        tax_summary=[render_tax_summary(row) for row in response["tax_summary"]],
        account_holdings=account_holdings,
        wallet_holdings=wallet_holdings,
    )


__all__ = [
    "CALCULATION_ENGINE",
    "DETAIL_ENRICHMENT",
    "FRAGMENT_PRICING_FIELDS",
    "PRICING_REVIEW_FIELDS",
    "accumulate_holdings",
    "carried_row",
    "detail_value",
    "direct_payout_compat_records",
    "empty_result",
    "loads",
    "raise_for_error",
    "read_ledger",
    "render_audit",
    "render_quarantine",
    "render_tax_summary",
    "rows_after_carry",
    "wallet_refs_by_label",
]
