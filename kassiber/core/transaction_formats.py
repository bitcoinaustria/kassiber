"""Transaction exports in other tools' import formats.

Each format is one ``PlatformFormat`` in ``PLATFORM_FORMATS``. A format only
maps a neutral ``PlatformRow`` (what Kassiber booked, in plain terms) onto its
own columns and vocabulary; it never reads the database or decides meaning.
Adding a tool means adding one entry here.

Meaning comes from the processed journal through ``transaction_classification``,
the same source as the year handoff. A conversion booked as taxable is
exported as a sale and a purchase at the booked proceeds and cost, which gives
the target tool the same gain. A swap that carries basis between own wallets
has no import row in these tools; it is exported as plain transfer legs and
counted in ``warnings`` so the user links it there. A row that is part
own-wallet transfer and part taxable event (a payment that also moves coins to
another own wallet) cannot be written as one row of either format, so the
export refuses instead of dropping the taxable part.

The export fails closed like the reports: stale journals, failed sources, and
quarantined transactions in scope block it.
"""

from __future__ import annotations

import csv
from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any, Callable

from ..errors import AppError
from ..msat import dec
from . import freshness as core_freshness
from .asset_presentation import AssetPresentation, describe_asset
from .custody_evidence import normalize_boundary_amounts
from .reports import (
    ReportHooks,
    _generic_report_transaction_rows,
    _optional_decimal_pref,
    _report_query_rows,
    _require_report_context,
    export_wallet_scope_label,
    resolve_export_wallets,
)
from .transaction_classification import (
    classify,
    journal_entry_types,
    pair_links,
    require_no_quarantines,
    taxable_conversion_ids,
)

# Neutral events a format maps to its own vocabulary.
EVENTS = (
    "buy",
    "sell",
    "income",
    "mining",
    "staking",
    "airdrop",
    "interest",
    "payment",
    "fee",
    "deposit",
    "withdrawal",
)
_INCOME_KIND_EVENTS = {
    "mining": "mining",
    "mining_reward": "mining",
    "staking": "staking",
    "airdrop": "airdrop",
    "interest": "interest",
    "lending_interest": "interest",
}


@dataclass(frozen=True)
class PlatformRow:
    occurred_at: datetime
    wallet: str
    transaction_id: str
    event: str
    classification: str
    asset: AssetPresentation
    asset_code: str
    amount: str
    fee: str
    fiat_currency: str
    fiat_amount: str
    value: str
    comment: str


@dataclass(frozen=True)
class PlatformFormat:
    id: str
    label: str
    filename_stem: str
    headers: tuple[str, ...]
    build_row: Callable[[PlatformRow], dict[str, str]]


def _number(value: Decimal | None) -> str:
    if value is None:
        return ""
    text = format(value.normalize(), "f")
    return "0" if text in {"-0", ""} else text


def _platform_asset_code(asset: AssetPresentation) -> tuple[str, bool]:
    """Ticker other tools understand, and whether it is a real ticker."""
    if asset.asset_id == "bitcoin:btc":
        return "BTC", True
    if asset.asset_id == "liquid:lbtc":
        return "LBTC", True
    if asset.chain == "liquid":
        # Issued Liquid assets have no ticker in Kassiber; the full asset id
        # is exact, and the export warns that the tool needs a mapping.
        return asset.asset_id.split(":", 1)[1], False
    return asset.display_code, True


# --- CoinTracking ------------------------------------------------------------
# Custom CSV import: https://cointracking.info/import/import_csv
_COINTRACKING_TYPES = {
    "buy": "Trade",
    "sell": "Trade",
    "income": "Income",
    "mining": "Mining",
    "staking": "Staking",
    "airdrop": "Airdrop",
    "interest": "Interest Income",
    "payment": "Spend",
    "fee": "Other Fee",
    "deposit": "Deposit",
    "withdrawal": "Withdrawal",
}
_INBOUND_EVENTS = {"buy", "income", "mining", "staking", "airdrop", "interest", "deposit"}


def _cointracking_row(row: PlatformRow) -> dict[str, str]:
    inbound = row.event in _INBOUND_EVENTS
    output = {
        "Type": _COINTRACKING_TYPES[row.event],
        "Buy Amount": row.amount if inbound else "",
        "Buy Currency": row.asset_code if inbound else "",
        "Sell Amount": "" if inbound else row.amount,
        "Sell Currency": "" if inbound else row.asset_code,
        "Fee": row.fee,
        "Fee Currency": row.asset_code if row.fee else "",
        "Exchange": row.wallet,
        "Trade-Group": "Kassiber",
        "Comment": row.comment,
        "Date": row.occurred_at.strftime("%Y-%m-%d %H:%M:%S"),
        "Tx-ID": row.transaction_id,
        "Buy Value in Account Currency": row.value if inbound else "",
        "Sell Value in Account Currency": "" if inbound else row.value,
    }
    if row.event == "buy":
        output["Sell Amount"], output["Sell Currency"] = row.fiat_amount, row.fiat_currency
    elif row.event == "sell":
        output["Buy Amount"], output["Buy Currency"] = row.fiat_amount, row.fiat_currency
    return output


COINTRACKING = PlatformFormat(
    id="cointracking",
    label="CoinTracking CSV",
    filename_stem="kassiber-cointracking",
    headers=(
        "Type",
        "Buy Amount",
        "Buy Currency",
        "Sell Amount",
        "Sell Currency",
        "Fee",
        "Fee Currency",
        "Exchange",
        "Trade-Group",
        "Comment",
        "Date",
        "Tx-ID",
        "Buy Value in Account Currency",
        "Sell Value in Account Currency",
    ),
    build_row=_cointracking_row,
)


# --- Blockpit ----------------------------------------------------------------
# "Blockpit Excel" manual import template (Integrations -> Blockpit Excel).
_BLOCKPIT_LABELS = {
    "buy": "Trade",
    "sell": "Trade",
    "income": "Income",
    "mining": "Mining",
    "staking": "Staking",
    "airdrop": "Airdrop",
    "interest": "Interest",
    "payment": "Payment",
    "fee": "Fee",
    "deposit": "Deposit",
    "withdrawal": "Withdrawal",
}


def _blockpit_row(row: PlatformRow) -> dict[str, str]:
    inbound = row.event in _INBOUND_EVENTS
    output = {
        "Date (UTC)": row.occurred_at.strftime("%d.%m.%Y %H:%M:%S"),
        "Integration Name": row.wallet,
        "Label": _BLOCKPIT_LABELS[row.event],
        "Outgoing Asset": "" if inbound else row.asset_code,
        "Outgoing Amount": "" if inbound else row.amount,
        "Incoming Asset": row.asset_code if inbound else "",
        "Incoming Amount": row.amount if inbound else "",
        "Fee Asset (optional)": row.asset_code if row.fee else "",
        "Fee Amount (optional)": row.fee,
        "Comment (optional)": row.comment,
        "Trx. ID (optional)": row.transaction_id,
    }
    if row.event == "buy":
        output["Outgoing Asset"], output["Outgoing Amount"] = row.fiat_currency, row.fiat_amount
    elif row.event == "sell":
        output["Incoming Asset"], output["Incoming Amount"] = row.fiat_currency, row.fiat_amount
    return output


BLOCKPIT = PlatformFormat(
    id="blockpit",
    label="Blockpit CSV",
    filename_stem="kassiber-blockpit",
    headers=(
        "Date (UTC)",
        "Integration Name",
        "Label",
        "Outgoing Asset",
        "Outgoing Amount",
        "Incoming Asset",
        "Incoming Amount",
        "Fee Asset (optional)",
        "Fee Amount (optional)",
        "Comment (optional)",
        "Trx. ID (optional)",
    ),
    build_row=_blockpit_row,
)

PLATFORM_FORMATS: dict[str, PlatformFormat] = {
    platform.id: platform for platform in (COINTRACKING, BLOCKPIT)
}


def platform_format(platform_id: str) -> PlatformFormat:
    platform = PLATFORM_FORMATS.get(str(platform_id or "").strip().lower())
    if platform is None:
        raise AppError(
            f"Unknown export format '{platform_id}'",
            code="validation",
            hint="Use one of: " + ", ".join(sorted(PLATFORM_FORMATS)) + ".",
            retryable=False,
        )
    return platform


def _parse_timestamp(value: str) -> datetime:
    parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _journal_values(conn, profile_id) -> dict[str, dict[str, Decimal]]:
    values: dict[str, dict[str, Decimal]] = {}
    for row in conn.execute(
        """
        SELECT transaction_id, entry_type,
               COALESCE(fiat_value_exact, fiat_value) AS fiat_value,
               COALESCE(proceeds_exact, proceeds) AS proceeds
        FROM journal_entries
        WHERE profile_id = ? AND entry_type IN ('income', 'disposal', 'acquisition')
        """,
        (profile_id,),
    ).fetchall():
        bucket = values.setdefault(row["transaction_id"], {})
        key = {"income": "income", "disposal": "proceeds", "acquisition": "cost"}[row["entry_type"]]
        amount = dec(row["proceeds"] if key == "proceeds" else row["fiat_value"])
        bucket[key] = bucket.get(key, Decimal("0")) + amount
    return values


_TAXABLE_PARTS = frozenset({"income", "acquisition", "disposal"})


def _event(
    classification: str,
    direction: str,
    kind: str,
    has_fiat_leg: bool,
    taxable_conversion: bool = False,
) -> tuple[str | None, str | None]:
    """Neutral event for one row, plus a warning key when it is approximated.

    ``None`` as the event means the row cannot be written as one row of a
    tool's import format without losing a taxable part.
    """
    inbound = direction == "inbound"
    fallback = "deposit" if inbound else "withdrawal"
    if "+" in classification:
        if _TAXABLE_PARTS & set(classification.split("+")):
            return None, "unsupported"
        return fallback, "swap"
    if taxable_conversion and classification == "disposal":
        return "sell", "conversion"
    if taxable_conversion and classification == "acquisition":
        return "buy", "conversion"
    if classification == "internal_transfer":
        return fallback, None
    if classification == "swap":
        return fallback, "swap"
    if classification == "income":
        return _INCOME_KIND_EVENTS.get(kind, "income"), None
    if classification == "acquisition":
        if kind == "buy" and has_fiat_leg:
            return "buy", None
        return "deposit", None
    if classification == "disposal":
        if kind == "sell" and has_fiat_leg:
            return "sell", None
        return "payment", None
    if classification == "fee":
        return "fee", None
    return fallback, "unbooked"


_WARNING_TEXT = {
    "swap": "{count} swap legs between own wallets are exported as withdrawal/deposit; "
    "link them as a conversion in the target tool.",
    "conversion": "{count} legs of taxable conversions are exported as a sale and a purchase "
    "at the booked proceeds and cost.",
    "unbooked": "{count} rows have no journal entry and are exported as plain deposit/withdrawal.",
    "ticker": "Assets without a ticker are exported by asset id ({assets}); map them in the target tool.",
    "value": "Value columns are in {currency}; they assume the target tool uses the same "
    "account currency.",
}


def build_platform_rows(
    conn,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
    *,
    wallet_ref=None,
    wallet_refs=None,
) -> dict[str, Any]:
    report_context = _require_report_context(conn, workspace_ref, profile_ref, hooks)
    profile = report_context.profile
    core_freshness.require_report_freshness(conn, profile["id"])
    wallets = resolve_export_wallets(conn, profile, hooks, wallet_ref, wallet_refs)
    wallet_ids = {wallet["id"] for wallet in wallets} if wallets else None
    wallet_labels = {wallet["label"] for wallet in wallets} if wallets else None
    require_no_quarantines(
        conn,
        profile["id"],
        in_scope=lambda row: wallet_ids is None or row["wallet_id"] in wallet_ids,
        code="transaction_export_quarantined",
        message="This export is blocked by quarantined transactions.",
        hint=(
            "Resolve or exclude every quarantined transaction in this scope, then process "
            "journals again. The target tool would otherwise receive a guessed event."
        ),
        details={"wallet": export_wallet_scope_label(wallets)},
    )

    query_rows = _report_query_rows(conn, profile, wallet=wallets[0] if wallets and len(wallets) == 1 else None)
    export_rows = _generic_report_transaction_rows({"query_rows": query_rows})
    tx_meta = {
        row["id"]: row
        for row in conn.execute(
            """
            SELECT id, COALESCE(NULLIF(kind_override, ''), kind, '') AS kind,
                   amount_includes_fee, COALESCE(external_id, '') AS external_id
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            """,
            (profile["id"],),
        ).fetchall()
    }
    external_ids = {tx_id: row["external_id"] for tx_id, row in tx_meta.items()}
    links, swap_ids = pair_links(conn, profile["id"], external_ids)
    taxable_conversions = taxable_conversion_ids(conn, profile["id"])
    entry_types = journal_entry_types(conn, profile["id"])
    journal_values = _journal_values(conn, profile["id"])
    book_currency = str(profile["fiat_currency"] or "").upper()

    rows: list[PlatformRow] = []
    unsupported: list[dict[str, Any]] = []
    warning_counts: dict[str, int] = {}
    tickerless: set[str] = set()
    for source, export_row in zip(query_rows["transactions"], export_rows):
        if wallet_labels is not None and source["wallet"] not in wallet_labels:
            continue
        tx_ref = source["row_id"]
        meta = tx_meta.get(tx_ref)
        kind = str(meta["kind"] if meta else "").strip().lower().replace("-", "_").replace(" ", "_")
        facts = entry_types.get(tx_ref, {"entry_types": set(), "at_categories": set()})
        classification = classify(facts["entry_types"], facts["at_categories"], tx_ref in swap_ids)
        recorded_value = _optional_decimal_pref(source, "fiat_value_exact", "fiat_value")
        fiat_currency = str(export_row["fiat_currency"] or "").upper()
        has_fiat_leg = recorded_value is not None and fiat_currency == book_currency
        event, warning = _event(
            classification,
            source["direction"],
            kind,
            has_fiat_leg,
            taxable_conversion=tx_ref in taxable_conversions,
        )
        if event is None:
            unsupported.append(
                {
                    "transaction_ref": tx_ref,
                    "transaction_id": source["transaction_id"] or "",
                    "wallet": source["wallet"],
                    "occurred_at": source["occurred_at"],
                    "classification": classification,
                }
            )
            continue
        if warning:
            warning_counts[warning] = warning_counts.get(warning, 0) + 1
        asset = describe_asset(source["asset"])
        asset_code, has_ticker = _platform_asset_code(asset)
        if not has_ticker:
            tickerless.add(asset_code)
        amounts = normalize_boundary_amounts(
            direction=source["direction"],
            amount_msat=source["amount"],
            fee_msat=source["fee"],
            amount_includes_fee=bool(meta["amount_includes_fee"]) if meta else False,
        )
        outbound = amounts.direction == "outbound"
        journal = journal_values.get(tx_ref, {})
        if event in {"income", "mining", "staking", "airdrop", "interest"} and "income" in journal:
            value = journal["income"]
        elif event in {"sell", "payment"} and "proceeds" in journal:
            value = journal["proceeds"]
        elif event == "buy" and warning == "conversion" and "cost" in journal:
            value = journal["cost"]
        elif export_row["fmv"] is not None:
            value = Decimal(str(export_row["fmv"]))
        else:
            value = None
        link = links.get(tx_ref)
        note_parts = [f"Kassiber {classification}"]
        if link and link["refs"]:
            note_parts.append("pair " + ", ".join(sorted(link["refs"])))
        comment = " · ".join(
            part for part in (str(export_row["description"] or "").strip(), "; ".join(note_parts)) if part
        )
        rows.append(
            PlatformRow(
                occurred_at=_parse_timestamp(source["occurred_at"]),
                wallet=source["wallet"],
                transaction_id=source["transaction_id"] or tx_ref,
                event=event,
                classification=classification,
                asset=asset,
                asset_code=asset_code,
                # A fee-only row moves its whole debit as the fee itself.
                amount=asset.exact(
                    amounts.wallet_movement_msat if event == "fee" else amounts.principal_msat
                ),
                fee=(
                    asset.exact(amounts.fee_msat)
                    if outbound and amounts.fee_msat and event != "fee"
                    else ""
                ),
                fiat_currency=book_currency,
                # A taxable conversion trades at its booked proceeds or cost.
                fiat_amount=(
                    _number(value)
                    if warning == "conversion"
                    else _number(recorded_value) if has_fiat_leg else ""
                ),
                value=_number(value),
                comment=comment,
            )
        )

    if unsupported:
        raise AppError(
            "Some transactions are part own-wallet transfer and part taxable event",
            code="platform_export_unsupported",
            hint=(
                "Neither tool's import row can hold both parts, and dropping one would change "
                "the tax result. Export these wallets as Kassiber Excel or CSV, or enter the "
                "rows in the target tool by hand."
            ),
            details={"count": len(unsupported), "transactions": unsupported[:20]},
            retryable=False,
        )

    warnings = [
        _WARNING_TEXT[key].format(count=count)
        for key, count in sorted(warning_counts.items())
    ]
    if tickerless:
        warnings.append(_WARNING_TEXT["ticker"].format(assets=", ".join(sorted(tickerless))))
    if any(row.value for row in rows):
        warnings.append(_WARNING_TEXT["value"].format(currency=book_currency))
    return {
        "workspace": report_context.workspace,
        "profile": profile,
        "wallets": wallets,
        "rows": rows,
        "warnings": warnings,
    }


def export_platform_csv(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    hooks: ReportHooks,
    *,
    platform: str,
    wallet_ref=None,
    wallet_refs=None,
) -> dict[str, Any]:
    target = platform_format(platform)
    built = build_platform_rows(
        conn, workspace_ref, profile_ref, hooks, wallet_ref=wallet_ref, wallet_refs=wallet_refs
    )
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(target.headers))
        writer.writeheader()
        for row in built["rows"]:
            writer.writerow(target.build_row(row))
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "format": "csv",
        "platform": target.id,
        "platform_label": target.label,
        "wallet": export_wallet_scope_label(built["wallets"]),
        "rows": len(built["rows"]),
        "warnings": built["warnings"],
    }


__all__ = [
    "BLOCKPIT",
    "COINTRACKING",
    "EVENTS",
    "PLATFORM_FORMATS",
    "PlatformFormat",
    "PlatformRow",
    "build_platform_rows",
    "export_platform_csv",
    "platform_format",
]
