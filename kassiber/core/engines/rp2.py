from __future__ import annotations

import os
import tempfile
import uuid
from collections import defaultdict
from contextlib import contextmanager
from dataclasses import dataclass
from decimal import Decimal
from importlib import import_module
from typing import Any, Iterable, Iterator, Mapping

from ...errors import AppError
from ...msat import btc_to_msat, dec, msat_to_btc
from ...tax_policy import build_tax_policy
from ...transfers import apply_manual_pairs, detect_intra_transfers
from ..austrian import (
    AT_SWAP_QUARANTINE_REASON,
    REGIME_NEU,
    kennzahl_for_disposal_category,
)
from ..tax_events import NormalizedTaxAssetInputs, build_tax_quarantine, normalize_tax_asset_inputs
from .base import TaxEngineLedgerInputs, TaxEngineLedgerResult

_RP2_MODULES = None
_RP2_EARN_TRANSACTION_TYPES = {
    "airdrop",
    "hardfork",
    "income",
    "interest",
    "mining",
    "staking",
    "wages",
}
_RP2_INBOUND_KIND_TO_TRANSACTION_TYPE = {
    "airdrop": "AIRDROP",
    "hardfork": "HARDFORK",
    "hard_fork": "HARDFORK",
    "income": "INCOME",
    "interest": "INTEREST",
    "lending_interest": "INTEREST",
    "mining": "MINING",
    "mining_reward": "MINING",
    "routing_income": "INCOME",
    "staking": "STAKING",
    "wages": "WAGES",
}


@dataclass(frozen=True)
class _RP2AssetResult:
    entries: list[dict[str, Any]]
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    tax_summary: list[dict[str, Any]]
    account_holdings: dict[tuple[Any, ...], dict[str, Any]]
    wallet_holdings: dict[tuple[Any, ...], dict[str, Any]]


@dataclass(frozen=True)
class _RP2AssetState:
    computed_data: Any | None
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    balance_set: Any | None


@dataclass(frozen=True)
class _RP2PreparedInput:
    """Output of the parse phase — carries the per-asset RP2 ``InputData`` plus the
    quarantines/audit accumulated while building the transaction sets. ``input_data`` is
    ``None`` when the asset has no acquisitions (nothing to compute). Kept separate from
    ``_RP2AssetState`` so cross-asset validation can run over every asset's ``InputData``
    before any ``compute_tax`` call — the whole point of splitting prepare from compute.
    """

    asset: str
    input_data: Any | None
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]


def _get_rp2_modules() -> dict[str, Any]:
    global _RP2_MODULES
    if _RP2_MODULES is not None:
        return _RP2_MODULES
    try:
        _RP2_MODULES = {
            "AVLTree": import_module("prezzemolo.avl_tree").AVLTree,
            "AbstractCountry": import_module("rp2.abstract_country").AbstractCountry,
            "AccountingEngine": import_module("rp2.accounting_engine").AccountingEngine,
            "BalanceSet": import_module("rp2.balance").BalanceSet,
            "Configuration": import_module("rp2.configuration").Configuration,
            "InputData": import_module("rp2.input_data").InputData,
            "InTransaction": import_module("rp2.in_transaction").InTransaction,
            "IntraTransaction": import_module("rp2.intra_transaction").IntraTransaction,
            "OutTransaction": import_module("rp2.out_transaction").OutTransaction,
            "TransactionSet": import_module("rp2.transaction_set").TransactionSet,
            "compute_tax": import_module("rp2.tax_engine").compute_tax,
            "RP2Decimal": import_module("rp2.rp2_decimal").RP2Decimal,
        }
    except ModuleNotFoundError as exc:
        raise AppError(
            "RP2 integration requires the 'rp2' package. Reinstall Kassiber in a Python >= 3.10 environment."
        ) from exc
    return _RP2_MODULES


def _rp2_decimal(value: Any):
    modules = _get_rp2_modules()
    return modules["RP2Decimal"](str(value))


def _load_at_country_module():
    try:
        return import_module("rp2.plugin.country.at")
    except ModuleNotFoundError as exc:
        raise AppError(
            "Austrian tax support requires rp2 with the `at` country plugin.",
            code="unsupported",
            hint=(
                "Install the Kassiber-maintained rp2 fork from `bitcoinaustria/rp2` "
                "with the Austrian country plugin."
            ),
            details={"missing_module": "rp2.plugin.country.at"},
        ) from exc


def _classify_at_disposal(gain_loss: Any) -> tuple[str, int | None]:
    at_module = _load_at_country_module()
    try:
        category = at_module.classify_disposal(gain_loss)
    except AttributeError as exc:
        raise AppError(
            "Austrian tax support requires rp2's `classify_disposal` API.",
            code="unsupported",
            hint=(
                "Update the Kassiber rp2 pin to a build from `bitcoinaustria/rp2` "
                "that exposes `rp2.plugin.country.at.classify_disposal`."
            ),
            details={"missing_symbol": "rp2.plugin.country.at.classify_disposal"},
        ) from exc
    category_value = str(getattr(category, "value", category))
    return category_value, kennzahl_for_disposal_category(category_value)


def _compose_event_notes(event: Any) -> str:
    """Serialize typed Austrian markers plus human description into rp2 notes.

    Markers come first in a fixed order (regime, pool, swap_link) so downstream
    diffs are stable; free-form description follows. Absent markers produce no
    token — the rp2 AT plugin treats "absent" and "empty value" differently
    (empty `at_swap_link=` raises RP2ValueError), so we never emit a bare
    `key=` token.
    """
    tokens: list[str] = []
    regime = getattr(event, "at_regime", None)
    if regime:
        tokens.append(f"at_regime={regime}")
    pool = getattr(event, "at_pool", None)
    if pool:
        tokens.append(f"at_pool={pool}")
    swap_link = getattr(event, "at_swap_link", None)
    if swap_link:
        tokens.append(f"at_swap_link={swap_link}")
    description = getattr(event, "description", "") or ""
    if description:
        tokens.append(description)
    return " ".join(tokens)


def _profile_str(profile: Mapping[str, Any], key: str) -> str:
    if hasattr(profile, "keys") and key in profile.keys():
        value = profile[key]
        if value is None:
            return ""
        return str(value).strip()
    return ""


def _normalized_event_kind(event: Any) -> str:
    raw_row = getattr(event, "raw_row", None) or {}
    kind = raw_row["kind"] if hasattr(raw_row, "keys") and "kind" in raw_row.keys() else None
    if kind is None:
        return ""
    return str(kind).strip().lower().replace("-", "_").replace(" ", "_")


def _rp2_in_transaction_type(event: Any) -> str:
    kind = _normalized_event_kind(event)
    return _RP2_INBOUND_KIND_TO_TRANSACTION_TYPE.get(kind, "BUY")


def _is_rp2_earn_transaction_type(transaction_type: Any) -> bool:
    checker = getattr(transaction_type, "is_earn_type", None)
    if callable(checker):
        return bool(checker())
    value = getattr(transaction_type, "value", transaction_type)
    return str(value or "").strip().lower() in _RP2_EARN_TRANSACTION_TYPES


def _compose_transfer_notes(transfer: Any) -> str:
    tokens: list[str] = []
    pool = getattr(transfer, "at_pool", None)
    if pool:
        tokens.append(f"at_pool={pool}")
    description = getattr(transfer, "description", "") or ""
    if description:
        tokens.append(description)
    return " ".join(tokens)


def _make_rp2_country(profile: Mapping[str, Any]):
    AbstractCountry = _get_rp2_modules()["AbstractCountry"]
    try:
        policy = build_tax_policy(profile)
    except ValueError as exc:
        raise AppError(str(exc)) from exc
    if policy.tax_country == "at":
        return _load_at_country_module().AT()
    currency_code = policy.fiat_currency

    class KassiberCountry(AbstractCountry):
        def __init__(self):
            super().__init__(policy.tax_country, currency_code)

        def get_long_term_capital_gain_period(self):
            return policy.long_term_days

        def get_default_accounting_method(self):
            return policy.default_accounting_method

        def get_accounting_methods(self):
            return set(policy.accounting_methods)

        def get_report_generators(self):
            return set(policy.report_generators)

        def get_default_generation_language(self):
            return policy.generation_language

    return KassiberCountry()


@contextmanager
def _rp2_configuration(
    profile: Mapping[str, Any],
    wallet_labels: Iterable[str],
    assets: Iterable[str],
) -> Iterator[Any]:
    Configuration = _get_rp2_modules()["Configuration"]
    sorted_wallet_labels = sorted(wallet_labels)
    sorted_assets = sorted(assets)
    if not sorted_wallet_labels:
        raise AppError("RP2 configuration requires at least one wallet")
    if not sorted_assets:
        raise AppError("RP2 configuration requires at least one asset")
    content = "\n".join(
        [
            "[general]",
            f"assets = {', '.join(sorted_assets)}",
            f"exchanges = {', '.join(sorted_wallet_labels)}",
            f"holders = {profile['label']}",
            "",
            "[in_header]",
            "timestamp = 0",
            "asset = 1",
            "exchange = 2",
            "holder = 3",
            "transaction_type = 4",
            "spot_price = 5",
            "crypto_in = 6",
            "crypto_fee = 7",
            "fiat_in_no_fee = 8",
            "fiat_in_with_fee = 9",
            "fiat_fee = 10",
            "unique_id = 11",
            "notes = 12",
            "",
            "[out_header]",
            "timestamp = 0",
            "asset = 1",
            "exchange = 2",
            "holder = 3",
            "transaction_type = 4",
            "spot_price = 5",
            "crypto_out_no_fee = 6",
            "crypto_fee = 7",
            "crypto_out_with_fee = 8",
            "fiat_out_no_fee = 9",
            "fiat_fee = 10",
            "unique_id = 11",
            "notes = 12",
            "",
            "[intra_header]",
            "timestamp = 0",
            "asset = 1",
            "from_exchange = 2",
            "from_holder = 3",
            "to_exchange = 4",
            "to_holder = 5",
            "spot_price = 6",
            "crypto_sent = 7",
            "crypto_received = 8",
            "unique_id = 9",
            "notes = 10",
            "",
        ]
    )
    handle = tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".ini", delete=False)
    config_path = handle.name
    try:
        handle.write(content)
        handle.flush()
    finally:
        handle.close()
    try:
        yield Configuration(config_path, _make_rp2_country(profile))
    finally:
        try:
            os.unlink(config_path)
        except OSError:
            pass


def _build_rp2_accounting_engine(profile: Mapping[str, Any]):
    modules = _get_rp2_modules()
    method_name = str(profile["gains_algorithm"]).strip().lower()
    try:
        policy = build_tax_policy(profile)
    except ValueError as exc:
        raise AppError(str(exc)) from exc
    if method_name not in set(policy.accounting_methods):
        raise AppError(f"Unsupported RP2 accounting method '{profile['gains_algorithm']}'")
    try:
        method_module = import_module(f"rp2.plugin.accounting_method.{method_name}")
    except ModuleNotFoundError as exc:
        raise AppError(f"RP2 accounting method '{profile['gains_algorithm']}' is not available") from exc
    years_to_methods = modules["AVLTree"]()
    years_to_methods.insert_node(1970, method_module.AccountingMethod())
    return modules["AccountingEngine"](years_2_methods=years_to_methods)


def _rows_by_transaction_id(normalized_inputs: NormalizedTaxAssetInputs) -> dict[str, Mapping[str, Any]]:
    rows_by_id = {event.transaction_id: event.raw_row for event in normalized_inputs.events}
    for transfer in normalized_inputs.transfers:
        rows_by_id[transfer.out_transaction_id] = transfer.out_row
        rows_by_id[transfer.in_transaction_id] = transfer.in_row
    return rows_by_id


def _prepare_rp2_asset_input(profile, normalized_inputs: NormalizedTaxAssetInputs, configuration) -> _RP2PreparedInput:
    """Build RP2 ``InputData`` for one asset without running ``compute_tax``.

    Splitting the parse and compute phases lets the caller run the country's cross-asset
    validator (e.g. Austrian `at_swap_link` pairing) against every asset's ``InputData``
    before any per-asset accounting happens — the validator cannot see other assets from
    inside a single ``compute_tax`` call.
    """

    modules = _get_rp2_modules()
    TransactionSet = modules["TransactionSet"]
    InTransaction = modules["InTransaction"]
    OutTransaction = modules["OutTransaction"]
    IntraTransaction = modules["IntraTransaction"]
    InputData = modules["InputData"]
    asset = normalized_inputs.asset
    in_set = TransactionSet(configuration, "IN", asset)
    out_set = TransactionSet(configuration, "OUT", asset)
    intra_set = TransactionSet(configuration, "INTRA", asset)
    holder = profile["label"]
    total_available = Decimal("0")
    priced_available = Decimal("0")
    quarantines = list(normalized_inputs.quarantines)
    intra_audit = []
    row_index = 1
    events_by_id = {event.transaction_id: event for event in normalized_inputs.events}
    transfers_by_id = {transfer.out_transaction_id: transfer for transfer in normalized_inputs.transfers}

    for item_kind, item_id in normalized_inputs.ordered_items:
        if item_kind == "transfer":
            transfer = transfers_by_id[item_id]
            if total_available < transfer.sent:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        transfer.out_row,
                        "insufficient_lots",
                        {
                            "from_wallet": transfer.from_wallet_label,
                            "asset": asset,
                            "required": float(transfer.sent),
                            "available": float(total_available),
                        },
                    )
                )
                continue
            if priced_available < transfer.sent:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        transfer.out_row,
                        "missing_cost_basis",
                        {
                            "from_wallet": transfer.from_wallet_label,
                            "asset": asset,
                            "required": float(transfer.sent),
                            "priced_available": float(priced_available),
                        },
                    )
                )
                continue
            intra_set.add_entry(
                IntraTransaction(
                    configuration=configuration,
                    timestamp=transfer.occurred_at,
                    asset=asset,
                    from_exchange=transfer.from_wallet_label,
                    from_holder=holder,
                    to_exchange=transfer.to_wallet_label,
                    to_holder=holder,
                    spot_price=_rp2_decimal(transfer.spot_price if transfer.spot_price is not None else 0),
                    crypto_sent=_rp2_decimal(transfer.sent),
                    crypto_received=_rp2_decimal(transfer.received),
                    row=row_index,
                    unique_id=transfer.out_transaction_id,
                    notes=_compose_transfer_notes(transfer),
                )
            )
            row_index += 1
            total_available -= transfer.fee
            priced_available -= transfer.fee
            intra_audit.append(
                {
                    "out_id": transfer.out_transaction_id,
                    "in_id": transfer.in_transaction_id,
                    "from_wallet_id": transfer.from_wallet_id,
                    "from_wallet_label": transfer.from_wallet_label,
                    "to_wallet_id": transfer.to_wallet_id,
                    "to_wallet_label": transfer.to_wallet_label,
                    "asset": asset,
                    "occurred_at": transfer.occurred_at,
                    "external_id": transfer.external_id,
                    "crypto_sent": float(transfer.sent),
                    "crypto_received": float(transfer.received),
                    "crypto_fee": float(transfer.fee),
                    "spot_price": float(transfer.spot_price) if transfer.spot_price is not None else 0.0,
                }
            )
            continue

        event = events_by_id[item_id]
        if event.direction == "inbound":
            total_available += event.amount
            in_set.add_entry(
                InTransaction(
                    configuration=configuration,
                    timestamp=event.occurred_at,
                    asset=asset,
                    exchange=event.wallet_label,
                    holder=holder,
                    transaction_type=_rp2_in_transaction_type(event),
                    spot_price=_rp2_decimal(event.spot_price),
                    crypto_in=_rp2_decimal(event.amount),
                    fiat_in_no_fee=_rp2_decimal(event.fiat_value),
                    fiat_in_with_fee=_rp2_decimal(event.fiat_value),
                    fiat_fee=_rp2_decimal(0),
                    row=row_index,
                    unique_id=event.transaction_id,
                    notes=_compose_event_notes(event),
                )
            )
            priced_available += event.amount
            row_index += 1
            continue

        needed = event.amount + event.fee
        if needed <= 0:
            continue
        # Marked Austrian swap-outs can depend on another asset's same-timestamp
        # swap-in. Feed the full graph to rp2's native runner instead of applying
        # Kassiber's single-asset availability gate first.
        is_marked_at_swap = bool(getattr(event, "at_swap_link", None))
        if total_available < needed and not is_marked_at_swap:
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    event.raw_row,
                    "insufficient_lots",
                    {
                        "wallet": event.wallet_label,
                        "asset": asset,
                        "required": float(needed),
                        "available": float(total_available),
                    },
                )
            )
            continue
        if priced_available < needed and not is_marked_at_swap:
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    event.raw_row,
                    "missing_cost_basis",
                    {
                        "wallet": event.wallet_label,
                        "asset": asset,
                        "required": float(needed),
                        "priced_available": float(priced_available),
                    },
                )
            )
            continue
        out_set.add_entry(
            OutTransaction(
                configuration=configuration,
                timestamp=event.occurred_at,
                asset=asset,
                exchange=event.wallet_label,
                holder=holder,
                transaction_type="SELL" if event.amount > 0 else "FEE",
                spot_price=_rp2_decimal(event.spot_price),
                crypto_out_no_fee=_rp2_decimal(event.amount),
                crypto_fee=_rp2_decimal(event.fee),
                fiat_out_no_fee=_rp2_decimal(event.fiat_value) if event.amount > 0 else None,
                fiat_fee=_rp2_decimal(event.fee * event.spot_price),
                row=row_index,
                unique_id=event.transaction_id,
                notes=_compose_event_notes(event),
            )
        )
        total_available -= needed
        priced_available -= needed
        row_index += 1

    if in_set.count == 0:
        return _RP2PreparedInput(
            asset=asset,
            input_data=None,
            quarantines=quarantines,
            intra_audit=intra_audit,
        )
    input_data = InputData(
        asset=asset,
        unfiltered_in_transaction_set=in_set,
        unfiltered_out_transaction_set=out_set,
        unfiltered_intra_transaction_set=intra_set,
    )
    return _RP2PreparedInput(
        asset=asset,
        input_data=input_data,
        quarantines=quarantines,
        intra_audit=intra_audit,
    )


def _rp2_asset_state_from_prepared(prepared: _RP2PreparedInput, profile, configuration) -> _RP2AssetState:
    """Run ``compute_tax`` + ``BalanceSet`` on a prepared input. No-op when there are no
    acquisitions (``prepared.input_data is None``)."""

    if prepared.input_data is None:
        return _RP2AssetState(
            computed_data=None,
            quarantines=prepared.quarantines,
            intra_audit=prepared.intra_audit,
            balance_set=None,
        )
    modules = _get_rp2_modules()
    compute_tax = modules["compute_tax"]
    try:
        computed_data = compute_tax(configuration, _build_rp2_accounting_engine(profile), prepared.input_data)
    except Exception as exc:
        raise AppError(f"RP2 tax calculation failed for asset '{prepared.asset}': {exc}") from exc
    return _rp2_asset_state_from_computed_data(prepared, computed_data, configuration)


def _rp2_asset_state_from_computed_data(
    prepared: _RP2PreparedInput,
    computed_data: Any,
    configuration: Any,
) -> _RP2AssetState:
    from datetime import date as _date

    balance_set = getattr(computed_data, "balance_set", None)
    if balance_set is None:
        BalanceSet = _get_rp2_modules()["BalanceSet"]
        balance_set = BalanceSet(configuration, prepared.input_data, _date.max)
    return _RP2AssetState(
        computed_data=computed_data,
        quarantines=prepared.quarantines,
        intra_audit=prepared.intra_audit,
        balance_set=balance_set,
    )


def _rp2_asset_states_from_prepared(
    prepared_by_asset: list[tuple[NormalizedTaxAssetInputs, _RP2PreparedInput]],
    profile: Mapping[str, Any],
    configuration: Any,
    *,
    requires_multi_asset_compute: bool = False,
) -> dict[str, _RP2AssetState]:
    input_data_by_asset = {
        prepared.asset: prepared.input_data
        for _, prepared in prepared_by_asset
        if prepared.input_data is not None
    }
    multi_asset_compute = getattr(configuration.country, "compute_tax_for_assets", None)
    if requires_multi_asset_compute and not callable(multi_asset_compute):
        raise AppError(
            "Installed rp2 is missing `AbstractCountry.compute_tax_for_assets`. "
            "Native Austrian swap carry requires bitcoinaustria/rp2 PR #6 or later.",
            code="unsupported",
            hint="Run `uv sync --refresh-package rp2` (or reinstall rp2 from the pin in pyproject.toml).",
        )

    if input_data_by_asset and callable(multi_asset_compute):
        try:
            computed_by_asset = multi_asset_compute(
                configuration,
                _build_rp2_accounting_engine(profile),
                input_data_by_asset,
            )
        except Exception as exc:
            raise AppError(f"RP2 multi-asset tax calculation failed: {exc}") from exc
        if computed_by_asset is not None:
            states: dict[str, _RP2AssetState] = {}
            for _, prepared in prepared_by_asset:
                if prepared.input_data is None:
                    states[prepared.asset] = _RP2AssetState(
                        computed_data=None,
                        quarantines=prepared.quarantines,
                        intra_audit=prepared.intra_audit,
                        balance_set=None,
                    )
                    continue
                computed_data = computed_by_asset.get(prepared.asset)
                if computed_data is None:
                    raise AppError(
                        f"RP2 multi-asset tax calculation did not return ComputedData for asset '{prepared.asset}'",
                        code="internal",
                    )
                states[prepared.asset] = _rp2_asset_state_from_computed_data(
                    prepared,
                    computed_data,
                    configuration,
                )
            return states

    return {
        prepared.asset: _rp2_asset_state_from_prepared(prepared, profile, configuration)
        for _, prepared in prepared_by_asset
    }


def _rp2_asset_state(profile, normalized_inputs: NormalizedTaxAssetInputs, configuration) -> _RP2AssetState:
    """Legacy single-call orchestrator: prepare then compute. Preserved for callers that
    don't need to run cross-asset validation between the two phases (all current direct
    callers). ``GenericRP2TaxEngine.build_ledger_state`` drives prepare/validate/compute
    explicitly and does not go through here.
    """

    prepared = _prepare_rp2_asset_input(profile, normalized_inputs, configuration)
    return _rp2_asset_state_from_prepared(prepared, profile, configuration)


def _effective_fiat_in_with_fee(computed_data: Any, transaction: Any) -> Any:
    accessor = getattr(computed_data, "get_in_transaction_fiat_in_with_fee", None)
    if callable(accessor):
        return accessor(transaction)
    return transaction.fiat_in_with_fee


def _append_rp2_journal_entries(entries, computed_data, wallet_refs_by_label, profile, row_by_id, intra_audit):
    tax_country = _profile_str(profile, "tax_country").lower()

    def _wallet_for(transaction):
        label = getattr(transaction, "exchange", None) or getattr(transaction, "from_exchange", None)
        ref = wallet_refs_by_label.get(label)
        if ref is None:
            raise AppError(
                f"RP2 emitted transaction for unknown wallet '{label}'",
                code="internal",
            )
        return ref

    for transaction in computed_data.in_transaction_set:
        source_row = row_by_id.get(transaction.unique_id)
        wallet = _wallet_for(transaction)
        fiat_in_with_fee = dec(_effective_fiat_in_with_fee(computed_data, transaction))
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "transaction_id": transaction.unique_id,
                "wallet_id": wallet["id"],
                "account_id": wallet["wallet_account_id"],
                "occurred_at": source_row["occurred_at"] if source_row else transaction.timestamp.isoformat(),
                "entry_type": "acquisition",
                "asset": transaction.asset,
                "quantity": dec(transaction.crypto_in),
                "fiat_value": fiat_in_with_fee,
                "unit_cost": fiat_in_with_fee / dec(transaction.crypto_in),
                "cost_basis": None,
                "proceeds": None,
                "gain_loss": None,
                "description": transaction.notes or (source_row["description"] if source_row else "Inbound transaction"),
            }
        )

    audit_by_out_id = {audit["out_id"]: audit for audit in intra_audit}
    realized_by_event = {}
    for gain_loss in computed_data.gain_loss_set:
        taxable_event = gain_loss.taxable_event
        wallet = _wallet_for(taxable_event)
        is_earn = _is_rp2_earn_transaction_type(taxable_event.transaction_type)
        is_intra = (
            taxable_event.unique_id in audit_by_out_id
            and taxable_event.asset == computed_data.asset
            and taxable_event.transaction_type.value.lower() == "move"
        )
        if is_earn:
            entry_type = "income"
        elif is_intra:
            entry_type = "transfer_fee"
        elif taxable_event.transaction_type.value == "FEE":
            entry_type = "fee"
        else:
            entry_type = "disposal"
        at_category = None
        at_kennzahl = None
        event_key: Any = taxable_event.internal_id
        if tax_country == "at":
            at_category, at_kennzahl = _classify_at_disposal(gain_loss)
            # One taxable event can split across multiple Austrian semantic
            # buckets when RP2 matches against heterogeneous acquired lots, so
            # keep separate journal rows per category.
            event_key = (taxable_event.internal_id, at_category)
        event = realized_by_event.setdefault(
            event_key,
            {
                "transaction_id": taxable_event.unique_id,
                "wallet": wallet,
                "occurred_at": row_by_id[taxable_event.unique_id]["occurred_at"] if taxable_event.unique_id in row_by_id else taxable_event.timestamp.isoformat(),
                "entry_type": entry_type,
                "asset": taxable_event.asset,
                "quantity": Decimal("0"),
                "fiat_value": Decimal("0"),
                "cost_basis": Decimal("0"),
                "proceeds": Decimal("0"),
                "gain_loss": Decimal("0"),
                "description": taxable_event.notes or (
                    row_by_id[taxable_event.unique_id]["description"] if taxable_event.unique_id in row_by_id else "Outbound transaction"
                ),
                "at_category": at_category,
                "at_kennzahl": at_kennzahl,
            },
        )
        event["quantity"] += dec(gain_loss.crypto_amount)
        event["cost_basis"] += dec(gain_loss.fiat_cost_basis)
        event["proceeds"] += dec(gain_loss.taxable_event_fiat_amount_with_fee_fraction)
        event["gain_loss"] += dec(gain_loss.fiat_gain)
    for event in realized_by_event.values():
        wallet = event["wallet"]
        description = event["description"]
        proceeds = event["proceeds"]
        cost_basis = event["cost_basis"]
        gain_loss = event["gain_loss"]
        quantity = event["quantity"] if event["entry_type"] == "income" else -event["quantity"]
        entry = {
            "id": str(uuid.uuid4()),
            "workspace_id": profile["workspace_id"],
            "profile_id": profile["id"],
            "transaction_id": event["transaction_id"],
            "wallet_id": wallet["id"],
            "account_id": wallet["wallet_account_id"],
            "occurred_at": event["occurred_at"],
            "entry_type": event["entry_type"],
            "asset": event["asset"],
            "quantity": quantity,
            "fiat_value": proceeds,
            "unit_cost": Decimal("0"),
            "cost_basis": cost_basis,
            "proceeds": proceeds,
            "gain_loss": gain_loss,
            "description": description,
        }
        if event.get("at_category") is not None:
            entry["at_category"] = event["at_category"]
            entry["at_kennzahl"] = event["at_kennzahl"]
        entries.append(entry)

    for audit in intra_audit:
        from_wallet = wallet_refs_by_label[audit["from_wallet_label"]]
        to_wallet = wallet_refs_by_label[audit["to_wallet_label"]]
        sent = dec(audit["crypto_sent"])
        received = dec(audit["crypto_received"])
        description = f"Transfer {from_wallet['label']} -> {to_wallet['label']}"
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "transaction_id": audit["out_id"],
                "wallet_id": from_wallet["id"],
                "account_id": from_wallet["wallet_account_id"],
                "occurred_at": audit["occurred_at"],
                "entry_type": "transfer_out",
                "asset": audit["asset"],
                "quantity": -sent,
                "fiat_value": Decimal("0"),
                "unit_cost": Decimal("0"),
                "cost_basis": None,
                "proceeds": None,
                "gain_loss": None,
                "description": description,
            }
        )
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "transaction_id": audit["in_id"],
                "wallet_id": to_wallet["id"],
                "account_id": to_wallet["wallet_account_id"],
                "occurred_at": audit["occurred_at"],
                "entry_type": "transfer_in",
                "asset": audit["asset"],
                "quantity": received,
                "fiat_value": Decimal("0"),
                "unit_cost": Decimal("0"),
                "cost_basis": None,
                "proceeds": None,
                "gain_loss": None,
                "description": description,
            }
        )


def _accumulate_asset_holdings(account_holdings, wallet_holdings, computed_data, balance_set, wallet_refs_by_label):
    asset = computed_data.asset
    total_quantity = Decimal("0")
    total_cost_basis = Decimal("0")
    for transaction in computed_data.in_transaction_set:
        sold_percent = dec(computed_data.get_in_lot_sold_percentage(transaction))
        remaining_ratio = Decimal("1") - sold_percent
        if remaining_ratio <= 0:
            continue
        total_quantity += dec(transaction.crypto_in) * remaining_ratio
        total_cost_basis += dec(_effective_fiat_in_with_fee(computed_data, transaction)) * remaining_ratio
    if total_quantity <= 0 or balance_set is None:
        return
    avg_basis_per_unit = total_cost_basis / total_quantity
    for balance in balance_set:
        wallet_label = balance.exchange
        wallet = wallet_refs_by_label.get(wallet_label)
        if wallet is None:
            continue
        quantity = dec(balance.final_balance)
        if quantity <= 0:
            continue
        cost_basis = quantity * avg_basis_per_unit
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


def _build_tax_summary_rows(computed_data):
    rows = []
    for yearly in sorted(
        computed_data.yearly_gain_loss_list,
        key=lambda row: (
            row.year,
            row.asset,
            getattr(row.transaction_type, "value", row.transaction_type),
            row.is_long_term_capital_gains,
        ),
    ):
        quantity = dec(yearly.crypto_amount)
        rows.append(
            {
                "year": int(yearly.year),
                "asset": yearly.asset,
                "transaction_type": str(getattr(yearly.transaction_type, "value", yearly.transaction_type)).lower(),
                "capital_gains_type": "long" if yearly.is_long_term_capital_gains else "short",
                "quantity": float(quantity),
                "quantity_msat": btc_to_msat(quantity),
                "proceeds": float(dec(yearly.fiat_amount)),
                "cost_basis": float(dec(yearly.fiat_cost_basis)),
                "gain_loss": float(dec(yearly.fiat_gain_loss)),
            }
        )
    return rows


def _prepare_assets(
    profile: Mapping[str, Any],
    rows_by_asset: Mapping[str, list[Mapping[str, Any]]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    pairs_by_asset: Mapping[str, list[Mapping[str, Any]]],
    configuration: Any,
    *,
    at_swap_link_by_row_id: Mapping[str, str] | None = None,
    excluded_row_ids: set[str] | None = None,
) -> list[tuple[NormalizedTaxAssetInputs, _RP2PreparedInput]]:
    prepared_by_asset: list[tuple[NormalizedTaxAssetInputs, _RP2PreparedInput]] = []
    excluded = excluded_row_ids or set()
    for asset, asset_rows in rows_by_asset.items():
        active_rows = [row for row in asset_rows if str(row["id"]) not in excluded]
        normalized_inputs = normalize_tax_asset_inputs(
            profile,
            asset,
            active_rows,
            wallet_refs_by_id,
            pairs_by_asset.get(asset, []),
            at_swap_link_by_row_id=at_swap_link_by_row_id,
        )
        prepared = _prepare_rp2_asset_input(profile, normalized_inputs, configuration)
        prepared_by_asset.append((normalized_inputs, prepared))
    return prepared_by_asset


def _prepared_quarantine_reasons(
    prepared_by_asset: list[tuple[NormalizedTaxAssetInputs, _RP2PreparedInput]]
) -> dict[str, str]:
    reasons: dict[str, str] = {}
    for _, prepared in prepared_by_asset:
        for quarantine in prepared.quarantines:
            reasons.setdefault(str(quarantine["transaction_id"]), str(quarantine["reason"]))
    return reasons


def _swap_pair_quarantines(
    profile: Mapping[str, Any],
    pair: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
    reason_code: str,
) -> list[dict[str, Any]]:
    out_id = str(pair["out_id"])
    in_id = str(pair["in_id"])
    out_row = rows_by_id[out_id]
    in_row = rows_by_id[in_id]
    pair_id = str(pair.get("pair_id") or f"{out_id}->{in_id}")
    out_amount = msat_to_btc(out_row["amount"]) or Decimal("0")
    detail = {
        "outgoing_asset": pair.get("out_asset") or out_row["asset"],
        "incoming_asset": pair.get("in_asset") or in_row["asset"],
        "out_amount": float(out_amount),
        "at_swap_link": pair_id,
        "reason_code": reason_code,
    }
    return [
        build_tax_quarantine(profile, out_row, AT_SWAP_QUARANTINE_REASON, detail),
        build_tax_quarantine(profile, in_row, AT_SWAP_QUARANTINE_REASON, detail),
    ]


def _select_at_cross_asset_swap_links(
    profile: Mapping[str, Any],
    cross_asset_pairs: list[dict[str, Any]],
    rows: Iterable[Mapping[str, Any]],
    prepared_by_asset: list[tuple[NormalizedTaxAssetInputs, _RP2PreparedInput]],
) -> tuple[dict[str, str], set[str], list[dict[str, Any]]]:
    """Select reviewed AT carrying-value pairs for RP2-native basis carry.

    Kassiber owns the review/provenance boundary and emits only stable swap-link
    markers. It does not compute the carried basis here; the RP2 country hook
    owns that pool math once the marked rows reach `compute_tax_for_assets`.
    """

    tax_country = _profile_str(profile, "tax_country").lower()
    if tax_country != "at" or not cross_asset_pairs:
        return {}, set(), []

    rows_by_id = {str(row["id"]): row for row in rows}
    events_by_id = {
        str(event.transaction_id): event
        for normalized_inputs, _ in prepared_by_asset
        for event in normalized_inputs.events
    }
    quarantine_reasons = _prepared_quarantine_reasons(prepared_by_asset)
    swap_link_by_row_id: dict[str, str] = {}
    quarantined_row_ids: set[str] = set()
    quarantines: list[dict[str, Any]] = []

    for pair in cross_asset_pairs:
        if pair.get("policy") != "carrying-value":
            continue
        out_id = str(pair["out_id"])
        in_id = str(pair["in_id"])
        if out_id not in rows_by_id or in_id not in rows_by_id:
            continue

        out_event = events_by_id.get(out_id)
        in_event = events_by_id.get(in_id)
        if out_event is None or in_event is None:
            reason_code = quarantine_reasons.get(out_id) or quarantine_reasons.get(in_id) or "swap_leg_unavailable"
            quarantines.extend(_swap_pair_quarantines(profile, pair, rows_by_id, reason_code))
            quarantined_row_ids.update({out_id, in_id})
            continue

        if getattr(out_event, "at_regime", None) != REGIME_NEU:
            continue

        pair_id = str(pair.get("pair_id") or f"{out_id}->{in_id}")
        swap_link_by_row_id[out_id] = pair_id
        swap_link_by_row_id[in_id] = pair_id

    return swap_link_by_row_id, quarantined_row_ids, quarantines


class GenericRP2TaxEngine:
    """Current generic RP2-backed implementation behind the engine seam."""

    def __init__(self, profile: Mapping[str, Any]):
        self.profile = profile

    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        if not inputs.rows:
            return TaxEngineLedgerResult(
                entries=[],
                quarantines=[],
                intra_audit=[],
                cross_asset_pairs=[],
                tax_summary=[],
                account_holdings={},
                wallet_holdings={},
            )

        wallet_labels = {row["wallet_label"] for row in inputs.rows}
        assets = {row["asset"] for row in inputs.rows}
        entries: list[dict[str, Any]] = []
        quarantines: list[dict[str, Any]] = []
        intra_audit_all: list[dict[str, Any]] = []
        account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        cross_asset_pairs: list[dict[str, Any]] = []
        tax_summary_all: list[dict[str, Any]] = []
        with _rp2_configuration(self.profile, wallet_labels, assets) as configuration:
            wallet_refs_by_label = {
                ref["label"]: ref for ref in inputs.wallet_refs_by_id.values()
            }
            auto_pairs, _ = detect_intra_transfers(inputs.rows)
            all_pairs, cross_asset_pairs = apply_manual_pairs(
                inputs.rows,
                auto_pairs,
                inputs.manual_pair_records,
            )
            rows_by_asset = defaultdict(list)
            for row in inputs.rows:
                rows_by_asset[row["asset"]].append(row)
            pairs_by_asset = defaultdict(list)
            for pair in all_pairs:
                pairs_by_asset[pair["out"]["asset"]].append(pair)

            # Phase 1: normalize + build RP2 `InputData` for every asset. No `compute_tax`
            # runs here so the country's cross-asset validator can see every asset's
            # markers before any accounting.
            prepared_by_asset = _prepare_assets(
                self.profile,
                rows_by_asset,
                inputs.wallet_refs_by_id,
                pairs_by_asset,
                configuration,
            )
            swap_link_by_row_id, quarantined_row_ids, swap_quarantines = _select_at_cross_asset_swap_links(
                self.profile,
                cross_asset_pairs,
                inputs.rows,
                prepared_by_asset,
            )
            quarantines.extend(swap_quarantines)
            if swap_link_by_row_id or quarantined_row_ids:
                prepared_by_asset = _prepare_assets(
                    self.profile,
                    rows_by_asset,
                    inputs.wallet_refs_by_id,
                    pairs_by_asset,
                    configuration,
                    at_swap_link_by_row_id=swap_link_by_row_id,
                    excluded_row_ids=quarantined_row_ids,
                )

            # Phase 2: cross-asset validation via the country hook. Catches invariants
            # (e.g. Austrian `at_swap_link` marker must appear on two different assets)
            # that Kassiber's annotator structurally cannot detect — a paired leg that
            # was never imported can't be annotated, so only a post-hoc scan sees it.
            # `validate_input_data` was added in bitcoinaustria/rp2 PR #4. If an older rp2
            # is installed (editable checkout, stale `uv sync`), fall through with a clear
            # upgrade hint rather than a confusing generic failure.
            validator = getattr(configuration.country, "validate_input_data", None)
            if validator is None:
                raise AppError(
                    "Installed rp2 is missing `AbstractCountry.validate_input_data`. "
                    "Cross-asset swap-link validation requires bitcoinaustria/rp2 PR #4 or later.",
                    code="unsupported",
                    hint="Run `uv sync --refresh-package rp2` (or reinstall rp2 from the pin in pyproject.toml).",
                )
            input_data_list = [prepared.input_data for _, prepared in prepared_by_asset if prepared.input_data is not None]
            try:
                validator(input_data_list)
            except Exception as exc:
                raise AppError(
                    f"RP2 cross-asset input validation failed: {exc}",
                    code="rp2_input_validation",
                ) from exc

            # Phase 3: compute tax + assemble per-asset results. Austrian
            # carrying-value swaps use rp2's country-level multi-asset hook so
            # moving-average pool state, same-timestamp ordering, and basis
            # carry stay inside the tax engine.
            asset_states = _rp2_asset_states_from_prepared(
                prepared_by_asset,
                self.profile,
                configuration,
                requires_multi_asset_compute=bool(swap_link_by_row_id),
            )
            for normalized_inputs, prepared in prepared_by_asset:
                asset_result = self._process_asset(
                    prepared,
                    normalized_inputs,
                    wallet_refs_by_label,
                    configuration,
                    asset_states[prepared.asset],
                )
                quarantines.extend(asset_result.quarantines)
                intra_audit_all.extend(asset_result.intra_audit)
                tax_summary_all.extend(asset_result.tax_summary)
                entries.extend(asset_result.entries)
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
            tax_summary=tax_summary_all,
            account_holdings=dict(account_holdings),
            wallet_holdings=dict(wallet_holdings),
        )

    def _process_asset(
        self,
        prepared: _RP2PreparedInput,
        normalized_inputs: NormalizedTaxAssetInputs,
        wallet_refs_by_label: Mapping[str, Mapping[str, Any]],
        configuration: Any,
        asset_state: _RP2AssetState | None = None,
    ) -> _RP2AssetResult:
        if asset_state is None:
            asset_state = _rp2_asset_state_from_prepared(
                prepared,
                self.profile,
                configuration,
            )
        if asset_state.computed_data is None:
            return _RP2AssetResult(
                entries=[],
                quarantines=asset_state.quarantines,
                intra_audit=asset_state.intra_audit,
                tax_summary=[],
                account_holdings={},
                wallet_holdings={},
            )
        entries = []
        account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        _append_rp2_journal_entries(
            entries,
            asset_state.computed_data,
            wallet_refs_by_label,
            self.profile,
            _rows_by_transaction_id(normalized_inputs),
            asset_state.intra_audit,
        )
        _accumulate_asset_holdings(
            account_holdings,
            wallet_holdings,
            asset_state.computed_data,
            asset_state.balance_set,
            wallet_refs_by_label,
        )
        return _RP2AssetResult(
            entries=entries,
            quarantines=asset_state.quarantines,
            intra_audit=asset_state.intra_audit,
            tax_summary=_build_tax_summary_rows(asset_state.computed_data),
            account_holdings=dict(account_holdings),
            wallet_holdings=dict(wallet_holdings),
        )


__all__ = [
    "GenericRP2TaxEngine",
]
