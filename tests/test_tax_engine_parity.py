"""Parity of the native tax engine with RP2 (plan 20).

Engine level: seeded random histories are built exactly as the adapter
builds its RP2 inputs (its configuration, constructor arguments, decimal
conversion and accounting engine) and run through RP2 and through the native
backend. Every value the adapter and ``explanation.py`` read is compared as a
string, so a decimal must match in sign, coefficient and exponent. Failures
must match in phase, class and message.

Adapter level: ``GenericRP2TaxEngine.build_ledger_state`` runs on finalized
custody projections under both backends and the results are compared
semantically (``tests/tax_engine_compare.py``).

The moving averages, Austrian classification and the Austrian multi-asset
hooks (``validate``, ``compute_multi``) are written against the engine
contract; their native runs fail with ``Unsupported`` until the engine
implements them.
"""

from __future__ import annotations

import json
import random
import unittest
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from typing import Any, Optional

import pytest

from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.core.engines import rp2 as adapter
from kassiber.msat import msat_to_btc
from tests import tax_engine_compare as compare
from tests.custody_tax_helpers import finalized_tax_inputs

SEED = 20261002
HOLDER = "Profile"
LOT_METHODS = ("fifo", "lifo", "hifo", "lofo")
EARN_TYPES = ("INCOME", "MINING", "STAKING", "INTEREST", "AIRDROP", "HARDFORK")
# 2021-03-01 00:00 in Europe/Vienna, the Austrian Alt/Neu cutoff.
AT_CUTOFF = datetime(2021, 2, 28, 23, 0, tzinfo=timezone.utc)
OFFSETS = (timedelta(hours=1), timedelta(hours=-5), timedelta(hours=5, minutes=30))
MAX_REPORTED = 3


# ---------------------------------------------------------------------------
# Engine level: histories


@dataclass
class History:
    name: str
    profile: dict[str, Any]
    wallets: list[str]
    rows: list[dict[str, Any]]


def _profile(country: str, method: str, long_term_days: int = 365) -> dict[str, Any]:
    return {
        "id": "profile",
        "workspace_id": "workspace",
        "label": HOLDER,
        "tax_country": country,
        "fiat_currency": "EUR",
        "gains_algorithm": method,
        "tax_long_term_days": long_term_days,
    }


def _btc(msat: int) -> Decimal:
    return msat_to_btc(msat)


def _price(rng: random.Random) -> Decimal:
    digits = rng.choice([0, 0, 2, 2, 4, 8])
    whole = rng.choice([1, 50, 999, 16000, 43210, 98765])
    if not digits:
        return Decimal(whole)
    return Decimal(f"{whole}.{rng.randrange(10**digits):0{digits}d}")


def _fiat(rng: random.Random, amount: Decimal, spot: Decimal) -> Decimal:
    # The adapter's fallback is amount * spot in Kassiber's context; imported
    # values are usually whole cents.
    if rng.random() < 0.5:
        return amount * spot
    return Decimal(rng.randint(1, 10**7)) / Decimal(100)


def _stamp(rng: random.Random, instant: datetime, odd: bool) -> str:
    style = rng.choices(["z", "offset", "micro"], (88, 7, 5) if odd else (100, 0, 0))[0]
    if style == "offset":
        return instant.astimezone(timezone(rng.choice(OFFSETS))).isoformat()
    if style == "micro":
        instant += timedelta(microseconds=rng.choice([1, 250000, 999999]))
        return instant.strftime("%Y-%m-%dT%H:%M:%S.%fZ")
    return instant.strftime("%Y-%m-%dT%H:%M:%SZ")


def _instants(rng: random.Random, long_term_days: int, *, austrian: bool) -> list[datetime]:
    years = [2019, 2020, 2021, 2022] if austrian else [2019, 2021, 2023]
    start = datetime(rng.choice(years), rng.randint(1, 12), rng.randint(1, 28), rng.choice([0, 12, 23]), tzinfo=timezone.utc)
    pool = [start + timedelta(days=rng.randint(0, 900), hours=rng.choice([0, 6, 23])) for _ in range(rng.randint(3, 9))]
    boundary = start + timedelta(days=long_term_days)
    pool += [start, boundary - timedelta(seconds=1), boundary, boundary + timedelta(seconds=1)]
    pool.append(datetime(start.year, 12, 31, 23, 59, 59, tzinfo=timezone.utc))
    if austrian:
        pool += [AT_CUTOFF - timedelta(seconds=1), AT_CUTOFF, start.replace(year=start.year + 1)]
    if rng.random() < 0.03:
        pool.append(datetime(1969, 12, 31, 12, tzinfo=timezone.utc))
    return sorted(set(pool))


def _notes(rng: random.Random, country: str, regime: str, kind: str) -> str:
    """Notes as ``_compose_event_notes`` writes them, with rare odd markers.

    ``regime`` is the one Kassiber assigns: the cutoff side for an inbound
    row, the inventory that covers an outbound one.
    """
    if country != "at":
        return rng.choice(["", "", "Exchange buy", "at_regime=neu Exchange buy"])
    tokens = []
    roll = rng.random()
    if roll < 0.9:
        tokens.append(f"at_regime={regime}")
    elif roll < 0.95:
        tokens.append("at_regime=neu" if regime == "alt" else "at_regime=alt")
    if rng.random() < 0.9:
        tokens.append("at_pool=default")
    if kind == "out" and rng.random() < 0.03:
        tokens.append(rng.choice(["at_swap_link=", "at_swap_link=lone"]))
    if rng.random() < 0.01:
        tokens.append("at_regime=alt at_regime=neu")
    if rng.random() < 0.3:
        tokens.append("Exchange note")
    return " ".join(tokens)


def _flow(
    rng: random.Random,
    asset: str,
    wallets: list[str],
    instants: list[datetime],
    *,
    country: str,
    gated: bool,
    odd: bool,
) -> list[dict[str, Any]]:
    """One asset's rows in adapter emission order: buys, earn receipts, sales
    with and without fees, fee-only and channel-open rows, and moves. Gated
    histories keep each wallet and the lot total non-negative, as the
    adapter's gate does; ungated ones reach RP2's own errors."""
    count = rng.randint(3, 12)
    raw = []
    for index in range(count):
        instant = rng.choice(instants[: max(2, len(instants) * (index + 1) // count)])
        kind = rng.choices(["in", "out", "intra"], [5, 4, 2])[0]
        raw.append((_stamp(rng, instant, odd), instant, kind, index))
    if gated or rng.random() < 0.5:
        rank = {"in": 0, "intra": 1, "out": 2}
        raw.sort(key=lambda item: (item[0], rank[item[2]], item[3]))
    else:
        rng.shuffle(raw)
    balance = {wallet: 0 for wallet in wallets}
    lots = 0
    # Inventory per Austrian regime, for tagging outbound rows.
    held = {"alt": 0, "neu": 0}
    rows: list[dict[str, Any]] = []
    for stamp, instant, kind, index in raw:
        wallet = rng.choice(wallets)
        spot = _price(rng)
        uid = f"{asset}-{kind}{index}"
        if kind == "in":
            regime = "alt" if instant < AT_CUTOFF else "neu"
        else:
            regime = "neu" if held["neu"] > 0 else "alt"
        notes = _notes(rng, country, regime, kind)
        if kind == "in":
            msat = rng.choice([rng.randint(1, 10**11), rng.randint(1, 10**8) * 1000, 10**11, 5 * 10**10])
            amount = _btc(msat)
            rows.append(
                {
                    "kind": "in", "asset": asset, "uid": uid, "timestamp": stamp, "wallet": wallet,
                    "type": rng.choice(["BUY"] * 5 + list(EARN_TYPES)),
                    "spot": spot, "amount": amount, "fiat": _fiat(rng, amount, spot), "notes": notes,
                }
            )
            balance[wallet] += msat
            lots += msat
            held[regime] += msat
        elif kind == "out":
            available = balance[wallet] if gated else rng.randint(1, 15 * 10**10)
            if available <= 0:
                continue
            shape = rng.choice(["sell", "sell", "sellfee", "feeonly", "channel"])
            if shape in ("feeonly", "channel"):
                fee = rng.randint(1, min(available, 10**8))
                needed = fee
                row = {"type": "FEE", "amount": Decimal("0"), "fee": _btc(fee), "fiat": None}
            else:
                total = rng.randint(1, available)
                fee = rng.randint(1, min(total - 1, 10**7)) if shape == "sellfee" and total > 1 else 0
                needed = total
                amount = _btc(total - fee)
                row = {"type": "SELL", "amount": amount, "fee": _btc(fee), "fiat": _fiat(rng, amount, spot)}
            if gated and (balance[wallet] < needed or lots < needed):
                continue
            rows.append({"kind": "out", "asset": asset, "uid": uid, "timestamp": stamp, "wallet": wallet, "spot": spot, "notes": notes, **row})
            balance[wallet] -= needed
            lots -= needed
            held[regime] = max(0, held[regime] - needed)
        else:
            others = [item for item in wallets if item != wallet] or [wallet]
            target = rng.choice(others)
            available = balance[wallet] if gated else rng.randint(1, 10**11)
            if available <= 0:
                continue
            sent = rng.randint(1, available)
            fee = rng.randint(0, min(sent, 10**7)) if rng.random() < 0.7 else 0
            move_spot: Optional[Decimal] = None if fee == 0 and rng.random() < 0.5 else spot
            rows.append(
                {
                    "kind": "intra", "asset": asset, "uid": uid, "timestamp": stamp, "src": wallet, "dst": target,
                    "spot": move_spot, "sent": _btc(sent), "received": _btc(sent - fee), "notes": notes,
                }
            )
            balance[wallet] -= sent
            balance[target] += sent - fee
            lots -= fee
            held[regime] = max(0, held[regime] - fee)
    if not any(row["kind"] == "in" for row in rows):
        rows.insert(
            0,
            {
                "kind": "in", "asset": asset, "uid": f"{asset}-seed", "timestamp": "2018-01-01T00:00:00Z",
                "wallet": wallets[0], "type": "BUY", "spot": Decimal("100"), "amount": Decimal("1"),
                "fiat": Decimal("100"), "notes": "at_regime=alt at_pool=default" if country == "at" else "",
            },
        )
    return rows


def random_history(index: int, *, country: str, method: str) -> History:
    rng = random.Random(f"{SEED}:{country}:{method}:{index}")
    long_term_days = rng.choice([1, 30, 365, 365, 366, 730])
    wallets = rng.choice([["A"], ["A", "B"], ["A", "B", "C"], ["A", "A-b", "A_x", "a"]])
    instants = _instants(rng, long_term_days, austrian=country == "at")
    gated = rng.random() < 0.85
    odd = rng.random() < 0.3
    assets = ["BTC"] if rng.random() < 0.8 else ["BTC", "LBTC"]
    rows = [
        row
        for asset in assets
        for row in _flow(rng, asset, wallets, instants, country=country, gated=gated, odd=odd)
    ]
    return History(f"{country}-{method}-{index:03d}", _profile(country, method, long_term_days), wallets, rows)


def swap_history(index: int, *, method: str) -> History:
    """An Austrian book with reviewed BTC -> LBTC swap legs, as the adapter
    marks them, plus the malformed pairings the validator rejects."""
    rng = random.Random(f"{SEED}:swap:{method}:{index}")
    wallets = ["Liquid", "Onchain"]
    alt_day = datetime(2020, rng.randint(1, 12), rng.randint(1, 28), tzinfo=timezone.utc)
    neu_day = datetime(2022, rng.randint(1, 12), rng.randint(1, 28), tzinfo=timezone.utc)
    rows: dict[str, list[dict[str, Any]]] = {"BTC": [], "LBTC": []}

    def buy(asset: str, wallet: str, instant: datetime, msat: int, regime: str, extra: str = "") -> None:
        spot = _price(rng)
        amount = _btc(msat)
        rows[asset].append(
            {
                "kind": "in", "asset": asset, "uid": f"{asset}-buy{len(rows[asset])}", "timestamp": _stamp(rng, instant, False),
                "wallet": wallet, "type": "BUY", "spot": spot, "amount": amount, "fiat": _fiat(rng, amount, spot),
                "notes": " ".join(token for token in (f"at_regime={regime}", "at_pool=default", extra) if token),
            }
        )

    def sell(asset: str, wallet: str, instant: datetime, msat: int, regime: str, extra: str = "", fee: int = 0) -> None:
        spot = _price(rng)
        amount = _btc(msat)
        rows[asset].append(
            {
                "kind": "out", "asset": asset, "uid": f"{asset}-sell{len(rows[asset])}", "timestamp": _stamp(rng, instant, False),
                "wallet": wallet, "type": "SELL", "spot": spot, "amount": amount, "fee": _btc(fee),
                "fiat": _fiat(rng, amount, spot),
                "notes": " ".join(token for token in (f"at_regime={regime}", "at_pool=default", extra) if token),
            }
        )

    buy("BTC", "Onchain", alt_day, rng.randint(1, 10) * 10**10, "alt")
    buy("BTC", "Onchain", neu_day, rng.randint(3, 10) * 10**10, "neu")
    if rng.random() < 0.5:
        buy("LBTC", "Liquid", neu_day - timedelta(days=3), rng.randint(1, 5) * 10**9, "neu")
    variant = rng.choices(["ok", "unpaired", "same_asset", "backwards", "empty", "alt_legs"], [70, 6, 6, 6, 6, 6])[0]
    when = neu_day + timedelta(days=rng.randint(1, 200))
    for swap in range(rng.randint(1, 3)):
        when += timedelta(days=rng.randint(0, 30))
        link = f"swap-{swap}" if variant != "empty" or swap else ""
        regime = "alt" if variant == "alt_legs" else "neu"
        msat = rng.randint(1, 5) * 10**9
        sell("BTC", "Onchain", when, msat, regime, f"at_swap_link={link}", fee=rng.choice([0, 0, 10**6]))
        if variant == "unpaired" and swap == 0:
            continue
        target = "BTC" if variant == "same_asset" and swap == 0 else "LBTC"
        wallet = "Onchain" if target == "BTC" else "Liquid"
        arrival = when - timedelta(hours=1) if variant == "backwards" and swap == 0 else when + timedelta(minutes=rng.choice([0, 0, 10]))
        buy(target, wallet, arrival, msat - rng.choice([0, 10**6]), regime, f"at_swap_link={link}")
    later = when + timedelta(days=rng.randint(1, 400))
    sell("LBTC", "Liquid", later, rng.randint(1, 3) * 10**8, "neu")
    sell("BTC", "Onchain", later + timedelta(days=1), rng.randint(1, 3) * 10**8, "neu")
    ordered = [row for asset in ("BTC", "LBTC") for row in sorted(rows[asset], key=lambda row: (row["timestamp"], row["kind"] != "in"))]
    return History(f"swap-{method}-{index:03d}-{variant}", _profile("at", method), wallets, ordered)


# ---------------------------------------------------------------------------
# Engine level: running one backend


def _construct(modules: dict[str, Any], configuration: Any, row: dict[str, Any], number: int) -> Any:
    """One transaction, with the arguments ``_prepare_rp2_asset_input`` passes."""
    decimal = adapter._rp2_decimal
    if row["kind"] == "in":
        return modules["InTransaction"](
            configuration=configuration,
            timestamp=row["timestamp"],
            asset=row["asset"],
            exchange=row["wallet"],
            holder=HOLDER,
            transaction_type=row["type"],
            spot_price=decimal(row["spot"]),
            crypto_in=decimal(row["amount"]),
            fiat_in_no_fee=decimal(row["fiat"]),
            fiat_in_with_fee=decimal(row["fiat"]),
            fiat_fee=decimal(0),
            row=number,
            unique_id=row["uid"],
            notes=row["notes"],
        )
    if row["kind"] == "out":
        sale = row["type"] == "SELL"
        return modules["OutTransaction"](
            configuration=configuration,
            timestamp=row["timestamp"],
            asset=row["asset"],
            exchange=row["wallet"],
            holder=HOLDER,
            transaction_type=row["type"],
            spot_price=decimal(row["spot"]),
            crypto_out_no_fee=decimal(row["amount"]),
            crypto_fee=decimal(row["fee"]),
            fiat_out_no_fee=decimal(row["fiat"]) if sale else None,
            fiat_fee=decimal(row["fee"] * row["spot"]),
            row=number,
            unique_id=row["uid"],
            notes=row["notes"],
        )
    return modules["IntraTransaction"](
        configuration=configuration,
        timestamp=row["timestamp"],
        asset=row["asset"],
        from_exchange=row["src"],
        from_holder=HOLDER,
        to_exchange=row["dst"],
        to_holder=HOLDER,
        spot_price=decimal(row["spot"] if row["spot"] is not None else 0),
        crypto_sent=decimal(row["sent"]),
        crypto_received=decimal(row["received"]),
        row=number,
        unique_id=row["uid"],
        notes=row["notes"],
    )


def _text(value: Any) -> Optional[str]:
    return None if value is None else str(value)


def _transaction_view(transaction: Any) -> Optional[dict[str, Any]]:
    if transaction is None:
        return None
    transaction_type = transaction.transaction_type
    view = {
        "class": type(transaction).__name__,
        "internal_id": transaction.internal_id,
        "unique_id": transaction.unique_id,
        "asset": transaction.asset,
        "transaction_type": transaction_type.value,
        "transaction_type_name": transaction_type.name,
        "is_earn_type": transaction_type.is_earn_type(),
        "wallet": getattr(transaction, "exchange", None) or getattr(transaction, "from_exchange", None),
        "notes": transaction.notes,
        "timestamp": transaction.timestamp.isoformat(),
        "spot_price": _text(transaction.spot_price),
        "fiat_fee": _text(transaction.fiat_fee),
        "crypto_fee": _text(transaction.crypto_fee),
    }
    if type(transaction).__name__ == "InTransaction":
        view["crypto_in"] = _text(transaction.crypto_in)
        view["fiat_in_with_fee"] = _text(transaction.fiat_in_with_fee)
    return view


def _failure(phase: str, error: BaseException) -> dict[str, Any]:
    return {"phase": phase, "error": list(compare.error_signature(error))}


def _classification(gain_loss: Any) -> Any:
    try:
        return list(adapter._classify_at_disposal(gain_loss))
    except Exception as exc:  # noqa: BLE001 - classification errors are compared
        return _failure("classify", exc)


def _computed_view(computed: Any, *, austrian: bool) -> dict[str, Any]:
    """Every ``ComputedData`` value the adapter and explanation.py read."""
    lots = list(computed.in_transaction_set)
    gain_losses = []
    for gain_loss in computed.gain_loss_set:
        view = {
            "event": _transaction_view(gain_loss.taxable_event),
            "lot": _transaction_view(gain_loss.acquired_lot),
            "crypto_amount": _text(gain_loss.crypto_amount),
            "fiat_cost_basis": _text(gain_loss.fiat_cost_basis),
            "proceeds": _text(gain_loss.taxable_event_fiat_amount_with_fee_fraction),
            "fiat_gain": _text(gain_loss.fiat_gain),
            "unit_cost_basis_override": _text(gain_loss.unit_cost_basis_override),
            "long_term": gain_loss.is_long_term_capital_gains(),
        }
        if gain_loss.acquired_lot is not None:
            view["lot_effective_basis"] = _text(computed.get_in_transaction_fiat_in_with_fee(gain_loss.acquired_lot))
        if austrian:
            view["at_category"] = _classification(gain_loss)
        gain_losses.append(view)
    return {
        "asset": computed.asset,
        "in_transactions": [
            {**_transaction_view(lot), "effective_basis": _text(computed.get_in_transaction_fiat_in_with_fee(lot))}
            for lot in lots
        ],
        "gain_losses": gain_losses,
        "yearly": [
            {
                "year": yearly.year,
                "asset": yearly.asset,
                "transaction_type": yearly.transaction_type.value,
                "long_term": yearly.is_long_term_capital_gains,
                "crypto_amount": _text(yearly.crypto_amount),
                "fiat_amount": _text(yearly.fiat_amount),
                "fiat_cost_basis": _text(yearly.fiat_cost_basis),
                "fiat_gain_loss": _text(yearly.fiat_gain_loss),
            }
            for yearly in computed.yearly_gain_loss_list
        ],
        "open_positions": [
            {
                "internal_id": lot.internal_id,
                "crypto_in": _text(lot.crypto_in),
                "sold_percentage": _text(computed.get_open_position_in_lot_sold_percentage(lot)),
                "fiat_in_with_fee": _text(computed.get_open_position_in_transaction_fiat_in_with_fee(lot)),
            }
            for lot in computed.open_position_in_transaction_set
        ],
        "balances": [
            {
                "exchange": balance.exchange,
                "holder": balance.holder,
                "final_balance": _text(balance.final_balance),
                "acquired_balance": _text(balance.acquired_balance),
                "sent_balance": _text(balance.sent_balance),
                "received_balance": _text(balance.received_balance),
            }
            for balance in computed.balance_set
        ],
    }


def run_history(backend: str, history: History, *, hooks: bool = False) -> dict[str, Any]:
    """Run ``history`` on ``backend`` the way the adapter does.

    ``hooks`` adds the Austrian country hooks the adapter calls before the
    per-asset computation: ``validate_input_data`` over every asset, then
    ``compute_tax_for_assets``, falling back per asset when it returns None.
    """
    profile = history.profile
    austrian = profile["tax_country"] == "at"
    assets = sorted({row["asset"] for row in history.rows})
    with adapter._use_tax_engine_backend(backend):
        modules = adapter._get_rp2_modules()
        with adapter._rp2_configuration(profile, history.wallets, assets) as configuration:
            sets = {
                asset: {kind: modules["TransactionSet"](configuration, kind.upper(), asset) for kind in ("in", "out", "intra")}
                for asset in assets
            }
            numbers = dict.fromkeys(assets, 0)
            for row in history.rows:
                numbers[row["asset"]] += 1
                try:
                    transaction = _construct(modules, configuration, row, numbers[row["asset"]])
                    sets[row["asset"]][row["kind"]].add_entry(transaction)
                except Exception as exc:  # noqa: BLE001 - construction errors are compared
                    return _failure("construct", exc)
            inputs = {
                asset: modules["InputData"](
                    asset=asset,
                    unfiltered_in_transaction_set=by_kind["in"],
                    unfiltered_out_transaction_set=by_kind["out"],
                    unfiltered_intra_transaction_set=by_kind["intra"],
                )
                for asset, by_kind in sets.items()
                if by_kind["in"].count
            }
            outcome: dict[str, Any] = {}
            computed_by_asset = None
            if hooks:
                country = configuration.country
                try:
                    country.validate_input_data(list(inputs.values()))
                except Exception as exc:  # noqa: BLE001
                    return _failure("validate", exc)
                try:
                    computed_by_asset = country.compute_tax_for_assets(
                        configuration, adapter._build_rp2_accounting_engine(profile), inputs
                    )
                except Exception as exc:  # noqa: BLE001
                    return _failure("compute_multi", exc)
                outcome["handled"] = computed_by_asset is not None
            for asset, input_data in inputs.items():
                if computed_by_asset is not None:
                    computed = computed_by_asset.get(asset)
                    outcome[asset] = None if computed is None else _computed_view(computed, austrian=austrian)
                    continue
                try:
                    computed = modules["compute_tax"](
                        configuration, adapter._build_rp2_accounting_engine(profile), input_data
                    )
                except Exception as exc:  # noqa: BLE001
                    outcome[asset] = _failure("compute", exc)
                    continue
                outcome[asset] = _computed_view(computed, austrian=austrian)
            return outcome


def _parity_failures(histories: list[History], *, hooks: bool = False) -> list[str]:
    failures = []
    for history in histories:
        expected = run_history("rp2", history, hooks=hooks)
        actual = run_history("native", history, hooks=hooks)
        differences = compare.value_differences(expected, actual, path=history.name)
        if differences:
            failures.append("\n    ".join([f"{history.name}:", *differences[:6]]))
    return failures


def _report(failures: list[str], total: int) -> str:
    shown = "\n  ".join(failures[:MAX_REPORTED])
    return f"{len(failures)} of {total} histories differ from RP2:\n  {shown}"


@pytest.mark.parametrize("method", LOT_METHODS)
def test_generic_lot_methods_match_rp2(method: str) -> None:
    histories = [random_history(index, country="generic", method=method) for index in range(150)]
    failures = _parity_failures(histories)
    assert not failures, _report(failures, len(histories))


@pytest.mark.parametrize("method", LOT_METHODS)
def test_austrian_lot_methods_match_rp2(method: str) -> None:
    # Austrian books on a lot method: the per-asset computation plus the
    # classification the adapter requests for every gain/loss.
    histories = [random_history(index, country="at", method=method) for index in range(60)]
    failures = _parity_failures(histories)
    assert not failures, _report(failures, len(histories))


@pytest.mark.parametrize(
    ("country", "method"),
    [("generic", "moving_average"), ("at", "moving_average"), ("at", "moving_average_at")],
)
def test_moving_averages_match_rp2(country: str, method: str) -> None:
    histories = [random_history(index, country=country, method=method) for index in range(30)]
    failures = _parity_failures(histories)
    assert not failures, _report(failures, len(histories))


@pytest.mark.parametrize("method", ["moving_average_at", "fifo", "moving_average"])
def test_austrian_country_hooks_match_rp2(method: str) -> None:
    # Swap pairs drive validate_input_data and the multi-asset runner; books
    # without pairs check the runner's "not handled" answer.
    histories = [swap_history(index, method=method) for index in range(24)]
    histories += [random_history(index, country="at", method=method) for index in range(6)]
    failures = _parity_failures(histories, hooks=True)
    assert not failures, _report(failures, len(histories))


def test_comparison_sees_representation_type_and_order() -> None:
    # The checks above are only as strict as the comparison.
    assert compare.value_differences({"a": Decimal("300")}, {"a": Decimal("3.0E+2")})
    assert compare.value_differences({"a": Decimal("0")}, {"a": Decimal("-0")})
    assert compare.value_differences({"a": Decimal("1")}, {"a": "1"})
    assert compare.value_differences({"a": 1, "b": 2}, {"b": 2, "a": 1})
    assert compare.value_differences([1, 2], [2, 1])
    assert compare.value_differences([0.0], [-0.0])
    assert not compare.value_differences({"a": [Decimal("1.50"), float("nan")]}, {"a": [Decimal("1.50"), float("nan")]})


# ---------------------------------------------------------------------------
# Adapter level

WALLET_REFS = {
    wallet_id: {
        "id": wallet_id,
        "label": label,
        "wallet_account_id": "acct-1",
        "account_code": "treasury",
        "account_label": "Treasury",
    }
    for wallet_id, label in (("A", "Cold"), ("B", "Hot"), ("C", "Exchange"), ("L", "Liquid"))
}


def _adapter_profile(country: str, method: str) -> dict[str, Any]:
    return {
        "id": "profile-1",
        "workspace_id": "ws-1",
        "label": "Default",
        "fiat_currency": "EUR",
        "tax_country": country,
        "tax_long_term_days": 365,
        "gains_algorithm": method,
    }


def _row(
    tx_id: str,
    wallet: str,
    direction: str,
    amount: int,
    occurred_at: str,
    *,
    fee: int = 0,
    rate: Optional[str] = "40000",
    fiat_value: Optional[str] = None,
    kind: Optional[str] = None,
    asset: str = "BTC",
) -> dict[str, Any]:
    """An imported exchange row (no chain proof), as the stores hold it."""
    ref = WALLET_REFS[wallet]
    liquid = asset in {"LBTC", "L-BTC"}
    return {
        "id": tx_id,
        "workspace_id": "ws-1",
        "profile_id": "profile-1",
        "wallet_id": wallet,
        "wallet_label": ref["label"],
        "wallet_account_id": ref["wallet_account_id"],
        "account_code": ref["account_code"],
        "account_label": ref["account_label"],
        "external_id": f"exchange-{tx_id}",
        "occurred_at": occurred_at,
        "created_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "fiat_currency": "EUR",
        "fiat_rate": None if rate is None else float(rate),
        "fiat_rate_exact": rate,
        "fiat_value": None if fiat_value is None else float(fiat_value),
        "fiat_value_exact": fiat_value,
        "kind": kind or ("deposit" if direction == "inbound" else "withdrawal"),
        "description": f"{tx_id} via {ref['label']}",
        "note": None,
        "raw_json": "{}",
        "config_json": json.dumps({"chain": "liquid" if liquid else "bitcoin", "network": "liquidv1" if liquid else "main"}),
        "excluded": 0,
    }


def _ledger_history() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    rows = [
        _row("buy-1", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        _row("buy-2", "C", "inbound", 50_000_000_000, "2024-03-01T00:00:00Z", rate="50000.5", kind="buy"),
        _row("mined", "A", "inbound", 1_234_567_890, "2024-04-01T00:00:00Z", rate="60000", kind="mining"),
        _row("staked", "A", "inbound", 987_654_321, "2024-04-02T00:00:00Z", rate="60100.25", kind="staking"),
        _row("sell-1", "C", "outbound", 30_000_000_000, "2024-06-01T00:00:00Z", fee=10_000_000, rate="65000", kind="sell"),
        _row("move-out", "C", "outbound", 20_000_000_000, "2024-07-01T00:00:00Z", fee=1_000_000, rate="61000"),
        _row("move-in", "B", "inbound", 20_000_000_000, "2024-07-01T00:10:00Z", rate="61000"),
        _row("sell-2", "B", "outbound", 5_000_000_000, "2025-08-01T00:00:00Z", rate="90000.123", kind="sell"),
        _row("sell-3", "C", "outbound", 70_000_000_000, "2025-09-01T00:00:00Z", fee=2_000_000, fiat_value="61234.56", rate=None, kind="sell"),
    ]
    pairs = [
        {"id": "pair-1", "out_transaction_id": "move-out", "in_transaction_id": "move-in", "kind": "manual", "policy": "carrying-value"}
    ]
    return rows, pairs


def _rail_history() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    rows = [
        _row("rail-buy-1", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        _row("rail-buy-2", "C", "inbound", 40_000_000_000, "2024-02-01T00:00:00Z", rate="45000.25", kind="buy"),
        _row("peg-out", "C", "outbound", 60_000_000_000, "2024-05-01T00:00:00Z", rate="60000"),
        _row("peg-in", "L", "inbound", 59_900_000_000, "2024-05-01T01:00:00Z", rate="60000", asset="LBTC"),
        _row("liquid-sell", "L", "outbound", 10_000_000_000, "2024-09-01T00:00:00Z", rate="70000", asset="LBTC", kind="sell"),
    ]
    pairs = [
        {"id": "rail-1", "out_transaction_id": "peg-out", "in_transaction_id": "peg-in", "kind": "peg-in", "policy": "carrying-value"}
    ]
    return rows, pairs


def _random_projection(index: int) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Imported buys, sales, income kinds, fees, missing prices, and reviewed
    same-asset moves between wallets, in Kassiber's canonical timestamps."""
    rng = random.Random(f"{SEED}:projection:{index}")
    start = datetime(rng.choice([2019, 2021, 2023]), rng.randint(1, 12), rng.randint(1, 28), tzinfo=timezone.utc)
    rows: list[dict[str, Any]] = []
    pairs: list[dict[str, Any]] = []
    held = {"A": 0, "B": 0, "C": 0}
    when = start
    for step in range(rng.randint(4, 14)):
        when += timedelta(days=rng.choice([0, 1, 7, 90, 200]), hours=rng.choice([0, 0, 5]))
        stamp = when.strftime("%Y-%m-%dT%H:%M:%SZ")
        rate = str(_price(rng)) if rng.random() < 0.95 else None
        roll = rng.random()
        if roll < 0.45 or not any(held.values()):
            wallet = rng.choice(["A", "C", "C"])
            kind = rng.choice(["buy", "buy", "deposit", "mining", "staking", "income", "interest"])
            amount = rng.choice([rng.randint(1, 10**11), rng.randint(1, 10**6) * 10**5])
            rows.append(_row(f"in-{step}", wallet, "inbound", amount, stamp, rate=rate, kind=kind))
            held[wallet] += amount
        elif roll < 0.8:
            wallet = rng.choice([name for name, value in held.items() if value > 0])
            amount = rng.randint(1, held[wallet])
            fee = rng.randint(0, min(amount - 1, 10**7)) if amount > 1 and rng.random() < 0.5 else 0
            rows.append(_row(f"out-{step}", wallet, "outbound", amount - fee, stamp, fee=fee, rate=rate, kind="sell"))
            held[wallet] -= amount
        else:
            source = rng.choice([name for name, value in held.items() if value > 0])
            target = rng.choice([name for name in held if name != source])
            amount = rng.randint(1, held[source])
            fee = rng.randint(0, min(amount - 1, 10**7)) if amount > 1 else 0
            arrival = (when + timedelta(minutes=rng.choice([0, 10]))).strftime("%Y-%m-%dT%H:%M:%SZ")
            rows.append(_row(f"move-out-{step}", source, "outbound", amount - fee, stamp, fee=fee, rate=rate))
            rows.append(_row(f"move-in-{step}", target, "inbound", amount - fee, arrival, rate=rate))
            pairs.append(
                {
                    "id": f"pair-{step}",
                    "out_transaction_id": f"move-out-{step}",
                    "in_transaction_id": f"move-in-{step}",
                    "kind": "manual",
                    "policy": "carrying-value",
                }
            )
            held[source] -= amount
            held[target] += amount - fee
    return rows, pairs


class AdapterParityTest(unittest.TestCase):
    """``build_ledger_state`` gives the same journal on both backends."""

    def _assert_parity(self, profile, rows, pairs):
        inputs = finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
        expected, actual = compare.run_both(GenericRP2TaxEngine(profile), inputs)
        self.assertEqual([], compare.ledger_differences(expected, actual))
        return expected

    def test_generic_ledger_matches_on_each_lot_method(self):
        rows, pairs = _ledger_history()
        for method in ("FIFO", "LIFO", "HIFO", "LOFO"):
            with self.subTest(method=method):
                result, error = self._assert_parity(_adapter_profile("generic", method), rows, pairs)
                self.assertIsNone(error)
                entry_types = {entry["entry_type"] for entry in result.entries}
                # The history reaches every journal shape the engine feeds.
                self.assertTrue(
                    {"acquisition", "disposal", "income", "transfer_fee", "transfer_out", "transfer_in"} <= entry_types,
                    entry_types,
                )
                self.assertTrue(result.tax_summary)
                self.assertTrue(result.wallet_holdings)

    def test_generic_bitcoin_rail_carry_matches(self):
        rows, pairs = _rail_history()
        for method in ("FIFO", "HIFO"):
            with self.subTest(method=method):
                result, error = self._assert_parity(_adapter_profile("generic", method), rows, pairs)
                self.assertIsNone(error)
                self.assertIn("disposal", {entry["entry_type"] for entry in result.entries})

    def test_random_generic_projections_match(self):
        for index in range(16):
            rows, pairs = _random_projection(index)
            method = ("FIFO", "LIFO", "HIFO", "LOFO")[index % 4]
            with self.subTest(index=index, method=method):
                self._assert_parity(_adapter_profile("generic", method), rows, pairs)

    def test_austrian_ledger_matches_on_lot_methods(self):
        rows, pairs = _ledger_history()
        for method in ("fifo", "hifo"):
            with self.subTest(method=method):
                self._assert_parity(_adapter_profile("at", method), rows, pairs)

    def test_austrian_ledger_matches_on_moving_average_at(self):
        for rows, pairs in (_ledger_history(), _rail_history()):
            with self.subTest(first=rows[0]["id"]):
                self._assert_parity(_adapter_profile("at", "moving_average_at"), rows, pairs)

    def test_random_austrian_projections_match(self):
        for index in range(8):
            rows, pairs = _random_projection(100 + index)
            method = ("moving_average_at", "fifo")[index % 2]
            with self.subTest(index=index, method=method):
                self._assert_parity(_adapter_profile("at", method), rows, pairs)
