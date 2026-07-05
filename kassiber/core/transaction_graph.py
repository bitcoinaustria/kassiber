"""Curated transaction-flow graph payloads for the desktop detail view.

The builder intentionally reuses the ownership-transfer parser/derivers for
classification hints, but returns a UI-specific model. It never exposes raw
transaction JSON, script hex, wallet configuration, descriptors, xpubs, backend
URLs, or credentials.
"""

from __future__ import annotations

import json
import sqlite3
from collections import defaultdict
from typing import Any, Mapping, NamedTuple, Sequence

from ..backends import (
    BACKEND_CONFIG_FIELDS,
    DEFAULT_BACKEND_SETTING,
    DEFAULT_BACKENDS,
    _http_url_base,
    backend_batch_size,
    preferred_mempool_api_backend,
)
from ..db import get_setting
from ..envelope import json_ready
from ..errors import AppError
from ..msat import msat_to_btc
from ..time_utils import now_iso
from ..transfers import detect_intra_transfers, normalize_group_txid
from . import ownership as core_ownership
from .ownership_transfers import (
    _norm_chain_network,
    _parse_onchain_tx,
    derive_multi_source_consolidations,
    derive_ownership_transfers,
    derive_recorded_fanout_transfers,
)
from .repo import current_context_snapshot
from .sync import normalize_backend_kind
from .sync_backends import (
    ElectrumClient,
    bitcoinrpc_call,
    decode_liquid_transaction,
    decode_raw_transaction,
    electrum_call_many,
    fetch_esplora_transaction,
    liquid_input_txid,
)


GRAPH_LOOKUP_TIMEOUT_SECONDS = 5
GRAPH_CACHE_SCHEMA_VERSION = 1
SATS_TO_MSAT = 1000
COINBASE_PREVOUT_TXID = "00" * 32
COINBASE_PREVOUT_VOUT = 0xFFFFFFFF
# Upper bound on how many input/output strands a single graph payload carries
# per side. Mirrors mempool.space's line limit so a fan-out transaction (large
# CoinJoins, sweeping consolidations) cannot inflate the payload or the rendered
# strand count without bound; the remainder collapses into one overflow node.
MAX_GRAPH_NODES_PER_SIDE = 250
MAX_ELECTRUM_GRAPH_PREVTX_LOOKUPS = MAX_GRAPH_NODES_PER_SIDE

_MISSING = object()


class _ProfileSemantics(NamedTuple):
    """Profile-scoped graph inputs that are independent of the focused tx.

    These are expensive to derive (they walk the whole profile), so the daemon
    caches one bundle per profile keyed by ``journal_input_version`` and reuses
    it across the primary graph request and its eagerly-prefetched swap legs.
    """

    owned_index: Any | None
    index_warnings: list[str]
    semantics: dict[str, Any]


def build_transaction_graph_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
    runtime_config: Mapping[str, Any] | None = None,
    *,
    semantics_cache: dict[str, tuple[tuple[Any, ...], _ProfileSemantics]] | None = None,
) -> dict[str, Any]:
    raw_args = args or {}
    if not isinstance(raw_args, dict):
        raw_args = {}
    unknown = sorted(
        set(raw_args) - {"transaction", "allowPublicLookup", "allow_public_lookup"}
    )
    if unknown:
        raise AppError(
            "ui.transactions.graph received unsupported fields",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    transaction_ref = str(raw_args.get("transaction") or "").strip()
    if not transaction_ref:
        raise AppError(
            "ui.transactions.graph requires args.transaction",
            code="validation",
            retryable=False,
        )

    context = current_context_snapshot(conn)
    profile_id = context["profile_id"]
    if not profile_id:
        return _empty_payload(transaction_ref, "no_active_profile")

    row = _fetch_transaction(conn, profile_id, transaction_ref)
    if row is None:
        return _empty_payload(transaction_ref, "not_found")

    bundle = _load_profile_semantics(conn, profile_id, cache=semantics_cache)
    owned_index = bundle.owned_index
    semantics = bundle.semantics
    raw = _json_obj(_row_get(row, "raw_json"))
    allow_public_lookup = _parse_public_lookup_arg(raw_args)
    enriched_raw = _enrich_graph_raw(
        conn,
        row,
        raw,
        runtime_config,
        allow_public_lookup=allow_public_lookup,
    )
    graph = _parse_graph(
        row,
        enriched_raw,
        local_outpoint_amounts=_local_wallet_outpoint_amounts(
            conn,
            profile_id,
            row,
            enriched_raw,
        ),
    )
    _annotate_graph(graph, row, owned_index, semantics)
    warnings = list(graph.pop("_warnings", []))
    warnings.extend(
        {"code": "ownership_index", "level": "info", "message": str(message)}
        for message in bundle.index_warnings
    )
    warnings.extend(_warnings_for_row(row, semantics))
    warnings.extend(_journal_warnings(row))

    tx_meta = _transaction_meta(row, graph)
    tx_id = str(row["id"])
    swap_route = _swap_route_for_row(conn, profile_id, row)
    return {
        "transaction": tx_meta,
        "supportLevel": graph["supportLevel"],
        "unsupportedReason": graph.get("unsupportedReason"),
        "warnings": _dedupe_warnings(warnings),
        "inputs": _public_nodes_capped(graph["inputs"], "input"),
        "outputs": _public_nodes_capped(graph["outputs"], "output"),
        "fee": graph.get("fee"),
        "annotations": semantics["by_row"].get(tx_id, []),
        "accounting": {
            "quarantine": _quarantine(row),
            "linkedPairs": _linked_pairs_for_row(row, semantics),
            "transferGroupIds": sorted(
                {
                    str(annotation.get("groupId"))
                    for annotation in semantics["by_row"].get(tx_id, [])
                    if annotation.get("groupId")
                }
            ),
        },
        "context": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
        },
        "swapRoute": swap_route,
    }


def _parse_public_lookup_arg(args: Mapping[str, Any]) -> bool:
    camel = args.get("allowPublicLookup", _MISSING)
    snake = args.get("allow_public_lookup", _MISSING)
    if camel is not _MISSING and snake is not _MISSING and camel != snake:
        raise AppError(
            "ui.transactions.graph received conflicting public lookup flags",
            code="validation",
            details={
                "allowPublicLookup": camel,
                "allow_public_lookup": snake,
            },
            retryable=False,
        )
    value = camel if camel is not _MISSING else snake
    if value is _MISSING or value is None:
        return False
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized == "true":
            return True
        if normalized == "false":
            return False
    raise AppError(
        "ui.transactions.graph allowPublicLookup must be a boolean",
        code="validation",
        details={"allowPublicLookup": value},
        retryable=False,
    )


def _empty_payload(transaction_ref: str, reason: str) -> dict[str, Any]:
    return {
        "transaction": None,
        "supportLevel": "graphless",
        "unsupportedReason": reason,
        "warnings": [
            {
                "code": reason,
                "level": "warning",
                "message": "Transaction graph is unavailable for the current profile.",
            }
        ],
        "inputs": [],
        "outputs": [],
        "fee": None,
        "annotations": [],
        "accounting": {"quarantine": None, "linkedPairs": [], "transferGroupIds": []},
        "context": {"workspace": None, "profile": None},
        "swapRoute": None,
        "query": transaction_ref,
    }


def _fetch_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
    transaction_ref: str,
) -> sqlite3.Row | None:
    return conn.execute(
        """
        SELECT
            t.*,
            w.label AS wallet_label,
            w.kind AS wallet_kind,
            w.config_json AS wallet_config_json,
            w.account_id AS wallet_account_id,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label,
            jq.reason AS quarantine_reason,
            jq.detail_json AS quarantine_detail_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN accounts a ON a.id = w.account_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE t.profile_id = ?
          AND (t.id = ? OR t.external_id = ?)
        ORDER BY CASE WHEN t.id = ? THEN 0 ELSE 1 END,
                 t.occurred_at DESC,
                 t.created_at DESC,
                 t.id DESC
        LIMIT 1
        """,
        (profile_id, transaction_ref, transaction_ref, transaction_ref),
    ).fetchone()


def _load_profile_transaction_rows(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[sqlite3.Row]:
    return conn.execute(
        """
        SELECT
            t.*,
            w.label AS wallet_label,
            w.account_id AS wallet_account_id,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE t.profile_id = ?
          AND COALESCE(t.excluded, 0) = 0
        ORDER BY t.occurred_at, t.created_at, t.id
        """,
        (profile_id,),
    ).fetchall()


def _wallet_refs_by_id(conn: sqlite3.Connection, profile_id: str) -> dict[str, dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.account_id AS wallet_account_id,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    return {
        str(row["id"]): {
            "id": row["id"],
            "label": row["label"],
            "wallet_account_id": row["wallet_account_id"],
            "account_code": row["account_code"],
            "account_label": row["account_label"],
        }
        for row in rows
    }


def _build_owned_index(
    conn: sqlite3.Connection,
    profile_id: str,
    rows: Sequence[Mapping[str, Any]],
) -> tuple[Any | None, list[str]]:
    wallets = core_ownership.load_profile_wallets(conn, profile_id)
    return core_ownership.build_owned_index(conn, profile_id, wallets)


def _load_profile_semantics(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    cache: dict[str, tuple[tuple[Any, ...], _ProfileSemantics]] | None,
) -> _ProfileSemantics:
    """Return the profile-scoped graph bundle, reusing a cached copy when fresh.

    The cache key captures everything the bundle depends on:
    ``journal_input_version`` (bumped by imports, metadata edits, wallet config
    edits, account changes and manual pairs), plus the wallet and output-inventory
    row counts. The owned index is derived from the wallet set and ``wallet_utxos``
    seeding, and adding a wallet or observing UTXOs does not always bump the
    journal version — so folding those counts in keeps the cache correct without
    forcing a journal reprocess on the high-frequency sync path. ``cache=None``
    (one-shot CLI and tests) recomputes every call, preserving prior behaviour.
    """
    signature = _profile_semantics_signature(conn, profile_id)
    if cache is not None:
        cached = cache.get(profile_id)
        if cached is not None and cached[0] == signature:
            return cached[1]
    bundle = _compute_profile_semantics(conn, profile_id)
    if cache is not None:
        # Keep at most the latest signature per profile so the cache stays
        # O(active profiles); any change simply replaces the stale entry.
        cache[profile_id] = (signature, bundle)
    return bundle


def _compute_profile_semantics(
    conn: sqlite3.Connection,
    profile_id: str,
) -> _ProfileSemantics:
    profile_rows = _load_profile_transaction_rows(conn, profile_id)
    wallet_refs_by_id = _wallet_refs_by_id(conn, profile_id)
    owned_index, index_warnings = _build_owned_index(conn, profile_id, profile_rows)
    manual_pair_records = _active_pair_records(conn, profile_id)
    semantics = _preview_ownership_semantics(
        profile_rows,
        owned_index,
        wallet_refs_by_id,
        manual_pair_records,
    )
    return _ProfileSemantics(
        owned_index=owned_index,
        index_warnings=list(index_warnings),
        semantics=semantics,
    )


def _profile_semantics_signature(
    conn: sqlite3.Connection, profile_id: str
) -> tuple[Any, ...]:
    # A no-FROM SELECT always returns exactly one row. The wallet_utxos COUNT and
    # MAX(last_seen_at) are read in a single scan: the count catches added/removed
    # outpoints, while MAX(last_seen_at) catches in-place re-attribution — the
    # inventory UPSERT restamps last_seen_at on every observed row, including
    # address/derivation rewrites that leave the row count unchanged but do change
    # the owned set the cached index is seeded from.
    row = conn.execute(
        """
        SELECT
            (SELECT journal_input_version FROM profiles WHERE id = :pid) AS version,
            (SELECT COUNT(*) FROM wallets WHERE profile_id = :pid) AS wallets,
            u.cnt AS utxos,
            u.seen AS utxo_seen
        FROM (
            SELECT COUNT(*) AS cnt, MAX(last_seen_at) AS seen
            FROM wallet_utxos WHERE profile_id = :pid
        ) AS u
        """,
        {"pid": profile_id},
    ).fetchone()
    return (
        int(_row_get(row, "version") or 0),
        int(_row_get(row, "wallets") or 0),
        int(_row_get(row, "utxos") or 0),
        _row_get(row, "utxo_seen"),
    )


def _public_nodes_capped(nodes: Sequence[Mapping[str, Any]], side: str) -> list[dict[str, Any]]:
    """Project graph nodes to public payloads, collapsing any overflow.

    A handful of transactions fan out to thousands of legs; rather than ship and
    render every strand, keep the first ``MAX_GRAPH_NODES_PER_SIDE - 1`` and fold
    the rest into a single aggregated overflow node (mirroring the client-side
    compaction, so the UI renders it identically).
    """
    public = [_public_node(node) for node in nodes]
    if len(public) <= MAX_GRAPH_NODES_PER_SIDE:
        return public
    keep = MAX_GRAPH_NODES_PER_SIDE - 1
    visible = public[:keep]
    hidden = public[keep:]
    known = [node["valueSats"] for node in hidden if isinstance(node.get("valueSats"), int)]
    # Only advertise a concrete total when every hidden leg has a known amount;
    # a partial sum (some confidential/missing-prevout legs) must not masquerade
    # as the full aggregate.
    total = sum(known) if known and len(known) == len(hidden) else None
    overflow: dict[str, Any] = {
        "id": f"{side}-overflow",
        "role": "overflow",
        "ownership": "overflow",
        "overflow": True,
        "overflowCount": len(hidden),
        "label": f"+{len(hidden)} more",
        "annotations": [
            {"code": "overflow", "label": f"{len(hidden)} aggregated {side} legs"}
        ],
    }
    if total is not None:
        overflow["valueSats"] = total
        overflow["valueBtc"] = _sats_to_btc(total)
    return [*visible, overflow]


def _local_wallet_outpoint_amounts(
    conn: sqlite3.Connection,
    profile_id: str,
    row: Mapping[str, Any],
    raw: Mapping[str, Any] | None,
) -> dict[str, int]:
    if not isinstance(raw, Mapping):
        return {}
    outpoints: set[str] = set()
    vin = raw.get("vin")
    if isinstance(vin, list):
        for entry in vin:
            if isinstance(entry, Mapping):
                outpoint = _outpoint(entry)
                if outpoint:
                    outpoints.add(outpoint.lower())
    txid = _string_or_none(raw.get("txid")) or _txid_from_row(row)
    vout = raw.get("vout")
    if txid and isinstance(vout, list):
        for index, entry in enumerate(vout):
            if not isinstance(entry, Mapping):
                continue
            n = _int_or_none(entry.get("n"))
            if n is None:
                n = index
            outpoints.add(f"{txid.lower()}:{n}")
    if not outpoints:
        return {}
    chain, network = _row_chain_network(row, default_chain="liquid", default_network="liquidv1")
    placeholders = ", ".join("?" for _ in outpoints)
    rows = conn.execute(
        f"""
        SELECT lower(outpoint) AS outpoint, amount
        FROM wallet_utxos
        WHERE profile_id = ?
          AND lower(chain) = ?
          AND lower(network) = ?
          AND lower(outpoint) IN ({placeholders})
        """,
        (profile_id, chain.lower(), network.lower(), *sorted(outpoints)),
    ).fetchall()
    amounts: dict[str, int] = {}
    for amount_row in rows:
        outpoint = _string_or_none(_row_get(amount_row, "outpoint"))
        amount_msat = _int_or_none(_row_get(amount_row, "amount"))
        if outpoint is not None and amount_msat is not None and amount_msat >= 0:
            amounts[outpoint.lower()] = amount_msat // SATS_TO_MSAT
    return amounts


def _local_outpoint_sats(
    local_outpoint_amounts: Mapping[str, int],
    outpoint: str | None,
) -> int | None:
    if not outpoint:
        return None
    value = local_outpoint_amounts.get(outpoint.lower())
    return value if isinstance(value, int) and value >= 0 else None


def _parse_graph(
    row: Mapping[str, Any],
    raw: Mapping[str, Any] | None = None,
    *,
    local_outpoint_amounts: Mapping[str, int] | None = None,
) -> dict[str, Any]:
    raw = dict(raw) if isinstance(raw, Mapping) else _json_obj(_row_get(row, "raw_json"))
    local_amounts = local_outpoint_amounts or {}
    metadata = _transaction_metadata(raw)
    vin = raw.get("vin")
    vout = raw.get("vout")
    warnings: list[dict[str, str]] = _graph_lookup_warnings(raw)
    confidential = _looks_liquid_or_confidential(row, raw)
    if not isinstance(vin, list) or not isinstance(vout, list):
        reason = "graphless_import"
        if confidential:
            reason = "liquid_reference_graph_not_local"
        graphless_warnings = [
            {
                "code": reason,
                "level": "info",
                "message": _graphless_message(reason),
            }
        ]
        lookup_warning = raw.get("_graphLookupWarning")
        if isinstance(lookup_warning, Mapping):
            graphless_warnings.append(
                {
                    "code": str(lookup_warning.get("code") or "graph_lookup_failed"),
                    "level": str(lookup_warning.get("level") or "warning"),
                    "message": str(
                        lookup_warning.get("message")
                        or "Public transaction reference lookup failed."
                    ),
                }
            )
        return {
            "supportLevel": "graphless",
            "unsupportedReason": reason,
            "metadata": {**metadata, "inputCount": 0, "outputCount": 0},
            "inputs": [],
            "outputs": [],
            "fee": _fee_from_row(row, metadata),
            "_warnings": graphless_warnings,
        }

    valued = _parse_onchain_tx(json.dumps(raw, sort_keys=True))
    inputs: list[dict[str, Any]] = []
    input_value_complete = True
    for index, entry in enumerate(vin):
        if not isinstance(entry, dict):
            continue
        prevout = entry.get("prevout") if isinstance(entry.get("prevout"), dict) else {}
        outpoint = _outpoint(entry)
        local_value_sats = _local_outpoint_sats(local_amounts, outpoint)
        value_hidden = local_value_sats is None and (confidential or _confidential_leg(prevout))
        value_sats = (
            local_value_sats
            if local_value_sats is not None
            else None
            if value_hidden
            else _value_sats_or_none(prevout.get("value"))
        )
        if value_sats is None:
            input_value_complete = False
        script = _script_from_prevout(prevout)
        inputs.append(
            {
                "id": f"in-{index}",
                "index": index,
                "outpoint": outpoint,
                "txid": str(entry.get("txid") or "") or None,
                "vout": _int_or_none(entry.get("vout")),
                "address": _string_or_none(prevout.get("scriptpubkey_address")),
                "scriptType": _script_type(prevout, script),
                "valueSats": value_sats,
                "valueBtc": _sats_to_btc(value_sats),
                "valueState": "confidential" if value_hidden else ("missing" if value_sats is None else "known"),
                "label": outpoint or f"Input {index + 1}",
                "ownership": "unknown",
                "role": "input",
                "annotations": [],
                "_script": script,
            }
        )

    outputs: list[dict[str, Any]] = []
    output_value_complete = True
    liquid_fee_sats: int | None = None
    for index, entry in enumerate(vout):
        if not isinstance(entry, dict):
            continue
        if confidential and _is_liquid_fee_output(entry):
            # Liquid encodes the network fee as a dedicated unblinded output. It
            # is the fee, not an OP_RETURN/non-address output, and its amount is
            # public even when every other leg is confidential — so route its
            # value to the fee node and keep it out of the output strands.
            fee_value = _value_sats_or_none(entry.get("value"))
            if fee_value is not None and fee_value >= 0:
                liquid_fee_sats = (liquid_fee_sats or 0) + fee_value
            continue
        script = _string_or_none(entry.get("scriptpubkey") or entry.get("script_hex"))
        n = _int_or_none(entry.get("n"))
        if n is None:
            n = index
        outpoint = f"{str(raw.get('txid') or _row_get(row, 'external_id') or '').lower()}:{n}"
        local_value_sats = _local_outpoint_sats(local_amounts, outpoint)
        value_hidden = local_value_sats is None and (confidential or _confidential_leg(entry))
        value_sats = (
            local_value_sats
            if local_value_sats is not None
            else None
            if value_hidden
            else _value_sats_or_none(entry.get("value"))
        )
        if value_sats is None:
            output_value_complete = False
        outputs.append(
            {
                "id": f"out-{n}",
                "index": n,
                "outpoint": outpoint,
                "address": _string_or_none(entry.get("scriptpubkey_address")),
                "scriptType": _script_type(entry, script),
                "valueSats": value_sats,
                "valueBtc": _sats_to_btc(value_sats),
                "valueState": "confidential" if value_hidden else ("missing" if value_sats is None else "known"),
                "label": f"Output {n}",
                "ownership": "unknown",
                "role": "output",
                "annotations": [],
                "_script": script,
            }
        )

    if input_value_complete and output_value_complete and (valued is not None or confidential):
        support = "full"
        reason = None
    else:
        support = "partial"
        if confidential:
            reason = "confidential_values_hidden"
        elif not input_value_complete and output_value_complete:
            reason = "input_prevout_values_missing"
        else:
            reason = "partial_graph_values_missing"
        warnings.append(
            {
                "code": reason,
                "level": "info",
                "message": (
                    "Amounts are confidential on at least one Liquid input/output. "
                    "Kassiber can show public references and ownership hints, but not value-sized dots or fee rate."
                    if confidential
                    else _partial_value_message(reason)
                ),
            }
        )

    metadata = {
        **metadata,
        "inputCount": len(inputs),
        "outputCount": len(outputs),
    }
    fee = _fee_from_graph_or_row(
        row, inputs, outputs, metadata, explicit_fee_sats=liquid_fee_sats
    )
    return {
        "supportLevel": support,
        "unsupportedReason": reason,
        "metadata": metadata,
        "inputs": inputs,
        "outputs": outputs,
        "fee": fee,
        "_warnings": warnings,
    }


def _enrich_graph_raw(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    raw: Mapping[str, Any],
    runtime_config: Mapping[str, Any] | None,
    *,
    allow_public_lookup: bool = False,
) -> Mapping[str, Any]:
    if not allow_public_lookup:
        return raw
    if _looks_liquid_or_confidential(row, raw):
        return _enrich_liquid_reference_graph_raw(conn, row, raw, runtime_config)
    return _enrich_bitcoin_graph_raw(conn, row, raw, runtime_config)


def _enrich_bitcoin_graph_raw(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    raw: Mapping[str, Any],
    runtime_config: Mapping[str, Any] | None,
) -> Mapping[str, Any]:
    if not _can_lookup_public_bitcoin_graph(row, raw):
        return raw
    txid = _string_or_none(raw.get("txid")) or _txid_from_row(row)
    if not _looks_like_txid(txid):
        return raw
    chain, network = _row_chain_network(row)
    cached = _load_graph_lookup_cache(conn, chain, network, str(txid))
    if cached is not None and _bitcoin_current_graph_has_required_prevouts(cached):
        return cached
    backend = _graph_lookup_backend(conn, row, runtime_config)
    if backend is None:
        return _with_graph_lookup_warning(
            raw,
            "bitcoin_reference_lookup_unavailable",
            "No configured Bitcoin graph backend is available to fetch transaction references.",
        )
    kind = normalize_backend_kind(backend.get("kind"))
    if kind == "electrum":
        try:
            return _fetch_bitcoin_electrum_graph_raw(
                conn,
                backend,
                chain,
                network,
                str(txid),
            )
        except Exception:
            return _with_graph_lookup_warning(
                raw,
                "bitcoin_reference_lookup_failed",
                "Could not fetch public Bitcoin transaction references from the selected Electrum backend.",
            )
    if kind == "bitcoinrpc":
        try:
            return _fetch_bitcoinrpc_transaction_graph(
                conn,
                backend,
                chain,
                network,
                str(txid),
            )
        except Exception:
            return _with_graph_lookup_warning(
                raw,
                "bitcoin_reference_lookup_failed",
                "Could not fetch public Bitcoin transaction references from the selected Bitcoin Core backend.",
            )
    try:
        fetched = _fetch_graph_esplora_transaction(backend, str(txid))
    except Exception:
        return _with_graph_lookup_warning(
            raw,
            "bitcoin_reference_lookup_failed",
            "Could not fetch Bitcoin transaction references from the selected backend.",
        )
    if not isinstance(fetched, Mapping):
        return _with_graph_lookup_warning(
            raw,
            "bitcoin_reference_lookup_invalid",
            "The selected Bitcoin explorer backend returned an invalid transaction response.",
        )
    sanitized = _sanitize_graph_lookup_raw(fetched, chain, str(txid))
    fetched_txid = _string_or_none(sanitized.get("txid"))
    if fetched_txid and fetched_txid.lower() != str(txid).lower():
        return _with_graph_lookup_warning(
            raw,
            "bitcoin_reference_lookup_mismatch",
            "Bitcoin reference lookup returned a different transaction id.",
        )
    if not isinstance(sanitized.get("vin"), list) or not isinstance(sanitized.get("vout"), list):
        return _with_graph_lookup_warning(
            raw,
            "bitcoin_reference_lookup_incomplete",
            "Bitcoin reference lookup did not return public input/output references.",
        )
    if not _bitcoin_current_graph_has_required_prevouts(sanitized):
        return _with_graph_lookup_warning(
            sanitized,
            "bitcoin_reference_lookup_incomplete",
            "Bitcoin reference lookup did not return every previous output needed for a complete graph.",
        )
    return _store_graph_lookup_cache(conn, chain, network, str(txid), sanitized)


def _with_graph_lookup_warning(
    raw: Mapping[str, Any],
    code: str,
    message: str,
) -> dict[str, Any]:
    return {
        **dict(raw),
        "_graphLookupWarning": {
            "code": code,
            "level": "warning",
            "message": message,
        },
    }


def _graph_lookup_warnings(raw: Mapping[str, Any]) -> list[dict[str, str]]:
    lookup_warning = raw.get("_graphLookupWarning")
    if not isinstance(lookup_warning, Mapping):
        return []
    return [
        {
            "code": str(lookup_warning.get("code") or "graph_lookup_warning"),
            "level": str(lookup_warning.get("level") or "warning"),
            "message": str(
                lookup_warning.get("message")
                or "Public transaction reference lookup did not return a complete graph."
            ),
        }
    ]


def _graph_lookup_timeout(backend: Mapping[str, Any]) -> int:
    configured = _int_or_none(backend.get("timeout"))
    if configured is None or configured <= 0:
        return GRAPH_LOOKUP_TIMEOUT_SECONDS
    return min(configured, GRAPH_LOOKUP_TIMEOUT_SECONDS)


def _fetch_bitcoinrpc_transaction_graph(
    conn: sqlite3.Connection,
    backend: Mapping[str, Any],
    chain: str,
    network: str,
    txid: str,
) -> Mapping[str, Any]:
    decoded = bitcoinrpc_call(
        dict(backend),
        "getrawtransaction",
        [txid, True],
        timeout=_graph_lookup_timeout(backend),
    )
    if not isinstance(decoded, Mapping):
        raise AppError("Bitcoin Core returned an invalid transaction response")
    raw = _bitcoinrpc_decoded_to_graph_raw(decoded)
    raw = _attach_bitcoinrpc_prevouts_from_cache_or_rpc(
        conn,
        dict(backend),
        chain,
        network,
        raw,
    )
    if not _bitcoin_current_graph_has_required_prevouts(raw):
        if raw.get("_graphLookupWarning"):
            return raw
        return _with_graph_lookup_warning(
            raw,
            "bitcoin_reference_lookup_incomplete",
            "Bitcoin Core lookup did not return every previous output needed for a complete graph.",
        )
    return _store_graph_lookup_cache(conn, chain, network, txid, raw)


def _bitcoinrpc_decoded_to_graph_raw(
    decoded: Mapping[str, Any],
) -> dict[str, Any]:
    txid = _string_or_none(decoded.get("txid"))
    graph: dict[str, Any] = {
        "txid": txid,
        "version": decoded.get("version"),
        "locktime": decoded.get("locktime"),
        "size": decoded.get("size"),
        "vsize": decoded.get("vsize"),
        "weight": decoded.get("weight"),
        "raw_hex": decoded.get("hex"),
        "vin": [],
        "vout": [],
    }
    for input_entry in decoded.get("vin") if isinstance(decoded.get("vin"), list) else []:
        if not isinstance(input_entry, Mapping):
            continue
        graph_input: dict[str, Any] = {
            "txid": input_entry.get("txid"),
            "vout": input_entry.get("vout"),
            "sequence": input_entry.get("sequence"),
        }
        # Core only inlines prevout at verbosity 2 (v25+); when present, keep it.
        # Otherwise the missing previous outputs are resolved once, deduplicated,
        # by _attach_bitcoinrpc_prevouts_from_cache_or_rpc.
        prevout = input_entry.get("prevout")
        if isinstance(prevout, Mapping):
            graph_input["prevout"] = _bitcoinrpc_prevout_to_graph(prevout)
        graph["vin"].append(graph_input)
    for output_entry in decoded.get("vout") if isinstance(decoded.get("vout"), list) else []:
        if isinstance(output_entry, Mapping):
            graph["vout"].append(_bitcoinrpc_vout_to_graph(output_entry))
    return graph


def _attach_bitcoinrpc_prevouts_from_cache_or_rpc(
    conn: sqlite3.Connection,
    backend: Mapping[str, Any],
    chain: str,
    network: str,
    raw: Mapping[str, Any],
) -> dict[str, Any]:
    """Resolve missing previous outputs for a Bitcoin Core graph.

    Mirrors the Electrum path (`_attach_bitcoin_prevouts_from_cache_or_electrum`):
    each distinct previous txid is fetched at most once, the durable graph cache
    is consulted and populated, and the number of uncached lookups is capped so a
    many-input transaction (e.g. a consolidation) cannot fan out into an
    unbounded run of serial `getrawtransaction` calls against the user's node.
    """
    enriched = dict(raw)
    vin = [dict(entry) for entry in enriched.get("vin", []) if isinstance(entry, Mapping)]
    enriched["vin"] = vin
    missing: list[str] = []
    seen_missing: set[str] = set()
    cached_prev: dict[str, Mapping[str, Any]] = {}
    for entry in vin:
        prev_txid = _string_or_none(entry.get("txid"))
        prev_vout = _int_or_none(entry.get("vout"))
        if not prev_txid or prev_vout is None:
            # Coinbase-like inputs have no spent previous output.
            continue
        prevout = entry.get("prevout")
        if isinstance(prevout, Mapping) and prevout.get("value") is not None:
            # Already inlined by Core at verbosity 2.
            continue
        if not _looks_like_txid(prev_txid):
            continue
        cached = _load_graph_lookup_cache(conn, chain, network, prev_txid)
        if cached is not None:
            cached_prev[prev_txid.lower()] = cached
            continue
        normalized = prev_txid.lower()
        if normalized not in seen_missing:
            seen_missing.add(normalized)
            missing.append(normalized)

    if len(missing) > MAX_ELECTRUM_GRAPH_PREVTX_LOOKUPS:
        _attach_bitcoin_prevouts_from_cached_graphs(enriched, cached_prev)
        return _with_graph_lookup_warning(
            enriched,
            "bitcoin_reference_lookup_prevout_limit",
            (
                "Bitcoin Core graph lookup needs too many uncached previous transactions; "
                "Kassiber capped the request to avoid flooding the node."
            ),
        )

    for prev_txid in missing:
        try:
            previous = bitcoinrpc_call(
                dict(backend),
                "getrawtransaction",
                [prev_txid, True],
                timeout=_graph_lookup_timeout(backend),
            )
        except Exception:
            # Best-effort: a single unfetchable prevout degrades the graph to a
            # warning rather than aborting the whole lookup.
            continue
        if not isinstance(previous, Mapping):
            continue
        prev_raw = _bitcoinrpc_decoded_to_graph_raw(previous)
        cached_prev[prev_txid] = _store_graph_lookup_cache(
            conn,
            chain,
            network,
            prev_txid,
            prev_raw,
        )

    _attach_bitcoin_prevouts_from_cached_graphs(enriched, cached_prev)
    return enriched


def _bitcoinrpc_prevout_to_graph(source: Mapping[str, Any]) -> dict[str, Any]:
    return _bitcoinrpc_vout_to_graph(source)


def _bitcoinrpc_vout_to_graph(source: Mapping[str, Any]) -> dict[str, Any]:
    script = source.get("scriptPubKey")
    script_obj = script if isinstance(script, Mapping) else {}
    address = _string_or_none(script_obj.get("address"))
    if not address:
        addresses = script_obj.get("addresses")
        if isinstance(addresses, list) and addresses:
            address = _string_or_none(addresses[0])
    payload: dict[str, Any] = {
        "n": source.get("n"),
        "value": source.get("value"),
        "scriptpubkey": script_obj.get("hex") or source.get("scriptpubkey"),
        "scriptpubkey_type": script_obj.get("type") or source.get("scriptpubkey_type"),
    }
    if address:
        payload["scriptpubkey_address"] = address
    return payload


def _graph_backend_with_timeout_cap(backend: Mapping[str, Any]) -> dict[str, Any]:
    capped = dict(backend)
    capped["timeout"] = _graph_lookup_timeout(backend)
    return capped


def _fetch_graph_esplora_transaction(backend: Mapping[str, Any], txid: str) -> Any:
    kwargs: dict[str, Any] = {"timeout": _graph_lookup_timeout(backend)}
    proxy_url = _string_or_none(backend.get("tor_proxy"))
    if proxy_url is not None:
        kwargs["proxy_url"] = proxy_url
    return fetch_esplora_transaction(str(backend["url"]), txid, **kwargs)


def _enrich_liquid_reference_graph_raw(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    raw: Mapping[str, Any],
    runtime_config: Mapping[str, Any] | None,
) -> Mapping[str, Any]:
    if isinstance(raw.get("vin"), list) and isinstance(raw.get("vout"), list):
        return raw
    txid = _string_or_none(raw.get("txid")) or _txid_from_row(row)
    if not _looks_like_txid(txid):
        return raw
    chain, network = _row_chain_network(row, default_chain="liquid", default_network="liquidv1")
    cached = _load_graph_lookup_cache(conn, chain, network, str(txid))
    if cached is not None:
        return cached
    backend = _liquid_graph_lookup_backend(conn, row, runtime_config)
    if backend is None:
        return _with_graph_lookup_warning(
            raw,
            "liquid_reference_lookup_unavailable",
            "No configured Liquid graph backend is available to fetch public transaction references.",
        )
    kind = normalize_backend_kind(backend.get("kind"))
    if kind == "electrum":
        try:
            return _fetch_liquid_electrum_graph_raw(
                conn,
                backend,
                chain,
                network,
                str(txid),
            )
        except Exception:
            return _with_graph_lookup_warning(
                raw,
                "liquid_reference_lookup_failed",
                "Could not fetch public Liquid transaction references from the selected Electrum backend.",
            )
    try:
        fetched = _fetch_graph_esplora_transaction(backend, str(txid))
    except Exception:
        return _with_graph_lookup_warning(
            raw,
            "liquid_reference_lookup_failed",
            "Could not fetch public Liquid transaction references from the selected explorer backend.",
        )
    if not isinstance(fetched, Mapping):
        return _with_graph_lookup_warning(
            raw,
            "liquid_reference_lookup_invalid",
            "The selected Liquid explorer backend returned an invalid transaction response.",
        )
    sanitized = _sanitize_graph_lookup_raw(fetched, chain, str(txid))
    fetched_txid = _string_or_none(sanitized.get("txid"))
    if fetched_txid and fetched_txid.lower() != str(txid).lower():
        return _with_graph_lookup_warning(
            raw,
            "liquid_reference_lookup_mismatch",
            "Liquid reference lookup returned a different transaction id.",
        )
    if not isinstance(sanitized.get("vin"), list) or not isinstance(sanitized.get("vout"), list):
        return _with_graph_lookup_warning(
            raw,
            "liquid_reference_lookup_incomplete",
            "Liquid reference lookup did not return public input/output references.",
        )
    return _store_graph_lookup_cache(conn, chain, network, str(txid), sanitized)


def _can_lookup_public_bitcoin_graph(row: Mapping[str, Any], raw: Mapping[str, Any]) -> bool:
    if _looks_liquid_or_confidential(row, raw):
        return False
    asset = str(_row_get(row, "asset") or "").upper()
    if asset and asset != "BTC":
        return False
    vin = raw.get("vin")
    vout = raw.get("vout")
    if isinstance(vin, list) and isinstance(vout, list):
        return _can_lookup_public_bitcoin_prevouts(row, raw)
    return True


def _can_lookup_public_bitcoin_prevouts(row: Mapping[str, Any], raw: Mapping[str, Any]) -> bool:
    vin = raw.get("vin")
    vout = raw.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return False
    if _looks_liquid_or_confidential(row, raw):
        return False
    asset = str(_row_get(row, "asset") or "").upper()
    if asset and asset != "BTC":
        return False
    return any(
        isinstance(entry, Mapping)
        and not _confidential_leg(entry.get("prevout") if isinstance(entry.get("prevout"), Mapping) else {})
        and _value_sats_or_none(
            (entry.get("prevout") if isinstance(entry.get("prevout"), Mapping) else {}).get("value")
        )
        is None
        for entry in vin
    )


def _graph_lookup_backend(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    runtime_config: Mapping[str, Any] | None,
) -> dict[str, Any] | None:
    chain, network = _row_chain_network(row)
    if chain != "bitcoin":
        return None
    runtime_candidate = _runtime_graph_lookup_backend(runtime_config, chain, network)
    if runtime_candidate is not None:
        return runtime_candidate
    default_candidate = _default_graph_backend(conn, chain, network)
    if default_candidate is not None:
        return default_candidate
    candidate = _preferred_mempool_graph_backend(conn, chain, network)
    if candidate is not None:
        return candidate
    return _configured_graph_backend(conn, chain, network)


def _liquid_graph_lookup_backend(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    runtime_config: Mapping[str, Any] | None,
) -> dict[str, Any] | None:
    chain, network = _row_chain_network(row, default_chain="liquid", default_network="liquidv1")
    if chain != "liquid":
        return None
    runtime_candidate = _runtime_graph_lookup_backend(runtime_config, chain, network)
    if runtime_candidate is not None:
        return runtime_candidate
    default_candidate = _default_graph_backend(conn, chain, network)
    if default_candidate is not None:
        return default_candidate
    candidate = _preferred_mempool_graph_backend(conn, chain, network)
    if candidate is not None:
        return candidate
    # Symmetry with the Bitcoin path: never silently fetch from a hardcoded
    # third-party explorer. Without a configured Liquid backend we decline the
    # lookup; the caller surfaces a warning and the UI still offers an explicit,
    # user-initiated "open in explorer" link.
    return _configured_graph_backend(conn, chain, network)


def _preferred_mempool_graph_backend(
    conn: sqlite3.Connection,
    chain: str,
    network: str,
) -> dict[str, Any] | None:
    candidate = preferred_mempool_api_backend(conn, chain, network)
    if candidate is None:
        return None
    return {
        "name": candidate.get("name"),
        "kind": "esplora",
        "chain": chain,
        "network": network,
        "url": candidate.get("api_base_url"),
        "timeout": candidate.get("timeout"),
        "tor_proxy": candidate.get("tor_proxy"),
    }


def _default_graph_backend(
    conn: sqlite3.Connection,
    chain: str,
    network: str,
) -> dict[str, Any] | None:
    default_name = _string_or_none(get_setting(conn, DEFAULT_BACKEND_SETTING))
    if default_name is None:
        return None
    row = conn.execute(
        """
        SELECT name, kind, chain, network, url, timeout, batch_size, tor_proxy, config_json
        FROM backends
        WHERE lower(name) = ?
        LIMIT 1
        """,
        (default_name.lower(),),
    ).fetchone()
    if row is None:
        return None
    backend = _graph_backend_from_row(row)
    if _graph_backend_matches(backend, chain, network):
        return _normalized_graph_lookup_backend(backend)
    return None


def _configured_graph_backend(
    conn: sqlite3.Connection,
    chain: str,
    network: str,
) -> dict[str, Any] | None:
    rows = conn.execute(
        """
        SELECT name, kind, chain, network, url, timeout, batch_size, tor_proxy, config_json
        FROM backends
        ORDER BY
            CASE lower(kind)
                WHEN 'esplora' THEN 0
                WHEN 'mempool' THEN 0
                WHEN 'liquid-esplora' THEN 0
                WHEN 'electrum' THEN 1
                WHEN 'bitcoinrpc' THEN 2
                ELSE 3
            END,
            updated_at DESC,
            name ASC
        """
    ).fetchall()
    for row in rows:
        backend = _graph_backend_from_row(row)
        if _graph_backend_matches(backend, chain, network):
            return _normalized_graph_lookup_backend(backend)
    return None


def _runtime_graph_lookup_backend(
    runtime_config: Mapping[str, Any] | None,
    chain: str,
    network: str,
) -> dict[str, Any] | None:
    if not isinstance(runtime_config, Mapping):
        return None
    backends = runtime_config.get("backends")
    if not isinstance(backends, Mapping):
        return None
    default_name = str(runtime_config.get("default_backend") or "").strip().lower()
    names = [default_name] if default_name else []
    names.extend(sorted(str(name) for name in backends if str(name) not in names))
    for name in names:
        backend = backends.get(name)
        if not isinstance(backend, Mapping):
            continue
        if _skip_implicit_builtin_runtime_graph_backend(runtime_config, name, backend, default_name):
            continue
        candidate = _graph_backend_from_mapping(name, backend)
        if _graph_backend_matches(candidate, chain, network):
            return _normalized_graph_lookup_backend(candidate)
    return None


def _skip_implicit_builtin_runtime_graph_backend(
    runtime_config: Mapping[str, Any],
    name: str,
    backend: Mapping[str, Any],
    default_name: str,
) -> bool:
    normalized_name = str(name).strip().lower()
    if normalized_name == default_name or normalized_name not in DEFAULT_BACKENDS:
        return False
    backend_chain, _backend_network = _norm_chain_network(
        backend.get("chain"),
        backend.get("network"),
    )
    if backend_chain != "liquid":
        return False
    if normalized_name in set(runtime_config.get("dotenv_backends") or ()):
        return False
    env_overrides = runtime_config.get("process_env_overrides", {}).get("backends", {})
    if isinstance(env_overrides, Mapping) and env_overrides.get(normalized_name):
        return False
    return backend.get("source") == "built-in default"


def _normalized_graph_lookup_backend(backend: Mapping[str, Any]) -> dict[str, Any]:
    candidate = dict(backend)
    kind = normalize_backend_kind(candidate.get("kind"))
    if kind in {"esplora", "mempool"}:
        api_url = _http_url_base(candidate.get("url"), api=True)
        if api_url is not None:
            candidate["url"] = api_url
    return candidate


def _graph_backend_from_row(row: Mapping[str, Any]) -> dict[str, Any]:
    config = _json_obj(_row_get(row, "config_json"))
    backend = {
        "name": _row_get(row, "name"),
        "kind": _row_get(row, "kind"),
        "chain": _row_get(row, "chain"),
        "network": _row_get(row, "network"),
        "url": _row_get(row, "url"),
        "timeout": _row_get(row, "timeout"),
        "batch_size": _row_get(row, "batch_size"),
        "tor_proxy": _row_get(row, "tor_proxy"),
    }
    backend.update(config)
    return backend


def _graph_backend_from_mapping(name: str, backend: Mapping[str, Any]) -> dict[str, Any]:
    config = backend.get("config") if isinstance(backend.get("config"), Mapping) else {}
    candidate = {
        "name": name,
        "kind": backend.get("kind"),
        "chain": backend.get("chain"),
        "network": backend.get("network"),
        "url": backend.get("url"),
        "timeout": backend.get("timeout"),
        "batch_size": backend.get("batch_size"),
        "tor_proxy": backend.get("tor_proxy") or backend.get("proxy"),
        "certificate": backend.get("certificate"),
        "insecure": backend.get("insecure"),
    }
    for key in BACKEND_CONFIG_FIELDS:
        if key in backend and backend.get(key) is not None:
            candidate[key] = backend.get(key)
    if isinstance(config, Mapping):
        candidate.update(config)
    return candidate


def _graph_backend_matches(
    backend: Mapping[str, Any],
    chain: str,
    network: str,
) -> bool:
    kind = normalize_backend_kind(backend.get("kind"))
    if kind not in {"esplora", "mempool", "electrum", "bitcoinrpc"}:
        return False
    if not _string_or_none(backend.get("url")):
        return False
    backend_chain, backend_network = _norm_chain_network(
        backend.get("chain"),
        backend.get("network"),
    )
    wanted_chain, wanted_network = _norm_chain_network(chain, network)
    return backend_chain == wanted_chain and backend_network == wanted_network


def _load_graph_lookup_cache(
    conn: sqlite3.Connection,
    chain: str,
    network: str,
    txid: str,
) -> dict[str, Any] | None:
    normalized_txid = str(txid).strip().lower()
    if not _looks_like_txid(normalized_txid):
        return None
    row = conn.execute(
        """
        SELECT payload_json
        FROM transaction_graph_cache
        WHERE schema_version = ?
          AND chain = ?
          AND network = ?
          AND txid = ?
        """,
        (GRAPH_CACHE_SCHEMA_VERSION, chain, network, normalized_txid),
    ).fetchone()
    if row is None:
        return None
    payload = _json_obj(row["payload_json"])
    if not isinstance(payload.get("vin"), list) or not isinstance(payload.get("vout"), list):
        return None
    cached_txid = _string_or_none(payload.get("txid"))
    if cached_txid and cached_txid.lower() != normalized_txid:
        return None
    payload["txid"] = normalized_txid
    return payload


def _store_graph_lookup_cache(
    conn: sqlite3.Connection,
    chain: str,
    network: str,
    txid: str,
    raw: Mapping[str, Any],
) -> dict[str, Any]:
    normalized_txid = str(txid).strip().lower()
    # The durable cache deliberately stores the smallest normalized public graph
    # shape needed to rebuild the UI graph. Do not persist Sparrow-style raw
    # serialized transactions here: raw tx bytes carry witnesses, arbitrary
    # scripts/op_return payloads, and unrelated backend response shape that the
    # Kassiber graph does not need to retain.
    sanitized = _sanitize_graph_lookup_raw(raw, chain, normalized_txid)
    if not isinstance(sanitized.get("vin"), list) or not isinstance(sanitized.get("vout"), list):
        return sanitized
    timestamp = now_iso()
    conn.execute(
        """
        INSERT INTO transaction_graph_cache(
            schema_version, chain, network, txid, payload_json, created_at, updated_at
        )
        VALUES(?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(schema_version, chain, network, txid) DO UPDATE SET
            payload_json = excluded.payload_json,
            updated_at = excluded.updated_at
        """,
        (
            GRAPH_CACHE_SCHEMA_VERSION,
            chain,
            network,
            normalized_txid,
            json.dumps(json_ready(sanitized), sort_keys=True, separators=(",", ":")),
            timestamp,
            timestamp,
        ),
    )
    conn.commit()
    return sanitized


def _fetch_bitcoin_electrum_graph_raw(
    conn: sqlite3.Connection,
    backend: Mapping[str, Any],
    chain: str,
    network: str,
    txid: str,
) -> Mapping[str, Any]:
    client_backend = _graph_backend_with_timeout_cap(backend)
    with ElectrumClient(client_backend) as client:
        raw_hex = client.call("blockchain.transaction.get", [txid])
        decoded = decode_raw_transaction(str(raw_hex))
        raw = _bitcoin_electrum_decoded_to_graph_raw(txid, decoded, str(raw_hex))
        raw = _attach_bitcoin_prevouts_from_cache_or_electrum(
            conn,
            client_backend,
            chain,
            network,
            raw,
            client,
        )
    if not _bitcoin_current_graph_has_required_prevouts(raw):
        if raw.get("_graphLookupWarning"):
            return raw
        return _with_graph_lookup_warning(
            raw,
            "bitcoin_reference_lookup_incomplete",
            "Bitcoin Electrum lookup did not return every previous output needed for a complete graph.",
        )
    return _store_graph_lookup_cache(conn, chain, network, txid, raw)


def _fetch_liquid_electrum_graph_raw(
    conn: sqlite3.Connection,
    backend: Mapping[str, Any],
    chain: str,
    network: str,
    txid: str,
) -> Mapping[str, Any]:
    with ElectrumClient(_graph_backend_with_timeout_cap(backend)) as client:
        raw_hex = client.call("blockchain.transaction.get", [txid])
    decoded = decode_liquid_transaction(str(raw_hex))
    raw = _liquid_electrum_decoded_to_graph_raw(txid, decoded, str(raw_hex))
    return _store_graph_lookup_cache(conn, chain, network, txid, raw)


def _attach_bitcoin_prevouts_from_cache_or_electrum(
    conn: sqlite3.Connection,
    backend: Mapping[str, Any],
    chain: str,
    network: str,
    raw: Mapping[str, Any],
    client: ElectrumClient,
) -> dict[str, Any]:
    enriched = dict(raw)
    vin = [dict(entry) for entry in enriched.get("vin", []) if isinstance(entry, Mapping)]
    enriched["vin"] = vin
    missing: list[str] = []
    seen_missing: set[str] = set()
    cached_prev: dict[str, Mapping[str, Any]] = {}
    for entry in vin:
        prev_txid = _string_or_none(entry.get("txid"))
        prev_vout = _int_or_none(entry.get("vout"))
        if not prev_txid or prev_vout is None:
            continue
        if isinstance(entry.get("prevout"), Mapping):
            prevout = entry["prevout"]
            if _value_sats_or_none(prevout.get("value")) is not None or _confidential_leg(prevout):
                continue
        cached = _load_graph_lookup_cache(conn, chain, network, prev_txid)
        if cached is not None:
            cached_prev[prev_txid.lower()] = cached
            continue
        normalized = prev_txid.lower()
        if normalized not in seen_missing:
            seen_missing.add(normalized)
            missing.append(normalized)

    if len(missing) > MAX_ELECTRUM_GRAPH_PREVTX_LOOKUPS:
        _attach_bitcoin_prevouts_from_cached_graphs(enriched, cached_prev)
        return _with_graph_lookup_warning(
            enriched,
            "bitcoin_reference_lookup_prevout_limit",
            (
                "Bitcoin Electrum graph lookup needs too many uncached previous transactions; "
                "Kassiber capped the request to avoid flooding the backend."
            ),
        )

    if missing:
        requests = [("blockchain.transaction.get", [prev_txid]) for prev_txid in missing]
        raw_hexes = electrum_call_many(
            client,
            requests,
            batch_size=backend_batch_size(backend),
        )
        for prev_txid, raw_hex in zip(missing, raw_hexes):
            decoded = decode_raw_transaction(str(raw_hex))
            prev_raw = _bitcoin_electrum_decoded_to_graph_raw(
                prev_txid,
                decoded,
                str(raw_hex),
            )
            cached_prev[prev_txid] = _store_graph_lookup_cache(
                conn,
                chain,
                network,
                prev_txid,
                prev_raw,
            )

    _attach_bitcoin_prevouts_from_cached_graphs(enriched, cached_prev)
    return enriched


def _attach_bitcoin_prevouts_from_cached_graphs(
    raw: dict[str, Any],
    cached_prev: Mapping[str, Mapping[str, Any]],
) -> None:
    vin = raw.get("vin")
    if not isinstance(vin, list):
        return
    for entry in vin:
        if not isinstance(entry, dict):
            continue
        prev_txid = _string_or_none(entry.get("txid"))
        prev_vout = _int_or_none(entry.get("vout"))
        if not prev_txid or prev_vout is None:
            continue
        prev_raw = cached_prev.get(prev_txid.lower())
        if prev_raw is None:
            continue
        prevout = _prevout_from_cached_graph(prev_raw, prev_vout)
        if prevout is not None:
            entry["prevout"] = prevout


def _prevout_from_cached_graph(
    raw: Mapping[str, Any],
    index: int,
) -> dict[str, Any] | None:
    vout = raw.get("vout")
    if not isinstance(vout, list):
        return None
    for entry in vout:
        if not isinstance(entry, Mapping):
            continue
        n = _int_or_none(entry.get("n"))
        if n != index:
            continue
        prevout: dict[str, Any] = {}
        for key in ("scriptpubkey", "scriptpubkey_type", "scriptpubkey_address", "value", "value_state"):
            if entry.get(key) is not None:
                prevout[key] = entry.get(key)
        return prevout
    return None


def _bitcoin_current_graph_has_required_prevouts(raw: Mapping[str, Any]) -> bool:
    vin = raw.get("vin")
    vout = raw.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return False
    for entry in vin:
        if not isinstance(entry, Mapping):
            return False
        prev_txid = _string_or_none(entry.get("txid"))
        prev_vout = _int_or_none(entry.get("vout"))
        if not prev_txid and prev_vout is None:
            # Coinbase-like inputs do not have a spent previous output.
            continue
        if not prev_txid or prev_vout is None:
            return False
        prevout = entry.get("prevout")
        if not isinstance(prevout, Mapping):
            return False
        value_sats = _int_or_none(prevout.get("value_sats"))
        if value_sats is None:
            value_sats = _value_sats_or_none(prevout.get("value"))
        if value_sats is None:
            return False
        if _string_or_none(prevout.get("scriptpubkey") or prevout.get("script_hex")) is None:
            return False
    return True


def _bitcoin_electrum_decoded_to_graph_raw(
    txid: str,
    decoded: Mapping[str, Any],
    raw_hex: str | None = None,
) -> dict[str, Any]:
    raw: dict[str, Any] = {
        "txid": str(txid).lower(),
        "version": _int_or_none(decoded.get("version")),
        "locktime": _int_or_none(decoded.get("locktime")),
        "vin": [],
        "vout": [],
    }
    if raw_hex:
        raw.update(_size_metadata_from_raw_hex(raw_hex))
    for entry in decoded.get("vin", []) if isinstance(decoded.get("vin"), list) else []:
        if not isinstance(entry, Mapping):
            continue
        vin_entry: dict[str, Any] = {}
        prev_txid = _string_or_none(entry.get("txid"))
        prev_vout = _int_or_none(entry.get("vout"))
        if prev_txid and prev_txid.lower() == COINBASE_PREVOUT_TXID and prev_vout == COINBASE_PREVOUT_VOUT:
            prev_txid = None
            prev_vout = None
        if prev_txid:
            vin_entry["txid"] = prev_txid.lower()
        if prev_vout is not None:
            vin_entry["vout"] = prev_vout
        raw["vin"].append(vin_entry)
    for index, entry in enumerate(decoded.get("vout", []) if isinstance(decoded.get("vout"), list) else []):
        if not isinstance(entry, Mapping):
            continue
        n = _int_or_none(entry.get("n"))
        if n is None:
            n = index
        output: dict[str, Any] = {"n": n}
        script = _string_or_none(entry.get("scriptpubkey") or entry.get("script_hex"))
        if script is not None:
            output["scriptpubkey"] = script
        value_sats = _int_or_none(entry.get("value_sats"))
        if value_sats is None:
            value_sats = _value_sats_or_none(entry.get("value"))
        if value_sats is not None:
            output["value"] = value_sats
        raw["vout"].append(output)
    return raw


def _liquid_electrum_decoded_to_graph_raw(
    txid: str,
    decoded: Any,
    raw_hex: str | None = None,
) -> dict[str, Any]:
    raw: dict[str, Any] = {
        "txid": str(txid).lower(),
        "version": _int_or_none(getattr(decoded, "version", None)),
        "locktime": _int_or_none(getattr(decoded, "locktime", None)),
        "vin": [],
        "vout": [],
    }
    if raw_hex:
        raw.update(_size_metadata_from_raw_hex(raw_hex))
    for vin in getattr(decoded, "vin", []) or []:
        try:
            prev_txid = liquid_input_txid(vin)
        except Exception:
            # Some Liquid decoders omit prevout material for coinbase/fee-ish
            # inputs. Keep the reference graph usable and mark the value generic.
            prev_txid = None
        entry: dict[str, Any] = {}
        if prev_txid:
            entry["txid"] = prev_txid.lower()
        prev_vout = _int_or_none(getattr(vin, "vout", None))
        if prev_vout is not None:
            entry["vout"] = prev_vout
        entry["prevout"] = {"value_state": "confidential"}
        raw["vin"].append(entry)
    for index, output in enumerate(getattr(decoded, "vout", []) or []):
        script = getattr(getattr(output, "script_pubkey", None), "data", b"")
        script_hex = bytes(script).hex() if isinstance(script, (bytes, bytearray)) else ""
        entry: dict[str, Any] = {"n": index}
        if script_hex:
            entry["scriptpubkey"] = script_hex
            entry["value_state"] = "confidential"
        else:
            value = _int_or_none(getattr(output, "value", None))
            if value is not None and value >= 0 and not bool(getattr(output, "is_blinded", False)):
                entry["scriptpubkey_type"] = "fee"
                entry["value"] = value
            else:
                entry["value_state"] = "confidential"
        raw["vout"].append(entry)
    return raw


def _sanitize_graph_lookup_raw(
    raw: Mapping[str, Any],
    chain: str,
    txid: str | None = None,
) -> dict[str, Any]:
    sanitized: dict[str, Any] = {}
    raw_txid = _string_or_none(raw.get("txid")) or txid
    if raw_txid:
        sanitized["txid"] = str(raw_txid).lower()
    for key in ("version", "locktime", "size", "vsize", "weight"):
        value = _int_or_none(raw.get(key))
        if value is not None:
            sanitized[key] = value
    vin = raw.get("vin")
    vout = raw.get("vout")
    if isinstance(vin, list):
        sanitized["vin"] = [
            entry
            for entry in (_sanitize_graph_vin(entry, chain) for entry in vin)
            if entry is not None
        ]
    if isinstance(vout, list):
        sanitized["vout"] = [
            entry
            for entry in (_sanitize_graph_vout(entry, chain, index) for index, entry in enumerate(vout))
            if entry is not None
        ]
    return sanitized


def _sanitize_graph_vin(entry: Any, chain: str) -> dict[str, Any] | None:
    if not isinstance(entry, Mapping):
        return None
    sanitized: dict[str, Any] = {}
    prev_txid = _string_or_none(entry.get("txid"))
    if prev_txid:
        sanitized["txid"] = prev_txid.lower()
    prev_vout = _int_or_none(entry.get("vout"))
    if prev_vout is not None:
        sanitized["vout"] = prev_vout
    prevout = entry.get("prevout")
    if isinstance(prevout, Mapping):
        clean_prevout = _sanitize_graph_prevout(prevout, chain)
        if clean_prevout:
            sanitized["prevout"] = clean_prevout
    return sanitized


def _sanitize_graph_prevout(entry: Mapping[str, Any], chain: str) -> dict[str, Any]:
    return _sanitize_graph_value_script(entry, chain, include_n=False)


def _sanitize_graph_vout(entry: Any, chain: str, index: int) -> dict[str, Any] | None:
    if not isinstance(entry, Mapping):
        return None
    sanitized = _sanitize_graph_value_script(entry, chain, include_n=True)
    if sanitized.get("n") is None:
        sanitized["n"] = index
    return sanitized


def _sanitize_graph_value_script(
    entry: Mapping[str, Any],
    chain: str,
    *,
    include_n: bool,
) -> dict[str, Any]:
    sanitized: dict[str, Any] = {}
    if include_n:
        n = _int_or_none(entry.get("n"))
        if n is not None:
            sanitized["n"] = n
    script = _string_or_none(entry.get("scriptpubkey") or entry.get("script_hex"))
    if script is not None:
        sanitized["scriptpubkey"] = script
    script_type = _string_or_none(entry.get("scriptpubkey_type") or entry.get("type"))
    if script_type is not None:
        sanitized["scriptpubkey_type"] = script_type
    address = _string_or_none(entry.get("scriptpubkey_address") or entry.get("address"))
    if address is not None:
        sanitized["scriptpubkey_address"] = address
    confidential = _confidential_leg(entry)
    value_sats = _int_or_none(entry.get("value_sats"))
    if value_sats is None:
        value_sats = _value_sats_or_none(entry.get("value"))
    if confidential:
        sanitized["value_state"] = "confidential"
    elif chain == "liquid" and not _is_liquid_fee_output(sanitized | {"value": value_sats}):
        sanitized["value_state"] = "confidential"
    elif value_sats is not None:
        sanitized["value"] = value_sats
    return sanitized


def _preview_ownership_semantics(
    rows: Sequence[Mapping[str, Any]],
    owned_index: Any | None,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    manual_pair_records: Sequence[Mapping[str, Any]] = (),
) -> dict[str, Any]:
    by_row: dict[str, list[dict[str, Any]]] = defaultdict(list)
    linked_pairs: dict[str, list[dict[str, Any]]] = defaultdict(list)
    working_rows: list[Mapping[str, Any]] = list(rows)
    manual_pair_ids = _manual_pair_ids(manual_pair_records)
    detected_pairs, _detected_ids = detect_intra_transfers(working_rows)
    surviving_detected_pairs = [
        pair
        for pair in detected_pairs
        if str(_row_get(pair.get("out"), "id")) not in manual_pair_ids
        and str(_row_get(pair.get("in"), "id")) not in manual_pair_ids
    ]
    touched: set[str] = set(manual_pair_ids)
    _record_detected_pairs(by_row, linked_pairs, touched, surviving_detected_pairs)

    if owned_index is not None:
        consolidation = derive_multi_source_consolidations(
            working_rows,
            index=owned_index,
            wallet_refs_by_id=wallet_refs_by_id,
            already_paired_ids=set(touched),
        )
        _record_result(by_row, linked_pairs, touched, consolidation, "multi_source_consolidation")
        drop_ids = consolidation.dropped_out_ids | consolidation.dropped_in_ids
        if drop_ids:
            working_rows = [row for row in working_rows if str(_row_get(row, "id")) not in drop_ids]
            working_rows.extend(consolidation.synthetic_rows)
            touched |= drop_ids

        ownership = derive_ownership_transfers(
            working_rows,
            index=owned_index,
            wallet_refs_by_id=wallet_refs_by_id,
            already_paired_ids=set(touched),
        )
        _record_result(by_row, linked_pairs, touched, ownership, "ownership_derived")
        if ownership.dropped_out_ids:
            touched |= ownership.dropped_out_ids
        if ownership.out_row_overrides:
            touched |= set(ownership.out_row_overrides)
        for blocked in ownership.blocked_sources:
            row = blocked.get("row")
            if row is None:
                continue
            row_id = str(_row_get(row, "id"))
            by_row[row_id].append(
                {
                    "code": str(blocked.get("reason") or "ownership_transfer_blocked"),
                    "label": _humanize_code(str(blocked.get("reason") or "ownership_transfer_blocked")),
                    "severity": "warning",
                    "detail": _safe_detail(blocked.get("detail")),
                }
            )

    fanout = derive_recorded_fanout_transfers(
        rows,
        already_paired_ids=set(touched),
    )
    _record_result(by_row, linked_pairs, touched, fanout, "recorded_fanout")
    return {
        "by_row": dict(by_row),
        "linked_pairs": dict(linked_pairs),
        "touched": touched,
    }


def _active_pair_records(conn: sqlite3.Connection, profile_id: str) -> list[sqlite3.Row]:
    return conn.execute(
        """
        SELECT *
        FROM transaction_pairs
        WHERE profile_id = ?
          AND deleted_at IS NULL
        """,
        (profile_id,),
    ).fetchall()


def _manual_pair_ids(manual_pair_records: Sequence[Mapping[str, Any]]) -> set[str]:
    ids: set[str] = set()
    for record in manual_pair_records:
        out_id = _row_get(record, "out_transaction_id")
        in_id = _row_get(record, "in_transaction_id")
        if out_id:
            ids.add(str(out_id))
        if in_id:
            ids.add(str(in_id))
    return ids


def _record_detected_pairs(
    by_row: dict[str, list[dict[str, Any]]],
    linked_pairs: dict[str, list[dict[str, Any]]],
    touched: set[str],
    pairs: Sequence[Mapping[str, Any]],
) -> None:
    for pair in pairs:
        out_row = pair.get("out")
        in_row = pair.get("in")
        out_id = str(_row_get(out_row, "id") or "")
        in_id = str(_row_get(in_row, "id") or "")
        if not out_id or not in_id:
            continue
        external_id = _row_get(out_row, "external_id") or _row_get(in_row, "external_id") or ""
        group_key = normalize_group_txid(str(external_id)) if external_id else out_id
        annotation = {
            "code": "recorded_self_transfer",
            "label": _semantic_label("recorded_self_transfer"),
            "severity": "info",
            "groupId": f"recorded-self-transfer:{group_key}",
            "outTransactionId": out_id,
            "inTransactionId": in_id,
            "amountMsat": int(_row_get(out_row, "amount") or 0),
            "amountBtc": _msat_to_btc(int(_row_get(out_row, "amount") or 0)),
        }
        for row_id in {out_id, in_id}:
            by_row[row_id].append(dict(annotation))
            linked_pairs[row_id].append(dict(annotation))
            touched.add(row_id)


def _record_result(
    by_row: dict[str, list[dict[str, Any]]],
    linked_pairs: dict[str, list[dict[str, Any]]],
    touched: set[str],
    result: Any,
    fallback_source: str,
) -> None:
    for row_id in getattr(result, "dropped_out_ids", set()):
        by_row[str(row_id)].append(
            {
                "code": "dropped_recorded_outbound",
                "label": "Recorded outbound replaced by derived transfer legs",
                "severity": "info",
            }
        )
        touched.add(str(row_id))
    for row_id in getattr(result, "dropped_in_ids", set()):
        by_row[str(row_id)].append(
            {
                "code": "dropped_recorded_destination_receipt",
                "label": "Recorded destination receipt replaced by consolidation legs",
                "severity": "info",
            }
        )
        touched.add(str(row_id))
    for row_id, override in getattr(result, "out_row_overrides", {}).items():
        by_row[str(row_id)].append(
            {
                "code": "partial_external_residual",
                "label": "Owned transfer plus external residual",
                "severity": "info",
                "residualMsat": int(_row_get(override, "amount") or 0),
                "residualBtc": _msat_to_btc(int(_row_get(override, "amount") or 0)),
            }
        )
        touched.add(str(row_id))

    for pair in getattr(result, "derived_pairs", []):
        source = str(pair.get("source") or fallback_source)
        group_id = pair.get("group_id")
        out_row = pair.get("out")
        in_row = pair.get("in")
        out_id = str(_row_get(out_row, "journal_transaction_id") or _row_get(out_row, "id"))
        in_id = str(_row_get(in_row, "journal_transaction_id") or _row_get(in_row, "id"))
        annotation = {
            "code": source,
            "label": _semantic_label(source),
            "severity": "info",
            "groupId": group_id,
            "outTransactionId": out_id,
            "inTransactionId": in_id,
            "amountMsat": int(_row_get(out_row, "amount") or 0),
            "amountBtc": _msat_to_btc(int(_row_get(out_row, "amount") or 0)),
        }
        for row_id in {out_id, in_id}:
            by_row[row_id].append({k: v for k, v in annotation.items() if v is not None})
            linked_pairs[row_id].append({k: v for k, v in annotation.items() if v is not None})
            touched.add(row_id)
        for blocked_row in pair.get("group_block_rows") or ():
            blocked_id = str(_row_get(blocked_row, "id"))
            by_row[blocked_id].append(
                {
                    "code": "derived_transfer_group_blocked_row",
                    "label": "Recorded row belongs to this derived transfer group",
                    "severity": "info",
                    "groupId": group_id,
                }
            )


def _annotate_graph(
    graph: dict[str, Any],
    row: Mapping[str, Any],
    owned_index: Any | None,
    semantics: Mapping[str, Any],
) -> None:
    if owned_index is None:
        return
    source_wallet_id = str(_row_get(row, "wallet_id") or "")
    row_chain_network = _norm_chain_network(*_row_chain_network(row))
    input_owner_ids: set[str] = set()
    for node in graph["inputs"]:
        matches = _filter_matches_to_chain_network(
            _input_matches(node, owned_index), row_chain_network
        )
        _apply_match_annotation(node, matches, "owned_input", "external_input")
        input_owner_ids.update(str(match.wallet_id) for match in matches)

    contributor_ids = (
        input_owner_ids
        if input_owner_ids or not _row_is_outbound(row)
        else {source_wallet_id}
    )
    for node in graph["outputs"]:
        script = node.get("_script")
        if _is_unspendable(script):
            node["ownership"] = "unspendable"
            node["role"] = "op_return"
            node["annotations"].append(_node_annotation("op_return", "OP_RETURN / non-address output"))
            continue
        matches = _filter_matches_to_chain_network(
            owned_index.lookup_script(script), row_chain_network
        )
        owner_ids = {str(match.wallet_id) for match in matches}
        if not matches:
            node["ownership"] = "external"
            node["role"] = "external_recipient"
            node["annotations"].append(_node_annotation("external_recipient", "External recipient"))
            continue
        _apply_match_annotation(node, matches, "owned_output", "external_recipient")
        if len(owner_ids) > 1:
            node["role"] = "ambiguous_owned_output"
            node["annotations"].append(_node_annotation("ambiguous_owned_output", "Owned by multiple wallets"))
        elif owner_ids & contributor_ids:
            node["role"] = "change"
            node["annotations"].append(_node_annotation("change", "Change back to an owned source wallet"))
        elif _row_is_inbound(row) and source_wallet_id in owner_ids:
            node["role"] = "incoming_payment"
            node["annotations"].append(_node_annotation("incoming_payment", "Incoming payment to this wallet"))
        else:
            node["role"] = "owned_destination"
            node["annotations"].append(_node_annotation("owned_destination", "Owned destination wallet"))

    row_id = str(_row_get(row, "id"))
    row_annotations = semantics.get("by_row", {}).get(row_id, [])
    group_ids = {annotation.get("groupId") for annotation in row_annotations if annotation.get("groupId")}
    for node in graph["outputs"]:
        if node.get("role") == "owned_destination":
            for group_id in group_ids:
                node["annotations"].append(
                    _node_annotation("linked_transfer_group", "Linked transfer group", group_id)
                )


def _input_matches(node: Mapping[str, Any], owned_index: Any) -> list[Any]:
    outpoint = node.get("outpoint")
    if outpoint:
        matches = _lookup_outpoint(owned_index, outpoint)
        if matches:
            return matches
    return owned_index.lookup_script(node.get("_script"))


def _lookup_outpoint(owned_index: Any, outpoint: Any) -> list[Any]:
    if hasattr(owned_index, "lookup_outpoint"):
        return list(owned_index.lookup_outpoint(outpoint))
    value = getattr(owned_index, "by_outpoint", {}).get(str(outpoint or "").lower())
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def _filter_matches_to_chain_network(
    matches: Sequence[Any], chain_network: tuple[str, str]
) -> list[Any]:
    return [
        match
        for match in matches
        if _norm_chain_network(getattr(match, "chain", None), getattr(match, "network", None))
        == chain_network
    ]


def _row_is_outbound(row: Mapping[str, Any]) -> bool:
    direction = str(_row_get(row, "direction") or "").lower()
    return direction in {"outbound", "send", "sent", "withdrawal", "sell"}


def _row_is_inbound(row: Mapping[str, Any]) -> bool:
    direction = str(_row_get(row, "direction") or "").lower()
    return direction in {"inbound", "receive", "received", "deposit", "income", "buy"}


def _apply_match_annotation(
    node: dict[str, Any],
    matches: Sequence[Any],
    owned_code: str,
    fallback_code: str,
) -> None:
    if not matches:
        node["ownership"] = "external" if fallback_code.startswith("external") else "unknown"
        return
    wallets = sorted({str(match.wallet_label) for match in matches})
    wallet_ids = sorted({str(match.wallet_id) for match in matches})
    node["ownership"] = "ambiguous" if len(wallet_ids) > 1 else "owned"
    node["wallet"] = wallets[0] if len(wallets) == 1 else ", ".join(wallets)
    node["walletId"] = wallet_ids[0] if len(wallet_ids) == 1 else None
    node["annotations"].append(
        _node_annotation(
            "ambiguous_ownership" if len(wallet_ids) > 1 else owned_code,
            "Owned by multiple wallets" if len(wallet_ids) > 1 else "Owned wallet",
        )
    )


def _transaction_meta(row: Mapping[str, Any], graph: Mapping[str, Any]) -> dict[str, Any]:
    amount_msat = int(_row_get(row, "amount") or 0)
    fee_msat = int(_row_get(row, "fee") or 0)
    external_id = _string_or_none(_row_get(row, "external_id"))
    metadata = graph.get("metadata") or {}
    return {
        "id": str(_row_get(row, "id")),
        "externalId": external_id,
        "txid": _txid_from_row(row),
        "occurredAt": _row_get(row, "occurred_at"),
        "confirmedAt": _row_get(row, "confirmed_at"),
        "direction": _row_get(row, "direction"),
        "asset": _row_get(row, "asset"),
        "amountMsat": amount_msat,
        "amountBtc": _msat_to_btc(amount_msat),
        "feeMsat": fee_msat,
        "feeBtc": _msat_to_btc(fee_msat),
        "wallet": {
            "id": _row_get(row, "wallet_id"),
            "label": _row_get(row, "wallet_label"),
            "kind": _row_get(row, "wallet_kind"),
        },
        "inputCount": metadata.get("inputCount"),
        "outputCount": metadata.get("outputCount"),
        "version": metadata.get("version"),
        "locktime": metadata.get("locktime"),
        "size": metadata.get("size"),
        "vsize": metadata.get("vsize"),
        "weight": metadata.get("weight"),
        "feeRateSatVb": (graph.get("fee") or {}).get("rateSatVb"),
    }


def _warnings_for_row(row: Mapping[str, Any], semantics: Mapping[str, Any]) -> list[dict[str, Any]]:
    warnings: list[dict[str, Any]] = []
    for annotation in semantics.get("by_row", {}).get(str(_row_get(row, "id")), []):
        if annotation.get("severity") == "warning" or str(annotation.get("code", "")).endswith("ambiguous"):
            warnings.append(
                {
                    "code": annotation.get("code"),
                    "level": "warning",
                    "message": annotation.get("label") or _humanize_code(annotation.get("code")),
                }
            )
    return warnings


def _journal_warnings(row: Mapping[str, Any]) -> list[dict[str, Any]]:
    reason = _row_get(row, "quarantine_reason")
    if not reason:
        return []
    return [
        {
            "code": str(reason),
            "level": "warning",
            "message": f"Journal blocker: {_humanize_code(str(reason))}",
        }
    ]


def _quarantine(row: Mapping[str, Any]) -> dict[str, Any] | None:
    reason = _row_get(row, "quarantine_reason")
    if not reason:
        return None
    return {
        "reason": reason,
        "detail": _safe_detail(_json_obj(_row_get(row, "quarantine_detail_json"))),
    }


def _linked_pairs_for_row(row: Mapping[str, Any], semantics: Mapping[str, Any]) -> list[dict[str, Any]]:
    return semantics.get("linked_pairs", {}).get(str(_row_get(row, "id")), [])


def _swap_route_for_row(
    conn: sqlite3.Connection,
    profile_id: str,
    row: Mapping[str, Any],
) -> dict[str, Any] | None:
    row_id = str(_row_get(row, "id") or "")
    if not row_id:
        return None
    pair = conn.execute(
        """
        SELECT
            p.id AS pair_id,
            p.kind AS pair_kind,
            p.policy AS pair_policy,
            p.swap_fee_msat,
            p.swap_fee_kind,
            p.confidence_at_pair,
            p.pair_source,
            p.out_amount AS pair_out_amount_msat,
            p.created_at AS pair_created_at,
            out_t.id AS out_id,
            out_t.external_id AS out_external_id,
            out_t.direction AS out_direction,
            out_t.asset AS out_asset,
            out_t.amount AS out_full_amount_msat,
            COALESCE(p.out_amount, out_t.amount) AS out_amount_msat,
            out_t.fee AS out_fee_msat,
            out_t.occurred_at AS out_occurred_at,
            out_t.confirmed_at AS out_confirmed_at,
            out_t.kind AS out_kind,
            out_t.counterparty AS out_counterparty,
            out_t.description AS out_description,
            out_t.raw_json AS out_raw_json,
            out_w.id AS out_wallet_id,
            out_w.label AS out_wallet_label,
            out_w.kind AS out_wallet_kind,
            in_t.id AS in_id,
            in_t.external_id AS in_external_id,
            in_t.direction AS in_direction,
            in_t.asset AS in_asset,
            in_t.amount AS in_amount_msat,
            in_t.fee AS in_fee_msat,
            in_t.occurred_at AS in_occurred_at,
            in_t.confirmed_at AS in_confirmed_at,
            in_t.kind AS in_kind,
            in_t.counterparty AS in_counterparty,
            in_t.description AS in_description,
            in_t.raw_json AS in_raw_json,
            in_w.id AS in_wallet_id,
            in_w.label AS in_wallet_label,
            in_w.kind AS in_wallet_kind
        FROM transaction_pairs p
        JOIN transactions out_t ON out_t.id = p.out_transaction_id
        JOIN transactions in_t ON in_t.id = p.in_transaction_id
        JOIN wallets out_w ON out_w.id = out_t.wallet_id
        JOIN wallets in_w ON in_w.id = in_t.wallet_id
        WHERE p.profile_id = ?
          AND p.deleted_at IS NULL
          AND (p.out_transaction_id = ? OR p.in_transaction_id = ?)
        ORDER BY p.created_at DESC, p.id DESC
        LIMIT 1
        """,
        (profile_id, row_id, row_id),
    ).fetchone()
    if pair is None:
        return None

    fee_msat = _int_or_none(pair["swap_fee_msat"])
    out_amount_msat = _int_or_none(pair["out_amount_msat"])
    out_full_amount_msat = _int_or_none(pair["out_full_amount_msat"])
    route: dict[str, Any] = {
        "id": pair["pair_id"],
        "kind": pair["pair_kind"],
        "routeKind": _paired_route_kind(pair),
        "policy": pair["pair_policy"],
        "pairSource": pair["pair_source"],
        "confidence": pair["confidence_at_pair"],
        "createdAt": pair["pair_created_at"],
        "currentLeg": "out" if pair["out_id"] == row_id else "in",
        "swapFeeMsat": fee_msat,
        "swapFeeBtc": _msat_to_btc(fee_msat) if fee_msat is not None else None,
        "swapFeeKind": pair["swap_fee_kind"],
        "outAmountMsat": out_amount_msat,
        "outAmountBtc": _msat_to_btc(out_amount_msat) if out_amount_msat is not None else None,
        "outFullAmountMsat": out_full_amount_msat,
        "outFullAmountBtc": (
            _msat_to_btc(out_full_amount_msat)
            if out_full_amount_msat is not None
            else None
        ),
        "out": _swap_route_leg(pair, "out", out_amount_msat),
        "in": _swap_route_leg(pair, "in", _int_or_none(pair["in_amount_msat"])),
    }
    return {key: value for key, value in route.items() if value is not None}


def _paired_route_kind(row: Mapping[str, Any]) -> str:
    pair_kind = str(_row_get(row, "pair_kind") or "").strip().lower()
    if pair_kind == "coinjoin" or "coinjoin" in pair_kind or "whirlpool" in pair_kind:
        return "coinjoin"
    out_asset = str(_row_get(row, "out_asset") or "").strip().upper()
    in_asset = str(_row_get(row, "in_asset") or "").strip().upper()
    if (
        (out_asset and in_asset and out_asset != in_asset)
        or pair_kind in {"peg-in", "peg-out", "submarine-swap", "swap-refund"}
        or "swap" in pair_kind
        or pair_kind.startswith("peg-")
    ):
        return "swap"
    if str(_row_get(row, "pair_policy") or "").strip().lower() == "carrying-value":
        return "transfer"
    return "pair"


def _swap_route_leg(
    row: Mapping[str, Any],
    prefix: str,
    amount_msat: int | None,
) -> dict[str, Any]:
    fee_msat = _int_or_none(_row_get(row, f"{prefix}_fee_msat"))
    asset = _string_or_none(_row_get(row, f"{prefix}_asset"))
    external_id = _string_or_none(_row_get(row, f"{prefix}_external_id"))
    leg = {
        "id": _row_get(row, f"{prefix}_id"),
        "externalId": external_id,
        "txid": external_id.lower() if external_id and len(external_id) == 64 else external_id,
        "direction": _row_get(row, f"{prefix}_direction"),
        "asset": asset,
        "network": _network_label_for_asset_kind(asset, _row_get(row, f"{prefix}_wallet_kind")),
        "amountMsat": amount_msat,
        "amountBtc": _msat_to_btc(amount_msat) if amount_msat is not None else None,
        "feeMsat": fee_msat,
        "feeBtc": _msat_to_btc(fee_msat) if fee_msat is not None else None,
        "occurredAt": _row_get(row, f"{prefix}_occurred_at"),
        "confirmedAt": _row_get(row, f"{prefix}_confirmed_at"),
        "kind": _row_get(row, f"{prefix}_kind"),
        "role": _swap_route_leg_role(row, prefix),
        "counterparty": _string_or_none(_row_get(row, f"{prefix}_counterparty")),
        "description": _string_or_none(_row_get(row, f"{prefix}_description")),
        "wallet": {
            "id": _row_get(row, f"{prefix}_wallet_id"),
            "label": _row_get(row, f"{prefix}_wallet_label"),
            "kind": _row_get(row, f"{prefix}_wallet_kind"),
        },
    }
    return {key: value for key, value in leg.items() if value is not None}


def _swap_route_leg_role(row: Mapping[str, Any], prefix: str) -> str | None:
    if prefix == "out":
        if _swap_route_leg_looks_like_consolidation(row, prefix):
            return "consolidation"
        return "spend"
    if prefix == "in":
        return "receive"
    return None


def _swap_route_leg_looks_like_consolidation(row: Mapping[str, Any], prefix: str) -> bool:
    text = " ".join(
        filter(
            None,
            (
                _string_or_none(_row_get(row, f"{prefix}_kind")),
                _string_or_none(_row_get(row, f"{prefix}_description")),
            ),
        )
    ).lower()
    if "consolidat" in text:
        return True
    raw = _json_obj(_row_get(row, f"{prefix}_raw_json"))
    vin = raw.get("vin")
    return isinstance(vin, list) and len(vin) > 1


def _network_label_for_asset_kind(asset: Any, wallet_kind: Any) -> str | None:
    kind_text = str(wallet_kind or "").lower()
    asset_text = str(asset or "").upper()
    if "liquid" in kind_text or asset_text in {"LBTC", "L-BTC", "LIQUID-BTC"}:
        return "Liquid"
    if asset_text == "BTC":
        return "Bitcoin"
    return asset_text or None


def _safe_detail(detail: Any) -> dict[str, Any]:
    if not isinstance(detail, Mapping):
        return {}
    safe: dict[str, Any] = {}
    for key, value in detail.items():
        key_text = str(key)
        if any(token in key_text.lower() for token in ("descriptor", "xpub", "config", "token", "secret", "raw")):
            continue
        if isinstance(value, (str, int, float, bool)) or value is None:
            safe[key_text] = value
    return safe


def _public_node(node: Mapping[str, Any]) -> dict[str, Any]:
    return {
        key: value
        for key, value in node.items()
        if not str(key).startswith("_") and value is not None
    }


def _fee_from_graph_or_row(
    row: Mapping[str, Any],
    inputs: Sequence[Mapping[str, Any]],
    outputs: Sequence[Mapping[str, Any]],
    metadata: Mapping[str, Any],
    explicit_fee_sats: int | None = None,
) -> dict[str, Any] | None:
    value_sats = None
    if explicit_fee_sats is not None and explicit_fee_sats >= 0:
        # An unblinded on-chain fee output (Liquid) is authoritative.
        value_sats = int(explicit_fee_sats)
    if value_sats is None:
        input_values = [node.get("valueSats") for node in inputs]
        output_values = [node.get("valueSats") for node in outputs]
        if input_values and all(isinstance(value, int) for value in input_values) and all(
            isinstance(value, int) for value in output_values
        ):
            computed = sum(int(value) for value in input_values) - sum(
                int(value) for value in output_values
            )
            if computed >= 0:
                value_sats = computed
    if value_sats is None:
        fee_msat = int(_row_get(row, "fee") or 0)
        if fee_msat <= 0:
            return None
        value_sats = fee_msat // SATS_TO_MSAT
    fee = {
        "id": "fee",
        "label": "Miner fee",
        "role": "fee",
        "ownership": "network_fee",
        "valueSats": value_sats,
        "valueBtc": _sats_to_btc(value_sats),
        "annotations": [_node_annotation("miner_fee", "Network fee")],
    }
    vsize = metadata.get("vsize")
    if value_sats is not None and vsize:
        fee["rateSatVb"] = round(float(value_sats) / float(vsize), 2)
    return fee


def _fee_from_row(row: Mapping[str, Any], metadata: Mapping[str, Any]) -> dict[str, Any] | None:
    return _fee_from_graph_or_row(row, [], [], metadata)


def _graphless_message(reason: str) -> str:
    if reason == "liquid_reference_graph_not_local":
        return (
            "Kassiber has only the imported Liquid row locally. "
            "Add a Liquid explorer backend to inspect public input/output references; confidential amounts may remain hidden."
        )
    return "This source record does not contain Bitcoin input/output references for a flow diagram."


def _partial_value_message(reason: str) -> str:
    if reason == "input_prevout_values_missing":
        return (
            "Bitcoin outputs in this transaction have values. Some input values are not stored locally "
            "because an input amount comes from the spent previous output, not from the transaction input itself. "
            "Kassiber can still draw the references and outputs; fee and fee rate need those prevout amounts."
        )
    return (
        "Amounts are missing for one or more inputs/outputs in the source data. "
        "Kassiber can still show references and ownership hints, but fee and fee rate may be incomplete."
    )


def _confidential_leg(entry: Mapping[str, Any]) -> bool:
    marker = str(entry.get("value_state") or entry.get("valueState") or "").strip().lower()
    if marker == "confidential":
        return True
    return any(
        entry.get(key) is not None
        for key in ("valuecommitment", "assetcommitment", "surjectionproof", "rangeproof")
    )


def _is_liquid_fee_output(entry: Mapping[str, Any]) -> bool:
    """True when a Liquid vout is the dedicated unblinded network-fee output.

    Liquid esplora tags it with ``scriptpubkey_type == "fee"``; otherwise it is
    recognisable as an explicit-value output with no script and no address (an
    OP_RETURN keeps its ``6a`` script, so it is excluded). Only called for
    confidential/Liquid transactions, so Bitcoin outputs never reach here.
    """
    explicit_type = _string_or_none(entry.get("scriptpubkey_type") or entry.get("type"))
    if explicit_type and explicit_type.lower() == "fee":
        return True
    if _confidential_leg(entry):
        return False
    if entry.get("scriptpubkey_address"):
        return False
    if _string_or_none(entry.get("scriptpubkey") or entry.get("script_hex")):
        return False
    return entry.get("value") is not None


def _outpoint(entry: Mapping[str, Any]) -> str | None:
    if entry.get("txid") is None or entry.get("vout") is None:
        return None
    try:
        return f"{str(entry.get('txid')).lower()}:{int(entry.get('vout'))}"
    except (TypeError, ValueError):
        return None


def _script_from_prevout(prevout: Mapping[str, Any]) -> str | None:
    return _string_or_none(prevout.get("scriptpubkey") or prevout.get("script_hex"))


def _script_type(source: Mapping[str, Any], script: Any) -> str:
    explicit = _string_or_none(source.get("scriptpubkey_type") or source.get("type"))
    if explicit:
        return explicit
    script_text = str(script or "").lower()
    if not script_text:
        return "empty"
    if script_text.startswith("6a"):
        return "op_return"
    if script_text.startswith("0014"):
        return "p2wpkh"
    if script_text.startswith("0020"):
        return "p2wsh"
    if script_text.startswith("5120"):
        return "p2tr"
    return "script"


def _is_unspendable(script: Any) -> bool:
    text = str(script or "").strip().lower()
    return not text or text.startswith("6a")


def _txid_from_row(row: Mapping[str, Any]) -> str | None:
    raw = _json_obj(_row_get(row, "raw_json"))
    txid = _string_or_none(raw.get("txid"))
    if txid:
        return txid.lower()
    external_id = _string_or_none(_row_get(row, "external_id"))
    if external_id and len(external_id) == 64:
        return external_id.lower()
    return external_id


def _looks_like_txid(value: Any) -> bool:
    text = str(value or "").strip()
    return len(text) == 64 and all(char in "0123456789abcdefABCDEF" for char in text)


def _transaction_metadata(raw: Mapping[str, Any]) -> dict[str, Any]:
    metadata = {
        "version": _int_or_none(raw.get("version")),
        "locktime": _int_or_none(raw.get("locktime")),
        "size": _int_or_none(raw.get("size")),
        "vsize": _int_or_none(raw.get("vsize")),
        "weight": _int_or_none(raw.get("weight")),
    }
    raw_hex = _string_or_none(raw.get("raw_hex") or raw.get("hex"))
    if raw_hex:
        for key, value in _size_metadata_from_raw_hex(raw_hex).items():
            if metadata.get(key) is None:
                metadata[key] = value
    return metadata


def _size_metadata_from_raw_hex(raw_hex: str) -> dict[str, int]:
    try:
        payload = bytes.fromhex(raw_hex)
    except ValueError:
        return {}
    if len(payload) < 10:
        return {}
    size = len(payload)
    has_witness = len(payload) > 5 and payload[4] == 0 and payload[5] != 0
    if not has_witness:
        return {"size": size, "vsize": size, "weight": size * 4}
    witness_start = _raw_hex_witness_start(payload)
    witness_end = _raw_hex_witness_end(payload, witness_start)
    if witness_start is None or witness_end is None:
        return {"size": size}
    stripped_size = size - 2 - max(0, witness_end - witness_start)
    weight = stripped_size * 3 + size
    return {"size": size, "vsize": (weight + 3) // 4, "weight": weight}


def _raw_hex_witness_start(payload: bytes) -> int | None:
    try:
        offset = 6
        input_count, offset = _read_varint(payload, offset)
        for _ in range(input_count):
            offset += 36
            script_len, offset = _read_varint(payload, offset)
            offset += script_len + 4
        output_count, offset = _read_varint(payload, offset)
        for _ in range(output_count):
            offset += 8
            script_len, offset = _read_varint(payload, offset)
            offset += script_len
    except (IndexError, ValueError):
        return None
    return offset if 0 <= offset <= len(payload) else None


def _raw_hex_witness_end(payload: bytes, offset: int | None) -> int | None:
    if offset is None:
        return None
    try:
        input_count, input_offset = _read_varint(payload, 6)
        for _ in range(input_count):
            input_offset += 36
            script_len, input_offset = _read_varint(payload, input_offset)
            input_offset += script_len + 4
        output_count, input_offset = _read_varint(payload, input_offset)
        for _ in range(output_count):
            input_offset += 8
            script_len, input_offset = _read_varint(payload, input_offset)
            input_offset += script_len
        if input_offset != offset:
            return None
        for _ in range(input_count):
            item_count, offset = _read_varint(payload, offset)
            for _ in range(item_count):
                item_len, offset = _read_varint(payload, offset)
                offset += item_len
    except (IndexError, ValueError):
        return None
    locktime_end = offset + 4
    return offset if locktime_end == len(payload) else None


def _read_varint(payload: bytes, offset: int) -> tuple[int, int]:
    if offset >= len(payload):
        raise ValueError("offset out of range")
    prefix = payload[offset]
    offset += 1
    if prefix < 0xFD:
        return prefix, offset
    if prefix == 0xFD:
        end = offset + 2
        if end > len(payload):
            raise ValueError("truncated varint")
        return int.from_bytes(payload[offset:end], "little"), end
    if prefix == 0xFE:
        end = offset + 4
        if end > len(payload):
            raise ValueError("truncated varint")
        return int.from_bytes(payload[offset:end], "little"), end
    end = offset + 8
    if end > len(payload):
        raise ValueError("truncated varint")
    return int.from_bytes(payload[offset:end], "little"), end


def _looks_liquid_or_confidential(row: Mapping[str, Any], raw: Mapping[str, Any]) -> bool:
    asset = str(_row_get(row, "asset") or "").upper()
    if asset in {"LBTC", "L-BTC", "LIQUID-BTC"}:
        return True
    wallet_kind = str(_row_get(row, "wallet_kind") or "").lower()
    if "liquid" in wallet_kind:
        return True
    # Inspect leg structure rather than serialising the whole transaction: this
    # avoids the cost of dumping large Bitcoin transactions and the false
    # positives a substring match would hit on memo/description text.
    return _raw_has_confidential_legs(raw)


def _raw_has_confidential_legs(raw: Mapping[str, Any]) -> bool:
    for key in ("vin", "vout"):
        legs = raw.get(key)
        if not isinstance(legs, list):
            continue
        for leg in legs:
            if not isinstance(leg, Mapping):
                continue
            if _confidential_leg(leg):
                return True
            prevout = leg.get("prevout")
            if isinstance(prevout, Mapping) and _confidential_leg(prevout):
                return True
    return False


def _row_chain_network(
    row: Mapping[str, Any],
    *,
    default_chain: str = "bitcoin",
    default_network: str = "main",
) -> tuple[str, str]:
    config = _json_obj(_row_get(row, "wallet_config_json"))
    chain = str(config.get("chain") or default_chain).lower()
    network = str(config.get("network") or default_network).lower()
    asset = str(_row_get(row, "asset") or "").upper()
    wallet_kind = str(_row_get(row, "wallet_kind") or "").lower()
    if "liquid" in wallet_kind or asset in {"LBTC", "L-BTC", "LIQUID-BTC"}:
        chain = "liquid"
        if network in {"", "main", "mainnet"}:
            network = "liquidv1"
    elif chain in {"", "btc"}:
        chain = "bitcoin"
    return _norm_chain_network(chain, network)


def _json_obj(value: Any) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return dict(parsed) if isinstance(parsed, Mapping) else {}


def _int_or_none(value: Any) -> int | None:
    if value is None or value == "":
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _value_sats_or_none(value: Any) -> int | None:
    if value is None or value == "":
        return None
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        # On-chain JSON encodes integer sats as ints; a float (e.g. 0.005 or a
        # round 1.0) always denotes a decimal-BTC amount, so scale to sats. Note
        # `1.0` is BTC, not 1 sat — never short-circuit whole-number floats.
        return int(round(value * 100_000_000))
    if isinstance(value, str):
        text = value.strip()
        if not text:
            return None
        try:
            if "." in text:
                return int(round(float(text) * 100_000_000))
            return int(text)
        except ValueError:
            return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _string_or_none(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _sats_to_btc(value_sats: Any) -> float | None:
    if value_sats is None:
        return None
    return int(value_sats) / 100_000_000


def _msat_to_btc(value_msat: int) -> float:
    return float(msat_to_btc(int(value_msat)))


def _row_get(row: Mapping[str, Any], key: str) -> Any:
    try:
        keys = row.keys()
    except AttributeError:
        return None
    return row[key] if key in keys else None


def _node_annotation(code: str, label: str, group_id: Any = None) -> dict[str, Any]:
    item = {"code": code, "label": label}
    if group_id is not None:
        item["groupId"] = group_id
    return item


def _semantic_label(source: str) -> str:
    labels = {
        "ownership_derived": "Ownership-derived transfer",
        "recorded_self_transfer": "Recorded self-transfer",
        "recorded_fanout": "Recorded fan-out transfer",
        "multi_source_consolidation": "Multi-wallet consolidation",
    }
    return labels.get(source, _humanize_code(source))


def _humanize_code(code: Any) -> str:
    return str(code or "").replace("_", " ").replace("-", " ").strip().capitalize()


def _dedupe_warnings(warnings: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    seen: set[tuple[str, str]] = set()
    out: list[dict[str, Any]] = []
    for warning in warnings:
        code = str(warning.get("code") or "")
        message = str(warning.get("message") or _humanize_code(code))
        key = (code, message)
        if key in seen:
            continue
        seen.add(key)
        out.append(
            {
                "code": code,
                "level": str(warning.get("level") or "info"),
                "message": message,
            }
        )
    return out
