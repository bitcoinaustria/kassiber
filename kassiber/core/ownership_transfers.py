"""Address-ownership self-transfer deriver.

``kassiber.transfers.detect_intra_transfers`` pairs a self-transfer only when
two wallets independently recorded a row that shares the *same* on-chain txid
(``external_id``). That misses the cases users actually hit:

* the destination wallet never recorded a row (it was not synced for that
  period) — the source outbound then looks like a disposal;
* the two wallets' rows carry different/missing txids (CSV imports);
* one transaction fans out to two or more owned wallets (1->N), which the
  conservative 1-out/1-in detector skips and the journal pipeline quarantines.

This module closes those by reading the *actual transaction graph*: for an
on-chain Bitcoin outbound whose full ``vin``/``vout`` are stored in
``transactions.raw_json``, it asks the profile-wide :class:`OwnedIndex`
("which of my wallets owns this output's script?"). An output paying an
address owned by a *different* wallet of the same profile is a self-transfer
leg — proven deterministically, with no amount/time heuristic.

Scope (intentionally conservative — anything outside falls through to the
existing row-matching + quarantine behavior, never mis-booked):

* **Single-source only.** When more than one owned wallet contributed inputs
  to the spend, per-wallet sync double-counts the network fee (each wallet's
  outbound row carries the whole ``fee``), so the amounts are unreliable. Such
  transactions are declined and left for the existing fan-out quarantine.
* **esplora/mempool shape.** The split needs per-output values; the Electrum
  decode form stores scripts without values, and Liquid / all CSV imports
  store no ``vin``/``vout`` at all. Those parse to ``None`` and are skipped
  (this is also what cleanly excludes cross-asset pegs).
* **Owned outputs only.** External recipients and OP_RETURN are never legs;
  the residual ``amount - Σ(legs)`` stays on the source as a real disposal.

Known limitation: a fan-out where the source shares its txid with *exactly one*
recorded destination inbound is pre-empted by ``detect_intra_transfers`` (which
pairs that 1-out/1-in shape before this runs), so the source lands in
``already_paired_ids`` and is skipped here. That partial fan-out degrades to the
existing ``transfer_fee_implausible`` quarantine (a safe review flag, not a
mis-booking). Fully-recorded fan-outs (1-out/N-in, which ``detect_intra``
skips) and fully sync-gapped ones are decomposed normally.

Amount model (from ``record_from_bitcoin_esplora_tx``): an outbound row's
``amount`` is the sum of its non-change output values (change-to-self and the
miner fee are both excluded), and ``fee`` is the miner fee. So for every
derived pair ``out_leg.amount == in_leg.amount`` by construction, which keeps
each leg under the journal pipeline's implausible-fee guard.

Pure-ish: no SQLite. The caller builds the :class:`OwnedIndex` once and passes
it in alongside the already-fetched rows; raw transaction JSON is read from the
row's ``raw_json`` column.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any, Mapping, Optional, Sequence

from ..msat import msat_to_btc
from ..wallet_descriptors import normalize_chain, normalize_network


SATS_TO_MSAT = 1000
# Reuse a destination's existing inbound row for a leg only when it sits within
# this window of the spend (and is otherwise unambiguous) — mirrors the swap
# matcher's default time tolerance.
REUSE_WINDOW_SECONDS = 24 * 60 * 60
# Synthetic outbound rows minted by earlier engine stages (direct-payout splits,
# cross-asset splits) keep the real txid in raw_json but are NOT fresh spends to
# re-decompose; skip them by id prefix.
_SYNTHETIC_ID_PREFIXES = ("owned-derive:", "cross-split:", "direct-payout:")


@dataclass(frozen=True)
class OwnershipDeriveResult:
    """What :func:`derive_ownership_transfers` contributes to the engine run.

    * ``derived_pairs`` — ``{"out": out_leg, "in": in_row, "source": ...}`` in
      the shape the journal pipeline's intra path consumes (same as
      ``apply_manual_pairs`` output) plus a provenance marker.
    * ``synthetic_rows`` — the split out-legs and any synthesized inbound legs
      that must be appended to the engine row set.
    * ``out_row_overrides`` — ``{out_id: reduced_row}`` for sources that also
      paid a real external recipient (the residual stays a disposal).
    * ``dropped_out_ids`` — sources fully consumed by owned legs (pure internal
      move); removed from the row set so they are not double-booked.
    """

    derived_pairs: list[dict[str, Any]] = field(default_factory=list)
    synthetic_rows: list[dict[str, Any]] = field(default_factory=list)
    out_row_overrides: dict[str, dict[str, Any]] = field(default_factory=dict)
    dropped_out_ids: set[str] = field(default_factory=set)
    blocked_sources: list[dict[str, Any]] = field(default_factory=list)


def derive_ownership_transfers(
    rows: Sequence[Mapping[str, Any]],
    *,
    index: Any,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    already_paired_ids: set[str],
) -> OwnershipDeriveResult:
    """Derive address-proven self-transfer pairs from the transaction graph.

    Args:
        rows: engine rows (sqlite3.Row-like or dict). Real on-chain rows expose
            ``raw_json`` with the full ``vin``/``vout``.
        index: a prebuilt :class:`kassiber.core.ownership.OwnedIndex` (or
            ``None`` — the deriver then no-ops).
        wallet_refs_by_id: profile-wide wallet refs (``id``, ``label`` and
            account fields). Must cover *every* wallet, including destinations
            with no rows, so synthesized inbound legs resolve to a real wallet.
        already_paired_ids: transaction ids already covered by a same-txid auto
            pair or a manual / split pair record (both out and in legs). Sources
            in this set are skipped; inbound rows in it are never consumed.

    Returns:
        :class:`OwnershipDeriveResult`.
    """
    result = OwnershipDeriveResult()
    if index is None:
        return result

    inbound_by_wallet: dict[str, list[Mapping[str, Any]]] = {}
    for row in rows:
        if _get(row, "direction") != "inbound":
            continue
        # Never reuse a synthetic inbound minted by another engine stage
        # (direct-payout / cross-split target legs); consuming one would strip a
        # leg the other path needs and double-handle the row.
        if str(_get(row, "id")).startswith(_SYNTHETIC_ID_PREFIXES):
            continue
        inbound_by_wallet.setdefault(str(_get(row, "wallet_id")), []).append(row)
    consumed_in_ids: set[str] = set()

    for row in rows:
        source_id = str(_get(row, "id"))
        if _get(row, "direction") != "outbound":
            continue
        if source_id in already_paired_ids:
            continue
        if source_id.startswith(_SYNTHETIC_ID_PREFIXES):
            continue
        if int(_get(row, "amount") or 0) <= 0:
            continue
        parsed = _parse_onchain_tx(_get(row, "raw_json"))
        if parsed is None:
            continue
        source_wallet_id = str(_get(row, "wallet_id"))
        # Constrain ownership to the source's own chain/network. The same
        # scriptpubkey hex is shared by mainnet and testnet siblings of one key
        # (and by a reused-key Liquid wallet), so an unfiltered script match can
        # route a real BTC payment into a wrong-chain wallet as a phantom MOVE.
        source_chain_network = _source_chain_network(
            parsed["inputs"], index, source_wallet_id
        )

        # Aggregate owned outputs per destination wallet — sync records one
        # inbound row per wallet per tx, so a wallet receiving two outputs in
        # the same tx must pair as a single leg of their combined value.
        by_dest: dict[str, dict[str, Any]] = {}
        ambiguous_output = False
        for output in parsed["outputs"]:
            matches = index.lookup_script(output["script"])
            if source_chain_network is not None:
                matches = [
                    match
                    for match in matches
                    if _norm_chain_network(match.chain, match.network)
                    == source_chain_network
                ]
            if not matches:
                # External recipient, OP_RETURN, or a same-script-hex collision
                # on another chain/network — never an owned leg; folded into the
                # residual disposal.
                continue
            owner_ids = {str(match.wallet_id) for match in matches}
            if source_wallet_id in owner_ids:
                # The source wallet also owns this script -> change back to self.
                # (Matches the sync amount model, which excludes change.)
                continue
            if len(owner_ids) > 1:
                # Owned by two different non-source wallets (shared descriptor /
                # address reuse): we cannot route the leg unambiguously. Decline
                # the whole tx rather than guess a destination.
                ambiguous_output = True
                break
            owner = matches[0]
            dest_wallet_id = str(owner.wallet_id)
            slot = by_dest.setdefault(
                dest_wallet_id,
                {"value_sats": 0, "label": owner.wallet_label, "min_n": output["n"]},
            )
            slot["value_sats"] += int(output["value_sats"])
            slot["min_n"] = min(slot["min_n"], output["n"])
        if ambiguous_output:
            _block_source(
                result,
                row,
                "ownership_transfer_ambiguous_output",
                {
                    "required_for": "ownership_transfer_review",
                    "wallet": _get(row, "wallet_label") or source_wallet_id,
                    "asset": _get(row, "asset"),
                    "external_id": _get(row, "external_id"),
                },
            )
            continue
        if not by_dest:
            continue  # ordinary outbound payment — leave on the disposal path
        if not _inputs_are_single_source(parsed["inputs"], index, source_wallet_id):
            _block_source(
                result,
                row,
                "ownership_transfer_source_ambiguous",
                {
                    "required_for": "ownership_transfer_review",
                    "wallet": _get(row, "wallet_label") or source_wallet_id,
                    "asset": _get(row, "asset"),
                    "external_id": _get(row, "external_id"),
                },
            )
            continue

        source_amount_msat = int(_get(row, "amount") or 0)
        legs_value_msat = sum(slot["value_sats"] * SATS_TO_MSAT for slot in by_dest.values())
        # The owned legs cannot exceed what the row says left the wallet; a
        # mismatch means the parsed graph and the recorded amount disagree
        # (re-org/RBF stale json, odd sync) — decline rather than guess.
        if legs_value_msat > source_amount_msat:
            _block_source(
                result,
                row,
                "ownership_transfer_amount_mismatch",
                {
                    "required_for": "ownership_transfer_review",
                    "wallet": _get(row, "wallet_label") or source_wallet_id,
                    "asset": _get(row, "asset"),
                    "external_id": _get(row, "external_id"),
                    "row_amount_msat": source_amount_msat,
                    "owned_outputs_msat": legs_value_msat,
                },
            )
            continue

        txid = str(parsed.get("txid") or _get(row, "external_id") or source_id)
        source_fee_msat = int(_get(row, "fee") or 0)
        legs = sorted(by_dest.items(), key=lambda item: (item[1]["min_n"], item[0]))
        leg_pairs: list[dict[str, Any]] = []
        leg_synthetic_rows: list[dict[str, Any]] = []
        ok = True
        decline_reason: Optional[str] = None
        decline_detail: dict[str, Any] = {}
        for position, (dest_wallet_id, slot) in enumerate(legs):
            leg_msat = slot["value_sats"] * SATS_TO_MSAT
            if leg_msat <= 0:
                ok = False
                break
            fee_for_leg = source_fee_msat if position == 0 else 0
            out_leg = _clone_row(
                row,
                amount=leg_msat,
                fee=fee_for_leg,
                row_id=f"owned-derive:{txid}:out:{slot['min_n']}",
                external_id=f"owned-derive:{txid}:out:{slot['min_n']}",
                kind="self_transfer_out",
                journal_transaction_id=source_id,
            )
            decision, in_row = _resolve_destination_inbound(
                inbound_by_wallet.get(dest_wallet_id, ()),
                leg_msat,
                txid,
                _get(row, "occurred_at"),
                consumed_in_ids,
                already_paired_ids,
                asset=_get(row, "asset"),
            )
            if decision == "decline":
                # The destination has an ambiguous match (>=2 equal-value
                # candidates, or a near non-matching inbound that might be this
                # very leg recorded by a CSV import). Synthesizing would risk a
                # duplicate inbound (silent holdings inflation); reusing would
                # risk cannibalizing an unrelated receipt. Leave the whole tx on
                # its existing disposal/quarantine path instead of guessing.
                ok = False
                decline_reason = "ownership_transfer_destination_ambiguous"
                decline_detail = {
                    "required_for": "ownership_transfer_review",
                    "wallet": _get(row, "wallet_label") or source_wallet_id,
                    "destination_wallet_id": dest_wallet_id,
                    "asset": _get(row, "asset"),
                    "external_id": _get(row, "external_id"),
                    "leg_amount_msat": leg_msat,
                }
                break
            if decision == "reuse":
                consumed_in_ids.add(str(_get(in_row, "id")))
            else:  # "synthesize" — the destination recorded no related inbound
                dest_ref = wallet_refs_by_id.get(dest_wallet_id)
                if dest_ref is None:
                    # No ref for the destination wallet — cannot book the MOVE
                    # target safely; leave the whole tx to existing handling.
                    ok = False
                    decline_reason = "ownership_transfer_destination_missing_ref"
                    decline_detail = {
                        "required_for": "ownership_transfer_review",
                        "wallet": _get(row, "wallet_label") or source_wallet_id,
                        "destination_wallet_id": dest_wallet_id,
                        "asset": _get(row, "asset"),
                        "external_id": _get(row, "external_id"),
                    }
                    break
                in_row = _clone_row(
                    row,
                    amount=leg_msat,
                    fee=0,
                    row_id=f"owned-derive:{txid}:in:{slot['min_n']}",
                    external_id=f"owned-derive:{txid}:in:{slot['min_n']}",
                    kind="self_transfer_in",
                    journal_transaction_id=source_id,
                    direction="inbound",
                    wallet_id=dest_wallet_id,
                    wallet_ref=dest_ref,
                )
                leg_synthetic_rows.append(in_row)
            leg_pairs.append({"out": out_leg, "in": in_row, "source": "ownership_derived"})
            leg_synthetic_rows.append(out_leg)
        if not ok or not leg_pairs:
            # Roll back any inbound rows we tentatively consumed for this tx.
            for pair in leg_pairs:
                consumed_in_ids.discard(str(_get(pair["in"], "id")))
            if decline_reason is not None:
                _block_source(result, row, decline_reason, decline_detail)
            continue

        result.derived_pairs.extend(leg_pairs)
        result.synthetic_rows.extend(leg_synthetic_rows)
        residual_msat = source_amount_msat - legs_value_msat
        if residual_msat > 0:
            # The spend also paid a real external recipient; keep the residual
            # portion as a disposal of the source row. The whole miner fee is
            # already attributed to the first MOVE leg above, so the residual
            # disposal must carry fee=0 — otherwise the fee leaves the source
            # pool twice (phantom fee disposal + a spurious over-sell).
            result.out_row_overrides[source_id] = _clone_row(
                row, amount=residual_msat, fee=0
            )
        else:
            # Fully internal move — the source carried only owned legs + fee.
            result.dropped_out_ids.add(source_id)

    return result


def derive_recorded_fanout_transfers(
    rows: Sequence[Mapping[str, Any]],
    *,
    already_paired_ids: set[str],
) -> OwnershipDeriveResult:
    """Decompose a recorded 1->N self-transfer fan-out from the rows alone.

    The address-ownership deriver needs a readable on-chain graph (esplora
    ``vin``/``vout``). Liquid output amounts are confidential, so a Liquid spend
    carries no per-output graph — and a CSV import may carry none either. But
    when every leg of the fan-out *was* synced, the rows themselves are enough:
    a group of rows sharing one ``(external_id, asset)`` across two or more of
    the profile's wallets is, by construction, all owned, and the sync amount
    model conserves value (an outbound's ``amount`` excludes change and the fee,
    so ``out.amount == sum(in.amount)`` for a pure fan-out on both Bitcoin and
    Liquid). ``detect_intra_transfers`` only pairs the clean 1-out/1-in shape, so
    a 1->N fan-out is otherwise quarantined ``owned_fanout_unresolved``.

    Scope (conservative — anything outside is left to that quarantine):

    * **Exactly one outbound.** Multi-source consolidations (>1 outbound) assign
      the whole fee to each contributing wallet's row, so amounts are unreliable.
    * **Two or more distinct destination wallets**, one inbound each (a wallet
      receiving twice in one tx records a single combined inbound).
    * **Exact conservation.** ``out.amount == sum(in.amount)``; a shortfall means
      a destination was not synced, so the split would be wrong.

    Pairs reuse the recorded inbound rows; the outbound is split into one MOVE
    leg per destination (whole fee on the first leg) and dropped from the row
    set. Runs *after* the address-ownership deriver and must be given that
    deriver's touched ids in ``already_paired_ids`` so a graph-readable Bitcoin
    fan-out is not decomposed twice.
    """
    result = OwnershipDeriveResult()
    groups: dict[tuple[str, Any], list[Mapping[str, Any]]] = {}
    for row in rows:
        external_id = _get(row, "external_id")
        if not external_id:
            continue
        if str(_get(row, "id")).startswith(_SYNTHETIC_ID_PREFIXES):
            continue
        groups.setdefault((str(external_id), _get(row, "asset")), []).append(row)

    for (external_id, asset), group in groups.items():
        outs = [
            row
            for row in group
            if _get(row, "direction") == "outbound"
            and int(_get(row, "amount") or 0) > 0
            and str(_get(row, "id")) not in already_paired_ids
        ]
        if len(outs) != 1:
            continue  # consolidation / nothing to split — leave to quarantine
        out_row = outs[0]
        source_wallet_id = str(_get(out_row, "wallet_id"))
        dest_ins = [
            row
            for row in group
            if _get(row, "direction") == "inbound"
            and str(_get(row, "id")) not in already_paired_ids
            and str(_get(row, "wallet_id")) != source_wallet_id
        ]
        if len(dest_ins) < 2:
            # 0 destinations -> not a transfer; exactly 1 -> the clean shape
            # detect_intra_transfers already pairs (and would be in
            # already_paired_ids). Either way, nothing to decompose here.
            continue
        dest_wallets = {str(_get(row, "wallet_id")) for row in dest_ins}
        if len(dest_wallets) != len(dest_ins):
            continue  # a wallet appears twice — odd shape, decline
        out_amount = int(_get(out_row, "amount") or 0)
        legs_total = sum(int(_get(row, "amount") or 0) for row in dest_ins)
        if legs_total != out_amount:
            continue  # a destination was not synced — amounts don't conserve

        out_fee = int(_get(out_row, "fee") or 0)
        legs = sorted(
            dest_ins,
            key=lambda row: (int(_get(row, "amount") or 0), str(_get(row, "id"))),
        )
        leg_pairs: list[dict[str, Any]] = []
        leg_rows: list[dict[str, Any]] = []
        ok = True
        for position, in_row in enumerate(legs):
            leg_msat = int(_get(in_row, "amount") or 0)
            if leg_msat <= 0:
                ok = False
                break
            out_leg = _clone_row(
                out_row,
                amount=leg_msat,
                fee=out_fee if position == 0 else 0,
                row_id=f"recorded-fanout:{external_id}:out:{_get(in_row, 'id')}",
                external_id=f"recorded-fanout:{external_id}:out:{_get(in_row, 'id')}",
                kind="self_transfer_out",
                journal_transaction_id=str(_get(out_row, "id")),
            )
            leg_pairs.append({"out": out_leg, "in": in_row, "source": "recorded_fanout"})
            leg_rows.append(out_leg)
        if not ok or not leg_pairs:
            continue
        result.derived_pairs.extend(leg_pairs)
        result.synthetic_rows.extend(leg_rows)
        result.dropped_out_ids.add(str(_get(out_row, "id")))

    return result


# -- internals --------------------------------------------------------------


def _inputs_are_single_source(
    inputs: Sequence[Mapping[str, Any]], index: Any, source_wallet_id: str
) -> bool:
    """True only when the source wallet owns every input.

    A foreign/unresolvable input (payjoin/coinjoin, or coins we do not watch)
    or an input from a *different* owned wallet (a multi-wallet consolidation)
    makes the recorded amount/fee unreliable for splitting. An input is
    acceptable only when ``source_wallet_id`` is among its owners; resolution is
    set-based (not first-match) so a shared descriptor / reused address does not
    make the verdict depend on index insertion order.
    """
    if not inputs:
        return False
    for entry in inputs:
        owners = _input_owner_ids(index, entry)
        if not owners or source_wallet_id not in owners:
            return False
    return True


def _input_owner_ids(index: Any, entry: Mapping[str, Any]) -> set[str]:
    """All owned-wallet ids for an input (outpoint inventory wins; else script).

    The outpoint inventory is unambiguous (one wallet per UTXO); only the
    script fallback can map to several wallets, and we return the full set so
    callers can reason about ambiguity instead of an arbitrary first match.
    """
    outpoint = entry.get("outpoint")
    if outpoint:
        match = index.by_outpoint.get(outpoint)
        if match is not None:
            return {str(match.wallet_id)}
    return {str(match.wallet_id) for match in index.lookup_script(entry.get("script"))}


def _norm_chain_network(chain: Any, network: Any) -> tuple[str, str]:
    """Canonical ``(chain, network)`` for comparison.

    The index seeds chain/network from three paths with inconsistent spelling —
    the descriptor path normalizes, but the address-list and inventory paths
    store raw config / DB values (``btc``, ``mainnet``, ``""`` …). Comparing the
    raw strings would drop a legitimate same-network self-transfer as if it were
    cross-chain, so both sides are normalized here. Unsupported values fall back
    to a lowercased raw tuple (still consistent for identical spellings) instead
    of raising.
    """
    try:
        canonical_chain = normalize_chain(chain)
        return (canonical_chain, normalize_network(canonical_chain, network))
    except ValueError:
        return (str(chain or "").strip().lower(), str(network or "").strip().lower())


def _source_chain_network(
    inputs: Sequence[Mapping[str, Any]], index: Any, source_wallet_id: str
) -> Optional[tuple[str, str]]:
    """Canonical ``(chain, network)`` of the source wallet, from its owned inputs.

    Used to reject output matches on a different chain/network (the same
    scriptpubkey hex is produced by mainnet/testnet siblings of one key, or a
    reused-key Liquid wallet). Returns ``None`` when no input resolves to the
    source wallet — the spend is then left to the single-source guard, which
    blocks it, so no chain filtering is needed.
    """
    for entry in inputs:
        outpoint = entry.get("outpoint")
        if outpoint:
            match = index.by_outpoint.get(outpoint)
            if match is not None and str(match.wallet_id) == source_wallet_id:
                return _norm_chain_network(match.chain, match.network)
        for match in index.lookup_script(entry.get("script")):
            if str(match.wallet_id) == source_wallet_id:
                return _norm_chain_network(match.chain, match.network)
    return None


def _parse_onchain_tx(raw_json: Any) -> Optional[dict[str, Any]]:
    """Parse stored esplora/mempool tx JSON into inputs + valued outputs.

    Returns ``None`` for anything without a usable ``vin``/``vout`` carrying
    per-output values: the Electrum decode form (scripts only, no value),
    Liquid (no vin/vout), and every CSV import. That ``None`` is exactly the
    clean skip for cross-asset pegs and non-on-chain sources.
    """
    try:
        raw = json.loads(raw_json or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    if not isinstance(raw, dict):
        return None
    vin = raw.get("vin")
    vout = raw.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return None

    inputs: list[dict[str, Any]] = []
    for position, entry in enumerate(vin):
        if not isinstance(entry, dict):
            return None
        prevout = entry.get("prevout") or {}
        outpoint = None
        if entry.get("txid") is not None and entry.get("vout") is not None:
            try:
                outpoint = f"{str(entry.get('txid')).lower()}:{int(entry.get('vout'))}"
            except (TypeError, ValueError):
                outpoint = None
        inputs.append({"outpoint": outpoint, "script": prevout.get("scriptpubkey")})

    outputs: list[dict[str, Any]] = []
    for position, entry in enumerate(vout):
        if not isinstance(entry, dict):
            return None
        script = entry.get("scriptpubkey")
        value = entry.get("value")
        if value is None:
            # Electrum decode form: scripts but no value — cannot split.
            return None
        try:
            value_sats = int(value)
        except (TypeError, ValueError):
            return None
        try:
            output_index = int(entry.get("n", position))
        except (TypeError, ValueError):
            output_index = position
        outputs.append(
            {"n": output_index, "script": script, "value_sats": value_sats}
        )
    return {"txid": raw.get("txid"), "inputs": inputs, "outputs": outputs}


def _resolve_destination_inbound(
    candidates: Sequence[Mapping[str, Any]],
    leg_msat: int,
    txid: str,
    source_occurred_at: Any,
    consumed_in_ids: set[str],
    already_paired_ids: set[str],
    *,
    asset: Any,
) -> tuple[str, Optional[Mapping[str, Any]]]:
    """Decide how to represent one destination leg.

    Returns ``("reuse", row)``, ``("synthesize", None)``, or
    ``("decline", None)``. The caller reuses the row, synthesizes a fresh
    inbound, or abandons the whole derivation respectively.

    The distinction that matters for correctness is *synthesize vs decline*:
    fabricating an inbound is only safe when the destination recorded **no**
    related inbound near the spend. If it did, synthesizing would double-count
    (the synthetic ``transfer_in`` plus the still-present real row); reusing an
    ambiguous match would instead cannibalize what may be an unrelated receipt.
    Either way, when we cannot be confident, we decline and leave the source on
    its existing disposal/quarantine path (status quo, surfaces for review).

    Decision order (candidates are the destination's same-asset, unpaired,
    unconsumed inbound rows):

    1. A row sharing this spend's on-chain txid is unambiguously this leg → reuse.
    2. Exactly one exact-value candidate that is not provably a *different*
       on-chain transaction and is within the time window → reuse it.
    3. Two or more such exact-value candidates → ambiguous → decline.
    4. No exact reuse, but the destination has some other in-window inbound that
       is not provably a different on-chain transaction (a near/off-value row
       that might be this very leg) → decline rather than fabricate a duplicate.
    5. Otherwise the destination is genuinely empty for this leg → synthesize.
    """
    asset_key = str(asset or "").upper()
    source_seconds = _iso_seconds(source_occurred_at)

    def _within_window(row: Mapping[str, Any]) -> bool:
        if source_seconds is None:
            return True
        row_seconds = _iso_seconds(_get(row, "occurred_at"))
        return row_seconds is not None and abs(row_seconds - source_seconds) <= REUSE_WINDOW_SECONDS

    def _different_onchain_tx(row: Mapping[str, Any]) -> bool:
        external_id = str(_get(row, "external_id") or "")
        return _looks_like_txid(external_id) and external_id.lower() != txid.lower()

    available = [
        row
        for row in candidates
        if str(_get(row, "id")) not in consumed_in_ids
        and str(_get(row, "id")) not in already_paired_ids
        and str(_get(row, "asset") or "").upper() == asset_key
    ]

    exact = [row for row in available if int(_get(row, "amount") or 0) == leg_msat]
    same_txid = [row for row in exact if str(_get(row, "external_id") or "") == txid]
    if len(same_txid) == 1:
        return ("reuse", same_txid[0])
    if len(same_txid) >= 2:
        return ("decline", None)

    reusable = [
        row
        for row in exact
        if not _different_onchain_tx(row) and _within_window(row)
    ]
    if reusable:
        return ("decline", None)  # ambiguous equal-value matches — never fabricate

    # No exact reuse. Synthesizing is safe only when nothing else in the
    # destination could plausibly be this leg. A row that provably belongs to a
    # different on-chain transaction does not block (it is a separate receipt);
    # any other in-window same-asset inbound does.
    nearby = [
        row
        for row in available
        if _within_window(row) and not _different_onchain_tx(row)
    ]
    if nearby:
        return ("decline", None)
    return ("synthesize", None)


def _looks_like_txid(value: Any) -> bool:
    text = str(value or "").strip()
    if len(text) != 64:
        return False
    try:
        int(text, 16)
    except ValueError:
        return False
    return True


def _block_source(
    result: OwnershipDeriveResult,
    row: Mapping[str, Any],
    reason: str,
    detail: Mapping[str, Any],
) -> None:
    result.blocked_sources.append(
        {"row": row, "reason": reason, "detail": dict(detail)}
    )


def _clone_row(
    source_row: Mapping[str, Any],
    *,
    amount: int,
    fee: int,
    row_id: Optional[str] = None,
    external_id: Optional[str] = None,
    kind: Optional[str] = None,
    journal_transaction_id: Optional[str] = None,
    direction: Optional[str] = None,
    wallet_id: Optional[str] = None,
    wallet_ref: Optional[Mapping[str, Any]] = None,
) -> dict[str, Any]:
    """Clone a source row into a split / synthetic leg.

    Mirrors ``rp2._split_review_source_row``: a row priced by value alone keeps
    its evidence by materializing a per-unit rate before the absolute
    ``fiat_value`` is cleared, so each leg reprices from its own amount instead
    of becoming a false missing-price quarantine.
    """
    base = dict(source_row)
    full = int(_get(source_row, "amount") or 0)
    if (
        _get(source_row, "fiat_rate_exact") in (None, "")
        and _get(source_row, "fiat_rate") in (None, "")
        and full > 0
    ):
        fiat_value = _get(source_row, "fiat_value_exact") or _get(source_row, "fiat_value")
        if fiat_value not in (None, ""):
            unit_rate = format(Decimal(str(fiat_value)) / msat_to_btc(full), "f")
            base["fiat_rate"] = unit_rate
            base["fiat_rate_exact"] = unit_rate
    base["amount"] = amount
    base["fee"] = fee
    base["fiat_value"] = None
    base["fiat_value_exact"] = None
    if row_id is not None:
        base["id"] = row_id
    if external_id is not None:
        base["external_id"] = external_id
    if kind is not None:
        base["kind"] = kind
    if journal_transaction_id is not None:
        base["journal_transaction_id"] = journal_transaction_id
    if direction is not None:
        base["direction"] = direction
    if wallet_id is not None:
        base["wallet_id"] = wallet_id
    if wallet_ref is not None:
        base["wallet_label"] = wallet_ref.get("label")
        base["wallet_account_id"] = wallet_ref.get("wallet_account_id")
        base["account_code"] = wallet_ref.get("account_code")
        base["account_label"] = wallet_ref.get("account_label")
    return base


def _iso_seconds(value: Any) -> Optional[float]:
    if not value:
        return None
    raw = str(value).strip()
    if not raw:
        return None
    if raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(raw)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.timestamp()


def _get(row: Any, key: str, default: Any = None) -> Any:
    if isinstance(row, dict):
        return row.get(key, default)
    try:
        keys = row.keys()
    except AttributeError:
        return getattr(row, key, default)
    if key in keys:
        return row[key]
    return default
