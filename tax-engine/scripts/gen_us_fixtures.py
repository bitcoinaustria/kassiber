#!/usr/bin/env python3
"""Generate RP2-oracle fixtures for the engine's United States country.

RP2's US country plugin (`rp2.plugin.country.us`) is the oracle for the lots
and gains of every lot method (fifo, lifo, hifo, lofo) under both of its
application methods:

* ``universal`` cases run ``rp2.tax_engine.compute_tax`` over seeded random
  histories (the generic generator's gated flows: buys, earn receipts,
  sales with and without fees, fee-only rows and moves with fees) and expect
  the engine's ``compute`` with ``{"kind": "us"}`` and the global pool scope
  to give RP2's response.
* ``per_wallet`` cases run ``rp2.per_wallet_tax_engine.compute_tax_per_wallet``
  over histories RP2 computes correctly, and expect the engine's wallet pool
  scope to consume the same lots: RP2 runs a transfer analysis and then one
  pass per wallet, so a wallet that disposes after sending units away can
  dispose of a lot it already sent (docs/reference/tax-country-us.md,
  US-RP2-PER-WALLET), and RP2 refuses fee-bearing moves. These histories
  have source wallets that buy, earn and sell and then send each of their
  outgoing moves after their last disposal, fee-free, and sink wallets that
  receive one move and then only sell.

Two differences are recorded rather than copied:

* Holding period: RP2's US plugin calls a lot long-term once whole elapsed
  days reach 365; the IRS counts calendar dates (more than one year, the
  anniversary itself short-term, 29 February's anniversary 28 February), in
  UTC here. Each gain/loss carries ``long_term`` by the IRS rule and
  ``rp2_long_term`` where RP2 differs; a case with a difference drops RP2's
  yearly summary (``yearly_differs``), whose long/short split follows it.
  The generator asserts that every difference is an anniversary-window lot.
* Per-wallet lots: RP2 hands a destination wallet a new lot per moved
  portion (rows -1, -2, ...); a gain/loss names the original lot here, and
  its basis is compared at RP2's 13 places because the new lot rounds its
  basis once more.

Run it from a scratch directory, as gen_engine_fixtures.py:

    cd "$(mktemp -d)" && uv run --locked --project <repo> \\
        python <repo>/tax-engine/scripts/gen_us_fixtures.py

It writes tax-engine/core/tests/data/us_fixtures.json; the output depends
only on the seed and the pinned RP2.
"""

from __future__ import annotations

import argparse
import calendar
import decimal
import json
import random
import tempfile
from datetime import date, datetime, timedelta, timezone
from decimal import ROUND_HALF_EVEN, Decimal
from importlib import import_module, util
from pathlib import Path
from typing import Any

from rp2.in_transaction import InTransaction
from rp2.input_data import InputData
from rp2.per_wallet_tax_engine import compute_tax_per_wallet
from rp2.rp2_decimal import RP2Decimal
from rp2.tax_engine import compute_tax
from rp2.transaction_set import TransactionSet

US = import_module("rp2.plugin.country.us").US

REPO = Path(__file__).resolve().parents[2]
OUTPUT = REPO / "tax-engine" / "core" / "tests" / "data" / "us_fixtures.json"
SEED = 20261107
UNIVERSAL_HISTORIES = 240
PER_WALLET_HISTORIES = 240
METHODS = ("fifo", "lifo", "hifo", "lofo")


def _load_generic():
    path = Path(__file__).resolve().parent / "gen_engine_fixtures.py"
    spec = util.spec_from_file_location("gen_engine_fixtures", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load {path}")
    module = util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


G = _load_generic()
V1 = G.V1_REQUEST


def anniversary(d: date) -> date:
    y = d.year + 1
    return date(y, d.month, min(d.day, calendar.monthrange(y, d.month)[1]))


def irs_long_term(acquired: datetime, disposed: datetime) -> bool:
    a = acquired.astimezone(timezone.utc).date()
    d = disposed.astimezone(timezone.utc).date()
    return d > anniversary(a)


def build(rows: list[dict[str, Any]], wallets: list[str], scratch: Path):
    """RP2's configuration, sets and transactions for `rows`, plus the
    engine entries (with RP2's text where a message embeds it)."""
    conf = G.rp2_configuration(US(), [G.ASSET], wallets, G.HOLDER, scratch)
    sets = {kind: TransactionSet(conf, kind.upper(), G.ASSET) for kind in G.RP2_CLASSES}
    entries, by_row = [], {}
    for row_number, row in enumerate(rows, 1):
        kind, rp2_kwargs = G.constructor_arguments(row, row_number, RP2Decimal)
        _, native_kwargs = G.constructor_arguments(row, row_number, Decimal)
        tx = G.RP2_CLASSES[kind](configuration=conf, **rp2_kwargs)
        entry = G.NATIVE_BUILDERS[kind](**native_kwargs)
        G.check_timestamp(f"row {row_number}", entry["ts"], tx.timestamp)
        entries.append(V1.with_text(entry, str(tx)))
        sets[kind].add_entry(tx)
        by_row[row_number] = tx
    data = InputData(
        asset=G.ASSET,
        unfiltered_in_transaction_set=sets["in"],
        unfiltered_out_transaction_set=sets["out"],
        unfiltered_intra_transaction_set=sets["intra"],
    )
    return conf, data, entries, by_row


def origin(lot: InTransaction) -> InTransaction:
    while lot.from_lot is not None:
        lot = lot.from_lot
    return lot


def gain_loss_documents(computed, by_row) -> tuple[list[dict[str, Any]], int]:
    """RP2's gain/losses with the original lot row and the IRS term; the
    number of rows whose term RP2 gives otherwise."""
    kinds = {"InTransaction": "in", "OutTransaction": "out", "IntraTransaction": "intra"}
    out, differs = [], 0
    for gl in computed.gain_loss_set:
        lot = origin(gl.acquired_lot) if gl.acquired_lot is not None else None
        doc = {
            "event": {"kind": kinds[type(gl.taxable_event).__name__], "row": gl.taxable_event.row},
            "lot": lot.row if lot is not None else None,
            "crypto_amount": G.enc(gl.crypto_amount),
            "fiat_cost_basis": G.enc(gl.fiat_cost_basis),
            "proceeds": G.enc(gl.taxable_event_fiat_amount_with_fee_fraction),
            "fiat_gain": G.enc(gl.fiat_gain),
            "long_term": False,
        }
        rp2 = gl.is_long_term_capital_gains()
        if lot is not None:
            irs = irs_long_term(lot.timestamp, gl.taxable_event.timestamp)
            doc["long_term"] = irs
            if irs != rp2:
                held = (gl.taxable_event.timestamp - lot.timestamp).days
                if not 364 <= held <= 366:
                    raise SystemExit(f"term differs outside the anniversary window: {held} days")
                doc["rp2_long_term"] = rp2
                differs += 1
        out.append(doc)
    return out, differs


class Cases:
    def __init__(self):
        self.cases: list[dict[str, Any]] = []
        self.stats: dict[str, int] = {}

    def add(self, name, application, request, expected):
        self.cases.append({"name": name, "application": application, "request": request, "expected": expected})
        key = f"{application}:{'ok' if expected['ok'] else 'error'}"
        self.stats[key] = self.stats.get(key, 0) + 1
        if expected.get("ok") and expected.get("yearly_differs"):
            self.stats[f"{application}:term_differs"] = self.stats.get(f"{application}:term_differs", 0) + 1


def run(name, application, method, wallets, rows, cases, scratch):
    try:
        conf, data, entries, by_row = build(rows, wallets, scratch)
    except Exception:  # noqa: BLE001 - a constructor refusal is the generic suite's
        return
    request = V1.compute_request(method=method, assets={G.ASSET: entries}, country={"kind": "us"})
    request["pool_scope"] = "global" if application == "universal" else "wallet"
    if not any(r["kind"] == "in" for r in rows):
        return
    try:
        engine = G.accounting_engine(method)
        if application == "universal":
            computed = compute_tax(conf, engine, data)
        else:
            semantics = import_module(f"rp2.plugin.accounting_method.{method}").AccountingMethod()
            computed = compute_tax_per_wallet(conf, engine, semantics, data)
    except Exception as exc:  # noqa: BLE001 - every RP2 failure is a fixture
        cases.add(name, application, request, G.error_document(exc))
        return
    # The engine reads `text` only for messages that embed it.
    for entry in request["assets"][0]["entries"]:
        entry.pop("text", None)
    gain_losses, differs = gain_loss_documents(computed, by_row)
    document = G.asset_document(computed)
    expected = {"ok": True, "gain_losses": gain_losses, "yearly_differs": differs > 0}
    if application == "universal":
        expected["asset"] = document
    else:
        # Count only histories whose wallet scope takes other lots than the
        # universal pass would, so the per-wallet cases are not vacuous.
        conf2, data2, _, by_row2 = build(rows, wallets, scratch)
        universal, _ = gain_loss_documents(compute_tax(conf2, G.accounting_engine(method), data2), by_row2)
        key = lambda gl: (gl["event"]["row"], gl["lot"] or 0, gl["crypto_amount"])
        expected["scope_matters"] = sorted(map(key, universal)) != sorted(map(key, gain_losses))
    cases.add(name, application, request, expected)
    if expected.get("scope_matters"):
        cases.stats["per_wallet:scope_matters"] = cases.stats.get("per_wallet:scope_matters", 0) + 1


def universal_history(index: int, seed: int) -> tuple[str, list[str], list[dict]]:
    rng = random.Random(seed * 1000 + index)
    method = METHODS[index % len(METHODS)]
    wallets = rng.choice([["A"], ["A", "B"], ["A", "B", "C"]])
    instants = G.instant_pool(rng, 365)
    # Anniversaries of the first instant at midnight UTC and a 29 February.
    start = min(instants)
    instants += [start + timedelta(days=d, hours=h) for d in (364, 365, 366) for h in (-1, 1)]
    # Universal identification ends with 2024 (US-POOL-2025): the engine
    # refuses later disposals on the global scope, which RP2 does not know.
    instants = sorted(t for t in set(instants) if t < datetime(2025, 1, 1, tzinfo=timezone.utc))
    rows = G.flow_rows(rng, wallets, instants, (100, 0, 0), gated=True)
    rows = [r for r in rows if r["timestamp"] < "2025-01-01"]
    return method, wallets, rows


def per_wallet_history(index: int, seed: int) -> tuple[str, list[str], list[dict]]:
    """Sources buy, earn and sell, then send; each sink receives one
    fee-free move and only sells afterwards."""
    rng = random.Random(seed * 7919 + index)
    method = METHODS[index % len(METHODS)]
    sources = ["S1", "S2"][: rng.randint(1, 2)]
    sinks = ["K1", "K2", "K3"][: rng.randint(1, 3)]
    start = datetime(rng.choice([2022, 2023, 2024]), rng.randint(1, 12), rng.randint(1, 28), 12, tzinfo=timezone.utc)
    clock = [start]

    def tick():
        clock[0] += timedelta(days=rng.choice([1, 3, 20, 90, 200, 364, 365, 366]), hours=rng.choice([0, 1, 13]))
        return clock[0].strftime("%Y-%m-%dT%H:%M:%SZ")

    rows, n = [], 0
    balance = {w: 0 for w in sources + sinks}
    for source in sources:
        for _ in range(rng.randint(2, 6)):
            n += 1
            msat = rng.randint(1, 10**11)
            spot = G.random_price(rng)
            amount = G.msat_to_btc(msat)
            type_ = rng.choice(["BUY"] * 4 + ["STAKING", "MINING", "INCOME"])
            rows.append(G.buy(f"in{n}", tick(), source, spot, amount, G.random_fiat(rng, amount, spot), type_))
            balance[source] += msat
            if rng.random() < 0.4 and balance[source] > 1:
                n += 1
                total = rng.randint(1, balance[source] // 2 or 1)
                fee = rng.randint(1, min(total - 1, 10**6)) if total > 1 and rng.random() < 0.4 else 0
                spot = G.random_price(rng)
                amount = G.msat_to_btc(total - fee)
                rows.append(G.sell(f"out{n}", tick(), source, amount, spot, G.random_fiat(rng, amount, spot), fee=G.msat_to_btc(fee)))
                balance[source] -= total
    # Each source sends after its last disposal; each sink gets one move.
    for sink in sinks:
        source = rng.choice(sources)
        if balance[source] <= 0:
            continue
        n += 1
        sent = rng.randint(1, balance[source])
        rows.append(G.move(f"mv{n}", tick(), source, sink, G.msat_to_btc(sent), G.msat_to_btc(sent), "0"))
        balance[source] -= sent
        balance[sink] += sent
    for sink in sinks:
        while balance[sink] > 0 and rng.random() < 0.8:
            n += 1
            total = rng.randint(1, balance[sink])
            spot = G.random_price(rng)
            amount = G.msat_to_btc(total)
            rows.append(G.sell(f"out{n}", tick(), sink, amount, spot, G.random_fiat(rng, amount, spot)))
            balance[sink] -= total
    rows.sort(key=lambda r: r["timestamp"])
    return method, sources + sinks, rows


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument("--universal", type=int, default=UNIVERSAL_HISTORIES)
    parser.add_argument("--per-wallet", type=int, default=PER_WALLET_HISTORIES)
    args = parser.parse_args()
    context = decimal.getcontext()
    if context.prec != 32 or context.rounding != ROUND_HALF_EVEN:
        raise SystemExit(f"expected RP2's 32-digit context on the main thread, found {context}")
    cases = Cases()
    with tempfile.TemporaryDirectory(prefix="kassiber-us-fixtures-") as scratch:
        scratch_path = Path(scratch)
        for index in range(args.universal):
            method, wallets, rows = universal_history(index, args.seed)
            run(f"U{index:03d}_{method}", "universal", method, wallets, rows, cases, scratch_path)
        for index in range(args.per_wallet):
            method, wallets, rows = per_wallet_history(index, args.seed)
            run(f"W{index:03d}_{method}", "per_wallet", method, wallets, rows, cases, scratch_path)
    document = {
        "schema": "kassiber-tax-engine/us-fixtures/v1",
        "generator": "tax-engine/scripts/gen_us_fixtures.py",
        "seed": args.seed,
        "notes": [
            "Each case's expected value is RP2's US plugin, with the IRS holding period and per-wallet lots named by their origin (see the generator).",
            "Decimals use the canonical form [-]<digits>E<exponent>; exponents are significant.",
        ],
        "stats": dict(sorted(cases.stats.items())),
        "cases": cases.cases,
    }
    text = json.dumps(document, ensure_ascii=False, separators=(",", ":")) + "\n"
    args.output.write_text(text, encoding="utf-8")
    print(f"wrote {len(cases.cases)} cases ({len(text.encode())} bytes) to {args.output}: {document['stats']}")


if __name__ == "__main__":
    main()
