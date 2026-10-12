"""Year handoff workbook for EA / Jahresabschluss bookkeeping.

The bookkeeping itself (spheres, accounts, UGB valuation) happens outside
Kassiber. This export hands over one book's year: every observed transaction
with Kassiber's classification and pair links, the journal rows that carry its
basis and gains, opening and closing balances, a per-asset roll-forward, a
reconciliation of the journal against observed wallet movements, the wallets
in scope, and the transactions the user excluded.

Nothing here assumes Bitcoin. Assets are grouped by their stored identity and
presented through ``asset_presentation.describe_asset``; quantities appear in
each asset's own unit with a lossless text copy, and satoshi columns are only
filled for bitcoin-denominated assets. Chain and network come from the wallet.

It reads the same processed journals as every other report and fails closed
the same way. It also refuses to run while any quarantine dated in or before
the year is unresolved, because such a row would be missing from the balances.
"""

from __future__ import annotations

import json
import sqlite3
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any, Mapping
from zoneinfo import ZoneInfo

from ..errors import AppError
from ..msat import dec
from . import rates as core_rates
from .asset_presentation import AssetPresentation, describe_asset
from .austrian import AT_TAX_TZ
from .custody_evidence import normalize_boundary_amounts
from .journal_markers import strip_markers
from .transaction_classification import classify, pair_links, require_no_quarantines
from .reports import (
    ReportHooks,
    _generic_report_transaction_rows,
    _generic_report_xlsx_formats,
    _estimate_wrapped_lines,
    _normalize_tax_year,
    _register_saved_export,
    _report_query_rows,
    _require_report_context,
    _row_get,
    _write_attachments_cell,
    _xlsx_write_value,
)

REPORT_KIND = "year-handoff.xlsx"
_UTC = ZoneInfo("UTC")

# Mirrors reports._holdings_quantity_delta / _holdings_basis_delta: a
# self-transfer's fee is already inside transfer_out, and income coins are
# already counted by their paired acquisition lot.
_QUANTITY_SKIP = frozenset({"income", "transfer_fee"})
_BASIS_SKIP = frozenset({"income"})


@dataclass(frozen=True)
class Column:
    key: str
    label: str
    format: str = "text"
    # Live formula over the same row; ``{other_key}`` names another column's
    # cell and ``{decimals}`` the row asset's stored decimals.
    formula: str | None = None


def _columns(*specs) -> tuple[Column, ...]:
    return tuple(Column(*spec) for spec in specs)


WALLET_COLUMNS = _columns(
    ("wallet", "Wallet"),
    ("wallet_kind", "Wallet Kind"),
    ("chain", "Chain"),
    ("network", "Network"),
    ("status", "Status"),
    ("assets", "Assets"),
    ("transactions_in_year", "Transactions In Year", "int"),
    ("excluded_in_year", "Excluded In Year", "int"),
    ("first_transaction_at", "First Transaction (UTC)"),
    ("last_transaction_at", "Last Transaction (UTC)"),
    ("last_synced_at", "Last Sync (UTC)"),
)

TRANSACTION_COLUMNS = _columns(
    ("occurred_at", "Occurred At (UTC)"),
    ("local_date", "Local Date"),
    ("wallet", "Wallet"),
    ("wallet_kind", "Wallet Kind"),
    ("chain", "Chain"),
    ("network", "Network"),
    ("transaction_ref", "Transaction Ref"),
    ("transaction_id", "Transaction ID"),
    ("direction", "Direction"),
    ("classification", "Kassiber Classification"),
    ("source_kind", "Source Kind"),
    ("pair_ref", "Pair Ref"),
    ("other_wallet", "Other Wallet"),
    ("other_leg_transaction_id", "Other Leg Transaction ID"),
    ("asset", "Asset"),
    ("asset_id", "Asset ID"),
    ("amount", "Amount", "quantity"),
    ("fee", "Fee", "quantity"),
    ("net_change", "Net Change", "quantity"),
    ("amount_sats", "Amount (sats)", "sats"),
    ("fee_sats", "Fee (sats)", "sats"),
    ("net_change_sats", "Net Change (sats)", "sats"),
    ("amount_exact", "Amount (exact)"),
    ("fee_exact", "Fee (exact)"),
    ("net_change_exact", "Net Change (exact)"),
    ("fiat_price", "Price ({fiat} per unit)", "money"),
    ("fiat_value", "Recorded Value ({fiat})", "money"),
    ("fmv", "Market Value ({fiat})", "money"),
    ("fiat_fee", "Network Fee ({fiat})", "money"),
    ("income_value", "Income ({fiat})", "money"),
    ("disposal_proceeds", "Proceeds ({fiat})", "money"),
    ("disposal_cost_basis", "Cost Basis Disposed ({fiat})", "money"),
    ("disposal_gain_loss", "Gain/Loss ({fiat})", "money"),
    ("fee_cost_basis", "Fee Cost Basis ({fiat})", "money"),
    ("journal_entry_types", "Journal Entry Types"),
    ("tax_category", "Tax Category"),
    ("description", "Description"),
    ("note", "Note"),
    ("counterparty", "Counterparty"),
    ("tags", "Tags"),
    ("references", "References"),
    ("attachments", "Attachments"),
)

JOURNAL_COLUMNS = _columns(
    ("occurred_at", "Occurred At (UTC)"),
    ("local_date", "Local Date"),
    ("wallet", "Wallet"),
    ("transaction_ref", "Transaction Ref"),
    ("transaction_id", "Transaction ID"),
    ("entry_type", "Entry Type"),
    ("asset", "Asset"),
    ("asset_id", "Asset ID"),
    ("quantity", "Quantity", "quantity"),
    ("quantity_exact", "Quantity (exact)"),
    ("fiat_value", "Value ({fiat})", "money"),
    ("cost_basis", "Cost Basis ({fiat})", "money"),
    ("proceeds", "Proceeds ({fiat})", "money"),
    ("gain_loss", "Gain/Loss ({fiat})", "money"),
    ("in_capital_gains_report", "In Capital Gains Report", "int"),
    ("tax_category", "Tax Category"),
    ("tax_form_field", "Tax Form Field", "int"),
    ("capital_gains_type", "Holding Period"),
    ("pricing_source", "Pricing Source"),
    ("pricing_quality", "Pricing Quality"),
    ("description", "Description"),
    ("journal_entry_id", "Journal Entry ID"),
)

BALANCE_COLUMNS = _columns(
    ("wallet", "Wallet"),
    ("chain", "Chain"),
    ("network", "Network"),
    ("asset", "Asset"),
    ("asset_id", "Asset ID"),
    ("opening_quantity", "Opening Quantity", "quantity"),
    ("opening_quantity_exact", "Opening Quantity (exact)"),
    ("opening_cost_basis", "Opening Cost Basis ({fiat})", "money"),
    ("closing_quantity", "Closing Quantity", "quantity"),
    ("closing_quantity_exact", "Closing Quantity (exact)"),
    ("closing_cost_basis", "Closing Cost Basis ({fiat})", "money"),
    ("year_end_rate", "Year-End Rate ({fiat} per unit)", "money"),
    ("year_end_market_value", "Year-End Market Value ({fiat})", "money"),
    ("rate_source", "Rate Source"),
    ("rate_timestamp", "Rate Timestamp (UTC)"),
)

ROLL_FORWARD_COLUMNS = _columns(
    ("asset", "Asset"),
    ("asset_id", "Asset ID"),
    ("opening_quantity", "Opening Quantity", "quantity"),
    ("acquired_quantity", "Acquired", "quantity"),
    ("disposed_quantity", "Disposed", "quantity"),
    ("fee_quantity", "Fees", "quantity"),
    ("transfer_net", "Own-Wallet Transfers Net", "quantity"),
    ("closing_quantity", "Closing Quantity", "quantity"),
    (
        "quantity_check",
        "Quantity Check",
        "quantity",
        "=ROUND({opening_quantity}+{acquired_quantity}-{disposed_quantity}"
        "-{fee_quantity}+{transfer_net}-{closing_quantity},{decimals})",
    ),
    ("opening_quantity_exact", "Opening Quantity (exact)"),
    ("closing_quantity_exact", "Closing Quantity (exact)"),
    ("opening_cost_basis", "Opening Cost Basis ({fiat})", "money"),
    ("acquired_value", "Acquired Value ({fiat})", "money"),
    ("income_value", "Of Which Income ({fiat})", "money"),
    ("disposed_cost_basis", "Cost Basis Disposed ({fiat})", "money"),
    ("disposal_proceeds", "Proceeds ({fiat})", "money"),
    ("realized_gain_loss", "Realized Gain/Loss ({fiat})", "money"),
    ("fee_cost_basis", "Fee Cost Basis ({fiat})", "money"),
    ("transfer_basis_net", "Own-Wallet Transfers Basis Net ({fiat})", "money"),
    ("closing_cost_basis", "Closing Cost Basis ({fiat})", "money"),
    (
        "basis_check",
        "Basis Check ({fiat})",
        "money",
        "=ROUND({opening_cost_basis}+{acquired_value}-{disposed_cost_basis}"
        "-{fee_cost_basis}+{transfer_basis_net}-{closing_cost_basis},2)",
    ),
    ("year_end_rate", "Year-End Rate ({fiat} per unit)", "money"),
    ("year_end_market_value", "Year-End Market Value ({fiat})", "money"),
    ("unrealized_gain_loss", "Unrealized Gain/Loss ({fiat})", "money"),
)

RECONCILIATION_COLUMNS = _columns(
    ("wallet", "Wallet"),
    ("asset", "Asset"),
    ("asset_id", "Asset ID"),
    ("opening_journal", "Opening (journal)", "quantity"),
    ("opening_observed", "Opening (observed)", "quantity"),
    ("inflow", "Received", "quantity"),
    ("outflow", "Sent", "quantity"),
    ("fee", "Fees", "quantity"),
    (
        "closing_observed",
        "Closing (observed)",
        "quantity",
        "=ROUND({opening_observed}+{inflow}-{outflow}-{fee},{decimals})",
    ),
    ("closing_journal", "Closing (journal)", "quantity"),
    (
        "opening_difference",
        "Opening Difference",
        "quantity",
        "=ROUND({opening_journal}-{opening_observed},{decimals})",
    ),
    (
        "closing_difference",
        "Closing Difference",
        "quantity",
        "=ROUND({closing_journal}-{closing_observed},{decimals})",
    ),
    (
        "status",
        "Status",
        "text",
        '=IF(AND({opening_difference}=0,{closing_difference}=0),"OK","DIFF")',
    ),
    ("opening_difference_exact", "Opening Difference (exact)"),
    ("closing_difference_exact", "Closing Difference (exact)"),
    ("timing_notes", "Timing Notes"),
)

EXCLUDED_COLUMNS = _columns(
    ("occurred_at", "Occurred At (UTC)"),
    ("local_date", "Local Date"),
    ("wallet", "Wallet"),
    ("transaction_ref", "Transaction Ref"),
    ("transaction_id", "Transaction ID"),
    ("direction", "Direction"),
    ("source_kind", "Source Kind"),
    ("asset", "Asset"),
    ("asset_id", "Asset ID"),
    ("amount", "Amount", "quantity"),
    ("fee", "Fee", "quantity"),
    ("amount_exact", "Amount (exact)"),
    ("fee_exact", "Fee (exact)"),
    ("excluded_at", "Excluded At (UTC)"),
    ("exclusion_reason", "Exclusion Reason"),
    ("description", "Description"),
    ("note", "Note"),
    ("tags", "Tags"),
)

OVERVIEW_COLUMNS = _columns(("group", "Group"), ("field", "Field"), ("value", "Value"))


def _parse_timestamp(value: str) -> datetime:
    parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=_UTC)
    return parsed


class _Period:
    """One accounting year in the profile's calendar.

    Austrian books follow the Europe/Vienna calendar like the E 1kv export;
    every other book uses UTC calendar years.
    """

    def __init__(self, year: int, tz: ZoneInfo):
        self.year = year
        self.tz = tz
        self.start = datetime(year, 1, 1, tzinfo=tz).astimezone(timezone.utc)
        self.end = datetime(year + 1, 1, 1, tzinfo=tz).astimezone(timezone.utc)

    def position(self, occurred_at: str) -> int:
        """-1 before the year, 0 inside it, 1 after it."""
        local_year = _parse_timestamp(occurred_at).astimezone(self.tz).year
        if local_year < self.year:
            return -1
        if local_year > self.year:
            return 1
        return 0

    def local_date(self, occurred_at: str) -> str:
        return _parse_timestamp(occurred_at).astimezone(self.tz).date().isoformat()

    @property
    def calendar(self) -> str:
        return str(self.tz.key)

    @staticmethod
    def iso(value: datetime) -> str:
        return value.strftime("%Y-%m-%dT%H:%M:%SZ")


def _period_for_profile(profile: Mapping[str, Any], year: int) -> _Period:
    tax_country = str(_row_get(profile, "tax_country") or "").lower()
    return _Period(year, AT_TAX_TZ if tax_country == "at" else _UTC)


def _decimal(exact: Any, legacy: Any) -> Decimal | None:
    for value in (exact, legacy):
        if value not in (None, ""):
            return dec(value)
    return None


def _money(value: Decimal | None) -> float | None:
    return float(value) if value is not None else None


def _asset_fields(asset: AssetPresentation) -> dict[str, Any]:
    return {"asset": asset.display_code, "asset_id": asset.asset_id, "_decimals": asset.stored_decimals}


def _quantity_fields(asset: AssetPresentation, prefix: str, stored: int) -> dict[str, Any]:
    return {prefix: asset.quantity(stored), f"{prefix}_exact": asset.exact(stored)}


def _journal_rows(conn, profile):
    return conn.execute(
        """
        SELECT
            je.id,
            je.occurred_at,
            je.transaction_id,
            COALESCE(NULLIF(t.external_id, ''), '') AS external_id,
            w.label AS wallet,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            je.fiat_value_exact,
            je.cost_basis,
            je.cost_basis_exact,
            je.proceeds,
            je.proceeds_exact,
            je.gain_loss,
            je.gain_loss_exact,
            je.at_category,
            je.at_kennzahl,
            je.capital_gains_type,
            COALESCE(je.pricing_source_kind, '') AS pricing_source,
            COALESCE(je.pricing_quality, '') AS pricing_quality,
            COALESCE(je.description, '') AS description,
            CASE
                WHEN je.entry_type IN ('disposal', 'income')
                     AND COALESCE(t.taxability_override, 1) != 0
                     AND COALESCE(je.at_category, '') != 'neu_swap'
                THEN 1 ELSE 0
            END AS in_capital_gains_report
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        (profile["id"],),
    ).fetchall()


def _journal_amounts(row) -> dict[str, Decimal | None]:
    return {
        "fiat_value": _decimal(row["fiat_value_exact"], row["fiat_value"]) or Decimal("0"),
        "cost_basis": _decimal(row["cost_basis_exact"], row["cost_basis"]),
        "proceeds": _decimal(row["proceeds_exact"], row["proceeds"]),
        "gain_loss": _decimal(row["gain_loss_exact"], row["gain_loss"]),
    }


def _empty_holding():
    return {"stored": 0, "cost_basis": Decimal("0")}


def _apply_holding(holding, row, amounts) -> None:
    stored = int(row["quantity"])
    entry_type = row["entry_type"]
    if entry_type not in _QUANTITY_SKIP:
        holding["stored"] += stored
    if entry_type in _BASIS_SKIP:
        return
    if stored >= 0:
        holding["cost_basis"] += amounts["fiat_value"]
    else:
        holding["cost_basis"] -= amounts["cost_basis"] or Decimal("0")


def _allocate_wallet_basis(holdings):
    """Spread each asset's residual basis across wallets by quantity.

    Per-wallet journal sums strand moved basis in the source wallet (a
    transfer_out carries no basis), so wallets get the asset's pooled average
    exactly like ``reports.portfolio-summary``. The per-asset total is exact.
    """
    pool_quantity: dict[str, int] = defaultdict(int)
    pool_basis: dict[str, Decimal] = defaultdict(Decimal)
    for (_wallet, asset), holding in holdings.items():
        pool_quantity[asset] += holding["stored"]
        pool_basis[asset] += holding["cost_basis"]
    allocated = {}
    for key, holding in holdings.items():
        asset = key[1]
        stored = holding["stored"]
        if stored > 0 and pool_quantity[asset] > 0:
            allocated[key] = pool_basis[asset] * stored / pool_quantity[asset]
        else:
            allocated[key] = Decimal("0")
    return allocated, pool_basis


def _year_end_rates(
    conn, profile, period: _Period, spellings_by_asset: Mapping[str, set[str]]
) -> dict[str, dict[str, Any]]:
    """Market rate per asset at the close of the year, with its provenance.

    A missing quote stays missing; it is never valued at zero. The fallback is
    the last recorded transaction price on or before year end. Assets without
    a rate pair (anything the rates cache does not cover) use the fallback.
    """
    fiat_currency = profile["fiat_currency"]
    end_iso = _Period.iso(period.end)
    rates: dict[str, dict[str, Any]] = {}
    for asset, spellings in sorted(spellings_by_asset.items()):
        spellings = sorted(spellings)
        pair = next(
            (
                found
                for found in (
                    core_rates.transaction_rate_pair(spelling, fiat_currency) for spelling in spellings
                )
                if found is not None
            ),
            None,
        )
        cached = None
        if pair is not None:
            try:
                cached = core_rates.get_cached_rate_at_or_before(conn, pair, end_iso)
            except sqlite3.OperationalError:
                cached = None
            except AppError as exc:
                if exc.code not in {"not_found", "validation"}:
                    raise
        if cached is not None:
            rate = _decimal(cached.get("rate_exact"), cached.get("rate"))
            if rate is not None:
                rates[asset] = {
                    "rate": rate,
                    "source": cached.get("source") or "rates cache",
                    "timestamp": cached.get("timestamp") or "",
                }
                continue
        row = conn.execute(
            f"""
            SELECT occurred_at, amount, fiat_rate, fiat_rate_exact,
                   fiat_value, fiat_value_exact
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
              AND asset IN ({", ".join("?" for _ in spellings)})
              AND occurred_at < ?
              AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL)
            ORDER BY occurred_at DESC, created_at DESC, id DESC
            LIMIT 1
            """,
            (profile["id"], *spellings, end_iso),
        ).fetchone()
        rate = None
        if row is not None:
            rate = _decimal(row["fiat_rate_exact"], row["fiat_rate"])
            value = _decimal(row["fiat_value_exact"], row["fiat_value"])
            if rate is None and value is not None and row["amount"]:
                rate = value / describe_asset(spellings[0]).quantity(row["amount"])
        if rate is not None:
            rates[asset] = {
                "rate": rate,
                "source": "transaction price",
                "timestamp": row["occurred_at"],
            }
    return rates


def _wallet_meta(conn, workspace, profile, hooks: ReportHooks):
    """Display facts per wallet label from the redacted wallet listing.

    Only kind, chain, network and status leave this function; descriptors,
    addresses and backend settings never reach the workbook. The last sync
    time is read as that single field from the wallet config.
    """
    synced_at = {}
    for row in conn.execute(
        "SELECT id, config_json FROM wallets WHERE profile_id = ?", (profile["id"],)
    ).fetchall():
        try:
            config = json.loads(row["config_json"] or "{}")
        except ValueError:
            config = {}
        value = config.get("last_synced_at") if isinstance(config, dict) else None
        synced_at[row["id"]] = value if isinstance(value, str) else ""
    meta = {}
    for wallet in hooks.list_wallets(conn, workspace["id"], profile["id"]):
        meta[wallet["label"]] = {
            "wallet_kind": wallet.get("kind") or "",
            "chain": wallet.get("chain") or "",
            "network": wallet.get("network") or "",
            "status": "deprecated" if wallet.get("deprecated") else "active",
            "last_synced_at": synced_at.get(wallet["id"], ""),
        }
    return meta


def _excluded_rows(conn, profile, period: _Period):
    rows = conn.execute(
        """
        SELECT t.id, t.occurred_at, COALESCE(t.external_id, '') AS external_id,
               w.label AS wallet, t.direction,
               COALESCE(NULLIF(t.kind_override, ''), t.kind, '') AS kind,
               t.asset, t.amount, t.fee,
               COALESCE(t.description, '') AS description,
               COALESCE(t.note, '') AS note,
               (
                 SELECT GROUP_CONCAT(code, ', ') FROM (
                   SELECT tg.code AS code
                   FROM transaction_tags tt JOIN tags tg ON tg.id = tt.tag_id
                   WHERE tt.transaction_id = t.id ORDER BY tg.code
                 )
               ) AS tags,
               exclusion.reason AS exclusion_reason,
               exclusion.changed_at AS excluded_at
        FROM transactions t
        LEFT JOIN transaction_edit_events exclusion ON exclusion.id = (
            SELECT event.id
            FROM transaction_edit_events event
            JOIN transaction_edit_fields field ON field.event_id = event.id
            WHERE event.transaction_id = t.id AND field.field = 'excluded'
            ORDER BY event.changed_at DESC, event.id DESC
            LIMIT 1
        )
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.excluded = 1
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile["id"],),
    ).fetchall()
    result = []
    for row in rows:
        if period.position(row["occurred_at"]) != 0:
            continue
        asset = describe_asset(row["asset"])
        result.append(
            {
                "occurred_at": row["occurred_at"],
                "local_date": period.local_date(row["occurred_at"]),
                "wallet": row["wallet"],
                "transaction_ref": row["id"],
                "transaction_id": row["external_id"],
                "direction": row["direction"],
                "source_kind": row["kind"],
                **_asset_fields(asset),
                **_quantity_fields(asset, "amount", int(row["amount"] or 0)),
                **_quantity_fields(asset, "fee", int(row["fee"] or 0)),
                "excluded_at": row["excluded_at"] or "",
                "exclusion_reason": row["exclusion_reason"] or "",
                "description": row["description"],
                "note": row["note"],
                "tags": row["tags"] or "",
            }
        )
    return result


def build_year_handoff(
    conn,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
    *,
    year,
) -> dict[str, Any]:
    report_context = _require_report_context(conn, workspace_ref, profile_ref, hooks)
    workspace = report_context.workspace
    profile = report_context.profile
    period = _period_for_profile(profile, _normalize_tax_year(year))
    require_no_quarantines(
        conn,
        profile["id"],
        in_scope=lambda row: period.position(row["occurred_at"]) <= 0,
        code="year_handoff_quarantined",
        message=f"The {period.year} handoff is blocked by quarantined transactions.",
        hint=(
            f"Resolve or exclude every quarantined transaction dated in or before {period.year}, "
            "then process journals again. A quarantined row is missing from the journal, so "
            "balances would not add up."
        ),
        details={"year": period.year},
    )

    observed_at = {
        row["id"]: row
        for row in conn.execute(
            """
            SELECT id, occurred_at, COALESCE(external_id, '') AS external_id
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            """,
            (profile["id"],),
        ).fetchall()
    }
    external_ids = {tx_id: row["external_id"] for tx_id, row in observed_at.items()}
    pair_link_map, swap_transaction_ids = pair_links(conn, profile["id"], external_ids)
    wallet_meta = _wallet_meta(conn, workspace, profile, hooks)

    # Group every figure by canonical asset identity, never by the stored
    # spelling: the journal and the transactions table may spell the same
    # asset differently (a Liquid asset id in upper vs lower case).
    asset_by_id: dict[str, AssetPresentation] = {}
    asset_spellings: dict[str, set[str]] = defaultdict(set)

    def _asset(raw) -> AssetPresentation:
        presentation = describe_asset(raw)
        asset_spellings[presentation.asset_id].add(str(raw))
        return asset_by_id.setdefault(presentation.asset_id, presentation)

    # Journal: balances at both ends of the year, roll-forward, and per-row data.
    # Quantities stay integers in stored units until presentation.
    opening: dict[tuple[str, str], dict[str, Any]] = defaultdict(_empty_holding)
    closing: dict[tuple[str, str], dict[str, Any]] = defaultdict(_empty_holding)
    roll: dict[str, dict[str, Any]] = defaultdict(
        lambda: {
            "acquired": 0,
            "disposed": 0,
            "fee": 0,
            "transfer_net": 0,
            "acquired_value": Decimal("0"),
            "income_value": Decimal("0"),
            "disposed_cost_basis": Decimal("0"),
            "disposal_proceeds": Decimal("0"),
            "realized_gain_loss": Decimal("0"),
            "fee_cost_basis": Decimal("0"),
            "transfer_basis_net": Decimal("0"),
        }
    )
    per_transaction: dict[str, dict[str, Any]] = defaultdict(
        lambda: {
            "entry_types": set(),
            "at_categories": set(),
            "income_value": None,
            "disposal_proceeds": None,
            "disposal_cost_basis": None,
            "disposal_gain_loss": None,
            "fee_cost_basis": None,
        }
    )

    def _add(bucket, key, value):
        if value is None:
            return
        bucket[key] = (bucket[key] or Decimal("0")) + value

    # A journal row can be booked on the other side of a year boundary from
    # its observation (a transfer's arrival is booked at send time). Name those
    # rows next to the wallet they move so a DIFF explains itself.
    timing_notes: dict[tuple[str, str], list[str]] = defaultdict(list)

    journal_rows = []
    for row in _journal_rows(conn, profile):
        position = period.position(row["occurred_at"])
        observed = observed_at.get(row["transaction_id"])
        if observed is not None and row["entry_type"] not in _QUANTITY_SKIP:
            observed_position = period.position(observed["occurred_at"])
            if (position < 0) != (observed_position < 0) or (position > 0) != (
                observed_position > 0
            ):
                timing_notes[(row["wallet"], _asset(row["asset"]).asset_id)].append(
                    f"{observed['external_id'] or row['transaction_id']}: booked "
                    f"{period.local_date(row['occurred_at'])}, observed "
                    f"{period.local_date(observed['occurred_at'])}"
                )
        # Classification describes the transaction, so it reads every journal
        # row; the figures below stay with the year they are booked in.
        tx = per_transaction[row["transaction_id"]]
        tx["entry_types"].add(row["entry_type"])
        if row["at_category"]:
            tx["at_categories"].add(str(row["at_category"]))
        if position > 0:
            continue
        amounts = _journal_amounts(row)
        asset = _asset(row["asset"])
        key = (row["wallet"], asset.asset_id)
        _apply_holding(closing[key], row, amounts)
        if position < 0:
            _apply_holding(opening[key], row, amounts)
            continue

        entry_type = row["entry_type"]
        stored = int(row["quantity"])
        magnitude = abs(stored)
        asset_roll = roll[asset.asset_id]
        if entry_type == "acquisition":
            asset_roll["acquired"] += magnitude
            asset_roll["acquired_value"] += amounts["fiat_value"]
        elif entry_type == "income":
            asset_roll["income_value"] += amounts["fiat_value"]
            _add(tx, "income_value", amounts["fiat_value"])
        elif entry_type == "disposal":
            asset_roll["disposed"] += magnitude
            asset_roll["disposed_cost_basis"] += amounts["cost_basis"] or Decimal("0")
            asset_roll["disposal_proceeds"] += amounts["proceeds"] or Decimal("0")
            asset_roll["realized_gain_loss"] += amounts["gain_loss"] or Decimal("0")
            _add(tx, "disposal_proceeds", amounts["proceeds"])
            _add(tx, "disposal_cost_basis", amounts["cost_basis"])
            _add(tx, "disposal_gain_loss", amounts["gain_loss"])
        elif entry_type in {"fee", "transfer_fee"}:
            asset_roll["fee"] += magnitude
            asset_roll["fee_cost_basis"] += amounts["cost_basis"] or Decimal("0")
            _add(tx, "fee_cost_basis", amounts["cost_basis"])
            if entry_type == "transfer_fee":
                # Already inside transfer_out; add it back so the transfer
                # column nets to zero and the fee is counted once.
                asset_roll["transfer_net"] += magnitude
        elif entry_type == "transfer_in":
            asset_roll["transfer_net"] += magnitude
            asset_roll["transfer_basis_net"] += amounts["fiat_value"]
        elif entry_type == "transfer_out":
            asset_roll["transfer_net"] -= magnitude
            asset_roll["transfer_basis_net"] -= amounts["cost_basis"] or Decimal("0")
        else:
            raise AppError(
                f"Unknown journal entry type '{entry_type}' in the year handoff.",
                code="year_handoff_unsupported_entry",
                details={"journal_entry_id": row["id"], "entry_type": entry_type},
                retryable=False,
            )
        journal_rows.append(
            {
                "occurred_at": row["occurred_at"],
                "local_date": period.local_date(row["occurred_at"]),
                "wallet": row["wallet"],
                "transaction_ref": row["transaction_id"],
                "transaction_id": row["external_id"],
                "entry_type": entry_type,
                **_asset_fields(asset),
                **_quantity_fields(asset, "quantity", stored),
                "fiat_value": _money(amounts["fiat_value"]),
                "cost_basis": _money(amounts["cost_basis"]),
                "proceeds": _money(amounts["proceeds"]),
                "gain_loss": _money(amounts["gain_loss"]),
                "in_capital_gains_report": int(row["in_capital_gains_report"]),
                "tax_category": row["at_category"] or "",
                "tax_form_field": row["at_kennzahl"] if row["at_kennzahl"] is not None else "",
                "capital_gains_type": row["capital_gains_type"] or "",
                "pricing_source": row["pricing_source"],
                "pricing_quality": row["pricing_quality"],
                "description": strip_markers(row["description"]),
                "journal_entry_id": row["id"],
            }
        )

    # Transactions: the same row builder as the transactions export, cut to
    # the year and joined with the journal-derived classification.
    query_rows = _report_query_rows(conn, profile)
    export_rows = _generic_report_transaction_rows({"query_rows": query_rows})
    tx_meta = {
        row["id"]: row
        for row in conn.execute(
            """
            SELECT id, COALESCE(NULLIF(kind_override, ''), kind, '') AS kind,
                   amount_includes_fee
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            """,
            (profile["id"],),
        ).fetchall()
    }
    observed_opening: dict[tuple[str, str], int] = defaultdict(int)
    flows: dict[tuple[str, str], dict[str, int]] = defaultdict(
        lambda: {"inflow": 0, "outflow": 0, "fee": 0}
    )
    wallet_activity: dict[str, dict[str, Any]] = defaultdict(
        lambda: {"in_year": 0, "first": None, "last": None, "assets": set()}
    )
    transaction_rows = []
    for source, row in zip(query_rows["transactions"], export_rows):
        activity = wallet_activity[source["wallet"]]
        activity["first"] = activity["first"] or source["occurred_at"]
        activity["last"] = source["occurred_at"]
        activity["assets"].add(_asset(source["asset"]).display_code)
        position = period.position(source["occurred_at"])
        if position > 0:
            continue
        tx_ref = source["row_id"]
        meta = tx_meta.get(tx_ref)
        amounts = normalize_boundary_amounts(
            direction=source["direction"],
            amount_msat=source["amount"],
            fee_msat=source["fee"],
            amount_includes_fee=bool(meta["amount_includes_fee"]) if meta else False,
        )
        asset = _asset(source["asset"])
        key = (source["wallet"], asset.asset_id)
        if position < 0:
            observed_opening[key] += amounts.wallet_delta_msat
            continue
        activity["in_year"] += 1
        flow = flows[key]
        if amounts.direction == "outbound":
            flow["outflow"] += amounts.principal_msat
            flow["fee"] += amounts.fee_msat
        else:
            flow["inflow"] += amounts.wallet_movement_msat
        journal = per_transaction.get(tx_ref)
        entry_types = journal["entry_types"] if journal else set()
        at_categories = journal["at_categories"] if journal else set()
        wallet = wallet_meta.get(source["wallet"], {})
        link = pair_link_map.get(tx_ref)
        amount_stored = int(source["amount"] or 0)
        fee_stored = int(source["fee"] or 0)
        transaction_rows.append(
            {
                "occurred_at": row["occurred_at"],
                "local_date": period.local_date(row["occurred_at"]),
                "wallet": row["wallet"],
                "wallet_kind": wallet.get("wallet_kind", ""),
                "chain": wallet.get("chain") or asset.chain,
                "network": wallet.get("network", ""),
                "transaction_ref": tx_ref,
                "transaction_id": row["transaction_id"],
                "direction": row["direction"],
                "classification": classify(
                    entry_types, at_categories, tx_ref in swap_transaction_ids
                ),
                "source_kind": meta["kind"] if meta else "",
                "pair_ref": ", ".join(sorted(link["refs"])) if link else "",
                "other_wallet": row["transfer"],
                "other_leg_transaction_id": ", ".join(sorted(link["others"])) if link else "",
                **_asset_fields(asset),
                **_quantity_fields(asset, "amount", amount_stored),
                **_quantity_fields(asset, "fee", fee_stored),
                **_quantity_fields(asset, "net_change", amounts.wallet_delta_msat),
                "amount_sats": asset.sats(amount_stored),
                "fee_sats": asset.sats(fee_stored),
                "net_change_sats": asset.sats(amounts.wallet_delta_msat),
                "fiat_price": row["fiat_price"],
                "fiat_value": row["fiat_value"],
                "fmv": row["fmv"],
                "fiat_fee": row["fiat_fee"],
                "income_value": _money(journal["income_value"]) if journal else None,
                "disposal_proceeds": _money(journal["disposal_proceeds"]) if journal else None,
                "disposal_cost_basis": _money(journal["disposal_cost_basis"]) if journal else None,
                "disposal_gain_loss": _money(journal["disposal_gain_loss"]) if journal else None,
                "fee_cost_basis": _money(journal["fee_cost_basis"]) if journal else None,
                "journal_entry_types": ", ".join(sorted(entry_types)),
                "tax_category": ", ".join(sorted(at_categories)),
                "description": row["description"],
                "note": row["note"],
                "counterparty": row["counterparty"],
                "tags": row["tags"],
                "references": row["references"],
                "attachments": row["attachments"],
                "attachments_list": row["attachments_list"],
            }
        )

    excluded_rows = _excluded_rows(conn, profile, period)

    # Balances at both ends of the year.
    opening_basis, opening_pool_basis = _allocate_wallet_basis(opening)
    closing_basis, closing_pool_basis = _allocate_wallet_basis(closing)
    assets = {asset for (_wallet, asset) in closing} | {asset for (_wallet, asset) in opening}
    year_end_rates = _year_end_rates(
        conn, profile, period, {asset_id: asset_spellings[asset_id] for asset_id in assets}
    )
    balance_rows = []
    for key in sorted(set(opening) | set(closing)):
        wallet_label, asset_key = key
        open_stored = opening[key]["stored"] if key in opening else 0
        close_stored = closing[key]["stored"] if key in closing else 0
        if open_stored == 0 and close_stored == 0:
            continue
        asset = asset_by_id[asset_key]
        wallet = wallet_meta.get(wallet_label, {})
        rate = year_end_rates.get(asset_key)
        balance_rows.append(
            {
                "wallet": wallet_label,
                "chain": wallet.get("chain") or asset.chain,
                "network": wallet.get("network", ""),
                **_asset_fields(asset),
                **_quantity_fields(asset, "opening_quantity", open_stored),
                "opening_cost_basis": _money(opening_basis.get(key, Decimal("0"))),
                **_quantity_fields(asset, "closing_quantity", close_stored),
                "closing_cost_basis": _money(closing_basis.get(key, Decimal("0"))),
                "year_end_rate": _money(rate["rate"]) if rate else None,
                "year_end_market_value": (
                    _money(asset.quantity(close_stored) * rate["rate"]) if rate else None
                ),
                "rate_source": rate["source"] if rate else "missing",
                "rate_timestamp": rate["timestamp"] if rate else "",
                "_closing_stored": close_stored,
            }
        )

    # Per-asset roll-forward: opening + movements = closing, in units and fiat.
    roll_rows = []
    for asset_key in sorted(assets | set(roll)):
        movement = roll[asset_key]
        asset = asset_by_id[asset_key]
        open_stored = sum(value["stored"] for (_w, a), value in opening.items() if a == asset_key)
        close_stored = sum(value["stored"] for (_w, a), value in closing.items() if a == asset_key)
        open_basis = opening_pool_basis.get(asset_key, Decimal("0"))
        close_basis = closing_pool_basis.get(asset_key, Decimal("0"))
        quantity_check = (
            open_stored
            + movement["acquired"]
            - movement["disposed"]
            - movement["fee"]
            + movement["transfer_net"]
            - close_stored
        )
        basis_check = (
            open_basis
            + movement["acquired_value"]
            - movement["disposed_cost_basis"]
            - movement["fee_cost_basis"]
            + movement["transfer_basis_net"]
            - close_basis
        )
        rate = year_end_rates.get(asset_key)
        market_value = asset.quantity(close_stored) * rate["rate"] if rate else None
        roll_rows.append(
            {
                **_asset_fields(asset),
                **_quantity_fields(asset, "opening_quantity", open_stored),
                "acquired_quantity": asset.quantity(movement["acquired"]),
                "disposed_quantity": asset.quantity(movement["disposed"]),
                "fee_quantity": asset.quantity(movement["fee"]),
                "transfer_net": asset.quantity(movement["transfer_net"]),
                **_quantity_fields(asset, "closing_quantity", close_stored),
                "quantity_check": asset.quantity(quantity_check),
                "opening_cost_basis": _money(open_basis),
                "acquired_value": _money(movement["acquired_value"]),
                "income_value": _money(movement["income_value"]),
                "disposed_cost_basis": _money(movement["disposed_cost_basis"]),
                "disposal_proceeds": _money(movement["disposal_proceeds"]),
                "realized_gain_loss": _money(movement["realized_gain_loss"]),
                "fee_cost_basis": _money(movement["fee_cost_basis"]),
                "transfer_basis_net": _money(movement["transfer_basis_net"]),
                "closing_cost_basis": _money(close_basis),
                "basis_check": _money(basis_check),
                "year_end_rate": _money(rate["rate"]) if rate else None,
                "year_end_market_value": _money(market_value),
                "unrealized_gain_loss": (
                    _money(market_value - close_basis) if market_value is not None else None
                ),
                "_quantity_check_stored": quantity_check,
            }
        )

    # Reconciliation: the processed journal against observed wallet movements.
    reconciliation_rows = []
    for key in sorted(set(opening) | set(closing) | set(observed_opening) | set(flows)):
        wallet_label, asset_key = key
        asset = asset_by_id[asset_key]
        opening_journal = opening[key]["stored"] if key in opening else 0
        closing_journal = closing[key]["stored"] if key in closing else 0
        opening_observed = observed_opening.get(key, 0)
        flow = flows.get(key, {"inflow": 0, "outflow": 0, "fee": 0})
        closing_observed = opening_observed + flow["inflow"] - flow["outflow"] - flow["fee"]
        if not any(
            (opening_journal, closing_journal, opening_observed, closing_observed, *flow.values())
        ):
            continue
        opening_difference = opening_journal - opening_observed
        closing_difference = closing_journal - closing_observed
        reconciliation_rows.append(
            {
                "wallet": wallet_label,
                **_asset_fields(asset),
                "opening_journal": asset.quantity(opening_journal),
                "opening_observed": asset.quantity(opening_observed),
                "inflow": asset.quantity(flow["inflow"]),
                "outflow": asset.quantity(flow["outflow"]),
                "fee": asset.quantity(flow["fee"]),
                "closing_observed": asset.quantity(closing_observed),
                "closing_journal": asset.quantity(closing_journal),
                "opening_difference": asset.quantity(opening_difference),
                "closing_difference": asset.quantity(closing_difference),
                "status": "OK" if opening_difference == 0 and closing_difference == 0 else "DIFF",
                "opening_difference_exact": asset.exact(opening_difference),
                "closing_difference_exact": asset.exact(closing_difference),
                "timing_notes": "\n".join(timing_notes.get(key, ())),
            }
        )

    # Wallet coverage: every wallet in the book, active or not this year.
    excluded_by_wallet: dict[str, int] = defaultdict(int)
    for row in excluded_rows:
        excluded_by_wallet[row["wallet"]] += 1
    wallet_rows = []
    for label in sorted(set(wallet_meta) | set(wallet_activity)):
        meta = wallet_meta.get(label, {})
        activity = wallet_activity.get(label, {"in_year": 0, "first": None, "last": None, "assets": set()})
        wallet_rows.append(
            {
                "wallet": label,
                "wallet_kind": meta.get("wallet_kind", ""),
                "chain": meta.get("chain", ""),
                "network": meta.get("network", ""),
                "status": meta.get("status", ""),
                "assets": ", ".join(sorted(activity["assets"])),
                "transactions_in_year": activity["in_year"],
                "excluded_in_year": excluded_by_wallet.get(label, 0),
                "first_transaction_at": activity["first"] or "",
                "last_transaction_at": activity["last"] or "",
                "last_synced_at": meta.get("last_synced_at", ""),
            }
        )

    differences = [row for row in reconciliation_rows if row["status"] != "OK"]
    roll_differences = [
        row
        for row in roll_rows
        if row["_quantity_check_stored"] != 0 or abs(row["basis_check"] or 0) >= 0.005
    ]
    missing_rates = sorted(
        {
            row["asset"]
            for row in balance_rows
            if row["_closing_stored"] and row["rate_source"] == "missing"
        }
    )
    fallback_rates = sorted(
        {
            f"{row['asset']} ({row['rate_timestamp']})"
            for row in balance_rows
            if row["_closing_stored"] and row["rate_source"] == "transaction price"
        }
    )
    warnings = []
    if differences:
        warnings.append(
            f"{len(differences)} wallet/asset rows do not reconcile with observed "
            "wallet movements; see the Reconciliation sheet."
        )
    if roll_differences:
        warnings.append(
            "The roll-forward does not close for "
            + ", ".join(row["asset"] for row in roll_differences)
            + "."
        )
    if missing_rates:
        warnings.append(
            "No year-end rate for " + ", ".join(missing_rates) + "; market value is left blank."
        )
    if fallback_rates:
        warnings.append(
            "Year-end market value uses the last transaction price for "
            + ", ".join(fallback_rates)
            + ", not a market rate at year end. Sync or set a rate before valuing holdings."
        )

    import kassiber

    return {
        "workspace": workspace,
        "profile": profile,
        "period": period,
        "generated_at": hooks.now_iso(),
        "kassiber_version": getattr(kassiber, "__version__", ""),
        "wallets": wallet_rows,
        "transactions": transaction_rows,
        "journal": journal_rows,
        "balances": balance_rows,
        "roll_forward": roll_rows,
        "reconciliation": reconciliation_rows,
        "excluded": excluded_rows,
        "reconciled": not differences and not roll_differences,
        "warnings": warnings,
    }


def _overview_rows(handoff) -> list[dict[str, Any]]:
    profile = handoff["profile"]
    period = handoff["period"]
    transactions = handoff["transactions"]
    classification_counts: dict[str, int] = defaultdict(int)
    for row in transactions:
        classification_counts[row["classification"]] += 1
    assets: dict[str, str] = {}
    for sheet in ("transactions", "balances", "journal"):
        for row in handoff[sheet]:
            assets.setdefault(row["asset_id"], row["asset"])
    rows = [
        ("Scope", "Workspace", _row_get(handoff["workspace"], "label")),
        ("Scope", "Book", _row_get(profile, "label")),
        ("Scope", "Year", period.year),
        ("Scope", "Calendar", period.calendar),
        ("Scope", "Period start (UTC)", _Period.iso(period.start)),
        ("Scope", "Period end (UTC, exclusive)", _Period.iso(period.end)),
        ("Scope", "Book currency", _row_get(profile, "fiat_currency")),
        ("Scope", "Tax country", _row_get(profile, "tax_country")),
        ("Scope", "Lot method", _row_get(profile, "gains_algorithm")),
        ("Scope", "Wallets in book", len(handoff["wallets"])),
    ]
    for asset_id in sorted(assets):
        rows.append(("Scope", "Asset", f"{assets[asset_id]} ({asset_id})"))
    rows += [
        ("Run", "Generated at", handoff["generated_at"]),
        ("Run", "Journals processed at", _row_get(profile, "last_processed_at")),
        ("Run", "Kassiber version", handoff["kassiber_version"]),
        ("Status", "Reconciliation", "OK" if handoff["reconciled"] else "DIFFERENCES"),
        ("Status", "Quarantined transactions in or before the year", 0),
    ]
    for warning in handoff["warnings"]:
        rows.append(("Status", "Warning", warning))
    rows.append(("Counts", "Transactions in the year", len(transactions)))
    rows.append(("Counts", "Excluded transactions in the year", len(handoff["excluded"])))
    rows.append(("Counts", "Journal entries in the year", len(handoff["journal"])))
    for classification in sorted(classification_counts):
        rows.append(("Classification", classification, classification_counts[classification]))
    notes = (
        "Wallets lists every wallet in this book. Wallets in other books are not "
        "included; a transfer to one of them appears as an external payment.",
        "Transactions lists every non-excluded transaction observed in the year. "
        "Excluded transactions are listed on the Excluded sheet and count nowhere else.",
        "Transaction Ref is Kassiber's id; Journal rows carry the same ref. Both legs of an "
        "own-wallet transfer or swap carry the same Pair Ref.",
        "Kassiber Classification is how the journal booked the row: internal_transfer moves "
        "between own wallets, swap is a reviewed conversion, income/acquisition/disposal "
        "cross the book's boundary, fee is a fee-only row. Source Kind is what the "
        "import or sync said.",
        "Amounts are in each asset's own unit. The (exact) columns are lossless text; "
        "sats columns are filled for BTC and L-BTC only. Net Change is signed: "
        "negative leaves the wallet.",
        "Money columns are in the book currency. Recorded Value is the cash amount "
        "recorded with the transaction (including exchange fees); Market Value is "
        "amount x price.",
        "Per-wallet cost basis is the asset's pooled residual basis allocated by quantity, "
        "as in portfolio-summary; the per-asset Roll-forward total is exact.",
        "Reconciliation compares journal balances with observed wallet movements at both "
        "ends of the year. A DIFF row needs an explanation before closing the books.",
        "Kassiber does not assign spheres, accounts, or the UGB/EA valuation rule. Tag rows "
        "in Kassiber to carry that assignment into this workbook.",
    )
    for note in notes:
        rows.append(("Notes", "", note))
    return [{"group": group, "field": field, "value": value} for group, field, value in rows]


def _handoff_sheets(handoff) -> list[dict[str, Any]]:
    year = handoff["period"].year
    return [
        {"name": "Overview", "title": f"Kassiber year handoff {year}", "columns": OVERVIEW_COLUMNS, "rows": _overview_rows(handoff)},
        {"name": "Wallets", "title": f"Wallets in this book {year}", "columns": WALLET_COLUMNS, "rows": handoff["wallets"]},
        {"name": "Transactions", "title": f"Transactions {year}", "columns": TRANSACTION_COLUMNS, "rows": handoff["transactions"]},
        {"name": "Journal", "title": f"Journal entries {year}", "columns": JOURNAL_COLUMNS, "rows": handoff["journal"]},
        {"name": "Balances", "title": f"Opening and closing balances {year}", "columns": BALANCE_COLUMNS, "rows": handoff["balances"]},
        {"name": "Roll-forward", "title": f"Per-asset roll-forward {year}", "columns": ROLL_FORWARD_COLUMNS, "rows": handoff["roll_forward"]},
        {"name": "Reconciliation", "title": f"Journal vs observed wallet movements {year}", "columns": RECONCILIATION_COLUMNS, "rows": handoff["reconciliation"]},
        {"name": "Excluded", "title": f"Excluded transactions {year}", "columns": EXCLUDED_COLUMNS, "rows": handoff["excluded"]},
    ]


def _handoff_formats(workbook):
    formats = _generic_report_xlsx_formats(workbook)
    # Up to 11 decimals so msat-scale amounts stay visible; the (exact)
    # columns carry the lossless value for any asset.
    formats["quantity"] = workbook.add_format(
        {"font_size": 11, "valign": "top", "num_format": "#,##0.00000000###"}
    )
    formats["sats"] = workbook.add_format({"font_size": 11, "valign": "top", "num_format": "#,##0.###"})
    return formats


def _write_sheet(workbook, sheet, formats, fiat_currency) -> None:
    from xlsxwriter.utility import xl_rowcol_to_cell

    columns = sheet["columns"]
    rows = sheet["rows"]
    labels = [column.label.format(fiat=fiat_currency) for column in columns]
    worksheet = workbook.add_worksheet(sheet["name"])
    worksheet.set_landscape()
    worksheet.fit_to_pages(1, 0)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)
    last_column = max(len(columns) - 1, 0)
    widths = []
    for column, label in zip(columns, labels):
        sample = max((min(len(str(row.get(column.key, "") or "")), 36) for row in rows[:50]), default=0)
        widths.append(max(10, min(max(len(label), sample) + 2, 38)))
    worksheet.set_row(0, 28)
    worksheet.merge_range(0, 0, 0, last_column, sheet["title"], formats["title"])
    header_lines = max(
        (_estimate_wrapped_lines(label, widths[index]) for index, label in enumerate(labels)),
        default=1,
    )
    worksheet.set_row(1, max(24, header_lines * 15 + 4))
    for index, label in enumerate(labels):
        worksheet.set_column(index, index, widths[index])
        worksheet.write_string(1, index, label, formats["header"])
    worksheet.freeze_panes(2, 0)

    row_index = 2
    for row in rows:
        cells = {column.key: xl_rowcol_to_cell(row_index, index) for index, column in enumerate(columns)}
        lines = 1
        for index, column in enumerate(columns):
            value = row.get(column.key, "")
            cell_format = formats[column.format if value not in (None, "") else "text"]
            if column.key == "attachments" and "attachments_list" in row:
                _write_attachments_cell(worksheet, row_index, index, row, formats)
                lines = max(lines, len(row.get("attachments_list") or []) or 1)
            elif column.formula:
                # The cached value is Kassiber's own result, so a reader without
                # a formula engine sees what Excel recomputes.
                formula = column.formula.format(decimals=row.get("_decimals", 2), **cells)
                cached = float(value) if isinstance(value, Decimal) else value
                worksheet.write_formula(row_index, index, formula, formats[column.format], cached)
            else:
                if column.format == "text":
                    lines = max(lines, _estimate_wrapped_lines(value, widths[index]))
                _xlsx_write_value(worksheet, row_index, index, value, cell_format)
        worksheet.set_row(row_index, max(16, lines * 15 + 4))
        row_index += 1
    if not rows:
        worksheet.write_string(row_index, 0, "No rows in scope.", formats["text"])
        row_index += 1
    worksheet.autofilter(1, 0, max(row_index - 1, 1), last_column)


def export_year_handoff_xlsx(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    hooks: ReportHooks,
    *,
    year,
) -> dict[str, Any]:
    import xlsxwriter

    handoff = build_year_handoff(conn, workspace_ref, profile_ref, hooks, year=year)
    sheets = _handoff_sheets(handoff)
    period = handoff["period"]
    fiat_currency = str(_row_get(handoff["profile"], "fiat_currency") or "")
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    workbook = xlsxwriter.Workbook(str(path))
    workbook.set_calc_mode("auto")
    workbook.set_properties(
        {
            "title": f"Kassiber year handoff {period.year} - {_row_get(handoff['profile'], 'label')}",
            "subject": "Kassiber year handoff for EA / Jahresabschluss",
            "author": "Kassiber",
            "comments": "Generated from the processed journal; checks are live formulas.",
        }
    )
    formats = _handoff_formats(workbook)
    for sheet in sheets:
        _write_sheet(workbook, sheet, formats, fiat_currency)
    workbook.close()

    result = {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "year": period.year,
        "calendar": period.calendar,
        "period_start": _Period.iso(period.start),
        "period_end": _Period.iso(period.end),
        "sheets": [sheet["name"] for sheet in sheets],
        "rows": {
            "wallets": len(handoff["wallets"]),
            "transactions": len(handoff["transactions"]),
            "journal": len(handoff["journal"]),
            "balances": len(handoff["balances"]),
            "reconciliation": len(handoff["reconciliation"]),
            "excluded": len(handoff["excluded"]),
        },
        "reconciled": handoff["reconciled"],
        "warnings": handoff["warnings"],
    }
    result["report_snapshot"] = _register_saved_export(
        conn,
        workspace=handoff["workspace"],
        profile=handoff["profile"],
        report_kind=REPORT_KIND,
        artifact_paths=(path,),
        period_start_year=period.year,
        period_end_year=period.year,
    )
    return result


__all__ = [
    "build_year_handoff",
    "export_year_handoff_xlsx",
]
