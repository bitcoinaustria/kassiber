"""Finalized tax projections for the ledger goldens and their random half (plan 20).

Three things live here:

- the builders the goldens use: imported rows compiled through the same
  pre-tax custody stages as production (``finalized``), and projections built
  directly (``direct``) for shapes the custody fixtures cannot reach easily
  but a finalized projection may have (design §3.5);
- ``random_inputs``, a seeded random generator for generic and Austrian
  books. Each dimension below is switched on by the seed with a stated
  probability, and every book it gives satisfies §3.5;
- ``corpus``, the named cases ``tax-engine/scripts/gen_ledger_fixtures.py``
  stores with the oracle's outcome. Most stay within §3.5; the LD-5 cases
  each have one shape a finalized projection cannot have, which production's
  request builder refuses.

Nothing here runs an engine.
"""

from __future__ import annotations

import hashlib
import json
import random
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any, Callable, Iterable, Mapping, Optional, Sequence

from kassiber.core import pricing
from kassiber.core.custody_tax_projection import FinalizedTaxProjection
from kassiber.core.engines.base import TaxEngineLedgerInputs, TaxEngineLedgerResult
from kassiber.core.transfer_chronology import ChainOrderEvidence
from tests.custody_tax_helpers import finalized_tax_inputs

SEED = 20261002
GENERIC_METHODS = ("FIFO", "LIFO", "HIFO", "LOFO", "MOVING_AVERAGE")
AT_METHODS = ("moving_average_at", "fifo", "hifo")
# The other methods an Austrian profile may pick: RP2's plain moving average
# and the generic lot methods ``build_austrian_policy`` allows too.
AT_EXTRA_METHODS = ("moving_average", "lifo", "lofo")
# The generic holding periods the goldens and ``terms`` books use besides
# the default 365 days.
LONG_TERM_DAYS = (1, 30, 730)

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
_LIQUID_ASSETS = {"LBTC", "L-BTC"}


def profile(country: str, method: str, *, long_term_days: int = 365, **extra: Any) -> dict[str, Any]:
    """A profile as the stores hold it."""
    return {
        "id": "profile-1",
        "workspace_id": "ws-1",
        "label": "Default",
        "fiat_currency": "EUR",
        "tax_country": country,
        "tax_long_term_days": long_term_days,
        "gains_algorithm": method,
        **extra,
    }


def row(
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
    **fields: Any,
) -> dict[str, Any]:
    """An imported exchange row (no chain proof), as the stores hold it.

    ``fields`` sets or adds any other column (``note``, ``pricing_quality``,
    ``at_regime_override``, ...).
    """
    ref = WALLET_REFS[wallet]
    liquid = asset in _LIQUID_ASSETS
    item = {
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
    item.update(fields)
    return item


def pair(
    pair_id: str,
    out_id: str,
    in_id: str,
    *,
    kind: str = "manual",
    policy: str = "carrying-value",
) -> dict[str, Any]:
    """A reviewed pair record between two imported rows."""
    return {"id": pair_id, "out_transaction_id": out_id, "in_transaction_id": in_id, "kind": kind, "policy": policy}


def finalized(
    book_profile: Mapping[str, Any],
    rows: Sequence[Mapping[str, Any]],
    pairs: Sequence[Mapping[str, Any]] = (),
    *,
    payouts: Sequence[Mapping[str, Any]] = (),
    wallet_refs: Mapping[str, Mapping[str, Any]] = WALLET_REFS,
) -> TaxEngineLedgerInputs:
    """Compile imported rows through the pre-tax custody stages."""
    return finalized_tax_inputs(
        book_profile,
        rows=[dict(item) for item in rows],
        wallet_refs_by_id=wallet_refs,
        manual_pair_records=[dict(item) for item in pairs],
        direct_payout_records=[dict(item) for item in payouts],
    )


def ledger_history() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Buys, earn kinds, sales with fees, and a fee-bearing reviewed move."""
    rows = [
        row("buy-1", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        row("buy-2", "C", "inbound", 50_000_000_000, "2024-03-01T00:00:00Z", rate="50000.5", kind="buy"),
        row("mined", "A", "inbound", 1_234_567_890, "2024-04-01T00:00:00Z", rate="60000", kind="mining"),
        row("staked", "A", "inbound", 987_654_321, "2024-04-02T00:00:00Z", rate="60100.25", kind="staking"),
        row("sell-1", "C", "outbound", 30_000_000_000, "2024-06-01T00:00:00Z", fee=10_000_000, rate="65000", kind="sell"),
        row("move-out", "C", "outbound", 20_000_000_000, "2024-07-01T00:00:00Z", fee=1_000_000, rate="61000"),
        row("move-in", "B", "inbound", 20_000_000_000, "2024-07-01T00:10:00Z", rate="61000"),
        row("sell-2", "B", "outbound", 5_000_000_000, "2025-08-01T00:00:00Z", rate="90000.123", kind="sell"),
        row("sell-3", "C", "outbound", 70_000_000_000, "2025-09-01T00:00:00Z", fee=2_000_000, fiat_value="61234.56", rate=None, kind="sell"),
    ]
    return rows, [pair("pair-1", "move-out", "move-in")]


def rail_history() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """A BTC to Liquid peg carrying value, then a Liquid sale."""
    rows = [
        row("rail-buy-1", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        row("rail-buy-2", "C", "inbound", 40_000_000_000, "2024-02-01T00:00:00Z", rate="45000.25", kind="buy"),
        row("peg-out", "C", "outbound", 60_000_000_000, "2024-05-01T00:00:00Z", rate="60000"),
        row("peg-in", "L", "inbound", 59_900_000_000, "2024-05-01T01:00:00Z", rate="60000", asset="LBTC"),
        row("liquid-sell", "L", "outbound", 10_000_000_000, "2024-09-01T00:00:00Z", rate="70000", asset="LBTC", kind="sell"),
    ]
    return rows, [pair("rail-1", "peg-out", "peg-in", kind="peg-in")]


# ---------------------------------------------------------------------------
# Projections built directly


def _digest(text: str) -> str:
    return hashlib.sha256(f"tax-ledger-golden:{text}".encode()).hexdigest()


def projected(
    tx_id: str,
    wallet: str,
    direction: str,
    amount: int,
    occurred_at: str,
    *,
    side: Optional[str] = None,
    anchor: Optional[str] = None,
    chain_order: Optional[ChainOrderEvidence] = None,
    **row_fields: Any,
) -> dict[str, Any]:
    """A finalized projection row, as ``compile_finalized_tax_projection`` gives one.

    The row id carries the synthetic ``custody-tax:`` prefix; ``anchor`` (the
    journal transaction id) defaults to ``tx_id``.
    """
    item = row(tx_id, wallet, direction, amount, occurred_at, **row_fields)
    quantity_hash = _digest(tx_id)
    item.update(
        {
            "id": f"custody-tax:{quantity_hash}:0:{amount}:{side or direction}",
            "custody_chain_order": chain_order,
            "amount_includes_fee": False,
            "journal_transaction_id": anchor or tx_id,
            "custody_finalized_tax_projection": True,
            "custody_quantity_hash": quantity_hash,
            "custody_slice_start_msat": 0,
            "custody_slice_end_msat": amount,
        }
    )
    return item


def move(
    out_row: Mapping[str, Any],
    in_row: Mapping[str, Any],
    *,
    pair_id: Optional[str] = None,
    group_id: Optional[str] = "derived",
    source: str = "reviewed_custody_component",
    reviewed_privacy: bool = False,
) -> dict[str, Any]:
    """A selected same-asset move between two projection rows.

    ``group_id="derived"`` gives the pair its own atomic bundle id; ``None``
    leaves it ungrouped.
    """
    pair_id = pair_id or f"move:{out_row['journal_transaction_id']}->{in_row['journal_transaction_id']}"
    return {
        "out": out_row,
        "in": in_row,
        "pair_id": pair_id,
        "source": source,
        "policy": "carrying-value",
        "group_id": f"bundle:{pair_id}" if group_id == "derived" else group_id,
        **({"kind": "custody-reviewed-coinjoin"} if reviewed_privacy else {}),
    }


def direct(
    rows: Sequence[Mapping[str, Any]],
    intra_pairs: Sequence[Mapping[str, Any]] = (),
    *,
    cross_asset_pairs: Sequence[Mapping[str, Any]] = (),
    quarantines: Sequence[Mapping[str, Any]] = (),
    wallet_refs: Mapping[str, Mapping[str, Any]] = WALLET_REFS,
) -> TaxEngineLedgerInputs:
    """Ledger inputs over a projection built here, not by the custody stages."""
    projection = FinalizedTaxProjection(
        rows=tuple(rows),
        intra_pairs=tuple(intra_pairs),
        cross_asset_pairs=tuple(cross_asset_pairs),
        quarantines=tuple(quarantines),
        selected_move_ids=tuple(str(item["pair_id"]) for item in intra_pairs),
    )
    return TaxEngineLedgerInputs(finalized_tax_projection=projection, wallet_refs_by_id=wallet_refs)


def chain_txid(label: str) -> str:
    return _digest(f"txid:{label}")


def graph_json(
    txid: str,
    *,
    confirmed: bool = True,
    spends: Iterable[tuple[str, int]] = (),
    chain: str = "bitcoin",
    network: str = "main",
) -> str:
    """Stored chain graph evidence: ``spends`` are the input outpoints."""
    status: dict[str, Any] = {"confirmed": confirmed}
    if confirmed:
        status.update(block_height=800_000, block_time=1_700_000_000)
    return json.dumps(
        {
            "txid": txid,
            "chain": chain,
            "network": network,
            "status": status,
            "vin": [{"txid": previous, "vout": vout} for previous, vout in spends],
            "vout": [],
        },
        sort_keys=True,
    )


def chain_order(label: str, *parents: str) -> ChainOrderEvidence:
    """Chain-order evidence: the move ``label`` spends outputs of ``parents``."""

    def scope(name: str) -> tuple[str, str, str, str]:
        return ("bitcoin", "main", chain_txid(name), "BTC")

    return ChainOrderEvidence(scope(label), frozenset(scope(parent) for parent in parents))


# ---------------------------------------------------------------------------
# The random half


def _stamp(instant: datetime) -> str:
    return instant.strftime("%Y-%m-%dT%H:%M:%SZ")


def _price(rng: random.Random) -> str:
    digits = rng.choice([0, 0, 2, 2, 4, 8])
    whole = rng.choice([1, 50, 999, 16000, 43210, 98765])
    if not digits:
        return str(whole)
    return f"{whole}.{rng.randrange(10**digits):0{digits}d}"


@dataclass(frozen=True)
class RandomBook:
    seed: int
    country: str
    profile: dict[str, Any]
    inputs: TaxEngineLedgerInputs
    # The dimensions the seed switched on.
    tags: tuple[str, ...]


# The probability that a seed switches each dimension on.
P_TAINT = 0.5
P_COARSE = 0.3
P_COARSE_REVIEW = 0.5
P_CHAIN = 0.25
P_RAIL = 0.3
P_OVERRIDES = 0.4
P_PRIVACY = 0.15
P_BUNDLE = 0.25
P_HOLD = 0.15
P_UNGROUPED = 0.25
P_MARKER_TEXT = 0.1
P_CROSS_ASSET = 0.3
_INCOME_KINDS = ("mining", "staking", "income", "interest", "routing_income")
# Austria cannot value these acquisitions; they quarantine and taint the asset.
_VALUATION_KINDS = ("airdrop", "hardfork")
_ODD_KINDS = ("referral_bonus", "cashback", "staking_reward")
_NON_SALE_KINDS = ("gift", "lost", "donation")
_MARKER_TEXTS = (
    "at_regime=alt",
    "at_regime=neu",
    "at_regime=alt at_regime=neu",
    "at_pool=default",
    "at_pool=other",
    "at_swap_link=lone",
    "note at_regime=neu",
)


class _RandomBookBuilder:
    def __init__(self, seed: int, country: str) -> None:
        self.rng = random.Random(f"{SEED}:ledger:{country}:{seed}")
        self.country = country
        self.rows: list[dict[str, Any]] = []
        self.pairs: list[dict[str, Any]] = []
        self.held = {"A": 0, "B": 0, "C": 0}
        self.liquid = 0
        self.tokens = 0
        self.tags: list[str] = []
        self.taint = False
        self.coarse = False
        self.serial = 0

    def name(self, prefix: str) -> str:
        self.serial += 1
        return f"{prefix}-{self.serial}"

    def rate(self) -> Optional[str]:
        if self.taint and self.rng.random() < 0.1:
            return None
        return _price(self.rng)

    def pricing(self, item: dict[str, Any]) -> dict[str, Any]:
        if self.coarse and item.get("fiat_rate_exact") and self.rng.random() < 0.35:
            item.update(pricing_quality=pricing.QUALITY_COARSE_FALLBACK, pricing_granularity="daily", pricing_provider="kraken")
        return item

    def inbound(self, when: datetime, wallet: str, amount: int, kind: str) -> None:
        self.rows.append(self.pricing(row(self.name("in"), wallet, "inbound", amount, _stamp(when), rate=self.rate(), kind=kind)))
        self.held[wallet] += amount

    def outbound(self, when: datetime, wallet: str, kind: str = "sell") -> None:
        spent = self.rng.randint(1, self.held[wallet])
        if self.rng.random() < 0.05:
            # A fee-only spend.
            amount, fee, kind = 0, spent, "withdrawal"
        else:
            fee = self.rng.randint(0, min(spent - 1, 10**7)) if spent > 1 and self.rng.random() < 0.5 else 0
            amount = spent - fee
        self.rows.append(self.pricing(row(self.name("out"), wallet, "outbound", amount, _stamp(when), fee=fee, rate=self.rate(), kind=kind)))
        self.held[wallet] -= spent

    def transfer(self, when: datetime, source: str, target: str, *, delay: int) -> tuple[str, str]:
        amount = self.rng.randint(1, self.held[source])
        fee = self.rng.randint(0, min(amount - 1, 10**7)) if amount > 1 else 0
        rate = self.rate()
        out_id, in_id = self.name("move-out"), self.name("move-in")
        self.rows.append(self.pricing(row(out_id, source, "outbound", amount - fee, _stamp(when), fee=fee, rate=rate)))
        self.rows.append(self.pricing(row(in_id, target, "inbound", amount - fee, _stamp(when + timedelta(minutes=delay)), rate=rate)))
        self.pairs.append(pair(self.name("pair"), out_id, in_id))
        self.held[source] -= amount
        self.held[target] += amount - fee
        return out_id, in_id


def _base_history(builder: _RandomBookBuilder, start: datetime) -> datetime:
    rng = builder.rng
    when = start
    for _ in range(rng.randint(4, 14)):
        when += timedelta(days=rng.choice([0, 1, 7, 90, 200]), hours=rng.choice([0, 0, 5]))
        roll = rng.random()
        if roll < 0.45 or not any(builder.held.values()):
            wallet = rng.choice(["A", "C", "C"])
            kind = rng.choice(["buy", "buy", "deposit", *_INCOME_KINDS, *(_VALUATION_KINDS if builder.taint else ())])
            if builder.taint and rng.random() < 0.08:
                kind = rng.choice(_ODD_KINDS)
            amount = rng.choice([rng.randint(1, 10**11), rng.randint(1, 10**6) * 10**5])
            builder.inbound(when, wallet, amount, kind)
        elif roll < 0.8:
            wallet = rng.choice([name for name, value in builder.held.items() if value > 0])
            kind = rng.choice(_NON_SALE_KINDS) if builder.taint and rng.random() < 0.1 else "sell"
            builder.outbound(when, wallet, kind)
        else:
            source = rng.choice([name for name, value in builder.held.items() if value > 0])
            target = rng.choice([name for name in builder.held if name != source])
            builder.transfer(when, source, target, delay=rng.choice([0, 10]))
    return when


def random_inputs(seed: int, country: str, *, terms: bool = False) -> RandomBook:
    """One seeded random book: its profile, ledger inputs, and switched-on dimensions.

    Every book has imported buys, earn kinds, sales with and without fees,
    fee-only spends, and reviewed moves between wallets. On top, per seed:

    - ``taint`` (``P_TAINT``): missing prices, unclassified income kinds,
      gifts and losses, and airdrops and hard forks (which Austria cannot
      value). Each quarantines its row and contaminates later lots, so the
      books without it keep every gate after it open;
    - ``coarse`` (``P_COARSE``): coarse fallback prices, with
      ``require_coarse_review`` on half of those books (``P_COARSE_REVIEW``);
    - ``chain`` (``P_CHAIN``): a same-instant move chain A->B->C carrying
      chain-order evidence;
    - ``rail`` (``P_RAIL``): BTC to LBTC rail pairs, carrying value or taxable;
    - ``overrides`` (``P_OVERRIDES``, Austria only): regime, category, and
      taxability overrides;
    - ``privacy`` (``P_PRIVACY``): a reviewed coinjoin move;
    - ``bundle`` (``P_BUNDLE``): two moves sharing one atomic bundle;
    - ``hold`` (``P_HOLD``): a pending or conflicting on-chain spend;
    - ``ungrouped`` (``P_UNGROUPED``): moves without a bundle id;
    - ``marker_text`` (``P_MARKER_TEXT``): user text with marker tokens;
    - ``cross_asset`` (``P_CROSS_ASSET``, Austria only): a swap pair between
      BTC and USDT, in either direction.

    ``terms`` (tagged so) changes only the profile: a generic book holds for
    one of ``LONG_TERM_DAYS`` instead of 365 days, and an Austrian book uses
    one of ``AT_EXTRA_METHODS``. The choice comes from a stream of its own,
    so the rows are the same as without it.
    """
    if country not in ("generic", "at"):
        raise ValueError(f"unknown country {country!r}")
    builder = _RandomBookBuilder(seed, country)
    rng = builder.rng
    method = rng.choice(GENERIC_METHODS if country == "generic" else AT_METHODS)
    extra: dict[str, Any] = {}
    if rng.random() < P_TAINT:
        builder.taint = True
        builder.tags.append("taint")
    if rng.random() < P_COARSE:
        builder.coarse = True
        builder.tags.append("coarse")
        if rng.random() < P_COARSE_REVIEW:
            extra["require_coarse_review"] = 1
            builder.tags.append("coarse_review")
    years = [2019, 2020, 2021, 2022] if country == "at" else [2019, 2021, 2023]
    start = datetime(rng.choice(years), rng.randint(1, 12), rng.randint(1, 28), tzinfo=timezone.utc)
    when = _base_history(builder, start)

    chain_ids: list[tuple[str, str]] = []
    if rng.random() < P_CHAIN and builder.held["A"] > 2:
        builder.tags.append("chain")
        when += timedelta(days=3)
        first = builder.transfer(when, "A", "B", delay=0)
        second = builder.transfer(when, "B", "C", delay=0)
        chain_ids = [first, second]
        if rng.random() < 0.5:
            # The projection sorts rows by (occurred_at, id), so the hop order
            # inside the instant follows the hashed row ids, and the evidence
            # decides it only when that order is wrong. This swap does not
            # reach the engine; it only shifts which rows the later overrides
            # and marker text pick, so it stays for the stored seed cases.
            builder.rows[-4:] = builder.rows[-2:] + builder.rows[-4:-2]

    if rng.random() < P_RAIL and builder.held["C"] > 1:
        builder.tags.append("rail")
        when += timedelta(days=rng.choice([1, 30]))
        amount = rng.randint(1, builder.held["C"])
        policy = rng.choice(["carrying-value", "carrying-value", "taxable"])
        rate = builder.rate()
        builder.rows.append(row(builder.name("peg-out"), "C", "outbound", amount, _stamp(when), rate=rate))
        builder.rows.append(
            row(builder.name("peg-in"), "L", "inbound", amount, _stamp(when + timedelta(hours=rng.choice([0, 1]))), rate=rate, asset="LBTC")
        )
        builder.pairs.append(pair(builder.name("rail"), builder.rows[-2]["id"], builder.rows[-1]["id"], kind="peg-in", policy=policy))
        builder.held["C"] -= amount
        builder.liquid += amount
        if builder.liquid > 1 and rng.random() < 0.6:
            when += timedelta(days=rng.choice([0, 10, 400]))
            back = rng.randint(1, builder.liquid)
            if rng.random() < 0.5:
                builder.rows.append(row(builder.name("liquid-sell"), "L", "outbound", back, _stamp(when), rate=builder.rate(), asset="LBTC", kind="sell"))
            else:
                builder.rows.append(row(builder.name("peg2-out"), "L", "outbound", back, _stamp(when), rate=builder.rate(), asset="LBTC"))
                builder.rows.append(row(builder.name("peg2-in"), "A", "inbound", back, _stamp(when + timedelta(hours=1)), rate=builder.rate()))
                builder.pairs.append(pair(builder.name("rail"), builder.rows[-2]["id"], builder.rows[-1]["id"], kind="peg-out", policy=policy))
                builder.held["A"] += back
            builder.liquid -= back

    if country == "at" and rng.random() < P_CROSS_ASSET and builder.held["C"] > 1:
        builder.tags.append("cross_asset")
        when += timedelta(days=rng.choice([1, 30]))
        policy = rng.choice(["carrying-value", "taxable"])
        usdt = rng.randint(1, 10**8) * 10**8
        if rng.random() < 0.5:
            amount = rng.randint(1, builder.held["C"])
            builder.rows.append(row(builder.name("swap-out"), "C", "outbound", amount, _stamp(when), rate=builder.rate(), kind="sell"))
            builder.rows.append(row(builder.name("swap-in"), "C", "inbound", usdt, _stamp(when), rate=rng.choice(["1", "0.98", None]), kind="buy", asset="USDT"))
            builder.held["C"] -= amount
            left = usdt
        else:
            # USDT bought for the swap: its out leg carries no taint from
            # earlier BTC history.
            builder.rows.append(row(builder.name("usdt-buy"), "C", "inbound", usdt, _stamp(when), rate=rng.choice(["1", "0.98"]), kind="buy", asset="USDT"))
            when += timedelta(days=rng.choice([0, 7]))
            spent = rng.randint(1, usdt)
            btc = rng.randint(1, 10**11)
            builder.rows.append(row(builder.name("swap-out"), "C", "outbound", spent, _stamp(when), rate="1", kind="sell", asset="USDT"))
            builder.rows.append(row(builder.name("swap-in"), "C", "inbound", btc, _stamp(when), rate=builder.rate(), kind="buy"))
            builder.held["C"] += btc
            left = usdt - spent
        builder.pairs.append(pair(builder.name("swap"), builder.rows[-2]["id"], builder.rows[-1]["id"], policy=policy))
        if left and rng.random() < 0.6:
            when += timedelta(days=rng.choice([0, 60]))
            builder.rows.append(row(builder.name("usdt-sell"), "C", "outbound", rng.randint(1, left), _stamp(when), rate="1.01", kind="sell", asset="USDT"))

    if rng.random() < P_PRIVACY and any(builder.held.values()):
        builder.tags.append("privacy")
        when += timedelta(days=2)
        source = rng.choice([name for name, value in builder.held.items() if value > 0])
        target = rng.choice([name for name in builder.held if name != source])
        out_id, in_id = builder.transfer(when, source, target, delay=10)
        boundary = rng.choice(["coinjoin", "payjoin", "sweep"])
        for item in builder.rows[-2:]:
            item["privacy_boundary"] = boundary

    if rng.random() < P_BUNDLE and builder.held["C"] > 2:
        builder.tags.append("bundle")
        when += timedelta(days=1)
        builder.transfer(when, "C", "A", delay=0)
        if builder.held["C"] > 1:
            builder.transfer(when, "C", "B", delay=0)
        else:
            builder.transfer(when, "A", "B", delay=0)

    if country == "at" and rng.random() < P_OVERRIDES:
        builder.tags.append("overrides")
        for item in builder.rows:
            roll = rng.random()
            if item["direction"] == "outbound" and roll < 0.15:
                item["at_regime_override"] = rng.choice(["alt", "neu"])
            elif roll < 0.25:
                item["at_category_override"] = rng.choice(["none", "neu_gain", "alt_taxfree", "neu_swap", "alt_spekulation"])
            elif roll < 0.32:
                item["taxability_override"] = 0

    if rng.random() < P_MARKER_TEXT and builder.rows:
        builder.tags.append("marker_text")
        rng.choice(builder.rows)["note"] = rng.choice(_MARKER_TEXTS)

    if terms:
        builder.tags.append("terms")
        choose = random.Random(f"{SEED}:ledger-terms:{country}:{seed}").choice
        if country == "generic":
            extra["long_term_days"] = choose(LONG_TERM_DAYS)
        else:
            method = choose(AT_EXTRA_METHODS)
    book_profile = profile(country, method, **extra)
    inputs = finalized(book_profile, builder.rows, builder.pairs)
    projection = inputs.finalized_tax_projection
    rows_by_anchor: dict[str, list[dict[str, Any]]] = {}
    for item in projection.rows:
        rows_by_anchor.setdefault(str(item["journal_transaction_id"]), []).append(item)

    if chain_ids:
        (first_out, first_in), (second_out, second_in) = chain_ids
        for anchor, evidence in (
            (first_out, chain_order(f"{seed}:hop1")),
            (first_in, chain_order(f"{seed}:hop1")),
            (second_out, chain_order(f"{seed}:hop2", f"{seed}:hop1")),
            (second_in, chain_order(f"{seed}:hop2", f"{seed}:hop1")),
        ):
            for item in rows_by_anchor.get(anchor, ()):
                item["custody_chain_order"] = evidence

    if rng.random() < P_HOLD and projection.rows:
        builder.tags.append("hold")
        target = rng.choice(list(projection.rows))
        txid = chain_txid(f"{seed}:held")
        if rng.random() < 0.5:
            target.update(external_id=txid, raw_json=graph_json(txid, confirmed=False))
        else:
            other = rng.choice(list(projection.rows))
            rival = chain_txid(f"{seed}:rival")
            spends = [(chain_txid(f"{seed}:prevout"), 0)]
            target.update(external_id=txid, raw_json=graph_json(txid, confirmed=False, spends=spends))
            if other is not target:
                other.update(external_id=rival, raw_json=graph_json(rival, confirmed=rng.random() < 0.5, spends=spends))

    if projection.intra_pairs and rng.random() < P_UNGROUPED:
        builder.tags.append("ungrouped")
        for item in projection.intra_pairs:
            if rng.random() < 0.6:
                item["group_id"] = None
    if "bundle" in builder.tags and len(projection.intra_pairs) >= 2:
        shared = f"bundle:{seed}"
        for item in list(projection.intra_pairs)[-2:]:
            item["group_id"] = shared
    return RandomBook(seed, country, book_profile, inputs, tuple(builder.tags))


# ---------------------------------------------------------------------------
# The named corpus

Outcome = tuple[Optional[TaxEngineLedgerResult], Optional[BaseException]]
Check = Callable[[Optional[TaxEngineLedgerResult], Optional[BaseException]], list[str]]


@dataclass(frozen=True)
class GoldenCase:
    name: str
    tags: tuple[str, ...]
    profile: dict[str, Any]
    inputs: TaxEngineLedgerInputs
    # What the oracle's outcome must show for the case to reach its shape;
    # returns the complaints (none when it does).
    check: Check
    # The one §3.5 shape an LD-5 case has (a ``projection_range`` name);
    # ``None`` for a case within range.
    invariant: Optional[str] = None


_REALIZED = ("income", "disposal", "fee", "transfer_fee")


def succeeds(
    *,
    reasons: Iterable[str] = (),
    absent: Iterable[str] = (),
    counts: Mapping[str, int] = {},
    entry_types: Iterable[str] = (),
    descriptions: Iterable[str] = (),
    categories: Iterable[Optional[str]] = (),
    regimes: Mapping[str, str] = {},
    gains: Mapping[str, bool] = {},
) -> Check:
    """The oracle succeeds, with these quarantine reasons (``counts``: exactly
    that many), entry types, description fragments, and Austrian categories,
    and none of ``absent``.

    ``regimes`` maps a row to the ``at_regime=`` its realized entries carry;
    ``gains`` maps a row to whether its disposals book a nonzero gain.
    """
    reasons, absent, entry_types = tuple(reasons), tuple(absent), tuple(entry_types)
    descriptions, categories = tuple(descriptions), tuple(categories)

    def check(result: Optional[TaxEngineLedgerResult], error: Optional[BaseException]) -> list[str]:
        if error is not None:
            return [f"expected success, got {type(error).__name__}: {error}"]
        assert result is not None
        found = [str(item["reason"]) for item in result.quarantines]
        complaints = [f"no {reason} quarantine in {found}" for reason in reasons if reason not in found]
        complaints += [f"unexpected {reason} quarantine" for reason in absent if reason in found]
        complaints += [f"{found.count(reason)} {reason} quarantines, not {count}" for reason, count in counts.items() if found.count(reason) != count]
        types = [str(entry["entry_type"]) for entry in result.entries]
        complaints += [f"no {kind} entry in {types}" for kind in entry_types if kind not in types]
        texts = [str(entry.get("description") or "") for entry in result.entries]
        complaints += [f"no description contains {text!r}" for text in descriptions if not any(text in item for item in texts)]
        realized = [entry for entry in result.entries if entry["entry_type"] in _REALIZED]
        found_categories = [entry.get("at_category") for entry in realized]
        complaints += [f"no {category!r} category in {found_categories}" for category in categories if category not in found_categories]
        for row_id, regime in regimes.items():
            texts = [str(entry["description"]) for entry in realized if entry["transaction_id"] == row_id]
            if not texts or not all(text.startswith(f"at_regime={regime} ") for text in texts):
                complaints.append(f"{row_id} is not at_regime={regime}: {texts}")
        for row_id, nonzero in gains.items():
            found_gains = [entry["gain_loss"] for entry in realized if entry["transaction_id"] == row_id and entry["entry_type"] == "disposal"]
            if not found_gains or any(bool(gain) != nonzero for gain in found_gains):
                complaints.append(f"{row_id} gains {found_gains}, expected {'nonzero' if nonzero else 'zero'}")
        return complaints

    return check


def fails(*fragments: str, error_class: str = "AppError") -> Check:
    """The oracle raises ``error_class`` with these fragments in its message."""

    def check(result: Optional[TaxEngineLedgerResult], error: Optional[BaseException]) -> list[str]:
        if error is None:
            return ["expected a failure, got a result"]
        message = str(error)
        complaints = [] if type(error).__name__ == error_class else [f"expected {error_class}, got {type(error).__name__}: {message}"]
        return complaints + [f"{fragment!r} not in {message!r}" for fragment in fragments if fragment not in message]

    return check


def _case(name: str, tags: Iterable[str], book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs, check: Check) -> GoldenCase:
    return GoldenCase(name, tuple(tags), dict(book_profile), inputs, check)


def _book(
    name: str,
    tags: Iterable[str],
    book_profile: Mapping[str, Any],
    rows: Sequence[Mapping[str, Any]],
    pairs: Sequence[Mapping[str, Any]] = (),
    check: Check = succeeds(),
    *,
    payouts: Sequence[Mapping[str, Any]] = (),
) -> GoldenCase:
    return _case(name, tags, book_profile, finalized(book_profile, rows, pairs, payouts=payouts), check)


def _ungroup(inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerInputs:
    """The same projection with every move ungrouped (no atomic bundle id)."""
    for item in inputs.finalized_tax_projection.intra_pairs:
        item["group_id"] = None
    return inputs


def _generic_cases() -> list[GoldenCase]:
    cases: list[GoldenCase] = []
    for method in GENERIC_METHODS:
        rows, pairs = ledger_history()
        cases.append(
            _book(
                f"generic-ledger-{method.lower()}",
                ("generic", "method", "fee"),
                profile("generic", method),
                rows,
                pairs,
                succeeds(entry_types=("acquisition", "disposal", "income", "transfer_fee", "transfer_out", "transfer_in")),
            )
        )
        rows, pairs = rail_history()
        cases.append(
            _book(
                f"generic-rail-{method.lower()}",
                ("generic", "method", "carry"),
                profile("generic", method),
                rows,
                pairs,
                succeeds(entry_types=("disposal",)),
            )
        )

    fifo = profile("generic", "FIFO")
    review = profile("generic", "FIFO", require_coarse_review=1)
    coarse = {"pricing_quality": pricing.QUALITY_COARSE_FALLBACK, "pricing_granularity": "daily", "pricing_provider": "kraken"}
    cases.append(
        _book(
            "generic-unpriced",
            ("generic", "pricing", "fee"),
            fifo,
            [
                row("buy", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
                row("gift-in", "A", "inbound", 10_000_000_000, "2024-02-01T00:00:00Z", rate=None, kind="buy"),
                row("sell-unpriced", "C", "outbound", 20_000_000_000, "2024-03-01T00:00:00Z", fee=1_000_000, rate=None, kind="sell"),
                row("sell", "C", "outbound", 10_000_000_000, "2024-04-01T00:00:00Z", rate="50000", kind="sell"),
                row("sell-a", "A", "outbound", 1_000_000_000, "2024-05-01T00:00:00Z", rate="50000", kind="sell"),
            ],
            check=succeeds(reasons=("missing_spot_price",)),
        )
    )
    coarse_rows = [
        row("buy", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        row("coarse-buy", "A", "inbound", 10_000_000_000, "2024-02-01T00:00:00Z", rate="42000", kind="buy", **coarse),
        row("coarse-sell", "C", "outbound", 20_000_000_000, "2024-03-01T00:00:00Z", fee=1_000_000, rate="45000", kind="sell", **coarse),
        row("sell", "C", "outbound", 10_000_000_000, "2024-04-01T00:00:00Z", rate="50000", kind="sell"),
    ]
    cases.append(
        _book("generic-coarse-events-review", ("generic", "pricing", "coarse", "fee"), review, coarse_rows, check=succeeds(reasons=("pricing_review_required",)))
    )
    cases.append(
        _book("generic-coarse-events-accepted", ("generic", "pricing", "coarse", "fee"), fifo, coarse_rows, check=succeeds(absent=("pricing_review_required",)))
    )
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] == "move-out":
            item.update(coarse)
    cases.append(
        _book("generic-coarse-move-out-priced", ("generic", "pricing", "coarse", "fee", "move"), review, rows, pairs, succeeds(reasons=("pricing_review_required",)))
    )
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] == "move-out":
            item.update(fiat_rate=None, fiat_rate_exact=None)
        if item["id"] == "move-in":
            item.update(coarse)
    cases.append(
        _book("generic-coarse-move-in-priced", ("generic", "pricing", "coarse", "fee", "move"), review, rows, pairs, succeeds(reasons=("pricing_review_required",)))
    )
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] in ("move-out", "move-in"):
            item.update(fiat_rate=None, fiat_rate_exact=None)
    cases.append(_book("generic-move-fee-unpriced", ("generic", "pricing", "fee", "move"), fifo, rows, pairs, succeeds(reasons=("missing_spot_price",))))

    income = [
        row(f"earn-{kind}", "A", "inbound", 1_000_000_000 + index, f"2024-01-{index + 1:02d}T00:00:00Z", rate="40000.5", kind=kind)
        for index, kind in enumerate(
            ("income", "interest", "lending_interest", "mining", "mining_reward", "routing_income", "staking", "airdrop", "hardfork", "wages", "deposit")
        )
    ]
    income.append(row("sell", "A", "outbound", 5_000_000_000, "2024-06-01T00:00:00Z", fee=2_000_000, rate="60000", kind="sell"))
    cases.append(_book("generic-income-kinds", ("generic", "income", "fee"), fifo, income, check=succeeds(entry_types=("income", "disposal"))))
    cases.append(
        _book(
            "generic-unclassified-income",
            ("generic", "income"),
            fifo,
            [
                row("buy", "A", "inbound", 10_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
                row("referral", "A", "inbound", 1_000_000_000, "2024-02-01T00:00:00Z", kind="referral_bonus"),
                row("cashback", "A", "inbound", 1_000_000_000, "2024-02-02T00:00:00Z", kind="card cashback"),
                row("sell", "A", "outbound", 5_000_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="sell"),
            ],
            check=succeeds(reasons=("unclassified_income_kind",)),
        )
    )
    cases.append(
        _book(
            "generic-gift-lost",
            ("generic", "non_sale", "fee"),
            fifo,
            [
                row("buy", "A", "inbound", 10_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
                row("gift", "A", "outbound", 1_000_000_000, "2024-02-01T00:00:00Z", fee=1_000, kind="gift"),
                row("lost", "A", "outbound", 1_000_000_000, "2024-03-01T00:00:00Z", kind="lost"),
                row("donation", "A", "outbound", 1_000_000_000, "2024-03-02T00:00:00Z", kind="Donation"),
                row("sell", "A", "outbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="sell"),
            ],
            check=succeeds(reasons=("non_sale_disposal_kind",)),
        )
    )

    # Same-instant A -> B -> C; listed hop 2 first, so only the order the
    # chain evidence (or the wallets) gives lets hop 2 find its coins.
    for evidence in (True, False):
        buy = projected("buy", "A", "inbound", 50_000_000_000, "2024-01-01T00:00:00Z", kind="buy")
        hop1_out = projected("hop1-out", "A", "outbound", 30_000_000_000, "2024-05-01T00:00:00Z", side="move-out", fee=1_000_000, rate="60000", chain_order=chain_order("hop1") if evidence else None)
        hop1_in = projected("hop1-in", "B", "inbound", 30_000_000_000, "2024-05-01T00:00:00Z", side="move-in", rate="60000", chain_order=chain_order("hop1") if evidence else None)
        hop2_out = projected("hop2-out", "B", "outbound", 20_000_000_000, "2024-05-01T00:00:00Z", side="move-out", fee=2_000_000, rate="60000", chain_order=chain_order("hop2", "hop1") if evidence else None)
        hop2_in = projected("hop2-in", "C", "inbound", 20_000_000_000, "2024-05-01T00:00:00Z", side="move-in", rate="60000", chain_order=chain_order("hop2", "hop1") if evidence else None)
        sell = projected("sell", "C", "outbound", 10_000_000_000, "2024-06-01T00:00:00Z", rate="70000", kind="sell")
        cases.append(
            _case(
                "generic-same-time-chain" + ("-evidence" if evidence else "-wallets"),
                ("generic", "move", "order", "fee") + (("chain_evidence",) if evidence else ()),
                fifo,
                direct([buy, hop2_out, hop2_in, hop1_out, hop1_in, sell], [move(hop2_out, hop2_in), move(hop1_out, hop1_in)]),
                succeeds(absent=("insufficient_lots",), entry_types=("transfer_fee", "disposal")),
            )
        )

    # Atomic bundle of two moves out of A; the second needs more than A holds.
    buy = projected("buy", "A", "inbound", 30_000_000_000, "2024-01-01T00:00:00Z", kind="buy")
    fan1_out = projected("fan1-out", "A", "outbound", 20_000_000_000, "2024-05-01T00:00:00Z", side="move-out", fee=1_000_000, rate="60000")
    fan1_in = projected("fan1-in", "B", "inbound", 20_000_000_000, "2024-05-01T00:10:00Z", side="move-in", rate="60000")
    fan2_out = projected("fan2-out", "A", "outbound", 20_000_000_000, "2024-05-01T00:00:00Z", side="move-out", rate="60000")
    fan2_in = projected("fan2-in", "C", "inbound", 20_000_000_000, "2024-05-01T00:10:00Z", side="move-in", rate="60000")
    later = projected("sell-b", "B", "outbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="70000", kind="sell")
    cases.append(
        _case(
            "generic-bundle-one-failing",
            ("generic", "move", "bundle", "gate", "fee"),
            fifo,
            direct([buy, fan1_out, fan1_in, fan2_out, fan2_in, later], [move(fan1_out, fan1_in, group_id="bundle:fan"), move(fan2_out, fan2_in, group_id="bundle:fan")]),
            succeeds(reasons=("insufficient_lots", "derived_transfer_group_blocked")),
        )
    )
    # The same bundle when A holds enough: both moves book.
    rich = projected("buy", "A", "inbound", 50_000_000_000, "2024-01-01T00:00:00Z", kind="buy")
    cases.append(
        _case(
            "generic-bundle-passing",
            ("generic", "move", "bundle", "fee"),
            fifo,
            direct([rich, fan1_out, fan1_in, fan2_out, fan2_in, later], [move(fan1_out, fan1_in, group_id="bundle:fan"), move(fan2_out, fan2_in, group_id="bundle:fan")]),
            succeeds(absent=("insufficient_lots", "derived_transfer_group_blocked"), entry_types=("transfer_out",)),
        )
    )
    # Bundles normalize blocks before the walk: one whose fee-bearing move
    # has no price, and one blocked twice (a coarse move under review, then
    # an unpriced one), which keeps its first reason. Their priced siblings
    # are flagged and book nothing.
    coarse_out = projected("coarse-out", "A", "outbound", 10_000_000_000, "2024-05-01T00:00:00Z", side="move-out", fee=1_000_000, rate="60000", **coarse)
    coarse_in = projected("coarse-in", "B", "inbound", 10_000_000_000, "2024-05-01T00:10:00Z", side="move-in", rate="60000", **coarse)
    unpriced_out = projected("unpriced-out", "A", "outbound", 10_000_000_000, "2024-05-02T00:00:00Z", side="move-out", fee=1_000_000, rate=None)
    unpriced_in = projected("unpriced-in", "B", "inbound", 10_000_000_000, "2024-05-02T00:10:00Z", side="move-in", rate=None)
    sibling_out = projected("sibling-out", "A", "outbound", 10_000_000_000, "2024-05-03T00:00:00Z", side="move-out", fee=1_000_000, rate="60000")
    sibling_in = projected("sibling-in", "C", "inbound", 10_000_000_000, "2024-05-03T00:10:00Z", side="move-in", rate="60000")
    later = projected("sell-a", "A", "outbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="70000", kind="sell")
    cases.append(
        _case(
            "generic-bundle-unpriced-move",
            ("generic", "move", "bundle", "pricing", "fee"),
            fifo,
            direct(
                [rich, unpriced_out, unpriced_in, sibling_out, sibling_in, later],
                [move(unpriced_out, unpriced_in, group_id="bundle:priced"), move(sibling_out, sibling_in, group_id="bundle:priced")],
            ),
            _blocked_in_normalize(("missing_spot_price", "missing_spot_price"), "missing_spot_price"),
        )
    )
    cases.append(
        _case(
            "generic-bundle-blocked-twice",
            ("generic", "move", "bundle", "pricing", "coarse", "fee"),
            review,
            direct(
                [rich, coarse_out, coarse_in, unpriced_out, unpriced_in, sibling_out, sibling_in, later],
                [
                    move(coarse_out, coarse_in, group_id="bundle:twice"),
                    move(unpriced_out, unpriced_in, group_id="bundle:twice"),
                    move(sibling_out, sibling_in, group_id="bundle:twice"),
                ],
            ),
            _blocked_in_normalize(
                ("pricing_review_required", "pricing_review_required", "missing_spot_price", "missing_spot_price"),
                "pricing_review_required",
            ),
        )
    )

    # Ungrouped moves: one that passes, one that fails with distinct
    # anchors, and one whose legs share one anchor (one quarantine).
    buy = projected("buy", "A", "inbound", 30_000_000_000, "2024-01-01T00:00:00Z", kind="buy")
    ok_out = projected("ok-out", "A", "outbound", 10_000_000_000, "2024-02-01T00:00:00Z", side="move-out", fee=1_000_000, rate="50000")
    ok_in = projected("ok-in", "B", "inbound", 10_000_000_000, "2024-02-01T00:10:00Z", side="move-in", rate="50000")
    short_out = projected("short-out", "B", "outbound", 15_000_000_000, "2024-03-01T00:00:00Z", side="move-out", rate="50000")
    short_in = projected("short-in", "C", "inbound", 15_000_000_000, "2024-03-01T00:10:00Z", side="move-in", rate="50000")
    shared_out = projected("shared-out", "B", "outbound", 12_000_000_000, "2024-04-01T00:00:00Z", side="move-out", anchor="shared-tx", rate="50000")
    shared_in = projected("shared-in", "C", "inbound", 12_000_000_000, "2024-04-01T00:00:00Z", side="move-in", anchor="shared-tx", rate="50000")
    sell = projected("sell", "B", "outbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="70000", kind="sell")
    cases.append(
        _case(
            "generic-ungrouped-pass",
            ("generic", "move", "ungrouped", "fee"),
            fifo,
            direct([buy, ok_out, ok_in, sell], [move(ok_out, ok_in, group_id=None)]),
            succeeds(absent=("insufficient_lots",), entry_types=("transfer_out", "transfer_fee")),
        )
    )
    cases.append(
        _case(
            "generic-ungrouped-fail-distinct-anchors",
            ("generic", "move", "ungrouped", "gate", "fee"),
            fifo,
            direct([buy, ok_out, ok_in, short_out, short_in, sell], [move(ok_out, ok_in, group_id=None), move(short_out, short_in, group_id=None)]),
            succeeds(counts={"insufficient_lots": 2}),
        )
    )
    cases.append(
        _case(
            "generic-ungrouped-fail-shared-anchor",
            ("generic", "move", "ungrouped", "gate", "fee"),
            fifo,
            direct([buy, ok_out, ok_in, shared_out, shared_in, sell], [move(ok_out, ok_in, group_id=None), move(shared_out, shared_in, group_id=None)]),
            succeeds(counts={"insufficient_lots": 1}),
        )
    )

    # A reviewed coinjoin move, and a standalone privacy hop.
    buy = projected("buy", "A", "inbound", 30_000_000_000, "2024-01-01T00:00:00Z", kind="buy")
    cj_out = projected("cj-out", "A", "outbound", 10_000_000_000, "2024-02-01T00:00:00Z", side="move-out", fee=5_000, rate="50000", privacy_boundary="coinjoin")
    cj_in = projected("cj-in", "B", "inbound", 10_000_000_000, "2024-02-01T00:10:00Z", side="move-in", rate="50000", privacy_boundary="coinjoin")
    hop_in = projected("hop-in", "C", "inbound", 2_000_000_000, "2024-03-01T00:00:00Z", rate="50000", privacy_boundary="payjoin", raw_json=json.dumps({"source": "wasabi", "isLikelyCoinJoin": True}))
    sell = projected("sell", "B", "outbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="70000", kind="sell")
    cases.append(
        _case(
            "generic-privacy",
            ("generic", "move", "privacy", "fee"),
            fifo,
            direct([buy, cj_out, cj_in, hop_in, sell], [move(cj_out, cj_in, reviewed_privacy=True)]),
            succeeds(reasons=("privacy_hop_unresolved",), entry_types=("transfer_out",)),
        )
    )

    # Holds: an unconfirmed spend, and a conflict over one prevout whose
    # unconfirmed loser is one leg of a move (its partner books standalone).
    cases.append(_case("generic-hold-pending", ("generic", "hold"), fifo, _hold_inputs(conflict=False), succeeds(reasons=("pending_onchain_confirmation",))))
    cases.append(_case("generic-hold-conflict", ("generic", "hold", "move"), fifo, _hold_inputs(conflict=True), succeeds(reasons=("conflicting_spend",))))

    # Wallet A over-sells while B still holds enough for the asset's total.
    cases.append(
        _book(
            "generic-account-local-oversell",
            ("generic", "gate", "fee"),
            fifo,
            [
                row("buy-a", "A", "inbound", 10_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
                row("buy-b", "B", "inbound", 50_000_000_000, "2024-01-02T00:00:00Z", kind="buy"),
                row("sell-a", "A", "outbound", 15_000_000_000, "2024-03-01T00:00:00Z", fee=1_000_000, rate="50000", kind="sell"),
                row("sell-b", "B", "outbound", 5_000_000_000, "2024-04-01T00:00:00Z", rate="50000", kind="sell"),
            ],
            check=succeeds(reasons=("insufficient_lots",), entry_types=("disposal",)),
        )
    )

    rows, pairs = ledger_history()
    cases.append(_book("generic-method-bogus", ("generic", "method", "error"), profile("generic", "bogus"), rows, pairs, fails("RP2 multi-asset tax calculation failed: Unsupported RP2 accounting method 'bogus'")))
    cases.append(
        _book(
            "generic-method-moving-average-at",
            ("generic", "method", "error"),
            profile("generic", "moving_average_at"),
            rows,
            pairs,
            fails("RP2 multi-asset tax calculation failed: Unsupported RP2 accounting method 'moving_average_at'"),
        )
    )
    cases.append(
        _book(
            "generic-method-bogus-without-inbound",
            ("generic", "method"),
            profile("generic", "bogus"),
            [row("sell", "A", "outbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="sell")],
            check=succeeds(reasons=("insufficient_lots",)),
        )
    )
    cases.append(_usertag_swap("generic-usertag-swap", "generic", "FIFO", tagged=True, check=succeeds(absent=("at_swap_basis_carry_unresolved",))))

    # An oracle quirk the engine keeps (design §4.12, property 5): a
    # quarantined sale consumes nothing, so pricing an earlier purchase
    # releases the sale it left short, which then starves a later move.
    for priced in (False, True):
        buy = projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy")
        late_buy = projected("late-buy", "A", "inbound", 100_000_000_000, "2024-02-01T00:00:00Z", rate="40000" if priced else None, kind="buy")
        sale = projected("sale", "A", "outbound", 150_000_000_000, "2024-03-01T00:00:00Z", rate="50000", kind="sell")
        move_out = projected("move", "A", "outbound", 80_000_000_000, "2024-04-01T00:00:00Z", side="move-out", rate="50000")
        move_in = projected("move", "B", "inbound", 80_000_000_000, "2024-04-01T00:00:00Z", side="move-in", rate="50000")
        cases.append(
            _case(
                "generic-pricing-releases-a-sale-" + ("priced" if priced else "unpriced"),
                ("generic", "pricing", "gate", "move", "quirk"),
                fifo,
                direct([buy, late_buy, sale, move_out, move_in], [move(move_out, move_in, group_id=None)]),
                _quarantined(
                    [("move", "insufficient_lots")] if priced else [("late-buy", "missing_spot_price"), ("sale", "insufficient_lots")],
                    ["acquisition", "acquisition", "disposal"] if priced else ["acquisition", "transfer_out", "transfer_in"],
                ),
            )
        )
    return cases


def _quarantined(quarantines: Sequence[tuple[str, str]], entry_types: Sequence[str]) -> Check:
    """The oracle succeeds with exactly these ``(transaction, reason)``
    quarantines and entry types, in order."""

    def check(result: Optional[TaxEngineLedgerResult], error: Optional[BaseException]) -> list[str]:
        if error is not None:
            return [f"expected success, got {type(error).__name__}: {error}"]
        assert result is not None
        found = [(str(item["transaction_id"]), str(item["reason"])) for item in result.quarantines]
        types = [str(entry["entry_type"]) for entry in result.entries]
        complaints = [] if found == list(quarantines) else [f"quarantines {found}, expected {list(quarantines)}"]
        return complaints + ([] if types == list(entry_types) else [f"entries {types}, expected {list(entry_types)}"])

    return check


def _blocked_in_normalize(reasons: tuple[str, ...], blocked_by: str) -> Check:
    """The oracle blocks the book's one bundle in normalize: its failing
    moves' quarantines come first, in order, then one flag for each other
    transaction of the bundle, naming the bundle's first reason, and no move
    of the bundle books."""

    def check(result: Optional[TaxEngineLedgerResult], error: Optional[BaseException]) -> list[str]:
        if error is not None:
            return [f"expected success, got {type(error).__name__}: {error}"]
        assert result is not None
        found = [str(item["reason"]) for item in result.quarantines]
        flags = [json.loads(item["detail_json"]) for item in result.quarantines if item["reason"] == "derived_transfer_group_blocked"]
        complaints = [] if found[: len(reasons)] == list(reasons) else [f"quarantines {found}, expected {list(reasons)} first"]
        if not flags or any(flag.get("blocked_by_reason") != blocked_by for flag in flags):
            complaints.append(f"group flags {flags} are not blocked by {blocked_by}")
        if result.intra_audit:
            complaints.append(f"{len(result.intra_audit)} moves booked")
        return complaints

    return check


def _hold_inputs(*, conflict: bool) -> TaxEngineLedgerInputs:
    prevout = (chain_txid("hold-prevout"), 0)
    held_txid = chain_txid("held")
    buy = projected("buy", "A", "inbound", 30_000_000_000, "2024-01-01T00:00:00Z", kind="buy")
    out_row = projected(
        "held-out", "A", "outbound", 10_000_000_000, "2024-02-01T00:00:00Z", side="move-out", fee=5_000, rate="50000",
        external_id=held_txid, raw_json=graph_json(held_txid, confirmed=False, spends=[prevout]),
    )
    in_row = projected("held-in", "B", "inbound", 10_000_000_000, "2024-02-01T00:10:00Z", side="move-in", rate="50000", external_id="exchange-held-in")
    rows = [buy, out_row, in_row]
    if conflict:
        winner = chain_txid("winner")
        rows.append(
            projected(
                "winner-out", "A", "outbound", 5_000_000_000, "2024-02-01T00:05:00Z", rate="50000", kind="sell",
                external_id=winner, raw_json=graph_json(winner, confirmed=True, spends=[prevout]),
            )
        )
    rows.append(projected("sell", "A", "outbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="70000", kind="sell"))
    return direct(rows, [move(out_row, in_row)])


def _holding_periods(expected: Mapping[str, Sequence[str]]) -> Check:
    """The oracle succeeds, and each row's realized entries carry these
    holding periods (``capital_gains_type``), in order."""

    def check(result: Optional[TaxEngineLedgerResult], error: Optional[BaseException]) -> list[str]:
        if error is not None:
            return [f"expected success, got {type(error).__name__}: {error}"]
        assert result is not None
        complaints = []
        for row_id, kinds in expected.items():
            found = [entry["capital_gains_type"] for entry in result.entries if entry["transaction_id"] == row_id and entry["entry_type"] in _REALIZED]
            if found != list(kinds):
                complaints.append(f"{row_id} holds {found}, expected {list(kinds)}")
        return complaints

    return check


def _holding_period_cases() -> list[GoldenCase]:
    """Sales just before, at, and just after a generic holding period ends.

    The first lot is bought late on a leap day at UTC-5; the sales give their
    times in other offsets (one sorts out of time order as text), so only the
    instants count. A held day is a whole 24 hours, floored, and a lot held
    ``tax_long_term_days`` days is long term. A move with a fee keeps the
    first lot's date, and the last sale takes the rest of the first lot and
    the start of the second, bought within the period.
    """

    def stamp(instant: datetime, hours: int) -> str:
        return instant.astimezone(timezone(timedelta(hours=hours))).isoformat()

    bought = datetime(2024, 2, 29, 23, 30, tzinfo=timezone(timedelta(hours=-5)))
    cases: list[GoldenCase] = []
    for days in LONG_TERM_DAYS:
        ends = bought + timedelta(days=days)
        rows = [
            row("buy-1", "C", "inbound", 100_000_000_000, stamp(bought, -5), kind="buy"),
            row("move-out", "C", "outbound", 90_000_000_000, stamp(bought + timedelta(hours=1), -5), fee=1_000_000, rate="40500"),
            row("move-in", "B", "inbound", 90_000_000_000, stamp(bought + timedelta(hours=1, minutes=10), -5), rate="40500"),
            row("buy-2", "C", "inbound", 100_000_000_000, stamp(bought + timedelta(hours=12), -5), rate="41000", kind="buy"),
            row("sell-before", "B", "outbound", 20_000_000_000, stamp(ends - timedelta(seconds=1), -5), rate="50000", kind="sell"),
            row("sell-at", "B", "outbound", 20_000_000_000, stamp(ends, 1), rate="50000.5", kind="sell"),
            row("sell-after", "B", "outbound", 20_000_000_000, stamp(ends + timedelta(seconds=1), 0), rate="50001", kind="sell"),
            row("sell-spanning", "C", "outbound", 50_000_000_000, stamp(ends + timedelta(hours=1), 0), fee=2_000_000, rate="51000", kind="sell"),
        ]
        check = _holding_periods(
            {"move-out": ("short",), "sell-before": ("short",), "sell-at": ("long",), "sell-after": ("long",), "sell-spanning": ("long", "short")}
        )
        for method in ("FIFO", "MOVING_AVERAGE"):
            cases.append(
                _book(
                    f"generic-holding-period-{days}-days-{method.lower()}",
                    ("generic", "method", "holding_period", "fee", "move"),
                    profile("generic", method, long_term_days=days),
                    rows,
                    [pair("pair-1", "move-out", "move-in")],
                    check,
                )
            )
    return cases


def _usertag_swap(name: str, country: str, method: str, *, tagged: bool, check: Check) -> GoldenCase:
    rows = [
        row("buy-btc", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="40000", kind="buy"),
        row("swap-out", "C", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="sell"),
        row("swap-in", "C", "inbound", 30_000_000_000_000, "2024-06-01T00:00:00Z", rate="1", kind="buy", asset="USDT"),
        row("sell-usdt", "C", "outbound", 10_000_000_000_000, "2024-09-01T00:00:00Z", rate="1.1", kind="sell", asset="USDT"),
    ]
    if tagged:
        rows[1]["note"] = "manual swap at_swap_link=s1"
        rows[2]["note"] = "manual swap at_swap_link=s1"
    return _book(name, (country, "marker_text", "cross_asset"), profile(country, method), rows, check=check)


def _carry_cases() -> list[GoldenCase]:
    cases: list[GoldenCase] = []
    fifo = profile("generic", "FIFO")

    def hops(*, same_time: bool, three: bool, unpriced_in: bool = False, sale_between: bool = True) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
        t1, t2, t3 = ("2024-05-01T00:00:00Z",) * 3 if same_time else ("2024-05-01T00:00:00Z", "2024-06-01T00:00:00Z", "2024-08-01T00:00:00Z")
        later = "2024-05-01T00:00:00Z" if same_time else "2024-05-01T01:00:00Z"
        rows = [
            row("buy", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
            row("buy-2", "C", "inbound", 20_000_000_000, "2024-02-01T00:00:00Z", rate="45000.5", kind="buy"),
            row("peg-out", "C", "outbound", 60_000_000_000, t1, fee=1_000_000, rate="60000"),
            row("peg-in", "L", "inbound", 60_000_000_000, later if same_time else "2024-05-01T01:00:00Z", rate=None if unpriced_in else "60000", asset="LBTC"),
            row("peg2-out", "L", "outbound", 50_000_000_000, t2, rate="65000", asset="LBTC"),
            row("peg2-in", "A", "inbound", 50_000_000_000, t2 if same_time else "2024-06-01T01:00:00Z", rate="65000"),
        ]
        pairs = [pair("rail-1", "peg-out", "peg-in", kind="peg-in"), pair("rail-2", "peg2-out", "peg2-in", kind="peg-out")]
        if sale_between:
            rows.append(row("liquid-sell", "L", "outbound", 5_000_000_000, "2024-05-15T00:00:00Z" if not same_time else "2024-05-20T00:00:00Z", rate="62000", asset="LBTC", kind="sell"))
        if three:
            rows.append(row("peg3-out", "A", "outbound", 30_000_000_000, t3, fee=500_000, rate="70000"))
            rows.append(row("peg3-in", "L", "inbound", 30_000_000_000, t3 if same_time else "2024-08-01T01:00:00Z", rate="70000", asset="LBTC"))
            pairs.append(pair("rail-3", "peg3-out", "peg3-in", kind="peg-in"))
        rows.append(row("sell", "A", "outbound", 10_000_000_000, "2024-09-01T00:00:00Z", rate="70000", kind="sell"))
        return rows, pairs

    for same_time in (False, True):
        for three in (False, True):
            rows, pairs = hops(same_time=same_time, three=three)
            name = f"carry-{'three' if three else 'two'}-hop-{'same-time' if same_time else 'spaced'}"
            cases.append(_book(name, ("generic", "carry", "fee"), fifo, rows, pairs, succeeds(entry_types=("disposal",))))
    rows, pairs = hops(same_time=False, three=False)
    cases.append(_book("carry-two-hop-hifo", ("generic", "carry", "fee"), profile("generic", "HIFO"), rows, pairs, succeeds(entry_types=("disposal",))))
    rows, pairs = hops(same_time=False, three=False)
    cases.append(_book("carry-two-hop-moving-average", ("generic", "carry", "fee"), profile("generic", "MOVING_AVERAGE"), rows, pairs, succeeds(entry_types=("disposal",))))
    rows, pairs = rail_history()
    pairs = [pair("rail-1", "peg-out", "peg-in", kind="peg-in", policy="taxable")]
    cases.append(_book("carry-taxable-rail", ("generic", "carry"), fifo, rows, pairs, succeeds(entry_types=("disposal",))))

    # D-carry-1: the carried peg-in is unpriced; hop 2 blocks for good on pass 1.
    rows = [
        row("buy", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        row("peg-out", "C", "outbound", 60_000_000_000, "2024-05-01T00:00:00Z", rate="60000"),
        row("peg-in", "L", "inbound", 60_000_000_000, "2024-05-01T01:00:00Z", rate=None, asset="LBTC"),
        row("peg2-out", "L", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", rate="65000", asset="LBTC"),
        row("peg2-in", "A", "inbound", 50_000_000_000, "2024-06-01T01:00:00Z", rate="65000"),
        row("sell", "A", "outbound", 10_000_000_000, "2024-07-01T00:00:00Z", rate="70000", kind="sell"),
    ]
    pairs = [pair("rail-1", "peg-out", "peg-in", kind="peg-in"), pair("rail-2", "peg2-out", "peg2-in", kind="peg-out")]
    cases.append(_book("carry-d1-unpriced-peg-in", ("generic", "carry", "d_carry"), fifo, rows, pairs, succeeds(reasons=("bitcoin_rail_carry_basis_unresolved", "insufficient_lots"))))
    # D-carry-2 and D-carry-3: a same-instant round trip. Under the moving
    # average the passes run out before the carries converge, so the
    # carrying-value LBTC peg-out books a gain; under FIFO they converge.
    stamp = "2024-05-01T00:00:00Z"
    rows = [
        row("buy", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        row("buy2", "C", "inbound", 100_000_000_000, "2024-02-01T00:00:00Z", rate="50000", kind="buy"),
        row("peg-out", "C", "outbound", 150_000_000_000, stamp, rate="60000"),
        row("peg-in", "L", "inbound", 150_000_000_000, stamp, rate="60000", asset="LBTC"),
        row("peg2-out", "L", "outbound", 150_000_000_000, stamp, rate="60000", asset="LBTC"),
        row("peg2-in", "A", "inbound", 150_000_000_000, stamp, rate="60000"),
        row("sell", "A", "outbound", 10_000_000_000, "2024-07-01T00:00:00Z", rate="70000", kind="sell"),
    ]
    cases.append(
        _book(
            "carry-d2-round-trip-moving-average",
            ("generic", "carry", "d_carry"),
            profile("generic", "MOVING_AVERAGE"),
            rows,
            pairs,
            succeeds(gains={"peg-out": False, "peg2-out": True}),
        )
    )
    cases.append(_book("carry-d3-round-trip-fifo", ("generic", "carry", "d_carry"), fifo, rows, pairs, succeeds(gains={"peg-out": False, "peg2-out": False})))

    # Direct payouts: a rail payout carrying value, and a taxable one.
    source = [
        row("btc-buy", "C", "inbound", 100_000_000_000, "2024-06-01T10:00:00Z", rate="10000", kind="buy"),
        row("payout-source", "C", "outbound", 50_000_000_000, "2025-03-01T09:00:00Z", rate="40000", fiat_value="20000", kind="withdrawal"),
        row("btc-sell", "C", "outbound", 10_000_000_000, "2025-04-01T09:00:00Z", rate="45000", kind="sell"),
    ]
    for policy, asset in (("carrying-value", "LBTC"), ("taxable", "BTC")):
        payout = _payout("payout-1", "payout-source", asset=asset, policy=policy, amount=49_990_000_000, value="24995")
        cases.append(_book(f"carry-direct-payout-{policy}", ("generic", "carry", "direct_payout"), fifo, source, check=succeeds(), payouts=[payout]))
    # A rail payout whose source has no basis to carry is blocked for good:
    # its source is short of lots (the gate's reason) or unpriced (normalize
    # drops it). Both settlement legs are quarantined, and none of its rows
    # books, the target-asset disposal ``<pair>:out`` included.
    blocked = _quarantined(
        (("payout-source", "bitcoin_rail_carry_basis_unresolved"), ("payout-source", "bitcoin_rail_carry_basis_unresolved")),
        ("acquisition", "disposal"),
    )
    for variant, rows in (
        ("short-source", [row("btc-buy", "C", "inbound", 40_000_000_000, "2024-06-01T10:00:00Z", rate="10000", kind="buy"), *source[1:]]),
        ("unpriced-source", [source[0], row("payout-source", "C", "outbound", 50_000_000_000, "2025-03-01T09:00:00Z", rate=None, fiat_value=None, kind="withdrawal"), source[2]]),
    ):
        payout = _payout("payout-1", "payout-source", asset="LBTC", policy="carrying-value", amount=49_990_000_000, value="24995")
        cases.append(_book(f"carry-direct-payout-{variant}", ("generic", "carry", "direct_payout"), fifo, rows, check=blocked, payouts=[payout]))

    # The method gate inside the carry's first pass, before any later stage.
    rows, pairs = rail_history()
    cases.append(_book("carry-method-bogus", ("generic", "carry", "method", "error"), profile("generic", "bogus"), rows, pairs, fails("RP2 multi-asset tax calculation failed: Unsupported RP2 accounting method 'bogus'")))
    # A pair the first pass prices and a later pass blocks: hop 1 is short
    # in its source wallet, so it is blocked on pass 1, and its Liquid
    # receipt, booked at market on pass 1, leaves pass 2; hop 2 spent that
    # receipt, carried on pass 1, and is short on pass 2. Blocks are for
    # good, so both pairs are quarantined and neither books.
    rows = [
        row("buy", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        row("peg-out", "C", "outbound", 150_000_000_000, "2024-05-01T00:00:00Z", rate="60000"),
        row("peg-in", "L", "inbound", 150_000_000_000, "2024-05-01T01:00:00Z", rate="60000", asset="LBTC"),
        row("peg2-out", "L", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", rate="65000", asset="LBTC"),
        row("peg2-in", "A", "inbound", 50_000_000_000, "2024-06-01T01:00:00Z", rate="65000"),
        row("sell", "C", "outbound", 10_000_000_000, "2024-07-01T00:00:00Z", rate="70000", kind="sell"),
    ]
    pairs = [pair("rail-1", "peg-out", "peg-in", kind="peg-in"), pair("rail-2", "peg2-out", "peg2-in", kind="peg-out")]
    cases.append(_book("carry-priced-then-blocked", ("generic", "carry"), fifo, rows, pairs, succeeds(counts={"bitcoin_rail_carry_basis_unresolved": 4}, entry_types=("disposal",))))
    # The pricing quirk of `generic-pricing-releases-a-sale-*` through the
    # carry. The first pass books every receipt at market, so the sale and
    # the last peg both book; pair x (short in its source) and, while the
    # Liquid receipt is unpriced, the peg back (short in Liquid) are blocked
    # there for good, and their receipts leave the later passes. Unpriced,
    # Cold then holds too little for the sale, and the last peg books;
    # priced, the peg back carries, the sale books, and the last peg is
    # short, so the carry blocks it: pricing adds its two quarantines.
    for priced in (False, True):
        rows = [
            row("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
            row("buy-b", "B", "inbound", 50_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
            row("x-out", "C", "outbound", 100_000_000_000, "2024-01-06T00:00:00Z", rate="45000", asset="LBTC"),
            row("x-in", "A", "inbound", 100_000_000_000, "2024-01-06T01:00:00Z", rate="45000"),
            row("peg-out", "B", "outbound", 50_000_000_000, "2024-01-11T00:00:00Z", rate="46000"),
            row("peg-in", "L", "inbound", 50_000_000_000, "2024-01-11T01:00:00Z", rate="46000" if priced else None, asset="LBTC"),
            row("peg2-out", "L", "outbound", 50_000_000_000, "2024-01-21T00:00:00Z", rate="47000", asset="LBTC"),
            row("peg2-in", "A", "inbound", 50_000_000_000, "2024-01-21T01:00:00Z", rate="47000"),
            row("sale", "A", "outbound", 120_000_000_000, "2024-01-31T00:00:00Z", rate="50000", kind="sell"),
            row("peg3-out", "A", "outbound", 80_000_000_000, "2024-02-10T00:00:00Z", rate="55000"),
            row("peg3-in", "L", "inbound", 80_000_000_000, "2024-02-10T01:00:00Z", rate="55000", asset="LBTC"),
        ]
        pairs = [
            pair("rail-x", "x-out", "x-in", kind="peg-out"),
            pair("rail-1", "peg-out", "peg-in", kind="peg-in"),
            pair("rail-2", "peg2-out", "peg2-in", kind="peg-out"),
            pair("rail-3", "peg3-out", "peg3-in", kind="peg-in"),
        ]
        rail = "bitcoin_rail_carry_basis_unresolved"
        blocked_x = (("x-out", rail), ("x-in", rail))
        if priced:
            quarantined = (*blocked_x, ("peg3-out", rail), ("peg3-in", rail))
            entry_types = ("acquisition", "acquisition", "acquisition", "disposal", "disposal", "acquisition", "disposal")
        else:
            quarantined = (*blocked_x, ("peg2-out", rail), ("peg2-in", rail), ("sale", "insufficient_lots"))
            entry_types = ("acquisition", "acquisition", "disposal", "disposal", "acquisition", "acquisition")
        name = f"carry-pricing-releases-a-sale-{'priced' if priced else 'unpriced'}"
        cases.append(_book(name, ("generic", "carry", "pricing_quirk"), fifo, rows, pairs, _quarantined(quarantined, entry_types)))
    return cases


def _payout(payout_id: str, source_id: str, *, asset: str, policy: str, amount: int, value: str, at: str = "2025-03-01T09:00:30Z") -> dict[str, Any]:
    return {
        "id": payout_id,
        "out_transaction_id": source_id,
        "kind": "direct-swap-payout",
        "policy": policy,
        "payout_asset": asset,
        "payout_amount": amount,
        "payout_occurred_at": at,
        "payout_fiat_value": value,
        "payout_external_id": f"{payout_id}-recipient",
        "counterparty": "external-recipient",
        "notes": "Reviewed swap payout",
        "swap_fee_msat": 10_000_000,
        "swap_fee_kind": "combined",
        "created_at": at,
    }


def _at_cases() -> list[GoldenCase]:
    cases: list[GoldenCase] = []
    for method in AT_METHODS:
        rows, pairs = ledger_history()
        cases.append(_book(f"at-ledger-{method}", ("at", "method", "fee"), profile("at", method), rows, pairs, succeeds(entry_types=("disposal", "transfer_fee"))))
        rows, pairs = rail_history()
        # Austria carries a rail pair as a swap, which only the moving
        # average honors; the lot methods abort.
        check = succeeds() if method == "moving_average_at" else fails("Austrian swap neutrality requires the `moving_average_at` accounting method")
        cases.append(_book(f"at-rail-{method}", ("at", "method", "carry"), profile("at", method), rows, pairs, check))
    mat = profile("at", "moving_average_at")

    # Alt, Neu and mixed wallets: A only Alt, B only Neu, C both (a sale
    # there elects Neu: Wahlrecht).
    regimes = [
        row("alt-a", "A", "inbound", 20_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
        row("alt-c", "C", "inbound", 20_000_000_000, "2020-07-01T00:00:00Z", rate="9000", kind="buy"),
        row("neu-b", "B", "inbound", 20_000_000_000, "2022-01-01T00:00:00Z", rate="40000", kind="buy"),
        row("neu-c", "C", "inbound", 20_000_000_000, "2022-02-01T00:00:00Z", rate="38000", kind="buy"),
        row("sell-a", "A", "outbound", 5_000_000_000, "2023-01-01T00:00:00Z", fee=1_000_000, rate="20000", kind="sell"),
        row("sell-b", "B", "outbound", 5_000_000_000, "2023-01-02T00:00:00Z", rate="20000", kind="sell"),
        row("sell-c", "C", "outbound", 5_000_000_000, "2023-01-03T00:00:00Z", rate="20000", kind="sell"),
        row("sell-alt-early", "A", "outbound", 1_000_000_000, "2020-12-01T00:00:00Z", rate="15000", kind="sell"),
    ]
    for method in AT_METHODS:
        cases.append(
            _book(
                f"at-alt-neu-wallets-{method}",
                ("at", "regime", "wahlrecht", "fee"),
                profile("at", method),
                regimes,
                check=succeeds(descriptions=("at_regime=alt", "at_regime=neu", "at_regime_basis=wahlrecht")),
            )
        )
    # Alt coins moved A -> B (with a fee) after the cutoff and sold from B;
    # B's later Neu buy makes its next sale an election.
    moved = [
        row("alt-a", "A", "inbound", 20_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
        row("move-out", "A", "outbound", 15_000_000_000, "2022-06-01T00:00:00Z", fee=2_000_000, rate="30000"),
        row("move-in", "B", "inbound", 15_000_000_000, "2022-06-01T00:10:00Z", rate="30000"),
        row("sell-b", "B", "outbound", 10_000_000_000, "2023-01-01T00:00:00Z", rate="20000", kind="sell"),
        row("neu-b", "B", "inbound", 5_000_000_000, "2023-02-01T00:00:00Z", rate="22000", kind="buy"),
        row("sell-b-2", "B", "outbound", 1_000_000_000, "2023-03-01T00:00:00Z", rate="25000", kind="sell"),
        row("sell-a", "A", "outbound", 1_000_000_000, "2023-01-02T00:00:00Z", rate="20000", kind="sell"),
    ]
    for method in ("moving_average_at", "fifo"):
        cases.append(
            _book(
                f"at-transfer-moved-availability-{method}",
                ("at", "regime", "move", "fee"),
                profile("at", method),
                moved,
                [pair("pair-1", "move-out", "move-in")],
                succeeds(descriptions=("at_alt_out=", "at_alt_in=", "at_regime_basis=wahlrecht"), entry_types=("transfer_fee",)),
            )
        )
    # Fee first: a move out of a mixed wallet pays its fee from the Neu
    # coins, so only the rest of the Neu reaches B; B's first sale uses it up
    # and the next one is Alt.
    fee_first = [
        row("buy-alt", "A", "inbound", 10_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
        row("buy-neu", "A", "inbound", 1_000_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="buy"),
        row("move-out", "A", "outbound", 10_000_000_000, "2025-01-01T00:00:00Z", fee=500_000_000, rate="90000"),
        row("move-in", "B", "inbound", 10_000_000_000, "2025-01-01T00:10:00Z", rate="90000"),
        row("sell-neu-remainder", "B", "outbound", 500_000_000, "2025-06-01T00:00:00Z", rate="95000", kind="sell"),
        row("sell-after-neu-fee", "B", "outbound", 100_000_000, "2025-06-02T00:00:00Z", rate="95000", kind="sell"),
    ]
    for method in ("moving_average_at", "fifo"):
        cases.append(
            _book(
                f"at-move-fee-first-{method}",
                ("at", "regime", "move", "fee"),
                profile("at", method),
                fee_first,
                [pair("pair-1", "move-out", "move-in")],
                succeeds(regimes={"move-out": "neu", "sell-neu-remainder": "neu", "sell-after-neu-fee": "alt"}),
            )
        )
    # Fees under a lot method: a fee-only spend and a move fee, each taking
    # the rest of an Alt lot and the start of a Neu lot, so each splits
    # into two fee rows by category before the category is cleared.
    spanning = [
        row("alt-a", "A", "inbound", 10_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
        row("sell-alt", "A", "outbound", 9_999_000_000, "2022-01-01T00:00:00Z", rate="40000", kind="sell"),
        row("neu-a", "A", "inbound", 10_000_000_000, "2022-02-01T00:00:00Z", rate="40000", kind="buy"),
        row("fee-only", "A", "outbound", 0, "2022-06-01T00:00:00Z", fee=3_000_000, rate="30000"),
        row("alt-b", "B", "inbound", 10_000_000_000, "2020-07-01T00:00:00Z", rate="9000", kind="buy"),
        row("sell-alt-b", "B", "outbound", 9_999_000_000, "2022-01-02T00:00:00Z", rate="40000", kind="sell"),
        row("neu-b", "B", "inbound", 10_000_000_000, "2022-02-02T00:00:00Z", rate="40000", kind="buy"),
        row("move-out", "B", "outbound", 5_000_000_000, "2022-07-01T00:00:00Z", fee=3_000_000, rate="30000"),
        row("move-in", "C", "inbound", 5_000_000_000, "2022-07-01T00:10:00Z", rate="30000"),
    ]

    def two_fee_rows(result: Optional[TaxEngineLedgerResult], error: Optional[BaseException]) -> list[str]:
        if error is not None:
            return [f"expected success, got {type(error).__name__}: {error}"]
        assert result is not None
        fees = [entry["transaction_id"] for entry in result.entries if entry["entry_type"] in ("fee", "transfer_fee")]
        return [] if fees.count("fee-only") == 2 else [f"expected two fee rows for fee-only, got {fees}"]

    cases.append(_book("at-fee-spanning-alt-neu-fifo", ("at", "regime", "fee", "move"), profile("at", "fifo"), spanning, [pair("pair-1", "move-out", "move-in")], two_fee_rows))

    # Overrides: regime, category (including none), taxability.
    overrides = [
        row("alt-a", "A", "inbound", 20_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
        row("neu-a", "A", "inbound", 20_000_000_000, "2022-01-01T00:00:00Z", rate="40000", kind="buy"),
        row("sell-regime", "A", "outbound", 2_000_000_000, "2023-01-01T00:00:00Z", rate="20000", kind="sell", at_regime_override="alt"),
        row("sell-category", "A", "outbound", 2_000_000_000, "2023-01-02T00:00:00Z", rate="20000", kind="sell", at_category_override="neu_gain"),
        row("sell-none", "A", "outbound", 2_000_000_000, "2023-01-03T00:00:00Z", rate="20000", kind="sell", at_category_override="none"),
        row("sell-off", "A", "outbound", 2_000_000_000, "2023-01-04T00:00:00Z", rate="20000", kind="sell", taxability_override=0),
        row("sell-off-taxfree", "A", "outbound", 2_000_000_000, "2023-01-05T00:00:00Z", rate="20000", kind="sell", taxability_override=0, at_category_override="alt_taxfree"),
        row("income", "A", "inbound", 1_000_000_000, "2023-02-01T00:00:00Z", rate="21000", kind="staking", at_category_override="income_capital_yield"),
        row("sell-late", "A", "outbound", 2_000_000_000, "2023-03-01T00:00:00Z", fee=1_000_000, rate="25000", kind="sell"),
    ]
    for method in ("moving_average_at", "fifo"):
        cases.append(
            _book(
                f"at-overrides-{method}",
                ("at", "overrides", "fee"),
                profile("at", method),
                overrides,
                check=succeeds(categories=(None, "neu_gain", "alt_taxfree")),
            )
        )
    cases.append(
        _book(
            "at-airdrop-hardfork",
            ("at", "income"),
            mat,
            [
                row("buy", "A", "inbound", 10_000_000_000, "2022-01-01T00:00:00Z", rate="40000", kind="buy"),
                row("airdrop", "A", "inbound", 1_000_000_000, "2022-02-01T00:00:00Z", rate="41000", kind="airdrop"),
                row("hardfork", "A", "inbound", 1_000_000_000, "2022-03-01T00:00:00Z", rate="42000", kind="hard_fork"),
                row("mined", "A", "inbound", 1_000_000_000, "2022-04-01T00:00:00Z", rate="43000", kind="mining"),
                row("sell", "A", "outbound", 2_000_000_000, "2022-06-01T00:00:00Z", rate="30000", kind="sell"),
            ],
            check=succeeds(reasons=("acquisition_valuation_unsupported",)),
        )
    )

    # Swaps: Neu (carry), Alt (skipped), a chained pair whose first hop is
    # 1 msat short (the Unpaired abort) and the same chain without it,
    # and direct payouts.
    def swap_book(*, alt: bool) -> list[dict[str, Any]]:
        when = "2020-06-01T00:00:00Z" if alt else "2024-06-01T00:00:00Z"
        return [
            row("buy-btc", "C", "inbound", 100_000_000_000, "2020-01-01T00:00:00Z" if alt else "2024-01-01T00:00:00Z", rate="9000" if alt else "40000", kind="buy"),
            row("swap-out", "C", "outbound", 50_000_000_000, when, fee=1_000_000, rate="60000", kind="sell"),
            row("swap-in", "C", "inbound", 30_000_000_000_000, when, rate="1", kind="buy", asset="USDT"),
            row("sell-usdt", "C", "outbound", 10_000_000_000_000, "2024-09-01T00:00:00Z", rate="1.1", kind="sell", asset="USDT"),
            row("sell-btc", "C", "outbound", 10_000_000_000, "2024-10-01T00:00:00Z", rate="65000", kind="sell"),
        ]

    for method in AT_METHODS:
        cases.append(
            _book(
                f"at-neu-swap-{method}",
                ("at", "swap", "fee"),
                profile("at", method),
                swap_book(alt=False),
                [pair("swap-1", "swap-out", "swap-in")],
                succeeds(descriptions=("at_swap_link=swap-1",))
                if method == "moving_average_at"
                else fails("Austrian swap neutrality requires the `moving_average_at` accounting method"),
            )
        )
    cases.append(_book("at-alt-swap", ("at", "swap"), mat, swap_book(alt=True), [pair("swap-1", "swap-out", "swap-in")], succeeds()))
    for short in (0, 1):
        rows = [
            row("btc-buy", "C", "inbound", 10_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
            row("swap1-out", "C", "outbound", 10_000_000_000 + short, "2024-02-01T00:00:00Z", rate="40000"),
            row("swap1-in", "C", "inbound", 400_000_000_000, "2024-02-01T00:00:00Z", rate="1", asset="USDT"),
            row("swap2-out", "C", "outbound", 400_000_000_000, "2024-03-01T00:00:00Z", rate="1", asset="USDT"),
            row("swap2-in", "L", "inbound", 9_000_000_000, "2024-03-01T00:00:00Z", rate="44000", asset="LBTC"),
        ]
        pairs = [pair("s1", "swap1-out", "swap1-in"), pair("s2", "swap2-out", "swap2-in")]
        if short:
            check = fails("RP2 cross-asset input validation failed: Unpaired `at_swap_link=s2`")
        else:
            check = succeeds(descriptions=("at_swap_link=s1", "at_swap_link=s2"))
        cases.append(_book(f"at-chained-swap{'-unpaired-abort' if short else ''}", ("at", "swap", "chain"), mat, rows, pairs, check))
    # A link the first pass chose on its regime: a swap-in makes the wallet
    # hold Neu, so the sale is Neu and linked; the swap-in's own pair is
    # blocked (no USDT to swap from), so the second pass sees only Alt, where
    # the sale is Alt and no swap leg, and its in leg is left unpaired.
    rows = [
        row("alt", "A", "inbound", 100_000_000_000, "2020-06-01T00:00:00Z", rate="9000", kind="buy"),
        row("usdt-short", "A", "outbound", 3_000_000_000_000_000, "2024-01-01T00:00:00Z", rate="1", kind="sell", asset="USDT"),
        row("neu-in", "A", "inbound", 50_000_000_000, "2024-01-01T00:00:00Z", kind="buy"),
        row("swap-out", "A", "outbound", 30_000_000_000, "2024-06-01T00:00:00Z", rate="60000", kind="sell"),
        row("swap-in", "A", "inbound", 1_800_000_000_000_000, "2024-06-01T00:00:00Z", rate="1", kind="buy", asset="USDT"),
    ]
    pairs = [pair("blocked", "usdt-short", "neu-in"), pair("kept", "swap-out", "swap-in")]
    cases.append(_book("at-swap-link-turns-alt-abort", ("at", "swap", "chain"), mat, rows, pairs, fails("RP2 cross-asset input validation failed: Unpaired `at_swap_link=kept`")))
    payout_source = [
        row("lbtc-buy", "L", "inbound", 100_000_000_000, "2024-06-01T10:00:00Z", rate="30000", fiat_value="30000", asset="LBTC"),
        row("payout-source", "L", "outbound", 50_000_000_000, "2025-03-01T09:00:00Z", rate="50000", fiat_value="25000", asset="LBTC", kind="withdrawal"),
        row("btc-buy", "L", "inbound", 10_000_000_000, "2025-03-02T09:00:00Z", rate="51000", fiat_value="5100", kind="buy"),
        row("btc-sell", "L", "outbound", 5_000_000_000, "2025-04-02T09:00:00Z", rate="52000", kind="sell"),
    ]
    for policy in ("carrying-value", "taxable"):
        payout = _payout("payout-1", "payout-source", asset="BTC", policy=policy, amount=49_990_000_000, value="24995")
        cases.append(_book(f"at-direct-payout-{policy}", ("at", "swap", "direct_payout"), mat, payout_source, check=succeeds(), payouts=[payout]))
    # A direct payout whose source cannot carry is blocked: its source is
    # short (the gate's reason) or unpriced (normalize drops it). Only the
    # source is quarantined, and neither settlement leg books, the
    # target-asset disposal ``<pair>:out`` included. A payout out of
    # Altvermögen is no Neu swap: it books as a disposal and an acquisition,
    # and its target-asset disposal books too.
    def payout_book(*, held: int = 100_000_000_000, rate: Optional[str] = "50000", value: Optional[str] = "25000", alt: bool = False) -> list[dict[str, Any]]:
        return [
            row("lbtc-buy", "L", "inbound", held, "2020-01-01T10:00:00Z" if alt else "2024-06-01T10:00:00Z", rate="30000", asset="LBTC"),
            row("payout-source", "L", "outbound", 50_000_000_000, "2020-06-01T09:00:00Z" if alt else "2025-03-01T09:00:00Z", rate=rate, fiat_value=value, asset="LBTC", kind="withdrawal"),
            *payout_source[2:],
        ]

    blocked = _quarantined((("payout-source", "at_swap_basis_carry_unresolved"),), ("acquisition", "acquisition", "disposal"))
    for variant, rows, check, at in (
        ("short-source", payout_book(held=40_000_000_000), blocked, "2025-03-01T09:00:30Z"),
        ("unpriced-source", payout_book(rate=None, value=None), blocked, "2025-03-01T09:00:30Z"),
        ("alt-source", payout_book(alt=True), succeeds(absent=("at_swap_basis_carry_unresolved",), regimes={"payout-source": "alt"}), "2020-06-01T09:00:30Z"),
    ):
        payout = _payout("payout-1", "payout-source", asset="BTC", policy="carrying-value", amount=49_990_000_000, value="24995", at=at)
        cases.append(_book(f"at-direct-payout-{variant}", ("at", "swap", "direct_payout"), mat, rows, check=check, payouts=[payout]))

    # The pass-B overdraw: a promoted swap-out bypasses the gate on pass B
    # after pass B dropped the 1-msat swap-in that funded it on pass A.
    def overdraw(variant: str) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
        rows = [
            row("buy-a", "A", "inbound", 40_000_000_000, "2024-01-01T00:00:00Z", rate="40000", kind="buy"),
            row("buy-c", "C", "inbound", 100_000_000_000, "2024-01-02T00:00:00Z", rate="40000", kind="buy"),
            row("usdt-out", "A", "outbound", 10_000_000_000_000, "2024-03-01T00:00:00Z", rate="1", kind="sell", asset="USDT"),
            row("btc-in", "A", "inbound", 1, "2024-03-01T00:00:00Z", rate="50000", kind="buy"),
            row("swap-out", "A", "outbound", 40_000_000_001, "2024-06-01T00:00:00Z", rate="60000", kind="sell"),
            row("swap-in", "A", "inbound", 24_000_000_000_000, "2024-06-01T00:00:00Z", rate="1", kind="buy", asset="USDT"),
        ]
        pairs = [pair("swap-0", "usdt-out", "btc-in"), pair("swap-1", "swap-out", "swap-in")]
        if variant == "sale":
            rows.append(row("sell-c", "C", "outbound", 100_000_000_000, "2024-09-01T00:00:00Z", rate="70000", kind="sell"))
        else:
            rows.append(row("move-out", "C", "outbound", 100_000_000_000, "2024-09-01T00:00:00Z", rate="70000"))
            rows.append(row("move-in", "B", "inbound", 100_000_000_000, "2024-09-01T00:10:00Z", rate="70000"))
            pairs.append(pair("pair-m", "move-out", "move-in"))
        return rows, pairs

    # The sale and the out leg of either move reach missing_cost_basis (the
    # outbound and the transfer gate); a grouped move blocks its in leg, an
    # ungrouped one quarantines it with the same reason.
    for variant, check in (
        ("sale", succeeds(counts={"missing_cost_basis": 1})),
        ("move", succeeds(counts={"missing_cost_basis": 1, "derived_transfer_group_blocked": 1})),
        ("ungrouped-move", succeeds(counts={"missing_cost_basis": 2}, absent=("derived_transfer_group_blocked",))),
    ):
        rows, pairs = overdraw(variant)
        inputs = finalized(mat, rows, pairs)
        if variant == "ungrouped-move":
            _ungroup(inputs)
        cases.append(_case(f"at-pass-b-overdraw-{variant}", ("at", "swap", "gate"), mat, inputs, check))

    # The Neu shortfall: a sale tagged Neu because the wallet holds some Neu,
    # which needs more than the Neu pool has.
    cases.append(
        _book(
            "at-neu-shortfall-abort",
            ("at", "regime"),
            mat,
            [
                row("alt", "A", "inbound", 20_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("neu", "A", "inbound", 1_000_000_000, "2022-01-01T00:00:00Z", rate="40000", kind="buy"),
                row("sell", "A", "outbound", 5_000_000_000, "2023-01-01T00:00:00Z", rate="20000", kind="sell"),
            ],
            check=fails("RP2 tax calculation failed for asset 'BTC'"),
        )
    )
    # Pricing can release the Neu shortfall (an oracle quirk the Austrian
    # pricing property excuses): a move out of an Alt-only wallet whose
    # regime the user set to Neu is held while its fee has no price; priced
    # by its in leg, it books a Neu fee no Neu lot covers.
    for variant, in_rate, check in (
        ("unpriced", None, _quarantined((("move-out", "missing_spot_price"), ("move-in", "missing_spot_price")), ("acquisition",))),
        ("priced", "40000", fails("RP2 tax calculation failed for asset 'BTC': Total in-transaction crypto value < total taxable crypto value")),
    ):
        rows = [
            row("alt-buy", "C", "inbound", 100_000_000_000, "2021-01-01T00:00:00Z", rate="30000", kind="buy"),
            row("move-out", "C", "outbound", 50_000_000_000, "2021-02-01T00:00:00Z", fee=100_000_000, rate=None, at_regime_override="neu"),
            row("move-in", "B", "inbound", 50_000_000_000, "2021-02-01T00:10:00Z", rate=in_rate),
        ]
        cases.append(_book(f"at-pricing-releases-a-neu-shortfall-{variant}", ("at", "regime", "pricing"), mat, rows, [pair("pair-1", "move-out", "move-in")], check))

    # User text with marker tokens keeps today's effect (design §3.6).
    cases.append(_usertag_swap("at-usertag-swap-moving-average-at", "at", "moving_average_at", tagged=True, check=succeeds(categories=("neu_swap",))))
    cases.append(_usertag_swap("at-usertag-swap-untagged", "at", "moving_average_at", tagged=False, check=succeeds(categories=("neu_gain",))))
    cases.append(_usertag_swap("at-usertag-swap-fifo", "at", "fifo", tagged=True, check=fails("RP2 multi-asset tax calculation failed: Austrian swap neutrality requires the `moving_average_at` accounting method")))
    single = [
        row("buy", "A", "inbound", 10_000_000_000, "2022-01-01T00:00:00Z", rate="40000", kind="buy"),
        row("sell", "A", "outbound", 1_000_000_000, "2023-01-01T00:00:00Z", rate="20000", kind="sell", note="paid at_swap_link=lone"),
    ]
    cases.append(_book("at-usertag-single-swap-link", ("at", "marker_text"), mat, single, check=fails("RP2 cross-asset input validation failed: Unpaired")))
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] == "sell-2":
            item["note"] = "at_regime=neu duplicate"
    # Under a lot method nothing parses a sale's regime marker, so the
    # duplicate stays in its description and the book succeeds.
    cases.append(
        _book(
            "at-usertag-duplicate-regime-unconsulted",
            ("at", "marker_text"),
            profile("at", "fifo"),
            rows,
            pairs,
            succeeds(descriptions=("at_regime=neu at_pool=default at_regime=neu duplicate",)),
        )
    )
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] == "move-in":
            # A move's text comes from its out leg, so nothing reads these
            # tokens; read, either would abort.
            item["note"] = "at_regime=alt at_swap_link=in-leg"
    cases.append(_book("at-usertag-move-in-note-unread", ("at", "marker_text", "move"), mat, rows, pairs, succeeds()))
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] == "buy-1":
            item["note"] = "at_regime=neu duplicate"
    cases.append(_book("at-usertag-duplicate-regime-consulted", ("at", "marker_text"), mat, rows, pairs, fails("Duplicate `at_regime=neu` markers")))
    # Under a lot method the acquisition's constructor already rejects it.
    cases.append(
        _book(
            "at-usertag-duplicate-regime-fifo",
            ("at", "marker_text"),
            profile("at", "fifo"),
            rows,
            pairs,
            fails("Duplicate `at_regime=neu` markers", error_class="RP2ValueError"),
        )
    )
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] == "sell-2":
            item["note"] = "at_regime=alt conflicting"
    cases.append(_book("at-usertag-conflicting-regime", ("at", "marker_text"), mat, rows, pairs, fails("Conflicting `at_regime` markers")))
    rows, pairs = ledger_history()
    for item in rows:
        if item["id"] == "move-out":
            item["note"] = "consolidation at_swap_link=m1"
    cases.append(_book("at-usertag-move-swap-link", ("at", "marker_text", "move"), mat, rows, pairs, fails("`at_swap_link=` marker on non-SELL disposal")))
    # Under a lot method nothing computes with the move's tag: classifying
    # its fee reads it (a Neu swap, which a fee then drops), and a repeated
    # tag fails that classification, unwrapped.
    for name, note, check in (
        ("at-usertag-move-swap-link-fifo", "consolidation at_swap_link=m1", succeeds(entry_types=("transfer_fee",), descriptions=("consolidation at_swap_link=m1",))),
        ("at-usertag-move-duplicate-swap-link-fifo", "at_swap_link=m1 at_swap_link=m2", fails("Duplicate `at_swap_link=` markers", error_class="RP2ValueError")),
    ):
        rows, pairs = ledger_history()
        for item in rows:
            if item["id"] == "move-out":
                item["note"] = note
        cases.append(_book(name, ("at", "marker_text", "move"), profile("at", "fifo"), rows, pairs, check))

    # The other methods an Austrian profile may pick. Only the Austrian
    # moving average keeps a Neu swap neutral, so the rail and swap books
    # abort under each.
    neutrality = fails("Austrian swap neutrality requires the `moving_average_at` accounting method")
    for method in AT_EXTRA_METHODS:
        method_profile = profile("at", method)
        rows, pairs = ledger_history()
        cases.append(_book(f"at-ledger-{method}", ("at", "method", "fee"), method_profile, rows, pairs, succeeds(entry_types=("disposal", "transfer_fee"))))
        rows, pairs = rail_history()
        cases.append(_book(f"at-rail-{method}", ("at", "method", "carry"), method_profile, rows, pairs, neutrality))
        cases.append(
            _book(
                f"at-alt-neu-wallets-{method}",
                ("at", "method", "regime", "wahlrecht", "fee"),
                method_profile,
                regimes,
                check=succeeds(descriptions=("at_regime=alt", "at_regime=neu", "at_regime_basis=wahlrecht")),
            )
        )
        cases.append(_book(f"at-neu-swap-{method}", ("at", "method", "swap", "fee"), method_profile, swap_book(alt=False), [pair("swap-1", "swap-out", "swap-in")], neutrality))
    return cases


def _error_cases() -> list[GoldenCase]:
    cases: list[GoldenCase] = []
    rows, pairs = ledger_history()
    cases.append(
        _book(
            "error-invalid-currency",
            ("generic", "error", "configuration"),
            profile("generic", "FIFO", fiat_currency="ZZZ"),
            rows,
            pairs,
            fails("non-ISO-4217 format value ZZZ", error_class="RP2ValueError"),
        )
    )

    def relabeled(label: str, wallet: str = "C", **profile_extra: Any) -> tuple[dict[str, Any], TaxEngineLedgerInputs]:
        refs = {key: dict(value) for key, value in WALLET_REFS.items()}
        refs[wallet]["label"] = label
        book_profile = profile("generic", "FIFO", **profile_extra)
        return book_profile, finalized(book_profile, rows, pairs, wallet_refs=refs)

    for name, label, check in (
        ("error-forbidden-character", "Cold, Savings", fails("wallet_label contains unsupported characters")),
        ("error-percent", "50% cold", fails("%", error_class="InterpolationSyntaxError")),
        ("error-duplicate-after-strip", " Cold", fails("duplicate elements", error_class="RP2ValueError")),
    ):
        book_profile, inputs = relabeled(label)
        cases.append(_case(name, ("generic", "error", "configuration"), book_profile, inputs, check))
    book_profile = profile("generic", "FIFO", label="Holder: one")
    cases.append(
        _case(
            "error-holder-forbidden-character",
            ("generic", "error", "configuration"),
            book_profile,
            finalized(book_profile, rows, pairs),
            fails("profile_label contains unsupported characters"),
        )
    )
    return cases


def _ld5_cases() -> list[GoldenCase]:
    """One case per shape a finalized projection cannot have (design §3.5).

    Each is a small generic book built directly with exactly one such shape.
    Production's request builder refuses it before the walk (listed
    difference LD-5); the oracle's outcome is stored as it is.
    """
    book_profile = profile("generic", "FIFO")

    def book(rows: Sequence[Mapping[str, Any]], pairs: Sequence[Mapping[str, Any]] = (), **extra: Any) -> TaxEngineLedgerInputs:
        return direct(rows, pairs, **extra)

    def buy() -> dict[str, Any]:
        return projected("ld5-buy", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", kind="buy")

    def sale(**fields: Any) -> dict[str, Any]:
        return projected("ld5-sell", "C", "outbound", 10_000_000_000, "2024-06-01T00:00:00Z", fee=1_000_000, rate="60000", kind="sell", **fields)

    def move_legs(amount_in: int = 20_000_000_000, **out_fields: Any) -> tuple[dict[str, Any], dict[str, Any]]:
        out_row = projected("ld5-move-out", "C", "outbound", 20_000_000_000, "2024-03-01T00:00:00Z", side="move-out", fee=1_000_000, rate="50000", **out_fields)
        in_row = projected("ld5-move-in", "B", "inbound", amount_in, "2024-03-01T00:10:00Z", side="move-in", rate="50000")
        return out_row, in_row

    cases: list[GoldenCase] = []

    def add(invariant: str, inputs: TaxEngineLedgerInputs) -> None:
        cases.append(
            GoldenCase(
                f"ld5-{invariant.replace('_', '-')}",
                ("generic", "error", "ld5"),
                dict(book_profile),
                inputs,
                lambda result, error: [],
                invariant,
            )
        )

    refs = {key: dict(value) for key, value in WALLET_REFS.items()}
    refs["B"]["id"] = "B-renamed"
    add("wallet_ref_id_mismatch", book([buy(), sale()], wallet_refs=refs))
    plain = sale()
    plain["id"] = "ld5-sell"
    add("non_synthetic_row_id", book([buy(), plain]))
    unanchored = sale()
    unanchored["journal_transaction_id"] = ""
    add("missing_anchor", book([buy(), unanchored]))
    elsewhere = sale()
    elsewhere["wallet_id"] = "Z"
    add("unknown_wallet", book([buy(), elsewhere]))
    sideways = sale()
    sideways["direction"] = "sideways"
    add("unknown_direction", book([buy(), sideways]))
    fractional = sale()
    fractional["fee"] = 1_000_000.5
    add("non_integer_msat", book([buy(), fractional]))
    refund = sale()
    refund["fee"] = -1_000_000
    add("negative_fee", book([buy(), refund]))
    # Its fee keeps the outbound total positive; the negative principal is the shape.
    reversed_sale = sale()
    reversed_sale["amount"] = -1
    add("negative_amount", book([buy(), reversed_sale]))
    empty = projected("ld5-empty", "C", "inbound", 0, "2024-02-01T00:00:00Z", kind="buy")
    add("non_positive_amount", book([buy(), empty, sale()]))
    inclusive = sale()
    inclusive["amount_includes_fee"] = True
    add("amount_includes_fee", book([buy(), inclusive]))
    add("custody_component_fields", book([buy(), sale(custody_component_id="component-1")]))
    add("corrupt_decimal_text", book([buy(), sale(fiat_rate_exact="sixty thousand")]))
    out_row, in_row = move_legs()
    blocked = move(out_row, in_row)
    blocked["group_block_rows"] = (dict(out_row),)
    add("group_block_rows", book([buy(), out_row, in_row], [blocked]))
    out_row, in_row = move_legs(privacy_boundary="coinjoin")
    add("unreviewed_privacy_pair", book([buy(), out_row, in_row], [move(out_row, in_row)]))
    out_row, in_row = move_legs()
    second_in = projected("ld5-move-in-2", "A", "inbound", 20_000_000_000, "2024-03-01T00:20:00Z", side="move-in", rate="50000")
    add(
        "shared_pair_leg",
        book([buy(), out_row, in_row, second_in], [move(out_row, in_row), move(out_row, second_in, pair_id="move:second")]),
    )
    out_row, in_row = move_legs(amount_in=19_000_000_000)
    add("unequal_move_amounts", book([buy(), out_row, in_row], [move(out_row, in_row)]))
    return cases


# The seeds the goldens store from the random half.
GOLDEN_SEEDS = tuple(range(12))


def _seed_cases() -> list[GoldenCase]:
    cases = []
    for country in ("generic", "at"):
        for seed in GOLDEN_SEEDS:
            book = random_inputs(seed, country)
            cases.append(_case(f"{country}-seed-{seed}", (country, "random", *book.tags), book.profile, book.inputs, lambda result, error: []))
    return cases


def corpus() -> list[GoldenCase]:
    """Every named case the goldens store, in file order."""
    cases = [*_generic_cases(), *_holding_period_cases(), *_carry_cases(), *_at_cases(), *_error_cases(), *_ld5_cases(), *_seed_cases()]
    names = [case.name for case in cases]
    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        raise ValueError(f"duplicate golden case names: {duplicates}")
    return cases
