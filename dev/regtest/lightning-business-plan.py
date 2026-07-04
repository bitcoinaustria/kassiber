#!/usr/bin/env python3
"""Generate a deterministic Lightning + mainchain business workload.

The shape is intentionally inspired by sim-ln's split between random topology
traffic and defined activity: this script uses a seed plus capacity/amount
knobs to produce a stable list of explicit payments that shell scripts can
assert against. It does not execute anything; it only writes the plan.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
from pathlib import Path
from typing import Any


DEFAULT_SEED = "kassiber-lightning-business-v1"
DEFAULT_EXPECTED_PAYMENT_MSAT = 150_000_000
DEFAULT_CHANNEL_CAPACITY_SAT = 5_000_000
DEFAULT_CAPACITY_MULTIPLIER = "0.35"


def _seed_int(seed: str) -> int:
    return int.from_bytes(hashlib.sha256(seed.encode("utf-8")).digest()[:8], "big")


def _clamp(value: int, low: int, high: int) -> int:
    return max(low, min(high, value))


def _round_msat(value: int) -> int:
    # Keep all values sat-aligned so lightning-cli amount strings are simple
    # while still carrying msat units through the plan.
    return max(1_000, (value // 1_000) * 1_000)


def _series(
    rng: random.Random,
    *,
    count: int,
    target_msat: int,
    minimum_msat: int,
    maximum_msat: int,
) -> list[int]:
    if count <= 0:
        return []
    base = max(target_msat // count, minimum_msat)
    values: list[int] = []
    for _ in range(count):
        amount = int(base * rng.uniform(0.55, 1.45))
        values.append(_round_msat(_clamp(amount, minimum_msat, maximum_msat)))
    return values


def _cap_groups_to_budget(
    groups: list[list[int]],
    *,
    budget_msat: int,
    minimum_msat: int,
) -> bool:
    """Scale mutable amount groups down to fit a shared liquidity budget."""
    total = sum(sum(group) for group in groups)
    if total <= budget_msat:
        return False
    flat = [(group_index, item_index, value) for group_index, group in enumerate(groups) for item_index, value in enumerate(group)]
    minimum_total = minimum_msat * len(flat)
    if not flat or minimum_total > budget_msat:
        raise ValueError("business workload minimums exceed the channel liquidity budget")
    adjustable = sum(max(0, value - minimum_msat) for _, _, value in flat)
    factor = (budget_msat - minimum_total) / adjustable if adjustable else 0.0
    for group_index, item_index, value in flat:
        scaled = minimum_msat + int(max(0, value - minimum_msat) * factor)
        groups[group_index][item_index] = _round_msat(scaled)
    while sum(sum(group) for group in groups) > budget_msat:
        largest_group = 0
        largest_index = 0
        largest_value = -1
        for group_index, group in enumerate(groups):
            for item_index, value in enumerate(group):
                if value > largest_value and value > minimum_msat:
                    largest_group = group_index
                    largest_index = item_index
                    largest_value = value
        groups[largest_group][largest_index] -= 1_000
    return True


def _invoice_rows(
    amounts: list[int],
    *,
    label_prefix: str,
    descriptions: list[str],
    expiry: int = 3600,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, amount in enumerate(amounts, start=1):
        stem = descriptions[(index - 1) % len(descriptions)]
        rows.append(
            {
                "label": f"{label_prefix}-{index:03d}",
                "amount_msat": amount,
                "description": f"{stem} {index:03d}",
                "expiry": expiry,
            }
        )
    return rows


def build_plan(
    *,
    seed: str,
    expected_payment_msat: int,
    channel_capacity_sat: int,
    capacity_multiplier: float,
) -> dict[str, Any]:
    rng = random.Random(_seed_int(seed))
    channel_capacity_msat = channel_capacity_sat * 1_000
    turnover_msat = int(channel_capacity_msat * capacity_multiplier)
    scale = max(0.25, min(2.0, capacity_multiplier / 0.35))

    merchant_count = _clamp(round(5 + scale * 3), 5, 10)
    supplier_count = _clamp(round(2 + scale * 2), 2, 5)
    routed_customer_supplier_count = _clamp(round(3 + scale * 2), 3, 7)
    routed_router_customer_count = _clamp(round(1 + scale), 1, 3)

    merchant_amounts = _series(
        rng,
        count=merchant_count,
        target_msat=_clamp(int(turnover_msat * 0.52), 550_000_000, 1_350_000_000),
        minimum_msat=max(35_000_000, expected_payment_msat // 4),
        maximum_msat=max(90_000_000, expected_payment_msat * 2),
    )
    supplier_amounts = _series(
        rng,
        count=supplier_count,
        target_msat=_clamp(int(turnover_msat * 0.30), 360_000_000, 720_000_000),
        minimum_msat=max(80_000_000, expected_payment_msat // 2),
        maximum_msat=max(180_000_000, expected_payment_msat * 2),
    )
    customer_supplier_amounts = _series(
        rng,
        count=routed_customer_supplier_count,
        target_msat=_clamp(int(turnover_msat * 0.34), 360_000_000, 820_000_000),
        minimum_msat=max(45_000_000, expected_payment_msat // 3),
        maximum_msat=max(150_000_000, expected_payment_msat * 2),
    )
    router_customer_amounts = _series(
        rng,
        count=routed_router_customer_count,
        target_msat=_clamp(int(turnover_msat * 0.12), 90_000_000, 320_000_000),
        minimum_msat=max(35_000_000, expected_payment_msat // 4),
        maximum_msat=max(110_000_000, expected_payment_msat),
    )

    liquidity_budget_msat = max(300_000_000, int(channel_capacity_msat * 0.43))
    customer_capped = _cap_groups_to_budget(
        [merchant_amounts, customer_supplier_amounts],
        budget_msat=liquidity_budget_msat,
        minimum_msat=max(25_000_000, expected_payment_msat // 5),
    )
    merchant_router_capped = _cap_groups_to_budget(
        [supplier_amounts, customer_supplier_amounts],
        budget_msat=liquidity_budget_msat,
        minimum_msat=max(35_000_000, expected_payment_msat // 4),
    )
    router_capped = _cap_groups_to_budget(
        [router_customer_amounts],
        budget_msat=liquidity_budget_msat,
        minimum_msat=max(25_000_000, expected_payment_msat // 5),
    )

    failed_amount_msat = max(channel_capacity_msat + 900_000_000, expected_payment_msat * 18)

    plan = {
        "schema_version": 1,
        "seed": seed,
        "traffic_model": {
            "inspired_by": "bitcoin-dev-project/sim-ln",
            "mode": "seeded-defined-activity",
            "expected_payment_msat": expected_payment_msat,
            "capacity_multiplier": capacity_multiplier,
            "channel_capacity_sat": channel_capacity_sat,
            "turnover_target_msat": turnover_msat,
            "liquidity_budget_msat": liquidity_budget_msat,
            "liquidity_capped": bool(customer_capped or merchant_router_capped or router_capped),
        },
        "lightning": {
            "merchant_invoices": _invoice_rows(
                merchant_amounts,
                label_prefix="merchant-pos-sale",
                descriptions=[
                    "Kassiber Coffee breakfast POS batch",
                    "Kassiber Coffee lunch POS batch",
                    "Kassiber Coffee catering invoice",
                    "Kassiber Coffee webshop order",
                    "Kassiber Coffee afternoon POS batch",
                ],
            ),
            "supplier_invoices": _invoice_rows(
                supplier_amounts,
                label_prefix="supplier-operating-expense",
                descriptions=[
                    "Supplier beans restock",
                    "Supplier dairy delivery",
                    "Supplier equipment lease",
                    "Supplier packaging order",
                ],
            ),
            "routed_customer_supplier": _invoice_rows(
                customer_supplier_amounts,
                label_prefix="routed-customer-supplier",
                descriptions=[
                    "Customer routed supplier settlement",
                    "Customer routed wholesale settlement",
                    "Customer routed logistics settlement",
                ],
            ),
            "routed_router_customer": _invoice_rows(
                router_customer_amounts,
                label_prefix="routed-router-customer",
                descriptions=[
                    "Router routed customer refund",
                    "Router routed customer payout",
                ],
            ),
            "expired_invoices": [
                {
                    "label": "expired-quote-wholesale-001",
                    "amount_msat": _round_msat(max(99_000_000, expected_payment_msat // 2)),
                    "description": "Expired wholesale quote 001",
                    "expiry": 1,
                }
            ],
            "failed_payments": [
                {
                    "label": "failed-liquidity-large-restock-001",
                    "amount_msat": _round_msat(failed_amount_msat),
                    "description": "Intentionally oversized supplier restock 001",
                    "expiry": 3600,
                    "expected_failure": "insufficient_liquidity",
                }
            ],
        },
        "mainchain": {
            "actor_wallets": [
                "kassiber-ln-customer-l1",
                "kassiber-ln-treasury-l1",
                "kassiber-ln-supplier-l1",
            ],
            "topups": [
                {
                    "wallet": "kassiber-ln-customer-l1",
                    "label": "mainchain-customer-settlement-001",
                    "amount_sat": 275_000,
                    "description": "Customer batch settlement to merchant CLN wallet",
                    "confirmations": 1,
                },
                {
                    "wallet": "kassiber-ln-treasury-l1",
                    "label": "mainchain-treasury-float-001",
                    "amount_sat": 450_000,
                    "description": "Treasury float top-up to merchant CLN wallet",
                    "confirmations": 2,
                },
                {
                    "wallet": "kassiber-ln-customer-l1",
                    "label": "mainchain-weekend-pos-sweep-001",
                    "amount_sat": 125_000,
                    "description": "Weekend POS on-chain sweep to merchant CLN wallet",
                    "confirmations": 1,
                },
            ],
            "withdrawals": [
                {
                    "wallet": "kassiber-ln-supplier-l1",
                    "label": "mainchain-supplier-payout-001",
                    "amount_sat": 210_000,
                    "description": "Merchant on-chain supplier payout",
                    "confirmations": 1,
                },
                {
                    "wallet": "kassiber-ln-treasury-l1",
                    "label": "mainchain-treasury-sweep-001",
                    "amount_sat": 180_000,
                    "description": "Merchant on-chain treasury sweep",
                    "confirmations": 2,
                },
            ],
            "actor_funding_buffer_sat": 75_000,
            "actor_funding_confirmations": 1,
        },
    }
    hash_payload = json.dumps(plan, sort_keys=True, separators=(",", ":")).encode("utf-8")
    plan["traffic_model"]["plan_hash"] = hashlib.sha256(hash_payload).hexdigest()
    return plan


def _env_int(name: str, default: int) -> int:
    value = os.environ.get(name)
    return int(value) if value not in (None, "") else default


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--seed",
        default=os.environ.get("KASSIBER_REGTEST_LIGHTNING_SEED", DEFAULT_SEED),
    )
    parser.add_argument(
        "--expected-payment-msat",
        type=int,
        default=_env_int(
            "KASSIBER_REGTEST_LIGHTNING_EXPECTED_PAYMENT_MSAT",
            DEFAULT_EXPECTED_PAYMENT_MSAT,
        ),
    )
    parser.add_argument(
        "--channel-capacity-sat",
        type=int,
        default=_env_int(
            "KASSIBER_REGTEST_LIGHTNING_CHANNEL_CAPACITY_SAT",
            DEFAULT_CHANNEL_CAPACITY_SAT,
        ),
    )
    parser.add_argument(
        "--capacity-multiplier",
        type=float,
        default=float(
            os.environ.get(
                "KASSIBER_REGTEST_LIGHTNING_CAPACITY_MULTIPLIER",
                DEFAULT_CAPACITY_MULTIPLIER,
            )
        ),
    )
    parser.add_argument(
        "--output",
        default=os.environ.get("KASSIBER_LIGHTNING_BUSINESS_PLAN"),
        help="Write the plan to this path. Omit to print to stdout.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    plan = build_plan(
        seed=args.seed,
        expected_payment_msat=args.expected_payment_msat,
        channel_capacity_sat=args.channel_capacity_sat,
        capacity_multiplier=args.capacity_multiplier,
    )
    text = json.dumps(plan, indent=2, sort_keys=True) + "\n"
    if args.output:
        path = Path(args.output)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
