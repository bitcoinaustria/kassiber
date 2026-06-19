from __future__ import annotations

import os
import sys
import tempfile
import uuid
from collections import defaultdict
from contextlib import contextmanager
from dataclasses import dataclass
from decimal import Decimal
from importlib import import_module
from pathlib import Path
from typing import Any, Iterable, Iterator, Mapping, Sequence

from ...errors import AppError
from ...msat import btc_to_msat, dec, msat_to_btc
from ...tax_policy import build_tax_policy
from ...transfers import apply_manual_pairs, detect_intra_transfers
from .. import pricing
from ..ownership_transfers import derive_ownership_transfers
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
_NON_REPORTABLE_AT_CATEGORY_OVERRIDES = {"alt_taxfree", "neu_swap"}
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
# Inbound kinds that look like income but are NOT in the map above. Defaulting
# them to BUY (a plain acquisition) silently drops the income declaration, so
# they are quarantined for explicit income-vs-acquisition classification.
_INCOME_LIKE_KIND_TOKENS = (
    "reward",
    "referral",
    "bonus",
    "cashback",
    "rebate",
    "dividend",
    "yield",
)
# Outbound kinds that are dispositions but NOT market sales. RP2 taxes every
# OutTransaction at full market value, so booking a gift/donation/lost coin as a
# SELL overstates gains. Quarantine for explicit handling (exclude, or apply a
# taxability/category override) instead of guessing the jurisdiction's rule.
_NON_SALE_DISPOSAL_KIND_TOKENS = (
    "gift",
    "donat",
    "lost",
    "stolen",
    "theft",
)


def _kind_has_token(kind: str, tokens: tuple[str, ...]) -> bool:
    return bool(kind) and any(token in kind for token in tokens)


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


def _prime_rp2_logger() -> None:
    # ``rp2.logger`` binds a ``logging.FileHandler`` to ``./log/rp2_<ts>.log``
    # at import time. In packaged macOS builds the daemon's cwd is the bundle's
    # read-only ``Contents/Resources`` directory, so the first rp2 import
    # crashes with EACCES. Trigger that import under a writable scratch cwd so
    # the handler binds to a writable file; subsequent rp2 imports reuse the
    # cached module.
    #
    # ``os.chdir`` is process-wide: any concurrent thread that reads cwd during
    # this window sees the scratch dir. The daemon is single-threaded for
    # request handling and priming happens once per process on the first
    # rp2-needing request, so this is safe today. If report generation is ever
    # parallelized, gate this behind a lock or move the chdir to daemon
    # startup.
    if "rp2.logger" in sys.modules:
        return
    scratch = Path(tempfile.gettempdir()) / "kassiber-rp2-logs"
    scratch.mkdir(parents=True, exist_ok=True)
    previous_cwd: str | None
    try:
        previous_cwd = os.getcwd()
    except OSError:
        previous_cwd = None
    try:
        os.chdir(scratch)
        import_module("rp2.logger")
    finally:
        if previous_cwd is not None:
            try:
                os.chdir(previous_cwd)
            except OSError:
                os.chdir(tempfile.gettempdir())


def _get_rp2_modules() -> dict[str, Any]:
    global _RP2_MODULES
    if _RP2_MODULES is not None:
        return _RP2_MODULES
    try:
        _prime_rp2_logger()
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
    return _rp2_transaction_type_value(transaction_type) in _RP2_EARN_TRANSACTION_TYPES


def _rp2_transaction_type_value(transaction_type: Any) -> str:
    value = getattr(transaction_type, "value", transaction_type)
    return str(value or "").strip().lower()


def _capital_gains_type(gain_loss: Any) -> str:
    is_long = getattr(gain_loss, "is_long_term_capital_gains", False)
    if callable(is_long):
        is_long = is_long()
    return "long" if bool(is_long) else "short"


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
        transfer_id = _transfer_item_id(transfer)
        if transfer_id != transfer.out_transaction_id:
            rows_by_id[transfer_id] = {
                **dict(transfer.out_row),
                "journal_transaction_id": transfer.out_transaction_id,
            }
        rows_by_id[transfer.out_transaction_id] = transfer.out_row
        rows_by_id[transfer.in_transaction_id] = transfer.in_row
    return rows_by_id


def _transfer_item_id(transfer: NormalizedTaxTransfer) -> str:
    return str(transfer.transfer_id or transfer.out_transaction_id)


def _row_get(row: Mapping[str, Any] | None, key: str, default: Any = None) -> Any:
    if row is None:
        return default
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def _journal_transaction_id(row: Mapping[str, Any] | None, fallback: str) -> str:
    return str(_row_get(row, "journal_transaction_id", fallback))


def _transaction_row_sort_key(row: Mapping[str, Any]) -> tuple[str, str, str]:
    return (
        str(_row_get(row, "occurred_at", "")),
        str(_row_get(row, "created_at", "")),
        str(_row_get(row, "id", "")),
    )


def _earliest_lot_contamination(dropped_acquisition_at, events) -> str | None:
    """Earliest instant the asset's lot state becomes uncertain.

    Combines normalize's earliest dropped-acquisition timestamp (missing/coarse
    pricing) with the gate-level drops that also leave RP2's lots inconsistent:
    unclassified-income inbounds (never enter the lot pool) and quarantined
    non-sale disposals (gift / donation / lost — a real outflow that is not
    booked). Any same-asset disposal at or after this instant cannot be trusted
    to select the right lot under any accounting method. ``None`` when nothing
    contaminates the lots.
    """
    earliest = dropped_acquisition_at
    for event in events:
        kind = _normalized_event_kind(event)
        contaminates = (
            event.direction == "inbound"
            and kind not in _RP2_INBOUND_KIND_TO_TRANSACTION_TYPE
            and _kind_has_token(kind, _INCOME_LIKE_KIND_TOKENS)
        ) or (
            event.direction == "outbound"
            and _kind_has_token(kind, _NON_SALE_DISPOSAL_KIND_TOKENS)
        )
        if contaminates and (earliest is None or event.occurred_at < earliest):
            earliest = event.occurred_at
    return earliest


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
    # Quantity balances are tracked PER ACCOUNT (wallet label; holder is constant
    # within a profile) to mirror rp2's BalanceSet, which enforces non-negative
    # balances per (exchange, holder). The old single global pool let an
    # account-local over-sell pass this gate and then crash compute_tax with an
    # uncatchable "balance went negative", aborting the whole report instead of
    # quarantining the one offending row.
    account_available: dict[str, Decimal] = defaultdict(lambda: Decimal("0"))
    # Cost basis, by contrast, is assigned by rp2's universal-application FIFO
    # GLOBALLY across accounts, so the priced/cost-basis pool stays global.
    priced_available = Decimal("0")
    # Basis-provenance guard. When the lot state for this asset becomes
    # uncertain — an acquisition dropped for missing/coarse pricing, an
    # unclassified-income inbound, or a quarantined non-sale disposal (gift /
    # donation / lost) that is NOT booked into RP2's lots — every later disposal
    # is computed against a lot the engine can no longer trust. Which lot is
    # wrong depends on the accounting method (FIFO picks an older lot,
    # moving-average folds the missing lot into the pool, LIFO/HIFO pick
    # differently), so rather than assume FIFO we conservatively quarantine ANY
    # same-asset disposal at or after the earliest contamination instant. The
    # user resolves the contaminating row (price it, classify it, or exclude it)
    # and re-runs. `_earliest_lot_contamination` folds the gate-level income /
    # gift drops in with normalize's quarantine-derived contamination instant.
    first_drop_at = _earliest_lot_contamination(
        normalized_inputs.earliest_lot_contamination_at,
        normalized_inputs.events,
    )
    quarantines = list(normalized_inputs.quarantines)
    intra_audit = []
    row_index = 1
    events_by_id = {event.transaction_id: event for event in normalized_inputs.events}
    transfers_by_id = {
        _transfer_item_id(transfer): transfer
        for transfer in normalized_inputs.transfers
    }

    # Walk the ledger in rp2's own order — by timestamp, and within a timestamp
    # IN before INTRA before OUT — so this gate's availability decision matches
    # the BalanceSet (which concatenates IN+INTRA+OUT then stable-sorts by
    # timestamp). The original-stream index is the final, deterministic tiebreak,
    # replacing the previous nondeterministic random-uuid order that could flip a
    # same-timestamp buy/sell between re-imports. row_index then follows the same
    # order rp2 itself processes in.
    def _gate_order_key(indexed_item):
        index, (kind, ident) = indexed_item
        if kind == "transfer":
            return (transfers_by_id[ident].occurred_at, 1, index)
        ev = events_by_id[ident]
        return (ev.occurred_at, 0 if ev.direction == "inbound" else 2, index)

    for _, (item_kind, item_id) in sorted(
        enumerate(normalized_inputs.ordered_items), key=_gate_order_key
    ):
        if item_kind == "transfer":
            transfer = transfers_by_id[item_id]
            transfer_id = _transfer_item_id(transfer)
            from_account = transfer.from_wallet_label
            to_account = transfer.to_wallet_label
            if account_available[from_account] < transfer.sent:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        transfer.out_row,
                        "insufficient_lots",
                        {
                            "from_wallet": transfer.from_wallet_label,
                            "asset": asset,
                            "required": float(transfer.sent),
                            "available": float(account_available[from_account]),
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
                    unique_id=transfer_id,
                    notes=_compose_transfer_notes(transfer),
                )
            )
            row_index += 1
            # The transfer debits `sent` from the source account and credits
            # `received` to the destination; the difference (the fee) is the only
            # quantity that leaves the global priced pool. Matches BalanceSet.
            account_available[from_account] -= transfer.sent
            account_available[to_account] += transfer.received
            priced_available -= transfer.fee
            audit_row = {
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
            if transfer_id != transfer.out_transaction_id:
                audit_row["rp2_unique_id"] = transfer_id
            intra_audit.append(audit_row)
            continue

        event = events_by_id[item_id]
        if event.direction == "inbound":
            kind = _normalized_event_kind(event)
            if (
                kind not in _RP2_INBOUND_KIND_TO_TRANSACTION_TYPE
                and _kind_has_token(kind, _INCOME_LIKE_KIND_TOKENS)
            ):
                # Looks like income but isn't a recognized earn type. Defaulting
                # to BUY would silently drop the income declaration, so quarantine
                # for explicit income-vs-acquisition classification.
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        event.raw_row,
                        "unclassified_income_kind",
                        {
                            "wallet": event.wallet_label,
                            "asset": asset,
                            "direction": "inbound",
                            "kind": kind,
                        },
                    )
                )
                continue
            account_available[event.wallet_label] += event.amount
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
        disposal_kind = _normalized_event_kind(event)
        if _kind_has_token(disposal_kind, _NON_SALE_DISPOSAL_KIND_TOKENS):
            # Gift / donation / lost-or-stolen: a disposition, but not a market
            # sale. rp2 taxes every OutTransaction at full market value, so
            # booking it as a SELL overstates gains. Quarantine for explicit
            # handling (exclude, or apply a taxability/category override).
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    event.raw_row,
                    "non_sale_disposal_kind",
                    {
                        "wallet": event.wallet_label,
                        "asset": asset,
                        "direction": "outbound",
                        "kind": disposal_kind,
                    },
                )
            )
            # Don't touch availability: the row isn't emitted, so RP2 keeps the
            # lot. Debiting here would diverge the gate from RP2 and over- or
            # under-gate a later sale depending on method. Instead the gift's
            # timestamp contaminates lot provenance (see _earliest_lot_contamination),
            # so any later disposal is quarantined until the gift is resolved.
            continue
        # Marked Austrian swap-outs can depend on another asset's same-timestamp
        # swap-in. Feed the full graph to rp2's native runner instead of applying
        # Kassiber's single-asset availability gate first.
        is_marked_at_swap = bool(getattr(event, "at_swap_link", None))
        if account_available[event.wallet_label] < needed and not is_marked_at_swap:
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    event.raw_row,
                    "insufficient_lots",
                    {
                        "wallet": event.wallet_label,
                        "asset": asset,
                        "required": float(needed),
                        "available": float(account_available[event.wallet_label]),
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
        if (
            first_drop_at is not None
            and not is_marked_at_swap
            and event.occurred_at >= first_drop_at
        ):
            # The lot state is contaminated from first_drop_at (a dropped /
            # unpriced acquisition, an unclassified income lot, or a quarantined
            # gift not booked into the pool). Which lot this disposal would draw
            # from is method-dependent and untrustworthy, so quarantine it until
            # the contaminating row is resolved rather than book a wrong basis.
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    event.raw_row,
                    "basis_provenance_incomplete",
                    {
                        "wallet": event.wallet_label,
                        "asset": asset,
                        "required": float(needed),
                        "lot_state_uncertain_since": first_drop_at,
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
        account_available[event.wallet_label] -= needed
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
                "transaction_id": _journal_transaction_id(source_row, transaction.unique_id),
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

    audit_by_rp2_id = {
        audit.get("rp2_unique_id", audit["out_id"]): audit
        for audit in intra_audit
    }
    realized_by_event = {}
    for gain_loss in computed_data.gain_loss_set:
        taxable_event = gain_loss.taxable_event
        wallet = _wallet_for(taxable_event)
        is_earn = _is_rp2_earn_transaction_type(taxable_event.transaction_type)
        is_intra = (
            taxable_event.unique_id in audit_by_rp2_id
            and taxable_event.asset == computed_data.asset
            and _rp2_transaction_type_value(taxable_event.transaction_type) == "move"
        )
        if is_earn:
            entry_type = "income"
        elif is_intra:
            entry_type = "transfer_fee"
        elif _rp2_transaction_type_value(taxable_event.transaction_type) == "fee":
            entry_type = "fee"
        else:
            entry_type = "disposal"
        capital_gains_type = _capital_gains_type(gain_loss)
        at_category = None
        at_kennzahl = None
        event_key: Any = (taxable_event.internal_id, capital_gains_type)
        source_row = row_by_id.get(taxable_event.unique_id)
        if tax_country == "at":
            at_category, at_kennzahl = _classify_at_disposal(gain_loss)
            category_override = _row_get(source_row, "at_category_override") if source_row else None
            taxability_override = _row_get(source_row, "taxability_override") if source_row else None
            if taxability_override == 0 and category_override in _NON_REPORTABLE_AT_CATEGORY_OVERRIDES:
                at_category = str(category_override)
                at_kennzahl = kennzahl_for_disposal_category(at_category)
            elif taxability_override == 0 or category_override == "none":
                at_category = None
                at_kennzahl = None
            elif category_override:
                at_category = str(category_override)
                at_kennzahl = kennzahl_for_disposal_category(at_category)
            # One taxable event can split across multiple Austrian semantic
            # buckets when RP2 matches against heterogeneous acquired lots, so
            # keep separate journal rows per category.
            event_key = (taxable_event.internal_id, capital_gains_type, at_category)
        event = realized_by_event.setdefault(
            event_key,
            {
                "transaction_id": _journal_transaction_id(row_by_id.get(taxable_event.unique_id), taxable_event.unique_id),
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
                "capital_gains_type": capital_gains_type,
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
        entry["capital_gains_type"] = event["capital_gains_type"]
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


def _validate_prepared_rp2_inputs(configuration: Any, input_data_list: list[Any]) -> None:
    """Run the country-level pre-accounting validation hook over all assets."""

    validator = getattr(configuration.country, "validate_input_data", None)
    if validator is None:
        raise AppError(
            "Installed rp2 is missing `AbstractCountry.validate_input_data`. "
            "Cross-asset swap-link validation requires bitcoinaustria/rp2 PR #4 or later.",
            code="unsupported",
            hint="Run `uv sync --refresh-package rp2` (or reinstall rp2 from the pin in pyproject.toml).",
        )
    try:
        validator(input_data_list)
    except Exception as exc:
        raise AppError(
            f"RP2 cross-asset input validation failed: {exc}",
            code="rp2_input_validation",
        ) from exc


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
            pair_id = str(pair.get("pair_id") or f"{out_id}->{in_id}")
            if pair_id.startswith("direct-payout:"):
                if out_id not in quarantine_reasons:
                    quarantines.append(
                        build_tax_quarantine(
                            profile,
                            rows_by_id[out_id],
                            AT_SWAP_QUARANTINE_REASON,
                            {
                                "outgoing_asset": pair.get("out_asset") or rows_by_id[out_id]["asset"],
                                "incoming_asset": pair.get("in_asset") or rows_by_id[in_id]["asset"],
                                "at_swap_link": pair_id,
                                "reason_code": reason_code,
                            },
                        )
                    )
            else:
                quarantines.extend(_swap_pair_quarantines(profile, pair, rows_by_id, reason_code))
            quarantined_row_ids.update({out_id, in_id})
            continue

        if getattr(out_event, "at_regime", None) != REGIME_NEU:
            continue

        pair_id = str(pair.get("pair_id") or f"{out_id}->{in_id}")
        swap_link_by_row_id[out_id] = pair_id
        swap_link_by_row_id[in_id] = pair_id

    return swap_link_by_row_id, quarantined_row_ids, quarantines


def _fiat_value_from_row(row: Mapping[str, Any]) -> Decimal | None:
    value = _row_get(row, "fiat_value_exact")
    if value is not None:
        return dec(value)
    value = _row_get(row, "fiat_value")
    if value is not None:
        return dec(value)
    return None


def _direct_payout_value(record: Mapping[str, Any], out_row: Mapping[str, Any]) -> Decimal | None:
    value = _row_get(record, "payout_fiat_value")
    if value is not None:
        return dec(value)
    row_value = _fiat_value_from_row(out_row)
    if row_value is not None:
        return row_value
    rate = _row_get(out_row, "fiat_rate_exact")
    if rate in (None, ""):
        rate = _row_get(out_row, "fiat_rate")
    amount_msat = int(_row_get(out_row, "amount") or 0)
    if rate in (None, "") or amount_msat <= 0:
        return None
    return dec(rate) * msat_to_btc(amount_msat)


def _direct_payout_proceeds_row(
    record: Mapping[str, Any],
    out_row: Mapping[str, Any],
) -> Mapping[str, Any]:
    """Apply reviewed external payout proceeds to the taxable source row.

    Direct payout reviews are not Austrian-only. The Austrian-only part is the
    cross-asset carrying-value synthesis below; ordinary taxable direct payouts
    still need the reviewed payout value to drive disposal proceeds.
    """

    payout_value = _row_get(record, "payout_fiat_value")
    if payout_value in (None, ""):
        return out_row
    payout_value = dec(payout_value)
    amount = msat_to_btc(int(out_row["amount"] or 0))
    if amount <= 0 or payout_value <= 0:
        return out_row
    reviewed = dict(out_row)
    payout_rate = payout_value / amount
    reviewed.update(
        {
            "fiat_rate": float(payout_rate),
            "fiat_value": float(payout_value),
            "fiat_rate_exact": str(payout_rate),
            "fiat_value_exact": str(payout_value),
            "pricing_source_kind": pricing.SOURCE_MANUAL_OVERRIDE,
            "pricing_quality": pricing.QUALITY_EXACT,
            "note": _row_get(record, "notes") or _row_get(out_row, "note"),
        }
    )
    return reviewed


def _direct_payout_record_out_amount(
    record: Mapping[str, Any],
    out_row: Mapping[str, Any],
) -> int:
    raw = _row_get(record, "out_amount")
    if raw in (None, ""):
        return int(out_row["amount"] or 0)
    return int(raw)


def _split_review_source_row(
    out_row: Mapping[str, Any],
    amount_msat: int,
    *,
    row_id: str | None = None,
    external_id: str | None = None,
    kind: str | None = None,
) -> dict[str, Any]:
    full = int(out_row["amount"] or 0)
    base = dict(out_row)
    if (
        _row_get(out_row, "fiat_rate_exact") in (None, "")
        and _row_get(out_row, "fiat_rate") in (None, "")
        and full > 0
    ):
        fiat_value = _row_get(out_row, "fiat_value_exact") or _row_get(out_row, "fiat_value")
        if fiat_value not in (None, ""):
            unit_rate = format(Decimal(str(fiat_value)) / msat_to_btc(full), "f")
            base["fiat_rate"] = unit_rate
            base["fiat_rate_exact"] = unit_rate
    base["amount"] = amount_msat
    base["fiat_value"] = None
    base["fiat_value_exact"] = None
    if row_id is not None:
        base["id"] = row_id
        base["journal_transaction_id"] = out_row["id"]
        base["fee"] = 0
    if external_id is not None:
        base["external_id"] = external_id
    if kind is not None:
        base["kind"] = kind
    return base


def _direct_payout_audit(record: Mapping[str, Any], out_row: Mapping[str, Any]) -> dict[str, Any]:
    payout_amount_msat = int(record["payout_amount"] or 0)
    return {
        "payout_id": str(record["id"]),
        "kind": record["kind"],
        "policy": record["policy"],
        "out_id": str(record["out_transaction_id"]),
        "out_asset": out_row["asset"],
        "out_amount_msat": _direct_payout_record_out_amount(record, out_row),
        "payout_asset": record["payout_asset"],
        "payout_amount_msat": payout_amount_msat,
        "payout_occurred_at": record["payout_occurred_at"] or out_row["occurred_at"],
        "payout_external_id": record["payout_external_id"],
        "counterparty": record["counterparty"],
        "swap_fee_msat": int(record["swap_fee_msat"] or 0),
        "swap_fee_kind": record["swap_fee_kind"],
    }


def _direct_payout_synthetic_rows(
    profile: Mapping[str, Any],
    rows: Sequence[Mapping[str, Any]],
    direct_payout_records: Sequence[Mapping[str, Any]],
) -> tuple[
    list[Mapping[str, Any]],
    list[dict[str, Any]],
    list[dict[str, Any]],
    list[dict[str, Any]],
    set[str],
]:
    """Return engine-only target legs for direct swap payouts.

    Cross-asset Austrian carrying-value payouts need two facts in RP2: a
    neutral swap into the payout asset, then an immediate external disposal.
    The synthetic rows are never persisted as transactions; journal entries
    produced from them point back to the real source row via
    ``journal_transaction_id``.
    """

    if not direct_payout_records:
        return list(rows), [], [], [], set()

    tax_country = _profile_str(profile, "tax_country").lower()
    rows_by_id = {str(row["id"]): row for row in rows}
    rows_for_engine: list[Mapping[str, Any]] = list(rows)
    row_overrides: dict[str, Mapping[str, Any]] = {}
    cross_asset_pairs: list[dict[str, Any]] = []
    direct_payouts: list[dict[str, Any]] = []
    quarantines: list[dict[str, Any]] = []
    blocked_row_ids: set[str] = set()

    for record in direct_payout_records:
        out_id = str(record["out_transaction_id"])
        out_row = rows_by_id.get(out_id)
        if out_row is None:
            continue
        reviewed_out_amount_msat = _direct_payout_record_out_amount(record, out_row)
        full_out_amount_msat = int(out_row["amount"] or 0)
        if reviewed_out_amount_msat <= 0 or reviewed_out_amount_msat > full_out_amount_msat:
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    out_row,
                    "direct_payout_out_amount_invalid",
                    {
                        "payout_id": record["id"],
                        "out_amount_msat": reviewed_out_amount_msat,
                        "full_out_amount_msat": full_out_amount_msat,
                    },
                )
            )
            blocked_row_ids.add(out_id)
            continue
        audit = _direct_payout_audit(record, out_row)
        direct_payouts.append(audit)
        payout_asset = str(record["payout_asset"])
        source_out_row = out_row
        source_is_synthetic = reviewed_out_amount_msat < full_out_amount_msat
        if source_is_synthetic:
            self_amount_msat = full_out_amount_msat - reviewed_out_amount_msat
            row_overrides[out_id] = _split_review_source_row(out_row, self_amount_msat)
            source_out_row = _split_review_source_row(
                out_row,
                reviewed_out_amount_msat,
                row_id=f"direct-payout:{record['id']}:source",
                external_id=f"direct-payout:{record['id']}:source",
                kind="sell",
            )
        is_cross_asset_carry = (
            tax_country == "at"
            and record["policy"] == "carrying-value"
            and out_row["asset"] != payout_asset
        )
        if not is_cross_asset_carry:
            proceeds_row = _direct_payout_proceeds_row(record, source_out_row)
            if source_is_synthetic:
                rows_for_engine.append(proceeds_row)
            else:
                row_overrides[out_id] = proceeds_row
            continue

        payout_amount_msat = int(record["payout_amount"] or 0)
        payout_amount = msat_to_btc(payout_amount_msat)
        payout_value = _direct_payout_value(record, source_out_row)
        if (
            payout_amount is None
            or payout_amount <= 0
            or payout_value is None
            or payout_value <= 0
        ):
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    out_row,
                    AT_SWAP_QUARANTINE_REASON,
                    {
                        "reason_code": "missing_payout_price",
                        "payout_id": record["id"],
                        "payout_asset": payout_asset,
                    },
                )
            )
            blocked_row_ids.add(out_id)
            continue

        if source_is_synthetic:
            rows_for_engine.append(source_out_row)
        payout_rate = payout_value / payout_amount
        payout_at = record["payout_occurred_at"] or out_row["occurred_at"]
        pair_id = f"direct-payout:{record['id']}"
        in_id = f"{pair_id}:in"
        out_virtual_id = f"{pair_id}:out"
        description = (
            record["notes"]
            or f"Direct swap payout to {record['counterparty'] or 'external recipient'}"
        )
        common = {
            "workspace_id": profile["workspace_id"],
            "profile_id": profile["id"],
            "wallet_id": source_out_row["wallet_id"],
            "wallet_label": _row_get(source_out_row, "wallet_label"),
            "wallet_account_id": _row_get(source_out_row, "wallet_account_id"),
            "account_code": _row_get(source_out_row, "account_code"),
            "account_label": _row_get(source_out_row, "account_label"),
            "asset": payout_asset,
            "amount": payout_amount_msat,
            "fee": 0,
            "fiat_rate": float(payout_rate),
            "fiat_value": float(payout_value),
            "fiat_rate_exact": str(payout_rate),
            "fiat_value_exact": str(payout_value),
            "pricing_source_kind": _row_get(out_row, "pricing_source_kind"),
            "pricing_quality": _row_get(out_row, "pricing_quality"),
            "created_at": record["created_at"],
            "journal_transaction_id": out_id,
            "note": description,
        }
        rows_for_engine.append(
            {
                **common,
                "id": in_id,
                "external_id": record["payout_external_id"],
                "occurred_at": payout_at,
                "direction": "inbound",
                "kind": "direct_swap_payout_in",
                "description": f"{description} (swap settlement)",
            }
        )
        rows_for_engine.append(
            {
                **common,
                "id": out_virtual_id,
                "external_id": record["payout_external_id"],
                "occurred_at": payout_at,
                "direction": "outbound",
                "kind": "sell",
                "description": f"{description} (external payout)",
            }
        )
        cross_asset_pairs.append(
            {
                "pair_id": pair_id,
                "kind": record["kind"],
                "policy": record["policy"],
                "out_id": str(source_out_row["id"]),
                "in_id": in_id,
                "out_asset": out_row["asset"],
                "in_asset": payout_asset,
            }
        )

    if row_overrides:
        rows_for_engine = [
            row_overrides.get(str(row["id"]), row)
            for row in rows_for_engine
        ]
    rows_for_engine = sorted(rows_for_engine, key=_transaction_row_sort_key)

    return rows_for_engine, cross_asset_pairs, direct_payouts, quarantines, blocked_row_ids


def _pair_record_out_amount(record: Mapping[str, Any]) -> Any:
    if hasattr(record, "keys"):
        return record["out_amount"] if "out_amount" in record.keys() else None
    return record.get("out_amount")


def _apply_cross_asset_splits(
    rows_for_engine: list[Mapping[str, Any]],
    manual_pair_records: Sequence[Mapping[str, Any]],
) -> tuple[list[Mapping[str, Any]], list[Mapping[str, Any]], dict[str, str]]:
    """Split one outbound across a same-asset self-transfer + a cross-asset peg.

    A cross-asset pair may carry only PART of its out leg (``out_amount`` < the
    row's amount) when a single spend both returned change to an owned wallet
    and pegged the rest to another asset. Reduce the real out row to the
    self-transfer remainder — so it auto-pairs with the same-txid change as a
    clean MOVE — and synthesize a separate outbound for the pegged portion that
    the cross-asset pair carries (carrying-value basis carry / taxable SELL+BUY,
    per the pair's policy). The pair record is rewritten to point at the
    synthetic leg, leaving the real reduced row free to auto-pair.

    No-ops for whole-row pairs (``out_amount`` unset / >= the row amount) and
    same-asset pairs. Returns ``(rows_for_engine, manual_pair_records,
    synthetic_out_id -> real_out_id)``; the first two are unchanged and the map
    empty when no split applies. The map lets callers show the real out tx in
    audits while the engine carries the synthetic leg.
    """
    rows_by_id = {str(row["id"]): row for row in rows_for_engine}
    overrides: dict[str, dict[str, Any]] = {}
    synthetic_rows: list[dict[str, Any]] = []
    rewritten: list[Mapping[str, Any]] = []
    out_id_to_real: dict[str, str] = {}
    changed = False
    for record in manual_pair_records:
        raw = _pair_record_out_amount(record)
        out_id = str(record["out_transaction_id"])
        in_id = str(record["in_transaction_id"])
        out_row = rows_by_id.get(out_id)
        in_row = rows_by_id.get(in_id)
        if (
            raw in (None, "")
            or out_row is None
            or in_row is None
            or out_row["asset"] == in_row["asset"]
        ):
            rewritten.append(record)
            continue
        peg = int(raw)
        full = int(out_row["amount"] or 0)
        self_amt = full - peg
        if peg <= 0 or self_amt <= 0:
            # Whole-row swap (or nothing left to self-transfer): existing path.
            rewritten.append(record)
            continue
        changed = True
        pair_id = (
            str(record["id"])
            if hasattr(record, "keys") and "id" in record.keys()
            else f"{out_id}->{in_id}"
        )
        # The per-unit rate survives the amount change, so each leg reprices from
        # its own amount once the (now-wrong) absolute fiat value is dropped. But
        # a row priced by value ALONE (no fiat_rate) would lose its only pricing
        # evidence — materialize a unit rate from value/amount first so it doesn't
        # become a false missing-price quarantine.
        base = dict(out_row)
        if (
            _row_get(out_row, "fiat_rate_exact") in (None, "")
            and _row_get(out_row, "fiat_rate") in (None, "")
            and full > 0
        ):
            fiat_value = _row_get(out_row, "fiat_value_exact") or _row_get(out_row, "fiat_value")
            if fiat_value not in (None, ""):
                # format("f") keeps plain decimal notation (no "6E+3") so the
                # derived rate matches how exact rates are stored elsewhere.
                unit_rate = format(Decimal(str(fiat_value)) / msat_to_btc(full), "f")
                base["fiat_rate"] = unit_rate
                base["fiat_rate_exact"] = unit_rate
        base["fiat_value"] = None
        base["fiat_value_exact"] = None
        overrides[out_id] = {**base, "amount": self_amt}
        swap_out_id = f"cross-split:{pair_id}:out"
        out_id_to_real[swap_out_id] = out_id
        synthetic_rows.append(
            {
                **base,
                "id": swap_out_id,
                "amount": peg,
                "fee": 0,
                "external_id": f"cross-split:{pair_id}",
                "direction": "outbound",
                "kind": "sell",
                "journal_transaction_id": out_id,
            }
        )
        rewritten.append({**dict(record), "out_transaction_id": swap_out_id})
    if not changed:
        return rows_for_engine, list(manual_pair_records), {}
    new_rows = [overrides.get(str(row["id"]), row) for row in rows_for_engine]
    new_rows.extend(synthetic_rows)
    return new_rows, rewritten, out_id_to_real


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
                direct_swap_payouts=[],
                tax_summary=[],
                account_holdings={},
                wallet_holdings={},
            )

        (
            rows_for_engine,
            payout_pairs,
            direct_payouts,
            payout_quarantines,
            blocked_payout_row_ids,
        ) = _direct_payout_synthetic_rows(
            self.profile,
            inputs.rows,
            inputs.direct_payout_records,
        )
        wallet_labels = {row["wallet_label"] for row in rows_for_engine}
        # The ownership deriver can route a MOVE into a wallet that recorded no
        # rows (sync gap); declare every profile wallet as an RP2 exchange so the
        # synthesized destination leg lands on a known account.
        wallet_labels |= {
            ref["label"]
            for ref in inputs.wallet_refs_by_id.values()
            if ref.get("label")
        }
        assets = {row["asset"] for row in rows_for_engine}
        entries: list[dict[str, Any]] = []
        quarantines: list[dict[str, Any]] = list(payout_quarantines)
        intra_audit_all: list[dict[str, Any]] = []
        account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        engine_cross_asset_pairs: list[dict[str, Any]] = list(payout_pairs)
        manual_cross_asset_pairs_all: list[dict[str, Any]] = []
        tax_summary_all: list[dict[str, Any]] = []
        with _rp2_configuration(self.profile, wallet_labels, assets) as configuration:
            wallet_refs_by_label = {
                ref["label"]: ref for ref in inputs.wallet_refs_by_id.values()
            }
            # Split spends (part same-asset self-transfer, part cross-asset peg)
            # are decomposed before pairing: the real out row is reduced to the
            # self-transfer remainder (auto-pairs with its change) and a synthetic
            # outbound carries the pegged portion into the cross-asset pair.
            rows_for_engine, split_pair_records, split_out_id_to_real = _apply_cross_asset_splits(
                rows_for_engine,
                inputs.manual_pair_records,
            )
            auto_pairs, auto_matched_ids = detect_intra_transfers(rows_for_engine)
            # Address-ownership deriver: prove self-transfers from the on-chain
            # graph (output paid an address owned by another of my wallets), for
            # the cases same-txid row matching misses — sync-gap destinations,
            # mismatched txids, and 1->N fan-outs. Supplements (does not replace)
            # detect_intra_transfers, which still covers same-txid rows the
            # deriver can't read (CSV imports with no transaction JSON).
            already_paired_ids = set(auto_matched_ids)
            for record in split_pair_records:
                already_paired_ids.add(str(record["out_transaction_id"]))
                already_paired_ids.add(str(record["in_transaction_id"]))
            # Direct-payout sources are already decomposed by
            # _direct_payout_synthetic_rows (the out row is reduced in place and
            # carries the real txid in raw_json); keep the deriver from
            # re-splitting them.
            for record in inputs.direct_payout_records:
                already_paired_ids.add(str(record["out_transaction_id"]))
            already_paired_ids |= {str(rid) for rid in blocked_payout_row_ids}
            ownership_result = derive_ownership_transfers(
                rows_for_engine,
                index=inputs.owned_index,
                wallet_refs_by_id=inputs.wallet_refs_by_id,
                already_paired_ids=already_paired_ids,
            )
            if ownership_result.dropped_out_ids or ownership_result.out_row_overrides:
                rows_for_engine = [
                    ownership_result.out_row_overrides.get(str(row["id"]), row)
                    for row in rows_for_engine
                    if str(row["id"]) not in ownership_result.dropped_out_ids
                ]
            if ownership_result.synthetic_rows:
                rows_for_engine = sorted(
                    list(rows_for_engine) + ownership_result.synthetic_rows,
                    key=_transaction_row_sort_key,
                )
            all_pairs, manual_cross_asset_pairs = apply_manual_pairs(
                rows_for_engine,
                auto_pairs,
                split_pair_records,
            )
            all_pairs = all_pairs + ownership_result.derived_pairs
            # The engine carries the synthetic split leg (so it can mark the
            # cross-asset swap-out without touching the self-transfer remainder),
            # but the result/audit should reference the real out tx — map it back.
            manual_cross_asset_pairs_all = [
                {**pair, "out_id": split_out_id_to_real.get(str(pair["out_id"]), pair["out_id"])}
                for pair in manual_cross_asset_pairs
            ]
            engine_cross_asset_pairs.extend(manual_cross_asset_pairs)
            rows_by_asset = defaultdict(list)
            for row in rows_for_engine:
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
                excluded_row_ids=blocked_payout_row_ids,
            )
            swap_link_by_row_id, quarantined_row_ids, swap_quarantines = _select_at_cross_asset_swap_links(
                self.profile,
                engine_cross_asset_pairs,
                rows_for_engine,
                prepared_by_asset,
            )
            quarantines.extend(swap_quarantines)
            payout_synthetic_ids = {
                synthetic_id
                for pair in payout_pairs
                for synthetic_id in (str(pair["in_id"]), f"{pair['pair_id']}:out")
            }
            selected_payout_synthetic_ids = {
                synthetic_id
                for pair in payout_pairs
                if str(pair["in_id"]) in swap_link_by_row_id
                for synthetic_id in (str(pair["in_id"]), f"{pair['pair_id']}:out")
            }
            excluded_row_ids = blocked_payout_row_ids | quarantined_row_ids | (payout_synthetic_ids - selected_payout_synthetic_ids)
            if swap_link_by_row_id or excluded_row_ids:
                prepared_by_asset = _prepare_assets(
                    self.profile,
                    rows_by_asset,
                    inputs.wallet_refs_by_id,
                    pairs_by_asset,
                    configuration,
                    at_swap_link_by_row_id=swap_link_by_row_id,
                    excluded_row_ids=excluded_row_ids,
                )

            # Phase 2: cross-asset validation via the country hook. Catches invariants
            # (e.g. Austrian `at_swap_link` marker must appear on two different assets)
            # that Kassiber's annotator structurally cannot detect — a paired leg that
            # was never imported can't be annotated, so only a post-hoc scan sees it.
            # `validate_input_data` was added in bitcoinaustria/rp2 PR #4. If an older rp2
            # is installed (editable checkout, stale `uv sync`), fall through with a clear
            # upgrade hint rather than a confusing generic failure.
            input_data_list = [
                prepared.input_data
                for _, prepared in prepared_by_asset
                if prepared.input_data is not None
            ]
            _validate_prepared_rp2_inputs(configuration, input_data_list)

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
            cross_asset_pairs=manual_cross_asset_pairs_all,
            direct_swap_payouts=direct_payouts,
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
