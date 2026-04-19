from __future__ import annotations

import uuid
from collections import defaultdict
from dataclasses import dataclass, field
from datetime import date, datetime
from decimal import Decimal
from typing import Any, Mapping, Sequence
from zoneinfo import ZoneInfo

from ...transfers import apply_manual_pairs, detect_intra_transfers
from ...util import parse_bool
from ..tax_events import (
    NormalizedTaxAssetInputs,
    NormalizedTaxEvent,
    build_tax_quarantine,
    normalize_tax_asset_inputs,
)
from .base import TaxEngineLedgerInputs, TaxEngineLedgerResult

_VIENNA = ZoneInfo("Europe/Vienna")
_ALTBESTAND_CUTOFF = date(2021, 2, 28)
_MOVING_AVERAGE_START = date(2023, 1, 1)
_ZERO = Decimal("0")
_INBOUND_KINDS_WITH_UNCLEAR_TAX_MEANING = {
    "airdrop",
    "gift",
    "gift_receive",
    "hardfork",
    "inheritance",
    "inherited_receive",
    "lightning_received",
    "mining",
    "mining_income",
    "routing_income",
    "staking",
}
_SUPPORTED_INBOUND_ACQUISITION_KINDS = {
    "buy",
}
_SUPPORTED_OUTBOUND_DISPOSAL_KINDS = {
    "sell",
    "spend",
}
_ANNOTATED_INBOUND_ACQUISITION_TYPES = {
    "receive_external",
    "mining_income",
    "routing_income",
    "staking_income",
    "airdrop",
    "hardfork",
}
_ANNOTATED_OUTBOUND_DISPOSAL_TYPES = {
    "sell",
    "spend",
}
_ANNOTATED_INCOME_TYPES = {
    "mining_income",
    "routing_income",
    "staking_income",
}
_ANNOTATED_ZERO_BASIS_TYPES = {
    "airdrop",
    "hardfork",
}


@dataclass(frozen=True)
class _AustrianLot:
    quantity: Decimal
    cost_basis: Decimal
    acquired_at: str
    acquired_at_dt: datetime


@dataclass(frozen=True)
class _ConsumedSegment:
    regime: str
    quantity: Decimal
    cost_basis: Decimal
    acquired_at: str | None = None
    acquired_at_dt: datetime | None = None


@dataclass
class _WalletAssetState:
    alt_lots: list[_AustrianLot] = field(default_factory=list)
    fifo_lots: list[_AustrianLot] = field(default_factory=list)
    avg_quantity: Decimal = field(default_factory=lambda: Decimal("0"))
    avg_cost_basis: Decimal = field(default_factory=lambda: Decimal("0"))

    def total_quantity(self) -> Decimal:
        return (
            sum((lot.quantity for lot in self.alt_lots), Decimal("0"))
            + sum((lot.quantity for lot in self.fifo_lots), Decimal("0"))
            + self.avg_quantity
        )

    def total_cost_basis(self) -> Decimal:
        return (
            sum((lot.cost_basis for lot in self.alt_lots), Decimal("0"))
            + sum((lot.cost_basis for lot in self.fifo_lots), Decimal("0"))
            + self.avg_cost_basis
        )


@dataclass(frozen=True)
class _AustrianAssetResult:
    entries: list[dict[str, Any]]
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    account_holdings: dict[tuple[Any, ...], dict[str, Any]]
    wallet_holdings: dict[tuple[Any, ...], dict[str, Any]]


def _parse_timestamp(value: str) -> datetime:
    return datetime.fromisoformat(str(value).replace("Z", "+00:00"))


def _local_date(value: str) -> date:
    return _parse_timestamp(value).astimezone(_VIENNA).date()


def _wallet_is_altbestand(wallet: Mapping[str, Any]) -> bool:
    return parse_bool(wallet.get("altbestand"), default=False)


def _kind_text(row: Mapping[str, Any]) -> str:
    if hasattr(row, "keys") and "kind" in row.keys():
        return str(row["kind"] or "").strip().lower()
    return str(getattr(row, "kind", "") or "").strip().lower()


def _annotation_text(event: NormalizedTaxEvent) -> str:
    return str(event.tax_event_type or "").strip().lower()


def _effective_event_kind(event: NormalizedTaxEvent) -> str:
    return _annotation_text(event) or _kind_text(event.raw_row)


def _regime_label(regime: str, disposed_at_dt: datetime | None = None, acquired_at_dt: datetime | None = None) -> str:
    if regime == "altbestand":
        if disposed_at_dt is not None and acquired_at_dt is not None and _is_altbestand_tax_free(acquired_at_dt, disposed_at_dt):
            return "Altbestand tax-free"
        if disposed_at_dt is not None and acquired_at_dt is not None:
            return "Altbestand speculative"
        return "Altbestand"
    if regime == "neuvermoegen_fifo":
        return "Neuvermoegen FIFO"
    return "Neuvermoegen moving average"


def _with_regime_note(description: str, regime: str, disposed_at_dt: datetime | None = None, acquired_at_dt: datetime | None = None) -> str:
    return f"{description} [{_regime_label(regime, disposed_at_dt, acquired_at_dt)}]"


def _is_altbestand_tax_free(acquired_at_dt: datetime, disposed_at_dt: datetime) -> bool:
    return (_local_date(disposed_at_dt.isoformat()) - _local_date(acquired_at_dt.isoformat())).days > 365


def _classify_acquisition(wallet: Mapping[str, Any], occurred_at: str) -> str:
    if _wallet_is_altbestand(wallet):
        return "altbestand"
    occurred_local = _local_date(occurred_at)
    if occurred_local <= _ALTBESTAND_CUTOFF:
        return "altbestand"
    if occurred_local < _MOVING_AVERAGE_START:
        return "neuvermoegen_fifo"
    return "neuvermoegen_avg"


def _insert_lot(lots: list[_AustrianLot], lot: _AustrianLot) -> None:
    lots.append(lot)
    lots.sort(key=lambda item: (item.acquired_at_dt, item.acquired_at))


def _add_segment_to_wallet(state: _WalletAssetState, segment: _ConsumedSegment) -> None:
    if segment.quantity <= 0:
        return
    if segment.regime == "altbestand":
        _insert_lot(
            state.alt_lots,
            _AustrianLot(
                quantity=segment.quantity,
                cost_basis=segment.cost_basis,
                acquired_at=segment.acquired_at or "",
                acquired_at_dt=segment.acquired_at_dt or _parse_timestamp(segment.acquired_at or "1970-01-01T00:00:00+00:00"),
            ),
        )
        return
    if segment.regime == "neuvermoegen_fifo":
        _insert_lot(
            state.fifo_lots,
            _AustrianLot(
                quantity=segment.quantity,
                cost_basis=segment.cost_basis,
                acquired_at=segment.acquired_at or "",
                acquired_at_dt=segment.acquired_at_dt or _parse_timestamp(segment.acquired_at or "1970-01-01T00:00:00+00:00"),
            ),
        )
        return
    state.avg_quantity += segment.quantity
    state.avg_cost_basis += segment.cost_basis


def _add_acquisition_to_wallet(
    state: _WalletAssetState,
    wallet: Mapping[str, Any],
    occurred_at: str,
    quantity: Decimal,
    cost_basis: Decimal,
) -> str:
    regime = _classify_acquisition(wallet, occurred_at)
    if regime == "altbestand":
        _insert_lot(
            state.alt_lots,
            _AustrianLot(
                quantity=quantity,
                cost_basis=cost_basis,
                acquired_at=occurred_at,
                acquired_at_dt=_parse_timestamp(occurred_at),
            ),
        )
    elif regime == "neuvermoegen_fifo":
        _insert_lot(
            state.fifo_lots,
            _AustrianLot(
                quantity=quantity,
                cost_basis=cost_basis,
                acquired_at=occurred_at,
                acquired_at_dt=_parse_timestamp(occurred_at),
            ),
        )
    else:
        state.avg_quantity += quantity
        state.avg_cost_basis += cost_basis
    return regime


def _consume_lots(lots: list[_AustrianLot], quantity: Decimal, regime: str) -> tuple[Decimal, list[_ConsumedSegment]]:
    remaining = quantity
    consumed: list[_ConsumedSegment] = []
    while remaining > 0 and lots:
        lot = lots[0]
        take = min(lot.quantity, remaining)
        cost_basis = (lot.cost_basis * take / lot.quantity) if lot.quantity else _ZERO
        consumed.append(
            _ConsumedSegment(
                regime=regime,
                quantity=take,
                cost_basis=cost_basis,
                acquired_at=lot.acquired_at,
                acquired_at_dt=lot.acquired_at_dt,
            )
        )
        remaining -= take
        if take == lot.quantity:
            lots.pop(0)
        else:
            lots[0] = _AustrianLot(
                quantity=lot.quantity - take,
                cost_basis=lot.cost_basis - cost_basis,
                acquired_at=lot.acquired_at,
                acquired_at_dt=lot.acquired_at_dt,
            )
    return remaining, consumed


def _consume_segments(state: _WalletAssetState, quantity: Decimal) -> list[_ConsumedSegment]:
    if quantity <= 0:
        return []
    remaining = quantity
    consumed: list[_ConsumedSegment] = []
    remaining, lot_segments = _consume_lots(state.alt_lots, remaining, "altbestand")
    consumed.extend(lot_segments)
    remaining, lot_segments = _consume_lots(state.fifo_lots, remaining, "neuvermoegen_fifo")
    consumed.extend(lot_segments)
    if remaining > 0 and state.avg_quantity > 0:
        take = min(state.avg_quantity, remaining)
        avg_unit_cost = state.avg_cost_basis / state.avg_quantity if state.avg_quantity else _ZERO
        cost_basis = avg_unit_cost * take
        state.avg_quantity -= take
        state.avg_cost_basis -= cost_basis
        if state.avg_quantity == 0:
            state.avg_cost_basis = _ZERO
        consumed.append(
            _ConsumedSegment(
                regime="neuvermoegen_avg",
                quantity=take,
                cost_basis=cost_basis,
            )
        )
        remaining -= take
    if remaining > 0:
        raise ValueError("insufficient lots")
    return consumed


def _split_segments(segments: list[_ConsumedSegment], first_quantity: Decimal) -> tuple[list[_ConsumedSegment], list[_ConsumedSegment]]:
    remaining = first_quantity
    first: list[_ConsumedSegment] = []
    second: list[_ConsumedSegment] = []
    for segment in segments:
        if remaining <= 0:
            second.append(segment)
            continue
        take = min(segment.quantity, remaining)
        if take > 0:
            taken_segment = _ConsumedSegment(
                regime=segment.regime,
                quantity=take,
                cost_basis=(segment.cost_basis * take / segment.quantity) if segment.quantity else _ZERO,
                acquired_at=segment.acquired_at,
                acquired_at_dt=segment.acquired_at_dt,
            )
            first.append(taken_segment)
            remaining -= take
        leftover = segment.quantity - take
        if leftover > 0:
            second.append(
                _ConsumedSegment(
                    regime=segment.regime,
                    quantity=leftover,
                    cost_basis=segment.cost_basis - taken_segment.cost_basis,
                    acquired_at=segment.acquired_at,
                    acquired_at_dt=segment.acquired_at_dt,
                )
            )
    return first, second


def _journal_entry(
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    *,
    transaction_id: str,
    occurred_at: str,
    entry_type: str,
    asset: str,
    quantity: Decimal,
    fiat_value: Decimal,
    unit_cost: Decimal,
    cost_basis: Decimal | None,
    proceeds: Decimal | None,
    gain_loss: Decimal | None,
    description: str,
) -> dict[str, Any]:
    return {
        "id": str(uuid.uuid4()),
        "workspace_id": profile["workspace_id"],
        "profile_id": profile["id"],
        "transaction_id": transaction_id,
        "wallet_id": wallet["id"],
        "account_id": wallet["wallet_account_id"],
        "occurred_at": occurred_at,
        "entry_type": entry_type,
        "asset": asset,
        "quantity": quantity,
        "fiat_value": fiat_value,
        "unit_cost": unit_cost,
        "cost_basis": cost_basis,
        "proceeds": proceeds,
        "gain_loss": gain_loss,
        "description": description,
    }


def _append_realized_entries(
    entries: list[dict[str, Any]],
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    *,
    transaction_id: str,
    occurred_at: str,
    entry_type: str,
    asset: str,
    total_proceeds: Decimal,
    description: str,
    segments: list[_ConsumedSegment],
) -> None:
    total_quantity = sum((segment.quantity for segment in segments), Decimal("0"))
    occurred_at_dt = _parse_timestamp(occurred_at)
    for segment in segments:
        proceeds = (total_proceeds * segment.quantity / total_quantity) if total_quantity else _ZERO
        cost_basis = segment.cost_basis
        gain_loss = proceeds - cost_basis
        entry_description = _with_regime_note(description, segment.regime, occurred_at_dt, segment.acquired_at_dt)
        if segment.regime == "altbestand" and segment.acquired_at_dt is not None:
            if _is_altbestand_tax_free(segment.acquired_at_dt, occurred_at_dt):
                entry_description = _with_regime_note(description, segment.regime, occurred_at_dt, segment.acquired_at_dt)
                if entry_type == "disposal":
                    cost_basis = proceeds
                else:
                    proceeds = _ZERO
                    cost_basis = _ZERO
                gain_loss = _ZERO
        entries.append(
            _journal_entry(
                profile,
                wallet,
                transaction_id=transaction_id,
                occurred_at=occurred_at,
                entry_type=entry_type,
                asset=asset,
                quantity=-segment.quantity,
                fiat_value=proceeds,
                unit_cost=_ZERO,
                cost_basis=cost_basis,
                proceeds=proceeds,
                gain_loss=gain_loss,
                description=entry_description,
            )
        )


def _holdings_for_asset(
    states_by_wallet_id: Mapping[str, _WalletAssetState],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    asset: str,
) -> tuple[dict[tuple[Any, ...], dict[str, Any]], dict[tuple[Any, ...], dict[str, Any]]]:
    account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
    wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
    for wallet_id, state in states_by_wallet_id.items():
        quantity = state.total_quantity()
        if quantity <= 0:
            continue
        cost_basis = state.total_cost_basis()
        wallet = wallet_refs_by_id[wallet_id]
        account_key = (
            wallet["wallet_account_id"],
            wallet["account_code"],
            wallet["account_label"],
            asset,
        )
        wallet_key = (
            wallet["id"],
            wallet["label"],
            wallet["account_code"],
            asset,
        )
        account_holdings[account_key]["quantity"] += quantity
        account_holdings[account_key]["cost_basis"] += cost_basis
        wallet_holdings[wallet_key]["quantity"] += quantity
        wallet_holdings[wallet_key]["cost_basis"] += cost_basis
    return dict(account_holdings), dict(wallet_holdings)


def _unsupported_inbound_kind(event: NormalizedTaxEvent) -> bool:
    annotation = _annotation_text(event)
    if annotation:
        return annotation not in _ANNOTATED_INBOUND_ACQUISITION_TYPES
    kind = _kind_text(event.raw_row)
    if kind in _SUPPORTED_INBOUND_ACQUISITION_KINDS:
        return False
    if kind in _INBOUND_KINDS_WITH_UNCLEAR_TAX_MEANING:
        return True
    return True


def _unsupported_outbound_kind(event: NormalizedTaxEvent) -> bool:
    annotation = _annotation_text(event)
    if annotation:
        return annotation not in _ANNOTATED_OUTBOUND_DISPOSAL_TYPES
    kind = _kind_text(event.raw_row)
    if kind in _SUPPORTED_OUTBOUND_DISPOSAL_KINDS:
        return False
    return True


def _allows_taxable_cross_asset_processing(
    cross_asset_pairs: Sequence[Mapping[str, Any]],
) -> bool:
    return bool(cross_asset_pairs) and all(
        str(pair.get("policy") or "").strip().lower() == "taxable"
        for pair in cross_asset_pairs
    )


class ExperimentalAustrianTaxEngine:
    """Austrian ledger engine on the shared journal seam.

    The implementation is intentionally conservative: it processes explicit
    acquisitions, disposals, and self-transfers from today's normalized tax
    inputs, while quarantining rows whose Austrian tax meaning is still not
    defensible from current provenance.
    """

    def __init__(self, profile: Mapping[str, Any]):
        self.profile = profile

    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        if not inputs.rows:
            return TaxEngineLedgerResult(
                entries=[],
                quarantines=[],
                intra_audit=[],
                cross_asset_pairs=[],
                account_holdings={},
                wallet_holdings={},
            )

        entries: list[dict[str, Any]] = []
        quarantines: list[dict[str, Any]] = []
        intra_audit_all: list[dict[str, Any]] = []
        account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})

        auto_pairs, _ = detect_intra_transfers(inputs.rows)
        all_pairs, cross_asset_pairs = apply_manual_pairs(
            inputs.rows,
            auto_pairs,
            inputs.manual_pair_records,
        )
        cross_asset_pairs_by_tx_id = defaultdict(list)
        for pair in cross_asset_pairs:
            cross_asset_pairs_by_tx_id[pair["out_id"]].append(pair)
            cross_asset_pairs_by_tx_id[pair["in_id"]].append(pair)
        rows_by_asset = defaultdict(list)
        for row in inputs.rows:
            rows_by_asset[row["asset"]].append(row)
        pairs_by_asset = defaultdict(list)
        for pair in all_pairs:
            pairs_by_asset[pair["out"]["asset"]].append(pair)

        for asset, asset_rows in rows_by_asset.items():
            normalized_inputs = normalize_tax_asset_inputs(
                self.profile,
                asset,
                asset_rows,
                inputs.wallet_refs_by_id,
                pairs_by_asset.get(asset, []),
                tax_annotations_by_tx_id=inputs.tax_annotations_by_tx_id,
            )
            asset_result = self._process_asset(
                normalized_inputs,
                inputs.wallet_refs_by_id,
                cross_asset_pairs_by_tx_id,
            )
            entries.extend(asset_result.entries)
            quarantines.extend(asset_result.quarantines)
            intra_audit_all.extend(asset_result.intra_audit)
            for key, totals in asset_result.account_holdings.items():
                account_holdings[key]["quantity"] += totals["quantity"]
                account_holdings[key]["cost_basis"] += totals["cost_basis"]
            for key, totals in asset_result.wallet_holdings.items():
                wallet_holdings[key]["quantity"] += totals["quantity"]
                wallet_holdings[key]["cost_basis"] += totals["cost_basis"]

        return TaxEngineLedgerResult(
            entries=entries,
            quarantines=quarantines,
            intra_audit=intra_audit_all,
            cross_asset_pairs=cross_asset_pairs,
            account_holdings=dict(account_holdings),
            wallet_holdings=dict(wallet_holdings),
        )

    def _process_asset(
        self,
        normalized_inputs: NormalizedTaxAssetInputs,
        wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
        cross_asset_pairs_by_tx_id: Mapping[str, list[Mapping[str, Any]]],
    ) -> _AustrianAssetResult:
        entries: list[dict[str, Any]] = []
        quarantines = list(normalized_inputs.quarantines)
        intra_audit: list[dict[str, Any]] = []
        states_by_wallet_id = {
            wallet_id: _WalletAssetState()
            for wallet_id in wallet_refs_by_id
        }
        events_by_id = {event.transaction_id: event for event in normalized_inputs.events}
        transfers_by_id = {transfer.out_transaction_id: transfer for transfer in normalized_inputs.transfers}

        for item_kind, item_id in normalized_inputs.ordered_items:
            if item_kind == "transfer":
                transfer = transfers_by_id[item_id]
                source_wallet = wallet_refs_by_id[transfer.from_wallet_id]
                destination_wallet = wallet_refs_by_id[transfer.to_wallet_id]
                source_state = states_by_wallet_id[transfer.from_wallet_id]
                destination_state = states_by_wallet_id[transfer.to_wallet_id]
                available = source_state.total_quantity()
                if available < transfer.sent:
                    quarantines.append(
                        build_tax_quarantine(
                            self.profile,
                            transfer.out_row,
                            "insufficient_lots",
                            {
                                "wallet": source_wallet["label"],
                                "asset": normalized_inputs.asset,
                                "required": float(transfer.sent),
                                "available": float(available),
                            },
                        )
                    )
                    continue
                segments = _consume_segments(source_state, transfer.sent)
                moved_segments, fee_segments = _split_segments(segments, transfer.received)
                for segment in moved_segments:
                    _add_segment_to_wallet(destination_state, segment)
                description = f"Transfer {source_wallet['label']} -> {destination_wallet['label']}"
                entries.append(
                    _journal_entry(
                        self.profile,
                        source_wallet,
                        transaction_id=transfer.out_transaction_id,
                        occurred_at=transfer.occurred_at,
                        entry_type="transfer_out",
                        asset=normalized_inputs.asset,
                        quantity=-transfer.sent,
                        fiat_value=_ZERO,
                        unit_cost=_ZERO,
                        cost_basis=None,
                        proceeds=None,
                        gain_loss=None,
                        description=description,
                    )
                )
                entries.append(
                    _journal_entry(
                        self.profile,
                        destination_wallet,
                        transaction_id=transfer.in_transaction_id,
                        occurred_at=transfer.occurred_at,
                        entry_type="transfer_in",
                        asset=normalized_inputs.asset,
                        quantity=transfer.received,
                        fiat_value=_ZERO,
                        unit_cost=_ZERO,
                        cost_basis=None,
                        proceeds=None,
                        gain_loss=None,
                        description=description,
                    )
                )
                if fee_segments:
                    _append_realized_entries(
                        entries,
                        self.profile,
                        source_wallet,
                        transaction_id=transfer.out_transaction_id,
                        occurred_at=transfer.occurred_at,
                        entry_type="transfer_fee",
                        asset=normalized_inputs.asset,
                        total_proceeds=transfer.fee * (transfer.spot_price or _ZERO),
                        description=transfer.description,
                        segments=fee_segments,
                    )
                intra_audit.append(
                    {
                        "asset": normalized_inputs.asset,
                        "occurred_at": transfer.occurred_at,
                        "out_id": transfer.out_transaction_id,
                        "in_id": transfer.in_transaction_id,
                        "from_wallet_id": transfer.from_wallet_id,
                        "from_wallet_label": transfer.from_wallet_label,
                        "to_wallet_id": transfer.to_wallet_id,
                        "to_wallet_label": transfer.to_wallet_label,
                        "crypto_sent": transfer.sent,
                        "crypto_received": transfer.received,
                        "crypto_fee": transfer.fee,
                        "spot_price": transfer.spot_price or _ZERO,
                        "external_id": transfer.external_id,
                    }
                )
                continue

            event = events_by_id[item_id]
            wallet = wallet_refs_by_id[event.wallet_id]
            state = states_by_wallet_id[event.wallet_id]
            cross_asset_pairs = cross_asset_pairs_by_tx_id.get(event.transaction_id, [])
            cross_asset_taxable_leg = _allows_taxable_cross_asset_processing(cross_asset_pairs)
            if cross_asset_pairs and not cross_asset_taxable_leg:
                for pair in cross_asset_pairs:
                    is_out_leg = pair["out_id"] == event.transaction_id
                    quarantines.append(
                        build_tax_quarantine(
                            self.profile,
                            event.raw_row,
                            "cross_asset_pair_unsupported",
                            {
                                "pair_id": pair["pair_id"],
                                "kind": pair["kind"],
                                "policy": pair["policy"],
                                "leg": "out" if is_out_leg else "in",
                                "other_transaction_id": pair["in_id"] if is_out_leg else pair["out_id"],
                                "other_asset": pair["in_asset"] if is_out_leg else pair["out_asset"],
                            },
                        )
                    )
                continue

            if event.direction == "inbound":
                if not cross_asset_taxable_leg and _unsupported_inbound_kind(event):
                    quarantines.append(
                        build_tax_quarantine(
                            self.profile,
                            event.raw_row,
                            "insufficient_tax_provenance",
                            {
                                "wallet": wallet["label"],
                                "asset": event.asset,
                                "direction": event.direction,
                                "kind": _effective_event_kind(event),
                            },
                        )
                    )
                    continue
                if event.amount <= 0:
                    continue
                annotation = _annotation_text(event)
                total_cost = event.fiat_value + (event.fee * (event.spot_price or _ZERO))
                acquisition_cost = _ZERO if annotation in _ANNOTATED_ZERO_BASIS_TYPES else total_cost
                regime = _add_acquisition_to_wallet(
                    state,
                    wallet,
                    event.occurred_at,
                    event.amount,
                    acquisition_cost,
                )
                if annotation in _ANNOTATED_INCOME_TYPES:
                    entries.append(
                        _journal_entry(
                            self.profile,
                            wallet,
                            transaction_id=event.transaction_id,
                            occurred_at=event.occurred_at,
                            entry_type="income",
                            asset=event.asset,
                            quantity=_ZERO,
                            fiat_value=event.fiat_value,
                            unit_cost=event.spot_price or _ZERO,
                            cost_basis=None,
                            proceeds=None,
                            gain_loss=None,
                            description=event.description,
                        )
                    )
                entries.append(
                    _journal_entry(
                        self.profile,
                        wallet,
                        transaction_id=event.transaction_id,
                        occurred_at=event.occurred_at,
                        entry_type="acquisition",
                        asset=event.asset,
                        quantity=event.amount,
                        fiat_value=acquisition_cost,
                        unit_cost=(acquisition_cost / event.amount) if event.amount else _ZERO,
                        cost_basis=None,
                        proceeds=None,
                        gain_loss=None,
                        description=_with_regime_note(event.description, regime),
                    )
                )
                continue

            if not cross_asset_taxable_leg and _unsupported_outbound_kind(event):
                quarantines.append(
                    build_tax_quarantine(
                        self.profile,
                        event.raw_row,
                        "insufficient_tax_provenance",
                        {
                            "wallet": wallet["label"],
                            "asset": event.asset,
                            "direction": event.direction,
                            "kind": _effective_event_kind(event),
                        },
                    )
                )
                continue

            quantity_needed = event.amount + event.fee
            available = state.total_quantity()
            if quantity_needed > available:
                quarantines.append(
                    build_tax_quarantine(
                        self.profile,
                        event.raw_row,
                        "insufficient_lots",
                        {
                            "wallet": wallet["label"],
                            "asset": event.asset,
                            "required": float(quantity_needed),
                            "available": float(available),
                        },
                    )
                )
                continue

            segments = _consume_segments(state, quantity_needed)
            if event.amount > 0:
                proceeds = event.fiat_value - (event.fee * (event.spot_price or _ZERO))
                if proceeds < _ZERO:
                    proceeds = _ZERO
                _append_realized_entries(
                    entries,
                    self.profile,
                    wallet,
                    transaction_id=event.transaction_id,
                    occurred_at=event.occurred_at,
                    entry_type="disposal",
                    asset=event.asset,
                    total_proceeds=proceeds,
                    description=event.description,
                    segments=segments,
                )
                continue
            if event.fee > 0:
                _append_realized_entries(
                    entries,
                    self.profile,
                    wallet,
                    transaction_id=event.transaction_id,
                    occurred_at=event.occurred_at,
                    entry_type="fee",
                    asset=event.asset,
                    total_proceeds=event.fee * (event.spot_price or _ZERO),
                    description=event.description,
                    segments=segments,
                )

        account_holdings, wallet_holdings = _holdings_for_asset(
            states_by_wallet_id,
            wallet_refs_by_id,
            normalized_inputs.asset,
        )
        return _AustrianAssetResult(
            entries=entries,
            quarantines=quarantines,
            intra_audit=intra_audit,
            account_holdings=account_holdings,
            wallet_holdings=wallet_holdings,
        )


__all__ = ["ExperimentalAustrianTaxEngine"]
