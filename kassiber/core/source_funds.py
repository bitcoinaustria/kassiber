from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import uuid
from collections import defaultdict, deque
from dataclasses import dataclass
from typing import Any, Callable, Mapping, Sequence

from ..envelope import json_ready
from ..errors import AppError
from ..msat import btc_to_msat, dec, msat_to_btc
from ..source_funds_pdf_report import write_source_funds_pdf
from ..time_utils import UNKNOWN_OCCURRED_AT, now_iso, parse_timestamp
from ..wallet_descriptors import normalize_asset_code, normalize_chain, normalize_network
from .source_funds_hints import enrich_findings_with_next_steps


REVEAL_MODES = ("labels_only", "minimal", "standard", "full")
REPORT_PURPOSES = ("existing_transaction", "planned_exchange_sale")
SOURCE_TYPES = (
    "fiat_purchase",
    "exchange_withdrawal",
    "mining",
    "income",
    "gift",
    "opening_balance_attestation",
    "missing_history",
    "unknown",
)
LINK_TYPES = (
    "self_transfer",
    "exchange_transfer",
    "trade",
    "swap",
    "peg_in",
    "peg_out",
    "lightning_funding",
    "lightning_close",
    "lightning_routed",
    "lightning_swap",
    "coinjoin",
    "payjoin",
    "manual_source",
    "missing_history",
)
LINK_STATES = ("suggested", "reviewed", "rejected")
CONFIDENCE_LEVELS = ("exact", "strong", "weak", "unknown")
ALLOCATION_POLICIES = ("explicit", "heuristic", "unknown")
PRIVACY_LINK_TYPES = {"coinjoin", "payjoin"}
ATTESTATION_SOURCE_TYPES = {"missing_history", "opening_balance_attestation"}
DETERMINISTIC_BULK_REVIEW_METHODS = {
    "same_external_id",
    "transaction_pair",
    "provider_trade_id",
    "provider_order_id",
    "provider_payment_id",
    "provider_exchange_order_id",
    "provider_ledger_id",
}
PROVIDER_UNIQUE_KEYS = (
    "trade_id",
    "order_id",
    "payment_id",
    "provider_trade_id",
    "exchange_order_id",
    "ledger_id",
)
PROVIDER_BROAD_KEYS = ("provider_id",)
PROVIDER_EVIDENCE_KEYS = PROVIDER_UNIQUE_KEYS + PROVIDER_BROAD_KEYS
SUGGESTION_WRITE_CAP = 500
_PUBLIC_TXID_RE = re.compile(r"^[0-9a-fA-F]{64}$")
_PUBLIC_EXPLORER_BASES = {
    ("bitcoin", "main"): ("mempool.space", "https://mempool.space"),
    ("bitcoin", "test"): ("mempool.space testnet", "https://mempool.space/testnet"),
    ("bitcoin", "signet"): ("mempool.space signet", "https://mempool.space/signet"),
    ("liquid", "liquidv1"): ("Liquid Network", "https://liquid.network"),
    ("liquid", "liquidtestnet"): ("Liquid Network Testnet", "https://liquid.network/testnet"),
}
# Hard cap for build_report's max_depth so a caller cannot run an
# unbounded BFS through transaction history. Real audit chains are
# well below this; the cap exists to prevent runaway sweeps when a
# malformed --max-depth, daemon arg, or coverage call slips through.
_MAX_BUILD_REPORT_DEPTH = 64
# Hard caps on the report graph itself. Depth is one bound, but a
# wide reviewed graph (or unreviewed suggestions still hanging on a
# target) can grow nodes/edges within the depth budget. Treat both
# as blocking truncation events: emit path_truncated once and stop
# walking so preview/cases.save/coverage stay bounded synchronously.
_MAX_BUILD_REPORT_NODES = 5_000
_MAX_BUILD_REPORT_EDGES = 5_000

ScopeResolver = Callable[[sqlite3.Connection, str | None, str | None], tuple[Mapping[str, Any], Mapping[str, Any]]]
TransactionResolver = Callable[..., Mapping[str, Any]]
FormatTable = Callable[..., list[str]]


@dataclass(frozen=True)
class SourceFundsHooks:
    resolve_scope: ScopeResolver
    resolve_transaction: TransactionResolver
    format_table: FormatTable


def _now() -> str:
    return now_iso()


def _normalize_reveal_mode(value: str | None) -> str:
    mode = (value or "standard").strip().lower()
    if mode not in REVEAL_MODES:
        raise AppError(
            f"Unsupported reveal mode '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(REVEAL_MODES)}",
        )
    return mode


def _normalize_report_purpose(value: str | None) -> str:
    purpose = (value or "existing_transaction").strip().lower().replace("-", "_")
    if purpose not in REPORT_PURPOSES:
        raise AppError(
            f"Unsupported source-funds report purpose '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(REPORT_PURPOSES)}",
        )
    return purpose


def _normalize_source_type(value: str) -> str:
    source_type = str(value or "").strip().lower().replace("-", "_")
    if source_type not in SOURCE_TYPES:
        raise AppError(
            f"Unsupported source type '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(SOURCE_TYPES)}",
        )
    return source_type


def _normalize_link_type(value: str) -> str:
    link_type = str(value or "").strip().lower().replace("-", "_")
    if link_type not in LINK_TYPES:
        raise AppError(
            f"Unsupported link type '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(LINK_TYPES)}",
        )
    return link_type


def _normalize_state(value: str | None) -> str:
    state = (value or "suggested").strip().lower()
    if state in {"accept", "accepted"}:
        state = "reviewed"
    if state in {"reject", "rejected"}:
        state = "rejected"
    if state not in LINK_STATES:
        raise AppError(
            f"Unsupported source-funds link state '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(LINK_STATES)}",
        )
    return state


def _normalize_confidence(value: str | None) -> str:
    confidence = (value or "unknown").strip().lower()
    if confidence not in CONFIDENCE_LEVELS:
        raise AppError(
            f"Unsupported confidence '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(CONFIDENCE_LEVELS)}",
        )
    return confidence


def _normalize_allocation_policy(value: str | None) -> str:
    policy = (value or "unknown").strip().lower()
    if policy not in ALLOCATION_POLICIES:
        raise AppError(
            f"Unsupported allocation policy '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(ALLOCATION_POLICIES)}",
        )
    return policy


def _amount_msat(value: Any, *, label: str, required: bool = False) -> int | None:
    if value in (None, ""):
        if required:
            raise AppError(f"{label} is required", code="validation")
        return None
    amount = btc_to_msat(dec(value))
    if amount < 0:
        raise AppError(f"{label} must not be negative", code="validation")
    return amount


def _normalize_provider_method(key: str) -> str:
    normalized = str(key or "").strip().lower()
    if normalized.startswith("provider_"):
        return normalized
    return f"provider_{normalized}"


def _same_asset_amount_close(out_tx: Mapping[str, Any], in_tx: Mapping[str, Any]) -> bool:
    if normalize_asset_code(out_tx["asset"]) != normalize_asset_code(in_tx["asset"]):
        return False
    out_amount = abs(int(out_tx["amount"]))
    in_amount = abs(int(in_tx["amount"]))
    tolerance = max(1000, max(out_amount, in_amount) // 100)
    return abs(out_amount - in_amount) <= tolerance


def _btc_value(msat: int | None) -> float | None:
    if msat is None:
        return None
    return float(msat_to_btc(msat))


def _safe_json_loads(value: str | None) -> Any:
    if not value:
        return {}
    try:
        return json.loads(value)
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}


def _canonical_optional_timestamp(value: Any) -> str | None:
    if value in (None, ""):
        return None
    try:
        parsed = parse_timestamp(value)
    except AppError:
        return None
    if parsed == UNKNOWN_OCCURRED_AT:
        return None
    return parsed


def _timestamp_after(left: Any, right: Any) -> bool:
    left_ts = _canonical_optional_timestamp(left)
    right_ts = _canonical_optional_timestamp(right)
    return bool(left_ts and right_ts and left_ts > right_ts)


def _public_tx_id(row: Mapping[str, Any], reveal_mode: str, *, is_target: bool = False) -> str:
    if reveal_mode == "labels_only":
        return ""
    if reveal_mode == "minimal" and not is_target:
        return ""
    return row["external_id"] or row["id"]


def _wallet_chain_network(config_json: Any, asset: Any) -> tuple[str, str]:
    config = {}
    if config_json:
        try:
            loaded = json.loads(str(config_json))
            if isinstance(loaded, dict):
                config = loaded
        except (TypeError, ValueError, json.JSONDecodeError):
            return "", ""
    chain_hint = config.get("chain") or (
        "liquid" if normalize_asset_code(str(asset or "")) == "LBTC" else "bitcoin"
    )
    try:
        chain = normalize_chain(chain_hint)
        network = normalize_network(chain, config.get("network"))
    except ValueError:
        return "", ""
    return chain, network


def _public_explorer_link(
    txid: str,
    asset: Any,
    wallet_config_json: Any = None,
) -> dict[str, Any] | None:
    if not _PUBLIC_TXID_RE.fullmatch(str(txid or "").strip()):
        return None
    chain, network = _wallet_chain_network(wallet_config_json, asset)
    explorer = _PUBLIC_EXPLORER_BASES.get((chain, network))
    if not explorer:
        return None
    label, base_url = explorer
    return {
        "txid": txid,
        "asset": normalize_asset_code(str(asset or "")) or "BTC",
        "chain": chain,
        "network": network,
        "label": label,
        "url": f"{base_url}/tx/{txid}",
    }


def _public_explanation(text: str | None, reveal_mode: str) -> str:
    """Redact the link explanation under reveal modes that disallow free text.

    Suggestion-builder explanations include provider key/value pairs
    (trade ID, order ID, payment ID), and reviewed links inherit any
    free text the user typed into ``--explanation``. labels_only and
    minimal modes already redact txids and free-text description /
    counterparty fields; the explanation deserves the same treatment.
    Standard and full keep the original text so the recipient can read
    why the link was reviewed.
    """
    if not text:
        return ""
    if reveal_mode in {"standard", "full"}:
        return text
    return ""


def _label(value: Any) -> str:
    return str(value or "").replace("_", " ")


def _mapping_value(row: Mapping[str, Any], key: str, default: Any = "") -> Any:
    if hasattr(row, "keys") and key in row.keys():
        return row[key]
    if hasattr(row, "get"):
        return row.get(key, default)
    return default


def _report_context(profile: Mapping[str, Any]) -> dict[str, Any]:
    tax_country = str(_mapping_value(profile, "tax_country", "generic") or "generic").lower()
    fiat_currency = str(_mapping_value(profile, "fiat_currency", "") or "").upper()
    is_at_eur = tax_country == "at" and fiat_currency == "EUR"
    jurisdiction_label = "Austria" if tax_country == "at" else "Generic"
    checklist = [
        "Original fiat-to-bitcoin purchase or income evidence is attached to the root source.",
        "Reviewed wallet-transfer and consolidation hops cover the disclosed target amount.",
        "The target exchange, broker, or bank deposit is the selected report target.",
        "The PDF is rendered from a saved immutable case snapshot.",
    ]
    if is_at_eur:
        checklist.append(
            "EUR values and Austrian profile context are present for a basic Mittelherkunftsnachweis workflow."
        )
    return {
        "tax_country": tax_country,
        "fiat_currency": fiat_currency,
        "jurisdiction_label": jurisdiction_label,
        "template_key": "at_eur_basic" if is_at_eur else "generic_basic",
        "report_title": (
            "Mittelherkunftsnachweis / Source of Funds Report"
            if is_at_eur
            else "Source of Funds Report"
        ),
        "report_subtitle": (
            "Reviewed local evidence disclosure for an Austrian/EUR source-of-funds workflow."
            if is_at_eur
            else "Reviewed local evidence disclosure from a saved immutable case snapshot."
        ),
        "evidence_checklist": checklist,
        "deferred": [
            "Full country-specific legal templates",
            "German localization",
            "Locale-specific money/date formatting beyond the current ReportLab renderer",
            "CoinJoin/PayJoin traversal",
        ],
    }


def _tx_label(row: Mapping[str, Any], reveal_mode: str, *, is_target: bool = False) -> str:
    wallet = row["wallet_label"] if "wallet_label" in row.keys() else row.get("wallet", "")
    public_id = _public_tx_id(row, reveal_mode, is_target=is_target)
    if public_id:
        return public_id
    return f"{wallet} {row['direction']} {row['asset']} {float(msat_to_btc(row['amount'])):.8f}"


def _row_dict(row: Mapping[str, Any] | None) -> dict[str, Any] | None:
    if row is None:
        return None
    return {key: row[key] for key in row.keys()}


def _attachment_summary(row: Mapping[str, Any], reveal_mode: str = "full") -> dict[str, Any]:
    mode = _normalize_reveal_mode(reveal_mode)
    item = {
        "id": row["id"],
        "attachment_type": row["attachment_type"],
        "label": row["label"],
    }
    if mode in {"standard", "full"}:
        item["transaction_id"] = row["transaction_id"]
        item["media_type"] = row["media_type"] or ""
        item["sha256"] = row["sha256"] or ""
    if mode == "full":
        item["source_url"] = row["source_url"] or ""
        item["stored_relpath"] = row["stored_relpath"] or ""
    return item


def _source_row_to_dict(conn: sqlite3.Connection, row: Mapping[str, Any]) -> dict[str, Any]:
    # _source_row_to_dict and _link_row_to_dict back the editor list
    # endpoints (`source-funds sources/links list`). Those views serve
    # the user's own desktop on their own data — the user is the
    # disclosure boundary, not a recipient — so attachment metadata is
    # rendered in full (`reveal_mode="full"` on `_attachment_summary`).
    # Disclosure-side redaction lives on `build_report`; do not change
    # this without thinking through the editor UX.
    attachments = conn.execute(
        """
        SELECT a.*
        FROM source_funds_source_attachments sfa
        JOIN attachments a ON a.id = sfa.attachment_id
        WHERE sfa.source_id = ?
        ORDER BY sfa.created_at ASC, a.id ASC
        """,
        (row["id"],),
    ).fetchall()
    return {
        "id": row["id"],
        "source_type": row["source_type"],
        "label": row["label"],
        "asset": row["asset"],
        "amount": _btc_value(row["amount"]),
        "amount_msat": row["amount"],
        "fiat_currency": row["fiat_currency"] or "",
        "fiat_value": row["fiat_value"],
        "acquired_at": row["acquired_at"] or "",
        "description": row["description"] or "",
        "review_state": row["review_state"],
        "attachments": [_attachment_summary(attachment) for attachment in attachments],
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def _link_row_to_dict(conn: sqlite3.Connection, row: Mapping[str, Any]) -> dict[str, Any]:
    attachments = conn.execute(
        """
        SELECT a.*
        FROM source_funds_link_attachments lfa
        JOIN attachments a ON a.id = lfa.attachment_id
        WHERE lfa.link_id = ?
        ORDER BY lfa.created_at ASC, a.id ASC
        """,
        (row["id"],),
    ).fetchall()
    return {
        "id": row["id"],
        "from_source_id": row["from_source_id"],
        "from_transaction_id": row["from_transaction_id"],
        "to_transaction_id": row["to_transaction_id"],
        "link_type": row["link_type"],
        "state": row["state"],
        "confidence": row["confidence"],
        "method": row["method"],
        "asset": row["asset"],
        "allocation_amount": _btc_value(row["allocation_amount"]),
        "allocation_amount_msat": row["allocation_amount"],
        "from_asset": row["from_asset"] or row["asset"],
        "from_allocation_amount": _btc_value(row["from_allocation_amount"]),
        "from_allocation_amount_msat": row["from_allocation_amount"],
        "allocation_policy": row["allocation_policy"],
        "explanation": row["explanation"] or "",
        "uses_chain_observation": bool(row["uses_chain_observation"]),
        "chain_data_confirmed": bool(row["chain_data_confirmed"]),
        "attachments": [_attachment_summary(attachment) for attachment in attachments],
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def _transaction_by_id(conn: sqlite3.Connection, profile_id: str, tx_id: str):
    return conn.execute(
        """
        SELECT t.*, w.label AS wallet_label, w.config_json AS wallet_config_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.id = ?
        """,
        (profile_id, tx_id),
    ).fetchone()


def _resolve_source(conn: sqlite3.Connection, profile_id: str, ref: str):
    rows = conn.execute(
        """
        SELECT *
        FROM source_funds_sources
        WHERE profile_id = ? AND (id = ? OR lower(label) = lower(?))
        ORDER BY created_at DESC, id DESC
        LIMIT 2
        """,
        (profile_id, ref, ref),
    ).fetchall()
    if len(rows) > 1:
        raise AppError(
            f"Source-funds source '{ref}' is ambiguous",
            code="ambiguous_reference",
            hint="Use the source id from `source-funds sources list`.",
        )
    if not rows:
        raise AppError(f"Source-funds source '{ref}' not found", code="not_found")
    return rows[0]


def _resolve_link(conn: sqlite3.Connection, profile_id: str, ref: str):
    row = conn.execute(
        "SELECT * FROM source_funds_links WHERE profile_id = ? AND id = ?",
        (profile_id, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Source-funds link '{ref}' not found", code="not_found")
    return row


def _require_attachment(conn: sqlite3.Connection, profile_id: str, attachment_id: str):
    row = conn.execute(
        "SELECT * FROM attachments WHERE profile_id = ? AND id = ?",
        (profile_id, attachment_id),
    ).fetchone()
    if not row:
        raise AppError(f"Attachment '{attachment_id}' not found", code="not_found")
    return row


def create_source(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    source_type: str,
    label: str,
    asset: str = "BTC",
    amount: Any = None,
    fiat_value: Any = None,
    fiat_currency: str | None = None,
    acquired_at: str | None = None,
    description: str | None = None,
    attachment_ids: Sequence[str] | None = None,
) -> dict[str, Any]:
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    source_id = str(uuid.uuid4())
    created_at = _now()
    normalized_asset = normalize_asset_code(asset)
    amount_msat = _amount_msat(amount, label="--amount")
    source_type = _normalize_source_type(source_type)
    fiat = None if fiat_value in (None, "") else float(dec(fiat_value))
    currency = (fiat_currency or profile["fiat_currency"] or "").strip().upper() or None
    stored_acquired_at = parse_timestamp(acquired_at) if acquired_at not in (None, "") else None
    label = str(label or "").strip()
    if not label:
        raise AppError("--label cannot be empty", code="validation")
    conn.execute(
        """
        INSERT INTO source_funds_sources(
            id, workspace_id, profile_id, source_type, label, asset, amount,
            fiat_currency, fiat_value, acquired_at, description, review_state,
            created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'reviewed', ?, ?)
        """,
        (
            source_id,
            workspace["id"],
            profile["id"],
            source_type,
            label,
            normalized_asset,
            amount_msat,
            currency,
            fiat,
            stored_acquired_at,
            description,
            created_at,
            created_at,
        ),
    )
    for attachment_id in attachment_ids or ():
        _require_attachment(conn, profile["id"], attachment_id)
        conn.execute(
            """
            INSERT OR IGNORE INTO source_funds_source_attachments(source_id, attachment_id, created_at)
            VALUES(?, ?, ?)
            """,
            (source_id, attachment_id, created_at),
        )
    conn.commit()
    return _source_row_to_dict(
        conn,
        conn.execute("SELECT * FROM source_funds_sources WHERE id = ?", (source_id,)).fetchone(),
    )


def list_sources(conn: sqlite3.Connection, workspace_ref: str | None, profile_ref: str | None, hooks: SourceFundsHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT *
        FROM source_funds_sources
        WHERE profile_id = ?
        ORDER BY created_at DESC, id DESC
        """,
        (profile["id"],),
    ).fetchall()
    return [_source_row_to_dict(conn, row) for row in rows]


def attach_source_evidence(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    source_ref: str,
    attachment_id: str,
) -> dict[str, Any]:
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    source = _resolve_source(conn, profile["id"], source_ref)
    _require_attachment(conn, profile["id"], attachment_id)
    conn.execute(
        """
        INSERT OR IGNORE INTO source_funds_source_attachments(source_id, attachment_id, created_at)
        VALUES(?, ?, ?)
        """,
        (source["id"], attachment_id, _now()),
    )
    conn.commit()
    return _source_row_to_dict(conn, source)


def _find_existing_link(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    from_source_id: str | None,
    from_transaction_id: str | None,
    to_transaction_id: str,
    method: str,
    link_type: str,
):
    if from_source_id:
        return conn.execute(
            """
            SELECT * FROM source_funds_links
            WHERE profile_id = ? AND from_source_id = ? AND to_transaction_id = ?
              AND method = ? AND link_type = ?
            LIMIT 1
            """,
            (profile_id, from_source_id, to_transaction_id, method, link_type),
        ).fetchone()
    return conn.execute(
        """
        SELECT * FROM source_funds_links
        WHERE profile_id = ? AND from_transaction_id = ? AND to_transaction_id = ?
          AND method = ? AND link_type = ?
        LIMIT 1
        """,
        (profile_id, from_transaction_id, to_transaction_id, method, link_type),
    ).fetchone()


def _validate_transaction_link_for_review(
    *,
    link_type: str,
    from_tx: Mapping[str, Any] | None,
    source: Mapping[str, Any] | None = None,
    to_tx: Mapping[str, Any],
    asset: str,
    from_asset: str | None,
    allocation_msat: int | None,
    from_allocation_msat: int | None,
) -> None:
    if allocation_msat is not None and allocation_msat > int(to_tx["amount"]):
        raise AppError(
            "A source-funds link allocation cannot exceed the target transaction amount.",
            code="validation",
        )
    if source and _timestamp_after(source["acquired_at"], to_tx["occurred_at"]):
        raise AppError(
            "A source-funds source cannot be acquired after the transaction it funds.",
            code="validation",
        )
    if not from_tx:
        return
    if from_tx["id"] == to_tx["id"]:
        raise AppError(
            "A source-funds link's from-transaction and to-transaction must differ.",
            code="validation",
        )
    if _timestamp_after(from_tx["occurred_at"], to_tx["occurred_at"]):
        raise AppError(
            "A source-funds link's parent transaction occurs after the child.",
            code="validation",
        )
    parent_required = (
        from_allocation_msat if from_allocation_msat is not None else allocation_msat
    )
    if parent_required is not None and parent_required > int(from_tx["amount"]):
        raise AppError(
            "A source-funds link from-allocation cannot exceed the parent transaction amount.",
            code="validation",
        )
    if link_type != "self_transfer":
        return
    from_tx_asset = normalize_asset_code(from_tx["asset"])
    to_tx_asset = normalize_asset_code(to_tx["asset"])
    link_asset = normalize_asset_code(asset)
    link_from_asset = normalize_asset_code(from_asset or from_tx_asset)
    if from_tx_asset != to_tx_asset:
        raise AppError(
            "Self-transfer source-funds links require the same asset on both transactions.",
            code="validation",
            hint="Use a swap, peg-in, or peg-out link for cross-asset flows.",
        )
    if link_asset != to_tx_asset:
        raise AppError(
            "A self-transfer link's asset must match the target transaction asset.",
            code="validation",
        )
    if link_from_asset != from_tx_asset:
        raise AppError(
            "A self-transfer link's from-asset must match the parent transaction asset.",
            code="validation",
        )


def create_link(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    to_transaction_ref: str,
    from_transaction_ref: str | None = None,
    from_source_ref: str | None = None,
    link_type: str = "self_transfer",
    state: str = "reviewed",
    confidence: str = "strong",
    method: str = "manual",
    asset: str | None = None,
    allocation_amount: Any = None,
    from_asset: str | None = None,
    from_allocation_amount: Any = None,
    allocation_policy: str = "explicit",
    explanation: str | None = None,
    uses_chain_observation: bool = False,
    chain_data_confirmed: bool = False,
    attachment_ids: Sequence[str] | None = None,
    commit: bool = True,
) -> dict[str, Any]:
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    if bool(from_transaction_ref) == bool(from_source_ref):
        raise AppError(
            "Provide exactly one of --from-transaction or --from-source",
            code="validation",
        )
    to_tx = hooks.resolve_transaction(conn, profile["id"], to_transaction_ref)
    from_tx = hooks.resolve_transaction(conn, profile["id"], from_transaction_ref) if from_transaction_ref else None
    source = _resolve_source(conn, profile["id"], from_source_ref) if from_source_ref else None
    link_type = _normalize_link_type(link_type)
    state = _normalize_state(state)
    confidence = _normalize_confidence(confidence)
    allocation_policy = _normalize_allocation_policy(allocation_policy)
    normalized_asset = normalize_asset_code(asset or to_tx["asset"])
    normalized_from_asset = normalize_asset_code(from_asset or (from_tx["asset"] if from_tx else source["asset"]))
    allocation_msat = _amount_msat(allocation_amount, label="--allocation-amount")
    from_allocation_msat = _amount_msat(from_allocation_amount, label="--from-amount")
    _validate_transaction_link_for_review(
        link_type=link_type,
        from_tx=from_tx,
        source=source,
        to_tx=to_tx,
        asset=normalized_asset,
        from_asset=normalized_from_asset,
        allocation_msat=allocation_msat,
        from_allocation_msat=from_allocation_msat,
    )
    existing = _find_existing_link(
        conn,
        profile["id"],
        from_source_id=source["id"] if source else None,
        from_transaction_id=from_tx["id"] if from_tx else None,
        to_transaction_id=to_tx["id"],
        method=method,
        link_type=link_type,
    )
    if existing:
        return _link_row_to_dict(conn, existing)
    link_id = str(uuid.uuid4())
    created_at = _now()
    conn.execute(
        """
        INSERT INTO source_funds_links(
            id, workspace_id, profile_id, from_source_id, from_transaction_id,
            to_transaction_id, link_type, state, confidence, method, asset,
            allocation_amount, from_asset, from_allocation_amount, allocation_policy,
            explanation, uses_chain_observation, chain_data_confirmed,
            created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            link_id,
            workspace["id"],
            profile["id"],
            source["id"] if source else None,
            from_tx["id"] if from_tx else None,
            to_tx["id"],
            link_type,
            state,
            confidence,
            method,
            normalized_asset,
            allocation_msat,
            normalized_from_asset,
            from_allocation_msat,
            allocation_policy,
            explanation,
            1 if uses_chain_observation else 0,
            1 if chain_data_confirmed else 0,
            created_at,
            created_at,
        ),
    )
    for attachment_id in attachment_ids or ():
        _require_attachment(conn, profile["id"], attachment_id)
        conn.execute(
            """
            INSERT OR IGNORE INTO source_funds_link_attachments(link_id, attachment_id, created_at)
            VALUES(?, ?, ?)
            """,
            (link_id, attachment_id, created_at),
        )
    if commit:
        conn.commit()
    return _link_row_to_dict(
        conn,
        conn.execute("SELECT * FROM source_funds_links WHERE id = ?", (link_id,)).fetchone(),
    )


def update_link_review(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    link_ref: str,
    state: str | None = None,
    link_type: str | None = None,
    confidence: str | None = None,
    allocation_amount: Any = None,
    from_allocation_amount: Any = None,
    allocation_policy: str | None = None,
    explanation: str | None = None,
    uses_chain_observation: bool | None = None,
    chain_data_confirmed: bool | None = None,
) -> dict[str, Any]:
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    link = _resolve_link(conn, profile["id"], link_ref)
    updates: dict[str, Any] = {}
    if state is not None:
        updates["state"] = _normalize_state(state)
    if link_type is not None:
        updates["link_type"] = _normalize_link_type(link_type)
    if confidence is not None:
        updates["confidence"] = _normalize_confidence(confidence)
    if allocation_amount not in (None, ""):
        updates["allocation_amount"] = _amount_msat(allocation_amount, label="--allocation-amount")
    if from_allocation_amount not in (None, ""):
        updates["from_allocation_amount"] = _amount_msat(from_allocation_amount, label="--from-amount")
    if allocation_policy is not None:
        updates["allocation_policy"] = _normalize_allocation_policy(allocation_policy)
    if explanation is not None:
        updates["explanation"] = explanation
    if uses_chain_observation is not None:
        if not isinstance(uses_chain_observation, bool):
            raise AppError(
                "uses_chain_observation must be a boolean",
                code="validation",
            )
        updates["uses_chain_observation"] = 1 if uses_chain_observation else 0
    if chain_data_confirmed is not None:
        if not isinstance(chain_data_confirmed, bool):
            raise AppError(
                "chain_data_confirmed must be a boolean",
                code="validation",
            )
        updates["chain_data_confirmed"] = 1 if chain_data_confirmed else 0
    if not updates:
        raise AppError("source-funds links review requires at least one update", code="validation")
    candidate_state = updates.get("state", link["state"])
    candidate_link_type = updates.get("link_type", link["link_type"])
    if candidate_state == "reviewed":
        from_tx = _transaction_by_id(conn, profile["id"], link["from_transaction_id"])
        to_tx = _transaction_by_id(conn, profile["id"], link["to_transaction_id"])
        source = (
            conn.execute(
                "SELECT * FROM source_funds_sources WHERE id = ?",
                (link["from_source_id"],),
            ).fetchone()
            if link["from_source_id"]
            else None
        )
        if to_tx:
            _validate_transaction_link_for_review(
                link_type=candidate_link_type,
                from_tx=from_tx,
                source=source,
                to_tx=to_tx,
                asset=updates.get("asset", link["asset"]),
                from_asset=updates.get("from_asset", link["from_asset"]),
                allocation_msat=updates.get("allocation_amount", link["allocation_amount"]),
                from_allocation_msat=updates.get(
                    "from_allocation_amount",
                    link["from_allocation_amount"],
                ),
            )
    updates["updated_at"] = _now()
    assignments = ", ".join(f"{key} = ?" for key in updates)
    conn.execute(
        f"UPDATE source_funds_links SET {assignments} WHERE id = ?",
        (*updates.values(), link["id"]),
    )
    conn.commit()
    return _link_row_to_dict(
        conn,
        conn.execute("SELECT * FROM source_funds_links WHERE id = ?", (link["id"],)).fetchone(),
    )


def attach_link_evidence(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    link_ref: str,
    attachment_id: str,
) -> dict[str, Any]:
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    link = _resolve_link(conn, profile["id"], link_ref)
    _require_attachment(conn, profile["id"], attachment_id)
    conn.execute(
        """
        INSERT OR IGNORE INTO source_funds_link_attachments(link_id, attachment_id, created_at)
        VALUES(?, ?, ?)
        """,
        (link["id"], attachment_id, _now()),
    )
    conn.commit()
    return _link_row_to_dict(conn, link)


def list_links(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    target_transaction_ref: str | None = None,
    state: str | None = None,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    where = ["profile_id = ?"]
    params: list[Any] = [profile["id"]]
    if target_transaction_ref:
        tx = hooks.resolve_transaction(conn, profile["id"], target_transaction_ref)
        where.append("to_transaction_id = ?")
        params.append(tx["id"])
    if state:
        where.append("state = ?")
        params.append(_normalize_state(state))
    rows = conn.execute(
        f"""
        SELECT *
        FROM source_funds_links
        WHERE {' AND '.join(where)}
        ORDER BY created_at DESC, id DESC
        """,
        params,
    ).fetchall()
    return [_link_row_to_dict(conn, row) for row in rows]


def _is_bulk_reviewable_suggestion(row: Mapping[str, Any]) -> bool:
    method = str(row["method"] or "")
    return (
        row["state"] == "suggested"
        and method in DETERMINISTIC_BULK_REVIEW_METHODS
        and row["confidence"] in {"exact", "strong"}
        and row["allocation_amount"] is not None
        and not bool(row["uses_chain_observation"])
    )


def _same_external_id_still_deterministic(
    conn: sqlite3.Connection,
    profile_id: str,
    row: Mapping[str, Any],
    from_tx: Mapping[str, Any] | None,
    to_tx: Mapping[str, Any],
) -> bool:
    if not from_tx:
        return False
    external_id = from_tx["external_id"]
    if not external_id or external_id != to_tx["external_id"]:
        return False
    if normalize_asset_code(from_tx["asset"]) != normalize_asset_code(to_tx["asset"]):
        return False
    group = [
        tx
        for tx in _active_transaction_rows(conn, profile_id)
        if tx["external_id"] == external_id
        and normalize_asset_code(tx["asset"]) == normalize_asset_code(to_tx["asset"])
    ]
    outs = [tx for tx in group if tx["direction"] == "outbound"]
    ins = [tx for tx in group if tx["direction"] == "inbound"]
    return (
        len(outs) == 1
        and len(ins) == 1
        and outs[0]["id"] == row["from_transaction_id"] == from_tx["id"]
        and ins[0]["id"] == row["to_transaction_id"] == to_tx["id"]
        and outs[0]["wallet_id"] != ins[0]["wallet_id"]
    )


def _transaction_pair_still_deterministic(
    conn: sqlite3.Connection,
    profile_id: str,
    row: Mapping[str, Any],
    from_tx: Mapping[str, Any] | None,
    to_tx: Mapping[str, Any],
) -> bool:
    if not from_tx:
        return False
    return bool(
        conn.execute(
            """
            SELECT 1
            FROM transaction_pairs
            WHERE profile_id = ? AND out_transaction_id = ? AND in_transaction_id = ?
              AND deleted_at IS NULL
            LIMIT 1
            """,
            (profile_id, from_tx["id"], to_tx["id"]),
        ).fetchone()
    )


def _provider_values_for_method(row: Mapping[str, Any], method: str) -> set[str]:
    return {
        value
        for key, value in _raw_evidence_values(row)
        if key in PROVIDER_UNIQUE_KEYS and _normalize_provider_method(key) == method
    }


def _provider_key_still_deterministic(
    conn: sqlite3.Connection,
    profile_id: str,
    row: Mapping[str, Any],
    from_tx: Mapping[str, Any] | None,
    to_tx: Mapping[str, Any],
) -> bool:
    if not from_tx:
        return False
    method = str(row["method"] or "")
    if method not in DETERMINISTIC_BULK_REVIEW_METHODS or not method.startswith("provider_"):
        return False
    shared_values = _provider_values_for_method(from_tx, method) & _provider_values_for_method(to_tx, method)
    if not shared_values:
        return False
    active_rows = _active_transaction_rows(conn, profile_id)
    for value in shared_values:
        group = [
            tx
            for tx in active_rows
            if value in _provider_values_for_method(tx, method)
        ]
        outs = [tx for tx in group if tx["direction"] == "outbound"]
        ins = [tx for tx in group if tx["direction"] == "inbound"]
        if (
            len(outs) == 1
            and len(ins) == 1
            and outs[0]["id"] == row["from_transaction_id"] == from_tx["id"]
            and ins[0]["id"] == row["to_transaction_id"] == to_tx["id"]
            and _same_asset_amount_close(from_tx, to_tx)
        ):
            return True
    return False


def _suggestion_still_deterministic(
    conn: sqlite3.Connection,
    profile_id: str,
    row: Mapping[str, Any],
    from_tx: Mapping[str, Any] | None,
    to_tx: Mapping[str, Any],
) -> bool:
    method = str(row["method"] or "")
    if method == "same_external_id":
        return _same_external_id_still_deterministic(conn, profile_id, row, from_tx, to_tx)
    if method == "transaction_pair":
        return _transaction_pair_still_deterministic(conn, profile_id, row, from_tx, to_tx)
    if method.startswith("provider_"):
        return _provider_key_still_deterministic(conn, profile_id, row, from_tx, to_tx)
    return False


def _target_scoped_link_rows(
    conn: sqlite3.Connection,
    profile_id: str,
    target_transaction_id: str,
) -> list[Mapping[str, Any]]:
    by_id: dict[str, Mapping[str, Any]] = {}
    visited_transactions = set()
    queue = deque([target_transaction_id])
    while queue:
        tx_id = queue.popleft()
        if tx_id in visited_transactions:
            continue
        visited_transactions.add(tx_id)
        rows = conn.execute(
            """
            SELECT *
            FROM source_funds_links
            WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
            ORDER BY created_at ASC, id ASC
            """,
            (profile_id, tx_id),
        ).fetchall()
        for row in rows:
            by_id.setdefault(row["id"], row)
            if row["from_transaction_id"]:
                queue.append(row["from_transaction_id"])
    return list(by_id.values())


def _validated_bulk_review_candidates(
    conn: sqlite3.Connection,
    profile_id: str,
    rows: Sequence[Mapping[str, Any]],
) -> list[Mapping[str, Any]]:
    candidates: list[Mapping[str, Any]] = []
    for row in rows:
        if not _is_bulk_reviewable_suggestion(row):
            continue
        to_tx = _transaction_by_id(conn, profile_id, row["to_transaction_id"])
        if not to_tx:
            continue
        from_tx = (
            _transaction_by_id(conn, profile_id, row["from_transaction_id"])
            if row["from_transaction_id"]
            else None
        )
        if not _suggestion_still_deterministic(conn, profile_id, row, from_tx, to_tx):
            continue
        try:
            _validate_transaction_link_for_review(
                link_type=row["link_type"],
                from_tx=from_tx,
                to_tx=to_tx,
                asset=row["asset"],
                from_asset=row["from_asset"],
                allocation_msat=row["allocation_amount"],
                from_allocation_msat=row["from_allocation_amount"],
            )
        except AppError:
            continue
        candidates.append(row)
    return candidates


def bulk_review_suggestions(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    target_transaction_ref: str,
) -> dict[str, Any]:
    """Accept deterministic source-funds suggestions as user-reviewed links.

    This is intentionally narrow: exact external-id matches, already-reviewed
    transaction_pairs, and one-to-one provider/import ids can be accepted in
    bulk. Broad account ids, weak time/amount guesses, and chain-observation
    hints stay manual.
    """
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    target = hooks.resolve_transaction(conn, profile["id"], target_transaction_ref)
    rows = [
        row
        for row in _target_scoped_link_rows(conn, profile["id"], target["id"])
        if row["state"] == "suggested"
    ]
    reviewable = _validated_bulk_review_candidates(conn, profile["id"], rows)
    now = _now()
    for row in reviewable:
        conn.execute(
            """
            UPDATE source_funds_links
            SET state = 'reviewed', allocation_policy = 'explicit', updated_at = ?
            WHERE id = ?
            """,
            (now, row["id"]),
        )
    conn.commit()
    reviewed_rows = [
        conn.execute("SELECT * FROM source_funds_links WHERE id = ?", (row["id"],)).fetchone()
        for row in reviewable
    ]
    return {
        "reviewed": len(reviewed_rows),
        "skipped": len(rows) - len(reviewed_rows),
        "target_transaction_id": target["id"],
        "links": [_link_row_to_dict(conn, row) for row in reviewed_rows if row],
        "policy": (
            "Bulk review only accepts exact/strong deterministic suggestions from same external ids, "
            "existing transaction_pairs, or one-to-one per-transaction provider/import ids. "
            "Weak time/amount matches, broad provider ids, and chain observations remain manual review items."
        ),
    }


def _active_transaction_rows(conn: sqlite3.Connection, profile_id: str):
    return conn.execute(
        """
        SELECT t.*, w.label AS wallet_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile_id,),
    ).fetchall()


def _insert_suggestion(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    *,
    from_tx: Mapping[str, Any],
    to_tx: Mapping[str, Any],
    link_type: str,
    method: str,
    confidence: str,
    allocation_msat: int,
    from_allocation_msat: int | None,
    explanation: str,
):
    existing = _find_existing_link(
        conn,
        profile_id,
        from_source_id=None,
        from_transaction_id=from_tx["id"],
        to_transaction_id=to_tx["id"],
        method=method,
        link_type=link_type,
    )
    if existing:
        return None
    created_at = _now()
    link_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO source_funds_links(
            id, workspace_id, profile_id, from_transaction_id, to_transaction_id,
            link_type, state, confidence, method, asset, allocation_amount,
            from_asset, from_allocation_amount, allocation_policy, explanation,
            uses_chain_observation, chain_data_confirmed, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, 'suggested', ?, ?, ?, ?, ?, ?, 'heuristic', ?, 0, 1, ?, ?)
        """,
        (
            link_id,
            workspace_id,
            profile_id,
            from_tx["id"],
            to_tx["id"],
            link_type,
            confidence,
            method,
            to_tx["asset"],
            allocation_msat,
            from_tx["asset"],
            from_allocation_msat,
            explanation,
            created_at,
            created_at,
        ),
    )
    return conn.execute("SELECT * FROM source_funds_links WHERE id = ?", (link_id,)).fetchone()


def _raw_evidence_values(row: Mapping[str, Any]) -> list[tuple[str, str]]:
    payload = _safe_json_loads(row["raw_json"])
    if not isinstance(payload, dict):
        return []
    values = []
    for key in PROVIDER_EVIDENCE_KEYS:
        value = payload.get(key)
        if value not in (None, ""):
            values.append((key, str(value)))
    return values


def _target_scoped_transaction_ids(
    conn: sqlite3.Connection,
    profile_id: str,
    target_transaction_id: str,
) -> set[str]:
    found: set[str] = {target_transaction_id}
    queue = deque([target_transaction_id])
    while queue:
        tx_id = queue.popleft()
        rows = conn.execute(
            """
            SELECT from_transaction_id
            FROM source_funds_links
            WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
            """,
            (profile_id, tx_id),
        ).fetchall()
        for row in rows:
            from_tx_id = row["from_transaction_id"]
            if from_tx_id and from_tx_id not in found:
                found.add(from_tx_id)
                queue.append(from_tx_id)
    return found


def suggest_links(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    target_transaction_ref: str | None = None,
    include_broad_hints: bool = False,
    max_suggestions: int = SUGGESTION_WRITE_CAP,
) -> dict[str, Any]:
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    target = hooks.resolve_transaction(conn, profile["id"], target_transaction_ref) if target_transaction_ref else None
    if max_suggestions <= 0:
        raise AppError("--max-suggestions must be positive", code="validation")
    # SUGGESTION_WRITE_CAP is an absolute ceiling, not a default. A caller
    # passing a larger value (CLI --max-suggestions, daemon args) would
    # otherwise let provider-id / time-amount Cartesian products spam the
    # link table. Clamp here so every callsite is covered by the same
    # ceiling regardless of how the request reaches us.
    if max_suggestions > SUGGESTION_WRITE_CAP:
        max_suggestions = SUGGESTION_WRITE_CAP
    rows = _active_transaction_rows(conn, profile["id"])
    rows_by_id = {row["id"]: row for row in rows}
    scoped_tx_ids = (
        _target_scoped_transaction_ids(conn, profile["id"], target["id"])
        if target
        else set(rows_by_id)
    )
    inserted = []

    def in_scope(*txs: Mapping[str, Any]) -> bool:
        return not target or any(tx["id"] in scoped_tx_ids for tx in txs)

    def remember(link: Mapping[str, Any] | None) -> None:
        if not link:
            return
        inserted.append(link)
        if len(inserted) > max_suggestions:
            conn.rollback()
            raise AppError(
                "source-funds suggestion write cap exceeded",
                code="validation",
                hint=(
                    "Run suggestions for a narrower --target-transaction, review or reject existing "
                    "suggestions, then try again."
                ),
                details={"max_suggestions": max_suggestions},
            )
        if target:
            scoped_tx_ids.add(link["from_transaction_id"])
            scoped_tx_ids.add(link["to_transaction_id"])

    by_external = defaultdict(list)
    for row in rows:
        if row["external_id"]:
            by_external[(row["external_id"], row["asset"])].append(row)
    for group in by_external.values():
        outs = [row for row in group if row["direction"] == "outbound"]
        ins = [row for row in group if row["direction"] == "inbound"]
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_tx, in_tx = outs[0], ins[0]
        if out_tx["wallet_id"] == in_tx["wallet_id"]:
            continue
        if not in_scope(out_tx, in_tx):
            continue
        link = _insert_suggestion(
            conn,
            workspace["id"],
            profile["id"],
            from_tx=out_tx,
            to_tx=in_tx,
            link_type="self_transfer",
            method="same_external_id",
            confidence="exact",
            allocation_msat=int(in_tx["amount"]),
            from_allocation_msat=int(out_tx["amount"]),
            explanation="Same external transaction id appears as an outbound and inbound row in two owned wallets.",
        )
        remember(link)

    pair_rows = conn.execute(
        "SELECT * FROM transaction_pairs WHERE profile_id = ? AND deleted_at IS NULL "
        "ORDER BY created_at ASC, id ASC",
        (profile["id"],),
    ).fetchall()
    for pair in pair_rows:
        out_tx = rows_by_id.get(pair["out_transaction_id"])
        in_tx = rows_by_id.get(pair["in_transaction_id"])
        if not out_tx or not in_tx:
            continue
        if not in_scope(out_tx, in_tx):
            continue
        link_type = "self_transfer" if out_tx["asset"] == in_tx["asset"] else "swap"
        link = _insert_suggestion(
            conn,
            workspace["id"],
            profile["id"],
            from_tx=out_tx,
            to_tx=in_tx,
            link_type=link_type,
            method="transaction_pair",
            confidence="strong",
            allocation_msat=int(in_tx["amount"]),
            from_allocation_msat=int(out_tx["amount"]),
            explanation=f"Existing reviewed transaction_pair ({pair['kind']}, {pair['policy']}) links these rows.",
        )
        remember(link)

    by_provider_key = defaultdict(list)
    for row in rows:
        for key, value in _raw_evidence_values(row):
            by_provider_key[(key, value)].append(row)
    for (key, value), group in by_provider_key.items():
        if len(group) < 2:
            continue
        outs = [row for row in group if row["direction"] == "outbound"]
        ins = [row for row in group if row["direction"] == "inbound"]
        method = _normalize_provider_method(key)
        is_unique_key = key in PROVIDER_UNIQUE_KEYS
        if not is_unique_key and not include_broad_hints:
            continue
        is_one_to_one = len(outs) == 1 and len(ins) == 1
        for out_tx in outs:
            for in_tx in ins:
                if out_tx["id"] == in_tx["id"]:
                    continue
                if not in_scope(out_tx, in_tx):
                    continue
                same_asset = normalize_asset_code(out_tx["asset"]) == normalize_asset_code(in_tx["asset"])
                amount_close = _same_asset_amount_close(out_tx, in_tx) if same_asset else False
                confidence = "strong" if is_unique_key and is_one_to_one and amount_close else "weak"
                link = _insert_suggestion(
                    conn,
                    workspace["id"],
                    profile["id"],
                    from_tx=out_tx,
                    to_tx=in_tx,
                    link_type="trade" if out_tx["asset"] == in_tx["asset"] else "swap",
                    method=method,
                    confidence=confidence,
                    allocation_msat=int(in_tx["amount"]),
                    from_allocation_msat=int(out_tx["amount"]),
                    explanation=(
                        f"Both imports carry {key}={value}."
                        + (" One-to-one amount match." if confidence == "strong" else " Manual review required.")
                    ),
                )
                remember(link)

    if include_broad_hints:
        for out_tx in [row for row in rows if row["direction"] == "outbound"]:
            out_time = str(out_tx["occurred_at"])
            for in_tx in [row for row in rows if row["direction"] == "inbound" and row["asset"] == out_tx["asset"]]:
                if out_tx["wallet_id"] == in_tx["wallet_id"] or out_tx["id"] == in_tx["id"]:
                    continue
                if not in_scope(out_tx, in_tx):
                    continue
                if out_time[:10] != str(in_tx["occurred_at"])[:10]:
                    continue
                if abs(int(out_tx["amount"]) - int(in_tx["amount"])) > max(1000, int(in_tx["amount"]) // 100):
                    continue
                link = _insert_suggestion(
                    conn,
                    workspace["id"],
                    profile["id"],
                    from_tx=out_tx,
                    to_tx=in_tx,
                    link_type="self_transfer",
                    method="tight_time_amount_match",
                    confidence="weak",
                    allocation_msat=int(in_tx["amount"]),
                    from_allocation_msat=int(out_tx["amount"]),
                    explanation="Same-day same-asset amount match across owned wallets; review before using as evidence.",
                )
                remember(link)

    conn.commit()
    links = [_link_row_to_dict(conn, row) for row in inserted]
    return {
        "inserted": len(links),
        "target_transaction_id": target["id"] if target else None,
        "links": links,
        "privacy_warning": (
            "No chain backend was queried. If public Esplora/Electrum observations are added later, "
            "the queried txids reveal the target path to that backend."
        ),
    }


def _reachable_link_ids(conn: sqlite3.Connection, profile_id: str, target_transaction_id: str) -> set[str]:
    found: set[str] = set()
    queue = deque([target_transaction_id])
    visited: set[str] = set()
    while queue:
        tx_id = queue.popleft()
        if tx_id in visited:
            continue
        visited.add(tx_id)
        rows = conn.execute(
            """
            SELECT id, from_transaction_id
            FROM source_funds_links
            WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
            """,
            (profile_id, tx_id),
        ).fetchall()
        for row in rows:
            found.add(row["id"])
            if row["from_transaction_id"]:
                queue.append(row["from_transaction_id"])
    return found


def _tx_node(
    row: Mapping[str, Any],
    reveal_mode: str,
    required_msat: int | None,
    *,
    is_target: bool = False,
) -> dict[str, Any]:
    # ``id`` and ``transaction_id`` carry the Kassiber-internal UUID at
    # every reveal mode. The UUID is random per database, has no
    # relationship to on-chain data, and the recipient already knows
    # the disclosure came from this user, so it is treated as a
    # non-secret graph identifier rather than disclosure surface.
    # ``internal_transaction_id`` is kept gated on ``full`` only so the
    # original (renamed-from) row id stays available for editor flows
    # without becoming a redundant copy at lower modes.
    free_text_visible = reveal_mode in {"standard", "full"}
    node = {
        "id": f"tx:{row['id']}",
        "node_type": "transaction",
        "transaction_id": row["id"],
        "label": _tx_label(row, reveal_mode, is_target=is_target),
        "wallet": row["wallet_label"],
        "occurred_at": row["occurred_at"],
        "direction": row["direction"],
        "asset": row["asset"],
        "amount": _btc_value(row["amount"]),
        "amount_msat": int(row["amount"]),
        "required_amount": _btc_value(required_msat),
        "required_amount_msat": required_msat,
        "external_id": _public_tx_id(row, reveal_mode, is_target=is_target),
        "fiat_currency": row["fiat_currency"] or "",
        "fiat_value": row["fiat_value"],
        "pricing_source_kind": row["pricing_source_kind"] or row["fiat_price_source"] or "",
        "description": row["description"] or "" if free_text_visible else "",
        "counterparty": row["counterparty"] or "" if free_text_visible else "",
    }
    if reveal_mode == "full":
        node["internal_transaction_id"] = row["id"]
    return node


def _source_node(source: Mapping[str, Any], reveal_mode: str, required_msat: int | None) -> dict[str, Any]:
    label = source["label"] if reveal_mode != "labels_only" else source["source_type"].replace("_", " ")
    free_text_visible = reveal_mode in {"standard", "full"}
    return {
        "id": f"source:{source['id']}",
        "node_type": "source",
        "source_id": source["id"],
        "source_type": source["source_type"],
        "label": label,
        "asset": source["asset"],
        "amount": _btc_value(source["amount"]),
        "amount_msat": source["amount"],
        "required_amount": _btc_value(required_msat),
        "required_amount_msat": required_msat,
        "fiat_currency": source["fiat_currency"] or "",
        "fiat_value": source["fiat_value"],
        "acquired_at": source["acquired_at"] or "",
        "description": source["description"] or "" if free_text_visible else "",
        "review_state": source["review_state"],
    }


def _finding(code: str, severity: str, message: str, *, ref: str | None = None) -> dict[str, Any]:
    return {"code": code, "severity": severity, "message": message, "ref": ref or ""}


def _add_finding(findings: list[dict[str, Any]], code: str, severity: str, message: str, *, ref: str | None = None):
    item = _finding(code, severity, message, ref=ref)
    if item not in findings:
        findings.append(item)


def _node_event_time(node: Mapping[str, Any]) -> str:
    return str(node.get("occurred_at") or node.get("acquired_at") or "")


def _summarize_report_data_sources(nodes: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str], dict[str, Any]] = {}
    for node in nodes:
        if node.get("node_type") == "transaction":
            label = str(node.get("wallet") or "Unlabeled wallet")
            kind = "wallet"
            transaction_count = 1
            source_count = 0
        else:
            label = str(node.get("label") or _label(node.get("source_type")) or "Reviewed source")
            kind = str(node.get("source_type") or "source")
            transaction_count = 0
            source_count = 1
        key = (kind, label)
        item = grouped.setdefault(
            key,
            {
                "label": label,
                "kind": kind,
                "transaction_count": 0,
                "source_count": 0,
                "assets": set(),
                "first_seen": "",
                "last_seen": "",
            },
        )
        item["transaction_count"] += transaction_count
        item["source_count"] += source_count
        if node.get("asset"):
            item["assets"].add(str(node["asset"]))
        event_time = _node_event_time(node)
        if event_time:
            item["first_seen"] = min([item["first_seen"], event_time]) if item["first_seen"] else event_time
            item["last_seen"] = max([item["last_seen"], event_time]) if item["last_seen"] else event_time
    rows: list[dict[str, Any]] = []
    for item in grouped.values():
        row = dict(item)
        row["assets"] = sorted(row["assets"])
        rows.append(row)
    return sorted(rows, key=lambda row: (row["kind"], row["label"]))


def _compact_flow_node(node: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "id": node.get("id", ""),
        "node_type": node.get("node_type", ""),
        "label": node.get("label", ""),
        "wallet": node.get("wallet", ""),
        "source_type": node.get("source_type", ""),
        "asset": node.get("asset", ""),
        "required_amount": node.get("required_amount"),
        "required_amount_msat": node.get("required_amount_msat"),
        "amount": node.get("amount"),
        "amount_msat": node.get("amount_msat"),
        "occurred_at": node.get("occurred_at", ""),
        "acquired_at": node.get("acquired_at", ""),
        "external_id": node.get("external_id", ""),
        "fiat_currency": node.get("fiat_currency", ""),
        "fiat_value": node.get("fiat_value"),
        "review_state": node.get("review_state", ""),
    }


def _build_flow_levels(report: Mapping[str, Any]) -> list[dict[str, Any]]:
    graph = report.get("graph") or {}
    nodes = list(graph.get("nodes") or [])
    edges = list(graph.get("edges") or [])
    node_by_id = {str(node.get("id")): node for node in nodes if node.get("id")}
    target = report.get("target") or {}
    target_id = str(target.get("id") or "")
    if not target_id:
        return []
    parents_by_child: dict[str, list[str]] = defaultdict(list)
    for edge in edges:
        parent = str(edge.get("from") or "")
        child = str(edge.get("to") or "")
        if parent and child:
            parents_by_child[child].append(parent)

    depths: dict[str, int] = {target_id: 0}
    queue = deque([target_id])
    while queue:
        child = queue.popleft()
        for parent in parents_by_child.get(child, []):
            next_depth = depths[child] + 1
            if parent not in depths or next_depth < depths[parent]:
                depths[parent] = next_depth
                queue.append(parent)

    fallback_depth = max(depths.values() or [0]) + 1
    for node_id in node_by_id:
        depths.setdefault(node_id, fallback_depth)

    levels: dict[int, list[Mapping[str, Any]]] = defaultdict(list)
    for node_id, depth in depths.items():
        node = node_by_id.get(node_id)
        if node:
            levels[depth].append(node)

    rows: list[dict[str, Any]] = []
    for depth in sorted(levels):
        level_nodes = sorted(
            levels[depth],
            key=lambda node: (
                node.get("node_type") == "source",
                _node_event_time(node),
                str(node.get("label") or ""),
            ),
        )
        rows.append(
            {
                "level": depth + 1,
                "role": "target" if depth == 0 else "upstream",
                "nodes": [_compact_flow_node(node) for node in level_nodes],
                "transaction_count": sum(1 for node in level_nodes if node.get("node_type") == "transaction"),
                "source_count": sum(1 for node in level_nodes if node.get("node_type") == "source"),
            }
        )
    return rows


def _compact_simplified_flow_node(
    node: Mapping[str, Any],
    *,
    deferred_privacy_hop: bool = False,
) -> dict[str, Any]:
    kind = node.get("source_type") or node.get("direction") or node.get("node_type") or ""
    return {
        "id": node.get("id", ""),
        "node_type": node.get("node_type", ""),
        "kind": kind,
        "label": node.get("label", ""),
        "wallet": node.get("wallet", ""),
        "asset": node.get("asset", ""),
        "amount": node.get("required_amount") if node.get("required_amount") is not None else node.get("amount"),
        "amount_msat": (
            node.get("required_amount_msat")
            if node.get("required_amount_msat") is not None
            else node.get("amount_msat")
        ),
        "occurred_at": node.get("occurred_at") or node.get("acquired_at") or "",
        "deferred_privacy_hop": deferred_privacy_hop,
    }


def _build_simplified_flow(report: Mapping[str, Any]) -> dict[str, Any]:
    """Build a compact chart model from reviewed graph edges.

    The chart follows reviewed source, self-transfer, consolidation, trade, and
    swap-style links. CoinJoin/PayJoin traversal is intentionally deferred:
    those links become boundary nodes so the PDF never implies that Kassiber can
    prove ownership through unrelated participant inputs.
    """
    graph = report.get("graph") or {}
    nodes = list(graph.get("nodes") or [])
    edges = list(graph.get("edges") or [])
    node_by_id = {str(node.get("id")): node for node in nodes if node.get("id")}
    target = report.get("target") or {}
    target_id = str(target.get("id") or "")
    if not target_id or target_id not in node_by_id:
        return {
            "levels": [],
            "edges": [],
            "deferred_privacy_hops": [],
            "note": "",
        }

    parents_by_child: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for edge in edges:
        parent = str(edge.get("from") or "")
        child = str(edge.get("to") or "")
        if parent and child:
            parents_by_child[child].append(edge)

    included: set[str] = {target_id}
    deferred_nodes: set[str] = set()
    deferred_hops: list[dict[str, Any]] = []
    depths: dict[str, int] = {target_id: 0}
    queue = deque([target_id])
    traversed_edges: set[str] = set()

    while queue:
        child = queue.popleft()
        child_depth = depths[child]
        for edge in parents_by_child.get(child, []):
            edge_id = str(edge.get("id") or "")
            if edge_id in traversed_edges:
                continue
            traversed_edges.add(edge_id)
            parent = str(edge.get("from") or "")
            if parent not in node_by_id:
                continue
            next_depth = child_depth + 1
            if parent not in depths or next_depth < depths[parent]:
                depths[parent] = next_depth
            included.add(parent)
            if edge.get("link_type") in PRIVACY_LINK_TYPES:
                deferred_nodes.add(parent)
                deferred_hops.append(
                    {
                        "link_id": edge.get("id", ""),
                        "link_type": edge.get("link_type", ""),
                        "from": parent,
                        "to": child,
                        "message": (
                            "CoinJoin/PayJoin traversal is deferred; this reviewed hop is "
                            "shown as a privacy boundary, not as ownership proof through "
                            "unrelated participant inputs."
                        ),
                    }
                )
                continue
            if parent not in deferred_nodes:
                queue.append(parent)

    levels_by_depth: dict[int, list[str]] = defaultdict(list)
    for node_id in included:
        levels_by_depth[depths.get(node_id, 0)].append(node_id)

    levels: list[dict[str, Any]] = []
    max_depth = max(levels_by_depth)
    for depth in sorted(levels_by_depth, reverse=True):
        level_nodes = sorted(
            levels_by_depth[depth],
            key=lambda node_id: (
                node_by_id[node_id].get("node_type") == "source",
                _node_event_time(node_by_id[node_id]),
                str(node_by_id[node_id].get("label") or ""),
            ),
        )
        role = "target" if depth == 0 else "upstream"
        if all(node_by_id[node_id].get("node_type") == "source" for node_id in level_nodes):
            role = "source"
        if any(node_id in deferred_nodes for node_id in level_nodes):
            role = "privacy_boundary" if role != "target" else role
        levels.append(
            {
                "level": max_depth - depth + 1,
                "distance_to_target": depth,
                "role": role,
                "nodes": [
                    _compact_simplified_flow_node(
                        node_by_id[node_id],
                        deferred_privacy_hop=node_id in deferred_nodes,
                    )
                    for node_id in level_nodes
                ],
            }
        )

    simplified_edges = []
    for edge in edges:
        parent = str(edge.get("from") or "")
        child = str(edge.get("to") or "")
        if parent in included and child in included:
            simplified_edges.append(
                {
                    "id": edge.get("id", ""),
                    "from": parent,
                    "to": child,
                    "link_type": edge.get("link_type", ""),
                    "asset": edge.get("asset", ""),
                    "amount": edge.get("allocation_amount"),
                    "amount_msat": edge.get("allocation_amount_msat"),
                    "deferred_privacy_hop": edge.get("link_type") in PRIVACY_LINK_TYPES,
                }
            )

    note = (
        "Simplified flow follows reviewed local links. Wallet transfers and "
        "consolidations are shown as reviewed hops; CoinJoin/PayJoin traversal "
        "is deferred and shown only as a privacy boundary."
    )
    return {
        "levels": levels,
        "edges": simplified_edges,
        "deferred_privacy_hops": deferred_hops,
        "note": note,
    }


def _source_mix_phrase(source_mix: Sequence[Mapping[str, Any]], asset: str) -> str:
    if not source_mix:
        return "no reviewed root sources"
    parts = []
    for row in source_mix[:4]:
        amount = _btc_value(row.get("amount_msat"))
        percent = row.get("percent_of_target")
        suffix = f", {float(percent):.1f}% of target" if percent is not None else ""
        parts.append(f"{_label(row.get('source_type'))}: {amount:.8f} {asset}{suffix}")
    if len(source_mix) > 4:
        parts.append(f"{len(source_mix) - 4} more source categories")
    return "; ".join(parts)


def _add_report_shape(envelope: dict[str, Any]) -> None:
    graph = envelope.get("graph") or {}
    nodes = list(graph.get("nodes") or [])
    edges = list(graph.get("edges") or [])
    tx_nodes = [node for node in nodes if node.get("node_type") == "transaction"]
    source_nodes = [node for node in nodes if node.get("node_type") == "source"]
    dates = sorted(_node_event_time(node) for node in nodes if _node_event_time(node))
    target = envelope.get("target") or {}
    target_asset = str(target.get("asset") or envelope.get("allocations", {}).get("asset") or "")
    data_sources = _summarize_report_data_sources(nodes)
    source_mix = list(envelope.get("source_mix") or [])
    blocker_count = len((envelope.get("explain_gates") or {}).get("blockers") or [])
    warning_count = len((envelope.get("explain_gates") or {}).get("warnings") or [])

    envelope["overview"] = {
        "target_label": target.get("label", ""),
        "target_asset": target_asset,
        "target_amount": target.get("required_amount"),
        "target_amount_msat": target.get("required_amount_msat"),
        "target_fiat_value": target.get("fiat_value"),
        "target_fiat_currency": target.get("fiat_currency", ""),
        "target_date": target.get("occurred_at", ""),
        "target_wallet": target.get("wallet", ""),
        "time_range": {
            "start": dates[0] if dates else "",
            "end": dates[-1] if dates else "",
        },
        "transaction_count": len(tx_nodes),
        "link_count": len(edges),
        "root_source_count": len(source_nodes),
        "source_category_count": len(source_mix),
        "data_source_count": len(data_sources),
        "blocker_count": blocker_count,
        "warning_count": warning_count,
    }
    envelope["data_sources"] = data_sources
    envelope["flow_levels"] = _build_flow_levels(envelope)
    envelope["simplified_flow"] = _build_simplified_flow(envelope)
    envelope["narrative"] = {
        "generated_by": "local_rule_summary",
        "paragraphs": [
            (
                f"This report traces {target.get('required_amount', 0):.8f} {target_asset} for "
                f"{target.get('label') or 'the selected target'} through {len(edges)} reviewed "
                f"link{'' if len(edges) == 1 else 's'} across {len(tx_nodes)} transaction"
                f"{'' if len(tx_nodes) == 1 else 's'}."
            ),
            (
                f"The reviewed source mix is {_source_mix_phrase(source_mix, target_asset)}. "
                f"The path spans {envelope['overview']['time_range']['start'] or 'unknown start'} "
                f"to {envelope['overview']['time_range']['end'] or 'unknown end'} and uses "
                f"{len(data_sources)} local data source{'' if len(data_sources) == 1 else 's'}."
            ),
            (
                "Export is currently clear under the current review gates."
                if blocker_count == 0
                else f"Export is blocked by {blocker_count} unresolved review gate"
                f"{'' if blocker_count == 1 else 's'}."
            ),
        ],
    }


def build_report(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    target_transaction_ref: str,
    target_amount: Any = None,
    report_purpose: str = "existing_transaction",
    planned_destination: str | None = None,
    planned_note: str | None = None,
    reveal_mode: str | None = None,
    max_depth: int = 8,
    save_case: bool = False,
    case_label: str | None = None,
    recipient_ref: str | None = None,
) -> dict[str, Any]:
    if max_depth <= 0:
        max_depth = 1
    elif max_depth > _MAX_BUILD_REPORT_DEPTH:
        max_depth = _MAX_BUILD_REPORT_DEPTH
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    target = hooks.resolve_transaction(conn, profile["id"], target_transaction_ref)
    from .source_funds_recipients import effective_reveal_mode
    resolved_mode, recipient = effective_reveal_mode(
        conn,
        profile["id"],
        explicit_reveal_mode=reveal_mode if reveal_mode is not None and reveal_mode != "" else None,
        recipient_ref=recipient_ref,
    )
    mode = _normalize_reveal_mode(resolved_mode)
    purpose = _normalize_report_purpose(report_purpose)
    target_amount_msat = _amount_msat(target_amount, label="--target-amount") if target_amount not in (None, "") else int(target["amount"])
    if target_amount_msat <= 0:
        raise AppError("target amount must be positive", code="validation")
    destination = (planned_destination or "").strip()
    note = (planned_note or "").strip()

    nodes: dict[str, dict[str, Any]] = {}
    edges: list[dict[str, Any]] = []
    findings: list[dict[str, Any]] = []
    source_mix = defaultdict(lambda: {"amount_msat": 0, "count": 0})
    source_consumption_msat = defaultdict(int)
    disclosure_txids: set[str] = set()
    explorer_links_by_txid: dict[str, dict[str, Any]] = {}
    disclosure_attachments: dict[str, dict[str, Any]] = {}
    visited: set[str] = set()
    queued: set[str] = {target["id"]}
    tx_requirements_msat = defaultdict(int)
    tx_required_assets = {target["id"]: normalize_asset_code(target["asset"])}
    tx_depths = {target["id"]: 0}
    tx_paths = {target["id"]: (target["id"],)}
    tx_requirements_msat[target["id"]] = target_amount_msat
    queue = deque([target["id"]])
    truncated_by_size = False

    def _graph_over_budget() -> bool:
        return (
            len(nodes) >= _MAX_BUILD_REPORT_NODES
            or len(edges) >= _MAX_BUILD_REPORT_EDGES
        )

    def _emit_size_truncation() -> None:
        nonlocal truncated_by_size
        if truncated_by_size:
            return
        _add_finding(
            findings,
            "path_truncated",
            "blocker",
            "Source-funds graph exceeded the per-report node/edge cap; review the upstream "
            "suggestions or split the disclosure into a narrower target before retrying.",
            ref=target["id"],
        )
        truncated_by_size = True

    while queue:
        if truncated_by_size or _graph_over_budget():
            _emit_size_truncation()
            break
        tx_id = queue.popleft()
        queued.discard(tx_id)
        required_msat = int(tx_requirements_msat[tx_id])
        required_asset = tx_required_assets[tx_id]
        depth = tx_depths[tx_id]
        path = tx_paths[tx_id]
        tx = _transaction_by_id(conn, profile["id"], tx_id)
        if tx is None:
            _add_finding(findings, "missing_history", "blocker", "Referenced transaction is no longer present.", ref=tx_id)
            continue
        node_id = f"tx:{tx_id}"
        is_target_tx = tx_id == target["id"]
        nodes[node_id] = _tx_node(tx, mode, required_msat, is_target=is_target_tx)
        disclosed_txid = _public_tx_id(tx, mode, is_target=is_target_tx)
        if disclosed_txid:
            disclosure_txids.add(disclosed_txid)
            explorer_link = _public_explorer_link(
                disclosed_txid,
                tx["asset"],
                tx["wallet_config_json"],
            )
            if explorer_link:
                explorer_links_by_txid[disclosed_txid] = explorer_link
        if tx["fiat_value"] is None and tx["fiat_rate"] is None:
            _add_finding(
                findings,
                "missing_pricing",
                "blocker",
                "A transaction on the disclosed path has no fiat pricing.",
                ref=tx_id,
            )
        if normalize_asset_code(required_asset) != normalize_asset_code(tx["asset"]):
            _add_finding(
                findings,
                "asset_mismatch",
                "blocker",
                "A reviewed path declares a different asset than the transaction being consumed.",
                ref=tx_id,
            )
        if required_msat > int(tx["amount"]):
            _add_finding(
                findings,
                "transaction_overallocation",
                "blocker",
                "A reviewed path requires more value than this transaction holds.",
                ref=tx_id,
            )
        if depth >= max_depth:
            _add_finding(findings, "path_truncated", "blocker", "Maximum source-funds path depth reached.", ref=tx_id)
            continue
        if tx_id in visited:
            continue
        visited.add(tx_id)
        link_rows = conn.execute(
            """
            SELECT *
            FROM source_funds_links
            WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
            ORDER BY state DESC, created_at ASC, id ASC
            """,
            (profile["id"], tx_id),
        ).fetchall()
        suggestions = [row for row in link_rows if row["state"] == "suggested"]
        for suggestion in suggestions:
            _add_finding(
                findings,
                "unreviewed_link",
                "blocker",
                "A suggested source-funds link still needs review before export.",
                ref=suggestion["id"],
            )
        reviewed = [row for row in link_rows if row["state"] == "reviewed"]
        reviewed_total = 0
        if not reviewed:
            _add_finding(
                findings,
                "missing_history",
                "blocker",
                "The path stops at a transaction without a reviewed root source or missing-history attestation.",
                ref=tx_id,
            )
            continue
        for link in reviewed:
            if _graph_over_budget():
                _emit_size_truncation()
                break
            allocation_msat = link["allocation_amount"]
            if allocation_msat is None:
                _add_finding(
                    findings,
                    "ambiguous_allocation",
                    "blocker",
                    "A reviewed link is missing an explicit allocation amount.",
                    ref=link["id"],
                )
                continue
            reviewed_total += int(allocation_msat)
            if link["allocation_policy"] != "explicit":
                _add_finding(
                    findings,
                    "ambiguous_allocation",
                    "blocker",
                    "A reviewed link still uses a heuristic or unknown allocation policy.",
                    ref=link["id"],
                )
            if link["uses_chain_observation"] and not link["chain_data_confirmed"]:
                _add_finding(
                    findings,
                    "unconfirmed_chain_data",
                    "blocker",
                    "Unconfirmed chain observations cannot be used as proof in a PDF export.",
                    ref=link["id"],
                )
            if link["method"] == "chain_observation" or link["uses_chain_observation"]:
                _add_finding(
                    findings,
                    "chain_observation_privacy",
                    "warning",
                    "Chain-backend observations are context only and do not prove ownership.",
                    ref=link["id"],
                )
            if link["link_type"] in PRIVACY_LINK_TYPES:
                _add_finding(
                    findings,
                    "privacy_hop_unresolved",
                    "warning",
                    "Privacy-hop links require explicit supporting evidence; unrelated participant inputs are not shown.",
                    ref=link["id"],
                )
            attachment_rows = conn.execute(
                """
                SELECT a.*
                FROM source_funds_link_attachments lfa
                JOIN attachments a ON a.id = lfa.attachment_id
                WHERE lfa.link_id = ?
                """,
                (link["id"],),
            ).fetchall()
            for attachment in attachment_rows:
                disclosure_attachments[attachment["id"]] = _attachment_summary(attachment, mode)
            if link["from_source_id"]:
                source = conn.execute(
                    "SELECT * FROM source_funds_sources WHERE id = ?",
                    (link["from_source_id"],),
                ).fetchone()
                if not source:
                    _add_finding(findings, "missing_history", "blocker", "Reviewed source record is missing.", ref=link["id"])
                    continue
                source_required = int(link["from_allocation_amount"] or allocation_msat)
                link_from_asset = normalize_asset_code(link["from_asset"] or link["asset"])
                if normalize_asset_code(source["asset"]) != link_from_asset:
                    _add_finding(
                        findings,
                        "source_asset_mismatch",
                        "blocker",
                        "A reviewed link allocates a different asset than its source record.",
                        ref=link["id"],
                    )
                if _timestamp_after(source["acquired_at"], tx["occurred_at"]):
                    _add_finding(
                        findings,
                        "chronology_violation",
                        "blocker",
                        "A reviewed source is dated after the transaction it funds.",
                        ref=link["id"],
                    )
                source_consumption_msat[source["id"]] += source_required
                if source["amount"] is None:
                    if source["source_type"] not in ATTESTATION_SOURCE_TYPES:
                        _add_finding(
                            findings,
                            "source_amount_missing",
                            "blocker",
                            "A concrete source record needs an amount before it can support export.",
                            ref=source["id"],
                        )
                elif source_consumption_msat[source["id"]] > int(source["amount"]):
                    _add_finding(
                        findings,
                        "source_overallocation",
                        "blocker",
                        "Reviewed links allocate more funds than the source record contains.",
                        ref=source["id"],
                    )
                source_node_id = f"source:{source['id']}"
                nodes[source_node_id] = _source_node(source, mode, source_required)
                if source["source_type"] == "missing_history":
                    _add_finding(
                        findings,
                        "missing_history",
                        "warning",
                        "Reviewed missing-history gap included; it is not a real root source.",
                        ref=source["id"],
                    )
                elif source["source_type"] == "opening_balance_attestation":
                    _add_finding(
                        findings,
                        "opening_balance_attestation",
                        "warning",
                        "Opening balance is an attested prior-history stop, not a fully traced root source.",
                        ref=source["id"],
                    )
                mix = source_mix[source["source_type"]]
                mix["amount_msat"] += source_required
                mix["count"] += 1
                for attachment in conn.execute(
                    """
                    SELECT a.*
                    FROM source_funds_source_attachments sfa
                    JOIN attachments a ON a.id = sfa.attachment_id
                    WHERE sfa.source_id = ?
                    """,
                    (source["id"],),
                ).fetchall():
                    disclosure_attachments[attachment["id"]] = _attachment_summary(attachment, mode)
                from_id = source_node_id
            else:
                from_tx = _transaction_by_id(conn, profile["id"], link["from_transaction_id"])
                if not from_tx:
                    _add_finding(findings, "missing_history", "blocker", "Reviewed parent transaction is missing.", ref=link["id"])
                    continue
                from_tx_id = from_tx["id"]
                parent_required = int(link["from_allocation_amount"] or allocation_msat)
                link_from_asset = normalize_asset_code(link["from_asset"] or from_tx["asset"])
                if _timestamp_after(from_tx["occurred_at"], tx["occurred_at"]):
                    _add_finding(
                        findings,
                        "chronology_violation",
                        "blocker",
                        "A reviewed parent transaction occurs after the child transaction it funds.",
                        ref=link["id"],
                    )
                if link["link_type"] == "self_transfer":
                    from_tx_asset = normalize_asset_code(from_tx["asset"])
                    to_tx_asset = normalize_asset_code(tx["asset"])
                    link_asset = normalize_asset_code(link["asset"])
                    if from_tx_asset != to_tx_asset or link_asset != to_tx_asset or link_from_asset != from_tx_asset:
                        _add_finding(
                            findings,
                            "asset_mismatch",
                            "blocker",
                            "A self-transfer link declares a different asset than its parent or target transaction.",
                            ref=link["id"],
                        )
                nodes[f"tx:{from_tx_id}"] = _tx_node(from_tx, mode, int(parent_required))
                if from_tx_id in path:
                    _add_finding(
                        findings,
                        "path_cycle",
                        "blocker",
                        "A reviewed source-funds path forms a cycle.",
                        ref=link["id"],
                    )
                else:
                    existing_asset = tx_required_assets.get(from_tx_id)
                    if existing_asset and existing_asset != link_from_asset:
                        _add_finding(
                            findings,
                            "asset_mismatch",
                            "blocker",
                            "A repeated upstream transaction is required with conflicting assets.",
                            ref=from_tx_id,
                        )
                    tx_required_assets[from_tx_id] = existing_asset or link_from_asset
                    tx_requirements_msat[from_tx_id] += parent_required
                    nodes[f"tx:{from_tx_id}"] = _tx_node(from_tx, mode, int(tx_requirements_msat[from_tx_id]))
                    if tx_requirements_msat[from_tx_id] > int(from_tx["amount"]):
                        _add_finding(
                            findings,
                            "transaction_overallocation",
                            "blocker",
                            "A reviewed path requires more value than this transaction holds.",
                            ref=from_tx_id,
                        )
                    if from_tx_id in visited:
                        _add_finding(
                            findings,
                            "ambiguous_allocation",
                            "blocker",
                            "A repeated upstream transaction received additional required amount after it was reviewed.",
                            ref=from_tx_id,
                        )
                    elif from_tx_id not in queued:
                        tx_depths[from_tx_id] = depth + 1
                        tx_paths[from_tx_id] = (*path, from_tx_id)
                        queue.append(from_tx_id)
                        queued.add(from_tx_id)
                    else:
                        tx_depths[from_tx_id] = max(tx_depths[from_tx_id], depth + 1)
                from_id = f"tx:{from_tx_id}"
            edges.append(
                {
                    "id": link["id"],
                    "from": from_id,
                    "to": node_id,
                    "link_type": link["link_type"],
                    "state": link["state"],
                    "confidence": link["confidence"],
                    "method": link["method"],
                    "asset": link["asset"],
                    "allocation_amount": _btc_value(allocation_msat),
                    "allocation_amount_msat": int(allocation_msat),
                    "from_asset": link["from_asset"] or link["asset"],
                    "from_allocation_amount": _btc_value(link["from_allocation_amount"]),
                    "from_allocation_amount_msat": link["from_allocation_amount"],
                    "allocation_policy": link["allocation_policy"],
                    "explanation": _public_explanation(link["explanation"], mode),
                    "attachments": [_attachment_summary(attachment, mode) for attachment in attachment_rows],
                }
            )
        if reviewed_total != required_msat:
            _add_finding(
                findings,
                "ambiguous_allocation",
                "blocker",
                "Reviewed allocations do not exactly cover the required amount.",
                ref=tx_id,
            )

    enrich_findings_with_next_steps(findings)
    blockers = [finding for finding in findings if finding["severity"] == "blocker"]
    warnings = [finding for finding in findings if finding["severity"] == "warning"]
    source_mix_rows = [
        {
            "source_type": source_type,
            "amount": _btc_value(values["amount_msat"]),
            "amount_msat": values["amount_msat"],
            "percent_of_target": (
                round((values["amount_msat"] / target_amount_msat) * 100, 4)
                if target_amount_msat
                else 0
            ),
            "count": values["count"],
        }
        for source_type, values in sorted(source_mix.items())
    ]
    envelope = {
        "workspace": workspace["label"],
        "profile": profile["label"],
        "report_context": _report_context(profile),
        "purpose": {
            "type": purpose,
            "label": "Planned exchange sale" if purpose == "planned_exchange_sale" else "Already completed transaction",
            "anchor_role": "funds_history_anchor" if purpose == "planned_exchange_sale" else "completed_transaction",
            "planned_destination": destination,
            "planned_note": note,
            "fiat_purchase_note": (
                "If the bitcoin was bought on an exchange, fiat-source evidence for that original purchase "
                "is a separate supporting document and should be attached to the root source."
                if purpose == "planned_exchange_sale"
                else ""
            ),
        },
        "target": _tx_node(
            {
                **_row_dict(target),
                "wallet_label": conn.execute(
                    "SELECT label FROM wallets WHERE id = ?",
                    (target["wallet_id"],),
                ).fetchone()["label"],
            },
            mode,
            target_amount_msat,
            is_target=True,
        ),
        "reveal_mode": mode,
        "graph": {
            "nodes": sorted(nodes.values(), key=lambda row: (row["node_type"], row["label"], row["id"])),
            "edges": edges,
        },
        "allocations": {
            "target_amount": _btc_value(target_amount_msat),
            "target_amount_msat": target_amount_msat,
            "asset": target["asset"],
            "reviewed_edge_count": len(edges),
        },
        "source_mix": source_mix_rows,
        "gaps": [finding for finding in findings if finding["code"] in {"missing_history", "ambiguous_allocation", "privacy_hop_unresolved", "path_truncated"}],
        "findings": findings,
        "explain_gates": {
            "exportable": not blockers,
            "blockers": blockers,
            "warnings": warnings,
        },
        "disclosure_preview": {
            "txids": sorted(disclosure_txids),
            "explorer_links": [
                explorer_links_by_txid[txid]
                for txid in sorted(explorer_links_by_txid)
            ],
            "attachments": sorted(disclosure_attachments.values(), key=lambda item: item["id"]),
            "privacy_note": (
                "Txids disclose on-chain neighbors to the recipient. Chain observations are context, not proof of ownership."
            ),
            "excluded": [
                "descriptors",
                "xpubs",
                "wallet files",
                "seeds",
                "backend tokens",
                "unrelated wallet history",
            ],
        },
    }
    if recipient is not None:
        envelope["recipient"] = {
            "id": recipient["id"],
            "label": recipient["label"],
            "kind": recipient["kind"],
            "default_reveal_mode": recipient["default_reveal_mode"],
        }
    _add_report_shape(envelope)
    if save_case:
        case = save_case_snapshot(
            conn,
            workspace["id"],
            profile["id"],
            target["id"],
            target_amount_msat,
            target["asset"],
            mode,
            "exportable" if not blockers else "blocked",
            envelope,
            label=case_label,
            target_external_id=target["external_id"] or "",
            recipient_id=recipient["id"] if recipient else None,
            recipient_label_snapshot=recipient["label"] if recipient else None,
            recipient_kind_snapshot=recipient["kind"] if recipient else None,
            recipient_reveal_mode_snapshot=recipient["default_reveal_mode"] if recipient else None,
        )
        envelope["case"] = case
    return envelope


def _snapshot_hash(snapshot: Mapping[str, Any]) -> str:
    payload = json.dumps(json_ready(snapshot), sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def save_case_snapshot(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    target_transaction_id: str,
    target_amount_msat: int,
    asset: str,
    reveal_mode: str,
    status: str,
    snapshot: Mapping[str, Any],
    *,
    label: str | None = None,
    target_external_id: str | None = None,
    recipient_id: str | None = None,
    recipient_label_snapshot: str | None = None,
    recipient_kind_snapshot: str | None = None,
    recipient_reveal_mode_snapshot: str | None = None,
) -> dict[str, Any]:
    case_id = str(uuid.uuid4())
    snapshot_id = str(uuid.uuid4())
    created_at = _now()
    snapshot_json = json.dumps(json_ready(snapshot), sort_keys=True)
    digest = _snapshot_hash(snapshot)
    conn.execute(
        """
        INSERT INTO source_funds_cases(
            id, workspace_id, profile_id, target_transaction_id, target_external_id,
            target_amount, asset, label, reveal_mode, status, snapshot_hash, snapshot_json,
            created_at, updated_at, recipient_id,
            recipient_label_snapshot, recipient_kind_snapshot, recipient_reveal_mode_snapshot
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            case_id,
            workspace_id,
            profile_id,
            target_transaction_id,
            target_external_id,
            target_amount_msat,
            asset,
            label,
            reveal_mode,
            status,
            digest,
            snapshot_json,
            created_at,
            created_at,
            recipient_id,
            recipient_label_snapshot,
            recipient_kind_snapshot,
            recipient_reveal_mode_snapshot,
        ),
    )
    conn.execute(
        """
        INSERT INTO source_funds_snapshots(id, case_id, snapshot_hash, snapshot_json, created_at)
        VALUES(?, ?, ?, ?, ?)
        """,
        (snapshot_id, case_id, digest, snapshot_json, created_at),
    )
    conn.commit()
    return {
        "id": case_id,
        "snapshot_id": snapshot_id,
        "snapshot_hash": digest,
        "status": status,
        "created_at": created_at,
    }


def list_cases(conn: sqlite3.Connection, workspace_ref: str | None, profile_ref: str | None, hooks: SourceFundsHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT *
        FROM source_funds_cases
        WHERE profile_id = ?
        ORDER BY created_at DESC, id DESC
        """,
        (profile["id"],),
    ).fetchall()
    cases: list[dict[str, Any]] = []
    for row in rows:
        keys = row.keys()
        recipient_id = row["recipient_id"] if "recipient_id" in keys else None
        # Use the snapshot fields written at save time so a later
        # rename or delete of the recipient cannot rewrite history.
        snapshot_label = row["recipient_label_snapshot"] if "recipient_label_snapshot" in keys else None
        snapshot_kind = row["recipient_kind_snapshot"] if "recipient_kind_snapshot" in keys else None
        snapshot_reveal = row["recipient_reveal_mode_snapshot"] if "recipient_reveal_mode_snapshot" in keys else None
        target_external = row["target_external_id"] if "target_external_id" in keys else None
        cases.append(
            {
                "id": row["id"],
                "label": row["label"] or "",
                "target_transaction_id": row["target_transaction_id"],
                "target_external_id": target_external or "",
                "target_amount": _btc_value(row["target_amount"]),
                "target_amount_msat": row["target_amount"],
                "asset": row["asset"],
                "reveal_mode": row["reveal_mode"],
                "status": row["status"],
                "snapshot_hash": row["snapshot_hash"],
                "recipient_id": recipient_id,
                "recipient_label": snapshot_label or "",
                "recipient_kind": snapshot_kind or "",
                "recipient_reveal_mode": snapshot_reveal or "",
                "created_at": row["created_at"],
                "updated_at": row["updated_at"],
            }
        )
    return cases


def load_case_snapshot(conn: sqlite3.Connection, workspace_ref: str | None, profile_ref: str | None, hooks: SourceFundsHooks, case_ref: str):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        "SELECT * FROM source_funds_cases WHERE profile_id = ? AND id = ?",
        (profile["id"], case_ref),
    ).fetchone()
    if not row:
        raise AppError(f"Source-funds case '{case_ref}' not found", code="not_found")
    snapshot = json.loads(row["snapshot_json"])
    if _snapshot_hash(snapshot) != row["snapshot_hash"]:
        raise AppError(
            "Source-funds case snapshot hash does not match stored payload",
            code="snapshot_integrity_error",
            retryable=False,
        )
    return snapshot


def build_report_lines(report: Mapping[str, Any], hooks: SourceFundsHooks) -> list[str]:
    target = report["target"]
    title = "Kassiber Source of Funds Report"
    lines = [title, "=" * len(title), ""]
    lines.extend(
        [
            f"Workspace:       {report['workspace']}",
            f"Profile:         {report['profile']}",
            f"Purpose:         {report.get('purpose', {}).get('label', 'Already completed transaction')}",
            f"Reveal mode:     {report['reveal_mode']}",
            f"{'Funds anchor' if report.get('purpose', {}).get('type') == 'planned_exchange_sale' else 'Target'}:          {target['label']}",
            f"{'Planned amount' if report.get('purpose', {}).get('type') == 'planned_exchange_sale' else 'Target amount'}:   {target['required_amount']:.8f} {target['asset']}",
            f"Exportable:      {report['explain_gates']['exportable']}",
            "",
        ]
    )
    purpose = report.get("purpose", {})
    if purpose.get("type") == "planned_exchange_sale":
        lines.extend(
            [
                "Planned Sale",
                "------------",
                f"Destination:     {purpose.get('planned_destination') or '(not specified)'}",
                f"Note:            {purpose.get('planned_note') or '(none)'}",
                purpose.get("fiat_purchase_note") or "",
                "",
            ]
        )
    overview = report.get("overview", {})
    if overview:
        time_range = overview.get("time_range", {})
        lines.extend(
            [
                "Overview",
                "--------",
                f"Time range:      {time_range.get('start') or ''} - {time_range.get('end') or ''}",
                f"Transactions:    {overview.get('transaction_count', 0)}",
                f"Reviewed links:  {overview.get('link_count', 0)}",
                f"Data sources:    {overview.get('data_source_count', 0)}",
                f"Source types:    {overview.get('source_category_count', 0)}",
                "",
            ]
        )
    narrative = report.get("narrative", {})
    paragraphs = narrative.get("paragraphs") if isinstance(narrative, dict) else None
    if paragraphs:
        lines.extend(["Origin and Transaction Flow", "---------------------------"])
        lines.extend(str(paragraph) for paragraph in paragraphs)
        lines.append("")
    data_sources = report.get("data_sources") or []
    if data_sources:
        lines.extend(["Data Sources", "------------"])
        lines.extend(
            hooks.format_table(
                ["Name", "Kind", "Transactions", "Sources", "Assets"],
                [
                    [
                        row.get("label", ""),
                        row.get("kind", ""),
                        row.get("transaction_count", 0),
                        row.get("source_count", 0),
                        ",".join(row.get("assets") or []),
                    ]
                    for row in data_sources
                ],
                [28, 20, 12, 10, 16],
                align_right={2, 3},
            )
        )
        lines.append("")
    lines.extend(
        [
            "Disclosure Preview",
            "------------------",
            "Txids: " + (", ".join(report["disclosure_preview"]["txids"]) or "(none in this reveal mode)"),
            "Evidence attachments: "
            + (", ".join(item["label"] for item in report["disclosure_preview"]["attachments"]) or "(none)"),
            report["disclosure_preview"]["privacy_note"],
            "",
            "Source Mix",
            "----------",
        ]
    )
    if report["source_mix"]:
        lines.extend(
            hooks.format_table(
                ["Source", "Amount", "Asset", "Count"],
                [
                    [
                        row["source_type"],
                        f"{row['amount']:.8f}",
                        report["allocations"]["asset"],
                        row["count"],
                    ]
                    for row in report["source_mix"]
                ],
                [32, 16, 8, 8],
                align_right={1, 3},
            )
        )
    else:
        lines.append("No reviewed root sources yet.")
    lines.extend(["", "Findings", "--------"])
    if report["findings"]:
        for finding in report["findings"]:
            lines.append(f"{finding['severity'].upper()} {finding['code']}: {finding['message']} {finding['ref']}".rstrip())
            next_step = finding.get("next_step") if isinstance(finding, dict) else None
            if isinstance(next_step, dict) and next_step.get("headline"):
                lines.append(f"  -> {next_step['headline']}")
    else:
        lines.append("No blockers or warnings.")
    lines.extend(["", "Flow Links", "----------"])
    if report["graph"]["edges"]:
        lines.extend(
            hooks.format_table(
                ["Type", "State", "Method", "Amount", "Policy", "Explanation"],
                [
                    [
                        edge["link_type"],
                        edge["state"],
                        edge["method"],
                        f"{edge['allocation_amount']:.8f} {edge['asset']}",
                        edge["allocation_policy"],
                        edge["explanation"],
                    ]
                    for edge in report["graph"]["edges"]
                ],
                [18, 10, 24, 18, 10, 52],
            )
        )
    else:
        lines.append("No reviewed links yet.")
    lines.extend(
        [
            "",
            "Limitations",
            "-----------",
            "Kassiber reports reviewed local evidence. It does not certify ownership, perform chain-surveillance scoring, or provide legal/AML advice.",
            "Opening balances are rendered as attested prior-history stops, not as real root sources.",
            "Suggested links and unconfirmed chain observations are never used as PDF proof.",
        ]
    )
    return lines


def export_pdf(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    file_path: str,
    hooks: SourceFundsHooks,
    *,
    case_ref: str | None = None,
    target_transaction_ref: str | None = None,
    target_amount: Any = None,
    report_purpose: str = "existing_transaction",
    planned_destination: str | None = None,
    planned_note: str | None = None,
    reveal_mode: str | None = None,
) -> dict[str, Any]:
    if not case_ref:
        raise AppError(
            "export-source-funds-pdf requires --case from a saved source-funds preview",
            code="validation",
            hint=(
                "Run `reports source-funds --save-case ...` first, review the "
                "disclosure preview, then export that case id."
            ),
        )
    report = load_case_snapshot(conn, workspace_ref, profile_ref, hooks, case_ref)
    if not report["explain_gates"]["exportable"]:
        raise AppError(
            "Source-of-funds PDF export is blocked by unresolved review gates",
            code="export_blocked",
            hint="Run `reports source-funds --machine ...` and resolve every explain_gates.blockers item.",
            details={"blockers": report["explain_gates"]["blockers"]},
            retryable=False,
        )
    snapshot_hash = _snapshot_hash(report)

    result = dict(
        write_source_funds_pdf(
            str(file_path),
            report=report,
            generated_at=_now(),
            snapshot_hash=snapshot_hash,
        )
    )
    result.update(
        {
            "scope": "source_funds",
            "format": "pdf",
            "snapshot_hash": snapshot_hash,
            "reveal_mode": report["reveal_mode"],
            "purpose": report.get("purpose", {}).get("type", "existing_transaction"),
            "target_transaction_id": report["target"]["transaction_id"],
            "exportable": True,
        }
    )
    return result
