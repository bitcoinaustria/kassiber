"""Classify on-chain transactions that are Lightning channel opens/closes.

Opening a channel moves the operator's own BTC from their on-chain wallet into a
2-of-2 output they co-control — the coins stay owned, so it is NOT a disposal.
Closing returns them — NOT an acquisition; the basis carries. But an on-chain
backend that happens to watch the node's on-chain addresses (a "dual-sync"
setup: an LN adapter for the node PLUS a separate wallet for its L1 UTXOs) sees
the funding tx as a plain send (disposal) and the close as a plain receive
(acquisition), which mis-taxes both.

This module does NOT import channel transactions (that would double-count
against the L1 wallet). Instead it derives, from the owned channels' funding and
closing txids, a ``transaction_id -> role`` map that the tax engine consumes via
the same non-event suppression it uses for loan collateral lock/release
(``kassiber.core.loans`` CHANNEL_OPEN / CHANNEL_CLOSE). If no L1 wallet recorded
the channel tx, the map is empty and nothing changes (correct).

The funding txid is taken from each channel's ``funding_outpoint`` (always
captured). The closing txid is best-effort — see the adapter capture — so the
close side only fires when a closing txid is known.
"""

from __future__ import annotations

import json
from typing import Any, Iterable, Mapping, TypeAlias

from ...transfers import canonical_txid, onchain_transfer_scope
from ..loans import (
    CHANNEL_CLOSE,
    CHANNEL_CLOSE_MISMATCH,
    CHANNEL_OPEN,
    CHANNEL_OPEN_MISMATCH,
)
from ..transfer_matching import DEFAULT_FEE_PCT_MAX, DEFAULT_FEE_SATS_MIN, fee_threshold_msat

CHANNEL_RECORD_TYPE = "channel"
_ATOMIC_CHANNEL_ID_PREFIXES = ("lnd:", "coreln:")
LifecycleScope: TypeAlias = tuple[str, str, str]


def _field(row: Any, key: str) -> Any:
    """Read ``key`` from a sqlite3.Row-like, dict, or object row."""
    if isinstance(row, Mapping):
        return row.get(key)
    try:
        keys = row.keys()
    except AttributeError:
        return getattr(row, key, None)
    return row[key] if key in keys else None


def _txid_from_outpoint(value: Any) -> str | None:
    """Return the canonical txid from ``<txid>:<vout>`` or a bare txid."""
    text = str(value or "").strip()
    if not text:
        return None
    direct = canonical_txid(text)
    if direct is not None:
        return direct
    outpoint = _canonical_outpoint(text)
    return outpoint[0] if outpoint is not None else None


def _canonical_outpoint(value: Any) -> tuple[str, int] | None:
    """Return a canonical ``(txid, vout)`` pair, or ``None``."""

    text = str(value or "").strip()
    if ":" not in text:
        return None
    txid_text, vout_text = text.rsplit(":", 1)
    txid = canonical_txid(txid_text)
    if txid is None:
        return None
    try:
        vout = int(vout_text)
    except (TypeError, ValueError):
        return None
    return (txid, vout) if vout >= 0 else None


def _transaction_lifecycle_scope(row: Any) -> LifecycleScope | None:
    """Physical Bitcoin identity for one stored on-chain transaction row.

    The shared transfer scope treats the raw transaction graph as authoritative,
    rejects a graph txid that contradicts a canonical ``external_id``, and keeps
    identical txids on different networks separate.  Lifecycle evidence is BTC
    only; the asset default exists solely for legacy/minimal pure-function rows.
    """

    probe = _row_dict(row)
    if not str(probe.get("asset") or "").strip():
        probe["asset"] = "BTC"
    scope = onchain_transfer_scope(probe)
    if scope is None:
        return None
    chain, network, txid, asset = scope
    if chain != "bitcoin" or asset != "BTC":
        return None
    return (chain, network, txid)


def _channel_lifecycle_scope(
    row: Any,
    txid: str | None,
) -> LifecycleScope | None:
    """Scope adapter-authored lifecycle evidence to one Bitcoin network.

    Channel records are metadata rather than raw L1 transactions. Their wallet
    config, optional direct backend ``chain`` / ``network`` fields, and the
    adapter-observed scope persisted in ``raw_json`` are independent sources.
    Feeding all three through ``onchain_transfer_scope`` makes contradictory or
    unsupported metadata fail closed while allowing complementary partial
    metadata (for example backend ``chain`` plus observed ``network``).
    """

    if txid is None:
        return None
    direct_scope = {
        key: value
        for key in ("chain", "network")
        if (value := _field(row, key)) not in (None, "")
    }
    observed = _raw_mapping(row)
    observed_scope = {
        key: value
        for key in ("chain", "network")
        if (value := observed.get(key)) not in (None, "")
    }
    # ``onchain_transfer_scope`` already treats root and ownership-graph
    # metadata as separate evidence sources and rejects conflicting values.
    # This is only an in-memory validation probe; no fictitious ownership graph
    # is persisted on the lifecycle record.
    evidence_scope = dict(observed_scope)
    if direct_scope:
        evidence_scope["ownership_graph"] = direct_scope
    probe = {
        "id": f"channel-lifecycle:{txid}",
        "external_id": txid,
        # This value comes from the adapter's dedicated lifecycle ``txid`` /
        # ``outpoint`` field, never its generic provider external id.
        "external_id_kind": "txid",
        "asset": "BTC",
        "config_json": (
            _field(row, "config_json")
            or _field(row, "wallet_config_json")
            or {}
        ),
        "raw_json": evidence_scope,
    }
    scope = onchain_transfer_scope(probe)
    if scope is None:
        return None
    chain, network, scoped_txid, asset = scope
    if chain != "bitcoin" or asset != "BTC":
        return None
    return (chain, network, scoped_txid)


def _funding_lifecycle_scope(row: Any) -> LifecycleScope | None:
    """Return a funding scope only when txid/outpoint evidence agrees."""

    txid_value = _field(row, "funding_txid")
    outpoint_value = _field(row, "funding_outpoint")
    txid = _txid_from_outpoint(txid_value) if txid_value not in (None, "") else None
    outpoint_txid = (
        _txid_from_outpoint(outpoint_value)
        if outpoint_value not in (None, "")
        else None
    )
    if txid_value not in (None, "") and txid is None:
        return None
    if outpoint_value not in (None, "") and outpoint_txid is None:
        return None
    if txid is not None and outpoint_txid is not None and txid != outpoint_txid:
        return None
    return _channel_lifecycle_scope(row, txid or outpoint_txid)


def _closing_lifecycle_scope(row: Any) -> LifecycleScope | None:
    value = _field(row, "closing_txid")
    if value in (None, ""):
        return None
    return _channel_lifecycle_scope(row, canonical_txid(value))


def _raw_mapping(row: Any) -> Mapping[str, Any]:
    raw = _field(row, "raw_json")
    if isinstance(raw, Mapping):
        return raw
    if not raw:
        return {}
    try:
        payload = json.loads(str(raw))
    except (TypeError, ValueError):
        return {}
    return payload if isinstance(payload, Mapping) else {}


def _vin_outpoints(row: Any) -> set[tuple[str, int]]:
    """Canonical prevouts spent by one stored on-chain transaction row."""

    payload = _raw_mapping(row)
    nested = payload.get("tx")
    if isinstance(nested, Mapping):
        payload = nested
    vin = payload.get("vin")
    if not isinstance(vin, list):
        return set()
    outpoints: set[tuple[str, int]] = set()
    for entry in vin:
        if not isinstance(entry, Mapping):
            continue
        outpoint = _canonical_outpoint(
            f"{entry.get('txid')}:{entry.get('vout')}"
        )
        if outpoint is not None:
            outpoints.add(outpoint)
    return outpoints


def _trusted_local_close_sweeps(
    row: Any,
) -> set[tuple[str, tuple[str, int]]]:
    """Adapter-attested ``(sweep txid, commitment outpoint)`` evidence.

    This evidence belongs to the node adapter's curated lifecycle record, not
    to a user-importable transaction ``raw_json``.  Requiring both identities
    means a CSV row cannot suppress an acquisition merely by claiming that it
    spends a known commitment output.
    """

    payload = _raw_mapping(row)
    provenance = payload.get("_kassiber_provenance")
    if not isinstance(provenance, Mapping) or provenance.get(
        "import_source"
    ) not in {"lnd_adapter"}:
        return set()
    values = payload.get("channel_close_local_sweeps")
    if not isinstance(values, list):
        return set()
    evidence: set[tuple[str, tuple[str, int]]] = set()
    for value in values:
        if not isinstance(value, Mapping):
            continue
        sweep_txid = canonical_txid(value.get("sweep_txid"))
        outpoint = _canonical_outpoint(value.get("outpoint"))
        if sweep_txid is not None and outpoint is not None:
            evidence.add((sweep_txid, outpoint))
    return evidence


def _close_balance_mismatch(received_msat: int, balance_msat: int) -> bool:
    """True when the settled-balance gap is not a plausible close fee.

    The synthesized close pair clones the receipt row, so the generic
    transfer-fee implausibility guard (out.amount - in.amount) is
    definitionally zero for it and can never fire — this check is the ONLY
    ceiling between a mis-captured close balance (unsynced sweep, HTLC value
    lost to the peer) and an unbounded silent "fee" disposal. Symmetric:
    receiving clearly MORE than the settled balance is just as much a data
    problem as receiving less.
    """
    if balance_msat < 0:
        return True
    if balance_msat == 0:
        return False
    tolerance = fee_threshold_msat(
        balance_msat, DEFAULT_FEE_PCT_MAX, DEFAULT_FEE_SATS_MIN
    )
    return abs(balance_msat - received_msat) > tolerance


def _close_leg_groups(
    closing_keys: set[LifecycleScope] | Mapping[LifecycleScope, Any],
    close_balance_by_scope: Mapping[LifecycleScope, int],
    tx_rows,
    local_sweeps_by_scope: Mapping[
        LifecycleScope, set[tuple[str, tuple[str, int]]]
    ] | None = None,
) -> dict[LifecycleScope, dict[str, Any]]:
    """Group inbound close candidates per scoped close, classified TOGETHER.

    A close can pay the wallet in several legs (coop payout + timelocked
    to_local sweep + per-HTLC sweeps). The settled balance minus the group's
    TOTAL receipt is the single close fee — evaluating legs one at a time
    would book every other leg's amount as a "fee" once per leg.

    Merely spending an output of the commitment transaction never proves that
    the output belonged to this node: the peer may later spend its output to
    pay us. Every vin-matched sweep therefore requires an explicit local
    commitment-outpoint marker. Without it the candidate group is marked
    mismatched and no row is suppressed as a close.
    """
    candidates: dict[
        LifecycleScope,
        list[tuple[tuple, bool, frozenset[tuple[str, int]], Any]],
    ] = {}
    for tx in tx_rows:
        if _field(tx, "direction") != "inbound":
            continue
        key = _transaction_lifecycle_scope(tx)
        if key is None:
            # A contradictory raw graph / external id must never suppress the
            # row or synthesize a node MOVE under either asserted identity.
            continue
        close_key = key if key is not None and key in closing_keys else None
        matched_by_txid = close_key is not None
        vin_outpoints = _vin_outpoints(tx)
        vin_close_keys = sorted(
            {
                (key[0], key[1], txid)
                for txid, _vout in vin_outpoints
                if (key[0], key[1], txid) in closing_keys
            }
        )
        candidate_keys = [close_key] if close_key is not None else vin_close_keys
        if not candidate_keys:
            continue
        sort_key = (str(_field(tx, "occurred_at") or ""), str(_field(tx, "id")))
        for candidate_key in candidate_keys:
            trusted_sweeps = (local_sweeps_by_scope or {}).get(
                candidate_key, set()
            )
            local_outpoints = frozenset(
                outpoint
                for outpoint in vin_outpoints
                if outpoint[0] == candidate_key[2]
                and (key[2], outpoint) in trusted_sweeps
            )
            candidates.setdefault(candidate_key, []).append(
                (sort_key, matched_by_txid, local_outpoints, tx)
            )

    groups: dict[LifecycleScope, dict[str, Any]] = {}
    for close_key, entries in candidates.items():
        entries.sort(key=lambda item: item[0])
        balance = int(close_balance_by_scope.get(close_key, 0))
        direct = [entry for entry in entries if entry[1]]
        vin_matches = [entry for entry in entries if not entry[1]]
        selected = list(direct)
        direct_total = sum(int(_field(entry[3], "amount") or 0) for entry in direct)
        ambiguous = False
        if len(vin_matches) == 1:
            entry = vin_matches[0]
            # If a direct close payout already accounts for the whole settled
            # balance, the vin match is a later peer-output payment and stays
            # ordinary income. Otherwise uniqueness is not ownership proof:
            # require the exact local commitment outpoint or fail closed.
            if balance > 0 and direct_total >= balance:
                pass
            elif entry[2]:
                selected.append(entry)
            else:
                selected.append(entry)
                ambiguous = True
        elif len(vin_matches) > 1:
            locally_evidenced = [entry for entry in vin_matches if entry[2]]
            seen_local_outpoints: set[tuple[str, int]] = set()
            duplicate_local_outpoint = False
            for entry in locally_evidenced:
                if seen_local_outpoints & entry[2]:
                    duplicate_local_outpoint = True
                seen_local_outpoints.update(entry[2])
            if locally_evidenced and not duplicate_local_outpoint:
                selected.extend(locally_evidenced)
            else:
                # Chronological first-fit is not evidence: either candidate may
                # spend the peer commitment output. Likewise, two transactions
                # claiming the same local prevout are contradictory. Keep every
                # competing row in the mismatch group so none is suppressed.
                selected.extend(vin_matches)
                ambiguous = True
        legs = [entry[3] for entry in selected]
        total = sum(int(_field(tx, "amount") or 0) for tx in legs)
        groups[close_key] = {
            "legs": legs,
            "total_msat": total,
            "balance_msat": balance,
            "ambiguous": ambiguous,
            "mismatch": ambiguous or _close_balance_mismatch(total, balance),
        }
    return groups


def _funding_amount_mismatch(
    tx: Any, funded_msat: int, *, strict: bool = False
) -> bool:
    """True when the outbound's amount exceeds the funded amount implausibly.

    ``amount`` on node-backed rows excludes change and the miner fee, so any
    excess over the funded channel balance is value to a non-channel output.
    The tolerance mirrors the transfer-fee ceiling so ordinary rounding and
    fee-convention noise never trips it.
    """
    if funded_msat < 0:
        return True
    if funded_msat == 0:
        return False
    recorded = int(_field(tx, "amount") or 0)
    if strict:
        # Amount-bearing LND evidence is sat-exact. Any principal residual is a
        # co-payment/change/lease ambiguity, not fee-convention noise, and the
        # whole L1 row must remain unresolved rather than suppressed.
        return recorded != funded_msat
    return recorded - funded_msat > fee_threshold_msat(
        funded_msat, DEFAULT_FEE_PCT_MAX, DEFAULT_FEE_SATS_MIN
    )


def _requires_atomic_pair(row: Any) -> bool:
    """Whether this adapter marks lifecycle evidence as strict/amount-bearing.

    Modern LND and Core Lightning records use namespaced identities. Unlike
    legacy zero-amount metadata, they may suppress an L1 row only when an
    amount-bearing compensating node MOVE can be constructed in the same
    journal build.
    """
    channel_id = str(_field(row, "channel_id") or "")
    return channel_id.startswith(_ATOMIC_CHANNEL_ID_PREFIXES)


def channel_role_map(
    channel_rows: Iterable[Any],
    tx_rows: Iterable[Any],
) -> dict[str, str]:
    """Return ``{transaction_id: CHANNEL_OPEN|CHANNEL_CLOSE}`` for on-chain txs.

    ``channel_rows`` are persisted channel records exposing ``funding_txid`` /
    ``funding_outpoint`` and (optionally) ``closing_txid``. ``tx_rows`` are
    transaction rows exposing ``id``, ``external_id`` and ``direction``. Every
    automatic identity is canonical and scoped by chain + network; equal txids
    on mainnet and regtest are unrelated.
    """
    channel_rows = list(channel_rows)
    tx_rows = list(tx_rows)
    funding: set[LifecycleScope] = set()
    closing: set[LifecycleScope] = set()
    atomic_funding: set[LifecycleScope] = set()
    atomic_closing: set[LifecycleScope] = set()
    funding_amount_by_scope: dict[LifecycleScope, int] = {}
    close_balance_by_scope: dict[LifecycleScope, int] = {}
    local_sweeps_by_scope: dict[
        LifecycleScope, set[tuple[str, tuple[str, int]]]
    ] = {}
    for row in channel_rows:
        fund_key = _funding_lifecycle_scope(row)
        if fund_key is not None:
            funding.add(fund_key)
            if _requires_atomic_pair(row):
                atomic_funding.add(fund_key)
            funded = int(_field(row, "funding_amount_msat") or 0)
            if funded > 0:
                # A batched open (multifundchannel) shares one funding tx
                # across N channel records: SUM the funded amounts, or a
                # clean batched open false-positives the mismatch guard.
                funding_amount_by_scope[fund_key] = (
                    funding_amount_by_scope.get(fund_key, 0) + funded
                )
        close_key = _closing_lifecycle_scope(row)
        if close_key is not None:
            closing.add(close_key)
            local_sweeps_by_scope.setdefault(close_key, set()).update(
                _trusted_local_close_sweeps(row)
            )
            if _requires_atomic_pair(row):
                atomic_closing.add(close_key)
            balance = int(_field(row, "close_balance_msat") or 0)
            if balance > 0:
                # Several channels can share one close/sweep txid (batched
                # opens closing together): sum our settled balances.
                close_balance_by_scope[close_key] = (
                    close_balance_by_scope.get(close_key, 0) + balance
                )

    # Strict adapters get normal suppressing roles only for rows that also have
    # an explicit compensating MOVE.  Build those pairs against minimal wallet
    # refs so role classification and pair construction are atomic even though
    # the public API exposes them as two pure functions.
    atomic_pair_row_ids: set[str] = set()
    atomic_rows = [row for row in channel_rows if _requires_atomic_pair(row)]
    if atomic_rows:
        wallet_refs = {
            str(_field(row, "wallet_id")): {
                "id": str(_field(row, "wallet_id")),
                "label": str(_field(row, "wallet_id")),
                "wallet_account_id": None,
                "account_code": None,
                "account_label": None,
            }
            for row in atomic_rows
            if _field(row, "wallet_id")
        }
        for pair in channel_transfer_pairs(atomic_rows, tx_rows, wallet_refs):
            if pair["kind"] == CHANNEL_OPEN:
                atomic_pair_row_ids.add(str(_field(pair["out"], "id")))
            elif pair["kind"] == CHANNEL_CLOSE:
                atomic_pair_row_ids.add(str(_field(pair["in"], "id")))

    roles: dict[str, str] = {}
    # Direct payout from the close tx (coop close / to_remote) matches by
    # txid; a force-close's timelocked sweep matches by spending the
    # commitment tx. All legs of one close are classified TOGETHER.
    if closing:
        for close_key, group in _close_leg_groups(
            closing,
            close_balance_by_scope,
            tx_rows,
            local_sweeps_by_scope,
        ).items():
            for tx in group["legs"]:
                tx_id = str(_field(tx, "id"))
                mismatch = group["mismatch"] or (
                    close_key in atomic_closing and tx_id not in atomic_pair_row_ids
                )
                roles[tx_id] = (
                    CHANNEL_CLOSE_MISMATCH if mismatch else CHANNEL_CLOSE
                )
    for tx in tx_rows:
        key = _transaction_lifecycle_scope(tx)
        if key is None:
            continue
        direction = _field(tx, "direction")
        tx_id = str(_field(tx, "id"))
        # The funding tx leaves the on-chain wallet (outbound). Guarding on
        # direction avoids mislabeling a change/receive leg that happens to
        # share the txid.
        if direction == "outbound" and key in funding:
            if (
                _funding_amount_mismatch(
                    tx,
                    funding_amount_by_scope.get(key, 0),
                    strict=key in atomic_funding,
                )
                or (key in atomic_funding and tx_id not in atomic_pair_row_ids)
            ):
                # The recorded outflow clearly exceeds the funded amount: the
                # tx ALSO paid an external recipient. Suppressing the whole
                # row would silently untax that payment — flag for review.
                roles[tx_id] = CHANNEL_OPEN_MISMATCH
                continue
            roles[tx_id] = CHANNEL_OPEN
    return roles


def channel_transfer_pairs(
    channel_rows: Iterable[Any],
    tx_rows: Iterable[Any],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
) -> list[dict[str, Any]]:
    """Return explicit same-asset pairs for channel capacity moves.

    ``channel_role_map`` only suppresses the L1 row as a non-event. That preserves
    profile-wide holdings but strands the capacity in the funding wallet. When the
    channel metadata carries the node wallet id, synthesize the missing other leg
    so funding becomes an on-chain-wallet -> node MOVE and cooperative close
    becomes node -> on-chain-wallet.
    """
    funding_wallet_by_scope: dict[LifecycleScope, str] = {}
    closing_wallet_by_scope: dict[LifecycleScope, str] = {}
    ambiguous_funding_scopes: set[LifecycleScope] = set()
    ambiguous_closing_scopes: set[LifecycleScope] = set()
    close_balance_by_scope: dict[LifecycleScope, int] = {}
    local_sweeps_by_scope: dict[
        LifecycleScope, set[tuple[str, tuple[str, int]]]
    ] = {}
    funding_amount_by_scope: dict[LifecycleScope, int] = {}
    strict_funding_scopes: set[LifecycleScope] = set()
    close_funding_by_scope: dict[LifecycleScope, set[LifecycleScope]] = {}
    channel_funding_by_owner: dict[
        tuple[str, str], set[LifecycleScope]
    ] = {}
    close_channels_by_scope: dict[
        LifecycleScope, set[tuple[str, str]]
    ] = {}

    def _remember_owner(
        owners: dict[LifecycleScope, str],
        ambiguous: set[LifecycleScope],
        key: LifecycleScope,
        wallet_id: str,
    ) -> None:
        existing = owners.get(key)
        if existing is None:
            if key not in ambiguous:
                owners[key] = wallet_id
            return
        if existing != wallet_id:
            ambiguous.add(key)
            owners.pop(key, None)

    for row in channel_rows:
        wallet_id = _field(row, "wallet_id")
        if not wallet_id or str(wallet_id) not in wallet_refs_by_id:
            continue
        wallet_id = str(wallet_id)
        channel_id = _field(row, "channel_id")
        channel_owner = (
            (wallet_id, str(channel_id))
            if channel_id not in (None, "")
            else None
        )
        fund_key = _funding_lifecycle_scope(row)
        if fund_key is not None:
            funded = int(_field(row, "funding_amount_msat") or 0)
            if _requires_atomic_pair(row):
                strict_funding_scopes.add(fund_key)
            if _requires_atomic_pair(row) and funded <= 0:
                funding_wallet_by_scope.pop(fund_key, None)
                ambiguous_funding_scopes.add(fund_key)
            else:
                _remember_owner(
                    funding_wallet_by_scope,
                    ambiguous_funding_scopes,
                    fund_key,
                    wallet_id,
                )
            if channel_owner is not None:
                channel_funding_by_owner.setdefault(channel_owner, set()).add(fund_key)
            if funded > 0:
                # A batched open (multifundchannel) shares one funding tx
                # across N channel records: SUM the funded amounts, or a
                # clean batched open false-positives the mismatch guard.
                funding_amount_by_scope[fund_key] = (
                    funding_amount_by_scope.get(fund_key, 0) + funded
                )
        close_key = _closing_lifecycle_scope(row)
        if close_key is not None:
            local_sweeps_by_scope.setdefault(close_key, set()).update(
                _trusted_local_close_sweeps(row)
            )
            balance = int(_field(row, "close_balance_msat") or 0)
            if _requires_atomic_pair(row) and balance <= 0:
                closing_wallet_by_scope.pop(close_key, None)
                ambiguous_closing_scopes.add(close_key)
            else:
                _remember_owner(
                    closing_wallet_by_scope,
                    ambiguous_closing_scopes,
                    close_key,
                    wallet_id,
                )
            if fund_key is not None:
                close_funding_by_scope.setdefault(close_key, set()).add(fund_key)
            if channel_owner is not None:
                close_channels_by_scope.setdefault(close_key, set()).add(channel_owner)
            if balance > 0:
                # Several channels can share one close/sweep txid (batched
                # opens closing together): sum our settled balances.
                close_balance_by_scope[close_key] = (
                    close_balance_by_scope.get(close_key, 0) + balance
                )

    for close_key, channel_owners in close_channels_by_scope.items():
        linked_funding: set[LifecycleScope] = set()
        for channel_owner in channel_owners:
            linked_funding.update(channel_funding_by_owner.get(channel_owner, set()))
        linked_funding = {
            funding_key
            for funding_key in linked_funding
            if funding_key[:2] == close_key[:2]
        }
        if linked_funding:
            close_funding_by_scope.setdefault(close_key, set()).update(linked_funding)

    tx_rows = list(tx_rows)
    pairs: list[dict[str, Any]] = []
    opened_funding_scopes: set[LifecycleScope] = set()
    for tx in tx_rows:
        tx_id = str(_field(tx, "id"))
        key = _transaction_lifecycle_scope(tx)
        if key is None:
            continue
        direction = _field(tx, "direction")
        amount = int(_field(tx, "amount") or 0)
        if amount <= 0:
            continue
        if direction == "outbound" and key in funding_wallet_by_scope:
            if _funding_amount_mismatch(
                tx,
                funding_amount_by_scope.get(key, 0),
                strict=key in strict_funding_scopes,
            ):
                # role map flags this row CHANNEL_OPEN_MISMATCH; a synthesized
                # MOVE would absorb the external payment as channel capacity.
                continue
            opened_funding_scopes.add(key)
            node_wallet_id = funding_wallet_by_scope[key]
            in_row = _clone_channel_leg(
                tx,
                wallet_refs_by_id[node_wallet_id],
                row_id=f"channel-open:{tx_id}:in:{node_wallet_id}",
                direction="inbound",
                fee=0,
            )
            pairs.append(
                {
                    "out": tx,
                    "in": in_row,
                    "source": "channel_lifecycle",
                    "kind": CHANNEL_OPEN,
                }
            )

    if not closing_wallet_by_scope:
        return pairs
    eligible_closing_wallet_by_scope = {
        close_key: wallet_id
        for close_key, wallet_id in closing_wallet_by_scope.items()
        if close_key not in ambiguous_closing_scopes
        and bool(close_funding_by_scope.get(close_key, set()) & opened_funding_scopes)
    }
    if not eligible_closing_wallet_by_scope:
        return pairs
    for close_key, group in _close_leg_groups(
        eligible_closing_wallet_by_scope,
        close_balance_by_scope,
        tx_rows,
        local_sweeps_by_scope,
    ).items():
        if group["mismatch"]:
            # role map flags these legs CHANNEL_CLOSE_MISMATCH for quarantine;
            # a synthesized MOVE would book the whole gap as an unbounded
            # "fee" (the generic implausibility guard cannot fire on a
            # cloned-amount pair).
            continue
        node_wallet_id = eligible_closing_wallet_by_scope[close_key]
        # When our settled channel balance at close is known, the gap to the
        # GROUP's total receipt is the single close fee (commitment + sweep
        # miner fees). It rides on the largest leg so the node wallet is
        # debited fully instead of stranding the difference — booking it per
        # leg would count every other leg's amount as a "fee" once each.
        close_fee = 0
        balance = group["balance_msat"]
        if balance > group["total_msat"]:
            close_fee = balance - group["total_msat"]
        fee_leg_id = None
        if close_fee > 0 and group["legs"]:
            fee_leg_id = str(
                _field(
                    max(
                        group["legs"],
                        key=lambda leg: (
                            int(_field(leg, "amount") or 0),
                            str(_field(leg, "id")),
                        ),
                    ),
                    "id",
                )
            )
        for tx in group["legs"]:
            if int(_field(tx, "amount") or 0) <= 0:
                continue
            tx_id = str(_field(tx, "id"))
            out_row = _clone_channel_leg(
                tx,
                wallet_refs_by_id[node_wallet_id],
                row_id=f"channel-close:{tx_id}:out:{node_wallet_id}",
                direction="outbound",
                fee=close_fee if tx_id == fee_leg_id else 0,
            )
            pairs.append(
                {
                    "out": out_row,
                    "in": tx,
                    "source": "channel_lifecycle",
                    "kind": CHANNEL_CLOSE,
                }
            )
    return pairs


def _clone_channel_leg(
    row: Any,
    wallet_ref: Mapping[str, Any],
    *,
    row_id: str,
    direction: str,
    fee: int,
) -> dict[str, Any]:
    cloned = _row_dict(row)
    cloned.update(
        {
            "id": row_id,
            "journal_transaction_id": _field(row, "id"),
            "direction": direction,
            "fee": fee,
            "wallet_id": wallet_ref["id"],
            "wallet_label": wallet_ref["label"],
            "wallet_account_id": wallet_ref.get("wallet_account_id"),
            "account_code": wallet_ref.get("account_code"),
            "account_label": wallet_ref.get("account_label"),
            "kind": "self_transfer_in" if direction == "inbound" else "self_transfer_out",
            "description": row_id,
        }
    )
    return cloned


def _row_dict(row: Any) -> dict[str, Any]:
    if isinstance(row, Mapping):
        return dict(row)
    try:
        return {key: row[key] for key in row.keys()}
    except AttributeError:
        return dict(getattr(row, "__dict__", {}))
