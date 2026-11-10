"""United Kingdom (registry value gb) reference calculator for
docs/reference/tax-country-gb.md. Research only, not product code.

Share-matching for cryptoassets (CRYPTO22200): per token, per UK calendar
day; same-day rule (TCGA92/S105), then the 30-day rule (S106A(5)), then the
section 104 pool (S104) at average cost.
"""

from __future__ import annotations

from collections import defaultdict
from datetime import date, timedelta
from decimal import ROUND_DOWN, ROUND_HALF_UP, Decimal
from fractions import Fraction

from common import F, local_date, local_dt, money, qty

TZ = "Europe/London"

# Annual exempt amount for individuals, by tax year start (sources in the spec).
AEA = {2018: "11700", 2019: "12000", 2020: "12300", 2021: "12300", 2022: "12300", 2023: "6000", 2024: "3000", 2025: "3000", 2026: "3000"}
RATE_CHANGE = date(2024, 10, 30)  # main CGT rates 10/20 -> 18/24 for disposals on or after this date
MISC_KINDS = {"staking", "mining", "routing_income", "airdrop_service", "lending_interest", "interest"}


class Refusal(Exception):
    def __init__(self, code, event, reason):
        super().__init__(code)
        self.code, self.event, self.reason = code, event, reason


def tax_year(d: date) -> int:
    return d.year if (d.month, d.day) >= (4, 6) else d.year - 1


def label(y: int) -> str:
    return f"{y}-{(y + 1) % 100:02d}"


def pounds_truncated(v: Fraction) -> str:
    d = Decimal(v.numerator) / Decimal(v.denominator)
    return str(d.quantize(Decimal(1), rounding=ROUND_DOWN))


def pounds_nearest(v: Fraction) -> int:
    d = Decimal(v.numerator) / Decimal(v.denominator)
    return int(d.quantize(Decimal(1), rounding=ROUND_HALF_UP))


def run(vector):
    policy = vector.get("policy", {})
    acq = defaultdict(lambda: defaultdict(lambda: {"qty": Fraction(0), "cost": Fraction(0), "events": []}))
    disp = defaultdict(lambda: defaultdict(lambda: {"qty": Fraction(0), "proceeds": Fraction(0), "incidental": Fraction(0), "events": []}))
    income = []

    def add_acq(asset, d, q, cost, ev_id):
        a = acq[asset][d]
        a["qty"] += q; a["cost"] += cost; a["events"].append(ev_id)

    def add_disp(asset, d, q, proceeds, incidental, ev_id):
        x = disp[asset][d]
        x["qty"] += q; x["proceeds"] += proceeds; x["incidental"] += incidental; x["events"].append(ev_id)

    for ev in sorted(vector["events"], key=lambda e: local_dt(e["time"], TZ)):
        t, d = ev["type"], local_date(ev["time"], TZ)
        asset = ev.get("asset", "BTC")
        if t == "airdrop" and ev.get("subkind") != "airdrop_service":
            raise Refusal("airdrop_classification_required", ev["id"], "CRYPTO21250: income only if received for a service; cost basis of a free airdrop is not settled")
        if t == "gift_out" and ev.get("recipient") not in ("other",):
            raise Refusal("gift_recipient_classification_required", ev["id"], "spouse/civil partner (no gain/no loss) or charity relief must be reviewed")
        if t == "hardfork":
            raise Refusal("hardfork_cost_split_unsupported", ev["id"], "CRYPTO22300: just and reasonable apportionment of pooled cost is not modelled")
        if t == "peg" and not policy.get("rail_policy"):
            raise Refusal("rail_policy_required", ev["id"], "BTC↔LBTC change needs an explicit book decision")
        if t == "buy":
            add_acq(asset, d, F(ev["quantity"]), F(ev["fiat"]) + F(ev.get("fee_fiat")), ev["id"])
        elif t in ("sell", "spend", "gift_out"):
            q = F(ev["quantity"]); p = F(ev["fiat"]); inc = F(ev.get("fee_fiat"))
            fq, fv = F(ev.get("fee_quantity")), F(ev.get("fee_fiat_value"))
            # CRYPTO22280: fee tokens are disposed at market value and that value is an allowable cost.
            add_disp(asset, d, q + fq, p + fv, inc + fv, ev["id"])
        elif t == "swap":
            given, received = ev.get("fiat"), ev.get("in_fiat")
            given = F(given) if given is not None else None
            received = F(received) if received is not None else None
            proceeds = received if received is not None else given
            cost_in = given if given is not None else received
            fee = F(ev.get("fee_fiat"))
            # CRYPTO22150: a swap fee is attributable to both assets; 50/50 is accepted as just and reasonable.
            add_disp(asset, d, F(ev["quantity"]), proceeds, fee / 2, ev["id"])
            add_acq(ev["in_asset"], d, F(ev["in_quantity"]), cost_in + fee / 2, ev["id"])
        elif t == "transfer":
            fq = F(ev.get("fee_quantity"))
            if fq:
                add_disp(asset, d, fq, F(ev["fee_fiat_value"]), Fraction(0), ev["id"] + ":fee")
        elif t in ("income", "airdrop"):
            kind = ev["subkind"]
            if kind not in MISC_KINDS:
                raise Refusal("income_kind_unsupported", ev["id"], kind)
            v = F(ev["fiat"])
            income.append({"event": ev["id"], "date": d.isoformat(), "tax_year": label(tax_year(d)), "subkind": kind, "quantity": qty(F(ev["quantity"])), "value": money(v), "_v": v, "_y": tax_year(d)})
            add_acq(asset, d, F(ev["quantity"]), v, ev["id"])
        else:
            raise ValueError(t)

    computations = []
    pools = {}
    for asset in sorted(set(acq) | set(disp)):
        A = {d: dict(v) for d, v in acq[asset].items()}
        D = {d: dict(v) | {"matches": []} for d, v in disp[asset].items()}
        for d in A:
            A[d]["rem"], A[d]["rem_cost"] = A[d]["qty"], A[d]["cost"]
        for d in D:
            D[d]["rem"] = D[d]["qty"]

        def match(dd, ad, rule):
            a, x = A[ad], D[dd]
            m = min(a["rem"], x["rem"])
            if m <= 0:
                return
            cost = a["rem_cost"] * m / a["rem"]
            a["rem"] -= m; a["rem_cost"] -= cost; x["rem"] -= m
            x["matches"].append({"rule": rule, "acquired": ad.isoformat(), "quantity": m, "cost": cost})

        for d in sorted(D):  # same day
            if d in A:
                match(d, d, "same_day")
        for d in sorted(D):  # 30 days after, earliest acquisition first, earliest disposal first
            for ad in sorted(A):
                if d < ad <= d + timedelta(days=30) and D[d]["rem"] > 0:
                    match(d, ad, "thirty_day")
        pool_q, pool_c = Fraction(0), Fraction(0)
        for d in sorted(set(A) | set(D)):
            if d in A and A[d]["rem"] > 0:
                pool_q += A[d]["rem"]; pool_c += A[d]["rem_cost"]
            if d in D and D[d]["rem"] > 0:
                m = D[d]["rem"]
                if m > pool_q:
                    raise Refusal("insufficient_holdings", ",".join(D[d]["events"]), f"{asset} disposal of {qty(m)} exceeds pool {qty(pool_q)}")
                cost = pool_c * m / pool_q
                pool_q -= m; pool_c -= cost; D[d]["rem"] = Fraction(0)
                D[d]["matches"].append({"rule": "s104", "acquired": None, "quantity": m, "cost": cost, "pool_before": [qty(pool_q + m), money(pool_c + cost)]})
        pools[asset] = {"quantity": qty(pool_q), "cost": money(pool_c)}
        for d in sorted(D):
            x = D[d]
            cost = sum((m["cost"] for m in x["matches"]), Fraction(0))
            gain = x["proceeds"] - cost - x["incidental"]
            computations.append({
                "asset": asset, "date": d.isoformat(), "tax_year": label(tax_year(d)), "_y": tax_year(d), "_d": d,
                "events": x["events"], "quantity": qty(x["qty"]), "proceeds": money(x["proceeds"]),
                "allowable_costs": money(cost + x["incidental"]), "incidental": money(x["incidental"]),
                "matches": [{"rule": m["rule"], "acquired": m["acquired"], "quantity": qty(m["quantity"]), "cost": money(m["cost"]), **({"pool_before": m["pool_before"]} if "pool_before" in m else {})} for m in x["matches"]],
                "gain": money(gain), "_g": gain, "_p": x["proceeds"], "_c": cost + x["incidental"],
            })

    years = {}
    for y in sorted({c["_y"] for c in computations} | {i["_y"] for i in income}):
        cs = [c for c in computations if c["_y"] == y]
        gains = sum((c["_g"] for c in cs if c["_g"] > 0), Fraction(0))
        losses = -sum((c["_g"] for c in cs if c["_g"] < 0), Fraction(0))
        proceeds = sum((c["_p"] for c in cs), Fraction(0))
        costs = sum((c["_c"] for c in cs), Fraction(0))
        net = gains - losses
        aea = F(AEA[y]) if y in AEA else None
        threshold = F("50000") if y >= 2023 else (4 * aea if aea is not None else None)
        entry = {
            "sa108_section": "cryptoassets boxes 13.1-13.5" if y >= 2024 else "other property, assets and gains boxes 14-17, 19",
            "disposals": len(cs),
            "proceeds": money(proceeds),
            "allowable_costs": money(costs),
            "gains": money(gains),
            "losses": money(losses),
            "net_gains": money(net),
            "annual_exempt_amount": money(aea) if aea is not None else None,
            "gains_above_aea": (money(max(net - aea, Fraction(0))) if aea is not None else None),
            "loss_available": money(max(-net, Fraction(0))),
            "boxes": {"count": len(cs), "proceeds": pounds_truncated(proceeds), "costs": pounds_truncated(costs), "gains": pounds_truncated(gains), "losses": pounds_truncated(losses)},
            "report_required": {
                "proceeds_over_threshold": (proceeds > threshold) if threshold is not None else None,
                "gains_over_aea": (gains > aea) if aea is not None else None,
                "loss_to_claim": losses > 0,
            },
            "misc_income_box17": money(sum((i["_v"] for i in income if i["_y"] == y), Fraction(0))),
        }
        if aea is None:
            entry["status"] = "aea_unknown_for_year"
        if y == 2024:
            before = [c for c in cs if c["_d"] < RATE_CHANGE]
            after = [c for c in cs if c["_d"] >= RATE_CHANGE]
            entry["rate_split_2024_25"] = {
                "gains_before_30_oct": money(sum((c["_g"] for c in before if c["_g"] > 0), Fraction(0))),
                "losses_before_30_oct": money(-sum((c["_g"] for c in before if c["_g"] < 0), Fraction(0))),
                "gains_from_30_oct": money(sum((c["_g"] for c in after if c["_g"] > 0), Fraction(0))),
                "losses_from_30_oct": money(-sum((c["_g"] for c in after if c["_g"] < 0), Fraction(0))),
            }
        years[label(y)] = entry

    strip = lambda d: {k: v for k, v in d.items() if not k.startswith("_")}
    return {"computations": [strip(c) for c in computations], "income": [strip(i) for i in income], "years": years, "pools": pools}


def evaluate(vector):
    try:
        return run(vector)
    except Refusal as r:
        return {"refusal": {"code": r.code, "event": r.event, "reason": r.reason}}


def check_hand(vid, expected, hand):
    """Compare with HMRC's published figures to the nearest pound (±1: HMRC rounds each step)."""
    comps = {(c["asset"], c["date"]): c for c in expected["computations"]}
    for key, want in hand.get("gains", {}).items():
        asset, d = key.split("@")
        got = pounds_nearest(F(comps[(asset, d)]["gain"]))
        assert abs(got - want) <= 1, (vid, key, got, want)
    for asset, (q, c) in hand.get("pools", {}).items():
        got = expected["pools"][asset]
        assert F(got["quantity"]) == F(q), (vid, asset, got, q)
        assert abs(pounds_nearest(F(got["cost"])) - c) <= 1, (vid, asset, got, c)
