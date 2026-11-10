"""United States (us) reference calculator for docs/reference/tax-country-us.md.

Research only: an independent model of the spec used to compute the expected
values in tax-country-us-vectors.json. Not product code. Exact arithmetic
with Fraction; amounts are rounded only when rendered.

Model (see the spec's rule ids):

- US-HOLD-1Y: long-term when held more than one year, by UTC calendar date:
  the period starts the day after acquisition and includes the disposal
  date, so long-term iff disposal date > anniversary(acquisition date), and
  29 February's anniversary is 28 February.
- US-POOL-WALLET / US-POOL-GLOBAL: lots are taken from the disposing wallet
  only (wallet scope), or from every wallet (global scope, universal
  application, allowed for disposals before 2025-01-01 only).
- US-METHOD-*: FIFO by acquisition time, LIFO the reverse, HIFO/LOFO by
  unit cost then acquisition time; ties by input order.
- US-TRANSFER: a move between own wallets is not a disposal; its fee units
  leave first by the method, then the moved units, which keep their basis
  and acquisition date.
- US-TRANSFER-FEE: a move's fee units realize no gain or loss: their basis
  leaves the book with them (Kassiber's composition of fee rows; open
  question 2). They are listed as fees, not Form 8949 rows.
- US-FEE-SALE: a sale's fee units are part of the disposal and its proceeds
  are the sale's USD amount (the engine's single-row form; totals equal the
  final regulations' split into a sale and a fee disposition).
- US-INCOME: staking, mining, interest and service airdrops are ordinary
  income at USD fair market value on receipt, and that value is the basis.
- US-8949: rows by term; box C/F before 2025, I/L (digital assets without
  Form 1099-DA) for 2025 and 2026; later years have no table: unknown.
"""

from __future__ import annotations

import calendar
from datetime import date, datetime, timezone
from fractions import Fraction

from common import F, money, qty

INCOME_KINDS = {"staking", "mining", "interest", "income", "airdrop_service"}
FORM_YEARS = range(2014, 2027)  # Notice 2014-21 onwards; the 2026 forms are the latest tables


class Refusal(Exception):
    def __init__(self, code: str, event: str, reason: str):
        super().__init__(code)
        self.code, self.event, self.reason = code, event, reason


def instant(text: str) -> datetime:
    dt = datetime.fromisoformat(text)
    if dt.tzinfo is None:
        raise ValueError(f"timestamp without offset: {text}")
    return dt.astimezone(timezone.utc)


def utc_date(text: str) -> date:
    return instant(text).date()


def anniversary(d: date) -> date:
    y = d.year + 1
    return date(y, d.month, min(d.day, calendar.monthrange(y, d.month)[1]))


def long_term(acquired: date, disposed: date) -> bool:
    return disposed > anniversary(acquired)


def boxes(year: int):
    if year not in FORM_YEARS:
        return None
    return ("C", "F") if year < 2025 else ("I", "L")


class Book:
    def __init__(self, method: str, scope: str):
        self.method = method
        self.scope = scope
        self.portions: dict[str, list[dict]] = {}
        self.seq = 0

    def add(self, wallet, lot):
        key = wallet if self.scope == "wallet" else "*"
        bucket = self.portions.setdefault(key, [])
        for p in bucket:
            if p["id"] == lot["id"]:
                p["qty"] += lot["qty"]
                p["cost"] += lot["cost"]
                return
        bucket.append(dict(lot))

    def rank(self, p):
        unit = p["unit"]
        if self.method == "fifo":
            return (p["time"], p["seq"])
        if self.method == "lifo":
            return (-p["time"].timestamp(), -p["seq"])
        if self.method == "hifo":
            return (-unit, p["time"], p["seq"])
        if self.method == "lofo":
            return (unit, p["time"], p["seq"])
        raise ValueError(self.method)

    def take(self, wallet, amount: Fraction, when: datetime, event_id: str):
        key = wallet if self.scope == "wallet" else "*"
        bucket = [p for p in self.portions.get(key, []) if p["qty"] > 0 and p["time"] <= when]
        have = sum((p["qty"] for p in bucket), Fraction(0))
        if amount > have:
            raise Refusal("insufficient_lots", event_id, f"needs {qty(amount)}, {key} holds {qty(have)}")
        out, left = [], amount
        for p in sorted(bucket, key=self.rank):
            if left == 0:
                break
            t = min(left, p["qty"])
            cost = p["unit"] * t
            out.append({"lot": p["id"], "acquired": p["date"], "time": p["time"], "seq": p["seq"], "unit": p["unit"], "qty": t, "cost": cost})
            p["qty"] -= t
            p["cost"] -= cost
            left -= t
        return out

    def holdings(self):
        out = {}
        for key, bucket in sorted(self.portions.items()):
            q = sum((p["qty"] for p in bucket), Fraction(0))
            if q:
                out[key] = {"BTC": {"quantity": qty(q), "cost": money(sum((p["cost"] for p in bucket), Fraction(0)))}}
        return out


def run(vector: dict) -> dict:
    policy = vector["policy"]
    method, scope = policy["method"], policy["pool_scope"]
    if method not in ("fifo", "lifo", "hifo", "lofo"):
        raise Refusal("method_unsupported", "-", f"{method} is not a US method")
    book = Book(method, scope)
    disposals, income, fees = [], [], []

    def acquire(ev, wallet, amount, cost, lot_id=None):
        book.seq += 1
        when = instant(ev["time"])
        book.add(wallet, {"id": lot_id or ev["id"], "qty": amount, "cost": cost, "unit": cost / amount, "time": when, "date": when.date(), "seq": book.seq})

    def dispose(ev, kind, wallet, amount, proceeds, event_id=None):
        event_id = event_id or ev["id"]
        when = instant(ev["time"])
        if scope == "global" and when.date() >= date(2025, 1, 1):
            raise Refusal("wallet_scope_required", event_id, "Treas. Reg. § 1.1012-1(j): from 2025 basis is identified per wallet or account")
        frags = book.take(wallet, amount, when, event_id)
        rows = []
        for f in frags:
            p = proceeds * f["qty"] / amount
            rows.append({
                "lot": f["lot"], "acquired": f["acquired"].isoformat(), "quantity": f["qty"], "cost": f["cost"],
                "proceeds": p, "gain": p - f["cost"], "term": "long" if long_term(f["acquired"], when.date()) else "short",
            })
        disposals.append({"event": event_id, "kind": kind, "asset": "BTC", "wallet": wallet, "date": when.date().isoformat(), "year": when.year, "fragments": rows})

    rank = {"buy": 0, "income": 0, "airdrop": 0, "gift_in": 0, "transfer": 1, "sell": 2, "spend": 2, "gift_out": 2}
    order = {ev["id"]: i for i, ev in enumerate(vector["events"])}
    events = sorted(vector["events"], key=lambda e: (instant(e["time"]), rank[e["type"]], order[e["id"]]))
    for ev in events:
        t = ev["type"]
        if ev.get("asset", "BTC") != "BTC":
            raise Refusal("asset_unsupported", ev["id"], "the US vectors cover BTC only")
        if t == "airdrop" and ev.get("subkind") != "airdrop_service":
            raise Refusal("airdrop_classification_required", ev["id"], "Rev. Rul. 2019-24: income only with dominion and control; the receipt's nature must be reviewed")
        if t == "gift_in":
            raise Refusal("gift_basis_required", ev["id"], "§ 1015: the donor's basis and holding period are required (Publication 551)")
        if t == "gift_out":
            raise Refusal("gift_out_unsupported", ev["id"], "a gift is not a sale; gift-tax reporting is out of scope")
        if t == "buy":
            acquire(ev, ev["wallet"], F(ev["quantity"]), F(ev["fiat"]))
        elif t in ("sell", "spend"):
            amount = F(ev["quantity"]) + F(ev.get("fee_quantity"))
            dispose(ev, t, ev["wallet"], amount, F(ev["fiat"]))
        elif t == "transfer":
            fee_q = F(ev.get("fee_quantity"))
            if fee_q:
                if ev.get("fee_fiat_value") is None:
                    raise Refusal("fee_unpriced", ev["id"], "a move's fee needs its USD value")
                when = instant(ev["time"])
                if scope == "global" and when.date() >= date(2025, 1, 1):
                    raise Refusal("wallet_scope_required", ev["id"], "Treas. Reg. § 1.1012-1(j): from 2025 basis is identified per wallet or account")
                taken = book.take(ev["wallet"], fee_q, when, ev["id"])
                fees.append({"event": ev["id"], "wallet": ev["wallet"], "date": when.date().isoformat(), "year": when.year,
                             "quantity": fee_q, "cost": sum((f["cost"] for f in taken), Fraction(0)), "value": F(ev["fee_fiat_value"])})
            if ev["to_wallet"] != ev["wallet"]:
                when = instant(ev["time"])
                for f in book.take(ev["wallet"], F(ev["quantity"]), when, ev["id"]):
                    book.add(ev["to_wallet"], {"id": f["lot"], "qty": f["qty"], "cost": f["cost"], "unit": f["unit"], "time": f["time"], "date": f["acquired"], "seq": f["seq"]})
        elif t in ("income", "airdrop"):
            kind = ev["subkind"]
            if kind not in INCOME_KINDS:
                raise Refusal("income_kind_unsupported", ev["id"], kind)
            value = F(ev["fiat"])
            d = utc_date(ev["time"])
            income.append({"event": ev["id"], "date": d.isoformat(), "year": d.year, "subkind": kind, "quantity": F(ev["quantity"]), "value": value})
            acquire(ev, ev["wallet"], F(ev["quantity"]), value)
        else:
            raise ValueError(t)

    years = {}

    def year(y):
        return years.setdefault(y, {"sp": Fraction(0), "sc": Fraction(0), "lp": Fraction(0), "lc": Fraction(0), "inc": {}, "rows": 0})

    for d in disposals:
        y = year(d["year"])
        for f in d["fragments"]:
            y["rows"] += 1
            if f["term"] == "short":
                y["sp"] += f["proceeds"]; y["sc"] += f["cost"]
            else:
                y["lp"] += f["proceeds"]; y["lc"] += f["cost"]
    for f in fees:
        y = year(f["year"])
        y["fees"] = y.get("fees", Fraction(0)) + f["cost"]
    for i in income:
        y = year(i["year"])
        y["inc"][i["subkind"]] = y["inc"].get(i["subkind"], Fraction(0)) + i["value"]

    out_years = {}
    for y, v in sorted(years.items()):
        bx = boxes(y)
        short_gain, long_gain = v["sp"] - v["sc"], v["lp"] - v["lc"]
        out_years[str(y)] = {
            "status": "supported" if bx else "unknown",
            "form_8949_short_box": bx[0] if bx else None,
            "form_8949_long_box": bx[1] if bx else None,
            "short_proceeds": money(v["sp"]),
            "short_cost": money(v["sc"]),
            "short_gain": money(short_gain),
            "long_proceeds": money(v["lp"]),
            "long_cost": money(v["lc"]),
            "long_gain": money(long_gain),
            "schedule_d_line_3": money(short_gain) if bx else None,
            "schedule_d_line_10": money(long_gain) if bx else None,
            "ordinary_income": money(sum(v["inc"].values(), Fraction(0))),
            "income_by_kind": {k: money(a) for k, a in sorted(v["inc"].items())},
            "network_fee_cost": money(v.get("fees", Fraction(0))),
            "digital_asset_question": "yes" if (v["rows"] or v["inc"] or v.get("fees")) else "no",
        }

    def render(f):
        return {"lot": f["lot"], "acquired": f["acquired"], "quantity": qty(f["quantity"]), "cost": money(f["cost"]),
                "proceeds": money(f["proceeds"]), "gain": money(f["gain"]), "term": f["term"]}

    return {
        "disposals": [{k: v for k, v in d.items() if k not in ("fragments", "year")} | {"fragments": [render(f) for f in d["fragments"]]} for d in disposals],
        "income": [{"event": i["event"], "date": i["date"], "subkind": i["subkind"], "quantity": qty(i["quantity"]), "value": money(i["value"])} for i in income],
        "fees": [{"event": f["event"], "wallet": f["wallet"], "date": f["date"], "quantity": qty(f["quantity"]), "cost": money(f["cost"]), "value": money(f["value"])} for f in fees],
        "years": out_years,
        "holdings": book.holdings(),
    }


def evaluate(vector):
    try:
        return run(vector)
    except Refusal as r:
        return {"refusal": {"code": r.code, "event": r.event, "reason": r.reason}}


def check_hand(vector_id, expected, hand):
    """Hand-computed figures (from the IRS examples the vector cites) must
    equal the model's."""
    for path, value in hand.items():
        node = expected
        for part in path.split("."):
            node = node[int(part)] if isinstance(node, list) else node[part]
        if node != value:
            raise AssertionError(f"{vector_id}: {path} is {node!r}, hand check says {value!r}")
