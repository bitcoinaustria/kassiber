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

Amount model: an outbound row's spend capacity is ``amount + fee`` unless
``amount_includes_fee`` is set. Esplora-style rows usually have ``amount`` as
the sum of non-change output values and ``fee`` as the miner fee, while some
Core wallet shapes report ``amount`` net of the fee. The deriver therefore
matches owned outputs against total capacity and assigns only the fee still
available after those outputs are covered.

Pure-ish: no SQLite. The caller builds the :class:`OwnedIndex` once and passes
it in alongside the already-fetched rows; raw transaction JSON is read from the
row's ``raw_json`` column.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from decimal import Decimal
from typing import Any, Mapping, Optional, Sequence

from ..msat import msat_to_btc
from ..transfers import normalize_group_txid
from ..wallet_descriptors import normalize_chain, normalize_network


SATS_TO_MSAT = 1000
# Synthetic outbound rows minted by earlier engine stages (direct-payout splits,
# cross-asset splits) keep the real txid in raw_json but are NOT fresh spends to
# re-decompose; skip them by id prefix.
_SYNTHETIC_ID_PREFIXES = (
    "owned-derive:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
)


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
    * ``dropped_in_ids`` — recorded inbound rows replaced by synthesized MOVE
      in-legs (a multi-source consolidation splits one recorded destination
      receipt into one leg per contributing wallet); removed from the row set so
      the receipt is not also booked as a standalone acquisition.
    """

    derived_pairs: list[dict[str, Any]] = field(default_factory=list)
    synthetic_rows: list[dict[str, Any]] = field(default_factory=list)
    out_row_overrides: dict[str, dict[str, Any]] = field(default_factory=dict)
    dropped_out_ids: set[str] = field(default_factory=set)
    dropped_in_ids: set[str] = field(default_factory=set)
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
        if not _inputs_are_single_source_or_recorded_source(
            parsed["inputs"], index, source_wallet_id, row
        ):
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
        source_fee_msat = int(_get(row, "fee") or 0)
        source_total_msat = source_amount_msat
        if not _get(row, "amount_includes_fee"):
            source_total_msat += source_fee_msat
        legs_value_msat = sum(slot["value_sats"] * SATS_TO_MSAT for slot in by_dest.values())
        # The owned legs cannot exceed what the row says left the wallet. Some
        # Core rows carry a net amount (owned outputs minus fee) plus a fee
        # column; comparing against amount alone would falsely reject pure
        # fan-outs where amount + fee exactly equals the owned outputs.
        if legs_value_msat > source_total_msat:
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
                    "row_total_outflow_msat": source_total_msat,
                    "owned_outputs_msat": legs_value_msat,
                },
            )
            continue

        txid = str(parsed.get("txid") or _get(row, "external_id") or source_id)
        transfer_group_id = f"owned-derive:{txid}" if len(by_dest) > 1 else None
        fee_budget_msat = min(
            source_fee_msat,
            max(0, source_total_msat - legs_value_msat),
        )
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
            fee_for_leg = fee_budget_msat if position == 0 else 0
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
            pair = {"out": out_leg, "in": in_row, "source": "ownership_derived"}
            if transfer_group_id:
                pair["group_id"] = transfer_group_id
            leg_pairs.append(pair)
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
        residual_msat = source_total_msat - legs_value_msat - fee_budget_msat
        if residual_msat > 0:
            # The spend also paid a real external recipient; keep the residual
            # portion as a disposal of the source row. Any available miner fee is
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
        # Normalize txid casing so a source synced lowercase and a destination
        # imported uppercase (CSV) land in the same group — matching every other
        # self-transfer path (detect_intra_transfers, _owned_fanout_row_ids,
        # derive_multi_source_consolidations). Without this a mixed-case fan-out
        # the decomposer should split is instead stuck in the fan-out quarantine.
        groups.setdefault(
            (normalize_group_txid(str(external_id)), _get(row, "asset")), []
        ).append(row)

    for (external_id, asset), group in groups.items():
        # Count the group's TRUE source rows first — every positive outbound,
        # paired or not. The consolidation guard must reflect how many wallets
        # actually funded the spend; filtering already_paired_ids first would let
        # a multi-source consolidation masquerade as single-source once one of
        # its sources was handled elsewhere, and the surviving source (whose
        # per-wallet amount is unreliable) would be wrongly split.
        outs = [
            row
            for row in group
            if _get(row, "direction") == "outbound"
            and int(_get(row, "amount") or 0) > 0
        ]
        if len(outs) != 1:
            continue  # consolidation / nothing to split — leave to quarantine
        out_row = outs[0]
        if str(_get(out_row, "id")) in already_paired_ids:
            continue  # the single source is already handled elsewhere
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
        transfer_group_id = f"recorded-fanout:{external_id}"
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
            leg_pairs.append(
                {
                    "out": out_leg,
                    "in": in_row,
                    "source": "recorded_fanout",
                    "group_id": transfer_group_id,
                }
            )
            leg_rows.append(out_leg)
        if not ok or not leg_pairs:
            continue
        result.derived_pairs.extend(leg_pairs)
        result.synthetic_rows.extend(leg_rows)
        result.dropped_out_ids.add(str(_get(out_row, "id")))

    return result


def derive_multi_source_consolidations(
    rows: Sequence[Mapping[str, Any]],
    *,
    index: Any,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    already_paired_ids: set[str],
) -> OwnershipDeriveResult:
    """Decompose an N->1 cross-wallet consolidation into per-source MOVE legs.

    A spend funded by inputs from two or more owned wallets (a consolidation,
    e.g. sweeping Cold + Hot into Savings) is the one case both
    :func:`derive_ownership_transfers` and :func:`derive_recorded_fanout_transfers`
    deliberately decline: each contributing wallet syncs the transaction
    independently and stamps the *whole* network fee onto its own outbound row
    (``record_from_bitcoin_esplora_tx``), so naively summing the per-wallet rows
    double-counts the fee. Left undisambiguated it lands in the
    ``owned_fanout_unresolved`` quarantine.

    But the readable on-chain graph plus the per-wallet rows are jointly enough
    to book it correctly without trusting any single row's fee twice:

    * the miner fee is the *same* value on every contributor's row (it is the
      whole-tx fee), so it is read once, not summed;
    * each contributor's recorded ``amount`` is ``its inputs - its change - fee``
      (the esplora amount model), so its true net outflow is ``amount + fee``;
    * the destination's received value is taken from the graph outputs.

    With ``a_S`` = contributor ``S``'s recorded amount, ``F`` = the shared fee,
    ``n`` = number of contributors and ``out_C`` = the single destination's
    graph output total, conservation is the exact identity
    ``Σ a_S + (n-1)·F == out_C``. The whole fee is assigned to the largest
    contributor's leg; that leg moves ``a_bearer`` and every other leg moves
    ``a_S + F``, so the legs sum to ``out_C`` and each contributor's pool is
    debited exactly its true net outflow (leg amount + leg fee).

    Scope (conservative — anything outside is left to the existing quarantine):

    * **>=2 contributing wallets, exactly one owned destination, no external
      output.** A consolidation that also pays a non-owned recipient has
      ambiguous fee attribution and is left for explicit review; ``N->M`` (two
      or more destinations) likewise.
    * **All inputs owned by the contributing wallets.** A foreign input
      (payjoin/coinjoin, unwatched coins) makes the amount/fee math unreliable.
    * **Readable esplora graph + a single shared fee.** Liquid (confidential
      outputs) and graphless CSV imports parse to ``None`` and are skipped; a
      fee that differs across contributors means at least one row is not the
      node's whole-tx fee, so decline.
    * **Exact conservation.** A mismatch means a sync gap or stale graph.

    Runs *before* :func:`derive_ownership_transfers`; the caller must feed every
    id this pass touches (contributors + the destination receipt) into that
    deriver's ``already_paired_ids`` so the single-source deriver does not also
    block-and-quarantine the same contributors.
    """
    result = OwnershipDeriveResult()
    if index is None:
        return result

    groups: dict[tuple[str, Any], list[Mapping[str, Any]]] = {}
    for row in rows:
        external_id = _get(row, "external_id")
        if not external_id:
            continue
        if str(_get(row, "id")).startswith(_SYNTHETIC_ID_PREFIXES):
            continue
        groups.setdefault(
            (normalize_group_txid(str(external_id)), _get(row, "asset")), []
        ).append(row)

    for (_txid_key, asset), group in groups.items():
        senders = [
            row
            for row in group
            if _get(row, "direction") == "outbound" and int(_get(row, "amount") or 0) > 0
        ]
        if len(senders) < 2:
            continue  # single-source / not a transfer — other paths handle it
        if any(str(_get(row, "id")) in already_paired_ids for row in group):
            continue  # a leg is already handled (manual / split / detect_intra)
        sender_wallets = {str(_get(row, "wallet_id")) for row in senders}
        if len(sender_wallets) != len(senders):
            continue  # a wallet recorded two outbounds for one tx — odd, decline

        parsed = None
        for row in senders:
            parsed = _parse_onchain_tx(_get(row, "raw_json"))
            if parsed is not None:
                break
        if parsed is None:
            continue  # Liquid / CSV — no readable graph; leave to quarantine

        fees = {int(_get(row, "fee") or 0) for row in senders}
        if len(fees) != 1:
            continue  # contributors disagree on the fee — not all the node's
        fee = next(iter(fees))

        chain_network = _source_chain_network(
            parsed["inputs"], index, str(_get(senders[0], "wallet_id"))
        )
        dest_value: dict[str, int] = {}
        external_sats = 0
        ambiguous = False
        for output in parsed["outputs"]:
            matches = index.lookup_script(output["script"])
            if chain_network is not None:
                matches = [
                    match
                    for match in matches
                    if _norm_chain_network(match.chain, match.network) == chain_network
                ]
            if not matches:
                external_sats += int(output["value_sats"])
                continue
            owner_ids = {str(match.wallet_id) for match in matches}
            if owner_ids & sender_wallets:
                continue  # change back to a contributing wallet — excluded
            if len(owner_ids) > 1:
                ambiguous = True  # owned by two non-contributors — cannot route
                break
            dest_id = next(iter(owner_ids))
            dest_value[dest_id] = dest_value.get(dest_id, 0) + int(output["value_sats"])
        if ambiguous:
            continue
        if external_sats > 0:
            continue  # consolidation that also pays external — leave to review
        if len(dest_value) != 1:
            continue  # 0 destinations -> not a transfer; >=2 -> N->M, decline
        dest_wallet_id, out_c_sats = next(iter(dest_value.items()))
        out_c_msat = out_c_sats * SATS_TO_MSAT

        input_owner_ids = set().union(
            *(_input_owner_ids(index, entry) for entry in parsed["inputs"])
        )
        if not sender_wallets <= input_owner_ids:
            continue  # every claimed sender must actually fund at least one input
        if not _inputs_owned_by(parsed["inputs"], index, sender_wallets):
            continue  # a foreign input makes the recorded amounts unreliable

        n = len(senders)
        sum_amounts = sum(int(_get(row, "amount") or 0) for row in senders)
        if sum_amounts + (n - 1) * fee != out_c_msat:
            continue  # conservation broken (sync gap / stale graph) — decline

        # Destination-receipt reconciliation. The legs credit ``out_C`` to the
        # destination, so any *existing* recorded receipt of these same coins
        # must be removed to avoid double-counting. That is only safe when the
        # receipt sits in this spend's own (external_id, asset) group — then it
        # is unambiguously this transaction and we drop it below. Two cases force
        # a decline back to the single-source deriver's conservative block:
        #   * a same-group destination receipt whose recorded value disagrees
        #     with the graph total (an odd / partial sync), and
        #   * a same-asset destination receipt recorded OUTSIDE this group whose
        #     value equals the consolidated total — almost certainly this very
        #     receipt under a different id (CSV / separate sync), which we cannot
        #     match to the spend without amount/time heuristics.
        group_ids = {str(_get(r, "id")) for r in group}
        asset_key = str(asset or "").upper()
        dest_in_group_total = sum(
            int(_get(r, "amount") or 0)
            for r in group
            if _get(r, "direction") == "inbound"
            and str(_get(r, "wallet_id")) == dest_wallet_id
        )
        if dest_in_group_total and dest_in_group_total != out_c_msat:
            continue
        # A same-asset destination receipt recorded OUTSIDE this group is
        # plausibly THIS consolidation's receipt under another id (CSV / separate
        # sync) when its amount is compatible with the consolidated total (exact,
        # or within a fee/rounding tolerance — 0.79999 vs a 0.8 graph total) AND
        # it is not a provably different on-chain transaction. Crediting
        # synthetic legs on top of such a receipt would double-count the
        # destination, so decline. The signal is AMOUNT + txid-novelty, NOT a
        # time window: a blunt 24h window false-declined a sync-gapped
        # consolidation whenever the destination merely had an unrelated near-time
        # deposit (booking phantom disposals), and missed a same-amount receipt
        # recorded outside the window (double-count). An unrelated deposit of a
        # different magnitude — at any time — must not look like this receipt.
        # Compare against the PARSED graph txid, not the group key: senders may be
        # grouped by an imported provider id (`_txid_key`) while the destination
        # recorded its receipt under the real on-chain txid, so keying on
        # `_txid_key` would treat the real receipt as a "different" tx and
        # double-count it. Skip receipts already handled elsewhere
        # (`already_paired_ids`) — an unrelated, separately-paired same-amount
        # deposit must not false-decline this consolidation.
        graph_txid = normalize_group_txid(str(parsed.get("txid") or _txid_key))
        has_external_receipt = any(
            _get(r, "direction") == "inbound"
            and str(_get(r, "wallet_id")) == dest_wallet_id
            and str(_get(r, "asset") or "").upper() == asset_key
            and str(_get(r, "id")) not in group_ids
            and str(_get(r, "id")) not in already_paired_ids
            and not _is_provably_different_onchain_tx(_get(r, "external_id"), graph_txid)
            and _amounts_compatible(int(_get(r, "amount") or 0), out_c_msat)
            for r in rows
        )
        if has_external_receipt:
            continue

        dest_ref = wallet_refs_by_id.get(dest_wallet_id)
        if dest_ref is None:
            continue  # cannot book the MOVE target safely

        dropped_destination_rows = tuple(
            row
            for row in group
            if _get(row, "direction") == "inbound"
            and str(_get(row, "wallet_id")) == dest_wallet_id
        )
        if len(dropped_destination_rows) > 1:
            continue  # ambiguous recorded destination receipt split

        # Whole fee on the largest contributor; deterministic tie-break.
        senders_sorted = sorted(
            senders,
            key=lambda row: (-int(_get(row, "amount") or 0), str(_get(row, "wallet_id"))),
        )
        bearer_id = str(_get(senders_sorted[0], "id"))
        txid = str(parsed.get("txid") or _txid_key)
        transfer_group_id = f"multi-consol:{txid}"

        leg_pairs: list[dict[str, Any]] = []
        leg_rows: list[dict[str, Any]] = []
        ok = True
        for row in senders_sorted:
            a_s = int(_get(row, "amount") or 0)
            is_bearer = str(_get(row, "id")) == bearer_id
            leg_value = a_s if is_bearer else a_s + fee
            leg_fee = fee if is_bearer else 0
            in_journal_id = (
                str(_get(dropped_destination_rows[0], "id"))
                if dropped_destination_rows
                else str(_get(row, "id"))
            )
            if leg_value <= 0:
                ok = False
                break
            wallet = str(_get(row, "wallet_id"))
            out_leg = _clone_row(
                row,
                amount=leg_value,
                fee=leg_fee,
                row_id=f"multi-consol:{txid}:out:{wallet}",
                external_id=f"multi-consol:{txid}:out:{wallet}",
                kind="self_transfer_out",
                journal_transaction_id=str(_get(row, "id")),
            )
            in_leg = _clone_row(
                row,
                amount=leg_value,
                fee=0,
                    row_id=f"multi-consol:{txid}:in:{wallet}",
                    external_id=f"multi-consol:{txid}:in:{wallet}",
                    kind="self_transfer_in",
                    journal_transaction_id=in_journal_id,
                    direction="inbound",
                    wallet_id=dest_wallet_id,
                    wallet_ref=dest_ref,
            )
            leg_pairs.append(
                {
                    "out": out_leg,
                    "in": in_leg,
                    "source": "multi_source_consolidation",
                    "group_id": transfer_group_id,
                    "group_block_rows": dropped_destination_rows,
                }
            )
            leg_rows.append(out_leg)
            leg_rows.append(in_leg)
        if not ok:
            continue

        result.derived_pairs.extend(leg_pairs)
        result.synthetic_rows.extend(leg_rows)
        result.dropped_out_ids.update(str(_get(row, "id")) for row in senders)
        for row in group:
            if (
                _get(row, "direction") == "inbound"
                and str(_get(row, "wallet_id")) == dest_wallet_id
            ):
                result.dropped_in_ids.add(str(_get(row, "id")))

    return result


def graph_partial_payment_out_ids(
    pairs: Sequence[Mapping[str, Any]], index: Any
) -> set[str]:
    """Out-row ids of clean 1-out/1-in pairs that are graph-proven partial payments.

    ``detect_intra_transfers`` pairs a same-txid 1-out/1-in self-transfer before
    the address-ownership deriver runs. When that spend's outbound *also* paid a
    non-owned recipient (a partial payment), the 1-out/1-in pairing absorbs the
    external payment into the implied MOVE fee (``sent - received``) and never
    taxes it as a disposal. Withholding such a pair lets
    :func:`derive_ownership_transfers` re-derive it from the graph: it books the
    owned leg as a MOVE and keeps the external residual as a real disposal.

    A pair is flagged only when, from the outbound row's readable graph:

    * the inputs are single-source (the contributor owns every input), and
    * some value landed in *other* owned wallets (an owned destination exists),
      and the recorded outbound ``amount`` (its non-change output total) exceeds
      that owned-to-others value — i.e. the remainder left to a non-owned
      recipient.

    Graphless rows (CSV / Liquid) and pure self-transfers (no external residual)
    are left to ``detect_intra_transfers``.
    """
    if index is None:
        return set()
    flagged: set[str] = set()
    for pair in pairs:
        out_row = pair.get("out") if hasattr(pair, "get") else pair["out"]
        if out_row is None:
            continue
        parsed = _parse_onchain_tx(_get(out_row, "raw_json"))
        if parsed is None:
            continue
        source_wallet_id = str(_get(out_row, "wallet_id"))
        if not _inputs_are_single_source_or_recorded_source(
            parsed["inputs"], index, source_wallet_id, out_row
        ):
            continue
        chain_network = _source_chain_network(
            parsed["inputs"], index, source_wallet_id
        )
        owned_to_others_sats = 0
        owned_dest_wallets: set[str] = set()
        for output in parsed["outputs"]:
            matches = index.lookup_script(output["script"])
            if chain_network is not None:
                matches = [
                    match
                    for match in matches
                    if _norm_chain_network(match.chain, match.network) == chain_network
                ]
            if not matches:
                continue  # external recipient / OP_RETURN
            owner_ids = {str(match.wallet_id) for match in matches}
            if source_wallet_id in owner_ids:
                continue  # change back to self
            owned_to_others_sats += int(output["value_sats"])
            owned_dest_wallets |= owner_ids
        owned_to_others_msat = owned_to_others_sats * SATS_TO_MSAT
        amount_msat = int(_get(out_row, "amount") or 0)
        # Withhold so the ownership deriver re-derives this spend when EITHER it
        # also paid a non-owned recipient (external residual) OR it fanned out to
        # two or more owned wallets. In the fan-out case detect_intra_transfers
        # pairs only the single destination that recorded a same-txid inbound,
        # leaving the other owned legs to be absorbed as a (taxable) MOVE fee —
        # so the deriver must decompose the full 1->N fan-out instead.
        external_residual = owned_to_others_msat > 0 and amount_msat > owned_to_others_msat
        if external_residual or len(owned_dest_wallets) >= 2:
            flagged.add(str(_get(out_row, "id")))
    return flagged


def graph_multi_owned_destination_out_ids(
    pairs: Sequence[Mapping[str, Any]], index: Any
) -> set[str]:
    """Out-row ids whose graph pays two or more non-source owned wallets.

    This is the subset of :func:`graph_partial_payment_out_ids` that is withheld
    because a same-txid 1-out/1-in pair hides a larger 1->N owned fan-out. If
    the ownership deriver later declines that fan-out, restoring only the
    original 1->1 pair is unsafe: the extra owned destination can still book as a
    standalone acquisition while the restored pair quarantines the source group,
    inflating holdings. Single-owned-destination partial payments keep the old
    restore path.
    """
    if index is None:
        return set()
    flagged: set[str] = set()
    for pair in pairs:
        out_row = pair.get("out") if hasattr(pair, "get") else pair["out"]
        if out_row is None:
            continue
        parsed = _parse_onchain_tx(_get(out_row, "raw_json"))
        if parsed is None:
            continue
        source_wallet_id = str(_get(out_row, "wallet_id"))
        if not _inputs_are_single_source_or_recorded_source(
            parsed["inputs"], index, source_wallet_id, out_row
        ):
            continue
        chain_network = _source_chain_network(
            parsed["inputs"], index, source_wallet_id
        )
        owned_dest_wallets: set[str] = set()
        for output in parsed["outputs"]:
            matches = index.lookup_script(output["script"])
            if chain_network is not None:
                matches = [
                    match
                    for match in matches
                    if _norm_chain_network(match.chain, match.network) == chain_network
                ]
            owner_ids = {str(match.wallet_id) for match in matches}
            if source_wallet_id in owner_ids:
                continue
            if len(owner_ids) == 1:
                owned_dest_wallets |= owner_ids
        if len(owned_dest_wallets) >= 2:
            flagged.add(str(_get(out_row, "id")))
    return flagged


def detect_conflicting_spend_ids(rows: Sequence[Mapping[str, Any]]) -> set[str]:
    """Row ids of transactions that conflict over a shared input outpoint.

    Two transactions spending the SAME prevout (an RBF replacement, a reorg
    replacement, or a double-spend) can never both confirm on-chain, yet nothing
    else reconciles them: ``detect_intra_transfers`` and the derivers key on txid,
    so each conflicting self-transfer is booked independently as a carrying MOVE —
    inflating the destination and over-debiting the source. Detect the conflict
    from the stored graph's input outpoints. When exactly ONE conflicting txid is
    confirmed it is the on-chain winner and the others are losers; otherwise (none
    or several confirmed) every conflicting txid is returned so the whole conflict
    surfaces for review rather than being mis-booked. Returns ALL rows (out and in
    legs) of every loser txid, identified by normalized ``external_id``.

    This is the self-transfer-scoped slice of the broader RBF/reorg canonicalization
    pass; it is purely a quarantine signal and never books anything.
    """
    row_txid: dict[str, str] = {}
    txid_confirmed: dict[str, bool] = {}
    outpoint_txids: dict[str, set[str]] = {}
    for row in rows:
        parsed = _parse_onchain_tx(_get(row, "raw_json"))
        # A synthetic split / direct-payout leg keeps the REAL transaction in
        # raw_json but renames external_id (e.g. "cross-split:..."), so prefer the
        # parsed graph txid and fall back to external_id. Keying every leg this way
        # ensures a losing transaction's synthetic legs are quarantined too — not
        # just the rows whose external_id literally equals the txid.
        txid = normalize_group_txid(
            str((parsed.get("txid") if parsed else None) or _get(row, "external_id") or "")
        )
        if not txid:
            continue
        row_txid[str(_get(row, "id"))] = txid
        # Confirmation can land on ANY leg of a transaction — when wallets sync at
        # different times a destination inbound may be confirmed while the source's
        # outbound row is still unconfirmed — so fold every row's state into the
        # per-txid confirmation, not just outbound rows.
        txid_confirmed[txid] = txid_confirmed.get(txid, False) or bool(
            _get(row, "confirmed_at")
        )
        if parsed is not None:
            # Collect input outpoints from ANY leg carrying the graph, not just
            # outbound rows: a conflict whose loser was synced only as a
            # destination INBOUND still has the full vin in its raw_json. Keying
            # only off outbound rows would miss it and let the loser inbound book
            # as a phantom acquisition.
            for entry in parsed["inputs"]:
                outpoint = entry.get("outpoint")
                if outpoint:
                    outpoint_txids.setdefault(outpoint, set()).add(txid)

    loser_txids: set[str] = set()
    for txids in outpoint_txids.values():
        if len(txids) < 2:
            continue  # one transaction owns this outpoint — no conflict
        confirmed = {txid for txid in txids if txid_confirmed.get(txid)}
        if len(confirmed) == 1:
            loser_txids |= txids - confirmed  # the unconfirmed replacements lose
        else:
            loser_txids |= txids  # ambiguous — surface the whole conflict
    return {rid for rid, txid in row_txid.items() if txid in loser_txids}


# -- internals --------------------------------------------------------------


def _inputs_owned_by(
    inputs: Sequence[Mapping[str, Any]], index: Any, owner_set: set[str]
) -> bool:
    """True only when every input is owned, and owned solely by ``owner_set``.

    Used by the multi-source consolidation deriver to reject a spend that pulls
    in a foreign input (payjoin/coinjoin or unwatched coins) — which would make
    the per-wallet recorded amounts and fee unreliable for splitting. A shared
    descriptor that maps an input to a contributor *and* an outside wallet is
    also rejected (the input's owner set must be a subset of the contributors).
    """
    if not inputs:
        return False
    for entry in inputs:
        owners = _input_owner_ids(index, entry)
        if not owners or not owners <= owner_set:
            return False
    return True


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


def _inputs_are_single_source_or_recorded_source(
    inputs: Sequence[Mapping[str, Any]],
    index: Any,
    source_wallet_id: str,
    row: Mapping[str, Any],
) -> bool:
    """Accept a single-input outbound row when historical input ownership is absent.

    ``record_from_bitcoin_esplora_tx`` can only create a positive outbound row
    for a wallet when tracked source value left that wallet. If the spend has
    exactly one input, the source wallet necessarily funded that input even when
    the ownership index cannot resolve the old spent outpoint (for example,
    because the wallet was first inventoried after that output was already
    spent). Keep multi-input spends on the strict index-only path.
    """
    if _inputs_are_single_source(inputs, index, source_wallet_id):
        return True
    if (
        len(inputs) != 1
        or _get(row, "direction") != "outbound"
        or int(_get(row, "amount") or 0) <= 0
        or str(_get(row, "wallet_id")) != source_wallet_id
    ):
        return False
    outpoint = inputs[0].get("outpoint")
    if not outpoint:
        return False
    prev_txid = str(outpoint).split(":", 1)[0].lower()
    return any(
        str(wallet_id) == source_wallet_id
        for wallet_id, _wallet_label in index.txid_wallets.get(prev_txid, set())
    )


def _input_owner_ids(index: Any, entry: Mapping[str, Any]) -> set[str]:
    """All owned-wallet ids for an input (outpoint inventory wins; else script).

    The outpoint inventory is unambiguous (one wallet per UTXO); only the
    script fallback can map to several wallets, and we return the full set so
    callers can reason about ambiguity instead of an arbitrary first match.
    """
    outpoint = entry.get("outpoint")
    if outpoint:
        matches = _lookup_outpoint(index, outpoint)
        if matches:
            return {str(match.wallet_id) for match in matches}
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

    NOTE: a genuinely blank chain AND network normalizes to ``("bitcoin",
    "main")`` here (``normalize_chain("")`` defaults empty to bitcoin). That is
    intentional for legacy address-list / inventory matches that stored no chain
    metadata — they are Bitcoin mainnet, and a bitcoin/main source paying one of
    them must still pass the same-chain filter. Distinguishing a genuinely-unknown
    cross-chain blank from a legacy-mainnet blank has to happen when the index is
    BUILT (stamp bitcoin/main on legacy blanks), not at comparison time, or a real
    same-chain self-transfer would be mis-booked as an external disposal. See the
    deferred C2 item in TODO.md.
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
            for match in _lookup_outpoint(index, outpoint):
                if str(match.wallet_id) == source_wallet_id:
                    return _norm_chain_network(match.chain, match.network)
        for match in index.lookup_script(entry.get("script")):
            if str(match.wallet_id) == source_wallet_id:
                return _norm_chain_network(match.chain, match.network)
    return None


def _lookup_outpoint(index: Any, outpoint: Any) -> list[Any]:
    if hasattr(index, "lookup_outpoint"):
        return list(index.lookup_outpoint(outpoint))
    value = getattr(index, "by_outpoint", {}).get(str(outpoint or "").lower())
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


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

    def _different_onchain_tx(row: Mapping[str, Any]) -> bool:
        return _is_provably_different_onchain_tx(_get(row, "external_id"), txid)

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

    # A same-asset candidate that is NOT a provably-different on-chain tx and is
    # amount-compatible with this leg could BE this leg recorded under another id
    # (CSV / settlement-dated / late sync) — at ANY time. Reusing it risks
    # cannibalizing an unrelated deposit; synthesizing risks a duplicate
    # transfer_in (silent holdings inflation). So decline for review. AMOUNT (not
    # a time window) is the discriminator: an unrelated deposit of a different
    # magnitude does not block, and a real same-amount receipt recorded outside
    # any window is no longer either double-counted (synthesize) or missed.
    blocking = [
        row
        for row in available
        if not _different_onchain_tx(row)
        and _amounts_compatible(int(_get(row, "amount") or 0), leg_msat)
    ]
    if blocking:
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


def _is_provably_different_onchain_tx(external_id: Any, txid: str) -> bool:
    """True when ``external_id`` is a 64-hex txid that is NOT this spend's txid.

    Such a row provably belongs to a *different* on-chain transaction, so it is a
    separate receipt — never this self-transfer's destination leg.
    """
    # Strip before comparing: _looks_like_txid strips internally, so a
    # whitespace-wrapped " <txid> " would otherwise validate as a txid yet compare
    # unequal to the bare txid and be misclassified as a DIFFERENT transaction
    # (synthesizing a duplicate transfer-in / double-counting the destination).
    text = str(external_id or "").strip()
    return _looks_like_txid(text) and text.lower() != str(txid or "").strip().lower()


def _amounts_compatible(a_msat: int, b_msat: int) -> bool:
    """Whether two msat amounts are close enough to be the same receipt.

    A destination may record a receipt net of a small internal/settlement fee or
    with sat rounding, so allow the swap-fee tolerance (``max(1%, 2500 sats)``).
    This is the AMOUNT signal that replaces the old blunt 24h time window for
    deciding whether an off-group inbound is this self-transfer's receipt: an
    unrelated deposit of a different magnitude must not look like a match (which
    would either false-decline a real move or double-count a real receipt).
    """
    tolerance = max(abs(b_msat) // 100, 2_500_000)  # 1% or 2500 sats, in msat
    return abs(int(a_msat) - int(b_msat)) <= tolerance


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
