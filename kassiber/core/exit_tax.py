"""Exit-tax (deemed-disposal) report.

Estimates the tax a taxpayer would owe on the *unrealized* gains of the crypto
held in a book if they gave up tax residence on a chosen departure date. For
Austria this is the Wegzugsbesteuerung (§ 27 Abs 6 EStG): a fiktive Veräußerung
at fair market value of Neubestand holdings, taxed at the 27.5 % Sondersteuersatz
(§ 27a EStG); Altbestand realizations are tax-free and excluded.

Design (see docs/plan/11-exit-tax-deemed-disposal.md):

- This module owns NO tax math. It reads the stored journal projection RP2
  already computed and reconstructs the remaining inventory and its
  cost basis per regime directly from those entries: acquisitions add (by
  acquisition-date regime), disposals subtract (by their `at_category` regime
  and the engine's own consumed cost basis). Income recognition lines are
  skipped for quantity/basis — RP2 already books the matching acquisition lot
  for earn receipts, so counting income again would double the pool (same skip
  as holdings reports). Internal transfers create no acquisition/disposal
  entry, so they never touch the global pool. The number is therefore
  consistent by construction with "what your capital-gains report would show if
  you sold everything at FMV on the departure date".
- The 27.5 % rate is the only Austrian constant here; it is not file-ready tax,
  it is a headline estimate a Steuerberater reviews and stamps.

Not tax advice.
"""

from __future__ import annotations

import sqlite3
from datetime import datetime, timezone
from decimal import ROUND_HALF_UP, Decimal
from typing import Any, Mapping, Optional, Sequence

from ..errors import AppError
from ..tax_policy import build_tax_policy
from ..time_utils import parse_timestamp
from . import pricing
from . import report_context as core_report_context
from . import custody_journal
from . import rates as core_rates
from .journal_markers import MARKER_REGIME, parse_marker
from .austrian import infer_regime_from_timestamp, kennzahl_for_disposal_category
from ..msat import dec

# Austrian capital-gains special rate (§ 27a Abs 1 Z 2 EStG). Headline-estimate
# only; the deemed-disposal gain a Steuerberater files may differ. Kept here, not
# in TaxPolicy, because it is presentation/estimate logic, not engine tax math.
AT_SONDERSTEUERSATZ = Decimal("0.275")

DESTINATION_EU_EEA = "eu_eea"
DESTINATION_THIRD_COUNTRY = "third_country"

_SATS_PER_BTC = Decimal("100000000")
_CENT = Decimal("0.01")

# Engine-entry contract this report depends on (produced by
# kassiber/core/engines/rp2.py::_append_rp2_journal_entries). The walk below
# reasons explicitly about these entry_type values; an unrecognized type still
# behaves sanely (it is bucketed as an inflow/outflow by the sign of
# ``quantity``), but a drift guard test (tests/test_exit_tax.py) asserts the
# real engine emits only this set so a NEW type is caught at dev time rather
# than silently changing a taxpayer's estimate. Keep this in lockstep with the
# engine if entry types change.
RECOGNIZED_ENTRY_TYPES = frozenset(
    {
        "acquisition",
        "income",
        "disposal",
        "fee",
        "transfer_in",
        "transfer_out",
        "transfer_fee",
    }
)

EXIT_TAX_REVIEW_GATE = (
    "Estimate only — not tax advice. The exit-tax (Wegzugsbesteuerung) liability "
    "across all of your assets is determined by your Steuerberater. Hand this draft "
    "to your tax adviser for review before relying on it or filing."
)

# Invoked-default notes, mirroring the AT-00x / EXIT-00x style in
# docs/plan/11-exit-tax-deemed-disposal.md. Surfaced so a reviewer sees exactly
# which assumptions shaped the estimate.
_ASSUMPTION_ALT_EXCLUDED = (
    "EXIT: Altbestand (acquired before 2021-03-01 Europe/Vienna) is treated as "
    "tax-free and excluded from the deemed-disposal base; only Neubestand "
    "unrealized gains are taxed."
)
_ASSUMPTION_RATE = (
    "EXIT-002: Neubestand gains estimated at the 27.5% Sondersteuersatz "
    "(§ 27a EStG). Losses net within the deemed disposal; if the total Neubestand "
    "gain is negative the estimated tax is zero (no carryforward, and loss offset "
    "beyond the same income type is out of scope)."
)
_ASSUMPTION_DERIVED_TOKENS = (
    "EXIT-004: Income-type receipts (staking/mining/airdrop) are present in this "
    "book. Inventory uses the matching acquisition lots (income recognition "
    "lines are not counted again); confirm the product mechanics with your "
    "adviser."
)
_ASSUMPTION_UNTAGGED_DISPOSAL = (
    "EXIT-010: A disposal marked non-reportable carries no Alt/Neu tag; its regime "
    "was inferred from the disposal date and may misattribute Altbestand sold after "
    "2021-03-01. Review those rows."
)
_ASSUMPTION_DEFERRAL_EU = (
    "EXIT-002: EU/EEA destination — tax is assessed but, on application, not "
    "collected until you actually sell (Nichtfestsetzung, § 27 Abs 6 Z 1 lit a). "
    "Not the 7-year business-asset Ratenzahlung."
)
_ASSUMPTION_DEFERRAL_THIRD = (
    "EXIT-002: Third-country (non-EU/EEA) destination — tax is due immediately "
    "on departure."
)
_ASSUMPTION_FMV = (
    "EXIT-005: Fair market value uses the best available cached rate at or before "
    "the departure date; it does not imply intraday coverage."
)
_ASSUMPTION_FUTURE_TIGHTENING = (
    "EXIT-003: A planned 1 July 2026 tightening (annual proof when deferred gains "
    "exceed EUR 100,000) is not modelled — verify against enacted law."
)
_ASSUMPTION_GENERIC = (
    "EXIT: Non-Austrian profile — no Altbestand grandfathering or special rate is "
    "applied; this shows the total unrealized gain only. Have a local adviser set "
    "the applicable exit-tax treatment."
)


def _eur(value: Optional[Decimal]) -> Optional[float]:
    if value is None:
        return None
    return float(dec(value).quantize(_CENT, rounding=ROUND_HALF_UP))


def _sats(qty_btc: Optional[Decimal]) -> int:
    if qty_btc is None:
        return 0
    return int((dec(qty_btc) * _SATS_PER_BTC).quantize(Decimal("1"), rounding=ROUND_HALF_UP))


def _normalize_destination(destination: Optional[str]) -> str:
    value = str(destination or DESTINATION_EU_EEA).strip().lower().replace("-", "_")
    if value in ("eu", "eea", "eu_eea", "euea"):
        return DESTINATION_EU_EEA
    if value in ("third_country", "third", "non_eu", "noneu"):
        return DESTINATION_THIRD_COUNTRY
    raise AppError(
        f"Unsupported exit-tax destination '{destination}'",
        code="validation",
        hint="Choose one of: eu_eea, third_country",
    )


def _resolve_departure(departure_date: Optional[str]) -> tuple[str, str]:
    """Return (departure_date 'YYYY-MM-DD', cutoff RFC3339 end-of-day)."""

    if not departure_date:
        day = datetime.now(timezone.utc).date().isoformat()
    else:
        # parse_timestamp validates and normalizes; keep only the date part.
        day = parse_timestamp(str(departure_date))[:10]
    cutoff = f"{day}T23:59:59Z"
    return day, cutoff


def _description_at_regime(description: Any) -> Optional[str]:
    marker = parse_marker(description, MARKER_REGIME)
    return marker if marker in ("alt", "neu") else None


def _entry_is_alt(entry: Mapping[str, Any]) -> bool:
    """Regime of a journal entry for inventory accounting.

    Disposals/fees carry an `at_category` (e.g. ``neu_gain``, ``alt_taxfree``);
    its prefix is authoritative. The RP2 adapter serializes explicit Austrian
    overrides and transfer fee regimes into the description as ``at_regime=...``;
    honor that marker before falling back to the acquisition-date cutoff.
    """

    marker = _description_at_regime(entry.get("description"))
    if marker is not None:
        return marker == "alt"
    category = entry.get("at_category")
    if category:
        return str(category).startswith("alt")
    occurred_at = entry.get("occurred_at")
    if occurred_at:
        return infer_regime_from_timestamp(str(occurred_at)) == "alt"
    return False


def _fmv_at(
    conn: sqlite3.Connection,
    asset: str,
    fiat_currency: str,
    target_iso: str,
    fallback_rates: Mapping[str, Any],
) -> tuple[Optional[Decimal], dict[str, Any]]:
    """Fair market value of one unit of ``asset`` at/just before ``target_iso``.

    Uses the canonical cache lookup ``get_cached_rate_at_or_before`` so the FMV is
    always bounded by the departure date (a historical departure never picks up a
    later/future rate) and a reviewed manual override wins over a provider row at
    the same timestamp. If no rate at-or-before the date exists, fall back to the
    profile's transaction price, else mark the asset unpriced.
    """

    pair = core_rates.transaction_rate_pair(asset, fiat_currency)
    if pair is not None:
        try:
            cached = core_rates.get_cached_rate_at_or_before(conn, pair, target_iso)
        except (AppError, sqlite3.OperationalError):
            cached = None
        if cached is not None:
            rate = pricing.decimal_from_exact(cached.get("rate_exact"), cached.get("rate"))
            if rate is not None and rate > 0:
                return rate, {
                    "asset": asset,
                    "rate": float(rate),
                    "pair": pair,
                    "asOf": cached.get("timestamp"),
                    "source": "cache",
                }
    fallback = fallback_rates.get(asset) if fallback_rates else None
    if fallback is not None:
        fallback_dec = dec(fallback)
        if fallback_dec > 0:
            return fallback_dec, {
                "asset": asset,
                "rate": float(fallback_dec),
                "pair": pair or f"{asset}-{str(fiat_currency).upper()}",
                "asOf": target_iso,
                "source": "transaction",
            }
    return None, {
        "asset": asset,
        "rate": None,
        "pair": pair or f"{asset}-{str(fiat_currency).upper()}",
        "asOf": target_iso,
        "source": "missing",
    }


class _RegimeBucket:
    __slots__ = ("qty", "basis")

    def __init__(self) -> None:
        self.qty = Decimal("0")
        self.basis = Decimal("0")

    def add_inflow(self, qty: Decimal, cost: Decimal) -> None:
        self.qty += qty
        self.basis += cost

    def remove_outflow(self, qty: Decimal, basis: Decimal) -> None:
        self.qty -= qty
        self.basis -= basis

    def clamped(self) -> tuple[Decimal, Decimal]:
        return (max(self.qty, Decimal("0")), max(self.basis, Decimal("0")))


def compute_deemed_disposal(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    state: Mapping[str, Any],
    *,
    departure_date: Optional[str] = None,
    destination: Optional[str] = None,
) -> dict[str, Any]:
    """Build the exit-tax preview payload from a processed ledger ``state``.

    ``state`` is the dict returned by the engine ledger hook (entries, holdings,
    quarantines, latest_rates). The returned shape is the frozen
    ``ui.reports.exit_tax_preview`` contract (camelCase), reused verbatim by the
    CLI JSON output so there is a single source of truth.

    Engine-entry contract (from ``_append_rp2_journal_entries`` in
    ``kassiber/core/engines/rp2.py``) this walk relies on — if RP2 changes any of
    these, the drift guard test in ``tests/test_exit_tax.py`` is the safety net:

    - ``entry_type``: one of ``RECOGNIZED_ENTRY_TYPES``. ``transfer_in`` /
      ``transfer_out`` are skipped (net-zero across owned wallets);
      ``income`` is skipped for inventory (RP2 already books the matching
      acquisition lot) but still flags EXIT-004; everything else is bucketed
      by the sign of ``quantity``.
    - ``quantity`` (Decimal, signed): ``>= 0`` is an inflow (acquisition),
      ``< 0`` is an outflow (disposal/fee). The sign is authoritative.
    - inflow cost: acquisitions carry it in ``fiat_value`` (``cost_basis`` is
      ``None``).
    - outflow basis: the engine-computed consumed ``cost_basis``.
    - ``at_category`` (AT only): ``None``, or a string whose prefix (``alt`` /
      ``neu`` / ``income``) is the regime; acquisitions/transfers carry ``None``.
    """

    jurisdiction = str(profile.get("tax_country") or "generic").strip().lower()
    is_at = jurisdiction == "at"
    fiat_currency = str(profile.get("fiat_currency") or "EUR").upper()
    day, cutoff = _resolve_departure(departure_date)
    dest = _normalize_destination(destination)
    method = build_tax_policy(profile).default_accounting_method

    entries: Sequence[Mapping[str, Any]] = state.get("entries") or ()
    fallback_rates = state.get("latest_rates") or {}

    # Per-asset Alt/Neu inventory + cost basis, rebuilt from engine entries.
    assets: dict[str, dict[str, _RegimeBucket]] = {}

    def _bucket(asset: str, alt: bool) -> _RegimeBucket:
        regimes = assets.setdefault(asset, {"alt": _RegimeBucket(), "neu": _RegimeBucket()})
        return regimes["alt" if alt else "neu"]

    has_income = False
    has_untagged_disposal = False

    for entry in entries:
        occurred_at = entry.get("occurred_at")
        if occurred_at and str(occurred_at) > cutoff:
            continue  # not yet held / disposed of as of the departure date
        entry_type = str(entry.get("entry_type") or "")
        if entry_type in ("transfer_in", "transfer_out"):
            continue  # net-zero across owned wallets; never touches the pool
        if entry_type == "income":
            # Earn receipts already have an acquisition lot with the same
            # quantity; counting income again would double holdings (see
            # _HOLDINGS_QUANTITY_SKIP_ENTRY_TYPES). Still flag EXIT-004.
            has_income = True
            continue
        asset = str(entry.get("asset") or "")
        if not asset:
            continue
        qty = dec(entry.get("quantity") or 0)
        # Generic profiles have no regime; pool everything as Neubestand-equiv.
        alt = _entry_is_alt(entry) if is_at else False
        bucket = _bucket(asset, alt)
        if qty >= 0:
            # Inflow: acquisition cost is in fiat_value (cost_basis is None).
            cost = entry.get("cost_basis")
            cost = dec(cost) if cost is not None else dec(entry.get("fiat_value") or 0)
            bucket.add_inflow(qty, cost)
        else:
            # Outflow (disposal/fee): remove the engine-computed consumed basis.
            # A disposal the user marked non-reportable carries no at_category, so
            # its regime fell back to the disposal date and may misattribute
            # Altbestand sold after the cutoff — flag it (EXIT-010).
            if (
                is_at
                and not entry.get("at_category")
                and _description_at_regime(entry.get("description")) is None
            ):
                has_untagged_disposal = True
            basis = dec(entry.get("cost_basis") or 0)
            bucket.remove_outflow(-qty, basis)

    fmv_source: list[dict[str, Any]] = []
    fmv_by_asset: dict[str, Optional[Decimal]] = {}
    for asset in sorted(assets):
        rate, source = _fmv_at(conn, asset, fiat_currency, cutoff, fallback_rates)
        fmv_by_asset[asset] = rate
        fmv_source.append(source)

    lots: list[dict[str, Any]] = []
    neu_qty_total = Decimal("0")
    neu_market_total: Optional[Decimal] = Decimal("0")
    neu_basis_total = Decimal("0")
    neu_gain_total: Optional[Decimal] = Decimal("0")
    alt_qty_total = Decimal("0")
    alt_market_total: Optional[Decimal] = Decimal("0")
    has_unpriced_neu = False
    has_unpriced_alt = False

    for asset in sorted(assets):
        fmv = fmv_by_asset.get(asset)
        for regime in ("neu", "alt"):
            qty, basis = assets[asset][regime].clamped()
            if qty <= 0:
                continue
            market = qty * fmv if fmv is not None else None
            if regime == "neu":
                gain = (market - basis) if market is not None else None
                taxable = True
                category = "neu_gain" if (gain is None or gain >= 0) else "neu_loss"
                neu_qty_total += qty
                neu_basis_total += basis
                if market is None:
                    has_unpriced_neu = True
                elif neu_market_total is not None and neu_gain_total is not None:
                    neu_market_total += market
                    neu_gain_total += gain
            else:
                gain = None
                taxable = False
                category = "alt_taxfree"
                alt_qty_total += qty
                if market is None:
                    has_unpriced_alt = True
                elif alt_market_total is not None:
                    alt_market_total += market
            lots.append(
                {
                    "asset": asset,
                    "regime": regime,
                    "quantitySats": _sats(qty),
                    "marketValue": _eur(market),
                    "costBasis": _eur(basis) if regime == "neu" else None,
                    "gain": _eur(gain),
                    "taxable": taxable,
                    "category": category if is_at else "unrealized",
                    "kennzahl": kennzahl_for_disposal_category(category) if is_at else None,
                }
            )

    if has_unpriced_neu:
        neu_market_total = None
        neu_gain_total = None
    if has_unpriced_alt:
        alt_market_total = None

    taxable_gain = (
        max(Decimal("0"), neu_gain_total) if neu_gain_total is not None else None
    )
    rate = AT_SONDERSTEUERSATZ if is_at else None
    estimated_tax = (
        (taxable_gain * rate)
        if rate is not None and taxable_gain is not None
        else None
    )
    collection_timing = "deferred" if dest == DESTINATION_EU_EEA else "immediate"

    wallet_holdings = []
    for key, totals in (state.get("wallet_holdings") or {}).items():
        # key = (wallet_id, wallet_label, account_code, asset)
        wallet_label = key[1]
        asset = key[3]
        qty = dec(totals.get("quantity") or 0)
        if qty <= 0:
            continue
        fmv = fmv_by_asset.get(asset)
        wallet_holdings.append(
            {
                "wallet": wallet_label,
                "asset": asset,
                "quantitySats": _sats(qty),
                "marketValue": _eur(qty * fmv) if fmv is not None else None,
            }
        )
    wallet_holdings.sort(key=lambda row: (row["asset"], row["wallet"]))

    assumptions: list[str] = []
    if is_at:
        assumptions.append(_ASSUMPTION_ALT_EXCLUDED)
        assumptions.append(_ASSUMPTION_RATE)
        assumptions.append(
            _ASSUMPTION_DEFERRAL_EU if dest == DESTINATION_EU_EEA else _ASSUMPTION_DEFERRAL_THIRD
        )
        assumptions.append(_ASSUMPTION_FMV)
        if dest == DESTINATION_EU_EEA:
            assumptions.append(_ASSUMPTION_FUTURE_TIGHTENING)
        if has_income:
            assumptions.append(_ASSUMPTION_DERIVED_TOKENS)
        if has_untagged_disposal:
            assumptions.append(_ASSUMPTION_UNTAGGED_DISPOSAL)
    else:
        assumptions.append(_ASSUMPTION_GENERIC)
        assumptions.append(_ASSUMPTION_FMV)
    if any(source["source"] == "missing" for source in fmv_source):
        assumptions.append(
            "EXIT-005: No cached rate found for one or more assets — run "
            "`kassiber rates sync`; market value, gain, and estimated tax stay "
            "incomplete (null) until those holdings are priced."
        )

    quarantines = list(state.get("quarantines") or ())

    return {
        "workspace": profile.get("workspace_label") or profile.get("workspace_id"),
        "profile": profile.get("label") or profile.get("id"),
        "jurisdictionCode": "AT" if is_at else "generic",
        "fiatCurrency": fiat_currency,
        "departureDate": day,
        "destination": dest,
        "method": method,
        "fmvSource": fmv_source,
        "totals": {
            "neuQuantitySats": _sats(neu_qty_total),
            "neuMarketValue": _eur(neu_market_total),
            "neuCostBasis": _eur(neu_basis_total),
            "neuGain": _eur(neu_gain_total),
            "altQuantitySats": _sats(alt_qty_total),
            "altMarketValue": _eur(alt_market_total),
            "taxableGain": _eur(taxable_gain),
            "estimatedTaxRate": float(rate) if rate is not None else None,
            "estimatedTax": _eur(estimated_tax),
            "collectionTiming": collection_timing,
        },
        "lots": lots,
        "walletHoldings": wallet_holdings,
        "assumptions": assumptions,
        "reviewGate": EXIT_TAX_REVIEW_GATE,
        "status": {
            "needsJournals": not bool(entries),
            "quarantines": len(quarantines),
        },
    }


def report_exit_tax(
    conn: sqlite3.Connection,
    workspace_ref: Any,
    profile_ref: Any,
    hooks: Any,
    *,
    departure_date: Optional[str] = None,
    destination: Optional[str] = None,
    report_context: core_report_context.ReportContext | None = None,
) -> dict[str, Any]:
    """Build the exit-tax payload from a mandatory fresh report context."""

    if report_context is None:
        report_context = core_report_context.require_report_context(
            conn,
            workspace_ref,
            profile_ref,
            hooks.resolve_scope,
        )
    workspace = report_context.workspace
    profile = report_context.profile
    state = custody_journal.load_stored_ledger_state(conn, profile)
    profile = dict(profile)
    workspace_label = workspace["label"] if "label" in workspace.keys() else None
    profile.setdefault("workspace_label", workspace_label)
    return compute_deemed_disposal(
        conn,
        profile,
        state,
        departure_date=departure_date,
        destination=destination,
    )


def _fmt_eur(value: Optional[float], currency: str) -> str:
    if value is None:
        return "—"
    return f"{value:,.2f} {currency}"


def _fmt_btc_from_sats(sats: int) -> str:
    return f"{(Decimal(int(sats)) / _SATS_PER_BTC):.8f}"


def build_exit_tax_report_lines(
    conn: sqlite3.Connection,
    workspace_ref: Any,
    profile_ref: Any,
    hooks: Any,
    *,
    departure_date: Optional[str] = None,
    destination: Optional[str] = None,
    report_context: core_report_context.ReportContext | None = None,
) -> list[str]:
    """Human-readable plain-text rendering of the exit-tax estimate."""

    report = report_exit_tax(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        departure_date=departure_date,
        destination=destination,
        report_context=report_context,
    )
    return format_exit_tax_lines(report)


def format_exit_tax_lines(report: Mapping[str, Any]) -> list[str]:
    """Render an exit-tax report payload to plain-text lines (PDF/CLI reuse)."""

    ccy = report["fiatCurrency"]
    totals = report["totals"]
    lines: list[str] = []
    lines.append(f"Exit-tax estimate — {report['profile']} ({report['jurisdictionCode']})")
    lines.append(f"Departure date: {report['departureDate']}    Destination: {report['destination']}")
    lines.append(f"Accounting method: {report['method']}")
    lines.append("")
    lines.append("Deemed disposal at fair market value:")
    lines.append(
        f"  Neubestand (taxable):  {_fmt_btc_from_sats(totals['neuQuantitySats'])} "
        f"@ market {_fmt_eur(totals['neuMarketValue'], ccy)}, "
        f"basis {_fmt_eur(totals['neuCostBasis'], ccy)}, "
        f"gain {_fmt_eur(totals['neuGain'], ccy)}"
    )
    lines.append(
        f"  Altbestand (excluded): {_fmt_btc_from_sats(totals['altQuantitySats'])} "
        f"@ market {_fmt_eur(totals['altMarketValue'], ccy)} — tax-free, not in the base"
    )
    lines.append("")
    lines.append(f"  Taxable gain:        {_fmt_eur(totals['taxableGain'], ccy)}")
    if totals["estimatedTaxRate"] is not None:
        lines.append(f"  Rate:                {totals['estimatedTaxRate'] * 100:.1f}%")
    lines.append(f"  Estimated exit tax:  {_fmt_eur(totals['estimatedTax'], ccy)}")
    if totals["collectionTiming"] == "deferred":
        lines.append("  Collection:          assessed but deferred until you sell (EU/EEA Nichtfestsetzung)")
    else:
        lines.append("  Collection:          due immediately on departure (third country)")
    lines.append("")
    if report["status"]["quarantines"]:
        lines.append(
            f"⚠ {report['status']['quarantines']} quarantined transaction(s) — "
            "estimate is incomplete until resolved."
        )
        lines.append("")
    lines.append("Assumptions:")
    for note in report["assumptions"]:
        lines.append(f"  - {note}")
    lines.append("")
    lines.append(report["reviewGate"])
    return lines


__all__ = [
    "AT_SONDERSTEUERSATZ",
    "DESTINATION_EU_EEA",
    "DESTINATION_THIRD_COUNTRY",
    "EXIT_TAX_REVIEW_GATE",
    "RECOGNIZED_ENTRY_TYPES",
    "build_exit_tax_report_lines",
    "compute_deemed_disposal",
    "format_exit_tax_lines",
    "report_exit_tax",
]
