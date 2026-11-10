"""Germany (de) reference calculator for docs/reference/tax-country-de.md.

Research only: an independent model of the spec used to compute the
expected values in tax-country-de-vectors.json. Not product code.
"""

from __future__ import annotations

from fractions import Fraction

from common import F, anniversary, local_date, local_dt, money, qty

TZ = "Europe/Berlin"


def freigrenze_23(year: int) -> Fraction:
    # §23 Abs. 3 Satz 5 EStG: weniger als 600 € (bis VZ 2023), 1.000 € (ab VZ 2024)
    return F("600") if year <= 2023 else F("1000")


FREIGRENZE_22_3 = F("256")  # §22 Nr. 3 Satz 2 EStG
LEISTUNG_KINDS = {"staking", "lending_interest", "routing_income", "airdrop_service", "interest"}
MINING_KINDS = {"mining"}


class Refusal(Exception):
    def __init__(self, code: str, event: str, reason: str):
        super().__init__(code)
        self.code, self.event, self.reason = code, event, reason


def tax_free(acquired, sold) -> bool:
    """§23 Abs. 1 Satz 1 Nr. 2: taxable while not more than one year has passed.

    The period starts the day after acquisition (§187 Abs. 1 BGB) and ends at
    the end of the anniversary date (§188 Abs. 2, 3 BGB via §108 AO), so a
    disposal is tax-free from the day after the anniversary.
    """
    return sold > anniversary(acquired)


class Wallet:
    def __init__(self):
        self.lots: list[dict] = []  # FIFO order by acquisition time
        self.total_cost = Fraction(0)  # used by the average method

    def quantity(self):
        return sum((lot["qty"] for lot in self.lots), Fraction(0))

    def add(self, lot):
        self.lots.append(lot)
        self.lots.sort(key=lambda l: (l["time"], l["seq"]))
        self.total_cost += lot["cost"]

    def take(self, amount: Fraction, method: str, event_id: str):
        """Consume `amount` FIFO by acquisition time; returns fragments with cost."""
        have = self.quantity()
        if amount > have:
            raise Refusal("insufficient_lots", event_id, f"needs {qty(amount)}, wallet holds {qty(have)}")
        avg = self.total_cost / have if have else Fraction(0)
        out = []
        left = amount
        while left > 0:
            lot = self.lots[0]
            take = min(left, lot["qty"])
            if method == "fifo":
                cost = lot["cost"] * take / lot["qty"]
            else:  # average: FIFO decides the holding period, the wallet average the value
                cost = avg * take
            frag = {"lot": lot["id"], "acquired": lot["date"], "time": lot["time"], "qty": take, "cost": cost}
            out.append(frag)
            if method == "fifo":
                lot["cost"] -= cost
            lot["qty"] -= take
            left -= take
            if lot["qty"] == 0:
                self.lots.pop(0)
        self.total_cost -= sum((f["cost"] for f in out), Fraction(0))
        if method == "fifo":
            self.total_cost = sum((l["cost"] for l in self.lots), Fraction(0))
        return out


def run(vector: dict) -> dict:
    policy = vector["policy"]
    method = policy["method"]
    assert policy["pool_scope"] == "wallet"
    wallets: dict[tuple[str, str], Wallet] = {}
    seq = 0
    disposals, income = [], []

    def w(name, asset):
        return wallets.setdefault((name, asset), Wallet())

    def acquire(ev, wallet, asset, amount, cost, time=None, date=None, lot_id=None):
        nonlocal seq
        seq += 1
        w(wallet, asset).add(
            {
                "id": lot_id or ev["id"],
                "qty": amount,
                "cost": cost,
                "time": time or local_dt(ev["time"], TZ),
                "date": date or local_date(ev["time"], TZ),
                "seq": seq,
            }
        )

    def dispose(ev, kind, wallet, asset, amount, proceeds, expenses):
        sold = local_date(ev["time"], TZ)
        frags = w(wallet, asset).take(amount, method, ev["id"])
        rows = []
        for f in frags:
            share = f["qty"] / amount
            p, e = proceeds * share, expenses * share
            rows.append(
                {
                    "lot": f["lot"],
                    "acquired": f["acquired"].isoformat(),
                    "quantity": f["qty"],
                    "cost": f["cost"],
                    "proceeds": p,
                    "expenses": e,
                    "gain": p - f["cost"] - e,
                    "taxable": not tax_free(f["acquired"], sold),
                }
            )
        disposals.append(
            {"event": ev["id"], "kind": kind, "asset": asset, "wallet": wallet, "date": sold.isoformat(), "year": sold.year, "fragments": rows}
        )

    events = sorted(vector["events"], key=lambda e: local_dt(e["time"], TZ))
    for ev in events:
        t = ev["type"]
        asset = ev.get("asset", "BTC")
        if t == "airdrop" and ev.get("subkind") not in ("airdrop_service",):
            raise Refusal("airdrop_classification_required", ev["id"], "airdrop without a reviewed Leistung/gift classification")
        if t == "hardfork":
            raise Refusal("hardfork_cost_split_unsupported", ev["id"], "Rn. 67: cost split by market-value ratio is not modelled")
        if t == "peg" and not policy.get("rail_policy"):
            raise Refusal("rail_policy_required", ev["id"], "BTC↔LBTC change needs an explicit book decision")
        if t == "gift_in" and not ev.get("predecessor_lots"):
            raise Refusal("gift_predecessor_data_required", ev["id"], "§23 Abs. 1 Satz 3: the donor's acquisition date and cost are required")
        if t == "buy":
            acquire(ev, ev["wallet"], asset, F(ev["quantity"]), F(ev["fiat"]) + F(ev.get("fee_fiat")))
        elif t in ("sell", "spend"):
            amount = F(ev["quantity"])
            fee_q = F(ev.get("fee_quantity"))
            fee_v = F(ev.get("fee_fiat_value"))
            if fee_q:
                # Rn. 54/60: the fee units are given for a service: a disposal at market value.
                # Fee first, as the engine pays a move's fee before its principal.
                dispose({**ev, "id": ev["id"] + ":fee"}, "fee", ev["wallet"], asset, fee_q, fee_v, Fraction(0))
            dispose(ev, t, ev["wallet"], asset, amount, F(ev["fiat"]), F(ev.get("fee_fiat")) + fee_v)
        elif t == "swap":
            amount = F(ev["quantity"])
            given = F(ev.get("fiat")) if ev.get("fiat") is not None else None
            received = F(ev.get("in_fiat")) if ev.get("in_fiat") is not None else None
            proceeds = received if received is not None else given  # Rn. 58
            cost_in = given if given is not None else received  # Rn. 59
            dispose(ev, "swap", ev["wallet"], asset, amount, proceeds, F(ev.get("fee_fiat")))
            acquire(ev, ev["wallet"], ev["in_asset"], F(ev["in_quantity"]), cost_in, lot_id=ev["id"] + ":in")
        elif t == "transfer":
            amount = F(ev["quantity"])
            fee_q = F(ev.get("fee_quantity"))
            if fee_q:
                dispose({**ev, "id": ev["id"] + ":fee"}, "fee", ev["wallet"], asset, fee_q, F(ev["fee_fiat_value"]), Fraction(0))
            frags = w(ev["wallet"], asset).take(amount, method, ev["id"])
            for f in frags:
                acquire(ev, ev["to_wallet"], asset, f["qty"], f["cost"], time=f["time"], date=f["acquired"], lot_id=f["lot"])
        elif t in ("income", "airdrop"):
            kind = ev["subkind"]
            value = F(ev["fiat"])
            d = local_date(ev["time"], TZ)
            section = "p22_3" if kind in LEISTUNG_KINDS else ("mining_classification_required" if kind in MINING_KINDS else None)
            if section is None:
                raise Refusal("income_kind_unsupported", ev["id"], kind)
            income.append({"event": ev["id"], "date": d.isoformat(), "year": d.year, "subkind": kind, "quantity": F(ev["quantity"]), "value": value, "section": section})
            acquire(ev, ev["wallet"], asset, F(ev["quantity"]), value)
        else:
            raise ValueError(t)

    years = {}
    for d in disposals:
        y = years.setdefault(d["year"], {"tp": Fraction(0), "tc": Fraction(0), "te": Fraction(0), "tg": Fraction(0), "eg": Fraction(0), "p22": Fraction(0), "mining": Fraction(0)})
        for f in d["fragments"]:
            if f["taxable"]:
                y["tp"] += f["proceeds"]; y["tc"] += f["cost"]; y["te"] += f["expenses"]; y["tg"] += f["gain"]
            else:
                y["eg"] += f["gain"]
    for i in income:
        y = years.setdefault(i["year"], {"tp": Fraction(0), "tc": Fraction(0), "te": Fraction(0), "tg": Fraction(0), "eg": Fraction(0), "p22": Fraction(0), "mining": Fraction(0)})
        if i["section"] == "p22_3":
            y["p22"] += i["value"]
        else:
            y["mining"] += i["value"]

    out_years = {}
    for year, y in sorted(years.items()):
        limit = freigrenze_23(year)
        net = y["tg"]
        if net >= limit:
            res, amount = "taxable", net
        elif net >= 0:
            res, amount = "exempt_freigrenze" if net > 0 else "none", Fraction(0)
        else:
            res, amount = "loss", net
        p22 = y["p22"]
        p22_res = "taxable" if p22 >= FREIGRENZE_22_3 else ("exempt_freigrenze" if p22 > 0 else "none")
        out_years[str(year)] = {
            "p23_taxable_proceeds": money(y["tp"]),
            "p23_taxable_cost": money(y["tc"]),
            "p23_taxable_expenses": money(y["te"]),
            "p23_net_gain": money(net),
            "p23_freigrenze": money(limit),
            "p23_result": res,
            "p23_amount": money(amount),
            "exempt_gain_outside_period": money(y["eg"]),
            "p22_3_total": money(p22),
            "p22_3_result": p22_res,
            "p22_3_amount": money(p22 if p22_res == "taxable" else Fraction(0)),
            "mining_classification_required_total": money(y["mining"]),
        }

    def render_frag(f):
        return {
            "lot": f["lot"], "acquired": f["acquired"], "quantity": qty(f["quantity"]), "cost": money(f["cost"]),
            "proceeds": money(f["proceeds"]), "expenses": money(f["expenses"]), "gain": money(f["gain"]), "taxable": f["taxable"],
        }

    holdings = {}
    for (wallet, asset), wl in sorted(wallets.items()):
        if wl.quantity():
            holdings.setdefault(wallet, {})[asset] = {"quantity": qty(wl.quantity()), "cost": money(wl.total_cost)}
    return {
        "disposals": [
            {k: v for k, v in d.items() if k not in ("fragments", "year")} | {"fragments": [render_frag(f) for f in d["fragments"]]}
            for d in disposals
        ],
        "income": [{k: (money(v) if k == "value" else qty(v) if k == "quantity" else v) for k, v in i.items() if k != "year"} for i in income],
        "years": out_years,
        "holdings": holdings,
    }


def evaluate(vector):
    try:
        return run(vector)
    except Refusal as r:
        return {"refusal": {"code": r.code, "event": r.event, "reason": r.reason}}
