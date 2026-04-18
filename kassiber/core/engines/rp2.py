from __future__ import annotations

import tempfile
import uuid
from collections import defaultdict
from decimal import Decimal
from importlib import import_module
from typing import Any, Iterable, Mapping

from ...errors import AppError
from ...msat import dec
from ...tax_policy import build_tax_policy
from ...util import parse_bool
from ..tax_events import NormalizedTaxAssetInputs, build_tax_quarantine
from .base import TaxEngineAssetResult

_RP2_MODULES = None


def get_rp2_modules() -> dict[str, Any]:
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


def rp2_decimal(value: Any):
    modules = get_rp2_modules()
    return modules["RP2Decimal"](str(value))


def make_rp2_country(profile: Mapping[str, Any]):
    AbstractCountry = get_rp2_modules()["AbstractCountry"]
    try:
        policy = build_tax_policy(profile)
    except ValueError as exc:
        raise AppError(str(exc)) from exc
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


def make_rp2_configuration(profile: Mapping[str, Any], wallet_labels: Iterable[str], assets: Iterable[str]):
    Configuration = get_rp2_modules()["Configuration"]
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
    try:
        handle.write(content)
        handle.flush()
    finally:
        handle.close()
    return Configuration(handle.name, make_rp2_country(profile)), handle.name


def build_rp2_accounting_engine(profile: Mapping[str, Any]):
    modules = get_rp2_modules()
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


def _rp2_quarantine(profile, row, reason, detail):
    return build_tax_quarantine(profile, row, reason, detail)


def _wallet_is_altbestand(wallet):
    return parse_bool(wallet.get("altbestand"), default=False)


def _rp2_asset_state(profile, normalized_inputs: NormalizedTaxAssetInputs, configuration):
    """Build RP2 ``ComputedData`` for one asset across every wallet in the profile."""

    modules = get_rp2_modules()
    TransactionSet = modules["TransactionSet"]
    InTransaction = modules["InTransaction"]
    OutTransaction = modules["OutTransaction"]
    IntraTransaction = modules["IntraTransaction"]
    InputData = modules["InputData"]
    BalanceSet = modules["BalanceSet"]
    compute_tax = modules["compute_tax"]
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
    row_by_id = dict(normalized_inputs.row_by_id)
    events_by_id = {event.transaction_id: event for event in normalized_inputs.events}
    transfers_by_id = {transfer.out_transaction_id: transfer for transfer in normalized_inputs.transfers}

    for item_kind, item_id in normalized_inputs.ordered_items:
        if item_kind == "transfer":
            transfer = transfers_by_id[item_id]
            if total_available < transfer.sent:
                quarantines.append(
                    _rp2_quarantine(
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
                    _rp2_quarantine(
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
                    spot_price=rp2_decimal(transfer.spot_price if transfer.spot_price is not None else 0),
                    crypto_sent=rp2_decimal(transfer.sent),
                    crypto_received=rp2_decimal(transfer.received),
                    row=row_index,
                    unique_id=transfer.out_transaction_id,
                    notes=transfer.description,
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
                    transaction_type="BUY",
                    spot_price=rp2_decimal(event.spot_price),
                    crypto_in=rp2_decimal(event.amount),
                    fiat_in_no_fee=rp2_decimal(event.fiat_value),
                    fiat_in_with_fee=rp2_decimal(event.fiat_value),
                    fiat_fee=rp2_decimal(0),
                    row=row_index,
                    unique_id=event.transaction_id,
                    notes=event.description,
                )
            )
            priced_available += event.amount
            row_index += 1
            continue

        needed = event.amount + event.fee
        if needed <= 0:
            continue
        if total_available < needed:
            quarantines.append(
                _rp2_quarantine(
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
        if priced_available < needed:
            quarantines.append(
                _rp2_quarantine(
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
                spot_price=rp2_decimal(event.spot_price),
                crypto_out_no_fee=rp2_decimal(event.amount),
                crypto_fee=rp2_decimal(event.fee),
                fiat_out_no_fee=rp2_decimal(event.fiat_value) if event.amount > 0 else None,
                fiat_fee=rp2_decimal(event.fee * event.spot_price),
                row=row_index,
                unique_id=event.transaction_id,
                notes=event.description,
            )
        )
        total_available -= needed
        priced_available -= needed
        row_index += 1

    if in_set.count == 0:
        return None, quarantines, row_by_id, intra_audit, None
    input_data = InputData(
        asset=asset,
        unfiltered_in_transaction_set=in_set,
        unfiltered_out_transaction_set=out_set,
        unfiltered_intra_transaction_set=intra_set,
    )
    try:
        computed_data = compute_tax(configuration, build_rp2_accounting_engine(profile), input_data)
    except Exception as exc:
        raise AppError(f"RP2 tax calculation failed for asset '{asset}': {exc}") from exc
    from datetime import date as _date

    balance_set = BalanceSet(configuration, input_data, _date.max)
    return computed_data, quarantines, row_by_id, intra_audit, balance_set


def _append_rp2_journal_entries(entries, computed_data, wallet_refs_by_label, profile, row_by_id, intra_audit):
    altbestand_by_label = {label: _wallet_is_altbestand(ref) for label, ref in wallet_refs_by_label.items()}

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
                "fiat_value": dec(transaction.fiat_in_with_fee),
                "unit_cost": dec(transaction.fiat_in_with_fee) / dec(transaction.crypto_in),
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
        is_intra = (
            taxable_event.unique_id in audit_by_out_id
            and taxable_event.asset == computed_data.asset
            and taxable_event.transaction_type.value.lower() == "move"
        )
        if is_intra:
            entry_type = "transfer_fee"
        elif taxable_event.transaction_type.value == "FEE":
            entry_type = "fee"
        else:
            entry_type = "disposal"
        event = realized_by_event.setdefault(
            taxable_event.internal_id,
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
            },
        )
        event["quantity"] += dec(gain_loss.crypto_amount)
        event["cost_basis"] += dec(gain_loss.fiat_cost_basis)
        event["proceeds"] += dec(gain_loss.taxable_event_fiat_amount_with_fee_fraction)
        event["gain_loss"] += dec(gain_loss.fiat_gain)
    for event in realized_by_event.values():
        wallet = event["wallet"]
        altbestand = altbestand_by_label.get(wallet["label"], False)
        description = event["description"]
        proceeds = event["proceeds"]
        cost_basis = event["cost_basis"]
        gain_loss = event["gain_loss"]
        if altbestand:
            description = f"{description} [Altbestand tax-free]"
            if event["entry_type"] in ("fee", "transfer_fee"):
                proceeds = Decimal("0")
                cost_basis = Decimal("0")
            else:
                cost_basis = proceeds
            gain_loss = Decimal("0")
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "transaction_id": event["transaction_id"],
                "wallet_id": wallet["id"],
                "account_id": wallet["wallet_account_id"],
                "occurred_at": event["occurred_at"],
                "entry_type": event["entry_type"],
                "asset": event["asset"],
                "quantity": -event["quantity"],
                "fiat_value": proceeds,
                "unit_cost": Decimal("0"),
                "cost_basis": cost_basis,
                "proceeds": proceeds,
                "gain_loss": gain_loss,
                "description": description,
            }
        )

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
        total_cost_basis += dec(transaction.fiat_in_with_fee) * remaining_ratio
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


class GenericRP2TaxEngine:
    """Current generic RP2-backed implementation behind the engine seam."""

    def __init__(self, profile: Mapping[str, Any]):
        self.profile = profile

    def make_configuration(self, wallet_labels: Iterable[str], assets: Iterable[str]):
        return make_rp2_configuration(self.profile, wallet_labels, assets)

    def process_asset(
        self,
        normalized_inputs: NormalizedTaxAssetInputs,
        wallet_refs_by_label: Mapping[str, Mapping[str, Any]],
        configuration: Any,
    ) -> TaxEngineAssetResult:
        computed_data, quarantines, row_by_id, intra_audit, balance_set = _rp2_asset_state(
            self.profile,
            normalized_inputs,
            configuration,
        )
        if computed_data is None:
            return TaxEngineAssetResult(
                entries=[],
                quarantines=quarantines,
                intra_audit=intra_audit,
                account_holdings={},
                wallet_holdings={},
            )
        entries = []
        account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        _append_rp2_journal_entries(entries, computed_data, wallet_refs_by_label, self.profile, row_by_id, intra_audit)
        _accumulate_asset_holdings(account_holdings, wallet_holdings, computed_data, balance_set, wallet_refs_by_label)
        return TaxEngineAssetResult(
            entries=entries,
            quarantines=quarantines,
            intra_audit=intra_audit,
            account_holdings=dict(account_holdings),
            wallet_holdings=dict(wallet_holdings),
        )


__all__ = [
    "GenericRP2TaxEngine",
    "build_rp2_accounting_engine",
    "get_rp2_modules",
    "make_rp2_configuration",
    "rp2_decimal",
]
