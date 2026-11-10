#!/usr/bin/env python3
"""Reference calculator and RP2 runner for the tax-country research vectors.

Research tooling for docs/reference/tax-country-{es,ie,jp}.md; it is not
product code and nothing in Kassiber imports it. It reads one vectors file
(docs/reference/tax-country-<cc>-vectors.json) and either

  check   recomputes every vector with the reference rules written in the
          country doc and compares the result with the hand-computed
          ``expected`` block, or
  rp2     runs RP2's country plugin (the dev-time oracle in the venv) on the
          same inputs and prints, or with ``--write`` stores, its results
          under each vector's ``rp2.computed``.

RP2 writes ``./log/rp2_<ts>.log`` when imported, so ``rp2`` needs
``--scratch DIR`` (a directory under the worktree's ``target/``); the import
runs there and the log handler is removed afterwards.

  python scripts/tax-country-vectors.py check docs/reference/tax-country-es-vectors.json
  python scripts/tax-country-vectors.py rp2 docs/reference/tax-country-es-vectors.json \
      --scratch "$(realpath target)/rp2-scratch" --write
"""

from __future__ import annotations

import argparse
import decimal
import json
import logging
import os
import sys
import tempfile
from collections import defaultdict
from datetime import date, datetime, timezone
from decimal import ROUND_CEILING, ROUND_FLOOR, ROUND_HALF_UP, Decimal
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo

# The engine's context: 32 significant digits, round-half-even.
CTX = decimal.Context(prec=32, rounding=decimal.ROUND_HALF_EVEN)
decimal.setcontext(CTX)

ZERO = Decimal(0)
CENT = Decimal("0.01")


def D(value: Any) -> Decimal:
    if value is None:
        return ZERO
    return Decimal(str(value))


def cents(value: Decimal) -> Decimal:
    """Half-up to cents (presentation rounding, see the country docs)."""
    return value.quantize(CENT, rounding=ROUND_HALF_UP)


def money(value: Decimal) -> str:
    return str(cents(value))


def plain(value: Decimal) -> str:
    """Exact decimal text without exponent or trailing zeros."""
    if value == ZERO:
        return "0"
    text = format(value.normalize(), "f")
    return text


def parse_ts(text: str) -> datetime:
    parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError(f"timestamp without offset: {text}")
    return parsed


ACQUIRE, TRANSFER, DISPOSE = 0, 1, 2
PHASE = {
    "buy": ACQUIRE,
    "income": ACQUIRE,
    "swap": DISPOSE,
    "transfer": TRANSFER,
    "sell": DISPOSE,
    "spend": DISPOSE,
    "gift_out": DISPOSE,
    "lost": DISPOSE,
}


def ordered_events(vector: dict[str, Any]) -> list[dict[str, Any]]:
    """Per instant: acquisitions, then transfers, then disposals (the walk's order)."""
    indexed = list(enumerate(vector["events"]))
    indexed.sort(key=lambda item: (parse_ts(item[1]["ts"]), PHASE[item[1]["type"]], item[0]))
    return [event for _, event in indexed]


class Quarantine(Exception):
    pass


# --------------------------------------------------------------------------
# Lot pools (ES, IE): FIFO over one global pool per asset.
# --------------------------------------------------------------------------


class FifoPool:
    def __init__(self) -> None:
        self.lots: dict[str, list[list[Any]]] = defaultdict(list)  # asset -> [[event_id, qty, cost]]

    def add(self, asset: str, event_id: str, qty: Decimal, cost: Decimal) -> None:
        self.lots[asset].append([event_id, qty, cost])

    def take(self, asset: str, qty: Decimal) -> tuple[Decimal, list[dict[str, str]]]:
        remaining = qty
        cost = ZERO
        used: list[dict[str, str]] = []
        lots = self.lots[asset]
        while remaining > ZERO:
            if not lots:
                raise Quarantine("insufficient_lots")
            lot_id, lot_qty, lot_cost = lots[0]
            take = min(remaining, lot_qty)
            part = lot_cost if take == lot_qty else lot_cost * take / lot_qty
            cost += part
            used.append({"lot": lot_id, "qty": plain(take), "cost": money(part)})
            if take == lot_qty:
                lots.pop(0)
            else:
                lots[0][1] = lot_qty - take
                lots[0][2] = lot_cost - part
            remaining -= take
        return cost, used


# --------------------------------------------------------------------------
# Spain
# --------------------------------------------------------------------------

ES_INCOME = {
    "staking": ("0027", "ahorro"),
    "lending_interest": ("rcm_art25_2", "ahorro"),
    "interest": ("rcm_art25_2", "ahorro"),
    "airdrop": ("0304", "general"),
    "mining": ("0304", "general"),
}


def es_compute(vector: dict[str, Any]) -> dict[str, Any]:
    tz = ZoneInfo(vector["book"]["timezone"])
    pool = FifoPool()
    disposals: list[dict[str, Any]] = []
    income: list[dict[str, Any]] = []
    quarantines: list[dict[str, str]] = []
    flags: list[dict[str, str]] = []
    years: dict[int, dict[str, Decimal]] = defaultdict(lambda: defaultdict(Decimal))
    acquisitions_by_asset: dict[str, list[tuple[datetime, str]]] = defaultdict(list)

    for event in vector["events"]:
        if event["type"] == "buy":
            acquisitions_by_asset[event["asset"]].append((parse_ts(event["ts"]), event["id"]))

    def dispose(event, asset, units, vt, kind, contraprestacion, gift=False):
        year = parse_ts(event["ts"]).astimezone(tz).year
        va, lots = pool.take(asset, units)
        vt_c, va_c = cents(vt), cents(va)
        gain = vt_c - va_c
        computable = not (gift and gain < ZERO)
        disposals.append(
            {
                "event": event["id"],
                "asset": asset,
                "qty": plain(units),
                "kind": kind,
                "contraprestacion": contraprestacion,
                "valor_transmision": str(vt_c),
                "valor_adquisicion": str(va_c),
                "ganancia": str(gain),
                "computable": computable,
                "lots": lots,
            }
        )
        if computable:
            if gain >= ZERO:
                years[year]["g"] += gain
            else:
                years[year]["p"] += -gain
        else:
            years[year]["non_computable_losses"] += -gain
            flags.append({"event": event["id"], "code": "es_gift_loss_not_computable"})
        if gain < ZERO and computable and kind == "sale":
            disposal_ts = parse_ts(event["ts"])
            for acquired_ts, acquired_id in acquisitions_by_asset[asset]:
                delta = (acquired_ts - disposal_ts).total_seconds()
                if 0 < delta <= 366 * 86400:
                    flags.append({"event": event["id"], "code": "es_33_5_e_reacquisition_review"})
                    break
        return vt

    for event in ordered_events(vector):
        kind = event["type"]
        year = parse_ts(event["ts"]).astimezone(tz).year
        try:
            if kind == "buy":
                pool.add(event["asset"], event["id"], D(event["qty"]), D(event["fiat"]) + D(event.get("fiat_fee")))
            elif kind == "income":
                income_kind = event["income_kind"]
                if income_kind not in ES_INCOME:
                    raise Quarantine(f"es_{income_kind}_requires_review")
                casilla, base = ES_INCOME[income_kind]
                value = D(event["fiat"])
                income.append(
                    {"event": event["id"], "asset": event["asset"], "kind": income_kind, "casilla": casilla, "base": base, "value": money(value)}
                )
                years[year][f"income_{casilla}"] += cents(value)
                if casilla == "rcm_art25_2":
                    flags.append({"event": event["id"], "code": "es_lending_casilla_unconfirmed"})
                pool.add(event["asset"], event["id"], D(event["qty"]), value)
            elif kind == "sell":
                units = D(event["qty"]) + D(event.get("crypto_fee"))
                price = D(event["fiat"])
                market = event.get("market_value")
                if market is not None and price < D(market):
                    flags.append({"event": event["id"], "code": "es_price_below_market"})
                    price = D(market)
                dispose(event, event["asset"], units, price - D(event.get("fiat_fee")), "sale", "moneda_curso_legal")
            elif kind == "swap":
                vt = max(D(event["out_value"]), D(event["in_value"]))
                dispose(event, event["out_asset"], D(event["out_qty"]), vt, "swap", "otra_moneda_virtual")
                pool.add(event["in_asset"], event["id"], D(event["in_qty"]), vt)
            elif kind == "spend":
                vt = max(D(event["fiat"]), D(event.get("market_value")))
                dispose(event, event["asset"], D(event["qty"]), vt, "spend", "otro_bien_o_servicio")
            elif kind == "gift_out":
                dispose(event, event["asset"], D(event["qty"]), D(event["fiat"]), "gift", "sin_contraprestacion", gift=True)
            elif kind == "transfer":
                fee = D(event.get("fee"))
                if fee > ZERO:
                    dispose(event, event["asset"], fee, D(event["fee_value"]), "network_fee", "otro_bien_o_servicio")
            elif kind == "lost":
                raise Quarantine("es_lost_requires_review")
        except Quarantine as exc:
            quarantines.append({"event": event["id"], "reason": str(exc)})

    # Crypto-only ahorro integration (art. 49 LIRPF), oldest losses first,
    # four following years, 25 % cross-offset against capital-mobiliario yields.
    out_years: dict[str, Any] = {}
    carry: dict[int, Decimal] = {}
    first = min(years) if years else None
    last = max(years) if years else None
    for year in range(first, last + 1) if first is not None else []:
        row = years[year]
        gp = row["g"] - row["p"]
        rcm = row["income_0027"] + row["income_rcm_art25_2"]
        limit = cents(rcm * Decimal("0.25"))
        used_rcm = ZERO
        prior_applied = ZERO
        if gp < ZERO:
            offset = min(-gp, limit)
            used_rcm += offset
            gp += offset
            if gp < ZERO:
                carry_current = -gp
                gp = ZERO
            else:
                carry_current = ZERO
        else:
            carry_current = ZERO
        for origin in sorted(carry):
            if origin < year - 4:
                continue
            amount = carry[origin]
            use = min(amount, gp)
            gp -= use
            amount -= use
            prior_applied += use
            use_rcm = min(amount, limit - used_rcm)
            used_rcm += use_rcm
            amount -= use_rcm
            prior_applied += use_rcm
            carry[origin] = amount
        carry = {origin: amount for origin, amount in carry.items() if amount > ZERO and origin >= year - 3}
        if carry_current > ZERO:
            carry[year] = carry_current
        out_years[str(year)] = {
            "casilla_1804": {"ganancias": money(row["g"]), "perdidas": money(row["p"]), "saldo": money(row["g"] - row["p"])},
            "casilla_0027": money(row["income_0027"]),
            "rcm_art25_2": money(row["income_rcm_art25_2"]),
            "casilla_0304": money(row["income_0304"]),
            "non_computable_losses": money(row["non_computable_losses"]),
            "ahorro_crypto_only": {
                "rcm_offset_25pct": money(used_rcm),
                "prior_losses_applied": money(prior_applied),
                "base": money(gp + rcm - used_rcm),
                "carry_forward": {str(origin): money(amount) for origin, amount in sorted(carry.items())},
            },
        }
    return {"disposals": disposals, "income": income, "quarantines": quarantines, "flags": flags, "years": out_years}


# --------------------------------------------------------------------------
# Ireland
# --------------------------------------------------------------------------

IE_CASE_IV = {"staking", "lending_interest", "interest", "mining"}
IE_RATE_CHANGE = date(2026, 10, 7)
IE_EXEMPTION = Decimal("1270")


def ie_rate(local_day: date) -> Decimal:
    if local_day >= IE_RATE_CHANGE:
        return Decimal("0.31")
    if local_day >= date(2012, 12, 6):
        return Decimal("0.33")
    raise Quarantine("ie_rate_before_2012_12_06_not_modelled")


def ie_allocate(buckets: dict[tuple[str, Decimal], Decimal], deductions: Decimal) -> dict[tuple[str, Decimal], Decimal]:
    """Set deductions against gains, highest rate first, then initial before later."""
    order = sorted(buckets, key=lambda key: (-key[1], 0 if key[0] == "initial" else 1))
    net = dict(buckets)
    for key in order:
        use = min(net[key], deductions)
        net[key] -= use
        deductions -= use
    return net


def ie_compute(vector: dict[str, Any]) -> dict[str, Any]:
    tz = ZoneInfo(vector["book"]["timezone"])
    pool = FifoPool()
    disposals: list[dict[str, Any]] = []
    income: list[dict[str, Any]] = []
    quarantines: list[dict[str, str]] = []
    flags: list[dict[str, str]] = []
    rows_by_year: dict[int, list[dict[str, Any]]] = defaultdict(list)
    case_iv: dict[int, Decimal] = defaultdict(Decimal)
    buys: dict[str, list[tuple[datetime, str]]] = defaultdict(list)
    for event in vector["events"]:
        if event["type"] == "buy":
            buys[event["asset"]].append((parse_ts(event["ts"]), event["id"]))

    def dispose(event, asset, units, consideration, disposal_costs, kind):
        local = parse_ts(event["ts"]).astimezone(tz)
        cost, lots = pool.take(asset, units)
        allowable = cost + disposal_costs
        gain = cents(consideration) - cents(allowable)
        row = {
            "event": event["id"],
            "asset": asset,
            "qty": plain(units),
            "kind": kind,
            "date": local.date().isoformat(),
            "period": "initial" if (local.month, local.day) <= (11, 30) else "later",
            "rate": str(ie_rate(local.date())),
            "consideration": money(consideration),
            "allowable_cost": money(allowable),
            "gain": str(gain),
            "lots": lots,
        }
        disposals.append(row)
        rows_by_year[local.year].append(row)
        if row["rate"] == "0.31":
            flags.append({"event": event["id"], "code": "ie_rate_31_pending_finance_act"})
        if gain < ZERO and kind == "sale":
            disposal_ts = parse_ts(event["ts"])
            for acquired_ts, _ in buys[asset]:
                if abs((acquired_ts - disposal_ts).total_seconds()) <= 28 * 86400 and acquired_ts != disposal_ts:
                    flags.append({"event": event["id"], "code": "ie_s581_four_week_review"})
                    break

    for event in ordered_events(vector):
        kind = event["type"]
        local = parse_ts(event["ts"]).astimezone(tz)
        try:
            if kind == "buy":
                pool.add(event["asset"], event["id"], D(event["qty"]), D(event["fiat"]) + D(event.get("fiat_fee")))
            elif kind == "income":
                income_kind = event["income_kind"]
                if income_kind not in IE_CASE_IV:
                    raise Quarantine(f"ie_{income_kind}_requires_review")
                value = D(event["fiat"])
                income.append({"event": event["id"], "asset": event["asset"], "kind": income_kind, "schedule": "case_iv", "value": money(value)})
                case_iv[local.year] += cents(value)
                pool.add(event["asset"], event["id"], D(event["qty"]), value)
            elif kind == "sell":
                units = D(event["qty"]) + D(event.get("crypto_fee"))
                dispose(event, event["asset"], units, D(event["fiat"]), D(event.get("fiat_fee")), "sale")
            elif kind == "swap":
                consideration = D(event["in_value"]) if event.get("in_value") is not None else D(event["out_value"])
                dispose(event, event["out_asset"], D(event["out_qty"]), consideration, ZERO, "swap")
                pool.add(event["in_asset"], event["id"], D(event["in_qty"]), consideration)
            elif kind == "spend":
                consideration = D(event["fiat"]) if event.get("fiat") is not None else D(event["market_value"])
                dispose(event, event["asset"], D(event["qty"]), consideration, ZERO, "spend")
            elif kind == "gift_out":
                flags.append({"event": event["id"], "code": "ie_gift_connected_party"})
                dispose(event, event["asset"], D(event["qty"]), D(event["fiat"]), ZERO, "gift")
            elif kind == "transfer":
                fee = D(event.get("fee"))
                if fee > ZERO:
                    dispose(event, event["asset"], fee, D(event["fee_value"]), ZERO, "network_fee")
            elif kind == "lost":
                raise Quarantine("ie_lost_requires_negligible_value_claim")
        except Quarantine as exc:
            quarantines.append({"event": event["id"], "reason": str(exc)})

    out_years: dict[str, Any] = {}
    losses_forward = D(vector["book"].get("losses_brought_forward_opening"))
    years = sorted(set(rows_by_year) | set(case_iv))
    for year in range(years[0], years[-1] + 1) if years else []:
        rows = rows_by_year.get(year, [])
        gains = sum((D(r["gain"]) for r in rows if D(r["gain"]) > ZERO), ZERO)
        losses = sum((-D(r["gain"]) for r in rows if D(r["gain"]) < ZERO), ZERO)
        line14 = max(ZERO, gains - losses)
        line17 = min(losses_forward, line14)
        line18 = min(IE_EXEMPTION, line14 - line17)
        line19 = line14 - line17 - line18
        line21 = max(ZERO, losses - gains)
        line22 = losses_forward - line17
        buckets: dict[tuple[str, Decimal], Decimal] = defaultdict(Decimal)
        for r in rows:
            if D(r["gain"]) > ZERO:
                buckets[(r["period"], D(r["rate"]))] += D(r["gain"])
        annual = ie_allocate(buckets, losses + line17 + line18)
        tax_total = cents(sum((net * rate for (_, rate), net in annual.items()), ZERO))
        # Initial period: computed as if the year ended on 30 November.
        init_rows = [r for r in rows if r["period"] == "initial"]
        g_i = sum((D(r["gain"]) for r in init_rows if D(r["gain"]) > ZERO), ZERO)
        l_i = sum((-D(r["gain"]) for r in init_rows if D(r["gain"]) < ZERO), ZERO)
        n14 = max(ZERO, g_i - l_i)
        n17 = min(losses_forward, n14)
        n18 = min(IE_EXEMPTION, n14 - n17)
        init_buckets = {key: value for key, value in buckets.items() if key[0] == "initial"}
        initial = ie_allocate(init_buckets, l_i + n17 + n18)
        tax_initial = cents(sum((net * rate for (_, rate), net in initial.items()), ZERO))
        later = tax_total - tax_initial
        by_line: dict[str, str] = {}
        for (period, rate), net in sorted(annual.items(), key=lambda item: (item[0][0] != "initial", -item[0][1])):
            line = "24" if period == "initial" else "25"
            by_line[f"line_{line}_at_{int(rate * 100)}pct"] = money(net)
        out_years[str(year)] = {
            "line_1k_disposals": len(rows),
            "line_1k_consideration": money(sum((D(r["consideration"]) for r in rows), ZERO)),
            "line_10": money(gains),
            "line_11": money(losses),
            "line_14": money(line14),
            "line_17": money(line17),
            "line_18": money(line18),
            "line_19": money(line19),
            "line_21": money(line21),
            "line_22": money(line22),
            "line_23": money(line21 + line22),
            "rate_lines": by_line,
            "tax_total": money(tax_total),
            "payment_initial": money(tax_initial),
            "payment_later": money(max(ZERO, later)),
            "overpaid_initial": money(max(ZERO, -later)),
            "case_iv_income": money(case_iv.get(year, ZERO)),
        }
        losses_forward = line21 + line22
    return {"disposals": disposals, "income": income, "quarantines": quarantines, "flags": flags, "years": out_years}


# --------------------------------------------------------------------------
# Japan
# --------------------------------------------------------------------------

JP_INCOME = {"mining", "staking", "lending_interest", "interest"}
JP_LAST_SUPPORTED_YEAR = 2026


def jp_compute(vector: dict[str, Any]) -> dict[str, Any]:
    tz = ZoneInfo(vector["book"]["timezone"])
    method = vector["book"]["method"]
    quarantines: list[dict[str, str]] = []
    flags: list[dict[str, str]] = []
    refusals: list[dict[str, Any]] = []
    # Per year and asset: acquisitions and disposals in local-date order.
    acq: dict[tuple[int, str], list[tuple[Decimal, Decimal]]] = defaultdict(list)
    seq: dict[tuple[int, str], list[tuple[str, Any]]] = defaultdict(list)  # moving average sequence
    revenue: dict[tuple[int, str], Decimal] = defaultdict(Decimal)
    other_expenses: dict[tuple[int, str], Decimal] = defaultdict(Decimal)
    disposed: dict[tuple[int, str], Decimal] = defaultdict(Decimal)
    receipts: dict[int, dict[str, Decimal]] = defaultdict(lambda: defaultdict(Decimal))
    years: set[int] = set()
    assets: set[str] = set()

    def acquire(year, asset, qty, cost):
        acq[(year, asset)].append((qty, cost))
        seq[(year, asset)].append(("acquire", (qty, cost)))
        assets.add(asset)

    def dispose(year, asset, qty, rev, expense=ZERO):
        disposed[(year, asset)] += qty
        revenue[(year, asset)] += rev
        other_expenses[(year, asset)] += expense
        seq[(year, asset)].append(("dispose", qty))
        assets.add(asset)

    for event in ordered_events(vector):
        kind = event["type"]
        year = parse_ts(event["ts"]).astimezone(tz).year
        years.add(year)
        try:
            if kind == "buy":
                acquire(year, event["asset"], D(event["qty"]), D(event["fiat"]) + D(event.get("fiat_fee")))
            elif kind == "income":
                income_kind = event["income_kind"]
                if income_kind == "hardfork":
                    acquire(year, event["asset"], D(event["qty"]), ZERO)
                elif income_kind in JP_INCOME:
                    value = D(event["fiat"])
                    receipts[year][income_kind] += value
                    acquire(year, event["asset"], D(event["qty"]), value)
                else:
                    raise Quarantine(f"jp_{income_kind}_requires_review")
            elif kind == "sell":
                price = D(event["fiat"])
                market = event.get("market_value")
                if market is not None and price < D(market) * Decimal("0.7"):
                    flags.append({"event": event["id"], "code": "jp_low_value_transfer_70pct"})
                    price = D(market) * Decimal("0.7")
                fee_units = D(event.get("crypto_fee"))
                fee_value = D(event.get("fee_value"))
                dispose(year, event["asset"], D(event["qty"]) + fee_units, price + fee_value, D(event.get("fiat_fee")) + fee_value)
            elif kind == "swap":
                value = D(event["in_value"])
                dispose(year, event["out_asset"], D(event["out_qty"]), value)
                acquire(year, event["in_asset"], D(event["in_qty"]), value)
            elif kind == "spend":
                value = D(event["fiat"]) if event.get("fiat") is not None else D(event["market_value"])
                dispose(year, event["asset"], D(event["qty"]), value)
            elif kind == "gift_out":
                dispose(year, event["asset"], D(event["qty"]), D(event["fiat"]))
            elif kind == "transfer":
                fee = D(event.get("fee"))
                if fee > ZERO:
                    dispose(year, event["asset"], fee, D(event["fee_value"]), D(event["fee_value"]))
            elif kind == "lost":
                raise Quarantine("jp_lost_requires_review")
        except Quarantine as exc:
            quarantines.append({"event": event["id"], "reason": str(exc)})

    out_years: dict[str, Any] = {}
    opening: dict[str, tuple[Decimal, Decimal]] = {}
    for year in range(min(years), max(years) + 1) if years else []:
        if year > JP_LAST_SUPPORTED_YEAR:
            refusals.append({"year": year, "reason": "jp_separate_taxation_regime_unconfirmed"})
            continue
        per_asset: dict[str, Any] = {}
        total = ZERO
        blocked = False
        for asset in sorted(assets):
            key = (year, asset)
            q0, v0 = opening.get(asset, (ZERO, ZERO))
            qa = sum((q for q, _ in acq[key]), ZERO)
            va = sum((c for _, c in acq[key]), ZERO)
            qd = disposed[key]
            if q0 == ZERO and qa == ZERO and qd == ZERO:
                continue
            if method == "total_average":
                qt = q0 + qa
                vt = v0 + va
                if qd > qt:
                    quarantines.append({"event": f"{year}:{asset}", "reason": "missing_cost_basis"})
                    blocked = True
                    continue
                cost = vt * qd / qt if qt > ZERO else ZERO
                closing_q, closing_v = qt - qd, vt - cost
                unit = vt / qt if qt > ZERO else ZERO
            else:
                q, book, unit, cost = q0, v0, (v0 / q0 if q0 > ZERO else ZERO), ZERO
                for action, payload in seq[key]:
                    if action == "acquire":
                        aq, ac = payload
                        q += aq
                        book += ac
                        unit = book / q
                    else:
                        if payload > q:
                            raise SystemExit(f"{vector['id']}: moving average shortfall")
                        remaining = q - payload
                        new_book = (unit * remaining).quantize(Decimal(1), rounding=ROUND_CEILING) if remaining > ZERO else ZERO
                        cost += book - new_book
                        q, book = remaining, new_book
                closing_q, closing_v = q, book
            rev = revenue[key]
            exp = cost + other_expenses[key]
            rev_r = rev.quantize(Decimal(1), rounding=ROUND_FLOOR)
            exp_r = exp.quantize(Decimal(1), rounding=ROUND_CEILING)
            per_asset[asset] = {
                "opening_qty": plain(q0),
                "opening_value": plain(v0),
                "acquired_qty": plain(qa),
                "acquired_value": plain(va),
                "unit_cost": plain(unit),
                "disposed_qty": plain(qd),
                "cost_of_disposals": plain(cost),
                "closing_qty": plain(closing_q),
                "closing_value": plain(closing_v),
                "revenue": plain(rev),
                "other_expenses": plain(other_expenses[key]),
                "revenue_rounded": plain(rev_r),
                "expenses_rounded": plain(exp_r),
                "income": plain(rev_r - exp_r),
            }
            if rev > Decimal(3_000_000):
                flags.append({"event": f"{year}:{asset}", "code": "jp_revenue_over_3m_classification"})
            total += rev_r - exp_r
            opening[asset] = (closing_q, closing_v)
        receipts_total = sum(receipts[year].values(), ZERO).quantize(Decimal(1), rounding=ROUND_FLOOR)
        out_years[str(year)] = {
            "method": method,
            "assets": per_asset,
            "income_receipts": {k: plain(v) for k, v in sorted(receipts[year].items())},
            "income_receipts_rounded": plain(receipts_total),
            "zatsu_crypto_total": None if blocked else plain(total + receipts_total),
        }
        if not blocked and total + receipts_total < ZERO:
            flags.append({"event": str(year), "code": "jp_zatsu_loss_no_offset"})
    return {"quarantines": quarantines, "flags": flags, "refusals": refusals, "years": out_years}


COMPUTE = {"es": es_compute, "ie": ie_compute, "jp": jp_compute}


def diff(expected: Any, actual: Any, path: str = "") -> list[str]:
    if isinstance(expected, dict) and isinstance(actual, dict):
        out = []
        for key in sorted(set(expected) | set(actual)):
            if key not in actual:
                out.append(f"{path}.{key}: missing in computed")
            elif key not in expected:
                out.append(f"{path}.{key}: unexpected {json.dumps(actual[key], ensure_ascii=False)}")
            else:
                out.extend(diff(expected[key], actual[key], f"{path}.{key}"))
        return out
    if isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            return [f"{path}: expected {len(expected)} items, computed {len(actual)}: {json.dumps(actual, ensure_ascii=False)}"]
        out = []
        for index, (e, a) in enumerate(zip(expected, actual)):
            out.extend(diff(e, a, f"{path}[{index}]"))
        return out
    if expected != actual:
        return [f"{path}: expected {json.dumps(expected, ensure_ascii=False)}, computed {json.dumps(actual, ensure_ascii=False)}"]
    return []


# The vectors use the shared schema kassiber.tax-country-vectors/1 (time,
# quantity, policy; currency and timezone on the document), which every
# country's file and the Rust runner (crates/reports/tests/
# tax_country_vectors.rs) read. The calculators below were written against
# an earlier spelling; `legacy` turns a vector back into it.
LEGACY_KEYS = {"time": "ts", "quantity": "qty", "to_wallet": "to", "subkind": "income_kind",
               "fee_fiat": "fiat_fee", "fee_fiat_value": "fee_value", "in_quantity": "in_qty", "in_fiat": "in_value"}


def legacy(vector: dict[str, Any], document: dict[str, Any]) -> dict[str, Any]:
    events = []
    for event in vector["events"]:
        out = {}
        for key, value in event.items():
            if event["type"] == "swap" and key in ("asset", "quantity", "fiat"):
                out[{"asset": "out_asset", "quantity": "out_qty", "fiat": "out_value"}[key]] = value
            elif key == "fee_quantity":
                out["fee" if event["type"] == "transfer" else "crypto_fee"] = value
            else:
                out[LEGACY_KEYS.get(key, key)] = value
        events.append(out)
    book = {"currency": document["currency"], "timezone": document["timezone"]}
    book.update({k: v for k, v in vector["policy"].items() if k != "pool_scope"})
    return {**{k: v for k, v in vector.items() if k not in ("policy", "events")}, "book": book, "events": events}


def command_check(path: Path, show: bool) -> int:
    document = json.loads(path.read_text(encoding="utf-8"))
    compute = COMPUTE[document["country"]]
    failures = 0
    for vector in document["vectors"]:
        computed = compute(legacy(vector, document))
        if show:
            print(json.dumps({vector["id"]: computed}, indent=2, ensure_ascii=False))
            continue
        problems = diff(vector["expected"], computed, vector["id"])
        if problems:
            failures += 1
            print("\n".join(problems))
    if not show:
        print(f"{path.name}: {len(document['vectors']) - failures}/{len(document['vectors'])} vectors match")
    return 1 if failures else 0


# --------------------------------------------------------------------------
# RP2
# --------------------------------------------------------------------------

RP2_INCOME_TYPES = {
    "staking": "staking",
    "lending_interest": "interest",
    "interest": "interest",
    "mining": "mining",
    "airdrop": "airdrop",
    "hardfork": "hardfork",
    "income": "income",
}

INI_HEADERS = """
[in_header]
timestamp = 0
asset = 1
exchange = 2
holder = 3
transaction_type = 4
spot_price = 5
crypto_in = 6
crypto_fee = 7
fiat_in_no_fee = 8
fiat_in_with_fee = 9
fiat_fee = 10
unique_id = 11
notes = 12

[out_header]
timestamp = 0
asset = 1
exchange = 2
holder = 3
transaction_type = 4
spot_price = 5
crypto_out_no_fee = 6
crypto_fee = 7
crypto_out_with_fee = 8
fiat_out_no_fee = 9
fiat_fee = 10
unique_id = 11
notes = 12

[intra_header]
timestamp = 0
asset = 1
from_exchange = 2
from_holder = 3
to_exchange = 4
to_holder = 5
spot_price = 6
crypto_sent = 7
crypto_received = 8
unique_id = 9
notes = 10
"""


def import_rp2(scratch: Path) -> dict[str, Any]:
    scratch.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="rp2-import-", dir=scratch))
    previous = os.getcwd()
    os.chdir(work)
    try:
        import importlib

        logger_module = importlib.import_module("rp2.logger")
        modules = {
            name: importlib.import_module(f"rp2.{name}")
            for name in ("configuration", "in_transaction", "out_transaction", "intra_transaction", "transaction_set", "input_data", "tax_engine", "accounting_engine", "rp2_decimal")
        }
        modules["avl"] = importlib.import_module("prezzemolo.avl_tree")
        modules["fifo"] = importlib.import_module("rp2.plugin.accounting_method.fifo")
        modules["countries"] = {cc: importlib.import_module(f"rp2.plugin.country.{cc}") for cc in ("es", "ie", "jp")}
    finally:
        os.chdir(previous)
    logger = getattr(logger_module, "LOGGER", logging.getLogger("rp2"))
    for handler in list(logger.handlers):
        log_file = getattr(handler, "baseFilename", None)
        logger.removeHandler(handler)
        handler.close()
        if log_file:
            Path(log_file).unlink(missing_ok=True)
    logger.addHandler(logging.NullHandler())
    logger.propagate = False
    return modules | {"work": work}


def rp2_build(vector: dict[str, Any], country: Any, modules: dict[str, Any], notes: list[str]) -> tuple[Any, dict[str, dict[str, Any]]]:
    """RP2 configuration and transaction sets for ``vector``, as Kassiber's rows would map.

    A constructor refusal is recorded in ``notes`` and the event is skipped.
    """
    R = modules["rp2_decimal"].RP2Decimal
    events = vector["events"]
    assets = sorted({e.get("asset") or e.get("out_asset") for e in events} | {e["in_asset"] for e in events if e["type"] == "swap"})
    wallets = sorted({e.get("wallet", "Main") for e in events} | {e["to"] for e in events if e["type"] == "transfer"})
    ini = modules["work"] / f"{vector['id']}.ini"
    ini.write_text(
        f"[general]\nassets = {', '.join(assets)}\nexchanges = {', '.join(wallets)}\nholders = Holder\n" + INI_HEADERS,
        encoding="utf-8",
    )
    config = modules["configuration"].Configuration(str(ini), country)
    In, Out, Intra = modules["in_transaction"].InTransaction, modules["out_transaction"].OutTransaction, modules["intra_transaction"].IntraTransaction
    sets: dict[str, dict[str, Any]] = {}

    def tx_set(asset: str, kind: str):
        sets.setdefault(asset, {})
        if kind not in sets[asset]:
            sets[asset][kind] = modules["transaction_set"].TransactionSet(config, kind.upper(), asset)
        return sets[asset][kind]

    row = 0
    for event in events:
        row += 1
        ts = event["ts"].replace("Z", "+00:00")
        wallet = event.get("wallet", "Main")
        kind = event["type"]
        try:
            if kind == "buy":
                qty, fiat, fee = R(event["qty"]), R(event["fiat"]), R(event.get("fiat_fee") or "0")
                tx_set(event["asset"], "in").add_entry(
                    In(config, ts, event["asset"], wallet, "Holder", "BUY", fiat / qty, qty, fiat_in_no_fee=fiat, fiat_fee=fee if fee > R(0) else None, row=row, unique_id=event["id"])
                )
            elif kind == "income":
                qty, fiat = R(event["qty"]), R(event.get("fiat") or "0")
                if fiat == R(0):
                    notes.append(f"{event['id']}: zero-value {event['income_kind']} needs a positive spot price in RP2; skipped")
                    continue
                tx_set(event["asset"], "in").add_entry(
                    In(config, ts, event["asset"], wallet, "Holder", RP2_INCOME_TYPES[event["income_kind"]].upper(), fiat / qty, qty, fiat_in_no_fee=fiat, row=row, unique_id=event["id"])
                )
            elif kind in ("sell", "spend", "gift_out", "lost"):
                qty = R(event["qty"])
                # A Kassiber row carries the coins' market value; RP2 knows no other.
                if kind == "spend":
                    fiat = R(event.get("market_value") or event["fiat"])
                else:
                    fiat = R(event.get("fiat") or "0")
                tx_type = {"sell": "SELL", "spend": "SELL", "gift_out": "GIFT", "lost": "LOST"}[kind]
                spot = fiat / qty if fiat > R(0) else R(1)
                crypto_fee = R(event.get("crypto_fee") or "0")
                fiat_fee = R(event.get("fee_value") or event.get("fiat_fee") or "0")
                tx_set(event["asset"], "out").add_entry(
                    Out(config, ts, event["asset"], wallet, "Holder", tx_type, spot, qty, crypto_fee, fiat_out_no_fee=fiat if fiat > R(0) else None, fiat_fee=fiat_fee if crypto_fee == R(0) and fiat_fee > R(0) else None, row=row, unique_id=event["id"])
                )
            elif kind == "swap":
                # Each leg at its own market value, as a taxable cross-asset pair books today.
                out_qty, out_value = R(event["out_qty"]), R(event["out_value"] or event["in_value"])
                in_qty, in_value = R(event["in_qty"]), R(event["in_value"])
                tx_set(event["out_asset"], "out").add_entry(
                    Out(config, ts, event["out_asset"], wallet, "Holder", "SELL", out_value / out_qty, out_qty, R(0), fiat_out_no_fee=out_value, row=row, unique_id=event["id"])
                )
                row += 1
                tx_set(event["in_asset"], "in").add_entry(
                    In(config, ts, event["in_asset"], wallet, "Holder", "BUY", in_value / in_qty, in_qty, fiat_in_no_fee=in_value, row=row, unique_id=event["id"] + ":in")
                )
            elif kind == "transfer":
                qty, fee = R(event["qty"]), R(event.get("fee") or "0")
                spot = R(event["fee_value"]) / fee if fee > R(0) else R(1)
                tx_set(event["asset"], "intra").add_entry(
                    Intra(config, ts, event["asset"], wallet, "Holder", event["to"], "Holder", spot, qty + fee, qty, row=row, unique_id=event["id"])
                )
        except Exception as exc:  # RP2 constructor refusals are results, not crashes
            notes.append(f"{event['id']}: {type(exc).__name__}: {str(exc).splitlines()[0]}")
    return config, sets


def rp2_compute(config: Any, sets: dict[str, dict[str, Any]], modules: dict[str, Any], notes: list[str]) -> tuple[dict[str, Any], bool]:
    """RP2's ComputedData per asset (FIFO); an asset whose computation fails is noted and left out."""
    years_2_methods = modules["avl"].AVLTree()
    years_2_methods.insert_node(1970, modules["fifo"].AccountingMethod())
    computed: dict[str, Any] = {}
    ok = True
    for asset in sorted(sets):
        if "in" not in sets[asset]:
            notes.append(f"{asset}: no acquisitions; RP2 computes nothing for it")
            continue
        input_data = modules["input_data"].InputData(
            asset,
            sets[asset]["in"],
            sets[asset].get("out") or modules["transaction_set"].TransactionSet(config, "OUT", asset),
            sets[asset].get("intra") or modules["transaction_set"].TransactionSet(config, "INTRA", asset),
        )
        engine = modules["accounting_engine"].AccountingEngine(years_2_methods=years_2_methods)
        try:
            computed[asset] = modules["tax_engine"].compute_tax(config, engine, input_data)
        except Exception as exc:
            ok = False
            notes.append(f"{asset}: {type(exc).__name__}: {str(exc).splitlines()[0]}")
    return computed, ok


def rp2_run(vector: dict[str, Any], country_code: str, modules: dict[str, Any]) -> dict[str, Any]:
    country_class = {"es": "ES", "ie": "IE", "jp": "JP"}[country_code]
    country = getattr(modules["countries"][country_code], country_class)()
    if not country.get_accounting_methods():
        return {"status": "refused", "reason": "RP2's JP country offers no accounting method in the pinned fork (plugin disabled)"}
    notes: list[str] = []
    config, sets = rp2_build(vector, country, modules, notes)
    computed_by_asset, ok = rp2_compute(config, sets, modules, notes)
    results: dict[str, Any] = {"status": "ok" if ok else "error", "method": "fifo", "gains": [], "yearly": [], "notes": notes}
    for asset, computed in computed_by_asset.items():
        for gain_loss in computed.gain_loss_set:
            event = gain_loss.taxable_event
            results["gains"].append(
                {
                    "event": event.unique_id,
                    "type": event.transaction_type.value,
                    "asset": asset,
                    "lot": gain_loss.acquired_lot.unique_id if gain_loss.acquired_lot else None,
                    "qty": plain(Decimal(str(gain_loss.crypto_amount))),
                    "proceeds": money(Decimal(str(gain_loss.taxable_event_fiat_amount_with_fee_fraction))),
                    "cost_basis": money(Decimal(str(gain_loss.fiat_cost_basis))),
                    "gain": money(Decimal(str(gain_loss.fiat_gain))),
                    "year": event.timestamp.year,
                }
            )
        for yearly in computed.yearly_gain_loss_list:
            results["yearly"].append(
                {
                    "year": yearly.year,
                    "asset": yearly.asset,
                    "type": yearly.transaction_type.value,
                    "long_term": yearly.is_long_term_capital_gains,
                    "proceeds": money(Decimal(str(yearly.fiat_amount))),
                    "cost_basis": money(Decimal(str(yearly.fiat_cost_basis))),
                    "gain": money(Decimal(str(yearly.fiat_gain_loss))),
                }
            )
    return results


def command_rp2(path: Path, scratch: Path, write: bool) -> int:
    document = json.loads(path.read_text(encoding="utf-8"))
    modules = import_rp2(scratch)
    for vector in document["vectors"]:
        computed = rp2_run(legacy(vector, document), document["country"], modules)
        vector.setdefault("rp2", {})["computed"] = computed
        if not write:
            print(json.dumps({vector["id"]: computed}, indent=2, ensure_ascii=False))
    if write:
        path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"wrote RP2 results for {len(document['vectors'])} vectors to {path}")
    return 0


def command_rp2_jp_template(path: Path, scratch: Path, write: bool) -> int:
    """Evaluate RP2's JP report template (tax_report_jp) on the JP vectors.

    RP2's JP country is disabled in the pinned fork, but its report template
    still implements the total-average method as spreadsheet formulas. This
    runs RP2's lot engine under a JP subclass that allows FIFO (the template
    reads the transactions, not the lots), renders the template, lets
    headless LibreOffice recalculate it into XLSX and reads each year's
    summary sheet back.
    """
    import shutil
    import subprocess

    import openpyxl

    document = json.loads(path.read_text(encoding="utf-8"))
    modules = import_rp2(scratch)
    import importlib

    jp_module = modules["countries"]["jp"]
    generator_module = importlib.import_module("rp2.plugin.report.jp.tax_report_jp")
    configuration_module = modules["configuration"]

    class JPForTemplate(jp_module.JP):
        def get_accounting_methods(self):
            return {"fifo"}

        def get_default_accounting_method(self):
            return "fifo"

    soffice = shutil.which("soffice")
    if soffice is None:
        raise SystemExit("soffice (LibreOffice) is required for rp2-jp-template")
    profile = scratch / "lo-profile"
    for vector in document["vectors"]:
        result: dict[str, Any] = {"status": "ok", "summary": {}, "notes": []}
        out_dir = Path(tempfile.mkdtemp(prefix=f"{vector['id']}-", dir=scratch))
        try:
            config, sets = rp2_build(legacy(vector, document), JPForTemplate(), modules, result["notes"])
            computed_by_asset, _ = rp2_compute(config, sets, modules, result["notes"])
            if not computed_by_asset:
                raise RuntimeError("nothing to render")
            generator_module.Generator().generate(
                country=JPForTemplate(),
                years_2_accounting_method_names={1970: "fifo"},
                asset_to_computed_data=computed_by_asset,
                output_dir_path=str(out_dir),
                output_file_prefix="",
                from_date=configuration_module.MIN_DATE,
                to_date=configuration_module.MAX_DATE,
                generation_language="en",
            )
            ods = next(out_dir.glob("*tax_report_jp.ods"))
            subprocess.run(
                [soffice, f"-env:UserInstallation=file://{profile}", "--headless", "--norestore", "--convert-to", "xlsx", "--outdir", str(out_dir), str(ods)],
                check=True,
                capture_output=True,
                timeout=180,
            )
            workbook = openpyxl.load_workbook(ods.with_suffix(".xlsx"), data_only=True)
            for sheet in workbook.worksheets:
                if not sheet.title.endswith("_Summary"):
                    continue
                year = sheet.title.split("_")[0]
                rows = []
                for row in sheet.iter_rows(min_row=8, values_only=True):
                    if row[0] in (None, ""):
                        continue
                    rows.append(
                        {
                            "asset": row[0],
                            "donations": row[1],
                            "gifts": row[2],
                            "average_unit_price": row[3],
                            "end_balance_crypto": row[4],
                            "end_balance_yen": row[5],
                            "net_income": row[6],
                        }
                    )
                result["summary"][year] = rows
        except Exception as exc:
            result = {"status": "error", "notes": result["notes"] + [f"{type(exc).__name__}: {str(exc).splitlines()[0]}"]}
        vector.setdefault("rp2", {})["template"] = result
        if not write:
            print(json.dumps({vector["id"]: result}, indent=2, ensure_ascii=False, default=str))
    if write:
        path.write_text(json.dumps(document, indent=2, ensure_ascii=False, default=str) + "\n", encoding="utf-8")
        print(f"wrote RP2 JP template results for {len(document['vectors'])} vectors to {path}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check")
    check.add_argument("vectors", type=Path)
    check.add_argument("--show", action="store_true", help="print the computed results instead of comparing")
    rp2 = sub.add_parser("rp2")
    rp2.add_argument("vectors", type=Path)
    rp2.add_argument("--scratch", type=Path, required=True)
    rp2.add_argument("--write", action="store_true")
    template = sub.add_parser("rp2-jp-template")
    template.add_argument("vectors", type=Path)
    template.add_argument("--scratch", type=Path, required=True)
    template.add_argument("--write", action="store_true")
    args = parser.parse_args()
    if args.command == "check":
        return command_check(args.vectors, args.show)
    if args.command == "rp2-jp-template":
        return command_rp2_jp_template(args.vectors, args.scratch, args.write)
    return command_rp2(args.vectors, args.scratch, args.write)


if __name__ == "__main__":
    sys.exit(main())
