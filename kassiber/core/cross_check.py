"""Cross-check a CoinTracking or Blockpit export against the book.

A user who keeps a second tool, or moves between tools, needs to see where the
tool and Kassiber disagree before trusting either. This reads the tool's own
CSV export with the same parser as the import (``read_tax_platform_rows``),
matches its rows to Kassiber transactions, and reports every difference that
changes a tax result separately from cosmetic ones.

Matching, in order:

1. Transaction id and direction. Kassiber's own tool exports carry the txid,
   or the Kassiber row id for rows without one, so a round trip matches fully.
2. Rows without a usable id: same direction and asset, the same amount (with
   or without the fee folded in, to the satoshi) within a day of each other.
   These matches are labelled ``probable``.
3. Leftover tool rows against transactions the user excluded in Kassiber, so
   "the tool still has a row I removed" is named as such.

A consistent clock offset between the two (a tool account kept in local time)
is detected from id matches and applied to everything else, then reported once
instead of as a time difference on every row.

The comparison is a diagnostic, not a report: it reads the book as it is.
Stale journals only switch off the type comparison (with a notice), and
quarantined rows are compared like any other. Nothing is written to the book.
Only Bitcoin-family rows (BTC, L-BTC) are compared because the tool parser
reads only those; other assets are counted and named, never silently dropped.
"""

from __future__ import annotations

import bisect
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any, Iterable, Mapping
from zoneinfo import ZoneInfo

from ..errors import AppError
from ..importers import TAX_PLATFORM_READERS, read_tax_platform_rows
from ..msat import btc_to_msat, dec
from . import custody_journal
from .asset_presentation import describe_asset
from .custody_evidence import normalize_boundary_amounts
from .reports import (
    ReportHooks,
    XLSX_BODY,
    XLSX_MONEY_FORMAT,
    _generic_report_xlsx_formats,
    _row_get,
    _xlsx_print_setup,
    _xlsx_write_value,
    export_wallet_scope_label,
    resolve_export_wallets,
)
from .transaction_classification import classify, journal_entry_types, pair_links
from .transaction_formats import _journal_values
from .year_handoff import _period_for_profile, accounting_years

STATUSES = (
    "different",
    "only_tool",
    "only_kassiber",
    "excluded_in_kassiber",
    "matched",
    "not_comparable",
)
COMPARED_ASSETS = frozenset({"bitcoin:btc", "liquid:lbtc"})

# One satoshi: tools keep 8 decimals, Lightning rows in Kassiber keep msat.
_AMOUNT_TOLERANCE_MSAT = 1000
_PROBABLE_WINDOW = timedelta(hours=26)
_TIME_NOTICE = timedelta(minutes=10)
_OFFSET_MIN_PAIRS = 3
_OFFSET_SHARE = Decimal("0.8")
_VALUE_TOLERANCE_SHARE = Decimal("0.01")
_VALUE_TOLERANCE_MIN = Decimal("1")

# What a row means for tax, on each side. A tool's "deposit" may be
# Kassiber's acquisition from an unknown sender (neither is taxable at
# receipt), and a fee-only row may appear as a withdrawal, so those pairs are
# compatible; everything else that differs is a tax difference.
_TOOL_KIND_CLASS = {
    "buy": "acquire",
    None: "acquire",
    "deposit": "transfer",
    "withdrawal": "transfer",
    "sell": "dispose",
    "spend": "dispose",
    "gift": "dispose",
    "donation": "dispose",
    "lost": "dispose",
    "stolen": "dispose",
    "expense_non_taxable": "dispose_free",
    "income": "income",
    "wages": "income",
    "mining": "income",
    "staking": "income",
    "interest": "income",
    "lending_interest": "income",
    "airdrop": "income",
    "hardfork": "income",
}
_KASSIBER_CLASS = {
    "internal_transfer": "transfer",
    "swap": "transfer",
    "income": "income",
    "acquisition": "acquire",
    "disposal": "dispose",
    "fee": "dispose",
}
_VALUE_COLUMNS = {
    "inbound": ("Buy Value in Account Currency", "Buy Value in EUR", "Buy Value"),
    "outbound": ("Sell Value in Account Currency", "Sell Value in EUR", "Sell Value"),
}


def _utc(value: Any) -> datetime | None:
    text = str(value or "").strip()
    if not text:
        return None
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _rows(count: int) -> str:
    return f"{count} row" if count == 1 else f"{count} rows"


def _iso(value: datetime | None) -> str:
    return value.strftime("%Y-%m-%dT%H:%M:%SZ") if value else ""


def _money(value: Decimal | None) -> str:
    if value is None:
        return ""
    return format(value.quantize(Decimal("0.01")), "f")


def _cell(record: Mapping[str, Any], *names: str) -> str:
    wanted = {name.casefold() for name in names}
    for key, value in record.items():
        if str(key).strip().casefold() in wanted and str(value or "").strip():
            return str(value).strip()
    return ""


def _decimal_cell(text: str) -> Decimal | None:
    """A value cell in either 1,234.56 or 1.234,56 notation."""
    if not text:
        return None
    cleaned = text.replace("\u00a0", "").replace(" ", "")
    if "," in cleaned and cleaned.rfind(",") > cleaned.rfind("."):
        cleaned = cleaned.replace(".", "").replace(",", ".")
    else:
        cleaned = cleaned.replace(",", "")
    try:
        return abs(Decimal(cleaned))
    except ArithmeticError:
        return None


@dataclass
class _ToolRow:
    index: int
    source: str
    label: str
    txid: str
    occurred_at: datetime | None
    direction: str = ""
    asset_id: str = ""
    amount_msat: int = 0
    fee_msat: int = 0
    kind: str | None = None
    value: Decimal | None = None
    value_currency: str = ""
    error: str = ""

    @property
    def comparable(self) -> bool:
        return not self.error and bool(self.direction)


@dataclass
class _BookRow:
    id: str
    wallet: str
    wallet_id: str
    txid: str
    occurred_at: datetime
    direction: str
    asset_id: str
    principal_msat: int
    fee_msat: int
    classification: str
    value: Decimal | None
    excluded: bool
    # The user reviewed this row as not taxable (taxability_override = 0).
    non_taxable: bool = False
    matched: bool = field(default=False)

    @property
    def movement_msat(self) -> int:
        return self.principal_msat + (self.fee_msat if self.direction == "outbound" else 0)


def _tool_rows(tool: str, file_path, book_currency: str) -> tuple[list[_ToolRow], int]:
    """Comparable tool rows plus the count of rows in other assets."""
    _, parsed = read_tax_platform_rows(tool, file_path)
    rows: list[_ToolRow] = []
    other_assets = 0
    source_columns = ("Exchange",) if tool == "cointracking" else ("Integration Name",)
    label_columns = ("Type",) if tool == "cointracking" else ("Label",)
    for item in parsed:
        record = item["record"]
        normalized = item["normalized"]
        source = _cell(record, *source_columns) or "—"
        label = _cell(record, *label_columns)
        if item["error"]:
            rows.append(
                _ToolRow(
                    index=item["index"],
                    source=source,
                    label=label,
                    txid=_cell(record, "Tx-ID", "Tx-ID (optional)", "Trx. ID", "Trx. ID (optional)"),
                    occurred_at=_utc(_cell(record, "Date", "Date (UTC)")),
                    error=item["error"],
                )
            )
            continue
        if normalized is None:
            other_assets += 1
            continue
        direction = normalized["direction"]
        asset = describe_asset(normalized["asset"])
        value = None
        currency = ""
        fiat_value = normalized.get("fiat_value")
        fiat_currency = str(normalized.get("fiat_currency") or "").upper()
        if fiat_value is not None and fiat_currency in {"", book_currency}:
            value, currency = dec(fiat_value), book_currency
        else:
            column_value = _decimal_cell(_cell(record, *_VALUE_COLUMNS[direction]))
            if column_value is not None:
                value, currency = column_value, book_currency
        rows.append(
            _ToolRow(
                index=item["index"],
                source=source,
                label=label,
                txid=str(normalized.get("txid") or "").strip(),
                occurred_at=_utc(normalized["occurred_at"]),
                direction=direction,
                asset_id=asset.asset_id,
                amount_msat=btc_to_msat(normalized["amount"]) or 0,
                fee_msat=btc_to_msat(normalized.get("fee") or 0) or 0,
                kind=normalized.get("kind"),
                value=value,
                value_currency=currency,
            )
        )
    return rows, other_assets


def _book_rows(conn, profile, wallet_ids: set[str] | None, book_currency: str):
    """Kassiber transactions in wallet scope, plus counts of uncompared assets."""
    tx_rows = conn.execute(
        """
        SELECT t.id, t.wallet_id, w.label AS wallet, t.occurred_at, t.direction,
               t.asset, t.amount, t.fee, t.amount_includes_fee, t.excluded,
               COALESCE(t.external_id, '') AS external_id,
               COALESCE(t.fiat_value_exact, t.fiat_value) AS fiat_value,
               COALESCE(t.fiat_currency, '') AS fiat_currency,
               t.taxability_override
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ?
        ORDER BY t.occurred_at, t.id
        """,
        (profile["id"],),
    ).fetchall()
    external_ids = {row["id"]: row["external_id"] for row in tx_rows if not row["excluded"]}
    _, swap_ids = pair_links(conn, profile["id"], external_ids)
    entry_types = journal_entry_types(conn, profile["id"])
    journal_values = _journal_values(conn, profile["id"])
    rows: list[_BookRow] = []
    other_assets: Counter[str] = Counter()
    for row in tx_rows:
        if wallet_ids is not None and row["wallet_id"] not in wallet_ids:
            continue
        asset = describe_asset(row["asset"])
        if asset.asset_id not in COMPARED_ASSETS:
            if not row["excluded"]:
                other_assets[asset.display_code] += 1
            continue
        amounts = normalize_boundary_amounts(
            direction=row["direction"],
            amount_msat=row["amount"],
            fee_msat=row["fee"],
            amount_includes_fee=bool(row["amount_includes_fee"]),
        )
        facts = entry_types.get(row["id"], {"entry_types": set(), "at_categories": set()})
        classification = classify(facts["entry_types"], facts["at_categories"], row["id"] in swap_ids)
        journal = journal_values.get(row["id"], {})
        if "income" in journal:
            value = journal["income"]
        elif "proceeds" in journal:
            value = journal["proceeds"]
        elif "cost" in journal and "swap" not in classification:
            # What the journal booked as the acquisition's cost: the carried
            # basis of a conversion, the market value otherwise. The tool
            # export writes the same figure.
            value = journal["cost"]
        elif row["fiat_value"] not in (None, "") and str(row["fiat_currency"]).upper() in {"", book_currency}:
            value = dec(row["fiat_value"])
        else:
            value = None
        rows.append(
            _BookRow(
                id=row["id"],
                wallet=row["wallet"],
                wallet_id=row["wallet_id"],
                txid=row["external_id"],
                occurred_at=_utc(row["occurred_at"]) or datetime.min.replace(tzinfo=timezone.utc),
                direction=amounts.direction,
                asset_id=asset.asset_id,
                principal_msat=int(amounts.principal_msat or 0),
                fee_msat=int(amounts.fee_msat or 0),
                classification=classification,
                value=value,
                excluded=bool(row["excluded"]),
                non_taxable=row["taxability_override"] == 0,
            )
        )
    return rows, other_assets


def _same(a: int, b: int) -> bool:
    return abs(a - b) < _AMOUNT_TOLERANCE_MSAT


def _amount_fits(tool: _ToolRow, book: _BookRow) -> bool:
    if _same(tool.amount_msat, book.principal_msat):
        return True
    if book.direction != "outbound":
        return False
    return _same(tool.amount_msat + tool.fee_msat, book.movement_msat) or _same(
        tool.amount_msat, book.movement_msat
    )


def _is_chain_txid(value: str) -> bool:
    return len(value) == 64 and all(char in "0123456789abcdef" for char in value.lower())


def _conflicting_ids(tool: _ToolRow, book: _BookRow) -> bool:
    """Both sides name a different on-chain transaction: not the same row."""
    return (
        _is_chain_txid(tool.txid)
        and _is_chain_txid(book.txid)
        and tool.txid.lower() != book.txid.lower()
    )


def _detect_offset(pairs: Iterable[tuple[_ToolRow, _BookRow]]) -> timedelta:
    """A whole-hour clock offset most id matches share, or none.

    A pair only votes for an hour when it sits within the time tolerance of
    it, so ordinary drift (a tool stamping 31 minutes late) never becomes a
    timezone correction.
    """
    hours = Counter()
    total = 0
    tolerance = _TIME_NOTICE.total_seconds()
    for tool, book in pairs:
        if tool.occurred_at is None:
            continue
        total += 1
        seconds = (tool.occurred_at - book.occurred_at).total_seconds()
        hour = round(seconds / 3600)
        if abs(seconds - hour * 3600) <= tolerance:
            hours[hour] += 1
    if total < _OFFSET_MIN_PAIRS or not hours:
        return timedelta(0)
    hour, count = hours.most_common(1)[0]
    if hour == 0 or abs(hour) > 14 or Decimal(count) < _OFFSET_SHARE * total:
        return timedelta(0)
    return timedelta(hours=hour)


class _Index:
    """Unmatched book rows by (direction, asset), ordered by time."""

    def __init__(self, rows: Iterable[_BookRow]):
        self._rows: dict[tuple[str, str], list[_BookRow]] = defaultdict(list)
        for row in rows:
            self._rows[(row.direction, row.asset_id)].append(row)
        for bucket in self._rows.values():
            bucket.sort(key=lambda row: row.occurred_at)
        self._times = {key: [row.occurred_at for row in bucket] for key, bucket in self._rows.items()}

    def probable(self, tool: _ToolRow, offset: timedelta) -> _BookRow | None:
        if tool.occurred_at is None:
            return None
        key = (tool.direction, tool.asset_id)
        bucket = self._rows.get(key)
        if not bucket:
            return None
        moment = tool.occurred_at - offset
        times = self._times[key]
        start = bisect.bisect_left(times, moment - _PROBABLE_WINDOW)
        end = bisect.bisect_right(times, moment + _PROBABLE_WINDOW)
        best = None
        for row in bucket[start:end]:
            if row.matched or _conflicting_ids(tool, row) or not _amount_fits(tool, row):
                continue
            distance = abs(row.occurred_at - moment)
            if best is None or distance < best[0]:
                best = (distance, row)
        return best[1] if best else None


def _type_difference(tool: _ToolRow, book: _BookRow) -> dict[str, Any] | None:
    tool_class = _TOOL_KIND_CLASS.get(tool.kind, "acquire" if tool.direction == "inbound" else "dispose")
    parts = [part for part in book.classification.split("+") if part in _KASSIBER_CLASS]
    if not parts:
        return None
    book_classes = {_KASSIBER_CLASS[part] for part in parts}
    if book.non_taxable:
        # Reviewed as not taxable: a receipt is a plain acquisition and an
        # outflow matches only a tool's non-taxable type.
        book_classes = {
            "acquire" if cls == "income" else "dispose_free" if cls == "dispose" else cls
            for cls in book_classes
        }
    # One tool row has one meaning. A row Kassiber books as part own-wallet
    # transfer and part disposal or income cannot agree with it, whichever
    # part the tool picked: the other part is missing there.
    mixed = "transfer" in book_classes and book_classes & {"dispose", "dispose_free", "income"}
    if tool_class in book_classes and not mixed:
        return None
    # A tool shows an inbound from an unknown sender as a deposit and a
    # fee-only spend as a withdrawal; neither changes the tax result.
    if tool_class == "transfer" and (book_classes == {"acquire"} or parts == ["fee"]):
        return None
    return {
        "field": "type",
        "severity": "tax",
        "tool": tool.label or (tool.kind or ""),
        "kassiber": book.classification,
        "message": f"{tool.label or tool.kind} in the tool, {book.classification} in Kassiber",
    }


def _differences(
    tool: _ToolRow,
    book: _BookRow,
    *,
    offset: timedelta,
    calendar: ZoneInfo,
    compare_types: bool,
) -> list[dict[str, Any]]:
    asset = describe_asset("LBTC" if book.asset_id == "liquid:lbtc" else "BTC")
    out: list[dict[str, Any]] = []
    if compare_types:
        type_diff = _type_difference(tool, book)
        if type_diff:
            out.append(type_diff)

    outbound = book.direction == "outbound"
    tool_movement = tool.amount_msat + (tool.fee_msat if outbound else 0)
    if outbound and not _same(tool_movement, book.movement_msat):
        if _same(tool.amount_msat, book.movement_msat) and not tool.fee_msat:
            out.append(
                {
                    "field": "fee",
                    "severity": "info",
                    "tool": asset.exact(tool.amount_msat),
                    "kassiber": asset.exact(book.fee_msat),
                    "message": "The tool folds the network fee into the amount",
                }
            )
        elif _same(tool.amount_msat, book.principal_msat):
            out.append(
                {
                    "field": "fee",
                    "severity": "tax" if "disposal" in book.classification else "info",
                    "tool": asset.exact(tool.fee_msat),
                    "kassiber": asset.exact(book.fee_msat),
                    "delta": asset.exact(tool.fee_msat - book.fee_msat),
                    "message": "Fees differ",
                }
            )
        else:
            out.append(
                {
                    "field": "amount",
                    "severity": "tax",
                    "tool": asset.exact(tool.amount_msat),
                    "kassiber": asset.exact(book.principal_msat),
                    "delta": asset.exact(tool_movement - book.movement_msat),
                    "message": "Amounts differ",
                }
            )
    elif not outbound and not _same(tool.amount_msat, book.principal_msat):
        out.append(
            {
                "field": "amount",
                "severity": "tax",
                "tool": asset.exact(tool.amount_msat),
                "kassiber": asset.exact(book.principal_msat),
                "delta": asset.exact(tool.amount_msat - book.principal_msat),
                "message": "Amounts differ",
            }
        )
    elif outbound and not _same(tool.amount_msat, book.principal_msat):
        out.append(
            {
                "field": "fee",
                "severity": "info",
                "tool": asset.exact(tool.fee_msat),
                "kassiber": asset.exact(book.fee_msat),
                "message": "Same total, but the fee is split differently",
            }
        )

    if tool.value is not None and book.value is not None:
        tolerance = max(_VALUE_TOLERANCE_MIN, abs(book.value) * _VALUE_TOLERANCE_SHARE)
        if abs(tool.value - book.value) > tolerance:
            out.append(
                {
                    "field": "value",
                    "severity": "tax",
                    "tool": _money(tool.value),
                    "kassiber": _money(book.value),
                    "delta": _money(tool.value - book.value),
                    "message": "Values differ",
                }
            )

    if tool.occurred_at is not None:
        shifted = tool.occurred_at - offset
        drift = shifted - book.occurred_at
        # A different tax year matters however small the drift: 22:59Z and
        # 23:01Z on 31 December are two years in an Austrian book.
        crosses_year = shifted.astimezone(calendar).year != book.occurred_at.astimezone(calendar).year
        if crosses_year or abs(drift) > _TIME_NOTICE:
            minutes = int(drift.total_seconds() // 60)
            out.append(
                {
                    "field": "time",
                    "severity": "tax" if crosses_year else "info",
                    "tool": _iso(tool.occurred_at),
                    "kassiber": _iso(book.occurred_at),
                    "delta": str(minutes),
                    "message": (
                        "Dates fall in different tax years" if crosses_year else f"Times differ by {minutes} min"
                    ),
                }
            )
    return out


def _tool_payload(row: _ToolRow) -> dict[str, Any]:
    asset = describe_asset("LBTC" if row.asset_id == "liquid:lbtc" else "BTC")
    return {
        "row": row.index,
        "source": row.source,
        "type": row.label,
        "occurred_at": _iso(row.occurred_at),
        "direction": row.direction,
        "asset": asset.display_code if row.direction else "",
        "amount": asset.exact(row.amount_msat) if row.direction else "",
        "fee": asset.exact(row.fee_msat) if row.fee_msat else "",
        "value": _money(row.value),
        "txid": row.txid,
    }


def _book_payload(row: _BookRow) -> dict[str, Any]:
    asset = describe_asset("LBTC" if row.asset_id == "liquid:lbtc" else "BTC")
    return {
        "ref": row.id,
        "wallet": row.wallet,
        "classification": row.classification,
        "occurred_at": _iso(row.occurred_at),
        "direction": row.direction,
        "asset": asset.display_code,
        "amount": asset.exact(row.principal_msat),
        "fee": asset.exact(row.fee_msat) if row.fee_msat else "",
        "value": _money(row.value),
        "txid": row.txid,
        "excluded": row.excluded,
    }


def _wallet_pairs(
    rows: list[dict[str, Any]],
    source_wallets: Mapping[str, Counter[str]],
) -> list[dict[str, Any]]:
    """Kassiber wallets next to the tool's names for them, and each row's pair.

    A tool names its wallets in its own column (CoinTracking's Exchange,
    Blockpit's Integration Name). The name whose rows matched a Kassiber
    wallet most often is that wallet's counterpart, so a row only the tool has
    still lands next to the right wallet. A wallet only one side knows is one
    finding about the wallet (``kassiber_only`` / ``tool_only``), not a list of
    row problems. Sets ``row["pair"]`` on every row.
    """
    counterpart = {
        source: wallets.most_common(1)[0][0] for source, wallets in source_wallets.items() if wallets
    }
    groups: dict[str, dict[str, Any]] = {}
    for row in rows:
        book = row["kassiber"]
        tool = row["tool"]
        wallet = book["wallet"] if book else counterpart.get(tool["source"]) if tool else None
        key = f"wallet:{wallet}" if wallet else f"tool:{tool['source'] if tool else ''}"
        group = groups.setdefault(
            key,
            {"key": key, "wallet": wallet, "tool_names": set(), "counts": Counter(), "rows": 0},
        )
        if tool:
            group["tool_names"].add(tool["source"])
        group["counts"][row["status"]] += 1
        group["rows"] += 1
        row["pair"] = key
    for source, wallet in counterpart.items():
        if f"wallet:{wallet}" in groups:
            groups[f"wallet:{wallet}"]["tool_names"].add(source)
    out = []
    for group in groups.values():
        counts = group["counts"]
        both_sides = counts["matched"] + counts["different"] + counts["excluded_in_kassiber"]
        if both_sides or (group["wallet"] and group["tool_names"]):
            state = "paired"
        elif group["wallet"]:
            state = "kassiber_only"
        else:
            state = "tool_only"
        issues = sum(count for status, count in counts.items() if status != "matched")
        out.append(
            {
                "key": group["key"],
                "wallet": group["wallet"],
                "tool_names": sorted(group["tool_names"]),
                "state": state,
                "rows": group["rows"],
                "issues": issues if state == "paired" else 0,
                "counts": {status: counts.get(status, 0) for status in STATUSES},
            }
        )
    order = {"paired": 0, "kassiber_only": 1, "tool_only": 2}
    out.sort(key=lambda pair: (order[pair["state"]], -pair["issues"], str(pair["wallet"] or pair["tool_names"])))
    return out


def _resolve_tool(tool: str) -> tuple[str, str]:
    key = str(tool or "").strip().lower()
    if key not in TAX_PLATFORM_READERS:
        raise AppError(
            f"Unknown tool '{tool}'",
            code="validation",
            hint="Use one of: " + ", ".join(sorted(TAX_PLATFORM_READERS)) + ".",
            retryable=False,
        )
    return key, TAX_PLATFORM_READERS[key][0]


def _empty_totals() -> dict[str, Any]:
    return {"inbound": 0, "outbound": 0, "fees": 0, "income_value": Decimal("0"), "income_unvalued": 0}


def cross_check(
    conn,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
    *,
    tool: str,
    file_path,
    wallet_ref=None,
    wallet_refs=None,
    year: int | None = None,
    tool_sources: Iterable[str] | None = None,
    include_tool_wallets: bool = False,
) -> dict[str, Any]:
    tool_id, tool_label = _resolve_tool(tool)
    path = Path(str(file_path or "")).expanduser()
    if not str(file_path or "").strip() or not path.is_file():
        raise AppError(
            f"{tool_label} export not found",
            code="validation",
            hint=f"Choose the transaction CSV you exported from {tool_label}.",
            retryable=False,
        )
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    profile = conn.execute("SELECT * FROM profiles WHERE id = ?", (profile["id"],)).fetchone()
    book_currency = str(_row_get(profile, "fiat_currency") or "").upper()
    if year is not None:
        try:
            year = int(year)
        except (TypeError, ValueError):
            year = 0
        if not 2009 <= year <= 2100:
            raise AppError("Year must be a calendar year such as 2025", code="validation", retryable=False)
    period = _period_for_profile(profile, year) if year is not None else None
    calendar = _period_for_profile(profile, 2000).tz

    explicit = wallet_refs is not None or bool(wallet_ref)
    wallets = resolve_export_wallets(conn, profile, hooks, wallet_ref, wallet_refs)
    skipped_wallets: list[str] = []
    if wallets is None:
        all_wallets = hooks.list_wallets(conn, workspace["id"], profile["id"])
        if not include_tool_wallets and not explicit:
            skipped_wallets = [w["label"] for w in all_wallets if str(w.get("kind") or "").lower() == tool_id]
        wallet_ids = (
            {w["id"] for w in all_wallets if w["label"] not in skipped_wallets} if skipped_wallets else None
        )
    else:
        wallet_ids = {wallet["id"] for wallet in wallets}

    tool_rows, tool_other_assets = _tool_rows(tool_id, path, book_currency)
    book_rows, book_other_assets = _book_rows(conn, profile, wallet_ids, book_currency)
    freshness = custody_journal.projection_freshness(conn, profile)
    compare_types = bool(freshness.get("is_current"))

    def in_period(moment: datetime | None) -> bool:
        if period is None or moment is None:
            return True
        return moment.astimezone(period.tz).year == period.year

    # Sources are listed for the whole period so a filter never hides itself.
    source_counts: Counter[str] = Counter(row.source for row in tool_rows if in_period(row.occurred_at))
    wanted_sources = {str(name) for name in tool_sources} if tool_sources is not None else None
    if wanted_sources is not None:
        tool_rows = [row for row in tool_rows if row.source in wanted_sources]

    comparable = [row for row in tool_rows if row.comparable]
    active = [row for row in book_rows if not row.excluded]
    excluded = [row for row in book_rows if row.excluded]

    # 1. Transaction id (or the Kassiber row id Kassiber's own exports write).
    by_id: dict[tuple[str, str], list[_BookRow]] = defaultdict(list)
    for row in active:
        for key in {row.txid.lower(), row.id.lower()} - {""}:
            by_id[(key, row.direction)].append(row)
    pairs: list[tuple[_ToolRow, _BookRow, str]] = []
    unmatched_tool: list[_ToolRow] = []
    for tool_row in comparable:
        candidates = [
            row
            for row in by_id.get((tool_row.txid.lower(), tool_row.direction), [])
            if not row.matched and row.asset_id == tool_row.asset_id
        ] if tool_row.txid else []
        if candidates:
            best = min(candidates, key=lambda row: abs(row.principal_msat - tool_row.amount_msat))
            best.matched = True
            pairs.append((tool_row, best, "txid"))
        else:
            unmatched_tool.append(tool_row)
    offset = _detect_offset((tool_row, book) for tool_row, book, _ in pairs)

    # 2. The same id on a transaction the user excluded. An exact id beats any
    # amount match, so an excluded row is never hidden behind a look-alike.
    excluded_by_id: dict[tuple[str, str], _BookRow] = {}
    for row in excluded:
        for key in {row.txid.lower(), row.id.lower()} - {""}:
            excluded_by_id.setdefault((key, row.direction), row)
    excluded_pairs: list[tuple[_ToolRow, _BookRow, str]] = []
    without_id_match: list[_ToolRow] = []
    for tool_row in unmatched_tool:
        book = excluded_by_id.get((tool_row.txid.lower(), tool_row.direction)) if tool_row.txid else None
        if book is not None and not book.matched and book.asset_id == tool_row.asset_id:
            book.matched = True
            excluded_pairs.append((tool_row, book, "txid"))
        else:
            without_id_match.append(tool_row)

    # 3. Same amount within a day, closest in time first: active rows, then
    # excluded ones. Two different on-chain ids are never paired.
    index = _Index(row for row in active if not row.matched)
    excluded_index = _Index(row for row in excluded if not row.matched)
    only_tool: list[_ToolRow] = []
    for tool_row in sorted(
        without_id_match, key=lambda row: row.occurred_at or datetime.min.replace(tzinfo=timezone.utc)
    ):
        book = index.probable(tool_row, offset)
        if book is not None:
            book.matched = True
            pairs.append((tool_row, book, "probable"))
            continue
        book = excluded_index.probable(tool_row, offset)
        if book is not None:
            book.matched = True
            excluded_pairs.append((tool_row, book, "probable"))
        else:
            only_tool.append(tool_row)

    rows: list[dict[str, Any]] = []
    totals = {"tool": defaultdict(_empty_totals), "kassiber": defaultdict(_empty_totals)}
    source_wallets: dict[str, Counter[str]] = defaultdict(Counter)
    source_matched: Counter[str] = Counter()

    def add_tool_total(row: _ToolRow) -> None:
        bucket = totals["tool"][row.asset_id]
        if row.direction == "inbound":
            bucket["inbound"] += row.amount_msat
            if _TOOL_KIND_CLASS.get(row.kind) == "income":
                if row.value is None:
                    bucket["income_unvalued"] += 1
                else:
                    bucket["income_value"] += row.value
        else:
            bucket["outbound"] += row.amount_msat
            bucket["fees"] += row.fee_msat

    def add_book_total(row: _BookRow) -> None:
        bucket = totals["kassiber"][row.asset_id]
        if row.direction == "inbound":
            bucket["inbound"] += row.principal_msat
            if "income" in row.classification.split("+") and row.value is not None and not row.non_taxable:
                bucket["income_value"] += row.value
        else:
            bucket["outbound"] += row.principal_msat
            bucket["fees"] += row.fee_msat

    for tool_row, book, match in pairs:
        if not (in_period(tool_row.occurred_at - offset) or in_period(book.occurred_at)):
            continue
        diffs = _differences(tool_row, book, offset=offset, calendar=calendar, compare_types=compare_types)
        status = "different" if any(diff["severity"] == "tax" for diff in diffs) else "matched"
        rows.append(
            {
                "status": status,
                "match": match,
                "tool": _tool_payload(tool_row),
                "kassiber": _book_payload(book),
                "differences": diffs,
            }
        )
        if in_period(tool_row.occurred_at - offset):
            add_tool_total(tool_row)
        if in_period(book.occurred_at):
            add_book_total(book)
        source_wallets[tool_row.source][book.wallet] += 1
        source_matched[tool_row.source] += 1
    for tool_row, book, match in excluded_pairs:
        if not (in_period(tool_row.occurred_at - offset) or in_period(book.occurred_at)):
            continue
        rows.append(
            {
                "status": "excluded_in_kassiber",
                "match": match,
                "tool": _tool_payload(tool_row),
                "kassiber": _book_payload(book),
                "differences": [],
            }
        )
        if in_period(tool_row.occurred_at - offset):
            add_tool_total(tool_row)
    for tool_row in only_tool:
        if not in_period(tool_row.occurred_at - offset):
            continue
        rows.append({"status": "only_tool", "match": None, "tool": _tool_payload(tool_row), "kassiber": None, "differences": []})
        add_tool_total(tool_row)
    for book in active:
        if book.matched or not in_period(book.occurred_at):
            continue
        rows.append({"status": "only_kassiber", "match": None, "tool": None, "kassiber": _book_payload(book), "differences": []})
        add_book_total(book)
    for tool_row in tool_rows:
        if tool_row.comparable or not in_period(tool_row.occurred_at):
            continue
        payload = _tool_payload(tool_row)
        rows.append(
            {
                "status": "not_comparable",
                "match": None,
                "tool": payload,
                "kassiber": None,
                "differences": [],
                "reason": tool_row.error,
            }
        )

    def moment(row: Mapping[str, Any]) -> str:
        side = row["kassiber"] or row["tool"] or {}
        return str(side.get("occurred_at") or "")

    status_order = {status: position for position, status in enumerate(STATUSES)}
    rows.sort(key=lambda row: (moment(row), status_order[row["status"]]))
    for position, row in enumerate(rows, start=1):
        row["id"] = f"r{position}"

    counts = {status: 0 for status in STATUSES}
    for row in rows:
        counts[row["status"]] += 1
    tax_differences = sum(
        1 for row in rows for diff in row["differences"] if diff["severity"] == "tax"
    )

    total_rows = []
    for asset_id in sorted(set(totals["tool"]) | set(totals["kassiber"])):
        asset = describe_asset("LBTC" if asset_id == "liquid:lbtc" else "BTC")
        tool_bucket = totals["tool"].get(asset_id) or _empty_totals()
        book_bucket = totals["kassiber"].get(asset_id) or _empty_totals()
        for metric in ("inbound", "outbound", "fees"):
            total_rows.append(
                {
                    "asset": asset.display_code,
                    "metric": metric,
                    "tool": asset.exact(tool_bucket[metric]),
                    "kassiber": asset.exact(book_bucket[metric]),
                    "delta": asset.exact(tool_bucket[metric] - book_bucket[metric]),
                    "agrees": _same(tool_bucket[metric], book_bucket[metric]),
                }
            )
        if tool_bucket["income_value"] or book_bucket["income_value"]:
            # A tool export without value columns (Blockpit's template) has
            # no income value to compare; unknown is not zero.
            known = not tool_bucket["income_unvalued"]
            delta = tool_bucket["income_value"] - book_bucket["income_value"]
            total_rows.append(
                {
                    "asset": asset.display_code,
                    "metric": "income_value",
                    "tool": _money(tool_bucket["income_value"]) if known else "",
                    "kassiber": _money(book_bucket["income_value"]),
                    "delta": _money(delta) if known else "",
                    "agrees": abs(delta) <= _VALUE_TOLERANCE_MIN if known else None,
                }
            )

    insights: list[dict[str, Any]] = []
    if offset:
        hours = int(offset.total_seconds() // 3600)
        insights.append(
            {
                "code": "clock_offset",
                "params": {"hours": hours},
                "message": f"{tool_label} times are {hours:+d} h from Kassiber's (UTC); matching allowed for it.",
            }
        )
    if not compare_types:
        insights.append(
            {
                "code": "journals_stale",
                "params": {},
                "message": "Journals are not current, so transaction types were not compared. Process journals and check again.",
            }
        )
    if skipped_wallets:
        insights.append(
            {
                "code": "skipped_tool_wallets",
                "params": {"count": len(skipped_wallets), "wallets": skipped_wallets},
                "message": (
                    f"Skipped {len(skipped_wallets)} {'wallet' if len(skipped_wallets) == 1 else 'wallets'} "
                    f"imported from {tool_label} "
                    f"({', '.join(skipped_wallets)}); comparing them would check {tool_label} against itself."
                ),
            }
        )
    if tool_other_assets:
        insights.append(
            {
                "code": "tool_other_assets",
                "params": {"count": tool_other_assets},
                "message": f"{tool_label}: {_rows(tool_other_assets)} without a Bitcoin leg not compared.",
            }
        )
    if book_other_assets:
        assets = ", ".join(sorted(book_other_assets))
        insights.append(
            {
                "code": "kassiber_other_assets",
                "params": {"count": sum(book_other_assets.values()), "assets": assets},
                "message": f"Kassiber: {_rows(sum(book_other_assets.values()))} in {assets} not compared.",
            }
        )
    probable = sum(1 for row in rows if row["match"] == "probable")
    if probable:
        insights.append(
            {
                "code": "probable_matches",
                "params": {"count": probable},
                "message": f"{_rows(probable)} without a shared transaction id matched by amount and time.",
            }
        )

    sources = [
        {"name": name, "rows": count, "matched": source_matched.get(name, 0)}
        for name, count in sorted(source_counts.items(), key=lambda item: (-item[1], item[0]))
    ]
    pairs = _wallet_pairs(rows, source_wallets)

    return {
        "tool": tool_id,
        "tool_label": tool_label,
        "file_name": path.name,
        "book_currency": book_currency,
        "year": period.year if period else None,
        "calendar": period.calendar if period else "",
        # The book's years in its own calendar, for choosing another one.
        "years": accounting_years(conn, profile),
        "wallet": export_wallet_scope_label(wallets),
        "agrees": counts["matched"] == len(rows) and bool(rows),
        "counts": counts,
        "tax_differences": tax_differences,
        "totals": total_rows,
        "sources": sources,
        "pairs": pairs,
        "insights": insights,
        "rows": rows,
        "_profile": profile,
    }


# --- Workbook ----------------------------------------------------------------

_SHEETS = (
    ("Differences", ("different",)),
    ("Only in tool", ("only_tool",)),
    ("Only in Kassiber", ("only_kassiber",)),
    ("Excluded in Kassiber", ("excluded_in_kassiber",)),
    ("Matched", ("matched",)),
    ("Not comparable", ("not_comparable",)),
)
_ROW_COLUMNS = (
    ("status", "Status", 16),
    ("match", "Match", 10),
    ("differences", "Differences", 40),
    ("tool_row", "{tool} row", 8),
    ("tool_source", "{tool} source", 18),
    ("tool_type", "{tool} type", 14),
    ("tool_date", "{tool} date (UTC)", 20),
    ("direction", "Direction", 10),
    ("asset", "Asset", 7),
    ("tool_amount", "{tool} amount", 16),
    ("tool_fee", "{tool} fee", 14),
    ("tool_value", "{tool} value ({fiat})", 14),
    ("wallet", "Kassiber wallet", 18),
    ("classification", "Kassiber type", 18),
    ("book_date", "Kassiber date (UTC)", 20),
    ("book_amount", "Kassiber amount", 16),
    ("book_fee", "Kassiber fee", 14),
    ("book_value", "Kassiber value ({fiat})", 14),
    ("txid", "Transaction id", 30),
    ("kassiber_ref", "Kassiber ref", 14),
)


def _sheet_row(row: Mapping[str, Any]) -> dict[str, Any]:
    tool = row["tool"] or {}
    book = row["kassiber"] or {}
    differences = "; ".join(diff["message"] for diff in row["differences"])
    if row.get("reason"):
        differences = row["reason"]
    return {
        "status": row["status"].replace("_", " "),
        "match": row["match"] or "",
        "differences": differences,
        "tool_row": tool.get("row", ""),
        "tool_source": tool.get("source", ""),
        "tool_type": tool.get("type", ""),
        "tool_date": tool.get("occurred_at", ""),
        "direction": book.get("direction") or tool.get("direction", ""),
        "asset": book.get("asset") or tool.get("asset", ""),
        "tool_amount": tool.get("amount", ""),
        "tool_fee": tool.get("fee", ""),
        "tool_value": tool.get("value", ""),
        "wallet": book.get("wallet", ""),
        "classification": book.get("classification", ""),
        "book_date": book.get("occurred_at", ""),
        "book_amount": book.get("amount", ""),
        "book_fee": book.get("fee", ""),
        "book_value": book.get("value", ""),
        "txid": book.get("txid") or tool.get("txid", ""),
        "kassiber_ref": str(book.get("ref", ""))[:12],
    }


def export_cross_check_xlsx(
    conn,
    workspace_ref,
    profile_ref,
    output_path,
    hooks: ReportHooks,
    **options: Any,
) -> dict[str, Any]:
    """Write the comparison to XLSX; ``options`` are ``cross_check``'s."""
    import xlsxwriter

    result = cross_check(conn, workspace_ref, profile_ref, hooks, **options)
    result.pop("_profile", None)
    fiat = result["book_currency"]
    tool_label = result["tool_label"]
    path = Path(output_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    workbook = xlsxwriter.Workbook(str(path))
    workbook.set_properties(
        {
            "title": f"Kassiber vs {tool_label}",
            "subject": "Cross-check of a tool export against the Kassiber book",
            "author": "Kassiber",
        }
    )
    formats = _generic_report_xlsx_formats(workbook)
    quantity = workbook.add_format({**XLSX_BODY, "num_format": "#,##0.00000000###"})
    money = workbook.add_format({**XLSX_BODY, "num_format": XLSX_MONEY_FORMAT})

    summary = workbook.add_worksheet("Summary")
    summary.set_column(0, 0, 28)
    summary.set_column(1, 4, 20)
    summary.write_string(0, 0, f"Kassiber vs {tool_label}", formats["title"])
    facts = [
        ("File", result["file_name"]),
        ("Year", str(result["year"] or "All years")),
        ("Calendar", result["calendar"] or "UTC"),
        ("Wallets", result["wallet"] or "All wallets"),
        ("Tax-relevant differences", result["tax_differences"]),
    ]
    facts.extend((status.replace("_", " ").capitalize(), count) for status, count in result["counts"].items())
    line = 2
    for label, value in facts:
        summary.write_string(line, 0, label, formats["label"])
        _xlsx_write_value(summary, line, 1, value, formats["text"])
        line += 1
    line += 1
    for col, label in enumerate(("Asset", "Total", tool_label, "Kassiber", "Difference")):
        summary.write_string(line, col, label, formats["header"])
    for total in result["totals"]:
        line += 1
        number = money if total["metric"] == "income_value" else quantity
        metric = {"income_value": f"Income value ({fiat})"}.get(total["metric"], total["metric"].capitalize())
        summary.write_string(line, 0, total["asset"], formats["text"])
        summary.write_string(line, 1, metric, formats["text"])
        for col, key in ((2, "tool"), (3, "kassiber"), (4, "delta")):
            if total[key] == "":
                summary.write_string(line, col, "not in file", formats["text"])
            else:
                summary.write_number(line, col, float(Decimal(total[key])), number)
    if result["pairs"]:
        line += 2
        for col, label in enumerate(("Kassiber wallet", f"{tool_label} name", "State", "Rows", "To review")):
            summary.write_string(line, col, label, formats["header"])
        for pair in result["pairs"]:
            line += 1
            summary.write_string(line, 0, pair["wallet"] or "", formats["text"])
            summary.write_string(line, 1, ", ".join(pair["tool_names"]), formats["text"])
            summary.write_string(line, 2, pair["state"].replace("_", " "), formats["text"])
            summary.write_number(line, 3, pair["rows"], formats["text"])
            summary.write_number(line, 4, pair["issues"], formats["text"])
    if result["insights"]:
        line += 2
        summary.write_string(line, 0, "Notes", formats["header"])
        for insight in result["insights"]:
            line += 1
            summary.write_string(line, 0, insight["message"], formats["text"])
    _xlsx_print_setup(summary)

    numeric = {"tool_amount", "tool_fee", "book_amount", "book_fee"}
    monetary = {"tool_value", "book_value"}
    for name, statuses in _SHEETS:
        sheet_rows = [_sheet_row(row) for row in result["rows"] if row["status"] in statuses]
        worksheet = workbook.add_worksheet(name)
        worksheet.freeze_panes(1, 0)
        for col, (key, label, width) in enumerate(_ROW_COLUMNS):
            worksheet.set_column(col, col, width)
            worksheet.write_string(0, col, label.format(tool=tool_label, fiat=fiat), formats["header"])
        for line, sheet_row in enumerate(sheet_rows, start=1):
            for col, (key, _, _) in enumerate(_ROW_COLUMNS):
                value = sheet_row[key]
                if key in numeric and value != "":
                    worksheet.write_number(line, col, float(Decimal(value)), quantity)
                elif key in monetary and value != "":
                    worksheet.write_number(line, col, float(Decimal(value)), money)
                else:
                    _xlsx_write_value(worksheet, line, col, value, formats["text"])
        worksheet.autofilter(0, 0, max(len(sheet_rows), 1), len(_ROW_COLUMNS) - 1)
        if sheet_rows:
            worksheet.conditional_format(
                1, 0, len(sheet_rows), len(_ROW_COLUMNS) - 1,
                {"type": "formula", "criteria": "=MOD(ROW(),2)=1", "format": formats["band"]},
            )
        _xlsx_print_setup(worksheet, header_row=0)
    workbook.close()
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "tool": result["tool"],
        "year": result["year"],
        "counts": result["counts"],
        "tax_differences": result["tax_differences"],
        "agrees": result["agrees"],
    }


def public_cross_check(result: Mapping[str, Any]) -> dict[str, Any]:
    """The comparison without internal fields, for CLI and daemon output."""
    return {key: value for key, value in result.items() if not key.startswith("_")}


__all__ = [
    "COMPARED_ASSETS",
    "STATUSES",
    "cross_check",
    "export_cross_check_xlsx",
    "public_cross_check",
]
