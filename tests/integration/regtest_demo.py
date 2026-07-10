from __future__ import annotations

import argparse
import base64
from bisect import bisect_right
import csv
import json
import os
import random
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any, Callable
from urllib import error, request

from kassiber.core import rates as core_rates
from kassiber.core import silent_payments as core_silent_payments
from kassiber.core.sync_backends import sanitize_wallet_segment
from kassiber.db import open_db
from kassiber.importers import GENERIC_LEDGER_COLUMNS
from kassiber.msat import btc_to_msat, dec


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SCENARIO = ROOT / "dev" / "regtest" / "scenarios" / "full_accounting.json"
SAT = Decimal("0.00000001")
TRANSACTION_LIST_LIMIT = "1000"
SECONDS_PER_DAY = 86_400


def _wallet_kind(wallet_spec: dict[str, Any]) -> str:
    return str(wallet_spec.get("kind") or "address").strip().lower()


def _wallet_chain(wallet_spec: dict[str, Any]) -> str:
    return str(wallet_spec.get("chain") or "bitcoin").strip().lower()


def _is_core_wallet_spec(wallet_spec: dict[str, Any]) -> bool:
    return _wallet_kind(wallet_spec) == "address" and _wallet_chain(wallet_spec) == "bitcoin"


def _is_liquid_live_wallet_spec(wallet_spec: dict[str, Any]) -> bool:
    return _wallet_kind(wallet_spec) == "descriptor" and _wallet_chain(wallet_spec) == "liquid"


def _is_silent_payment_wallet_spec(wallet_spec: dict[str, Any]) -> bool:
    return _wallet_kind(wallet_spec) == core_silent_payments.WALLET_KIND


def _is_core_wallet(wallet: "DemoWallet") -> bool:
    return wallet.chain == "bitcoin" and bool(wallet.address and wallet.core_wallet)


def _is_silent_payment_wallet(wallet: "DemoWallet") -> bool:
    return wallet.kind == core_silent_payments.WALLET_KIND


def _is_liquid_live_wallet(wallet: "DemoWallet") -> bool:
    return wallet.chain == "liquid" and wallet.kind == "descriptor" and bool(wallet.core_wallet)


@dataclass
class DemoWallet:
    key: str
    label: str
    account: str
    kind: str = "address"
    chain: str = "bitcoin"
    network: str = "regtest"
    core_wallet: str = ""
    address: str = ""
    addresses: list[str] = field(default_factory=list)
    source_file: str = ""
    source_format: str = ""
    sp_descriptor: str = ""
    sp_scan_start_height: int = 0
    sp_scan_file: str = ""
    descriptor_file: str = ""
    change_descriptor_file: str = ""
    kassiber_id: str | None = None
    watchonly_wallet: str | None = None
    receive_cursor: int = 0
    change_cursor: int = 1

    def receive_address(self) -> str:
        """Rotate deterministically through the watched addresses like a real
        wallet handing out a fresh receive address per payment request."""
        if not self.addresses:
            return self.address
        chosen = self.addresses[self.receive_cursor % len(self.addresses)]
        self.receive_cursor += 1
        return chosen

    def change_address(self) -> str:
        if not self.addresses:
            return self.address
        chosen = self.addresses[self.change_cursor % len(self.addresses)]
        self.change_cursor += 1
        return chosen


@dataclass
class DemoTruth:
    scenario_id: str
    transaction_rows: list[dict[str, Any]] = field(default_factory=list)
    transfer_pairs: list[dict[str, Any]] = field(default_factory=list)
    skipped_txids: list[dict[str, str]] = field(default_factory=list)
    core_utxos: dict[str, dict[str, Any]] = field(default_factory=dict)

    def record_transaction(
        self,
        op_id: str,
        txid: str,
        wallet: DemoWallet,
        direction: str,
        *,
        asset: str | None = None,
        confirmed: bool = True,
        confirmation_expected: bool = True,
        source: str = "scenario",
    ) -> None:
        if direction not in {"inbound", "outbound"}:
            raise RuntimeError(f"Unsupported expected transaction direction: {direction}")
        self.transaction_rows.append(
            {
                "op_id": op_id,
                "external_id": str(txid).lower(),
                "wallet_key": wallet.key,
                "wallet_label": wallet.label,
                "wallet_id": wallet.kassiber_id,
                "direction": direction,
                "asset": (asset or ("LBTC" if wallet.chain == "liquid" else "BTC")).upper(),
                "confirmed": bool(confirmed),
                "confirmation_expected": bool(confirmation_expected),
                "source": source,
            }
        )

    def record_skipped_txid(self, op_id: str, txid: str, reason: str) -> None:
        self.skipped_txids.append(
            {"op_id": op_id, "external_id": str(txid).lower(), "reason": reason}
        )

    def record_transfer_pair(
        self,
        pair_id: str,
        out_txid: str,
        in_txid: str,
        *,
        kind: str,
        policy: str,
        note: str = "",
    ) -> None:
        self.transfer_pairs.append(
            {
                "id": pair_id,
                "out_external_id": str(out_txid).lower(),
                "in_external_id": str(in_txid).lower(),
                "kind": kind,
                "policy": policy,
                "note": note,
            }
        )

    def to_dict(self) -> dict[str, Any]:
        transaction_rows = sorted(
            self.transaction_rows,
            key=lambda row: (
                row["external_id"],
                row["wallet_key"],
                row["asset"],
                row["direction"],
                row["op_id"],
            ),
        )
        transfer_pairs = sorted(
            self.transfer_pairs,
            key=lambda row: (
                row["out_external_id"],
                row["in_external_id"],
                row["kind"],
                row["policy"],
            ),
        )
        by_asset = Counter(row["asset"] for row in transaction_rows)
        by_wallet = Counter(row["wallet_label"] for row in transaction_rows)
        by_direction = Counter(row["direction"] for row in transaction_rows)
        return {
            "schema_version": 1,
            "scenario": self.scenario_id,
            "transactions": {
                "count": len(transaction_rows),
                "confirmed": sum(1 for row in transaction_rows if row["confirmed"]),
                "unconfirmed": sum(1 for row in transaction_rows if not row["confirmed"]),
                "by_asset": dict(sorted(by_asset.items())),
                "by_wallet": dict(sorted(by_wallet.items())),
                "by_direction": dict(sorted(by_direction.items())),
                "rows": transaction_rows,
            },
            "transfer_pairs": {
                "count": len(transfer_pairs),
                "rows": transfer_pairs,
            },
            "skipped_txids": sorted(
                self.skipped_txids,
                key=lambda row: (row["external_id"], row["op_id"], row["reason"]),
            ),
            "core_utxos": dict(sorted(self.core_utxos.items())),
        }


def load_scenario(path: Path = DEFAULT_SCENARIO) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        scenario = json.load(handle)
    validate_scenario(scenario)
    return scenario


def validate_scenario(scenario: dict[str, Any]) -> None:
    required = {
        "schema_version",
        "id",
        "base_time",
        "latest_time",
        "workspace",
        "profile",
        "wallets",
        "operations",
        "expected",
    }
    missing = sorted(required.difference(scenario))
    if missing:
        raise ValueError(f"Scenario manifest is missing: {', '.join(missing)}")
    if scenario["schema_version"] != 1:
        raise ValueError(f"Unsupported scenario schema_version: {scenario['schema_version']}")
    wallet_keys = [wallet["key"] for wallet in scenario["wallets"]]
    if len(wallet_keys) != len(set(wallet_keys)):
        raise ValueError("Scenario wallet keys must be unique")
    wallet_key_set = set(wallet_keys)
    wallet_specs_by_key = {wallet["key"]: wallet for wallet in scenario["wallets"]}
    core_wallet_keys = {wallet["key"] for wallet in scenario["wallets"] if _is_core_wallet_spec(wallet)}
    for wallet in scenario["wallets"]:
        for field in ("key", "label", "account", "initial_btc"):
            if not wallet.get(field):
                raise ValueError(f"Scenario wallet {wallet.get('key')!r} is missing {field}")
        _btc_or_zero(wallet["initial_btc"])
        kind = _wallet_kind(wallet)
        chain = _wallet_chain(wallet)
        if chain not in {"bitcoin", "liquid"}:
            raise ValueError(f"Scenario wallet {wallet['key']!r} has unsupported chain: {chain}")
        if kind not in {"address", "custom", "descriptor", core_silent_payments.WALLET_KIND}:
            raise ValueError(f"Scenario wallet {wallet['key']!r} has unsupported kind: {kind}")
        if kind == core_silent_payments.WALLET_KIND:
            if chain != "bitcoin":
                raise ValueError(f"Scenario Silent Payments wallet {wallet['key']!r} must be Bitcoin")
            descriptor = str(wallet.get("sp_descriptor") or "")
            try:
                core_silent_payments.validate_watch_only_descriptor(
                    descriptor,
                    chain=chain,
                    network=wallet.get("network") or "regtest",
                )
            except Exception as exc:
                raise ValueError(f"Scenario Silent Payments wallet {wallet['key']!r} has invalid material") from exc
            if int(wallet.get("sp_scan_start_height") or 0) < 0:
                raise ValueError(
                    f"Scenario Silent Payments wallet {wallet['key']!r} has invalid sp_scan_start_height"
                )
        if chain == "liquid" and not _is_liquid_live_wallet_spec(wallet):
            raise ValueError(
                f"Scenario Liquid wallet {wallet['key']!r} must be a descriptor wallet backed by elementsregtest"
            )
        address_count = int(wallet.get("addresses") or 1)
        if address_count < 1 or address_count > 256:
            raise ValueError(f"Scenario wallet {wallet['key']!r} addresses must be between 1 and 256")
        if address_count > 1 and not (_is_core_wallet_spec(wallet) or _is_liquid_live_wallet_spec(wallet)):
            raise ValueError(f"Scenario wallet {wallet['key']!r} only live wallets rotate addresses")
        address_type = str(wallet.get("address_type") or "bech32")
        if _is_core_wallet_spec(wallet) and address_type not in {"legacy", "p2sh-segwit", "bech32", "bech32m"}:
            raise ValueError(
                f"Scenario wallet {wallet['key']!r} has unsupported Bitcoin address_type: {address_type}"
            )
    def _validate_operation(operation: dict[str, Any], *, pending: bool = False) -> None:
        op_id = operation.get("id") or "<unnamed>"
        kind = operation.get("kind")
        if not kind:
            raise ValueError(f"Scenario operation {op_id} is missing kind")
        if pending and kind != "external_receipt":
            raise ValueError(f"Scenario pending operation {op_id} must be an external_receipt")
        for amount_field in ("amount_btc", "fee_btc", "payment_btc", "equal_output_btc", "replacement_fee_btc"):
            if amount_field in operation:
                _btc(operation[amount_field])
        if kind == "self_transfer_fanout":
            outputs = operation.get("outputs")
            if not isinstance(outputs, list) or len(outputs) < 2:
                raise ValueError(f"Scenario operation {op_id} must have at least two fan-out outputs")
            destinations = []
            for output_index, output in enumerate(outputs, start=1):
                if not isinstance(output, dict):
                    raise ValueError(f"Scenario operation {op_id} output {output_index} must be an object")
                to_key = output.get("to")
                if to_key not in wallet_key_set:
                    raise ValueError(
                        f"Scenario operation {op_id} output {output_index} references unknown wallet: {to_key}"
                    )
                if to_key not in core_wallet_keys:
                    raise ValueError(
                        f"Scenario operation {op_id} output {output_index} references non-Core wallet: {to_key}"
                    )
                if to_key == operation.get("from"):
                    raise ValueError(
                        f"Scenario operation {op_id} output {output_index} cannot pay the source wallet"
                    )
                _btc(output.get("amount_btc"))
                destinations.append(to_key)
            if len(set(destinations)) != len(destinations):
                raise ValueError(f"Scenario operation {op_id} fan-out destinations must be unique")
        if kind == "rbf_replaced_payment":
            if "replacement_fee_btc" not in operation:
                raise ValueError(f"Scenario operation {op_id} is missing replacement_fee_btc")
            if _btc(operation["replacement_fee_btc"]) <= _btc(operation["fee_btc"]):
                raise ValueError(f"Scenario operation {op_id} replacement fee must exceed the original fee")
        for value in operation.get("outputs_btc", []):
            _btc(value)
        if "count" in operation and int(operation["count"]) <= 0:
            raise ValueError(f"Scenario operation {op_id} count must be positive")
        refs = []
        for ref_field in ("from", "to", "payer", "merchant", "tracked_output_wallet", "wallet"):
            value = operation.get(ref_field)
            if value and value != "external":
                refs.append((ref_field, value))
        refs.extend(("signers", signer) for signer in operation.get("signers", []))
        for ref_field, value in refs:
            if value not in wallet_key_set:
                raise ValueError(f"Scenario operation {op_id} references unknown {ref_field}: {value}")
            if value not in core_wallet_keys:
                raise ValueError(f"Scenario operation {op_id} references non-Core wallet {ref_field}: {value}")

    for operation in scenario["operations"]:
        _validate_operation(operation)
    for operation in scenario.get("pending_operations") or []:
        _validate_operation(operation, pending=True)
    stress = scenario.get("stress") or {}
    if stress.get("enabled"):
        cycles = int(stress.get("cycles") or 0)
        days_between_cycles = int(stress.get("days_between_cycles") or 0)
        if cycles <= 0:
            raise ValueError("Scenario stress.cycles must be positive")
        if days_between_cycles <= 0:
            raise ValueError("Scenario stress.days_between_cycles must be positive")
        for index, operation in enumerate(scenario["operations"], start=1):
            if operation.get("cycle") is None:
                continue
            cycle = int(operation["cycle"])
            if cycle < 1 or cycle > cycles:
                raise ValueError(f"Scenario operations[{index}] cycle is outside the stress range")
        for field in ("receipt_btc", "payment_btc"):
            entries = stress.get(field)
            if not isinstance(entries, dict) or not entries:
                raise ValueError(f"Scenario stress.{field} must be a non-empty object")
            for key, value in entries.items():
                if key not in wallet_key_set:
                    raise ValueError(f"Scenario stress.{field} references unknown wallet: {key}")
                if key not in core_wallet_keys:
                    raise ValueError(f"Scenario stress.{field} references non-Core wallet: {key}")
                _recurring_amount_spec(value, label=f"stress.{field}.{key}")
        if not stress.get("fee_btc"):
            raise ValueError("Scenario stress.fee_btc must be set")
        _btc(stress["fee_btc"])
        base_ts = _parse_iso_to_ts(str(scenario["base_time"]))
        latest_ts = _parse_iso_to_ts(str(scenario["latest_time"]))
        estimated_end_ts = estimate_scenario_end_ts(scenario, start_ts=base_ts)
        if latest_ts <= base_ts:
            raise ValueError("Scenario latest_time must be after base_time")
        if estimated_end_ts > latest_ts:
            estimated = _iso_from_ts(estimated_end_ts)
            latest = _iso_from_ts(latest_ts)
            raise ValueError(
                f"Scenario timeline ends at {estimated}, after latest_time {latest}"
            )
        expenses = stress.get("business_expenses") or {}
        if expenses.get("enabled"):
            schedule = expenses.get("schedule")
            if not isinstance(schedule, list) or not schedule:
                raise ValueError("Scenario stress.business_expenses.schedule must be a non-empty list")
            if int(expenses.get("every_cycles") or 1) <= 0:
                raise ValueError("Scenario stress.business_expenses.every_cycles must be positive")
            _btc(expenses.get("fee_btc") or stress["fee_btc"])
            for index, expense in enumerate(schedule, start=1):
                role = expense.get("role")
                if role not in wallet_key_set:
                    raise ValueError(
                        f"Scenario stress.business_expenses.schedule[{index}] references unknown role: {role}"
                    )
                if role not in core_wallet_keys:
                    raise ValueError(
                        f"Scenario stress.business_expenses.schedule[{index}] references non-Core role: {role}"
                    )
                _recurring_amount_spec(expense, label=f"stress.business_expenses.schedule[{index}]")
                if int(expense.get("every_cycles") or 1) <= 0:
                    raise ValueError(
                        f"Scenario stress.business_expenses.schedule[{index}].every_cycles must be positive"
                    )
                start_cycle = int(expense.get("start_cycle") or 1)
                if start_cycle < 1 or start_cycle > cycles:
                    raise ValueError(
                        f"Scenario stress.business_expenses.schedule[{index}].start_cycle is outside the stress range"
                    )
        for index, era in enumerate(stress.get("fee_curve") or [], start=1):
            _parse_iso_to_ts(str(era.get("start") or ""))
            multiplier = Decimal(str(era.get("multiplier") or "0"))
            if multiplier <= 0:
                raise ValueError(f"Scenario stress.fee_curve[{index}].multiplier must be positive")
        pool = stress.get("pool_payouts") or {}
        if pool.get("enabled"):
            start_cycle = int(pool.get("start_cycle") or 0)
            end_cycle = int(pool.get("end_cycle") or 0)
            every_cycles = int(pool.get("every_cycles") or 0)
            if start_cycle < 1 or end_cycle < start_cycle or end_cycle > cycles:
                raise ValueError("Scenario stress.pool_payouts cycle range is invalid")
            if every_cycles <= 0:
                raise ValueError("Scenario stress.pool_payouts.every_cycles must be positive")
            role = pool.get("role")
            if role not in core_wallet_keys:
                raise ValueError(f"Scenario stress.pool_payouts references unknown role: {role}")
            _recurring_amount_spec(pool, label="stress.pool_payouts")
        for index, rotation in enumerate(stress.get("wallet_rotations") or [], start=1):
            cycle = int(rotation.get("cycle") or 0)
            if cycle < 1 or cycle > cycles:
                raise ValueError(f"Scenario stress.wallet_rotations[{index}] cycle is outside the stress range")
            for field in ("role", "from", "to"):
                if rotation.get(field) not in wallet_key_set:
                    raise ValueError(
                        f"Scenario stress.wallet_rotations[{index}] references unknown {field}: {rotation.get(field)}"
                    )
                if rotation.get(field) not in core_wallet_keys:
                    raise ValueError(
                        f"Scenario stress.wallet_rotations[{index}] references non-Core {field}: {rotation.get(field)}"
                    )
            _btc(rotation.get("amount_btc"))
            _btc(rotation.get("fee_btc") or stress["fee_btc"])
        for index, bridge in enumerate(stress.get("swap_bridges") or [], start=1):
            cycle = int(bridge.get("cycle") or 0)
            if cycle < 1 or cycle > cycles:
                raise ValueError(f"Scenario stress.swap_bridges[{index}] cycle is outside the stress range")
            for field in ("from_role", "to_role"):
                if bridge.get(field) not in wallet_key_set:
                    raise ValueError(
                        f"Scenario stress.swap_bridges[{index}] references unknown {field}: {bridge.get(field)}"
                    )
            source = wallet_specs_by_key[bridge["from_role"]]
            target = wallet_specs_by_key[bridge["to_role"]]
            if not (_is_core_wallet_spec(source) or _is_core_wallet_spec(target)):
                if _wallet_chain(source) != "liquid" or _wallet_chain(target) != "liquid":
                    raise ValueError(
                        f"Scenario stress.swap_bridges[{index}] needs a Core or descriptor-backed Liquid endpoint"
                    )
            _btc(bridge.get("out_btc"))
            _btc(bridge.get("in_btc"))
            _btc(bridge.get("fee_btc") or stress["fee_btc"])
            pair_kind = bridge.get("pair_kind") or "submarine-swap"
            if pair_kind not in {"chain-swap", "peg-in", "peg-out", "reverse-submarine-swap", "submarine-swap", "swap-refund"}:
                raise ValueError(f"Scenario stress.swap_bridges[{index}] has unsupported pair_kind: {pair_kind}")
        variation_bp = int(stress.get("variation_bp") or 0)
        if variation_bp < 0 or variation_bp > 4000:
            raise ValueError("Scenario stress.variation_bp must be between 0 and 4000 basis points")
        for index, event in enumerate(stress.get("mining_events") or [], start=1):
            cycle = int(event.get("cycle") or 0)
            if cycle < 1 or cycle > cycles:
                raise ValueError(f"Scenario stress.mining_events[{index}] cycle is outside the stress range")
            # Coinbase rewards need 100 blocks to mature; every later cycle mines
            # at least three blocks, so leave a comfortable maturity margin.
            if (cycles - cycle) * 3 < 105:
                raise ValueError(
                    f"Scenario stress.mining_events[{index}] is too late to mature before sync"
                )
            role = event.get("role")
            if role not in wallet_key_set or role not in core_wallet_keys:
                raise ValueError(f"Scenario stress.mining_events[{index}] references unknown role: {role}")
            blocks = int(event.get("blocks") or 0)
            if blocks < 1 or blocks > 3:
                raise ValueError(f"Scenario stress.mining_events[{index}] blocks must be between 1 and 3")
        regimes = stress.get("economic_regimes") or []
        regime_cycle_total = 0
        has_downturn = False
        has_boom = False
        for index, phase in enumerate(regimes, start=1):
            span = int(phase.get("cycles") or 0)
            if span <= 0:
                raise ValueError(f"Scenario stress.economic_regimes[{index}] cycles must be positive")
            receipt_scale = Decimal(str(phase.get("receipt_scale", "1")))
            spend_scale = Decimal(str(phase.get("spend_scale", "1")))
            if receipt_scale <= 0 or spend_scale <= 0:
                raise ValueError(f"Scenario stress.economic_regimes[{index}] scales must be positive")
            regime_cycle_total += span
            if spend_scale > receipt_scale:
                has_downturn = True
            if receipt_scale > spend_scale:
                has_boom = True
        if regimes:
            if regime_cycle_total > cycles:
                raise ValueError("Scenario stress.economic_regimes span more cycles than stress.cycles")
            if not (has_downturn and has_boom):
                raise ValueError(
                    "Scenario stress.economic_regimes must include both a downturn "
                    "(spend_scale > receipt_scale) and a boom phase so balances rise and fall"
                )
    for index, deprecated_key in enumerate(scenario.get("deprecated_wallets") or [], start=1):
        if deprecated_key not in wallet_key_set:
            raise ValueError(f"Scenario deprecated_wallets[{index}] references unknown wallet: {deprecated_key}")
    if scenario.get("liquid_ledger"):
        raise ValueError("Regtest scenarios must use real elementsregtest Liquid transactions, not liquid_ledger fixtures")
    pricing = scenario.get("pricing") or {}
    fallback = pricing.get("fallback") or pricing
    if fallback.get("rate_sequence"):
        rates = [Decimal(str(value)) for value in fallback["rate_sequence"]]
        if any(rate <= 0 for rate in rates):
            raise ValueError("Scenario pricing fallback rate_sequence values must be positive")
        if rates == sorted(rates) or rates == sorted(rates, reverse=True):
            raise ValueError("Scenario pricing fallback rate_sequence must be volatile, not monotonic")


def _stress_jitter_bp(cycle: int, salt: int, spread_bp: int) -> int:
    """Deterministic pseudo-variation in [-spread_bp, +spread_bp] basis points."""
    if spread_bp <= 0:
        return 0
    seed = (cycle * 2654435761 + salt * 40503 + 94261) % (2**32)
    return int(seed % (2 * spread_bp + 1)) - spread_bp


def _recurring_amount_spec(value: Any, *, label: str) -> tuple[str, Decimal]:
    """Return (denomination, amount) for a recurring manifest value.

    Scalar values and ``amount_btc`` retain the v1 BTC-denominated contract;
    ``amount_eur`` opts into date-sensitive fiat denomination.
    """
    if isinstance(value, dict):
        has_btc = value.get("amount_btc") not in (None, "")
        has_eur = value.get("amount_eur") not in (None, "")
        if has_btc == has_eur:
            raise ValueError(f"Scenario {label} must set exactly one of amount_btc or amount_eur")
        if has_eur:
            amount = Decimal(str(value["amount_eur"])).quantize(Decimal("0.01"))
            if amount <= 0:
                raise ValueError(f"Scenario {label}.amount_eur must be positive")
            return "fiat", amount
        return "btc", _btc(value["amount_btc"])
    return "btc", _btc(value)


def _load_bundled_daily_rates(pair: str) -> list[tuple[int, Decimal]]:
    normalized = str(pair).strip().upper().replace("-", "")
    kraken_pair = {"BTCEUR": "XBTEUR", "BTCUSD": "XBTUSD"}.get(normalized)
    if not kraken_pair:
        raise ValueError(f"No bundled Kraken daily history for recurring denomination pair {pair!r}")
    path = ROOT / "kassiber" / "data" / "rates" / "kraken" / "btc_daily" / f"{kraken_pair}_1440.csv"
    rows: list[tuple[int, Decimal]] = []
    with path.open("r", encoding="utf-8", newline="") as handle:
        for line_number, row in enumerate(csv.reader(handle), start=1):
            if len(row) != 7:
                raise ValueError(f"Bundled Kraken history {path.name}:{line_number} is malformed")
            # Kassiber stores daily candles at their close timestamp and prices
            # a transaction from the latest cached sample at or before it.
            close_ts = int(row[0]) + SECONDS_PER_DAY
            close_rate = Decimal(row[4])
            if close_rate <= 0:
                raise ValueError(f"Bundled Kraken history {path.name}:{line_number} has no close rate")
            rows.append((close_ts, close_rate))
    if not rows:
        raise ValueError(f"Bundled Kraken history {path.name} is empty")
    return rows


def _cached_rate_at_or_before(history: list[tuple[int, Decimal]], timestamp: int) -> Decimal:
    index = bisect_right([row[0] for row in history], timestamp) - 1
    if index < 0:
        raise ValueError(f"Bundled Kraken history does not cover {_iso_from_ts(timestamp)}")
    return history[index][1]


def _recurring_btc(value: Any, *, rate: Decimal, label: str) -> Decimal:
    denomination, amount = _recurring_amount_spec(value, label=label)
    if denomination == "btc":
        return amount.quantize(SAT)
    converted = (amount / rate).quantize(SAT)
    return max(converted, SAT)


def _cycle_activity_mode(cycle: int) -> str:
    """Deterministic ~5% activity skips and ~5% double-booked busy cycles."""
    if cycle == 1:
        return "normal"
    bucket = (_stress_jitter_bp(cycle, 71, 4999) + 4999) % 100
    if bucket < 5:
        return "skip"
    if bucket < 10:
        return "double"
    return "normal"


def _cycle_timestamp(first_target_ts: int, cycle: int, days_between_cycles: int) -> int:
    jitter_days = _stress_jitter_bp(cycle, 79, 5)
    hour = (_stress_jitter_bp(cycle, 83, 11) + 11) % 24
    return first_target_ts + ((cycle - 1) * days_between_cycles + jitter_days) * SECONDS_PER_DAY + hour * 3600


def _event_timestamp(cycle_ts: int, cycle: int, slot: int) -> int:
    minute_jitter = _stress_jitter_bp(cycle, 101 + slot, 45)
    return cycle_ts + slot * 3600 + minute_jitter * 60


def _rare_receipt_multiplier(cycle: int, wallet_index: int, wallet_count: int) -> Decimal:
    if cycle % 17 or wallet_index != (cycle // 17) % max(wallet_count, 1):
        return Decimal("1")
    return Decimal(5 + ((_stress_jitter_bp(cycle, 109, 4999) + 4999) % 16))


def _fee_curve_multiplier(stress: dict[str, Any], timestamp: int) -> Decimal:
    multiplier = Decimal("1")
    for era in sorted(stress.get("fee_curve") or [], key=lambda row: _parse_iso_to_ts(str(row["start"]))):
        if timestamp < _parse_iso_to_ts(str(era["start"])):
            break
        multiplier = Decimal(str(era["multiplier"]))
    return multiplier


def _varied_amount(
    amount: Decimal,
    cycle: int,
    *,
    salt: int,
    spread_bp: int,
    ragged_sats: int = 0,
    scale: Decimal = Decimal("1"),
) -> Decimal:
    """Scale a planned amount by a deterministic per-cycle factor.

    Receipts and payments share the same salt so a lean cycle shrinks both
    sides and wallet margins survive every cycle ordering. `scale` layers the
    economic regime (boom vs. downturn) on top, and `ragged_sats` roughens
    receipt amounts so the ledger does not look like a spreadsheet of round
    numbers. Always returns a positive sat amount so downstream _btc() holds.
    """
    delta_bp = _stress_jitter_bp(cycle, salt, spread_bp)
    varied = (amount * scale * (10_000 + delta_bp) / 10_000).quantize(SAT)
    if ragged_sats > 0:
        varied += ((cycle * 7919 + salt * 271) % ragged_sats) * SAT
    if varied <= 0:
        varied = SAT
    return varied


def _regime_scales(cycle: int, regimes: list[dict[str, Any]]) -> tuple[Decimal, Decimal, str]:
    """Deterministic (receipt_scale, spend_scale, label) for a cycle.

    Regime phases tile from cycle 1 in order; cycles past the last phase (or
    with no regimes configured) run at neutral 1.0/1.0. A downturn scales
    receipts down and spend up, so balances genuinely draw down — the book is
    not monotonically up-and-to-the-right.
    """
    start = 1
    for phase in regimes:
        span = int(phase.get("cycles") or 0)
        if span <= 0:
            continue
        if start <= cycle < start + span:
            return (
                Decimal(str(phase.get("receipt_scale", "1"))),
                Decimal(str(phase.get("spend_scale", "1"))),
                str(phase.get("label") or "regime"),
            )
        start += span
    return (Decimal("1"), Decimal("1"), "steady")


def _btc(value: Any) -> Decimal:
    amount = Decimal(str(value)).quantize(SAT)
    if amount <= 0:
        raise ValueError(f"BTC amount must be positive: {value}")
    return amount


def _btc_or_zero(value: Any) -> Decimal:
    amount = Decimal(str(value)).quantize(SAT)
    if amount < 0:
        raise ValueError(f"BTC amount must not be negative: {value}")
    return amount


def _rpc_amount(value: Decimal) -> float:
    return float(value.quantize(SAT))


def _json_ready(value: Any) -> Any:
    if isinstance(value, Decimal):
        return _rpc_amount(value)
    if isinstance(value, dict):
        return {key: _json_ready(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_json_ready(item) for item in value]
    return value


def _iso_from_ts(ts: int) -> str:
    return datetime.fromtimestamp(ts, tz=timezone.utc).isoformat().replace("+00:00", "Z")


def _decimal_text(value: Any) -> str:
    if value in (None, ""):
        return ""
    if isinstance(value, Decimal):
        return format(value, "f")
    return str(value)


def _btc_to_msat_int(value: Any) -> int:
    return int(btc_to_msat(Decimal(str(value)).quantize(SAT)))


def _truth_transaction_key(row: dict[str, Any]) -> tuple[str, str, str, str]:
    return (
        str(row.get("external_id") or "").lower(),
        str(row.get("wallet_label") or row.get("wallet") or ""),
        str(row.get("asset") or "").upper(),
        str(row.get("direction") or ""),
    )


def _truth_key_dict(key: tuple[str, str, str, str]) -> dict[str, str]:
    txid, wallet, asset, direction = key
    return {
        "external_id": txid,
        "wallet": wallet,
        "asset": asset,
        "direction": direction,
    }


def _pair_key(row: dict[str, Any]) -> tuple[str, str, str, str]:
    return (
        str(row.get("out_external_id") or row.get("out", {}).get("external_id") or "").lower(),
        str(row.get("in_external_id") or row.get("in", {}).get("external_id") or "").lower(),
        str(row.get("kind") or ""),
        str(row.get("policy") or ""),
    )


def _pair_key_dict(key: tuple[str, str, str, str]) -> dict[str, str]:
    out_txid, in_txid, kind, policy = key
    return {
        "out_external_id": out_txid,
        "in_external_id": in_txid,
        "kind": kind,
        "policy": policy,
    }


def _counter_diff(
    expected: Counter,
    actual: Counter,
    renderer,
    *,
    limit: int = 10,
) -> dict[str, list[dict[str, str]]]:
    missing = list((expected - actual).elements())[:limit]
    extra = list((actual - expected).elements())[:limit]
    return {
        "missing": [renderer(key) for key in missing],
        "extra": [renderer(key) for key in extra],
    }


def _refresh_truth_wallet_ids(truth: DemoTruth, wallets: dict[str, DemoWallet]) -> None:
    for row in truth.transaction_rows:
        wallet = wallets.get(str(row.get("wallet_key") or ""))
        if wallet is not None:
            row["wallet_id"] = wallet.kassiber_id

def _seed_live_liquid_wallets(
    url: str,
    username: str,
    password: str,
    scenario: dict[str, Any],
    wallets: dict[str, DemoWallet],
    *,
    faucet_wallet: str,
    mining_address: str,
    external_address: str,
    current_ts: int,
    txids: dict[str, str],
    truth: DemoTruth | None = None,
) -> int:
    for wallet_spec in scenario["wallets"]:
        if not _is_liquid_live_wallet_spec(wallet_spec):
            continue
        wallet = wallets[wallet_spec["key"]]
        receipt_amount = _btc_or_zero(wallet_spec.get("live_receipt_btc") or "0")
        spend_amount = _btc_or_zero(wallet_spec.get("live_spend_btc") or "0")
        if receipt_amount <= 0:
            continue
        current_ts = _advance_time(url, username, password, current_ts)
        receive_key = f"{wallet.key}_liquid_live_receive"
        txids[receive_key] = rpc(
            url,
            username,
            password,
            "sendtoaddress",
            [wallet.receive_address(), receipt_amount],
            wallet=faucet_wallet,
        )
        if truth is not None:
            truth.record_transaction(receive_key, txids[receive_key], wallet, "inbound", asset="LBTC")
        rpc(url, username, password, "generatetoaddress", [1, mining_address])
        if spend_amount > 0:
            current_ts = _advance_time(url, username, password, current_ts)
            spend_key = f"{wallet.key}_liquid_live_spend"
            txids[spend_key] = rpc(
                url,
                username,
                password,
                "sendtoaddress",
                [external_address, spend_amount],
                wallet=wallet.core_wallet,
            )
            if truth is not None:
                truth.record_transaction(spend_key, txids[spend_key], wallet, "outbound", asset="LBTC")
            rpc(url, username, password, "generatetoaddress", [1, mining_address])
    return current_ts


def _write_silent_payment_scan_files(
    base_dir: Path,
    scenario: dict[str, Any],
    wallets: dict[str, DemoWallet],
    *,
    url: str,
    username: str,
    password: str,
    faucet_wallet: str,
    mining_address: str,
    current_ts: int,
    txids: dict[str, str],
    truth: DemoTruth | None = None,
) -> int:
    import_dir = base_dir / "imports" / "silent-payments"
    for wallet_spec in scenario["wallets"]:
        if not _is_silent_payment_wallet_spec(wallet_spec):
            continue
        wallet = wallets[wallet_spec["key"]]
        import_dir.mkdir(parents=True, exist_ok=True)
        receive_address = rpc(
            url,
            username,
            password,
            "getnewaddress",
            [f"{wallet.label} detected receive", "bech32m"],
            wallet=faucet_wallet,
        )
        amount = _btc(wallet_spec.get("detected_btc") or "0.01234567")
        txid = rpc(
            url,
            username,
            password,
            "sendtoaddress",
            [receive_address, amount],
            wallet=faucet_wallet,
        )
        txids[f"{wallet.key}_silent_payment_receive"] = txid
        if truth is not None:
            truth.record_transaction(
                f"{wallet.key}_silent_payment_receive",
                txid,
                wallet,
                "inbound",
                source="silent_payment_scan",
            )
        current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts)
        tx = rpc(url, username, password, "getrawtransaction", [txid, True])
        block = rpc(url, username, password, "getblock", [tx["blockhash"]])
        block_height = int(block["height"])
        block_time = _iso_from_ts(int(block.get("time") or current_ts))
        matched_output = None
        for output in tx.get("vout") or []:
            script = output.get("scriptPubKey") if isinstance(output, dict) else {}
            if not isinstance(script, dict):
                continue
            if script.get("address") == receive_address:
                matched_output = output
                break
        if matched_output is None:
            raise RuntimeError(f"Unable to locate Silent Payments regtest output for {wallet.key}")
        rpc(
            url,
            username,
            password,
            "lockunspent",
            [False, [{"txid": txid, "vout": int(matched_output["n"])}]],
            wallet=faucet_wallet,
        )
        script = matched_output.get("scriptPubKey") or {}
        amount_sats = int((Decimal(str(matched_output["value"])) * Decimal("100000000")).to_integral_value())
        output_row = {
            "txid": txid,
            "vout": int(matched_output["n"]),
            "amount_sats": amount_sats,
            "script_pubkey": script["hex"],
            "silent_payment": True,
            "block_height": block_height,
            "block_time": block_time,
            "confirmations": int(tx.get("confirmations") or 1),
            "raw": {"source": "regtest-frigate-silent-payments"},
        }
        payload = {
            "schema_version": 1,
            "complete": True,
            "descriptor_fingerprint": core_silent_payments.descriptor_fingerprint(wallet.sp_descriptor),
            "range": {
                "from_height": wallet.sp_scan_start_height,
                "to_height": block_height,
            },
            "transactions": [
                {
                    "txid": txid,
                    "block_height": block_height,
                    "block_time": block_time,
                    "confirmations": int(tx.get("confirmations") or 1),
                    "outputs": [output_row],
                }
            ],
            "utxos": [output_row],
        }
        scan_path = import_dir / f"{sanitize_wallet_segment(wallet.key)}-scan.json"
        scan_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        os.chmod(scan_path, 0o600)
        wallet.sp_scan_file = str(scan_path)
    return current_ts


def rpc(url: str, username: str, password: str, method: str, params=None, wallet: str | None = None):
    endpoint = url.rstrip("/")
    if wallet:
        endpoint = f"{endpoint}/wallet/{wallet}"
    payload = json.dumps(
        {
            "jsonrpc": "1.0",
            "id": f"kassiber-regtest-demo-{method}",
            "method": method,
            "params": [] if params is None else _json_ready(params),
        }
    ).encode("utf-8")
    req = request.Request(endpoint, data=payload, headers={"Content-Type": "application/json"})
    token = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    req.add_header("Authorization", f"Basic {token}")
    decoded: dict[str, Any] | None = None
    for attempt in range(1, 4):
        try:
            with request.urlopen(req, timeout=180) as response:
                decoded = json.loads(response.read().decode("utf-8"))
            break
        except error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            try:
                decoded = json.loads(body)
            except json.JSONDecodeError as decode_error:
                raise RuntimeError(f"RPC {method} failed over HTTP {exc.code}: {body}") from decode_error
            break
        except (TimeoutError, socket.timeout) as exc:
            if attempt >= 3:
                raise RuntimeError(f"RPC {method} timed out after {attempt} attempts") from exc
            time.sleep(attempt)
    if decoded is None:
        raise RuntimeError(f"RPC {method} returned no response")
    if decoded.get("error"):
        raise RuntimeError(f"RPC {method} failed: {decoded['error']}")
    return decoded.get("result")


def _elements_url() -> str:
    return os.environ.get("KASSIBER_REGTEST_ELEMENTS_URL") or (
        f"http://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_ELEMENTS_RPC_PORT', '18547')}"
    )


def run_cli(data_root: Path, *args: str, pass_fds: tuple[int, ...] = ()) -> dict[str, Any]:
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
        pass_fds=pass_fds,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"kassiber {' '.join(args)} failed\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return json.loads(result.stdout)


def _local_backend_specs(*, silent_payment_scan_file: str | None = None) -> list[dict[str, Any]]:
    return [
        {
            "name": "bitcoin-electrum-regtest",
            "kind": "electrum",
            "url": f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT', '18543')}",
            "chain": "bitcoin",
            "network": "regtest",
            "display_name": "Bitcoin Electrum Regtest",
        },
        {
            "name": "bitcoin-frigate-regtest",
            "kind": "electrum",
            "url": f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_FRIGATE_PORT', '18548')}",
            "chain": "bitcoin",
            "network": "regtest",
            "display_name": "Bitcoin Frigate Silent Payments Regtest",
            "silent_payments": True,
            **({"silent_payment_scan_file": silent_payment_scan_file} if silent_payment_scan_file else {}),
        },
        {
            "name": "bitcoin-mempool-regtest",
            "kind": "mempool",
            "url": f"http://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT', '18544')}/api",
            "chain": "bitcoin",
            "network": "regtest",
            "display_name": "Bitcoin Mempool Regtest",
        },
        {
            "name": "liquid-electrum-regtest",
            "kind": "electrum",
            "url": f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT', '18545')}",
            "chain": "liquid",
            "network": "elementsregtest",
            "display_name": "Liquid Electrum Regtest",
        },
        {
            "name": "liquid-mempool-regtest",
            "kind": "liquid-esplora",
            "url": f"http://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT', '18546')}/api",
            "chain": "liquid",
            "network": "elementsregtest",
            "display_name": "Liquid Mempool Regtest",
        },
    ]


def _parse_iso_to_ts(value: str) -> int:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    return int(parsed.astimezone(timezone.utc).timestamp())


def estimate_scenario_end_ts(scenario: dict[str, Any], *, start_ts: int | None = None) -> int:
    current_ts = start_ts if start_ts is not None else _parse_iso_to_ts(str(scenario["base_time"]))

    # run_demo performs one maturity mine, one counterparty-funding mine, one
    # initial-funding mine, then one confirmation mine per unscheduled
    # hand-authored operation before the long stress run.
    current_ts += 600
    current_ts += 600
    current_ts += 600
    current_ts += sum(1 for operation in scenario.get("operations") or [] if operation.get("cycle") is None) * 600

    stress = scenario.get("stress") or {}
    if stress.get("enabled"):
        cycles = int(stress.get("cycles") or 0)
        days_between_cycles = int(stress.get("days_between_cycles") or 0)
        if cycles > 0 and days_between_cycles > 0:
            first_target_ts = current_ts + (2 * SECONDS_PER_DAY)
            last_cycle_ts = first_target_ts + ((cycles - 1) * days_between_cycles * SECONDS_PER_DAY)
            current_ts = max(current_ts, last_cycle_ts + (7 * SECONDS_PER_DAY))

    silent_payment_wallets = [
        wallet for wallet in scenario.get("wallets") or [] if _is_silent_payment_wallet_spec(wallet)
    ]
    current_ts += len(silent_payment_wallets) * 600
    current_ts += len(scenario.get("pending_operations") or []) * 600
    return current_ts


def _ensure_wallet(url: str, username: str, password: str, wallet_name: str) -> None:
    loaded = set(rpc(url, username, password, "listwallets") or [])
    if wallet_name in loaded:
        return
    try:
        rpc(url, username, password, "loadwallet", [wallet_name, True])
        return
    except RuntimeError:
        pass
    try:
        rpc(url, username, password, "createwallet", [wallet_name, False, False, "", False, True, True])
    except RuntimeError:
        rpc(url, username, password, "createwallet", [wallet_name])


def _unload_wallet(url: str, username: str, password: str, wallet_name: str) -> None:
    try:
        rpc(url, username, password, "unloadwallet", [wallet_name])
    except RuntimeError:
        # Best-effort teardown: the wallet may already be unloaded or never
        # created (setup aborted early); an unload failure must not mask the
        # real error being unwound.
        pass


def _unconfidential_address(
    url: str,
    username: str,
    password: str,
    wallet_name: str,
    address: str,
) -> str:
    try:
        info = rpc(url, username, password, "getaddressinfo", [address], wallet=wallet_name)
    except RuntimeError:
        return address
    return str(info.get("unconfidential") or address)


def _descriptor_without_checksum(value: str) -> str:
    return str(value or "").split("#", 1)[0]


def _active_descriptor(descriptors: list[dict[str, Any]], *, internal: bool) -> str:
    candidates = [
        row
        for row in descriptors
        if bool(row.get("active")) and bool(row.get("internal")) is internal and row.get("desc")
    ]
    if not candidates:
        candidates = [
            row
            for row in descriptors
            if bool(row.get("internal")) is internal and row.get("desc")
        ]
    if not candidates:
        label = "change" if internal else "receive"
        raise RuntimeError(f"Elements wallet did not expose an active {label} descriptor")

    def preference(row: dict[str, Any]) -> tuple[int, int, str]:
        desc = _descriptor_without_checksum(str(row.get("desc") or ""))
        if desc.startswith("wpkh("):
            type_rank = 0
        elif desc.startswith("sh(wpkh("):
            type_rank = 1
        elif desc.startswith("tr("):
            type_rank = 2
        else:
            type_rank = 3
        used_rank = 0 if int(row.get("next") or 0) > 0 else 1
        return used_rank, type_rank, desc

    return _descriptor_without_checksum(str(sorted(candidates, key=preference)[0]["desc"]))


def _blinded_liquid_descriptor(master_blinding_key: str, descriptor: str) -> str:
    descriptor = _descriptor_without_checksum(descriptor)
    if descriptor.startswith(("ct(", "blinded(")):
        return descriptor
    return f"ct(slip77({master_blinding_key}),{descriptor})"


def _write_liquid_descriptor_files(
    base_dir: Path,
    *,
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
) -> None:
    descriptor_dir = base_dir / "imports" / "liquid-descriptors"
    descriptor_dir.mkdir(parents=True, exist_ok=True)
    try:
        payload = rpc(url, username, password, "listdescriptors", [False], wallet=wallet.core_wallet)
    except RuntimeError:
        payload = rpc(url, username, password, "listdescriptors", [], wallet=wallet.core_wallet)
    descriptors = payload.get("descriptors") if isinstance(payload, dict) else payload
    if not isinstance(descriptors, list):
        raise RuntimeError(f"Elements wallet {wallet.core_wallet} returned no descriptors")
    master_blinding_key = str(rpc(url, username, password, "dumpmasterblindingkey", [], wallet=wallet.core_wallet))
    receive = _blinded_liquid_descriptor(master_blinding_key, _active_descriptor(descriptors, internal=False))
    change = _blinded_liquid_descriptor(master_blinding_key, _active_descriptor(descriptors, internal=True))
    receive_path = descriptor_dir / f"{sanitize_wallet_segment(wallet.key)}-receive.txt"
    change_path = descriptor_dir / f"{sanitize_wallet_segment(wallet.key)}-change.txt"
    receive_path.write_text(receive + "\n", encoding="utf-8")
    change_path.write_text(change + "\n", encoding="utf-8")
    os.chmod(receive_path, 0o600)
    os.chmod(change_path, 0o600)
    wallet.descriptor_file = str(receive_path)
    wallet.change_descriptor_file = str(change_path)


def _elements_policy_asset_id(url: str, username: str, password: str) -> str:
    try:
        labels = rpc(url, username, password, "dumpassetlabels")
    except RuntimeError:
        return ""
    if isinstance(labels, dict):
        return str(labels.get("bitcoin") or labels.get("LBTC") or labels.get("lbtc") or "")
    return ""


def _advance_time(url: str, username: str, password: str, current_ts: int, *, seconds: int = 600) -> int:
    next_ts = current_ts + seconds
    rpc(url, username, password, "setmocktime", [next_ts])
    return next_ts


def _mine(
    url: str,
    username: str,
    password: str,
    _wallet: str,
    address: str,
    current_ts: int,
    *,
    blocks: int = 1,
) -> int:
    next_ts = _advance_time(url, username, password, current_ts)
    rpc(url, username, password, "generatetoaddress", [blocks, address])
    return next_ts


def _mine_at(
    url: str,
    username: str,
    password: str,
    address: str,
    current_ts: int,
    target_ts: int,
) -> int:
    next_ts = max(target_ts, current_ts + 600)
    rpc(url, username, password, "setmocktime", [next_ts])
    rpc(url, username, password, "generatetoaddress", [1, address])
    return next_ts


def _wallet_utxos(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    *,
    min_confirmations: int = 1,
) -> list[dict[str, Any]]:
    utxos = rpc(
        url,
        username,
        password,
        "listunspent",
        [min_confirmations, 9999999, wallet.addresses or [wallet.address], True],
        wallet=wallet.core_wallet,
    )
    return sorted(
        (utxo for utxo in utxos or [] if utxo.get("spendable", True)),
        key=lambda item: (Decimal(str(item["amount"])), item["txid"], item["vout"]),
        reverse=True,
    )


def _select_one_utxo(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    needed: Decimal,
) -> dict[str, Any]:
    for utxo in _wallet_utxos(url, username, password, wallet):
        if Decimal(str(utxo["amount"])) >= needed:
            return utxo
    raise RuntimeError(f"Wallet {wallet.key} has no confirmed watched UTXO >= {needed} BTC")


def _select_utxos(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    needed: Decimal,
) -> list[dict[str, Any]]:
    """Greedy largest-first coin selection so spends can combine several
    watched UTXOs, like a real wallet funding a payment across addresses."""
    selected: list[dict[str, Any]] = []
    total = Decimal("0")
    for utxo in _wallet_utxos(url, username, password, wallet):
        selected.append(utxo)
        total += Decimal(str(utxo["amount"])).quantize(SAT)
        if total >= needed:
            return selected
    raise RuntimeError(
        f"Wallet {wallet.key} holds {total} BTC across {len(selected)} confirmed UTXOs, "
        f"needs {needed} BTC"
    )


def _send_raw_transaction(
    url: str,
    username: str,
    password: str,
    inputs: list[dict[str, Any]],
    outputs: dict[str, Decimal],
    signers: list[str],
) -> str:
    raw = rpc(url, username, password, "createrawtransaction", [inputs, outputs])
    if len(signers) == 1:
        signed = rpc(url, username, password, "signrawtransactionwithwallet", [raw], wallet=signers[0])
        if not signed.get("complete"):
            raise RuntimeError(f"Core did not complete signing with {signers[0]}: {signed}")
        return rpc(url, username, password, "sendrawtransaction", [signed["hex"]])
    psbt = rpc(url, username, password, "converttopsbt", [raw])
    signed_psbts = []
    for signer in signers:
        processed = rpc(url, username, password, "walletprocesspsbt", [psbt], wallet=signer)
        signed_psbts.append(processed["psbt"])
    combined = rpc(url, username, password, "combinepsbt", [signed_psbts])
    finalized = rpc(url, username, password, "finalizepsbt", [combined])
    if not finalized.get("complete"):
        raise RuntimeError(f"Core did not complete PSBT finalization: {finalized}")
    return rpc(url, username, password, "sendrawtransaction", [finalized["hex"]])


def _send_from_wallet(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    outputs: dict[str, Decimal],
    fee: Decimal,
) -> str:
    needed = sum(outputs.values(), Decimal("0")) + fee
    selected = _select_utxos(url, username, password, wallet, needed)
    input_amount = sum((Decimal(str(utxo["amount"])).quantize(SAT) for utxo in selected), Decimal("0"))
    change = (input_amount - needed).quantize(SAT)
    if change < 0:
        raise RuntimeError(f"Selected UTXOs are too small for {wallet.key}")
    final_outputs = dict(outputs)
    if change > 0:
        change_address = wallet.change_address()
        final_outputs[change_address] = final_outputs.get(change_address, Decimal("0")) + change
    return _send_raw_transaction(
        url,
        username,
        password,
        [{"txid": utxo["txid"], "vout": utxo["vout"]} for utxo in selected],
        final_outputs,
        [wallet.core_wallet],
    )


def _send_batched_payment(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    faucet_wallet: str,
    operation: dict[str, Any],
) -> str:
    outputs = {}
    for index, value in enumerate(operation["outputs_btc"], start=1):
        address = rpc(
            url,
            username,
            password,
            "getnewaddress",
            [f"{operation['id']} recipient {index}", "bech32"],
            wallet=faucet_wallet,
        )
        outputs[address] = _btc(value)
    return _send_from_wallet(url, username, password, wallet, outputs, _btc(operation["fee_btc"]))


def _send_self_transfer_fanout(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    operation: dict[str, Any],
) -> str:
    sender = wallets[operation["from"]]
    outputs: dict[str, Decimal] = {}
    for output in operation["outputs"]:
        receiver = wallets[output["to"]]
        address = receiver.receive_address()
        outputs[address] = outputs.get(address, Decimal("0")) + _btc(output["amount_btc"])
    return _send_from_wallet(url, username, password, sender, outputs, _btc(operation["fee_btc"]))


def _send_incoming_burst(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    faucet_wallet: str,
    operation: dict[str, Any],
    txids: dict[str, str],
    truth: DemoTruth | None = None,
) -> None:
    count = int(operation["count"])
    amount = _btc(operation["amount_btc"])
    for index in range(1, count + 1):
        # Roughen each point-of-sale receipt by a few sats and rotate the
        # invoice address, like a merchant terminal handing out fresh invoices.
        ragged = amount + ((index * 137) % 89) * SAT
        op_key = f"{operation['id']}_{index:03d}"
        txids[op_key] = rpc(
            url,
            username,
            password,
            "sendtoaddress",
            [wallet.receive_address(), ragged],
            wallet=faucet_wallet,
        )
        if truth is not None:
            truth.record_transaction(op_key, txids[op_key], wallet, "inbound")


def _send_many_input_consolidation(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    operation: dict[str, Any],
) -> str:
    requested_count = int(operation["count"])
    fee = _btc(operation["fee_btc"])
    utxos = sorted(
        _wallet_utxos(url, username, password, wallet),
        key=lambda item: Decimal(str(item["amount"])),
    )
    if len(utxos) < requested_count:
        raise RuntimeError(
            f"Wallet {wallet.key} has only {len(utxos)} UTXOs for "
            f"{requested_count}-input consolidation"
        )
    selected = utxos[:requested_count]
    input_amount = sum((Decimal(str(utxo["amount"])).quantize(SAT) for utxo in selected), Decimal("0"))
    output_amount = (input_amount - fee).quantize(SAT)
    if output_amount <= 0:
        raise RuntimeError(f"Consolidation fee is too large for {wallet.key}: {fee} BTC")
    return _send_raw_transaction(
        url,
        username,
        password,
        [{"txid": utxo["txid"], "vout": utxo["vout"]} for utxo in selected],
        {wallet.receive_address(): output_amount},
        [wallet.core_wallet],
    )


def _wait_for_watchonly_mempool_tx(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    txid: str,
    *,
    attempts: int = 40,
) -> None:
    """Wait until the watch-only wallet has picked the mempool tx up.

    Core delivers mempool arrivals to wallets through an async validation
    queue, so an immediate sync could race past the notification.
    """
    if not wallet.watchonly_wallet:
        raise RuntimeError(f"Wallet {wallet.key} has no watch-only Core wallet to poll")
    for _ in range(attempts):
        try:
            rpc(url, username, password, "gettransaction", [txid], wallet=wallet.watchonly_wallet)
            return
        except RuntimeError:
            time.sleep(0.25)
    raise RuntimeError(f"Watch-only wallet for {wallet.key} never saw pending tx {txid}")


def _send_rbf_replaced_payment(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    external_address: str,
    operation: dict[str, Any],
    txids: dict[str, str],
) -> str:
    """Broadcast a replaceable payment, then bump its fee before it confirms.

    Only the replacement is ever mined; the original stays in the watch-only
    wallet history as a conflicted transaction with negative confirmations,
    which the sync adapter must skip instead of double-counting the spend.
    """
    amount = _btc(operation["amount_btc"])
    original_fee = _btc(operation["fee_btc"])
    replacement_fee = _btc(operation["replacement_fee_btc"])
    needed = amount + replacement_fee
    selected = _select_utxos(url, username, password, wallet, needed)
    input_amount = sum((Decimal(str(utxo["amount"])).quantize(SAT) for utxo in selected), Decimal("0"))
    inputs = [
        {"txid": utxo["txid"], "vout": utxo["vout"], "sequence": 0xFFFFFFFD}
        for utxo in selected
    ]
    change_address = wallet.change_address()

    def outputs_for(fee: Decimal) -> dict[str, Decimal]:
        change = (input_amount - amount - fee).quantize(SAT)
        if change <= 0:
            raise RuntimeError(f"RBF selection leaves no change for {wallet.key}")
        return {external_address: amount, change_address: change}

    txids[f"{operation['id']}_replaced"] = _send_raw_transaction(
        url,
        username,
        password,
        inputs,
        outputs_for(original_fee),
        [wallet.core_wallet],
    )
    return _send_raw_transaction(
        url,
        username,
        password,
        inputs,
        outputs_for(replacement_fee),
        [wallet.core_wallet],
    )


def _send_coinjoin_shape(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    operation: dict[str, Any],
    external_address: str,
) -> str:
    signer_keys = list(operation["signers"])
    if len(signer_keys) != 2:
        raise RuntimeError("coinjoin_shape currently expects exactly two signers")
    fee = _btc(operation["fee_btc"])
    equal_output = _btc(operation["equal_output_btc"])
    signer_a = wallets[signer_keys[0]]
    signer_b = wallets[signer_keys[1]]
    utxo_a = _select_one_utxo(url, username, password, signer_a, equal_output + fee)
    utxo_b = _select_one_utxo(url, username, password, signer_b, equal_output + fee)
    amount_a = Decimal(str(utxo_a["amount"])).quantize(SAT)
    amount_b = Decimal(str(utxo_b["amount"])).quantize(SAT)
    half_fee = (fee / 2).quantize(SAT)
    change_a = (amount_a - equal_output - half_fee).quantize(SAT)
    change_b = (amount_b - equal_output - (fee - half_fee)).quantize(SAT)
    if change_a <= 0 or change_b <= 0:
        raise RuntimeError("coinjoin_shape selected UTXOs leave no change")
    tracked_output_wallet = wallets[operation["tracked_output_wallet"]]
    outputs = {
        tracked_output_wallet.receive_address(): equal_output,
        external_address: equal_output,
        signer_a.change_address(): change_a,
        signer_b.change_address(): change_b,
    }
    return _send_raw_transaction(
        url,
        username,
        password,
        [
            {"txid": utxo_a["txid"], "vout": utxo_a["vout"]},
            {"txid": utxo_b["txid"], "vout": utxo_b["vout"]},
        ],
        outputs,
        [signer_a.core_wallet, signer_b.core_wallet],
    )


def _send_payjoin_shape(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    operation: dict[str, Any],
) -> str:
    payer = wallets[operation["payer"]]
    merchant = wallets[operation["merchant"]]
    payment = _btc(operation["payment_btc"])
    fee = _btc(operation["fee_btc"])
    payer_utxo = _select_one_utxo(url, username, password, payer, payment + fee)
    merchant_utxo = _select_one_utxo(url, username, password, merchant, SAT)
    payer_amount = Decimal(str(payer_utxo["amount"])).quantize(SAT)
    merchant_amount = Decimal(str(merchant_utxo["amount"])).quantize(SAT)
    payer_change = (payer_amount - payment - fee).quantize(SAT)
    merchant_output = (merchant_amount + payment).quantize(SAT)
    if payer_change <= 0:
        raise RuntimeError("payjoin_shape selected payer UTXO leaves no change")
    outputs = {
        merchant.receive_address(): merchant_output,
        payer.change_address(): payer_change,
    }
    return _send_raw_transaction(
        url,
        username,
        password,
        [
            {"txid": payer_utxo["txid"], "vout": payer_utxo["vout"]},
            {"txid": merchant_utxo["txid"], "vout": merchant_utxo["vout"]},
        ],
        outputs,
        [payer.core_wallet, merchant.core_wallet],
    )


def _resolved_operation(
    operation: dict[str, Any],
    resolve_wallet_key: Callable[[str], str],
) -> dict[str, Any]:
    resolved = dict(operation)
    for field in ("from", "to", "payer", "merchant", "tracked_output_wallet", "wallet"):
        value = resolved.get(field)
        if value and value != "external":
            resolved[field] = resolve_wallet_key(str(value))
    if operation.get("signers"):
        resolved["signers"] = [resolve_wallet_key(str(value)) for value in operation["signers"]]
    if operation.get("outputs"):
        resolved["outputs"] = [
            {**output, "to": resolve_wallet_key(str(output["to"]))}
            for output in operation["outputs"]
        ]
    return resolved


def _execute_scenario_operation(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    operation: dict[str, Any],
    *,
    counterparty_wallets: dict[str, str],
    counterparty_addresses: dict[str, str],
    txids: dict[str, str],
    truth: DemoTruth,
) -> None:
    kind = operation["kind"]
    counterparty = str(operation.get("counterparty") or "")
    inbound_wallet = counterparty_wallets.get(counterparty) or counterparty_wallets["customer_pool"]
    outbound_address = counterparty_addresses.get(counterparty) or counterparty_addresses["supplier"]
    if kind in {"payment", "self_transfer", "loan_collateral_lock", "loan_principal_repaid"}:
        sender = wallets[operation["from"]]
        to_address = outbound_address if operation["to"] == "external" else wallets[operation["to"]].receive_address()
        txids[operation["id"]] = _send_from_wallet(
            url,
            username,
            password,
            sender,
            {to_address: _btc(operation["amount_btc"])},
            _btc(operation["fee_btc"]),
        )
        truth.record_transaction(operation["id"], txids[operation["id"]], sender, "outbound")
        if operation["to"] != "external":
            truth.record_transaction(
                operation["id"],
                txids[operation["id"]],
                wallets[operation["to"]],
                "inbound",
            )
    elif kind == "batched_payment":
        txids[operation["id"]] = _send_batched_payment(
            url,
            username,
            password,
            wallets[operation["from"]],
            counterparty_wallets.get(counterparty) or counterparty_wallets["supplier"],
            operation,
        )
        truth.record_transaction(operation["id"], txids[operation["id"]], wallets[operation["from"]], "outbound")
    elif kind == "self_transfer_fanout":
        txids[operation["id"]] = _send_self_transfer_fanout(url, username, password, wallets, operation)
        truth.record_transaction(operation["id"], txids[operation["id"]], wallets[operation["from"]], "outbound")
        for output in operation.get("outputs") or []:
            truth.record_transaction(operation["id"], txids[operation["id"]], wallets[output["to"]], "inbound")
    elif kind == "incoming_burst":
        _send_incoming_burst(
            url,
            username,
            password,
            wallets[operation["to"]],
            inbound_wallet,
            operation,
            txids,
            truth=truth,
        )
    elif kind == "many_input_consolidation":
        txids[operation["id"]] = _send_many_input_consolidation(
            url, username, password, wallets[operation["wallet"]], operation
        )
        truth.record_transaction(operation["id"], txids[operation["id"]], wallets[operation["wallet"]], "outbound")
    elif kind == "coinjoin_shape":
        txids[operation["id"]] = _send_coinjoin_shape(
            url,
            username,
            password,
            wallets,
            operation,
            counterparty_addresses["customer_pool"],
        )
        for signer_key in operation["signers"]:
            truth.record_transaction(
                operation["id"],
                txids[operation["id"]],
                wallets[signer_key],
                "outbound",
                source="collaborative_review",
            )
        truth.record_transaction(
            operation["id"],
            txids[operation["id"]],
            wallets[operation["tracked_output_wallet"]],
            "inbound",
            source="collaborative_review",
        )
    elif kind == "payjoin_shape":
        txids[operation["id"]] = _send_payjoin_shape(url, username, password, wallets, operation)
        truth.record_transaction(
            operation["id"], txids[operation["id"]], wallets[operation["payer"]], "outbound", source="collaborative_review"
        )
        truth.record_transaction(
            operation["id"], txids[operation["id"]], wallets[operation["merchant"]], "outbound", source="collaborative_review"
        )
    elif kind == "rbf_replaced_payment":
        txids[operation["id"]] = _send_rbf_replaced_payment(
            url,
            username,
            password,
            wallets[operation["from"]],
            outbound_address,
            operation,
            txids,
        )
        truth.record_transaction(operation["id"], txids[operation["id"]], wallets[operation["from"]], "outbound")
        truth.record_skipped_txid(
            f"{operation['id']}_replaced",
            txids[f"{operation['id']}_replaced"],
            "rbf_conflicted_original",
        )
    elif kind in {"loan_collateral_release", "loan_principal_received", "external_receipt"}:
        receiver = wallets[operation["to"]]
        txids[operation["id"]] = rpc(
            url,
            username,
            password,
            "sendtoaddress",
            [receiver.receive_address(), _btc(operation["amount_btc"])],
            wallet=inbound_wallet,
        )
        truth.record_transaction(operation["id"], txids[operation["id"]], receiver, "inbound")
    else:
        raise RuntimeError(f"Unsupported scenario operation kind: {kind}")


def _generate_stress_history(
    url: str,
    elements_url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    scenario: dict[str, Any],
    *,
    liquid_faucet_wallet: str,
    counterparty_wallets: dict[str, str],
    counterparty_addresses: dict[str, str],
    mining_address: str,
    liquid_mining_address: str,
    external_address: str,
    liquid_external_address: str,
    current_ts: int,
    elements_current_ts: int,
    txids: dict[str, str],
    truth: DemoTruth,
) -> tuple[int, int, dict[str, Any]]:
    stress = scenario.get("stress") or {}
    if not stress.get("enabled"):
        return current_ts, elements_current_ts, {"cycles": 0, "rows_expected": 0, "span_days": 0}

    cycles = int(stress["cycles"])
    days_between_cycles = int(stress["days_between_cycles"])
    receipt_plan = dict(sorted(stress["receipt_btc"].items()))
    payment_plan = list(sorted(stress["payment_btc"].items()))
    fee = _btc(stress["fee_btc"])
    rate_history = _load_bundled_daily_rates(str(scenario["pricing"]["pair"]))
    variation_bp = int(stress.get("variation_bp") or 0)
    fee_spread_bp = 4000 if variation_bp else 0
    regimes = stress.get("economic_regimes") or []
    regime_labels_seen: list[str] = []
    active_wallet_for = {key: key for key in wallets}
    rotations_by_cycle: dict[int, list[dict[str, Any]]] = {}
    for rotation in stress.get("wallet_rotations") or []:
        rotations_by_cycle.setdefault(int(rotation["cycle"]), []).append(rotation)
    bridges_by_cycle: dict[int, list[dict[str, Any]]] = {}
    for bridge in stress.get("swap_bridges") or []:
        bridges_by_cycle.setdefault(int(bridge["cycle"]), []).append(bridge)
    mining_by_cycle: dict[int, list[dict[str, Any]]] = {}
    for event in stress.get("mining_events") or []:
        mining_by_cycle.setdefault(int(event["cycle"]), []).append(event)
    operations_by_cycle: dict[int, list[dict[str, Any]]] = {}
    for operation in scenario.get("operations") or []:
        if operation.get("cycle") is not None:
            operations_by_cycle.setdefault(int(operation["cycle"]), []).append(operation)
    expenses = stress.get("business_expenses") or {}
    expense_schedule = expenses.get("schedule") or []
    expense_every = int(expenses.get("every_cycles") or 1)
    expense_fee = _btc(expenses.get("fee_btc") or stress["fee_btc"])
    pool = stress.get("pool_payouts") or {}
    first_target_ts = current_ts + (2 * SECONDS_PER_DAY)
    rotations_count = 0
    business_expense_count = 0
    swap_bridge_count = 0
    mined_reward_count = 0
    pool_payout_count = 0
    scheduled_operation_count = 0
    receipt_event_count = 0
    payment_event_count = 0
    rare_receipt_count = 0
    skipped_cycles: list[int] = []
    doubled_cycles: list[int] = []
    cycle_rates: list[Decimal] = []

    def active_wallet(key_or_role: str) -> DemoWallet:
        return wallets[active_wallet_for.get(key_or_role, key_or_role)]

    for cycle in range(cycles):
        cycle_number = cycle + 1
        cycle_ts = _cycle_timestamp(first_target_ts, cycle_number, days_between_cycles)
        cycle_rate = _cached_rate_at_or_before(rate_history, cycle_ts)
        cycle_rates.append(cycle_rate)
        fee_multiplier = _fee_curve_multiplier(stress, cycle_ts)
        activity_mode = _cycle_activity_mode(cycle_number)
        activity_runs = 0 if activity_mode == "skip" else (2 if activity_mode == "double" else 1)
        if activity_mode == "skip":
            skipped_cycles.append(cycle_number)
        elif activity_mode == "double":
            doubled_cycles.append(cycle_number)
        receipt_scale, spend_scale, regime_label = _regime_scales(cycle_number, regimes)
        if regime_label not in regime_labels_seen:
            regime_labels_seen.append(regime_label)
        for rotation in rotations_by_cycle.get(cycle_number, []):
            sender = wallets[rotation["from"]]
            receiver = wallets[rotation["to"]]
            rotation_key = f"{rotation['id']}_rotation"
            txids[rotation_key] = _send_from_wallet(
                url,
                username,
                password,
                sender,
                {receiver.receive_address(): _btc(rotation["amount_btc"])},
                (_btc(rotation.get("fee_btc") or stress["fee_btc"]) * fee_multiplier).quantize(SAT),
            )
            if truth is not None:
                truth.record_transaction(rotation_key, txids[rotation_key], sender, "outbound")
                truth.record_transaction(rotation_key, txids[rotation_key], receiver, "inbound")
            active_wallet_for[rotation["role"]] = rotation["to"]
            rotations_count += 1
            current_ts = _mine_at(
                url,
                username,
                password,
                mining_address,
                current_ts,
                _event_timestamp(cycle_ts, cycle_number, 0),
            )

        event_slot = 1
        for run_index in range(activity_runs):
            receipt_targets = []
            for wallet_index, (key, amount_spec) in enumerate(receipt_plan.items()):
                rare_multiplier = _rare_receipt_multiplier(cycle_number, wallet_index, len(receipt_plan))
                if rare_multiplier > 1:
                    rare_receipt_count += 1
                base_amount = _recurring_btc(
                    amount_spec,
                    rate=cycle_rate,
                    label=f"stress.receipt_btc.{key}",
                )
                receipt_targets.append(
                    (
                        key,
                        active_wallet(key),
                        _varied_amount(
                            base_amount,
                            cycle_number,
                            salt=wallet_index + run_index * 13,
                            spread_bp=variation_bp,
                            ragged_sats=991,
                            scale=receipt_scale * rare_multiplier,
                        ),
                    )
                )
            receipt_outputs = {
                wallet.receive_address(): varied_amount
                for _key, wallet, varied_amount in receipt_targets
            }
            receipt_key = f"stress_receipt_{cycle_number:03d}_{run_index + 1}"
            receipt_source = ("customer_pool", "exchange")[(cycle_number + run_index) % 2]
            txids[receipt_key] = rpc(
                url,
                username,
                password,
                "sendmany",
                ["", receipt_outputs],
                wallet=counterparty_wallets[receipt_source],
            )
            if truth is not None:
                for _key, wallet, _varied_amount_value in receipt_targets:
                    truth.record_transaction(receipt_key, txids[receipt_key], wallet, "inbound")
            current_ts = _mine_at(
                url,
                username,
                password,
                mining_address,
                current_ts,
                _event_timestamp(cycle_ts, cycle_number, event_slot),
            )
            receipt_event_count += 1
            event_slot += 1

        for operation in operations_by_cycle.get(cycle_number, []):
            resolved = _resolved_operation(
                operation,
                lambda key: active_wallet_for.get(key, key),
            )
            _execute_scenario_operation(
                url,
                username,
                password,
                wallets,
                resolved,
                counterparty_wallets=counterparty_wallets,
                counterparty_addresses=counterparty_addresses,
                txids=txids,
                truth=truth,
            )
            current_ts = _mine_at(
                url,
                username,
                password,
                mining_address,
                current_ts,
                _event_timestamp(cycle_ts, cycle_number, event_slot),
            )
            scheduled_operation_count += 1
            event_slot += 1

        for event in mining_by_cycle.get(cycle_number, []):
            miner = active_wallet(event["role"])
            mine_ts = max(_event_timestamp(cycle_ts, cycle_number, event_slot), current_ts + 600)
            rpc(url, username, password, "setmocktime", [mine_ts])
            block_hashes = rpc(
                url,
                username,
                password,
                "generatetoaddress",
                [int(event.get("blocks") or 1), miner.receive_address()],
            )
            current_ts = mine_ts
            for reward_index, block_hash in enumerate(block_hashes or [], start=1):
                block = rpc(url, username, password, "getblock", [block_hash])
                coinbase_txids = block.get("tx") or []
                if not coinbase_txids:
                    raise RuntimeError(f"Mined block {block_hash} has no coinbase transaction")
                reward_key = f"{event['id']}_{reward_index:02d}"
                txids[reward_key] = coinbase_txids[0]
                if truth is not None:
                    truth.record_transaction(reward_key, txids[reward_key], miner, "inbound")
                mined_reward_count += 1
            event_slot += 1

        payments_per_run = 1 if spend_scale < Decimal("0.8") else (3 if spend_scale >= Decimal("1.2") else 2)
        for payment_index in range(activity_runs * payments_per_run):
            payer_key, amount_spec = payment_plan[(cycle + payment_index) % len(payment_plan)]
            payment_key = f"stress_payment_{cycle_number:03d}_{payment_index + 1}"
            destination = ("supplier", "exchange", "lender")[(cycle_number + payment_index) % 3]
            base_amount = _recurring_btc(
                amount_spec,
                rate=cycle_rate,
                label=f"stress.payment_btc.{payer_key}",
            )
            txids[payment_key] = _send_from_wallet(
                url,
                username,
                password,
                active_wallet(payer_key),
                {counterparty_addresses[destination]: _varied_amount(
                    base_amount,
                    cycle_number,
                    salt=17 + payment_index,
                    spread_bp=variation_bp,
                    scale=spend_scale,
                )},
                _varied_amount(
                    (fee * fee_multiplier).quantize(SAT),
                    cycle_number,
                    salt=23 + payment_index,
                    spread_bp=fee_spread_bp,
                ),
            )
            if truth is not None:
                truth.record_transaction(payment_key, txids[payment_key], active_wallet(payer_key), "outbound")
            current_ts = _mine_at(
                url,
                username,
                password,
                mining_address,
                current_ts,
                _event_timestamp(cycle_ts, cycle_number, event_slot),
            )
            payment_event_count += 1
            event_slot += 1

        if expenses.get("enabled") and expense_schedule and cycle % expense_every == 0:
            for expense_index, expense in enumerate(expense_schedule):
                every_cycles = int(expense.get("every_cycles") or 1)
                start_cycle = int(expense.get("start_cycle") or 1)
                if cycle_number < start_cycle or (cycle_number - start_cycle) % every_cycles:
                    continue
                expense_id = str(
                    expense.get("id") or expense.get("category") or f"expense_{cycle_number:03d}"
                )
                expense_key = f"business_expense_{cycle_number:03d}_{expense_id}"
                expense_amount = _recurring_btc(
                    expense,
                    rate=cycle_rate,
                    label=f"stress.business_expenses.schedule.{expense_id}",
                )
                txids[expense_key] = _send_from_wallet(
                    url,
                    username,
                    password,
                    active_wallet(expense["role"]),
                    {
                        counterparty_addresses[str(expense.get("counterparty") or "supplier")]: expense_amount
                    },
                    _varied_amount(
                        (expense_fee * fee_multiplier).quantize(SAT),
                        cycle_number,
                        salt=31 + expense_index,
                        spread_bp=fee_spread_bp,
                    ),
                )
                if truth is not None:
                    truth.record_transaction(
                        expense_key,
                        txids[expense_key],
                        active_wallet(expense["role"]),
                        "outbound",
                    )
                business_expense_count += 1
                current_ts = _mine_at(
                    url,
                    username,
                    password,
                    mining_address,
                    current_ts,
                    _event_timestamp(cycle_ts, cycle_number, event_slot),
                )
                event_slot += 1

        if (
            pool.get("enabled")
            and int(pool["start_cycle"]) <= cycle_number <= int(pool["end_cycle"])
            and (cycle_number - int(pool["start_cycle"])) % int(pool["every_cycles"]) == 0
        ):
            pool_key = f"mining_pool_payout_{cycle_number:03d}"
            pool_wallet = active_wallet(str(pool["role"]))
            pool_amount = _recurring_btc(pool, rate=cycle_rate, label="stress.pool_payouts")
            txids[pool_key] = rpc(
                url,
                username,
                password,
                "sendtoaddress",
                [pool_wallet.receive_address(), pool_amount],
                wallet=counterparty_wallets["mining_pool"],
            )
            if truth is not None:
                truth.record_transaction(pool_key, txids[pool_key], pool_wallet, "inbound")
            current_ts = _mine_at(
                url,
                username,
                password,
                mining_address,
                current_ts,
                _event_timestamp(cycle_ts, cycle_number, event_slot),
            )
            pool_payout_count += 1
            event_slot += 1

        for bridge in bridges_by_cycle.get(cycle_number, []):
            bridge_id = bridge["id"]
            source = active_wallet(bridge["from_role"])
            target = active_wallet(bridge["to_role"])
            out_ts = _event_timestamp(cycle_ts, cycle_number, event_slot)
            in_ts = _event_timestamp(cycle_ts, cycle_number, event_slot + 1)
            if _is_core_wallet(source):
                txids[f"{bridge_id}_out"] = _send_from_wallet(
                    url,
                    username,
                    password,
                    source,
                    {external_address: _btc(bridge["out_btc"])},
                    (_btc(bridge.get("fee_btc") or stress["fee_btc"]) * fee_multiplier).quantize(SAT),
                )
                if truth is not None:
                    truth.record_transaction(f"{bridge_id}_out", txids[f"{bridge_id}_out"], source, "outbound")
                current_ts = _mine_at(
                    url,
                    username,
                    password,
                    mining_address,
                    current_ts,
                    out_ts,
                )
            elif _is_liquid_live_wallet(source):
                elements_current_ts = max(elements_current_ts, out_ts - 600)
                txids[f"{bridge_id}_out"] = rpc(
                    elements_url,
                    username,
                    password,
                    "sendtoaddress",
                    [liquid_external_address, _btc(bridge["out_btc"])],
                    wallet=source.core_wallet,
                )
                if truth is not None:
                    truth.record_transaction(
                        f"{bridge_id}_out",
                        txids[f"{bridge_id}_out"],
                        source,
                        "outbound",
                        asset="LBTC",
                    )
                elements_current_ts = _mine_at(
                    elements_url,
                    username,
                    password,
                    liquid_mining_address,
                    elements_current_ts,
                    out_ts,
                )
            else:
                raise RuntimeError(f"Swap bridge {bridge_id} source is not syncable: {source.key}")

            if _is_core_wallet(target):
                txids[f"{bridge_id}_in"] = rpc(
                    url,
                    username,
                    password,
                    "sendtoaddress",
                    [target.receive_address(), _btc(bridge["in_btc"])],
                    wallet=faucet_wallet,
                )
                if truth is not None:
                    truth.record_transaction(f"{bridge_id}_in", txids[f"{bridge_id}_in"], target, "inbound")
            elif _is_liquid_live_wallet(target):
                elements_current_ts = max(elements_current_ts, in_ts - 600)
                txids[f"{bridge_id}_in"] = rpc(
                    elements_url,
                    username,
                    password,
                    "sendtoaddress",
                    [target.receive_address(), _btc(bridge["in_btc"])],
                    wallet=liquid_faucet_wallet,
                )
                if truth is not None:
                    truth.record_transaction(
                        f"{bridge_id}_in",
                        txids[f"{bridge_id}_in"],
                        target,
                        "inbound",
                        asset="LBTC",
                    )
                elements_current_ts = _mine_at(
                    elements_url,
                    username,
                    password,
                    liquid_mining_address,
                    elements_current_ts,
                    in_ts,
                )
            else:
                raise RuntimeError(f"Swap bridge {bridge_id} target is not syncable: {target.key}")
            swap_bridge_count += 1
            if _is_core_wallet(target):
                current_ts = _mine_at(
                    url,
                    username,
                    password,
                    mining_address,
                    current_ts,
                    in_ts,
                )
            elif not _is_liquid_live_wallet(target):
                current_ts = max(current_ts, in_ts)
            event_slot += 2

    return current_ts, elements_current_ts, {
        "cycles": cycles,
        "receipt_wallets": len(receipt_plan),
        "payment_wallets": len(payment_plan),
        "business_expenses": business_expense_count,
        "receipt_events": receipt_event_count,
        "payment_events": payment_event_count,
        "scheduled_operations": scheduled_operation_count,
        "pool_payouts": pool_payout_count,
        "rare_receipts": rare_receipt_count,
        "skipped_cycles": skipped_cycles,
        "doubled_cycles": doubled_cycles,
        "wallet_rotations": rotations_count,
        "swap_bridges": swap_bridge_count,
        "mined_rewards": mined_reward_count,
        "variation_bp": variation_bp,
        "economic_regimes": regime_labels_seen,
        "first_cycle_rate": format(cycle_rates[0], "f") if cycle_rates else None,
        "last_cycle_rate": format(cycle_rates[-1], "f") if cycle_rates else None,
        "rows_expected": (
            receipt_event_count * len(receipt_plan)
            + payment_event_count
            + business_expense_count
            + (rotations_count * 2)
            + (swap_bridge_count * 2)
            + mined_reward_count
            + pool_payout_count
        ),
        "span_days": (cycles - 1) * days_between_cycles + _stress_jitter_bp(cycles, 79, 5),
    }


def _scope(scenario: dict[str, Any]) -> tuple[str, str, str, str]:
    return (
        "--workspace",
        scenario["workspace"],
        "--profile",
        scenario["profile"]["label"],
    )


def _create_kassiber_book(
    data_root: Path,
    scenario: dict[str, Any],
    wallets: dict[str, DemoWallet],
    *,
    url: str,
    elements_url: str,
    username: str,
    password: str,
    wallet_prefix: str,
    birthday: str,
) -> None:
    run_cli(data_root, "init")
    run_cli(data_root, "workspaces", "create", scenario["workspace"])
    profile = scenario["profile"]
    run_cli(
        data_root,
        "profiles",
        "create",
        profile["label"],
        "--workspace",
        scenario["workspace"],
        "--fiat-currency",
        profile["fiat_currency"],
        "--tax-country",
        profile["tax_country"],
        "--gains-algorithm",
        profile["gains_algorithm"],
    )
    with tempfile.TemporaryFile("w+") as username_fd, tempfile.TemporaryFile("w+") as password_fd:
        username_fd.write(username)
        username_fd.flush()
        username_fd.seek(0)
        password_fd.write(password)
        password_fd.flush()
        password_fd.seek(0)
        run_cli(
            data_root,
            "backends",
            "create",
            scenario["backend"]["name"],
            "--kind",
            "bitcoinrpc",
            "--url",
            url,
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--username-fd",
            str(username_fd.fileno()),
            "--password-fd",
            str(password_fd.fileno()),
            "--wallet-prefix",
            wallet_prefix,
            "--timeout",
            "60",
            pass_fds=(username_fd.fileno(), password_fd.fileno()),
    )
    silent_payment_scan_file = next(
        (wallet.sp_scan_file for wallet in wallets.values() if _is_silent_payment_wallet(wallet) and wallet.sp_scan_file),
        None,
    )
    local_backend_specs = _local_backend_specs(silent_payment_scan_file=silent_payment_scan_file)
    for backend in local_backend_specs:
        backend_args = [
            "backends",
            "create",
            backend["name"],
            "--kind",
            backend["kind"],
            "--url",
            backend["url"],
            "--chain",
            backend["chain"],
            "--network",
            backend["network"],
            "--display-name",
            backend["display_name"],
            "--timeout",
            "10",
        ]
        if backend.get("silent_payments"):
            backend_args.append("--silent-payments")
        if backend.get("silent_payment_scan_file"):
            backend_args.extend(["--silent-payment-scan-file", str(backend["silent_payment_scan_file"])])
        run_cli(data_root, *backend_args)
    default_backend_name = str(scenario["backend"].get("default") or scenario["backend"]["name"])
    run_cli(data_root, "backends", "set-default", default_backend_name)
    configured_backends = run_cli(data_root, "backends", "list")["data"]
    allowed_backend_names = {scenario["backend"]["name"], *(backend["name"] for backend in local_backend_specs)}
    for backend in configured_backends:
        name = str(backend.get("name") or "")
        if not name or name in allowed_backend_names:
            continue
        source = str(backend.get("source") or "").lower()
        network = str(backend.get("network") or "").lower()
        if source == "database" and network not in {"regtest", "elementsregtest"}:
            run_cli(data_root, "backends", "delete", name)
    remaining_backends = run_cli(data_root, "backends", "list")["data"]
    unexpected_backends = [
        backend
        for backend in remaining_backends
        if str(backend.get("name") or "") not in allowed_backend_names
        or str(backend.get("network") or "").lower() not in {"regtest", "elementsregtest"}
    ]
    if unexpected_backends:
        rendered = [
            {
                "name": row.get("name"),
                "kind": row.get("kind"),
                "network": row.get("network"),
                "source": row.get("source"),
            }
            for row in unexpected_backends
        ]
        raise RuntimeError(f"Regtest demo must not keep public/default backends: {rendered}")
    scope = _scope(scenario)
    existing_accounts = {
        account["code"]
        for account in run_cli(data_root, "accounts", "list", *scope)["data"]
    }
    for account in scenario["accounts"]:
        if account["code"] in existing_accounts:
            continue
        run_cli(
            data_root,
            "accounts",
            "create",
            *scope,
            "--code",
            account["code"],
            "--label",
            account["label"],
            "--type",
            account["type"],
            "--asset",
            account["asset"],
        )
    for wallet_spec in scenario["wallets"]:
        wallet = wallets[wallet_spec["key"]]
        if _is_core_wallet(wallet):
            address_args: list[str] = []
            for watched_address in wallet.addresses or [wallet.address]:
                address_args.extend(["--address", watched_address])
            created = run_cli(
                data_root,
                "wallets",
                "create",
                *scope,
                "--label",
                wallet.label,
                "--kind",
                "address",
                "--account",
                wallet.account,
                "--backend",
                scenario["backend"]["name"],
                "--chain",
                "bitcoin",
                "--network",
                "regtest",
                *address_args,
                "--birthday",
                birthday,
            )["data"]
        elif _is_silent_payment_wallet(wallet):
            if not wallet.sp_scan_file:
                raise RuntimeError(f"Silent Payments wallet {wallet.key} has no generated scan file")
            descriptor_path = Path(wallet.sp_scan_file).with_name(f"{sanitize_wallet_segment(wallet.key)}-descriptor.txt")
            descriptor_path.write_text(wallet.sp_descriptor + "\n", encoding="utf-8")
            os.chmod(descriptor_path, 0o600)
            created = run_cli(
                data_root,
                "wallets",
                "create",
                *scope,
                "--label",
                wallet.label,
                "--kind",
                core_silent_payments.WALLET_KIND,
                "--account",
                wallet.account,
                "--backend",
                "bitcoin-frigate-regtest",
                "--chain",
                "bitcoin",
                "--network",
                "regtest",
                "--sp-descriptor-file",
                str(descriptor_path),
                "--sp-scan-start-height",
                str(wallet.sp_scan_start_height),
            )["data"]
        elif _is_liquid_live_wallet(wallet):
            if not wallet.descriptor_file or not wallet.change_descriptor_file:
                raise RuntimeError(f"Liquid live wallet {wallet.key} has no generated descriptor files")
            policy_asset = _elements_policy_asset_id(elements_url, username, password)
            args = [
                "wallets",
                "create",
                *scope,
                "--label",
                wallet.label,
                "--kind",
                "descriptor",
                "--account",
                wallet.account,
                "--backend",
                "liquid-electrum-regtest",
                "--chain",
                "liquid",
                "--network",
                "elementsregtest",
                "--descriptor-file",
                wallet.descriptor_file,
                "--change-descriptor-file",
                wallet.change_descriptor_file,
                "--gap-limit",
                str(max(20, len(wallet.addresses) + 10)),
            ]
            if policy_asset:
                args.extend(["--policy-asset", policy_asset])
            created = run_cli(data_root, *args)["data"]
        else:
            wallet_config = {
                "chain": wallet.chain,
                "network": wallet.network,
                "source_file": wallet.source_file,
                "source_format": wallet.source_format,
            }
            created = run_cli(
                data_root,
                "wallets",
                "create",
                *scope,
                "--label",
                wallet.label,
                "--kind",
                wallet.kind,
                "--account",
                wallet.account,
                "--config",
                json.dumps(wallet_config, sort_keys=True),
            )["data"]
        wallet.kassiber_id = created["id"]
        if _is_core_wallet(wallet):
            wallet.watchonly_wallet = (
                f"{sanitize_wallet_segment(wallet_prefix)}-{sanitize_wallet_segment(created['id'])}"
            )


def _seed_synthetic_fallback_rates(
    data_root: Path,
    scenario: dict[str, Any],
    unique_times: list[str],
) -> dict[str, Any]:
    pricing = scenario["pricing"]
    fallback = pricing.get("fallback") or pricing
    base_rate = Decimal(fallback["base_rate"])
    step_rate = Decimal(fallback["step_rate"])
    rate_sequence = [Decimal(str(value)) for value in fallback.get("rate_sequence") or []]
    trend_rate = Decimal(str(fallback.get("trend_rate") or step_rate))
    pair = pricing["pair"]
    for index, occurred_at in enumerate(unique_times):
        if rate_sequence:
            rate = rate_sequence[index % len(rate_sequence)] + (trend_rate * (index // len(rate_sequence)))
        else:
            rate = base_rate + (step_rate * index)
        run_cli(
            data_root,
            "rates",
            "set",
            pair,
            occurred_at,
            str(rate),
            "--source",
            "regtest-demo-fallback",
            "--granularity",
            "exact",
            "--method",
            scenario["id"],
        )
    return {
        "source": "regtest-demo-fallback",
        "pair": pair,
        "samples": len(unique_times),
        "granularity": "exact",
        "method": scenario["id"],
    }


def _seed_real_price_cache(
    data_root: Path,
    scenario: dict[str, Any],
    unique_times: list[str],
) -> dict[str, Any]:
    pricing = scenario["pricing"]
    pair = pricing["pair"]
    source = pricing.get("source") or "kraken-bundled"
    if source != "kraken-bundled":
        return {
            "seed": _seed_synthetic_fallback_rates(data_root, scenario, unique_times),
            "fallback_reason": f"unsupported pricing source {source!r}",
        }

    archive_path = ROOT / "kassiber" / "data" / "rates" / "kraken" / "btc_daily"
    seed = run_cli(
        data_root,
        "rates",
        "sync",
        "--pair",
        pair,
        "--source",
        "kraken-csv",
        "--path",
        str(archive_path),
    )["data"]
    result: dict[str, Any] = {
        "seed": seed,
        "source": "kraken-bundled",
        "path": str(archive_path),
        "pair": pair,
        "transaction_times": len(unique_times),
    }

    live_env = str(pricing.get("live_source_env") or "KASSIBER_REGTEST_DEMO_LIVE_RATES")
    live_source = str(
        os.environ.get(live_env) or pricing.get("live_source") or ""
    ).strip().lower()
    if live_source and live_source not in {"0", "false", "no", "off"}:
        conn = open_db(data_root)
        try:
            normalized_live_source = core_rates.normalize_market_rate_provider(live_source)
            result["live"] = {
                "source": normalized_live_source,
                "pair": pair,
                "latest": core_rates.sync_latest_rates(
                    conn,
                    pair=pair,
                    source=normalized_live_source,
                    commit=True,
                ),
            }
            conn.execute(
                "DELETE FROM settings WHERE key = ? AND value = ?",
                (core_rates.MARKET_RATE_PROVIDER_SETTING, core_rates.RATE_SOURCE_MEMPOOL),
            )
            conn.commit()
        finally:
            conn.close()
    else:
        conn = open_db(data_root)
        try:
            for source in core_rates.LIVE_MARKET_RATE_SOURCES:
                conn.execute(
                    "DELETE FROM rates_cache WHERE pair = ? AND source = ? AND granularity = 'latest'",
                    (pair, source),
                )
            conn.execute(
                "DELETE FROM settings WHERE key = ? AND value = ?",
                (core_rates.MARKET_RATE_PROVIDER_SETTING, core_rates.RATE_SOURCE_MEMPOOL),
            )
            conn.commit()
        finally:
            conn.close()
        result["live"] = {
            "skipped": True,
            "env": live_env,
            "reason": "not requested",
        }
    return result


def _seed_rates_and_process(data_root: Path, scenario: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]], dict[str, Any]]:
    scope = _scope(scenario)
    transactions = run_cli(
        data_root,
        "transactions",
        "list",
        *scope,
        "--order",
        "asc",
        "--limit",
        TRANSACTION_LIST_LIMIT,
    )["data"]
    unique_times = sorted({row["occurred_at"] for row in transactions})
    rate_seed = _seed_real_price_cache(data_root, scenario, unique_times)
    journal = run_cli(data_root, "journals", "process", *scope)["data"]
    transactions = run_cli(
        data_root,
        "transactions",
        "list",
        *scope,
        "--order",
        "asc",
        "--limit",
        TRANSACTION_LIST_LIMIT,
    )["data"]
    return journal, transactions, rate_seed


def _expected_ownership_fanout_routes(scenario: dict[str, Any]) -> list[dict[str, Any]]:
    labels = {wallet["key"]: wallet["label"] for wallet in scenario["wallets"]}
    routes = []
    for operation in scenario["operations"]:
        if operation.get("kind") != "self_transfer_fanout":
            continue
        for output in operation.get("outputs") or []:
            routes.append(
                {
                    "operation": operation["id"],
                    "from_wallet": labels[operation["from"]],
                    "to_wallet": labels[output["to"]],
                    "received_msat": int(_btc(output["amount_btc"]) * Decimal("100000000000")),
                }
            )
    return routes


def _assert_ownership_self_transfer_matching(
    data_root: Path,
    scenario: dict[str, Any],
) -> dict[str, Any]:
    """Prove ownership-derived transfer matching before manual pairs hide it."""
    scope = _scope(scenario)
    existing_pairs = run_cli(data_root, "transfers", "list", *scope)["data"]
    if existing_pairs:
        raise RuntimeError(
            "Ownership-derived regtest proof must run before manual transaction_pairs exist"
        )
    journal, _transactions, rate_seed = _seed_rates_and_process(data_root, scenario)
    audit = run_cli(data_root, "journals", "transfers", "list", *scope)["data"]
    expected_routes = _expected_ownership_fanout_routes(scenario)
    observed_routes = [
        {
            "from_wallet": row.get("from_wallet"),
            "to_wallet": row.get("to_wallet"),
            "received_msat": int(row.get("received_msat") or 0),
        }
        for row in audit.get("same_asset_transfers") or []
        if row.get("pairing_source") == "ownership_derived"
    ]
    missing = []
    for route in expected_routes:
        expected = {
            "from_wallet": route["from_wallet"],
            "to_wallet": route["to_wallet"],
            "received_msat": route["received_msat"],
        }
        if expected not in observed_routes:
            missing.append(route)
    expected_count = int(
        scenario.get("expected", {}).get("ownership_derived_transfer_pairs")
        or len(expected_routes)
    )
    if len(observed_routes) != expected_count or missing:
        raise RuntimeError(
            "Ownership-derived self-transfer matching did not satisfy the regtest "
            f"expectation: expected_count={expected_count}, "
            f"observed={observed_routes}, missing={missing}"
        )
    return {
        "journal": journal,
        "rates": rate_seed,
        "expected_routes": expected_routes,
        "observed_routes": observed_routes,
    }


def _pair_transfers(
    data_root: Path,
    scenario: dict[str, Any],
    txids: dict[str, str],
    truth: DemoTruth | None = None,
) -> list[dict[str, Any]]:
    scope = _scope(scenario)
    paired = []
    for operation in scenario["operations"]:
        if operation["kind"] != "self_transfer":
            continue
        if truth is not None:
            truth.record_transfer_pair(
                operation["id"],
                txids[operation["id"]],
                txids[operation["id"]],
                kind="manual",
                policy="carrying-value",
                note=operation["note"],
            )
        paired.append(
            run_cli(
                data_root,
                "transfers",
                "pair",
                *scope,
                "--tx-out",
                txids[operation["id"]],
                "--tx-in",
                txids[operation["id"]],
                "--kind",
                "manual",
                "--policy",
                "carrying-value",
                "--note",
                operation["note"],
            )["data"]
        )
    stress = scenario.get("stress") or {}
    for rotation in stress.get("wallet_rotations") or []:
        if truth is not None:
            truth.record_transfer_pair(
                rotation["id"],
                txids[f"{rotation['id']}_rotation"],
                txids[f"{rotation['id']}_rotation"],
                kind="manual",
                policy="carrying-value",
                note=rotation.get("note") or f"Wallet key rotation into {rotation['to']}.",
            )
        paired.append(
            run_cli(
                data_root,
                "transfers",
                "pair",
                *scope,
                "--tx-out",
                txids[f"{rotation['id']}_rotation"],
                "--tx-in",
                txids[f"{rotation['id']}_rotation"],
                "--kind",
                "manual",
                "--policy",
                "carrying-value",
                "--note",
                rotation.get("note") or f"Wallet key rotation into {rotation['to']}.",
            )["data"]
        )
    for bridge in stress.get("swap_bridges") or []:
        bridge_id = bridge["id"]
        if truth is not None:
            truth.record_transfer_pair(
                bridge_id,
                txids[f"{bridge_id}_out"],
                txids[f"{bridge_id}_in"],
                kind=bridge.get("pair_kind") or "submarine-swap",
                policy=bridge.get("pair_policy") or "taxable",
                note=bridge.get("note") or f"{bridge_id} bridge pair.",
            )
        paired.append(
            run_cli(
                data_root,
                "transfers",
                "pair",
                *scope,
                "--tx-out",
                txids[f"{bridge_id}_out"],
                "--tx-in",
                txids[f"{bridge_id}_in"],
                "--kind",
                bridge.get("pair_kind") or "submarine-swap",
                "--policy",
                bridge.get("pair_policy") or "taxable",
                "--note",
                bridge.get("note") or f"{bridge_id} bridge pair.",
            )["data"]
        )
    return paired


def _mark_deprecated_wallets(
    data_root: Path,
    scenario: dict[str, Any],
    wallets: dict[str, DemoWallet],
) -> list[dict[str, Any]]:
    scope = _scope(scenario)
    deprecated = []
    for wallet_key in scenario.get("deprecated_wallets") or []:
        wallet = wallets[wallet_key]
        if not wallet.kassiber_id:
            raise RuntimeError(f"Wallet {wallet_key} was not created before deprecation")
        updated = run_cli(
            data_root,
            "wallets",
            "update",
            *scope,
            "--wallet",
            wallet.kassiber_id,
            "--config",
            json.dumps({"deprecated": True}),
        )["data"]
        deprecated.append(updated)
    return deprecated


def _mark_loans(data_root: Path, scenario: dict[str, Any], txids: dict[str, str]) -> dict[str, Any]:
    scope = _scope(scenario)
    mark_as = {
        "loan_collateral_lock": "collateral",
        "loan_collateral_release": "returned",
        "loan_principal_received": "principal-received",
        "loan_principal_repaid": "principal-repaid",
    }
    marked_txids_by_loan: dict[str, list[str]] = defaultdict(list)
    marks = []
    for operation in scenario["operations"]:
        role = mark_as.get(operation["kind"])
        if role is None:
            continue
        txid = txids[operation["id"]]
        loan_id = str(operation["loan_id"])
        marked_txids_by_loan[loan_id].append(txid)
        marks.append(
            run_cli(
                data_root,
                "loans",
                "mark",
                *scope,
                "--txid",
                txid,
                "--as",
                role,
                "--loan-id",
                operation["loan_id"],
                "--note",
                operation["note"],
            )["data"]
        )
    for loan_id, marked_txids in sorted(marked_txids_by_loan.items()):
        if len(marked_txids) >= 2:
            link_args = []
            for txid in marked_txids:
                link_args.extend(["--txid", txid])
            run_cli(
                data_root,
                "loans",
                "link",
                *scope,
                *link_args,
                "--loan-id",
                loan_id,
            )
    listing = run_cli(data_root, "loans", "list", *scope)["data"]
    return {"marks": marks, "listing": listing}


def _exclude_collaborative_shapes(
    data_root: Path,
    scenario: dict[str, Any],
    txids: dict[str, str],
    transactions: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    scope = _scope(scenario)
    collaborative_txids = {
        txids[operation["id"]]
        for operation in scenario["operations"]
        if operation["kind"] in {"coinjoin_shape", "payjoin_shape"}
    }
    excluded = []
    for row in transactions:
        if row["external_id"] not in collaborative_txids:
            continue
        updated = run_cli(
            data_root,
            "metadata",
            "records",
            "excluded",
            "set",
            *scope,
            "--transaction",
            row["id"],
            "--reason",
            "Regtest collaborative transaction shape reviewed outside this tax-report scenario.",
        )["data"]
        excluded.append(
            {
                "transaction_id": row["id"],
                "external_id": row["external_id"],
                "wallet": row["wallet"],
                "direction": row["direction"],
                "excluded": bool(updated["excluded"]),
            }
        )
    return excluded


def _export_reports(data_root: Path, export_dir: Path, scenario: dict[str, Any]) -> dict[str, Any]:
    scope = _scope(scenario)
    export_dir.mkdir(parents=True, exist_ok=True)
    exports = {
        "report_pdf": export_dir / "full-report.pdf",
        "summary_pdf": export_dir / "wallet-summary.pdf",
        "report_csv": export_dir / "full-report.csv",
        "report_xlsx": export_dir / "full-report.xlsx",
        "transactions_csv": export_dir / "transactions.csv",
        "transactions_xlsx": export_dir / "transactions.xlsx",
    }
    results = {
        "report_pdf": run_cli(data_root, "reports", "export-pdf", *scope, "--file", str(exports["report_pdf"]))[
            "data"
        ],
        "summary_pdf": run_cli(
            data_root,
            "reports",
            "export-summary-pdf",
            *scope,
            "--start",
            str(scenario["base_time"]),
            "--end",
            str(scenario["latest_time"]),
            "--include-snapshot",
            "--file",
            str(exports["summary_pdf"]),
        )["data"],
        "report_csv": run_cli(data_root, "reports", "export-csv", *scope, "--file", str(exports["report_csv"]))[
            "data"
        ],
        "report_xlsx": run_cli(
            data_root, "reports", "export-xlsx", *scope, "--file", str(exports["report_xlsx"])
        )["data"],
        "transactions_csv": run_cli(
            data_root,
            "transactions",
            "export",
            *scope,
            "--export-format",
            "csv",
            "--file",
            str(exports["transactions_csv"]),
        )["data"],
        "transactions_xlsx": run_cli(
            data_root,
            "transactions",
            "export",
            *scope,
            "--export-format",
            "xlsx",
            "--file",
            str(exports["transactions_xlsx"]),
        )["data"],
    }
    missing = [name for name, path in exports.items() if not path.exists() or path.stat().st_size <= 0]
    if missing:
        raise RuntimeError(f"Missing or empty export files: {', '.join(missing)}")
    if not results["report_xlsx"].get("verified"):
        raise RuntimeError("XLSX export did not include the self-verification sheets")
    _assert_summary_pdf_wallet_snapshot(
        run_cli(data_root, "reports", "portfolio-summary", *scope)["data"],
        results["summary_pdf"],
    )
    try:
        import openpyxl
    except ImportError:
        pass
    else:
        workbook = openpyxl.load_workbook(exports["report_xlsx"], read_only=True, data_only=False)
        try:
            required_sheets = {"Verify", "Control", "Acquisitions", "Disposals"}
            missing_sheets = sorted(required_sheets.difference(workbook.sheetnames))
            if missing_sheets:
                raise RuntimeError(f"XLSX export is missing verification sheets: {missing_sheets}")
        finally:
            workbook.close()
    return {name: {"path": str(exports[name]), **results[name]} for name in exports}


def _assert_summary_pdf_wallet_snapshot(
    portfolio_rows: list[dict[str, Any]],
    summary_pdf: dict[str, Any],
) -> None:
    holdings_totals = summary_pdf.get("holdings_totals") or {}
    metrics = summary_pdf.get("metrics") or {}
    history_rows = summary_pdf.get("balance_history") or []
    if history_rows:
        final_history = history_rows[-1]
        total_quantity = dec(holdings_totals.get("total_quantity"))
        total_market_value = dec(holdings_totals.get("total_market_value"))
        history_quantity = dec(final_history.get("quantity"))
        history_market_value = dec(final_history.get("market_value"))
        period_end_value = dec(metrics.get("period_end_value"))
        btc_stack_end = dec(metrics.get("btc_stack_end"))
        if (
            abs(total_quantity - history_quantity) > Decimal("0.00000001")
            or abs(total_market_value - history_market_value) > Decimal("0.01")
            or abs(total_market_value - period_end_value) > Decimal("0.01")
            or abs(total_quantity - btc_stack_end) > Decimal("0.00000001")
        ):
            raise RuntimeError(
                "Wallet summary PDF period-end totals disagree with balance history:\n"
                + json.dumps(
                    {
                        "holdings_totals": {
                            "total_quantity": float(total_quantity),
                            "total_market_value": float(total_market_value),
                        },
                        "final_balance_history": {
                            "quantity": float(history_quantity),
                            "market_value": float(history_market_value),
                        },
                        "metrics": {
                            "period_end_value": float(period_end_value),
                            "btc_stack_end": float(btc_stack_end),
                        },
                    },
                    indent=2,
                    sort_keys=True,
                )
            )

    if not summary_pdf.get("snapshot"):
        raise RuntimeError("Wallet summary PDF export did not include the current snapshot")
    selected_wallets = {str(row.get("label") or "") for row in summary_pdf.get("wallets") or []}
    snapshot_rows = summary_pdf.get("snapshot_wallets") or []
    snapshot_wallets = {str(row.get("wallet") or "") for row in snapshot_rows}
    if selected_wallets != snapshot_wallets:
        rendered = json.dumps(
            {
                "missing": sorted(selected_wallets.difference(snapshot_wallets)),
                "extra": sorted(snapshot_wallets.difference(selected_wallets)),
            },
            indent=2,
            sort_keys=True,
        )
        raise RuntimeError(f"Wallet summary PDF snapshot did not cover every wallet:\n{rendered}")

    expected: dict[str, dict[str, Decimal]] = defaultdict(
        lambda: {"quantity": Decimal("0"), "market_value": Decimal("0")}
    )
    for row in portfolio_rows:
        bucket = expected[str(row.get("wallet") or "")]
        bucket["quantity"] += dec(row.get("quantity"))
        bucket["market_value"] += dec(row.get("market_value"))
    observed = {
        str(row.get("wallet") or ""): {
            "quantity": dec(row.get("quantity")),
            "market_value": dec(row.get("market_value")),
        }
        for row in snapshot_rows
    }
    mismatches = []
    for wallet, values in sorted(expected.items()):
        row = observed.get(wallet)
        if row is None:
            mismatches.append({"wallet": wallet, "reason": "missing"})
            continue
        if abs(row["quantity"] - values["quantity"]) > Decimal("0.00000001") or abs(
            row["market_value"] - values["market_value"]
        ) > Decimal("0.01"):
            mismatches.append(
                {
                    "wallet": wallet,
                    "expected": {
                        "quantity": float(values["quantity"]),
                        "market_value": float(values["market_value"]),
                    },
                    "observed": {
                        "quantity": float(row["quantity"]),
                        "market_value": float(row["market_value"]),
                    },
                }
            )
    if mismatches:
        raise RuntimeError(
            "Wallet summary PDF snapshot disagrees with portfolio-summary:\n"
            + json.dumps(mismatches, indent=2, sort_keys=True)
        )


def _assert_chain_edge_rows(
    scenario: dict[str, Any],
    txids: dict[str, str],
    transactions: list[dict[str, Any]],
) -> None:
    """Pin the on-chain edge cases the scenario stages deliberately."""
    rows_by_external_id = {row["external_id"]: row for row in transactions}
    for operation in scenario["operations"]:
        if operation["kind"] != "rbf_replaced_payment":
            continue
        replaced = txids[f"{operation['id']}_replaced"]
        if replaced in rows_by_external_id:
            raise RuntimeError(
                f"RBF-replaced original {replaced} must not be booked next to its replacement"
            )
        if txids[operation["id"]] not in rows_by_external_id:
            raise RuntimeError(f"RBF replacement for {operation['id']} is missing from the ledger")
    for event in (scenario.get("stress") or {}).get("mining_events") or []:
        mined = [txid for key, txid in txids.items() if key.startswith(f"{event['id']}_")]
        if not mined:
            raise RuntimeError(f"Mining event {event['id']} produced no coinbase txids")
        missing = [txid for txid in mined if txid not in rows_by_external_id]
        if missing:
            raise RuntimeError(f"Matured coinbase rewards were not synced: {missing}")
    for operation in scenario.get("pending_operations") or []:
        row = rows_by_external_id.get(txids[operation["id"]])
        if row is None:
            raise RuntimeError(f"Pending mempool receipt {operation['id']} was not synced")
        if row.get("confirmed_at"):
            raise RuntimeError(
                f"Pending mempool receipt {operation['id']} must stay unconfirmed at sync time"
            )


def _assert_live_liquid_sync_rows(
    scenario: dict[str, Any],
    txids: dict[str, str],
    transactions: list[dict[str, Any]],
) -> None:
    rows_by_external_id = {row["external_id"]: row for row in transactions}
    for wallet_spec in scenario["wallets"]:
        if not _is_liquid_live_wallet_spec(wallet_spec):
            continue
        wallet_key = wallet_spec["key"]
        wallet_label = str(wallet_spec["label"])
        expected = [
            (f"{wallet_key}_liquid_live_receive", "inbound", _btc(wallet_spec["live_receipt_btc"])),
            (f"{wallet_key}_liquid_live_spend", "outbound", _btc(wallet_spec["live_spend_btc"])),
        ]
        for txid_key, direction, expected_amount in expected:
            txid = txids.get(txid_key)
            if not txid:
                raise RuntimeError(f"Liquid live sync did not stage txid {txid_key}")
            row = rows_by_external_id.get(txid)
            if row is None:
                raise RuntimeError(f"Liquid live sync tx {txid_key}={txid} was not imported")
            if row.get("wallet") != wallet_label:
                raise RuntimeError(
                    f"Liquid live sync tx {txid} landed in wallet {row.get('wallet')!r}, "
                    f"expected {wallet_label!r}"
                )
            if str(row.get("asset") or "").upper() != "LBTC":
                raise RuntimeError(f"Liquid live sync tx {txid} has asset {row.get('asset')!r}, expected LBTC")
            if row.get("direction") != direction:
                raise RuntimeError(
                    f"Liquid live sync tx {txid} has direction {row.get('direction')!r}, expected {direction!r}"
                )
            actual_amount = Decimal(str(row.get("amount") or "0")).quantize(SAT)
            if actual_amount != expected_amount:
                raise RuntimeError(
                    f"Liquid live sync tx {txid} has amount {actual_amount}, expected {expected_amount}"
                )
            actual_fee = Decimal(str(row.get("fee") or "0")).quantize(SAT)
            if direction == "inbound" and actual_fee != Decimal("0"):
                raise RuntimeError(f"Liquid live sync inbound tx {txid} unexpectedly has fee {actual_fee}")
            if direction == "outbound" and actual_fee <= Decimal("0"):
                raise RuntimeError(f"Liquid live sync outbound tx {txid} should include a positive fee")
            if not row.get("confirmed_at"):
                raise RuntimeError(f"Liquid live sync tx {txid} should be confirmed")


def _collect_core_utxo_truth(
    truth: DemoTruth,
    *,
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
) -> None:
    for wallet in wallets.values():
        if not _is_core_wallet(wallet):
            continue
        utxos = []
        for utxo in _wallet_utxos(url, username, password, wallet, min_confirmations=0):
            confirmations = int(utxo.get("confirmations") or 0)
            utxos.append(
                {
                    "txid": str(utxo["txid"]).lower(),
                    "vout": int(utxo["vout"]),
                    "amount_msat": _btc_to_msat_int(utxo["amount"]),
                    "confirmation_status": "confirmed" if confirmations > 0 else "mempool",
                    "confirmations": confirmations,
                    "address": str(utxo.get("address") or ""),
                }
            )
        utxos = sorted(
            utxos,
            key=lambda row: (row["txid"], row["vout"], row["amount_msat"], row["address"]),
        )
        truth.core_utxos[wallet.key] = {
            "wallet_label": wallet.label,
            "wallet_id": wallet.kassiber_id,
            "asset": "BTC",
            "balance_msat": sum(row["amount_msat"] for row in utxos),
            "utxo_count": len(utxos),
            "utxos": utxos,
        }


def _db_core_utxo_rows(data_root: Path, truth: DemoTruth) -> dict[str, list[dict[str, Any]]]:
    wallet_ids = [
        str(entry.get("wallet_id") or "")
        for entry in truth.core_utxos.values()
        if entry.get("wallet_id")
    ]
    if not wallet_ids:
        return {}
    placeholders = ", ".join("?" for _ in wallet_ids)
    conn = open_db(data_root)
    try:
        rows = conn.execute(
            f"""
            SELECT
                wallet_id,
                txid,
                vout,
                amount,
                confirmation_status,
                COALESCE(address, '') AS address
            FROM wallet_utxos
            WHERE wallet_id IN ({placeholders}) AND spent_at IS NULL
            ORDER BY wallet_id, txid, vout
            """,
            wallet_ids,
        ).fetchall()
    finally:
        conn.close()
    grouped: dict[str, list[dict[str, Any]]] = {wallet_id: [] for wallet_id in wallet_ids}
    for row in rows:
        grouped.setdefault(row["wallet_id"], []).append(
            {
                "txid": str(row["txid"]).lower(),
                "vout": int(row["vout"]),
                "amount_msat": int(row["amount"] or 0),
                "confirmation_status": str(row["confirmation_status"] or ""),
                "address": str(row["address"] or ""),
            }
        )
    return grouped


def _utxo_key(row: dict[str, Any]) -> tuple[str, int, int, str, str]:
    return (
        str(row.get("txid") or "").lower(),
        int(row.get("vout") or 0),
        int(row.get("amount_msat") or 0),
        str(row.get("confirmation_status") or ""),
        str(row.get("address") or ""),
    )


def _utxo_key_dict(key: tuple[str, int, int, str, str]) -> dict[str, Any]:
    txid, vout, amount_msat, status, address = key
    return {
        "txid": txid,
        "vout": vout,
        "amount_msat": amount_msat,
        "confirmation_status": status,
        "address": address,
    }


def _read_transactions_csv_keys(path: Path) -> Counter:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.reader(handle))
    header_index = None
    required = {"Transaction ID", "Wallet", "Direction", "Asset"}
    for index, row in enumerate(rows):
        if required.issubset(set(row)):
            header_index = index
            break
    if header_index is None:
        raise RuntimeError(f"Transactions CSV export {path} is missing the transaction header")
    headers = rows[header_index]
    indexes = {name: headers.index(name) for name in required}
    keys: Counter = Counter()
    for row in rows[header_index + 1:]:
        if not row or not any(cell.strip() for cell in row):
            break
        keys[
            (
                row[indexes["Transaction ID"]].strip().lower(),
                row[indexes["Wallet"]].strip(),
                row[indexes["Asset"]].strip().upper(),
                row[indexes["Direction"]].strip(),
            )
        ] += 1
    return keys


def _write_generated_truth(path: Path, truth: DemoTruth) -> dict[str, Any]:
    payload = truth.to_dict()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return {
        "path": str(path),
        "transactions": payload["transactions"]["count"],
        "transfer_pairs": payload["transfer_pairs"]["count"],
        "core_wallets": len(payload["core_utxos"]),
    }


def _assert_generated_truth(
    data_root: Path,
    truth: DemoTruth,
    *,
    transactions: list[dict[str, Any]],
    transfers: dict[str, Any],
    journal: dict[str, Any],
    summary: dict[str, Any],
    exports: dict[str, Any],
) -> None:
    expected_tx = Counter(_truth_transaction_key(row) for row in truth.transaction_rows)
    actual_tx = Counter(
        (
            str(row.get("external_id") or "").lower(),
            str(row.get("wallet") or ""),
            str(row.get("asset") or "").upper(),
            str(row.get("direction") or ""),
        )
        for row in transactions
    )
    if expected_tx != actual_tx:
        rendered = json.dumps(_counter_diff(expected_tx, actual_tx, _truth_key_dict), indent=2, sort_keys=True)
        raise RuntimeError(f"Generated demo transaction truth mismatch:\n{rendered}")

    expected_unconfirmed = Counter(
        _truth_transaction_key(row)
        for row in truth.transaction_rows
        if row.get("confirmation_expected", True) and not row.get("confirmed", True)
    )
    confirmation_asserted_keys = {
        _truth_transaction_key(row)
        for row in truth.transaction_rows
        if row.get("confirmation_expected", True)
    }
    actual_unconfirmed = Counter(
        (
            str(row.get("external_id") or "").lower(),
            str(row.get("wallet") or ""),
            str(row.get("asset") or "").upper(),
            str(row.get("direction") or ""),
        )
        for row in transactions
        if not row.get("confirmed_at")
        and (
            str(row.get("external_id") or "").lower(),
            str(row.get("wallet") or ""),
            str(row.get("asset") or "").upper(),
            str(row.get("direction") or ""),
        )
        in confirmation_asserted_keys
    )
    if expected_unconfirmed != actual_unconfirmed:
        rendered = json.dumps(
            _counter_diff(expected_unconfirmed, actual_unconfirmed, _truth_key_dict),
            indent=2,
            sort_keys=True,
        )
        raise RuntimeError(f"Generated demo pending-transaction truth mismatch:\n{rendered}")

    skipped = {row["external_id"] for row in truth.skipped_txids}
    imported_skipped = sorted(skipped.intersection({row["external_id"].lower() for row in transactions}))
    if imported_skipped:
        raise RuntimeError(f"Skipped generated txids were imported unexpectedly: {imported_skipped}")

    expected_pairs = Counter(_pair_key(row) for row in truth.transfer_pairs)
    actual_pairs = Counter(_pair_key(row) for row in transfers["pairs"])
    if expected_pairs != actual_pairs:
        rendered = json.dumps(_counter_diff(expected_pairs, actual_pairs, _pair_key_dict), indent=2, sort_keys=True)
        raise RuntimeError(f"Generated demo transfer-pair truth mismatch:\n{rendered}")

    expected_excluded = sum(
        1 for row in truth.transaction_rows if row.get("source") == "collaborative_review"
    )
    expected_active = len(truth.transaction_rows) - expected_excluded
    expected_export_tx = Counter(
        _truth_transaction_key(row)
        for row in truth.transaction_rows
        if row.get("source") != "collaborative_review"
    )
    metrics = summary["metrics"]
    if int(metrics.get("active_transactions") or 0) != expected_active:
        raise RuntimeError(
            f"Expected exactly {expected_active} active transactions from generated truth, "
            f"got {metrics.get('active_transactions')}"
        )
    if int(metrics.get("excluded_transactions") or 0) != expected_excluded:
        raise RuntimeError(
            f"Expected exactly {expected_excluded} excluded transactions from generated truth, "
            f"got {metrics.get('excluded_transactions')}"
        )
    if int(metrics.get("priced_transactions") or 0) != expected_active:
        raise RuntimeError(
            f"Expected exactly {expected_active} priced transactions from generated truth, "
            f"got {metrics.get('priced_transactions')}"
        )
    if int(journal.get("processed_transactions") or 0) != expected_active:
        raise RuntimeError(
            f"Expected journal to process {expected_active} generated active transactions, "
            f"got {journal.get('processed_transactions')}"
        )

    db_utxos = _db_core_utxo_rows(data_root, truth)
    for wallet_key, expected in truth.core_utxos.items():
        wallet_id = str(expected.get("wallet_id") or "")
        actual_rows = db_utxos.get(wallet_id, [])
        expected_counter = Counter(_utxo_key(row) for row in expected["utxos"])
        actual_counter = Counter(_utxo_key(row) for row in actual_rows)
        if expected_counter != actual_counter:
            rendered = json.dumps(
                _counter_diff(expected_counter, actual_counter, _utxo_key_dict),
                indent=2,
                sort_keys=True,
            )
            raise RuntimeError(
                f"Generated UTXO truth mismatch for {wallet_key} ({expected['wallet_label']}):\n{rendered}"
            )
        actual_balance = sum(int(row["amount_msat"]) for row in actual_rows)
        if actual_balance != int(expected["balance_msat"]):
            raise RuntimeError(
                f"Generated balance truth mismatch for {wallet_key}: "
                f"expected {expected['balance_msat']} msat, got {actual_balance} msat"
            )

    transactions_csv = exports.get("transactions_csv") or {}
    transactions_xlsx = exports.get("transactions_xlsx") or {}
    if int(transactions_csv.get("rows") or -1) != expected_active:
        raise RuntimeError(
            f"Transactions CSV export reported {transactions_csv.get('rows')} rows, "
            f"expected {expected_active}"
        )
    if int(transactions_xlsx.get("rows") or -1) != expected_active:
        raise RuntimeError(
            f"Transactions XLSX export reported {transactions_xlsx.get('rows')} rows, "
            f"expected {expected_active}"
        )
    csv_keys = _read_transactions_csv_keys(Path(transactions_csv["path"]))
    if csv_keys != expected_export_tx:
        rendered = json.dumps(_counter_diff(expected_export_tx, csv_keys, _truth_key_dict), indent=2, sort_keys=True)
        raise RuntimeError(f"Transactions CSV export content does not match generated truth:\n{rendered}")


def _assert_expected(
    scenario: dict[str, Any],
    *,
    transactions: list[dict[str, Any]],
    transfers: dict[str, Any],
    loans: dict[str, Any],
    journal: dict[str, Any],
    quarantines: list[dict[str, Any]],
    collaborative_excluded: list[dict[str, Any]],
    wallet_listing: list[dict[str, Any]],
    summary: dict[str, Any],
    exports: dict[str, Any],
) -> None:
    expected = scenario["expected"]
    if expected.get("wallets") is not None and len(wallet_listing) != int(expected["wallets"]):
        raise RuntimeError(f"Expected {expected['wallets']} wallets, got {len(wallet_listing)}")
    expected_deprecated = expected.get("deprecated_wallets")
    if expected_deprecated is not None:
        deprecated_count = sum(1 for wallet in wallet_listing if wallet.get("deprecated"))
        if deprecated_count != int(expected_deprecated):
            raise RuntimeError(f"Expected {expected_deprecated} deprecated wallets, got {deprecated_count}")
    expected_assets = set(expected.get("assets") or [])
    if expected_assets:
        transaction_assets = {row.get("asset") for row in transactions if row.get("asset")}
        missing_assets = sorted(expected_assets.difference(transaction_assets))
        if missing_assets:
            raise RuntimeError(f"Expected assets were not imported into transactions: {missing_assets}")
    if len(transactions) < int(expected["min_transactions"]):
        raise RuntimeError(f"Expected at least {expected['min_transactions']} transactions, got {len(transactions)}")
    if len(transfers["pairs"]) < int(expected["min_transfer_pairs"]):
        raise RuntimeError(f"Expected at least {expected['min_transfer_pairs']} transfer pairs")
    if len(collaborative_excluded) != int(expected["collaborative_excluded"]):
        raise RuntimeError(
            f"Expected {expected['collaborative_excluded']} collaborative rows to be excluded, "
            f"got {len(collaborative_excluded)}"
        )
    loan_listing = loans["listing"]
    if len(loan_listing["marks"]) != int(expected["loan_marks"]):
        raise RuntimeError(f"Expected {expected['loan_marks']} loan marks, got {len(loan_listing['marks'])}")
    if len(loan_listing["open_locks"]) != int(expected["open_collateral_locks"]):
        raise RuntimeError(
            f"Expected {expected['open_collateral_locks']} open collateral locks, got {len(loan_listing['open_locks'])}"
        )
    if int(journal.get("quarantined") or 0) != 0:
        rendered = json.dumps(quarantines[:10], indent=2, sort_keys=True)
        raise RuntimeError(f"Expected zero journal quarantines, got {journal.get('quarantined')}: {rendered}")
    metrics = summary["metrics"]
    if expected_assets and int(metrics.get("assets_in_scope") or 0) < len(expected_assets):
        raise RuntimeError("Report summary did not include all expected assets")
    expected_pricing_source = expected.get("pricing_source")
    if expected_pricing_source:
        observed_sources = {row.get("pricing_provider") for row in transactions if row.get("pricing_provider")}
        if expected_pricing_source not in observed_sources:
            raise RuntimeError(f"Expected pricing provider {expected_pricing_source!r}, got {sorted(observed_sources)}")
    required_pricing_assets = {str(asset).upper() for asset in expected.get("require_pricing_provider_assets") or []}
    if required_pricing_assets:
        unpriced = [
            row
            for row in transactions
            if str(row.get("asset") or "").upper() in required_pricing_assets
            and row.get("excluded") not in {True, 1, "1", "true", "True"}
            and not row.get("pricing_provider")
        ]
        if unpriced:
            rendered = [
                {
                    "external_id": row.get("external_id"),
                    "asset": row.get("asset"),
                    "kind": row.get("kind"),
                    "occurred_at": row.get("occurred_at"),
                }
                for row in unpriced[:10]
            ]
            raise RuntimeError(f"Expected active {sorted(required_pricing_assets)} rows to be priced: {rendered}")
    min_active = int(expected.get("min_active_transactions") or expected["min_transactions"])
    if int(metrics.get("active_transactions") or 0) < min_active:
        raise RuntimeError("Report summary did not include the expected active transactions")
    expected_pending = expected.get("pending_transactions")
    if expected_pending is not None:
        pending_count = sum(
            1
            for row in transactions
            if str(row.get("asset") or "").upper() == "BTC" and not row.get("confirmed_at")
        )
        if pending_count < int(expected_pending):
            raise RuntimeError(
                f"Expected at least {expected_pending} unconfirmed BTC rows, got {pending_count}"
            )
    for filename in expected["export_files"]:
        if not any(Path(item["path"]).name == filename for item in exports.values()):
            raise RuntimeError(f"Expected export file was not produced: {filename}")


TICK_FEE_BTC = Decimal("0.00001000")
TICK_RECEIPT_MEMOS = (
    "Point-of-sale settlement",
    "Customer invoice paid",
    "Marketplace payout",
    "Subscription renewal",
    "Consulting retainer",
)
TICK_PAYMENT_MEMOS = (
    "Supplier invoice",
    "Cloud hosting bill",
    "Contractor payout",
    "Hardware purchase",
    "Utility payment",
)


def active_tick_wallets(scenario: dict[str, Any], wallets: dict[str, DemoWallet]) -> list[str]:
    """Core wallets that should still see fresh activity: on-chain and not
    marked deprecated by a key rotation."""
    deprecated = set(scenario.get("deprecated_wallets") or [])
    return [key for key, wallet in wallets.items() if _is_core_wallet(wallet) and key not in deprecated]


def _tick_amount(rng: random.Random, low: Decimal, high: Decimal) -> Decimal:
    """A random sat-granular amount in [low, high] — inherently ragged, no
    round numbers, so the ledger keeps looking like real business flow."""
    low_sats = int((low / SAT).to_integral_value())
    high_sats = int((high / SAT).to_integral_value())
    return (Decimal(rng.randint(low_sats, high_sats)) * SAT).quantize(SAT)


def plan_tick_operations(
    active_keys: list[str],
    rng: random.Random,
    *,
    receipts: int = 2,
    payments: int = 1,
    transfers: int = 1,
) -> list[dict[str, Any]]:
    """Build a randomized batch of simulated business activity.

    Pure: given the active wallet keys and a seeded RNG it deterministically
    returns receipt (external -> wallet), payment (wallet -> external), and
    self-transfer (wallet -> wallet) operations. Execution is separate so this
    can be unit-tested without a node.
    """
    active = list(active_keys)
    if not active:
        raise ValueError("No active core wallets available for a business tick")
    ops: list[dict[str, Any]] = []
    for _ in range(max(0, receipts)):
        ops.append(
            {
                "kind": "receipt",
                "wallet": rng.choice(active),
                "to": None,
                "amount_btc": _tick_amount(rng, Decimal("0.00050000"), Decimal("0.00500000")),
                "memo": rng.choice(TICK_RECEIPT_MEMOS),
            }
        )
    for _ in range(max(0, payments)):
        ops.append(
            {
                "kind": "payment",
                "wallet": rng.choice(active),
                "to": None,
                "amount_btc": _tick_amount(rng, Decimal("0.00030000"), Decimal("0.00300000")),
                "memo": rng.choice(TICK_PAYMENT_MEMOS),
            }
        )
    if len(active) >= 2:
        for _ in range(max(0, transfers)):
            source, target = rng.sample(active, 2)
            ops.append(
                {
                    "kind": "transfer",
                    "wallet": source,
                    "to": target,
                    "amount_btc": _tick_amount(rng, Decimal("0.00050000"), Decimal("0.00400000")),
                    "memo": "Internal treasury rebalance",
                }
            )
    rng.shuffle(ops)
    return ops


def execute_business_tick(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    *,
    faucet_wallet: str,
    mining_address: str,
    external_address: str,
    current_ts: int,
    plan: list[dict[str, Any]],
    fee: Decimal = TICK_FEE_BTC,
) -> tuple[int, dict[str, Any]]:
    """Broadcast a tick plan against the running node and mine one block so the
    activity confirms and the next incremental sync has real work to do."""
    executed: list[dict[str, Any]] = []
    for op in plan:
        wallet = wallets[op["wallet"]]
        amount = _btc(op["amount_btc"])
        if op["kind"] == "receipt":
            txid = rpc(
                url,
                username,
                password,
                "sendtoaddress",
                [wallet.receive_address(), amount],
                wallet=faucet_wallet,
            )
            direction = "inbound"
        elif op["kind"] == "payment":
            txid = _send_from_wallet(url, username, password, wallet, {external_address: amount}, fee)
            direction = "outbound"
        elif op["kind"] == "transfer":
            target = wallets[op["to"]]
            txid = _send_from_wallet(url, username, password, wallet, {target.receive_address(): amount}, fee)
            direction = "transfer"
        else:
            raise RuntimeError(f"Unsupported tick operation kind: {op['kind']}")
        executed.append(
            {
                "kind": op["kind"],
                "wallet": op["wallet"],
                "to": op.get("to"),
                "amount_btc": format(amount, "f"),
                "direction": direction,
                "memo": op.get("memo"),
                "txid": txid,
            }
        )
    current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts)
    tip = rpc(url, username, password, "getbestblockhash")
    return current_ts, {"operations": executed, "count": len(executed), "tip": tip}


def reconstruct_wallets_from_summary(
    scenario: dict[str, Any],
    summary_data: dict[str, Any],
    run_id: str,
) -> dict[str, DemoWallet]:
    """Rebuild DemoWallet objects for a running demo node from a saved summary,
    so a standalone tick can reconnect without re-running the whole demo."""
    summary_wallets = summary_data.get("wallets") or {}
    wallets: dict[str, DemoWallet] = {}
    for wallet_spec in scenario["wallets"]:
        key = wallet_spec["key"]
        entry = summary_wallets.get(key) or {}
        addresses = list(entry.get("addresses") or ([entry["address"]] if entry.get("address") else []))
        core_wallet = ""
        if _is_core_wallet_spec(wallet_spec):
            core_wallet = f"kassiber-demo-{run_id}-{sanitize_wallet_segment(key)}"
        chain = _wallet_chain(wallet_spec)
        wallets[key] = DemoWallet(
            key=key,
            label=wallet_spec["label"],
            account=wallet_spec["account"],
            kind=_wallet_kind(wallet_spec),
            chain=chain,
            network=str(wallet_spec.get("network") or ("liquidv1" if chain == "liquid" else "regtest")),
            core_wallet=core_wallet,
            address=addresses[0] if addresses else "",
            addresses=addresses,
            source_format=str(wallet_spec.get("source_format") or ""),
            sp_descriptor=str(wallet_spec.get("sp_descriptor") or ""),
            sp_scan_start_height=int(wallet_spec.get("sp_scan_start_height") or 0),
            sp_scan_file=str(entry.get("sp_scan_file") or ""),
            kassiber_id=entry.get("kassiber_id"),
        )
    return wallets


def demo_tick(
    *,
    scenario_path: Path = DEFAULT_SCENARIO,
    summary_path: Path,
    count: int = 1,
    seed: int | None = None,
    receipts: int = 2,
    payments: int = 1,
    transfers: int = 1,
) -> dict[str, Any]:
    """Stage fresh simulated business activity on a running demo node so the
    next in-app refresh/sync actually imports something."""
    scenario = load_scenario(scenario_path)
    summary = json.loads(Path(summary_path).read_text(encoding="utf-8"))["data"]
    run_id = summary["run_id"]
    url = os.environ.get("KASSIBER_REGTEST_CORE_URL") or summary.get("core_url") or "http://127.0.0.1:18443"
    username = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
    password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")

    chain = rpc(url, username, password, "getblockchaininfo")
    if chain.get("chain") != "regtest":
        raise RuntimeError(f"Refusing to tick against non-regtest Core node: {chain.get('chain')}")

    faucet_wallet = f"kassiber-demo-{run_id}-external"
    wallets = reconstruct_wallets_from_summary(scenario, summary, run_id)
    active = active_tick_wallets(scenario, wallets)
    if not active:
        raise RuntimeError("Demo summary has no active core wallets to tick")

    _ensure_wallet(url, username, password, faucet_wallet)
    for key in active:
        _ensure_wallet(url, username, password, wallets[key].core_wallet)

    mining_address = rpc(url, username, password, "getnewaddress", ["tick mining", "bech32"], wallet=faucet_wallet)
    external_address = rpc(url, username, password, "getnewaddress", ["tick external", "bech32"], wallet=faucet_wallet)

    rng = random.Random(seed)
    # Advance from the current tip but never behind wall clock, so new activity
    # is stamped "now" relative to the backdated historical span.
    current_ts = max(int(chain.get("mediantime") or 0) + 600, int(time.time()))
    ticks: list[dict[str, Any]] = []
    try:
        for _ in range(max(1, count)):
            plan = plan_tick_operations(active, rng, receipts=receipts, payments=payments, transfers=transfers)
            current_ts, tick_result = execute_business_tick(
                url,
                username,
                password,
                wallets,
                faucet_wallet=faucet_wallet,
                mining_address=mining_address,
                external_address=external_address,
                current_ts=current_ts,
                plan=plan,
            )
            ticks.append(tick_result)
    finally:
        try:
            rpc(url, username, password, "setmocktime", [0])
        except RuntimeError:
            # Best-effort return to real time; a dead node here must not mask a
            # tick error being unwound.
            pass

    return {
        "kind": "regtest.demo.tick",
        "schema_version": 1,
        "data": {
            "run_id": run_id,
            "core_url": url,
            "active_wallets": active,
            "ticks": ticks,
            "total_operations": sum(tick["count"] for tick in ticks),
        },
    }


def run_demo(
    *,
    scenario_path: Path = DEFAULT_SCENARIO,
    data_root: Path | None = None,
    export_dir: Path | None = None,
    run_id: str | None = None,
    keep_core_wallets: bool = False,
    run_business_tick: bool = True,
) -> dict[str, Any]:
    scenario = load_scenario(scenario_path)
    url = os.environ.get("KASSIBER_REGTEST_CORE_URL", "http://127.0.0.1:18443")
    elements_url = _elements_url()
    username = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
    password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")
    run_id = run_id or os.environ.get("KASSIBER_REGTEST_DEMO_RUN_ID") or uuid.uuid4().hex[:12]
    base_dir = Path(os.environ.get("KASSIBER_REGTEST_DEMO_ROOT", f"/tmp/kassiber-regtest-demo-{run_id}"))
    data_root = Path(data_root or os.environ.get("KASSIBER_REGTEST_DEMO_DATA_ROOT", base_dir / "data"))
    export_dir = Path(export_dir or os.environ.get("KASSIBER_REGTEST_DEMO_EXPORT_DIR", base_dir / "exports"))
    if data_root.exists() or export_dir.exists():
        raise RuntimeError(
            f"Demo output path already exists ({data_root} / {export_dir}). "
            "Set KASSIBER_REGTEST_DEMO_ROOT to a new directory or remove the old run."
        )

    chain = rpc(url, username, password, "getblockchaininfo")
    if chain.get("chain") != "regtest":
        raise RuntimeError(f"Refusing to run against non-regtest Core node: {chain.get('chain')}")
    elements_chain = rpc(elements_url, username, password, "getblockchaininfo")
    if elements_chain.get("chain") != "elementsregtest":
        raise RuntimeError(f"Refusing to run against non-elementsregtest node: {elements_chain.get('chain')}")

    manifest_ts = _parse_iso_to_ts(scenario["base_time"])
    chain_median_ts = int(chain.get("mediantime") or manifest_ts)
    elements_median_ts = int(elements_chain.get("mediantime") or manifest_ts)
    current_ts = max(manifest_ts, chain_median_ts + 7200, elements_median_ts + 7200)
    latest_ts = _parse_iso_to_ts(scenario["latest_time"])
    estimated_end_ts = estimate_scenario_end_ts(scenario, start_ts=current_ts)
    if estimated_end_ts > latest_ts:
        raise RuntimeError(
            "Regtest demo chain tip is too far ahead for the backdated accounting scenario "
            f"(estimated end {_iso_from_ts(estimated_end_ts)}, latest allowed {_iso_from_ts(latest_ts)}). "
            "Reset the persistent demo chain with './scripts/integration-harness.sh demo-down --purge' "
            "or run demo-up after the managed rebuild resets the chain volume."
        )
    rpc(url, username, password, "setmocktime", [current_ts])
    rpc(elements_url, username, password, "setmocktime", [current_ts])
    birthday_ts = current_ts - SECONDS_PER_DAY

    created_core_wallets: list[str] = []
    created_elements_wallets: list[str] = []
    backend_wallet_prefix = f"{scenario['backend']['wallet_prefix']}-{run_id}"
    faucet_wallet = f"kassiber-demo-{run_id}-external"
    liquid_faucet_wallet = f"kassiber-demo-{run_id}-liquid-external"
    wallets: dict[str, DemoWallet] = {}
    txids: dict[str, str] = {}
    truth = DemoTruth(scenario["id"])
    counterparty_wallets: dict[str, str] = {}
    counterparty_addresses: dict[str, str] = {}
    try:
        _ensure_wallet(url, username, password, faucet_wallet)
        created_core_wallets.append(faucet_wallet)
        _ensure_wallet(elements_url, username, password, liquid_faucet_wallet)
        created_elements_wallets.append(liquid_faucet_wallet)
        mining_address = rpc(url, username, password, "getnewaddress", ["mining", "bech32"], wallet=faucet_wallet)
        liquid_mining_confidential = rpc(
            elements_url,
            username,
            password,
            "getnewaddress",
            ["liquid mining"],
            wallet=liquid_faucet_wallet,
        )
        liquid_mining_address = _unconfidential_address(
            elements_url,
            username,
            password,
            liquid_faucet_wallet,
            liquid_mining_confidential,
        )
        liquid_external_address = rpc(
            elements_url,
            username,
            password,
            "getnewaddress",
            ["liquid external"],
            wallet=liquid_faucet_wallet,
        )
        elements_current_ts = current_ts
        current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts, blocks=181)
        elements_current_ts = _mine(
            elements_url,
            username,
            password,
            liquid_faucet_wallet,
            liquid_mining_address,
            elements_current_ts,
            blocks=101,
        )

        for counterparty in ("customer_pool", "supplier", "lender", "exchange", "mining_pool"):
            wallet_name = f"kassiber-demo-{run_id}-{counterparty.replace('_', '-')}"
            _ensure_wallet(url, username, password, wallet_name)
            created_core_wallets.append(wallet_name)
            counterparty_wallets[counterparty] = wallet_name
            counterparty_addresses[counterparty] = rpc(
                url,
                username,
                password,
                "getnewaddress",
                [counterparty.replace("_", " "), "bech32"],
                wallet=wallet_name,
            )
        rpc(
            url,
            username,
            password,
            "sendmany",
            [
                "",
                {
                    counterparty_addresses[key]: (
                        Decimal("1000") if key in {"customer_pool", "exchange"} else Decimal("100")
                    )
                    for key in counterparty_addresses
                },
            ],
            wallet=faucet_wallet,
        )
        current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts)
        external_address = counterparty_addresses["supplier"]

        for wallet_spec in scenario["wallets"]:
            if _is_core_wallet_spec(wallet_spec):
                core_wallet = f"kassiber-demo-{run_id}-{sanitize_wallet_segment(wallet_spec['key'])}"
                _ensure_wallet(url, username, password, core_wallet)
                created_core_wallets.append(core_wallet)
                addresses = [
                    rpc(
                        url,
                        username,
                        password,
                        "getnewaddress",
                        [f"{wallet_spec['label']} receive {index}", str(wallet_spec.get("address_type") or "bech32")],
                        wallet=core_wallet,
                    )
                    for index in range(1, int(wallet_spec.get("addresses") or 1) + 1)
                ]
            elif _is_liquid_live_wallet_spec(wallet_spec):
                core_wallet = f"kassiber-demo-{run_id}-{sanitize_wallet_segment(wallet_spec['key'])}"
                _ensure_wallet(elements_url, username, password, core_wallet)
                created_elements_wallets.append(core_wallet)
                addresses = [
                    rpc(
                        elements_url,
                        username,
                        password,
                        "getnewaddress",
                        [f"{wallet_spec['label']} receive {index}"],
                        wallet=core_wallet,
                    )
                    for index in range(1, int(wallet_spec.get("addresses") or 1) + 1)
                ]
            else:
                core_wallet = ""
                addresses = []
            wallets[wallet_spec["key"]] = DemoWallet(
                key=wallet_spec["key"],
                label=wallet_spec["label"],
                account=wallet_spec["account"],
                kind=_wallet_kind(wallet_spec),
                chain=_wallet_chain(wallet_spec),
                network=str(wallet_spec.get("network") or ("liquidv1" if _wallet_chain(wallet_spec) == "liquid" else "regtest")),
                core_wallet=core_wallet,
                address=addresses[0] if addresses else "",
                addresses=addresses,
                source_format=str(wallet_spec.get("source_format") or ""),
                sp_descriptor=str(wallet_spec.get("sp_descriptor") or ""),
                sp_scan_start_height=int(wallet_spec.get("sp_scan_start_height") or 0),
            )
            if _is_liquid_live_wallet_spec(wallet_spec):
                _write_liquid_descriptor_files(
                    base_dir,
                    url=elements_url,
                    username=username,
                    password=password,
                    wallet=wallets[wallet_spec["key"]],
                )

        # Seed each funded wallet across all of its watched addresses so the
        # book starts with a realistic spread of UTXOs instead of one coin.
        funding_outputs: dict[str, Decimal] = {}
        funded_wallet_keys: list[str] = []
        for wallet_spec in scenario["wallets"]:
            if not _is_core_wallet_spec(wallet_spec):
                continue
            initial_btc = _btc_or_zero(wallet_spec["initial_btc"])
            if initial_btc <= 0:
                continue
            wallet = wallets[wallet_spec["key"]]
            share = (initial_btc / len(wallet.addresses)).quantize(SAT)
            remainder = initial_btc - (share * len(wallet.addresses))
            for index, funding_address in enumerate(wallet.addresses):
                amount = share + (remainder if index == 0 else Decimal("0"))
                if amount > 0:
                    funding_outputs[funding_address] = amount
            funded_wallet_keys.append(wallet.key)
        txids["initial_funding"] = rpc(
            url,
            username,
            password,
            "sendmany",
            ["", funding_outputs],
            wallet=faucet_wallet,
        )
        for wallet_key in funded_wallet_keys:
            truth.record_transaction("initial_funding", txids["initial_funding"], wallets[wallet_key], "inbound")
        current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts)
        elements_current_ts = _seed_live_liquid_wallets(
            elements_url,
            username,
            password,
            scenario,
            wallets,
            faucet_wallet=liquid_faucet_wallet,
            mining_address=liquid_mining_address,
            external_address=liquid_external_address,
            current_ts=elements_current_ts,
            txids=txids,
            truth=truth,
        )

        for operation in scenario["operations"]:
            if operation.get("cycle") is not None:
                continue
            _execute_scenario_operation(
                url,
                username,
                password,
                wallets,
                operation,
                counterparty_wallets=counterparty_wallets,
                counterparty_addresses=counterparty_addresses,
                txids=txids,
                truth=truth,
            )
            current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts)

        current_ts, elements_current_ts, stress_result = _generate_stress_history(
            url,
            elements_url,
            username,
            password,
            wallets,
            scenario,
            liquid_faucet_wallet=liquid_faucet_wallet,
            counterparty_wallets=counterparty_wallets,
            counterparty_addresses=counterparty_addresses,
            mining_address=mining_address,
            liquid_mining_address=liquid_mining_address,
            external_address=external_address,
            liquid_external_address=liquid_external_address,
            current_ts=current_ts,
            elements_current_ts=elements_current_ts,
            txids=txids,
            truth=truth,
        )

        current_ts = _write_silent_payment_scan_files(
            base_dir,
            scenario,
            wallets,
            url=url,
            username=username,
            password=password,
            faucet_wallet=faucet_wallet,
            mining_address=mining_address,
            current_ts=current_ts,
            txids=txids,
            truth=truth,
        )
        birthday = datetime.fromtimestamp(birthday_ts, tz=timezone.utc)
        birthday_iso = birthday.isoformat().replace("+00:00", "Z")
        _create_kassiber_book(
            data_root,
            scenario,
            wallets,
            url=url,
            elements_url=elements_url,
            username=username,
            password=password,
            wallet_prefix=backend_wallet_prefix,
            birthday=birthday_iso,
        )
        created_core_wallets.extend(
            wallet.watchonly_wallet for wallet in wallets.values() if wallet.watchonly_wallet
        )

        scope = _scope(scenario)
        sync = run_cli(data_root, "wallets", "sync", *scope, "--all")["data"]

        # Broadcast the pending receipts only after the watch-only wallets
        # exist: a descriptor import rescans blocks, not the mempool, so a
        # payment sent earlier would be invisible until it confirmed. Doing it
        # here mirrors real usage (a payment arrives mid-session, the user
        # refreshes) and drives the incremental sinceblock sync path live.
        pending_sync = None
        if scenario.get("pending_operations"):
            for operation in scenario["pending_operations"]:
                receiver = wallets[operation["to"]]
                current_ts = _advance_time(url, username, password, current_ts)
                txids[operation["id"]] = rpc(
                    url,
                    username,
                    password,
                    "sendtoaddress",
                    [receiver.receive_address(), _btc(operation["amount_btc"])],
                    wallet=faucet_wallet,
                )
                truth.record_transaction(
                    operation["id"],
                    txids[operation["id"]],
                    receiver,
                    "inbound",
                    confirmed=False,
                    source="pending_mempool",
                )
                _wait_for_watchonly_mempool_tx(
                    url,
                    username,
                    password,
                    receiver,
                    txids[operation["id"]],
                )
            pending_sync = run_cli(data_root, "wallets", "sync", *scope, "--all")["data"]

        transactions = run_cli(
            data_root,
            "transactions",
            "list",
            *scope,
            "--limit",
            TRANSACTION_LIST_LIMIT,
            "--order",
            "asc",
        )["data"]
        collaborative_excluded = _exclude_collaborative_shapes(data_root, scenario, txids, transactions)
        ownership_matching = _assert_ownership_self_transfer_matching(data_root, scenario)
        pairs = _pair_transfers(data_root, scenario, txids, truth=truth)
        loan_result = _mark_loans(data_root, scenario, txids)
        deprecated_wallets = _mark_deprecated_wallets(data_root, scenario, wallets)
        journal, transactions, rate_seed = _seed_rates_and_process(data_root, scenario)
        _assert_chain_edge_rows(scenario, txids, transactions)
        _assert_live_liquid_sync_rows(scenario, txids, transactions)
        wallet_listing = run_cli(data_root, "wallets", "list", *scope)["data"]
        backend_listing = run_cli(data_root, "backends", "list")["data"]
        transfer_listing = run_cli(data_root, "transfers", "list", *scope)["data"]
        quarantines = run_cli(data_root, "journals", "quarantined", *scope)["data"]
        summary = run_cli(data_root, "reports", "summary", *scope)["data"]
        exports = _export_reports(data_root, export_dir, scenario)
        _refresh_truth_wallet_ids(truth, wallets)
        _collect_core_utxo_truth(truth, url=url, username=username, password=password, wallets=wallets)
        truth_export = _write_generated_truth(export_dir / "generated-truth.json", truth)
        _assert_generated_truth(
            data_root,
            truth,
            transactions=transactions,
            transfers={"pairs": transfer_listing},
            journal=journal,
            summary=summary,
            exports=exports,
        )
        _assert_expected(
            scenario,
            transactions=transactions,
            transfers={"pairs": transfer_listing},
            loans=loan_result,
            journal=journal,
            quarantines=quarantines,
            collaborative_excluded=collaborative_excluded,
            wallet_listing=wallet_listing,
            summary=summary,
            exports=exports,
        )

        # Prove the incremental resync path does real work: after the book is
        # already synced, stage fresh business activity and sync again. A no-op
        # resync (nothing imported) would mean "refresh" in the app is dead.
        resync = None
        if run_business_tick:
            tick_rng = random.Random(0xC0FFEE)
            active_keys = active_tick_wallets(scenario, wallets)
            tick_plan = plan_tick_operations(active_keys, tick_rng, receipts=3, payments=2, transfers=1)
            rows_before = len(transactions)
            current_ts, tick_result = execute_business_tick(
                url,
                username,
                password,
                wallets,
                faucet_wallet=faucet_wallet,
                mining_address=mining_address,
                external_address=external_address,
                current_ts=current_ts,
                plan=tick_plan,
            )
            resync_sync = run_cli(data_root, "wallets", "sync", *scope, "--all")["data"]
            resync_rows = resync_sync if isinstance(resync_sync, list) else [resync_sync]
            imported = sum(int(row.get("imported") or 0) for row in resync_rows)
            modes = sorted({str(row.get("bitcoinrpc_sync_mode")) for row in resync_rows if row.get("bitcoinrpc_sync_mode")})
            transactions = run_cli(
                data_root,
                "transactions",
                "list",
                *scope,
                "--limit",
                TRANSACTION_LIST_LIMIT,
                "--order",
                "asc",
            )["data"]
            if imported <= 0:
                raise RuntimeError(
                    f"Resync after a {tick_result['count']}-op business tick imported nothing (modes={modes})"
                )
            if len(transactions) <= rows_before:
                raise RuntimeError("Business tick + resync did not add ledger rows")
            resync = {
                "tick": tick_result,
                "imported": imported,
                "sync_modes": modes,
                "rows_before": rows_before,
                "rows_after": len(transactions),
            }

        by_direction = Counter(row["direction"] for row in transactions)
        by_wallet = Counter(row["wallet"] for row in transactions)
        result = {
            "kind": "regtest.demo.full_accounting",
            "schema_version": 1,
            "data": {
                "scenario": scenario["id"],
                "run_id": run_id,
                "data_root": str(data_root),
                "export_dir": str(export_dir),
                "core_url": url,
                "elements_url": elements_url,
                "base_time": datetime.fromtimestamp(current_ts, tz=timezone.utc).isoformat().replace("+00:00", "Z"),
                "wallets": {
                    key: {
                        "label": wallet.label,
                        "address": wallet.address,
                        "addresses": list(wallet.addresses),
                        "chain": wallet.chain,
                        "source_file": wallet.source_file,
                        "descriptor_file": wallet.descriptor_file,
                        "sp_scan_file": wallet.sp_scan_file,
                        "kassiber_id": wallet.kassiber_id,
                    }
                    for key, wallet in wallets.items()
                },
                "deprecated_wallets": [
                    {"id": wallet["id"], "label": wallet["label"], "deprecated": wallet.get("deprecated")}
                    for wallet in deprecated_wallets
                ],
                "operations": [{"id": key, "txid": value} for key, value in sorted(txids.items())],
                "sync": sync,
                "pending_sync": pending_sync,
                "resync": resync,
                "transactions": {
                    "count": len(transactions),
                    "by_direction": dict(sorted(by_direction.items())),
                    "by_wallet": dict(sorted(by_wallet.items())),
                },
                "backends": [
                    {
                        "name": backend.get("name"),
                        "kind": backend.get("kind"),
                        "chain": backend.get("chain"),
                        "network": backend.get("network"),
                        "is_default": backend.get("default") == "yes",
                    }
                    for backend in backend_listing
                ],
                "stress": stress_result,
                "transfers": {
                    "paired": pairs,
                    "count": len(transfer_listing),
                    "ownership_matching": ownership_matching,
                },
                "collaborative_excluded": collaborative_excluded,
                "loans": {
                    "marks": len(loan_result["listing"]["marks"]),
                    "open_locks": len(loan_result["listing"]["open_locks"]),
                    "loan_ids": sorted(
                        {mark["loan_id"] for mark in loan_result["listing"]["marks"] if mark.get("loan_id")}
                    ),
                },
                "journal": journal,
                "rates": rate_seed,
                "summary_metrics": summary["metrics"],
                "exports": exports,
                "truth": truth_export,
            },
        }
        return result
    except Exception:
        if os.environ.get("KASSIBER_REGTEST_DEMO_KEEP_FAILED"):
            print(
                f"Keeping failed regtest demo data_root={data_root} export_dir={export_dir}",
                file=sys.stderr,
            )
        else:
            if data_root.exists():
                shutil.rmtree(data_root, ignore_errors=True)
            if export_dir.exists():
                shutil.rmtree(export_dir, ignore_errors=True)
        raise
    finally:
        try:
            rpc(url, username, password, "setmocktime", [0])
        except RuntimeError:
            # Best-effort mocktime reset during teardown; if the node is already
            # gone this must not override the error (if any) being unwound.
            pass
        try:
            rpc(elements_url, username, password, "setmocktime", [0])
        except RuntimeError:
            # Best-effort Elements mocktime reset during teardown; if the node is
            # already gone this must not override the error being unwound.
            pass
        if not keep_core_wallets:
            for wallet_name in reversed(created_core_wallets):
                if wallet_name:
                    _unload_wallet(url, username, password, wallet_name)
            for wallet_name in reversed(created_elements_wallets):
                if wallet_name:
                    _unload_wallet(elements_url, username, password, wallet_name)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run the full Kassiber Bitcoin Core regtest demo scenario.")
    parser.add_argument("--scenario", type=Path, default=DEFAULT_SCENARIO)
    parser.add_argument("--data-root", type=Path)
    parser.add_argument("--export-dir", type=Path)
    parser.add_argument("--run-id")
    parser.add_argument("--json-output", type=Path)
    parser.add_argument("--keep-core-wallets", action="store_true")
    parser.add_argument("--no-business-tick", action="store_true", help="skip the post-sync resync proof")
    parser.add_argument(
        "--tick",
        action="store_true",
        help="stage fresh business activity on a running demo node (needs --summary)",
    )
    parser.add_argument("--summary", type=Path, help="path to a demo-summary.json from a previous run")
    parser.add_argument("--tick-count", type=int, default=1)
    parser.add_argument("--tick-seed", type=int, help="seed for reproducible tick activity (default: random)")
    parser.add_argument("--receipts", type=int, default=2)
    parser.add_argument("--payments", type=int, default=1)
    parser.add_argument("--transfers", type=int, default=1)
    args = parser.parse_args(argv)

    if args.tick:
        if not args.summary:
            parser.error("--tick requires --summary pointing at a demo-summary.json")
        result = demo_tick(
            scenario_path=args.scenario,
            summary_path=args.summary,
            count=args.tick_count,
            seed=args.tick_seed,
            receipts=args.receipts,
            payments=args.payments,
            transfers=args.transfers,
        )
    else:
        result = run_demo(
            scenario_path=args.scenario,
            data_root=args.data_root,
            export_dir=args.export_dir,
            run_id=args.run_id,
            keep_core_wallets=args.keep_core_wallets,
            run_business_tick=not args.no_business_tick,
        )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
