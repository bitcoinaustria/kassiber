from __future__ import annotations

import logging
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
from ...transfers import is_bitcoin_rail_pair
from .. import pricing
from ..ownership_transfers import (
    detect_conflicting_spend_ids,
    detect_pending_onchain_ids,
)
from ..austrian import (
    AT_SWAP_QUARANTINE_REASON,
    REGIME_NEU,
    kennzahl_for_disposal_category,
)
from ..journal_markers import (
    MARKER_ALT_IN,
    MARKER_ALT_OUT,
    MARKER_POOL,
    MARKER_REGIME,
    MARKER_REGIME_BASIS,
    MARKER_SWAP_LINK,
    marker_token,
)
from ..tax_events import (
    NormalizedTaxAssetInputs,
    NormalizedTaxTransfer,
    build_tax_quarantine,
    normalize_tax_asset_inputs,
)
from ..loans import (
    CHANNEL_OPEN,
    LOCK_SUPPRESS_ROLES as _LOAN_LOCK_SUPPRESS_ROLES,
    RELEASE_SUPPRESS_ROLES as _LOAN_RELEASE_SUPPRESS_ROLES,
)
from .base import TaxEngineLedgerInputs, TaxEngineLedgerResult

_RP2_MODULES = None
GENERIC_BITCOIN_RAIL_QUARANTINE_REASON = "bitcoin_rail_carry_basis_unresolved"
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
_RP2_CONFIG_FORBIDDEN_CHARACTERS = frozenset(
    {",", "\n", "\r", "[", "]", "=", ":", "#", ";"}
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
        _disable_rp2_disk_logger(sys.modules["rp2.logger"])
        return
    # Use a process-private directory. CLI and daemon test processes can prime
    # RP2 concurrently; a shared scratch directory lets one process remove the
    # other's current working directory while ``rp2.logger`` is creating
    # ``./log``, which fails nondeterministically with ENOENT.
    scratch = Path(tempfile.mkdtemp(prefix="kassiber-rp2-logs-"))
    previous_cwd: str | None
    try:
        previous_cwd = os.getcwd()
    except OSError:
        previous_cwd = None
    try:
        os.chdir(scratch)
        logger_module = import_module("rp2.logger")
    finally:
        if previous_cwd is not None:
            try:
                os.chdir(previous_cwd)
            except OSError:
                os.chdir(tempfile.gettempdir())
    _disable_rp2_disk_logger(logger_module, cleanup_roots=(scratch / "log", scratch))


def _disable_rp2_disk_logger(
    logger_module: Any,
    *,
    cleanup_roots: Sequence[Path] = (),
) -> None:
    logger = getattr(logger_module, "LOGGER", logging.getLogger("rp2"))
    for handler in list(logger.handlers):
        log_file = getattr(handler, "baseFilename", None)
        logger.removeHandler(handler)
        try:
            handler.close()
        finally:
            if log_file:
                try:
                    Path(log_file).unlink()
                except OSError:
                    # Best-effort cleanup: a stale RP2 log file must not block reports.
                    pass
    if not any(isinstance(handler, logging.NullHandler) for handler in logger.handlers):
        logger.addHandler(logging.NullHandler())
    logger.propagate = False
    for root in cleanup_roots:
        try:
            root.rmdir()
        except OSError:
            # Best-effort cleanup: non-empty or already-removed scratch dirs are harmless.
            pass


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
        tokens.append(marker_token(MARKER_REGIME, regime))
    regime_basis = getattr(event, "at_regime_basis", None)
    if regime_basis:
        # Audit-only provenance of the regime choice (exercised Wahlrecht vs
        # forced); rp2 does not read it, the journal/GUI marker channel does.
        tokens.append(marker_token(MARKER_REGIME_BASIS, regime_basis))
    pool = getattr(event, "at_pool", None)
    if pool:
        tokens.append(marker_token(MARKER_POOL, pool))
    swap_link = getattr(event, "at_swap_link", None)
    if swap_link:
        tokens.append(marker_token(MARKER_SWAP_LINK, swap_link))
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


def _rp2_config_token(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise AppError(
            "RP2 configuration value is required",
            code="validation",
            details={"field": field},
            retryable=False,
        )
    forbidden = sorted(char for char in _RP2_CONFIG_FORBIDDEN_CHARACTERS if char in text)
    if forbidden:
        raise AppError(
            f"RP2 configuration {field} contains unsupported characters",
            code="validation",
            hint=(
                "Rename the wallet, profile, or asset label to remove commas, "
                "newlines, brackets, comments, or key delimiters before processing reports."
            ),
            details={"field": field, "value": text, "characters": forbidden},
            retryable=False,
        )
    return text


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
    # The MOVE's miner fee is a taxable disposal; under the Austrian
    # moving-average method rp2 needs its regime tag in notes or it aborts the
    # whole asset on "Ambiguous Austrian disposal" when both Alt and Neu lots
    # exist. Emit it first (matching _compose_event_notes' fixed marker order).
    regime = getattr(transfer, "at_regime", None)
    if regime:
        tokens.append(marker_token(MARKER_REGIME, regime))
    pool = getattr(transfer, "at_pool", None)
    if pool:
        tokens.append(marker_token(MARKER_POOL, pool))
    pairing_source = getattr(transfer, "pairing_source", None)
    if pairing_source == "ownership_derived":
        # Make the basis for the non-taxable treatment auditable: this leg was
        # proven from the on-chain transaction graph (an output paid an address
        # owned by another of the user's wallets), not a same-txid row match.
        tokens.append("self-transfer proven by address ownership")
    elif pairing_source == "multi_source_consolidation":
        # One leg of a consolidation funded by several owned wallets, split from
        # the graph + per-wallet rows so the network fee is booked once.
        tokens.append("self-transfer from multi-wallet consolidation")
    description = getattr(transfer, "description", "") or ""
    if description:
        tokens.append(description)
    return " ".join(tokens)


def _compose_transfer_journal_description(audit: Mapping[str, Any]) -> str:
    tokens: list[str] = []
    regime = audit.get("at_regime")
    if regime in ("alt", "neu"):
        tokens.append(marker_token(MARKER_REGIME, regime))
    flows = audit.get("regime_flows")
    if flows:
        # at_regime above describes only the FEE slice; a mixed move carries
        # lots from several regimes. Emit the tax-free (alt) quantity per side
        # so balance-hint consumers classify quantities, not whole MOVEs.
        tokens.append(
            marker_token(MARKER_ALT_OUT, int(flows.get("out", {}).get("alt", 0)))
        )
        tokens.append(
            marker_token(MARKER_ALT_IN, int(flows.get("in", {}).get("alt", 0)))
        )
    pool = audit.get("at_pool")
    if pool:
        tokens.append(marker_token(MARKER_POOL, pool))
    tokens.append(f"Transfer {audit['from_wallet_label']} -> {audit['to_wallet_label']}")
    pairing_source = audit.get("pairing_source")
    if pairing_source == "ownership_derived":
        # Record the basis for the non-taxable treatment on the entry itself,
        # so the audit/report shows this MOVE was proven from the on-chain
        # address graph rather than matched on a shared txid.
        tokens.append("(proven by address ownership)")
    elif pairing_source == "multi_source_consolidation":
        tokens.append("(multi-wallet consolidation)")
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
    sorted_wallet_labels = sorted(
        _rp2_config_token(wallet_label, "wallet_label") for wallet_label in wallet_labels
    )
    sorted_assets = sorted(_rp2_config_token(asset, "asset") for asset in assets)
    holder_label = _rp2_config_token(_profile_str(profile, "label"), "profile_label")
    if not sorted_wallet_labels:
        raise AppError("RP2 configuration requires at least one wallet")
    if not sorted_assets:
        raise AppError("RP2 configuration requires at least one asset")
    content = "\n".join(
        [
            "[general]",
            f"assets = {', '.join(sorted_assets)}",
            f"exchanges = {', '.join(sorted_wallet_labels)}",
            f"holders = {holder_label}",
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


def _ordered_rp2_items(
    normalized_inputs: NormalizedTaxAssetInputs,
) -> list[tuple[str, str]]:
    events_by_id = {event.transaction_id: event for event in normalized_inputs.events}
    transfers_by_id = {
        _transfer_item_id(transfer): transfer
        for transfer in normalized_inputs.transfers
    }
    by_time: dict[str, list[tuple[int, tuple[str, str]]]] = defaultdict(list)
    for index, (kind, ident) in enumerate(normalized_inputs.ordered_items):
        if kind == "transfer":
            occurred_at = transfers_by_id[ident].occurred_at
        else:
            occurred_at = events_by_id[ident].occurred_at
        by_time[str(occurred_at)].append((index, (kind, ident)))

    ordered: list[tuple[str, str]] = []
    for occurred_at in sorted(by_time):
        bucket = by_time[occurred_at]
        inbound_events: list[tuple[int, tuple[str, str]]] = []
        outbound_events: list[tuple[int, tuple[str, str]]] = []
        transfer_items: list[tuple[int, tuple[str, str]]] = []
        for indexed_item in bucket:
            _, (kind, ident) = indexed_item
            if kind == "transfer":
                transfer_items.append(indexed_item)
                continue
            event = events_by_id[ident]
            if event.direction == "inbound":
                inbound_events.append(indexed_item)
            else:
                outbound_events.append(indexed_item)
        ordered.extend(item for _, item in sorted(inbound_events))
        ordered.extend(_topologically_order_transfers(transfer_items, transfers_by_id))
        ordered.extend(item for _, item in sorted(outbound_events))
    return ordered


def _topologically_order_transfers(
    transfer_items: Sequence[tuple[int, tuple[str, str]]],
    transfers_by_id: Mapping[str, NormalizedTaxTransfer],
) -> list[tuple[str, str]]:
    if len(transfer_items) <= 1:
        return [item for _, item in transfer_items]

    original_order = {ident: index for index, (_, ident) in transfer_items}
    transfer_ids = [ident for _, (_, ident) in transfer_items]
    outgoing: dict[str, set[str]] = {ident: set() for ident in transfer_ids}
    incoming_count: dict[str, int] = {ident: 0 for ident in transfer_ids}
    for source_id in transfer_ids:
        source = transfers_by_id[source_id]
        for target_id in transfer_ids:
            if source_id == target_id:
                continue
            target = transfers_by_id[target_id]
            if source.to_wallet_id == target.from_wallet_id:
                if target_id not in outgoing[source_id]:
                    outgoing[source_id].add(target_id)
                    incoming_count[target_id] += 1

    ready = sorted(
        [ident for ident in transfer_ids if incoming_count[ident] == 0],
        key=lambda ident: original_order[ident],
    )
    ordered_ids: list[str] = []
    while ready:
        ident = ready.pop(0)
        ordered_ids.append(ident)
        for target_id in sorted(outgoing[ident], key=lambda tid: original_order[tid]):
            incoming_count[target_id] -= 1
            if incoming_count[target_id] == 0:
                ready.append(target_id)
        ready.sort(key=lambda tid: original_order[tid])

    if len(ordered_ids) != len(transfer_ids):
        return [item for _, item in sorted(transfer_items)]
    return [("transfer", ident) for ident in ordered_ids]


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
    ordered_rp2_items = _ordered_rp2_items(normalized_inputs)
    transfers_by_group: dict[str, list[NormalizedTaxTransfer]] = defaultdict(list)
    for item_kind, item_id in ordered_rp2_items:
        if item_kind != "transfer":
            continue
        transfer = transfers_by_id[item_id]
        if transfer.group_id:
            transfers_by_group[transfer.group_id].append(transfer)
    # Group gate state machine: absent -> not yet preflighted; "approved" ->
    # preflight passed, legs apply (and re-check) at their own positions;
    # "failed" -> preflight or a mid-group re-check failed, remaining legs skip.
    transfer_group_state: dict[str, str] = {}

    def _transfer_gate_failure(
        transfer: NormalizedTaxTransfer,
        account_state: Mapping[str, Decimal],
        priced_state: Decimal,
    ) -> tuple[str, dict[str, Any]] | None:
        from_account = transfer.from_wallet_label
        if account_state[from_account] < transfer.sent:
            return (
                "insufficient_lots",
                {
                    "from_wallet": transfer.from_wallet_label,
                    "asset": asset,
                    "required": float(transfer.sent),
                    "available": float(account_state[from_account]),
                },
            )
        if priced_state < transfer.sent:
            return (
                "missing_cost_basis",
                {
                    "from_wallet": transfer.from_wallet_label,
                    "asset": asset,
                    "required": float(transfer.sent),
                    "priced_available": float(priced_state),
                },
            )
        return None

    def _append_transfer_gate_quarantine(
        transfer: NormalizedTaxTransfer,
        reason: str,
        detail: Mapping[str, Any],
    ) -> None:
        quarantines.append(build_tax_quarantine(profile, transfer.out_row, reason, detail))
        out_transaction_id = str(
            _row_get(transfer.out_row, "journal_transaction_id")
            or transfer.out_row["id"]
        )
        in_transaction_id = str(
            _row_get(transfer.in_row, "journal_transaction_id")
            or transfer.in_row["id"]
        )
        if in_transaction_id == out_transaction_id:
            return
        quarantines.append(
            build_tax_quarantine(
                profile,
                transfer.in_row,
                reason,
                {
                    **dict(detail),
                    "paired_leg": "inbound",
                    "blocked_by_transaction_id": out_transaction_id,
                },
            )
        )

    def _append_group_gate_quarantines(
        group: Sequence[NormalizedTaxTransfer],
        failed_transfer: NormalizedTaxTransfer,
        reason: str,
        detail: Mapping[str, Any],
    ) -> None:
        group_id = str(failed_transfer.group_id)
        seen: set[str] = set()
        ordered_group = [failed_transfer] + [
            transfer for transfer in group if transfer is not failed_transfer
        ]
        for transfer in ordered_group:
            rows_to_flag = [
                ("out", transfer.out_row),
                ("in", transfer.in_row),
                *[
                    ("replaced", row)
                    for row in getattr(transfer, "group_block_rows", ())
                ],
            ]
            for side, row in rows_to_flag:
                transaction_id = str(_row_get(row, "journal_transaction_id") or row["id"])
                if transaction_id in seen:
                    continue
                seen.add(transaction_id)
                if transfer is failed_transfer and side == "out":
                    row_reason = reason
                    row_detail = {**dict(detail), "transfer_group_id": group_id}
                else:
                    row_reason = "derived_transfer_group_blocked"
                    row_detail = {
                        "asset": asset,
                        "from_wallet": transfer.from_wallet_label,
                        "to_wallet": transfer.to_wallet_label,
                        "direction": "transfer",
                        "paired_leg": side == "in",
                        "transfer_group_id": group_id,
                        "blocked_by_reason": reason,
                    }
                quarantines.append(
                    build_tax_quarantine(profile, row, row_reason, row_detail)
                )

    def _apply_transfer_to_rp2(transfer: NormalizedTaxTransfer) -> None:
        nonlocal priced_available, row_index
        from_account = transfer.from_wallet_label
        to_account = transfer.to_wallet_label
        transfer_id = _transfer_item_id(transfer)
        intra_set.add_entry(
            IntraTransaction(
                configuration=configuration,
                timestamp=transfer.occurred_at,
                asset=asset,
                from_exchange=transfer.from_wallet_label,
                from_holder=holder,
                to_exchange=transfer.to_wallet_label,
                to_holder=holder,
                spot_price=_rp2_decimal(
                    transfer.spot_price if transfer.spot_price is not None else 0
                ),
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
            # Synthetic ownership-derived legs are engine-local rows.  Keep
            # their real transaction anchors beside the display/audit ids so
            # the canonical quantity layer can tie the interpretation back to
            # immutable imported evidence without guessing from labels or
            # amounts.
            "out_anchor_transaction_id": str(
                _row_get(transfer.out_row, "journal_transaction_id")
                or transfer.out_transaction_id
            ),
            "in_anchor_transaction_id": str(
                _row_get(transfer.in_row, "journal_transaction_id")
                or transfer.in_transaction_id
            ),
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
            "crypto_sent_msat": btc_to_msat(transfer.sent),
            "crypto_received_msat": btc_to_msat(transfer.received),
            "crypto_fee_msat": btc_to_msat(transfer.fee),
            "spot_price": (
                float(transfer.spot_price) if transfer.spot_price is not None else 0.0
            ),
            # Provenance for the audit trail: "ownership_derived" when this
            # MOVE was proven from the on-chain graph rather than row-matched
            # (None for same-txid / manual pairs).
            "pairing_source": getattr(transfer, "pairing_source", None),
        }
        if getattr(transfer, "at_regime", None):
            audit_row["at_regime"] = transfer.at_regime
        if getattr(transfer, "regime_flows", None):
            audit_row["regime_flows"] = transfer.regime_flows
        if getattr(transfer, "at_pool", None):
            audit_row["at_pool"] = transfer.at_pool
        if transfer.group_id:
            audit_row["transfer_group_id"] = transfer.group_id
        if transfer_id != transfer.out_transaction_id:
            audit_row["rp2_unique_id"] = transfer_id
        intra_audit.append(audit_row)

    # Walk the ledger in rp2's own timestamp order, with inbounds before intra
    # transfers before outbounds at each instant. Same-timestamp transfer chains
    # are topologically ordered by wallet dependency (a MOVE into wallet X before
    # a same-time MOVE out of X) so this gate's availability decision and the
    # IntraTransaction insertion order both match the economic flow.
    for item_kind, item_id in ordered_rp2_items:
        if item_kind == "transfer":
            transfer = transfers_by_id[item_id]
            if transfer.group_id:
                group_id = str(transfer.group_id)
                state = transfer_group_state.get(group_id)
                if state == "approved":
                    # Preflight passed at the first leg; apply THIS leg at its
                    # own chronological position. Crediting the whole group at
                    # the first leg's position would let an intermediate
                    # destination spend pass this gate and then abort the whole
                    # asset inside rp2's strictly chronological BalanceSet
                    # (multi-timestamp manual N:1 groups). The preflight cannot
                    # see intermediate debits either, so re-check each leg at
                    # its own position: an intermediate spend that drained the
                    # source must downgrade the REST of the group to quarantine
                    # instead of driving the BalanceSet negative (earlier legs
                    # are already booked and stay booked).
                    failure = _transfer_gate_failure(
                        transfer, account_available, priced_available
                    )
                    if failure is not None:
                        reason, detail = failure
                        transfer_group_state[group_id] = "failed"
                        group = transfers_by_group[group_id]
                        position = next(
                            index
                            for index, leg in enumerate(group)
                            if leg is transfer
                        )
                        remaining = group[position:]
                        _append_group_gate_quarantines(
                            remaining, transfer, reason, detail
                        )
                        continue
                    _apply_transfer_to_rp2(transfer)
                    continue
                if state is not None:
                    continue
                transfer_group_state[group_id] = "failed"
                group = transfers_by_group[group_id]
                temp_account = defaultdict(lambda: Decimal("0"), account_available)
                temp_priced = priced_available
                failed: tuple[NormalizedTaxTransfer, str, dict[str, Any]] | None = None
                for candidate in group:
                    failure = _transfer_gate_failure(
                        candidate, temp_account, temp_priced
                    )
                    if failure is not None:
                        reason, detail = failure
                        failed = (candidate, reason, detail)
                        break
                    temp_account[candidate.from_wallet_label] -= candidate.sent
                    temp_account[candidate.to_wallet_label] += candidate.received
                    temp_priced -= candidate.fee
                if failed is not None:
                    failed_transfer, reason, detail = failed
                    _append_group_gate_quarantines(
                        group, failed_transfer, reason, detail
                    )
                    continue
                transfer_group_state[group_id] = "approved"
                _apply_transfer_to_rp2(transfer)
                continue

            failure = _transfer_gate_failure(
                transfer, account_available, priced_available
            )
            if failure is not None:
                reason, detail = failure
                _append_transfer_gate_quarantine(transfer, reason, detail)
                continue
            _apply_transfer_to_rp2(transfer)
            continue

        event = events_by_id[item_id]
        if event.direction == "inbound":
            if getattr(event, "loan_leg_role", None) in _LOAN_RELEASE_SUPPRESS_ROLES:
                # Collateral returning from escrow (repayment / recovery /
                # surplus): the coins re-enter the pool they never left, so
                # booking an acquisition would double-count. Non-event — skip
                # emission and leave availability untouched.
                continue
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
        loan_role = getattr(event, "loan_leg_role", None)
        channel_open_fee_only = loan_role == CHANNEL_OPEN and event.fee > 0
        if loan_role in _LOAN_LOCK_SUPPRESS_ROLES and not channel_open_fee_only:
            # Outbound loan non-events are not disposals: posted collateral stays
            # owned and principal repayment returns borrowed coins. Skip emission
            # and leave availability untouched. If collateral is liquidated
            # instead of returned, the user removes the mark and this outbound
            # books as the disposal it is.
            continue
        if channel_open_fee_only:
            # Funding a Lightning channel is not a disposal, but the L1 miner fee
            # did leave the owned BTC pool. Book only that fee; the funded
            # channel principal remains owned and keeps its basis.
            needed = event.fee
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
                transaction_type="SELL"
                if event.amount > 0 and not channel_open_fee_only
                else "FEE",
                spot_price=_rp2_decimal(event.spot_price),
                crypto_out_no_fee=_rp2_decimal(
                    0 if channel_open_fee_only else event.amount
                ),
                crypto_fee=_rp2_decimal(event.fee),
                fiat_out_no_fee=_rp2_decimal(event.fiat_value)
                if event.amount > 0 and not channel_open_fee_only
                else None,
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
        is_fee_entry = event["entry_type"] in {"fee", "transfer_fee"}
        if is_fee_entry:
            # Fees reduce the owned BTC balance and its attached basis, but they
            # are not capital-gains disposals in Kassiber's reporting model.
            proceeds = cost_basis
            gain_loss = Decimal("0")
            event["at_category"] = None
            event["at_kennzahl"] = None
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
        description = _compose_transfer_journal_description(audit)
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                # Synthetic legs (ownership-derived / split / payout) carry the
                # real source tx in journal_transaction_id; the journal entry's
                # transaction_id has an FK into transactions, so map through it
                # (real legs fall back to their own id).
                "transaction_id": _journal_transaction_id(
                    row_by_id.get(audit["out_id"]), audit["out_id"]
                ),
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
                "transaction_id": _journal_transaction_id(
                    row_by_id.get(audit["in_id"]), audit["in_id"]
                ),
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
        transaction_type = str(getattr(yearly.transaction_type, "value", yearly.transaction_type)).lower()
        if transaction_type in {"fee", "move"}:
            continue
        quantity = dec(yearly.crypto_amount)
        rows.append(
            {
                "year": int(yearly.year),
                "asset": yearly.asset,
                "transaction_type": transaction_type,
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
    loan_leg_by_transaction_id: Mapping[str, str] | None = None,
) -> list[tuple[NormalizedTaxAssetInputs, _RP2PreparedInput]]:
    prepared_by_asset: list[tuple[NormalizedTaxAssetInputs, _RP2PreparedInput]] = []
    excluded = excluded_row_ids or set()
    for asset, asset_rows in rows_by_asset.items():
        # Detect shared-prevout conflicts against the FULL asset row set, not the
        # post-exclusion active_rows: the Austrian second pass excludes more rows,
        # and if a confirmed conflict winner were dropped there the conflict would
        # vanish and the unconfirmed loser would book. Computing over asset_rows
        # keeps the loser identified (and quarantined) across both passes.
        conflict_row_ids = detect_conflicting_spend_ids(asset_rows)
        pending_onchain_row_ids = detect_pending_onchain_ids(asset_rows)
        active_rows = [row for row in asset_rows if str(row["id"]) not in excluded]
        normalized_inputs = normalize_tax_asset_inputs(
            profile,
            asset,
            active_rows,
            wallet_refs_by_id,
            pairs_by_asset.get(asset, []),
            at_swap_link_by_row_id=at_swap_link_by_row_id,
            loan_leg_by_transaction_id=loan_leg_by_transaction_id,
            conflict_row_ids=conflict_row_ids,
            pending_onchain_row_ids=pending_onchain_row_ids,
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

        # A leg Kassiber already quarantined in phase 1 (insufficient quantity,
        # missing basis, unclassified income, …) must NOT be promoted to an
        # at_swap_link. On the second prepare pass the marked row would bypass the
        # single-asset quantity gate (which defers to the native cross-asset
        # runner for at_swap rows) and carry the shortfall into compute_tax, where
        # rp2's per-account BalanceSet raises an uncatchable "balance went
        # negative" that aborts the WHOLE multi-asset report. Marking only the
        # surviving leg is no better — it orphans the at_swap_link and trips the
        # cross-asset validator instead. Quarantine the whole pair so neither leg
        # reaches compute, preserving the original phase-1 reason for review.
        leg_reason = quarantine_reasons.get(out_id) or quarantine_reasons.get(in_id)
        if leg_reason is not None:
            quarantines.extend(_swap_pair_quarantines(profile, pair, rows_by_id, leg_reason))
            quarantined_row_ids.update({out_id, in_id})
            continue

        if getattr(out_event, "at_regime", None) != REGIME_NEU:
            continue

        pair_id = str(pair.get("pair_id") or f"{out_id}->{in_id}")
        swap_link_by_row_id[out_id] = pair_id
        swap_link_by_row_id[in_id] = pair_id

    return swap_link_by_row_id, quarantined_row_ids, quarantines


def _direct_payout_compat_records(
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
        anchor = str(_row_get(row, "journal_transaction_id") or "")
        if anchor and _row_get(row, "direction") == "outbound":
            projected_by_anchor[anchor].append(row)
    result: list[dict[str, Any]] = []
    for record in records:
        payout_id = str(_row_get(record, "id") or "")
        out_id = str(_row_get(record, "out_transaction_id") or "")
        if not payout_id or not out_id:
            continue
        reviewed_amount = (
            _row_get(record, "out_amount")
            or _row_get(record, "out_amount_msat")
        )
        candidates = projected_by_anchor.get(out_id, [])
        reviewed_asset = str(_row_get(record, "out_asset") or "")
        if reviewed_asset:
            candidates = [
                row
                for row in candidates
                if str(_row_get(row, "asset") or "") == reviewed_asset
            ]
        if reviewed_amount not in (None, ""):
            candidates = [
                row
                for row in candidates
                if int(_row_get(row, "amount") or 0) == int(reviewed_amount)
            ]
        projected_source = candidates[0] if len(candidates) == 1 else {}
        result.append(
            {
                "payout_id": payout_id,
                "kind": _row_get(record, "kind"),
                "policy": _row_get(record, "policy"),
                "out_id": out_id,
                "out_asset": (
                    _row_get(record, "out_asset")
                    or _row_get(projected_source, "asset")
                ),
                "out_amount_msat": int(
                    _row_get(record, "out_amount")
                    or _row_get(record, "out_amount_msat")
                    or _row_get(projected_source, "amount")
                    or 0
                ),
                "payout_asset": _row_get(record, "payout_asset"),
                "payout_amount_msat": int(_row_get(record, "payout_amount") or 0),
                "payout_occurred_at": _row_get(record, "payout_occurred_at"),
                "payout_external_id": _row_get(record, "payout_external_id"),
                "counterparty": _row_get(record, "counterparty"),
                "swap_fee_msat": int(_row_get(record, "swap_fee_msat") or 0),
                "swap_fee_kind": _row_get(record, "swap_fee_kind"),
            }
        )
    return result


def _decimal_to_row_value(value: Decimal) -> str:
    return format(value.normalize(), "f")


def _with_carried_fiat_value(row: Mapping[str, Any], carried_basis: Decimal) -> dict[str, Any]:
    value = _decimal_to_row_value(carried_basis)
    quantity = msat_to_btc(int(_row_get(row, "amount") or 0))
    rate = _decimal_to_row_value(carried_basis / quantity) if quantity > 0 else None
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


def _computed_cost_basis_by_transaction_id(asset_states: Mapping[str, _RP2AssetState]) -> dict[str, Decimal]:
    basis_by_id: dict[str, Decimal] = defaultdict(lambda: Decimal("0"))
    for state in asset_states.values():
        computed_data = state.computed_data
        if computed_data is None:
            continue
        for gain_loss in computed_data.gain_loss_set:
            transaction_id = str(gain_loss.taxable_event.unique_id)
            basis_by_id[transaction_id] += dec(gain_loss.fiat_cost_basis)
    return dict(basis_by_id)


@dataclass(frozen=True)
class _GenericRailCarryResult:
    rows: list[Mapping[str, Any]]
    blocked_row_ids: set[str]
    quarantines: list[dict[str, Any]]


def _generic_bitcoin_rail_blocked_row_ids(pair: Mapping[str, Any]) -> set[str]:
    out_id = str(pair["out_id"])
    in_id = str(pair["in_id"])
    blocked = {out_id, in_id}
    pair_id = str(pair.get("pair_id") or "")
    if pair_id.startswith("direct-payout:"):
        blocked.add(f"{pair_id}:out")
    return blocked


def _generic_bitcoin_rail_pair_quarantines(
    profile: Mapping[str, Any],
    pair: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
    reason_code: str,
) -> list[dict[str, Any]]:
    out_id = str(pair["out_id"])
    in_id = str(pair["in_id"])
    pair_id = str(pair.get("pair_id") or f"{out_id}->{in_id}")
    detail = {
        "outgoing_asset": pair.get("out_asset") or rows_by_id.get(out_id, {}).get("asset"),
        "incoming_asset": pair.get("in_asset") or rows_by_id.get(in_id, {}).get("asset"),
        "rail_pair": pair_id,
        "policy": "carrying-value",
        "reason_code": reason_code,
    }
    quarantines: list[dict[str, Any]] = []
    for row_id in (out_id, in_id):
        row = rows_by_id.get(row_id)
        if row is not None:
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    row,
                    GENERIC_BITCOIN_RAIL_QUARANTINE_REASON,
                    detail,
                )
            )
    return quarantines


def _apply_generic_bitcoin_rail_carry_values(
    profile: Mapping[str, Any],
    rows_for_engine: list[Mapping[str, Any]],
    cross_asset_pairs: Sequence[Mapping[str, Any]],
    pairs_by_asset: Mapping[str, list[Mapping[str, Any]]],
    configuration: Any,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    *,
    excluded_row_ids: set[str],
    loan_leg_by_transaction_id: Mapping[str, str],
) -> _GenericRailCarryResult:
    """Carry BTC/LBTC basis for non-AT profiles without duplicating lot logic.

    Generic RP2 has no country-level multi-asset hook, so first let RP2 price the
    source disposal with the user's selected accounting method. The resulting
    source-leg cost basis is then used as both the neutral proceeds of the
    outgoing rail-change leg and the acquisition value of the incoming rail.
    """

    if _profile_str(profile, "tax_country").lower() == "at":
        return _GenericRailCarryResult(list(rows_for_engine), set(), [])
    rows_by_id = {str(row["id"]): row for row in rows_for_engine}
    eligible_pairs = [
        pair
        for pair in cross_asset_pairs
        if pair.get("policy") == "carrying-value"
        and is_bitcoin_rail_pair(pair.get("out_asset"), pair.get("in_asset"))
        and str(pair.get("out_id")) in rows_by_id
        and str(pair.get("in_id")) in rows_by_id
    ]
    if not eligible_pairs:
        return _GenericRailCarryResult(list(rows_for_engine), set(), [])

    current_rows = list(rows_for_engine)
    blocked_pair_reasons: dict[str, str] = {}
    blocked_row_ids: set[str] = set()
    # Chained rail moves need the second hop to see the first hop's carried
    # basis, not the first hop's market-priced acquisition. Recompute until the
    # BTC/LBTC carry overrides are stable.
    max_passes = max(1, len(eligible_pairs) + 1)
    for _ in range(max_passes):
        rows_by_id = {str(row["id"]): row for row in current_rows}
        rows_by_asset: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
        for row in current_rows:
            rows_by_asset[row["asset"]].append(row)
        prepared_by_asset = _prepare_assets(
            profile,
            rows_by_asset,
            wallet_refs_by_id,
            pairs_by_asset,
            configuration,
            excluded_row_ids=excluded_row_ids | blocked_row_ids,
            loan_leg_by_transaction_id=loan_leg_by_transaction_id,
        )
        asset_states = _rp2_asset_states_from_prepared(prepared_by_asset, profile, configuration)
        basis_by_id = _computed_cost_basis_by_transaction_id(asset_states)
        quarantine_reasons = _prepared_quarantine_reasons(prepared_by_asset)

        overrides: dict[str, Mapping[str, Any]] = {}
        rows_changed = False
        blocks_changed = False
        for pair in eligible_pairs:
            out_id = str(pair["out_id"])
            in_id = str(pair["in_id"])
            if out_id not in rows_by_id or in_id not in rows_by_id:
                continue
            pair_key = str(pair.get("pair_id") or f"{out_id}->{in_id}")
            if out_id in blocked_row_ids or in_id in blocked_row_ids:
                continue
            carried_basis = basis_by_id.get(out_id)
            if carried_basis is None or carried_basis <= 0:
                reason_code = (
                    quarantine_reasons.get(out_id)
                    or quarantine_reasons.get(in_id)
                    or "source_basis_unavailable"
                )
                pair_blocked_ids = _generic_bitcoin_rail_blocked_row_ids(pair)
                if not pair_blocked_ids <= blocked_row_ids:
                    blocks_changed = True
                    blocked_row_ids.update(pair_blocked_ids)
                blocked_pair_reasons[pair_key] = reason_code
                continue
            for row_id in (out_id, in_id):
                row = rows_by_id[row_id]
                override = _with_carried_fiat_value(row, carried_basis)
                if dict(row) != override:
                    rows_changed = True
                overrides[row_id] = override
        if overrides and rows_changed:
            current_rows = [overrides.get(str(row["id"]), row) for row in current_rows]
        if not rows_changed and not blocks_changed:
            break

    final_rows_by_id = {str(row["id"]): row for row in current_rows}
    quarantines: list[dict[str, Any]] = []
    for pair in eligible_pairs:
        out_id = str(pair["out_id"])
        in_id = str(pair["in_id"])
        pair_key = str(pair.get("pair_id") or f"{out_id}->{in_id}")
        reason_code = blocked_pair_reasons.get(pair_key)
        if reason_code is None:
            continue
        quarantines.extend(
            _generic_bitcoin_rail_pair_quarantines(
                profile,
                pair,
                final_rows_by_id,
                reason_code,
            )
        )
    return _GenericRailCarryResult(current_rows, blocked_row_ids, quarantines)


class GenericRP2TaxEngine:
    """Current generic RP2-backed implementation behind the engine seam."""

    def __init__(self, profile: Mapping[str, Any]):
        self.profile = profile

    def _build_finalized_ledger_state(
        self,
        inputs: TaxEngineLedgerInputs,
        configuration: Any | None,
    ) -> TaxEngineLedgerResult:
        """Run RP2 exclusively over finalized custody tax events and moves."""

        projection = inputs.finalized_tax_projection
        # ``FinalizedTaxProjection.__post_init__`` validates each row marker;
        # retain this explicit boundary check so a duck-typed raw-row bundle
        # cannot accidentally regain the production path.
        from ..custody_tax_projection import FinalizedTaxProjection

        if not isinstance(projection, FinalizedTaxProjection):
            raise TypeError(
                "GenericRP2TaxEngine requires a FinalizedTaxProjection; raw rows are not a tax-engine input"
            )
        rows_for_engine = list(projection.rows)
        quarantines = list(projection.quarantines)
        if not rows_for_engine:
            return TaxEngineLedgerResult(
                entries=[],
                quarantines=quarantines,
                intra_audit=[],
                cross_asset_pairs=list(projection.cross_asset_pairs),
                direct_swap_payouts=_direct_payout_compat_records(
                    inputs.direct_payout_records,
                    projection.rows,
                ),
                tax_summary=[],
                account_holdings={},
                wallet_holdings={},
            )

        assert configuration is not None
        pairs_by_asset: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
        for pair in projection.intra_pairs:
            out_row = pair["out"]
            in_row = pair["in"]
            if str(_row_get(out_row, "asset")) != str(_row_get(in_row, "asset")):
                raise AssertionError("finalized same-asset move pair crossed assets")
            pairs_by_asset[str(_row_get(out_row, "asset"))].append(pair)

        # The existing generic BTC/LBTC carrying-value helper remains RP2 lot
        # math, but it now sees only finalized projection rows/pairs.  It cannot
        # discover, suppress, or restore raw evidence.
        generic_rail = _apply_generic_bitcoin_rail_carry_values(
            self.profile,
            rows_for_engine,
            projection.cross_asset_pairs,
            pairs_by_asset,
            configuration,
            inputs.wallet_refs_by_id,
            excluded_row_ids=set(),
            loan_leg_by_transaction_id={},
        )
        rows_for_engine = generic_rail.rows
        quarantines.extend(generic_rail.quarantines)

        rows_by_asset: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
        for row in rows_for_engine:
            rows_by_asset[str(_row_get(row, "asset"))].append(row)
        prepared_by_asset = _prepare_assets(
            self.profile,
            rows_by_asset,
            inputs.wallet_refs_by_id,
            pairs_by_asset,
            configuration,
            excluded_row_ids=generic_rail.blocked_row_ids,
            loan_leg_by_transaction_id={},
        )
        swap_link_by_row_id, quarantined_row_ids, swap_quarantines = (
            _select_at_cross_asset_swap_links(
                self.profile,
                projection.cross_asset_pairs,
                rows_for_engine,
                prepared_by_asset,
            )
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
                excluded_row_ids=(
                    set(generic_rail.blocked_row_ids) | quarantined_row_ids
                ),
                loan_leg_by_transaction_id={},
            )
        _validate_prepared_rp2_inputs(
            configuration,
            [
                prepared.input_data
                for _, prepared in prepared_by_asset
                if prepared.input_data is not None
            ],
        )
        asset_states = _rp2_asset_states_from_prepared(
            prepared_by_asset,
            self.profile,
            configuration,
            requires_multi_asset_compute=bool(swap_link_by_row_id),
        )
        wallet_refs_by_label = {
            str(item["label"]): item
            for item in inputs.wallet_refs_by_id.values()
            if item.get("label") not in (None, "")
        }
        entries: list[dict[str, Any]] = []
        intra_audit: list[dict[str, Any]] = []
        tax_summary: list[dict[str, Any]] = []
        account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
        for normalized_inputs, prepared in prepared_by_asset:
            result = self._process_asset(
                prepared,
                normalized_inputs,
                wallet_refs_by_label,
                configuration,
                asset_states[prepared.asset],
            )
            quarantines.extend(result.quarantines)
            intra_audit.extend(result.intra_audit)
            tax_summary.extend(result.tax_summary)
            entries.extend(result.entries)
            for key, value in result.account_holdings.items():
                account_holdings[key]["quantity"] += value["quantity"]
                account_holdings[key]["cost_basis"] += value["cost_basis"]
            for key, value in result.wallet_holdings.items():
                wallet_holdings[key]["quantity"] += value["quantity"]
                wallet_holdings[key]["cost_basis"] += value["cost_basis"]
        return TaxEngineLedgerResult(
            entries=entries,
            quarantines=quarantines,
            # This is rendered output from the finalized MOVE projection.  It
            # is no longer an interpreter input for custody quantity.
            intra_audit=intra_audit,
            cross_asset_pairs=list(projection.cross_asset_pairs),
            direct_swap_payouts=_direct_payout_compat_records(
                inputs.direct_payout_records,
                projection.rows,
            ),
            tax_summary=tax_summary,
            account_holdings=dict(account_holdings),
            wallet_holdings=dict(wallet_holdings),
        )

    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        """Run RP2 exclusively over finalized custody tax events and moves."""

        from ..custody_tax_projection import FinalizedTaxProjection

        projection = inputs.finalized_tax_projection
        if not isinstance(projection, FinalizedTaxProjection):
            raise TypeError(
                "GenericRP2TaxEngine requires a FinalizedTaxProjection; raw rows are not a tax-engine input"
            )
        if not projection.rows:
            return self._build_finalized_ledger_state(inputs, None)
        wallet_labels = [
            str(item["label"])
            for item in inputs.wallet_refs_by_id.values()
            if item.get("label") not in (None, "")
        ]
        assets = {str(_row_get(row, "asset")) for row in projection.rows}
        with _rp2_configuration(self.profile, wallet_labels, assets) as configuration:
            return self._build_finalized_ledger_state(inputs, configuration)

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
