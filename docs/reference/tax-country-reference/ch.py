"""Switzerland (ch) reference calculator for docs/reference/tax-country-ch.md.
Research only, not product code.

Private capital gains are tax-free (Art. 16 Abs. 3 DBG); the report carries
income (Art. 16 Abs. 1, Art. 20 DBG), year-end wealth (Art. 13/14 StHG) and
the KS 36 safe-harbour indicators. Capital gains are informational.
"""

from __future__ import annotations

from datetime import date
from fractions import Fraction

from common import F, add_months, local_date, local_dt, money, qty

TZ = "Europe/Zurich"
CATEGORY = {
    "staking": "movable_assets_income",  # Art. 20 Abs. 1 DBG (ESTV Arbeitspapier 2.2.2)
    "lending_interest": "movable_assets_income",
    "interest": "movable_assets_income",
    "airdrop": "movable_assets_income",  # Dossier 4.2.2.1.3 / 11.2.1
    "airdrop_service": "general_clause_income",  # Art. 16 Abs. 1 DBG
    "routing_income": "general_clause_income",
    "mining": "mining_income",  # Art. 16 Abs. 1 / Art. 18 DBG if self-employed
}


class Refusal(Exception):
    def __init__(self, code, event, reason):
        super().__init__(code)
        self.code, self.event, self.reason = code, event, reason


def run(vector):
    policy = vector.get("policy", {})
    if policy.get("professional_trader"):
        raise Refusal("professional_trading_unsupported", "-", "KS 36: business-asset accounting (Art. 18 DBG) is not modelled")
    method = policy.get("method", "fifo")
    assert method == "fifo"
    lots: dict[str, list] = {}  # global FIFO per asset, informational
    bal: dict[tuple[str, str], Fraction] = {}
    disposals, income, info_income = [], [], []
    volume: dict[int, Fraction] = {}
    events = sorted(vector["events"], key=lambda e: local_dt(e["time"], TZ))
    snapshots = {}  # year -> balances at end of 31 Dec

    def snap_until(d):
        for y in range(min_year, d.year):
            if y not in snapshots:
                snapshots[y] = {k: v for k, v in bal.items() if v}

    def add(asset, wallet, q, cost, d, ev_id):
        lots.setdefault(asset, []).append({"lot": ev_id, "qty": q, "cost": cost, "date": d})
        bal[(wallet, asset)] = bal.get((wallet, asset), Fraction(0)) + q

    def take(asset, wallet, q, proceeds, d, ev_id, kind):
        bal[(wallet, asset)] = bal.get((wallet, asset), Fraction(0)) - q
        if bal[(wallet, asset)] < 0:
            raise Refusal("insufficient_holdings", ev_id, f"{wallet} {asset}")
        frags, left = [], q
        while left > 0:
            if not lots.get(asset):
                raise Refusal("insufficient_holdings", ev_id, asset)
            lot = lots[asset][0]
            t = min(left, lot["qty"])
            cost = lot["cost"] * t / lot["qty"]
            p = proceeds * t / q
            six = add_months(lot["date"], 6)
            frags.append({"lot": lot["lot"], "acquired": lot["date"].isoformat(), "quantity": qty(t), "cost": money(cost), "proceeds": money(p),
                          "gain": money(p - cost), "_g": p - cost, "held_6_months": d >= six})
            lot["qty"] -= t; lot["cost"] -= cost; left -= t
            if lot["qty"] == 0:
                lots[asset].pop(0)
        disposals.append({"event": ev_id, "kind": kind, "asset": asset, "date": d.isoformat(), "_y": d.year, "fragments": frags})

    min_year = local_date(events[0]["time"], TZ).year - 1 if events else 2000
    for ev in events:
        d = local_date(ev["time"], TZ)
        snap_until(d)
        t, asset, w = ev["type"], ev.get("asset", "BTC"), ev.get("wallet", "A")
        if t == "hardfork":
            raise Refusal("hardfork_treatment_unsupported", ev["id"], "ESTV guidance does not settle the income/cost treatment of fork coins")
        if t == "buy":
            c = F(ev["fiat"]) + F(ev.get("fee_fiat"))
            add(asset, w, F(ev["quantity"]), c, d, ev["id"])
            volume[d.year] = volume.get(d.year, Fraction(0)) + c
        elif t == "sell":
            p = F(ev["fiat"])
            take(asset, w, F(ev["quantity"]), p, d, ev["id"], "sell")
            volume[d.year] = volume.get(d.year, Fraction(0)) + p
        elif t in ("swap", "peg"):
            given = F(ev["fiat"]) if ev.get("fiat") is not None else None
            received = F(ev["in_fiat"]) if ev.get("in_fiat") is not None else given
            if t == "peg" and policy.get("rail_carrying_value", True):
                # tax-neutral for a private investor; carry cost (informational) to the new rail
                q = F(ev["quantity"])
                moved = []
                bal[(w, asset)] -= q
                left = q
                while left > 0:
                    lot = lots[asset][0]
                    tq = min(left, lot["qty"]); c = lot["cost"] * tq / lot["qty"]
                    moved.append((lot, tq, c)); lot["qty"] -= tq; lot["cost"] -= c; left -= tq
                    if lot["qty"] == 0:
                        lots[asset].pop(0)
                ratio = F(ev["in_quantity"]) / q
                for lot, tq, c in moved:
                    lots.setdefault(ev["in_asset"], []).append({"lot": lot["lot"], "qty": tq * ratio, "cost": c, "date": lot["date"]})
                bal[(w, ev["in_asset"])] = bal.get((w, ev["in_asset"]), Fraction(0)) + F(ev["in_quantity"])
                continue
            take(asset, w, F(ev["quantity"]), received, d, ev["id"], "swap")
            add(ev["in_asset"], w, F(ev["in_quantity"]), given if given is not None else received, d, ev["id"] + ":in")
            volume[d.year] = volume.get(d.year, Fraction(0)) + received + (given if given is not None else received)
        elif t == "transfer":
            q = F(ev["quantity"]); fq = F(ev.get("fee_quantity"))
            bal[(w, asset)] -= q + fq
            bal[(ev["to_wallet"], asset)] = bal.get((ev["to_wallet"], asset), Fraction(0)) + q
            if fq:  # the fee leaves the global pool (informational; transaction costs are not deductible)
                take(asset, ev["to_wallet"], fq, F(ev.get("fee_fiat_value")), d, ev["id"] + ":fee", "fee")
                bal[(ev["to_wallet"], asset)] += fq
        elif t in ("income", "airdrop"):
            kind = ev["subkind"] if t == "income" else ev.get("subkind", "airdrop")
            if kind == "airdrop_unclassified":
                kind = "airdrop"
            v = F(ev["fiat"])
            income.append({"event": ev["id"], "date": d.isoformat(), "_y": d.year, "subkind": kind, "category": CATEGORY[kind], "quantity": qty(F(ev["quantity"])), "value_chf": money(v), "_v": v})
            add(asset, w, F(ev["quantity"]), v, d, ev["id"])
        elif t in ("wages", "gift_in"):
            v = F(ev["fiat"])
            info_income.append({"event": ev["id"], "date": d.isoformat(), "_y": d.year, "kind": t, "value_chf": money(v), "_v": v,
                                "treatment": "employment income on the Lohnausweis (Art. 17 DBG)" if t == "wages" else "gift: not income (Art. 24 lit. a DBG); cantonal gift tax"})
            add(asset, w, F(ev["quantity"]), v, d, ev["id"])
        else:
            raise ValueError(t)
    last_year = max([local_date(e["time"], TZ).year for e in events] + [int(y[:4]) for y in vector.get("valuations", {})])
    snap_until(date(last_year + 1, 1, 1))

    vals = vector.get("valuations", {})

    def wealth(year):
        hold = snapshots.get(year, {})
        rows, total, complete = [], Fraction(0), True
        for (wallet, asset), q in sorted(hold.items()):
            per_year = vals.get(f"{year}-12-31", {})
            src_asset = asset
            flags = []
            if asset == "LBTC" and "LBTC" not in per_year:
                src_asset = "BTC"; flags.append("lbtc_valued_as_btc")
            v = per_year.get(src_asset)
            if v is None:
                rows.append({"wallet": wallet, "asset": asset, "quantity": qty(q), "unit_value_chf": None, "value_chf": None, "source": None, "flags": flags + ["missing_valuation"]})
                complete = False
                continue
            if v["source"] != "estv_kursliste":
                flags.append("not_estv_tax_value")
            val = q * F(v["chf"])
            total += val
            rows.append({"wallet": wallet, "asset": asset, "quantity": qty(q), "unit_value_chf": v["chf"], "value_chf": money(val), "source": v["source"], "flags": flags})
        return rows, (money(total) if complete else None), complete

    years = {}
    all_years = sorted({local_date(e["time"], TZ).year for e in events} | {int(k[:4]) for k in vals})
    declared = policy.get("declared", {})
    for y in all_years:
        ds = [d for d in disposals if d["_y"] == y]
        frags = [f for d in ds for f in d["fragments"]]
        gains = sum((f["_g"] for f in frags), Fraction(0))
        rows, total, complete = wealth(y)
        start_rows, start_total, start_complete = wealth(y - 1)
        inc = [i for i in income if i["_y"] == y]
        by_cat = {}
        for i in inc:
            by_cat[i["category"]] = by_cat.get(i["category"], Fraction(0)) + i["_v"]
        # KS 36 indicators
        c1 = "no_disposals" if not frags else ("met" if all(f["held_6_months"] for f in frags) else "not_met")
        vol = volume.get(y, Fraction(0))
        if start_total is None or F(start_total) == 0:
            c2 = {"result": "not_assessable", "volume_chf": money(vol), "limit_chf": None, "basis": "crypto_only"}
        else:
            limit = 5 * F(start_total)
            c2 = {"result": "met" if vol <= limit else "not_met", "volume_chf": money(vol), "limit_chf": money(limit), "basis": "crypto_only"}
        if "net_income_chf" in policy:
            half = F(policy["net_income_chf"]) / 2
            c3 = {"result": "met" if gains < half else "not_met", "realized_gains_chf": money(gains), "half_net_income_chf": money(half)}
        else:
            c3 = {"result": "not_assessable", "realized_gains_chf": money(gains), "half_net_income_chf": None}
        c4 = {True: "not_met", False: "met"}.get(declared.get("debt_financed_without_cover"), "not_assessable")
        c5 = {True: "met", False: "not_met"}.get(declared.get("derivatives_only_hedging"), "not_assessable")
        results = [c1 if c1 != "no_disposals" else "met", c2["result"], c3["result"], c4, c5]
        overall = "not_met" if "not_met" in results else ("inconclusive" if "not_assessable" in results else "met")
        years[str(y)] = {
            "taxable_capital_gains_chf": "0.00",
            "informational_realized_gain_chf": money(gains),
            "income_by_category_chf": {k: money(v) for k, v in sorted(by_cat.items())},
            "income_total_chf": money(sum(by_cat.values(), Fraction(0))),
            "not_in_income_total": [{k: v for k, v in i.items() if not k.startswith("_")} for i in info_income if i["_y"] == y],
            "wealth_31_dec": {"rows": rows, "total_chf": total, "complete": complete},
            "ks36": {"c1_holding_6_months": c1, "c2_volume_5x": c2, "c3_gains_under_half_income": c3, "c4_financing": c4, "c5_derivatives": c5, "overall": overall},
        }
    strip = lambda d: {k: v for k, v in d.items() if not k.startswith("_")}
    return {
        "disposals": [strip(d) | {"fragments": [strip(f) for f in d["fragments"]]} for d in disposals],
        "income": [strip(i) for i in income],
        "years": years,
    }


def evaluate(vector):
    try:
        return run(vector)
    except Refusal as r:
        return {"refusal": {"code": r.code, "event": r.event, "reason": r.reason}}
