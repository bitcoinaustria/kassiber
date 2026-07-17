"""Derive graph-proven on-chain custody moves for the journal interpreter.

The pure deriver consumes canonical transaction graphs plus one profile-wide
``OwnedIndex``. It emits conserving 1:N and N:1 owned-wallet legs, leaves
external output residuals for ordinary disposal treatment, and fails closed on
mixed-owner, PayJoin, CoinJoin, ambiguous N:M, or unknown-valued Liquid graphs.
Exact native-event claims remain stronger at arbitration time.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from decimal import Decimal
from typing import Any, Callable, Mapping, Optional, Sequence

from ..msat import msat_to_btc
from ..transfers import (
    bitcoin_network_domain_evidence,
    canonical_txid,
    normalize_group_txid,
    onchain_transfer_scope,
)
from ..wallet_descriptors import normalize_chain, normalize_network
from .onchain import (
    exact_onchain_fee_msat_from_parsed,
    merge_ownership_txs,
    parse_ownership_tx,
    parse_valued_tx,
    stored_tx_mapping,
)


SATS_TO_MSAT = 1000
# Synthetic outbound rows minted by earlier engine stages (direct-payout splits,
# cross-asset splits) keep the real txid in raw_json but are NOT fresh spends to
# re-decompose; skip them by id prefix.
_SYNTHETIC_ID_PREFIXES = (
    "owned-derive:",
    "recorded-fanout:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
)


@dataclass(frozen=True)
class OwnershipDeriveResult:
    """What :func:`derive_ownership_transfers` contributes to the engine run.

    * ``derived_pairs`` — ``{"out": out_leg, "in": in_row, "source": ...}`` in
      the shape the journal pipeline's native interpreter consumes, plus a
      provenance marker.
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


@dataclass(frozen=True)
class OwnershipReviewProof:
    """A graph-backed, user-pairable ownership review suggestion.

    Only real transaction rows are carried so accepting the suggestion can use
    the existing ``transaction_pairs`` store. ScriptPubKeys and derivation paths
    never leave this module.
    """

    out_row: Mapping[str, Any]
    in_row: Mapping[str, Any]
    owned_amount_msat: int
    reason: str
    conflict_set_id: str
    confidence: str
    conflict_size: int = 1


@dataclass(frozen=True)
class ProfileTransferDerivation:
    """Shared ownership pipeline result for journal and graph preview."""

    consolidation: OwnershipDeriveResult
    ownership: OwnershipDeriveResult
    fanout: OwnershipDeriveResult


def _rows_after_derivation(
    rows: Sequence[Mapping[str, Any]],
    result: OwnershipDeriveResult,
    *,
    sort_key: Callable[[Mapping[str, Any]], Any] | None,
) -> list[Mapping[str, Any]]:
    drop_ids = result.dropped_out_ids | result.dropped_in_ids
    next_rows = [
        result.out_row_overrides.get(str(_get(row, "id")), row)
        for row in rows
        if str(_get(row, "id")) not in drop_ids
    ]
    if result.synthetic_rows:
        next_rows.extend(result.synthetic_rows)
    return sorted(next_rows, key=sort_key) if sort_key is not None else next_rows


def derive_profile_transfers(
    rows: Sequence[Mapping[str, Any]],
    *,
    index: Any,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    already_paired_ids: set[str] | None = None,
    sort_key: Callable[[Mapping[str, Any]], Any] | None = None,
) -> ProfileTransferDerivation:
    """Run consolidation -> exact recorded fan-out -> graph ownership.

    Callers remain responsible for manual-pair, payout, quarantine, and tax
    policy. This function owns the ordering and the row set each deriver sees,
    preventing graph preview and journal booking from silently diverging.
    """

    handled = set(already_paired_ids or ())
    consolidation = derive_multi_source_consolidations(
        rows,
        index=index,
        wallet_refs_by_id=wallet_refs_by_id,
        already_paired_ids=handled,
    )
    rows_after_consolidation = _rows_after_derivation(
        rows, consolidation, sort_key=sort_key
    )
    handled |= consolidation.dropped_out_ids | consolidation.dropped_in_ids

    fanout = derive_recorded_fanout_transfers(
        rows_after_consolidation,
        already_paired_ids=handled,
    )
    rows_after_recorded_fanout = _rows_after_derivation(
        rows_after_consolidation, fanout, sort_key=sort_key
    )
    handled |= fanout.dropped_out_ids | fanout.dropped_in_ids
    handled |= {str(out_id) for out_id in fanout.out_row_overrides}
    handled |= {
        str(_get(pair.get("in"), "id")) for pair in fanout.derived_pairs
    }

    ownership = derive_ownership_transfers(
        rows_after_recorded_fanout,
        index=index,
        wallet_refs_by_id=wallet_refs_by_id,
        already_paired_ids=handled,
    )
    return ProfileTransferDerivation(
        consolidation=consolidation,
        ownership=ownership,
        fanout=fanout,
    )


_PAIRABLE_OWNERSHIP_REVIEW_REASONS = frozenset(
    {
        "owned_fanout_unresolved",
        "ownership_transfer_destination_ambiguous",
        "ownership_transfer_source_ambiguous",
    }
)
_REUSABLE_OWNERSHIP_PAIR_KINDS = frozenset({"manual", "coinjoin", "whirlpool"})


def derive_ownership_review_proofs(
    rows: Sequence[Mapping[str, Any]],
    *,
    index: Any,
    blocked_reasons_by_row_id: Mapping[str, str],
    active_pair_records: Sequence[Mapping[str, Any]] = (),
) -> list[OwnershipReviewProof]:
    """Return pair-store-compatible review proofs for blocked ownership moves.

    The journal remains authoritative about which sources are blocked. This
    helper only turns those persisted review reasons into actionable 1:1 links:
    a unique owned output wallet plus a compatible real inbound row. Missing
    destination rows, ambiguous script ownership, conflicts, and amount-mismatch
    blocks stay in quarantine rather than inventing a pair.
    """

    if index is None:
        return []
    active_records = list(active_pair_records)
    active_pairs = {
        (
            str(_get(record, "out_transaction_id") or ""),
            str(_get(record, "in_transaction_id") or ""),
        )
        for record in active_records
    }
    rows_by_id = {str(_get(row, "id")): row for row in rows}
    unavailable_leg_ids: set[str] = set()
    for record in active_records:
        out_id = str(_get(record, "out_transaction_id") or "")
        in_id = str(_get(record, "in_transaction_id") or "")
        # Direct payouts claim the outbound outright. Cross-asset and special
        # one-to-one pair kinds cannot participate in same-asset multi-leg
        # reuse either; offering those cards would make the pair API reject.
        if not in_id:
            unavailable_leg_ids.add(out_id)
            continue
        out_row = rows_by_id.get(out_id)
        in_row = rows_by_id.get(in_id)
        kind = str(_get(record, "kind") or "manual")
        policy = str(_get(record, "policy") or "carrying-value")
        if (
            out_row is None
            or in_row is None
            or str(_get(out_row, "asset") or "").upper()
            != str(_get(in_row, "asset") or "").upper()
            or policy != "carrying-value"
            or kind not in _REUSABLE_OWNERSHIP_PAIR_KINDS
        ):
            unavailable_leg_ids |= {out_id, in_id}
    inbound_by_wallet: dict[str, list[Mapping[str, Any]]] = {}
    groups: dict[tuple[Any, ...], list[Mapping[str, Any]]] = {}
    for row in rows:
        if _get(row, "direction") == "inbound" and int(_get(row, "amount") or 0) > 0:
            inbound_by_wallet.setdefault(str(_get(row, "wallet_id")), []).append(row)
        row_scope = onchain_transfer_scope(row)
        legacy_external_id = normalize_group_txid(
            str(_get(row, "external_id") or "")
        )
        if legacy_external_id:
            legacy_review_key = (
                "unscoped_review",
                legacy_external_id,
                str(_get(row, "asset") or "").upper(),
            )
            groups.setdefault(legacy_review_key, []).append(row)
        if row_scope is not None:
            groups.setdefault(("physical", *row_scope), []).append(row)

    proofs: list[OwnershipReviewProof] = []
    for source_id, reason in sorted(blocked_reasons_by_row_id.items()):
        if reason not in _PAIRABLE_OWNERSHIP_REVIEW_REASONS:
            continue
        source = rows_by_id.get(str(source_id))
        if (
            source is None
            or source_id in unavailable_leg_ids
            or _get(source, "direction") != "outbound"
            or int(_get(source, "amount") or 0) <= 0
        ):
            continue
        parsed = _parse_onchain_tx(_get(source, "raw_json"))
        destinations: dict[str, int] = {}
        source_scope: tuple[str, str, str, str] | None = None
        if parsed is not None:
            source_wallet_id = str(_get(source, "wallet_id"))
            source_scope = _ownership_onchain_scope(
                source, index=index, parsed=parsed
            )
            if source_scope is None:
                continue
            physical_scope = (source_scope[0], source_scope[1])
            ambiguous = False
            for output in parsed["outputs"]:
                matches = _matches_in_physical_scope(
                    index.lookup_script(output["script"]), physical_scope
                )
                owner_ids = {str(match.wallet_id) for match in matches}
                if source_wallet_id in owner_ids:
                    continue
                if len(owner_ids) > 1:
                    ambiguous = True
                    break
                if len(owner_ids) == 1:
                    owner_id = next(iter(owner_ids))
                    destinations[owner_id] = destinations.get(owner_id, 0) + (
                        int(output["value_sats"]) * SATS_TO_MSAT
                    )
            if ambiguous:
                continue
        elif reason == "owned_fanout_unresolved":
            source_scope = onchain_transfer_scope(source)
            legacy_external_id = normalize_group_txid(
                str(_get(source, "external_id") or "")
            )
            if source_scope is None and not legacy_external_id:
                continue
            group_key = (
                ("physical", *source_scope)
                if source_scope is not None
                else (
                    "unscoped_review",
                    legacy_external_id,
                    str(_get(source, "asset") or "").upper(),
                )
            )
            group = groups.get(group_key, [])
            positive_outs = [
                row
                for row in group
                if _get(row, "direction") == "outbound"
                and int(_get(row, "amount") or 0) > 0
            ]
            if len(positive_outs) != 1 or str(_get(positive_outs[0], "id")) != source_id:
                continue
            for inbound in group:
                if (
                    _get(inbound, "direction") == "inbound"
                    and int(_get(inbound, "amount") or 0) > 0
                    and str(_get(inbound, "wallet_id"))
                    != str(_get(source, "wallet_id"))
                ):
                    wallet_id = str(_get(inbound, "wallet_id"))
                    destinations[wallet_id] = destinations.get(wallet_id, 0) + int(
                        _get(inbound, "amount") or 0
                    )

        txid = str((parsed or {}).get("txid") or _get(source, "external_id") or "")
        source_scope = source_scope or onchain_transfer_scope(source)
        for destination_wallet_id, owned_amount_msat in sorted(destinations.items()):
            available = [
                inbound
                for inbound in inbound_by_wallet.get(destination_wallet_id, ())
                if str(_get(inbound, "asset") or "").upper()
                == str(_get(source, "asset") or "").upper()
                and str(_get(inbound, "id")) not in unavailable_leg_ids
                and (source_id, str(_get(inbound, "id"))) not in active_pairs
            ]
            compatible: list[tuple[Mapping[str, Any], str]] = []
            exact: list[tuple[Mapping[str, Any], str]] = []
            for inbound in available:
                inbound_amount_msat = int(_get(inbound, "amount") or 0)
                if not _amounts_compatible(inbound_amount_msat, owned_amount_msat):
                    continue
                inbound_scope = onchain_transfer_scope(inbound)
                if (
                    source_scope is not None
                    and inbound_scope is not None
                    and source_scope != inbound_scope
                ):
                    continue
                if _is_provably_different_onchain_tx(
                    _get(inbound, "external_id"), txid
                ):
                    continue

                # ``exact`` is deliberately a whole-row statement: accepting a
                # transaction-pair link consumes both real rows.  A graph output
                # that merely resembles one provider/import row by amount is still
                # valuable review evidence, but it cannot prove that row is the
                # complete destination leg.  Require the source graph, canonical
                # chain/network/txid equality, and exact coverage of both rows.
                confidence = "strong"
                if (
                    parsed is not None
                    and reason != "ownership_transfer_source_ambiguous"
                    and source_scope is not None
                    and inbound_scope == source_scope
                    and int(_get(source, "amount") or 0) == owned_amount_msat
                    and inbound_amount_msat == owned_amount_msat
                ):
                    confidence = "exact"
                    exact.append((inbound, confidence))
                else:
                    compatible.append((inbound, confidence))
            candidates = exact or compatible
            if not candidates:
                continue
            conflict_set_id = (
                f"ownership-review:{source_id}:{destination_wallet_id}"
            )
            for inbound, confidence in candidates:
                proofs.append(
                    OwnershipReviewProof(
                        out_row=source,
                        in_row=inbound,
                        owned_amount_msat=owned_amount_msat,
                        reason=reason,
                        conflict_set_id=conflict_set_id,
                        confidence=confidence,
                        conflict_size=len(candidates),
                    )
                )
    return proofs


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
        # The journal skips building the descriptor ownership index for a
        # profile whose imports contain no vout graph at all.  Different wallet
        # rows still prove that the receipt is profile-owned, and Liquid's
        # separately recorded miner fee makes any amount shortfall unsafe to
        # auto-pair.  Preserve that blocker even in the index-free fast path.
        for row in rows:
            if (
                _get(row, "direction") != "outbound"
                or not _row_is_liquid(row)
                or str(_get(row, "id")) in already_paired_ids
            ):
                continue
            row_scope = onchain_transfer_scope(row)
            if row_scope is None:
                continue
            same_tx_ins = [
                candidate
                for candidate in rows
                if _get(candidate, "direction") == "inbound"
                and str(_get(candidate, "wallet_id")) != str(_get(row, "wallet_id"))
                and str(_get(candidate, "asset") or "").upper()
                == str(_get(row, "asset") or "").upper()
                and onchain_transfer_scope(candidate) == row_scope
            ]
            owned_receipts_msat = sum(
                int(_get(candidate, "amount") or 0) for candidate in same_tx_ins
            )
            if same_tx_ins and owned_receipts_msat != int(_get(row, "amount") or 0):
                _block_source(
                    result,
                    row,
                    "liquid_transfer_graph_incomplete",
                    {
                        "row_amount_msat": int(_get(row, "amount") or 0),
                        "owned_receipts_msat": owned_receipts_msat,
                    },
                )
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
    parsed_by_row_id = _profile_parsed_transactions(rows, index)
    parsed_by_out_id: dict[str, Optional[dict[str, Any]]] = {}
    source_groups: dict[tuple[str, str, str, str], list[Mapping[str, Any]]] = {}
    for row in rows:
        if _get(row, "direction") != "outbound":
            continue
        source_id = str(_get(row, "id"))
        if source_id.startswith(_SYNTHETIC_ID_PREFIXES):
            continue
        if int(_get(row, "amount") or 0) <= 0:
            continue
        parsed = parsed_by_row_id.get(source_id)
        parsed_by_out_id[source_id] = parsed
        scope = _ownership_onchain_scope(row, index=index, parsed=parsed)
        if parsed is None or scope is None:
            continue
        source_groups.setdefault(scope, []).append(row)
    duplicate_source_groups: dict[str, list[Mapping[str, Any]]] = {}
    for group in source_groups.values():
        economic_rows: dict[tuple[int, int, bool], list[Mapping[str, Any]]] = {}
        for source_row in group:
            economic_rows.setdefault(
                (
                    int(_get(source_row, "amount") or 0),
                    int(_get(source_row, "fee") or 0),
                    bool(_get(source_row, "amount_includes_fee")),
                ),
                [],
            ).append(source_row)
        for repeated in economic_rows.values():
            if len(repeated) > 1:
                for source_row in repeated:
                    duplicate_source_groups[str(_get(source_row, "id"))] = repeated

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
        parsed = parsed_by_out_id.get(source_id)
        if parsed is None:
            # A graphless Liquid 1:1 row is safe as a MOVE only when its
            # non-fee amount exactly equals the owned receipt.  Any shortfall
            # could be an external payment and must never be relabelled as a
            # transfer fee merely because the txid matches.
            if _row_is_liquid(row):
                row_scope = onchain_transfer_scope(row)
                same_tx_ins = [
                    candidate
                    for candidate in rows
                    if _get(candidate, "direction") == "inbound"
                    and str(_get(candidate, "wallet_id"))
                    != str(_get(row, "wallet_id"))
                    and str(_get(candidate, "asset") or "").upper()
                    == str(_get(row, "asset") or "").upper()
                    and row_scope is not None
                    and onchain_transfer_scope(candidate) == row_scope
                ]
                if same_tx_ins and sum(
                    int(_get(candidate, "amount") or 0)
                    for candidate in same_tx_ins
                ) != int(_get(row, "amount") or 0):
                    _block_source(
                        result,
                        row,
                        "liquid_transfer_graph_incomplete",
                        {
                            "row_amount_msat": int(_get(row, "amount") or 0),
                            "owned_receipts_msat": sum(
                                int(_get(candidate, "amount") or 0)
                                for candidate in same_tx_ins
                            ),
                        },
                    )
            continue
        group_key = _ownership_onchain_scope(row, index=index, parsed=parsed)
        if group_key is None:
            # A readable payload whose transaction identity is only a provider
            # record id is useful review evidence, but not an automatic L1 MOVE.
            continue
        physical_scope = (group_key[0], group_key[1])
        source_wallet_id = str(_get(row, "wallet_id"))
        component_inputs, component_inputs_complete = _component_inputs(parsed, row)
        if parsed.get("evidence_conflicts") or not component_inputs_complete:
            _block_source(
                result,
                row,
                "ownership_transfer_asset_evidence_incomplete",
                {
                    "evidence_conflicts": list(parsed.get("evidence_conflicts") or ()),
                },
            )
            continue

        # The canonical transaction group is authoritative.  Input ownership
        # evidence may be missing for an old spent output, but evidence that is
        # present must never point exclusively at another physical network.
        # Failing closed here prevents a same-txid/outpoint collision from
        # authorizing a carrying-value MOVE on the wrong chain.
        if _source_input_scope_contradicts(
            component_inputs,
            index,
            source_wallet_id,
            physical_scope,
        ):
            _block_source(
                result,
                row,
                "ownership_transfer_source_ambiguous",
                {
                    "canonical_chain": physical_scope[0],
                    "canonical_network": physical_scope[1],
                    "scope_conflict": True,
                },
            )
            continue

        # Aggregate owned outputs per destination wallet — sync records one
        # inbound row per wallet per tx, so a wallet receiving two outputs in
        # the same tx must pair as a single leg of their combined value.
        by_dest: dict[str, dict[str, Any]] = {}
        ambiguous_output = False
        incomplete_owned_output = False
        external_value_sats = 0
        external_value_complete = True
        for output in parsed["outputs"]:
            matches = _matches_in_physical_scope(
                index.lookup_script(output["script"]), physical_scope
            )
            asset_relation = _leg_asset_relation(output, parsed, row)
            if not matches:
                # External recipient, OP_RETURN, or a same-script-hex collision
                # on another chain/network — never an owned leg; folded into the
                # residual disposal.
                if asset_relation == "same" and output.get("value_sats") is not None:
                    external_value_sats += int(output["value_sats"])
                elif asset_relation != "different":
                    external_value_complete = False
                continue
            owner_ids = {str(match.wallet_id) for match in matches}
            if asset_relation == "different":
                continue
            if asset_relation == "unknown" or output.get("value_sats") is None:
                # Public script ownership says this is ours, but the profile
                # lacks the value/asset needed to conserve this component.
                incomplete_owned_output = True
                break
            if source_wallet_id in owner_ids:
                # The source wallet also owns this script -> change back to self.
                # (Matches the sync amount model, which excludes change.)
                continue
            if len(owner_ids) > 1:
                # Owned by two different non-source wallets (shared descriptor /
                # address reuse): we cannot route the leg unambiguously. Decline
                # the owned slice rather than guess a destination. Continue so
                # graph-proven external outputs remain independently classifiable.
                ambiguous_output = True
                continue
            owner = matches[0]
            dest_wallet_id = str(owner.wallet_id)
            slot = by_dest.setdefault(
                dest_wallet_id,
                {"value_sats": 0, "label": owner.wallet_label, "min_n": output["n"]},
            )
            slot["value_sats"] += int(output["value_sats"])
            slot["min_n"] = min(slot["min_n"], output["n"])
        if incomplete_owned_output:
            _block_source(
                result,
                row,
                "ownership_transfer_asset_evidence_incomplete",
                {
                    "missing": "owned_output_value_or_asset",
                },
            )
            continue
        if ambiguous_output:
            _block_source(
                result,
                row,
                "ownership_transfer_ambiguous_output",
                (
                    {"verified_external_msat": external_value_sats * SATS_TO_MSAT}
                    if external_value_complete and external_value_sats > 0
                    else None
                ),
            )
            continue
        if not by_dest:
            continue  # ordinary outbound payment — leave on the disposal path
        duplicate_group = duplicate_source_groups.get(source_id)
        if duplicate_group is not None:
            _block_source(
                result,
                row,
                "ownership_transfer_duplicate_outbound",
                {
                    "outbound_count": len(duplicate_group),
                    "outbound_ids": sorted(str(_get(item, "id")) for item in duplicate_group),
                },
            )
            continue
        if not _inputs_are_single_source_or_recorded_source(
            component_inputs,
            index,
            source_wallet_id,
            row,
            physical_scope=physical_scope,
        ):
            _block_source(result, row, "ownership_transfer_source_ambiguous")
            continue
        source_amount_msat = int(_get(row, "amount") or 0)
        source_fee_msat = int(_get(row, "fee") or 0)
        source_total_msat = source_amount_msat
        fee_inclusive = bool(_get(row, "amount_includes_fee"))
        fee_attribution = str(
            _get(row, "observation_fee_attribution") or "unknown"
        ).strip().lower()
        implicit_wallet_delta = (
            fee_inclusive and fee_attribution == "implicit_wallet_delta"
        )
        exact_folded_fee_msat: int | None = None
        if fee_inclusive and not implicit_wallet_delta:
            exact_folded_fee_msat = exact_onchain_fee_msat_from_parsed(
                parsed, asset=str(_get(row, "asset") or "")
            )
            source_fee_msat = (
                0 if exact_folded_fee_msat is None else exact_folded_fee_msat
            )
        else:
            source_total_msat += source_fee_msat
        conserves, conservation_detail = _liquid_source_conserves(
            parsed,
            row,
            index,
            source_wallet_id,
            component_inputs,
            source_total_msat,
            physical_scope,
        )
        if not conserves:
            _block_source(
                result,
                row,
                "ownership_transfer_conservation_mismatch",
                {
                    **conservation_detail,
                },
            )
            continue
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
                    "row_amount_msat": source_amount_msat,
                    "row_total_outflow_msat": source_total_msat,
                    "owned_outputs_msat": legs_value_msat,
                },
            )
            continue

        folded_gap_msat = max(0, source_total_msat - legs_value_msat)
        if fee_inclusive and not implicit_wallet_delta and folded_gap_msat > 0:
            if exact_folded_fee_msat is None:
                _block_source(
                    result,
                    row,
                    "ownership_transfer_fee_evidence_incomplete",
                    {
                        "folded_gap_msat": folded_gap_msat,
                        "missing": "exact_network_fee",
                    },
                    required_for="complete_transfer_component",
                )
                continue
            if exact_folded_fee_msat > folded_gap_msat:
                _block_source(
                    result,
                    row,
                    "ownership_transfer_conservation_mismatch",
                    {
                        "folded_gap_msat": folded_gap_msat,
                        "exact_network_fee_msat": exact_folded_fee_msat,
                    },
                    required_for="complete_transfer_component",
                )
                continue

        # ``parsed.txid`` may retain a provider/import label while the canonical
        # external id supplied the validated physical scope. Every automatic id
        # and destination reuse decision must use that canonical scope member.
        txid = group_key[2]
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
                onchain_scope=group_key,
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
                    "destination_wallet_id": dest_wallet_id,
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
                        "destination_wallet_id": dest_wallet_id,
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
    a group of rows sharing one canonical ``(chain, network, txid, asset)``
    scope across two or more profile wallets is, by construction, all owned,
    and the sync amount
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
    set. Runs before the address-ownership deriver: a complete, exactly
    conserving set of independently recorded wallet rows is stronger than a
    graph interpretation and must not be pre-empted by a weaker/ambiguous graph
    candidate. The claim boundary independently revalidates the whole group.
    """
    result = OwnershipDeriveResult()
    groups: dict[tuple[str, str, str, str], list[Mapping[str, Any]]] = {}
    for row in rows:
        scope = onchain_transfer_scope(row)
        if scope is None:
            continue
        if str(_get(row, "id")).startswith(_SYNTHETIC_ID_PREFIXES):
            continue
        groups.setdefault(scope, []).append(row)

    for (_chain, _network, txid, asset), group in groups.items():
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
        transfer_group_id = f"recorded-fanout:{txid}"
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
                row_id=f"recorded-fanout:{txid}:out:{_get(in_row, 'id')}",
                external_id=f"recorded-fanout:{txid}:out:{_get(in_row, 'id')}",
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
    * **Readable locally-valued graph + a single shared fee.** Bitcoin uses its
      ordinary graph; Liquid merges the complementary legs persisted by each
      wallet. Graphless CSV imports are skipped. A fee that differs across
      contributors means at least one row is not the whole-tx fee, so decline.
    * **Exact conservation.** A mismatch means a sync gap or stale graph.

    Runs *before* :func:`derive_ownership_transfers`; the caller must feed every
    id this pass touches (contributors + the destination receipt) into that
    deriver's ``already_paired_ids`` so the single-source deriver does not also
    block-and-quarantine the same contributors.
    """
    result = OwnershipDeriveResult()
    if index is None:
        return result
    parsed_by_row_id = _profile_parsed_transactions(rows, index)

    groups: dict[tuple[str, str, str, str], list[Mapping[str, Any]]] = {}
    for row in rows:
        parsed = parsed_by_row_id.get(str(_get(row, "id")))
        scope = _ownership_onchain_scope(row, index=index, parsed=parsed)
        if scope is None:
            continue
        if str(_get(row, "id")).startswith(_SYNTHETIC_ID_PREFIXES):
            continue
        groups.setdefault(scope, []).append(row)

    for (_chain, _network, txid_key, asset), group in groups.items():
        physical_scope = (_chain, _network)
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
            parsed = parsed_by_row_id.get(str(_get(row, "id")))
            if parsed is not None:
                break
        if parsed is None:
            continue  # CSV / legacy graphless record — leave to quarantine
        component_inputs, component_complete = _component_inputs(parsed, senders[0])
        if not component_complete or parsed.get("evidence_conflicts"):
            continue

        fees = {int(_get(row, "fee") or 0) for row in senders}
        if len(fees) != 1:
            continue  # contributors disagree on the fee — not all the node's
        fee = next(iter(fees))

        dest_value: dict[str, int] = {}
        external_sats = 0
        ambiguous = False
        for output in parsed["outputs"]:
            matches = _matches_in_physical_scope(
                index.lookup_script(output["script"]), physical_scope
            )
            relation = _leg_asset_relation(output, parsed, senders[0])
            if relation == "different":
                continue
            if relation == "unknown":
                # The confidential output may carry this component.  A pure
                # N->1 consolidation cannot be proven until its asset is known.
                ambiguous = True
                break
            if output.get("role") == "fee":
                continue
            if output.get("value_sats") is None:
                ambiguous = True
                break
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
            *(
                _input_owner_ids(index, entry, physical_scope=physical_scope)
                for entry in component_inputs
            )
        )
        if not sender_wallets <= input_owner_ids:
            continue  # every claimed sender must actually fund at least one input
        if not _inputs_owned_by(
            component_inputs,
            index,
            sender_wallets,
            physical_scope=physical_scope,
        ):
            continue  # a foreign input makes the recorded amounts unreliable

        n = len(senders)
        sum_amounts = sum(int(_get(row, "amount") or 0) for row in senders)
        if sum_amounts + (n - 1) * fee != out_c_msat:
            continue  # conservation broken (sync gap / stale graph) — decline

        # Destination-receipt reconciliation. The legs credit ``out_C`` to the
        # destination, so any *existing* recorded receipt of these same coins
        # must be removed to avoid double-counting. That is only safe when the
        # receipt sits in this spend's own canonical transaction group — then it
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
        # Compare to the parsed graph txid as a defense in depth; provider row
        # ids never define this automatic group. Skip receipts handled elsewhere
        # (`already_paired_ids`) — an unrelated, separately-paired same-amount
        # deposit must not false-decline this consolidation.
        graph_txid = canonical_txid(parsed.get("txid")) or txid_key
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
        txid = str(canonical_txid(parsed.get("txid")) or txid_key)
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
    legs) of every losing physical transaction. Provider/import ids without a
    canonical chain identity never participate.

    This is the self-transfer-scoped slice of the broader RBF/reorg canonicalization
    pass; it is purely a quarantine signal and never books anything.
    """
    row_txid: dict[str, tuple[str, str, str]] = {}
    txid_confirmed: dict[tuple[str, str, str], bool] = {}
    outpoint_txids: dict[
        tuple[str, str, str], set[tuple[str, str, str]]
    ] = {}
    for row in rows:
        parsed = _parse_onchain_tx(_get(row, "raw_json"), allow_partial=True)
        # A synthetic split / direct-payout leg keeps the REAL transaction in
        # raw_json but renames external_id (e.g. "cross-split:..."), so prefer the
        # parsed graph txid and fall back to external_id. Keying every leg this way
        # ensures a losing transaction's synthetic legs are quarantined too — not
        # just the rows whose external_id literally equals the txid.
        scope = _physical_conflict_scope(row, parsed)
        if scope is None:
            continue
        physical = scope[:3]
        row_txid[str(_get(row, "id"))] = physical
        # Confirmation can land on ANY leg of a transaction — when wallets sync at
        # different times a destination inbound may be confirmed while the source's
        # outbound row is still unconfirmed — so fold every row's state into the
        # per-txid confirmation, not just outbound rows.
        txid_confirmed[physical] = txid_confirmed.get(physical, False) or bool(
            _get(row, "confirmed_at")
        )
        # Collect input outpoints from ANY leg carrying the graph, not just
        # outbound rows: a conflict whose loser was synced only as a
        # destination INBOUND still has the full vin in its raw_json. Chain and
        # network remain part of the key because identical transaction bytes can
        # exist on two Bitcoin networks.
        for entry in (parsed or {}).get("inputs", ()):
            outpoint = _canonical_outpoint(entry.get("outpoint"))
            if outpoint:
                outpoint_txids.setdefault(
                    (physical[0], physical[1], outpoint), set()
                ).add(physical)

    loser_txids: set[tuple[str, str, str]] = set()
    for txids in outpoint_txids.values():
        if len(txids) < 2:
            continue  # one transaction owns this outpoint — no conflict
        confirmed = {txid for txid in txids if txid_confirmed.get(txid)}
        if len(confirmed) == 1:
            loser_txids |= txids - confirmed  # the unconfirmed replacements lose
        else:
            loser_txids |= txids  # ambiguous — surface the whole conflict
    return {rid for rid, txid in row_txid.items() if txid in loser_txids}


def detect_pending_onchain_ids(rows: Sequence[Mapping[str, Any]]) -> set[str]:
    """Rows belonging to a transaction explicitly reported as unconfirmed.

    ``confirmed_at IS NULL`` is not sufficient: CSV/provider imports commonly
    have no confirmation timestamp. Chain sync payloads, however, persist an
    explicit ``status.confirmed`` boolean. Hold only those explicit mempool
    transactions out of tax booking until any synced leg proves confirmation.
    """

    row_txid: dict[str, tuple[str, str, str]] = {}
    txid_state: dict[tuple[str, str, str], tuple[bool, bool]] = {}
    for row in rows:
        payload = stored_tx_mapping(_get(row, "raw_json"), allow_nested=True)
        if payload is None:
            continue
        status = payload.get("status")
        if not isinstance(status, Mapping) or not isinstance(
            status.get("confirmed"), bool
        ):
            continue
        parsed = _parse_onchain_tx(_get(row, "raw_json"), allow_partial=True)
        scope = _physical_conflict_scope(row, parsed)
        if scope is None:
            continue
        physical = scope[:3]
        row_txid[str(_get(row, "id"))] = physical
        _explicit, confirmed = txid_state.get(physical, (False, False))
        txid_state[physical] = (True, confirmed or bool(status["confirmed"]))
    pending_txids = {
        txid
        for txid, (explicit, confirmed) in txid_state.items()
        if explicit and not confirmed
    }
    # Include graphless sibling legs sharing the same txid once any leg supplied
    # the explicit mempool state.
    for row in rows:
        parsed = _parse_onchain_tx(_get(row, "raw_json"), allow_partial=True)
        scope = _physical_conflict_scope(row, parsed)
        if scope is not None and scope[:3] in pending_txids:
            row_txid[str(_get(row, "id"))] = scope[:3]
    return {row_id for row_id, txid in row_txid.items() if txid in pending_txids}


# -- internals --------------------------------------------------------------


def _physical_onchain_scope(
    row: Mapping[str, Any],
) -> tuple[str, str, str, str] | None:
    """Canonical physical scope, including synthetic rows' retained graph.

    Synthetic journal rows correctly opt out of automatic transfer grouping:
    they are allocations, not new observations. Conflict and pending safety is
    different; every synthetic leg retaining a real anchor graph must be held
    with that physical transaction. Re-evaluate only identity with a neutral id.
    """

    scope = onchain_transfer_scope(row)
    if scope is not None:
        return scope
    if not str(_get(row, "id") or "").startswith(_SYNTHETIC_ID_PREFIXES):
        return None
    probe = dict(row)
    probe["id"] = f"physical-anchor:{_get(row, 'id')}"
    return onchain_transfer_scope(probe)


def _physical_conflict_scope(
    row: Mapping[str, Any], parsed: Mapping[str, Any] | None
) -> tuple[str, str, str, str] | None:
    """Physical identity usable only to quarantine transaction conflicts.

    Full transfer scope additionally requires a trustworthy asset component.
    That is essential before carrying basis, but unnecessarily strict for an
    RBF/double-spend safety check: two transactions spending one prevout
    conflict across the whole transaction.  Permit that narrower check only
    when the graph txid and the Bitcoin-family rail/network remain unambiguous.
    The returned asset slot is a sentinel and must never drive matching.
    """

    scope = _physical_onchain_scope(row)
    if scope is not None:
        return scope
    if parsed is None:
        return None

    raw_txid = str(parsed.get("txid") or "").strip()
    parsed_txid = canonical_txid(raw_txid)
    if raw_txid and parsed_txid is None:
        return None
    external_txid = canonical_txid(_get(row, "external_id"))
    if parsed_txid and external_txid and parsed_txid != external_txid:
        return None
    txid = parsed_txid or external_txid
    if txid is None:
        return None

    asset = str(_get(row, "asset") or "").strip().upper().replace("L-BTC", "LBTC")
    if asset == "BTC":
        chain = "bitcoin"
    elif asset == "LBTC":
        chain = "liquid"
    else:
        return None
    domain, valid = bitcoin_network_domain_evidence(row)
    if not valid:
        return None
    if domain is None:
        if chain != "bitcoin":
            return None
        domain = "main"
    return chain, domain, txid, "conflict-only"


def _canonical_outpoint(value: Any) -> str | None:
    """Return ``<canonical txid>:<nonnegative vout>`` or ``None``."""

    txid_text, separator, vout_text = str(value or "").strip().rpartition(":")
    txid = canonical_txid(txid_text)
    if not separator or txid is None:
        return None
    try:
        vout = int(vout_text)
    except (TypeError, ValueError):
        return None
    return f"{txid}:{vout}" if vout >= 0 else None


def _parsed_chain_network_for_row(
    parsed: Mapping[str, Any], index: Any, row: Mapping[str, Any]
) -> Optional[tuple[str, str]]:
    """Resolve an evidence scope without defaulting an unknown Liquid network."""

    wallet_id = str(_get(row, "wallet_id") or "")
    inferred = _source_chain_network(parsed.get("inputs") or (), index, wallet_id)
    if inferred is None:
        # An inbound-only observation may not know any prevout, but its owned
        # destination script still stamps the correct chain/network.
        output_scopes: set[tuple[str, str]] = set()
        for output in parsed.get("outputs") or ():
            for match in index.lookup_script(output.get("script")):
                if str(match.wallet_id) == wallet_id:
                    output_scopes.add(
                        _norm_chain_network(match.chain, match.network)
                    )
        if len(output_scopes) == 1:
            inferred = next(iter(output_scopes))

    config = stored_tx_mapping(
        _get(row, "config_json") or _get(row, "wallet_config_json")
    ) or {}
    raw_chains = [parsed.get("chain"), config.get("chain")]
    explicit_chains: set[str] = set()
    for value in raw_chains:
        if not str(value or "").strip():
            continue
        try:
            explicit_chains.add(normalize_chain(value))
        except ValueError:
            return None
    if len(explicit_chains) > 1:
        return None
    explicit_chain = next(iter(explicit_chains), None)

    network_chain = explicit_chain or (inferred[0] if inferred is not None else None)
    if network_chain is None:
        asset = str(_get(row, "asset") or "").strip().upper()
        network_chain = "liquid" if asset in {"LBTC", "L-BTC"} else "bitcoin"
    raw_networks = [parsed.get("network"), config.get("network")]
    explicit_networks: set[str] = set()
    for value in raw_networks:
        if not str(value or "").strip():
            continue
        try:
            explicit_networks.add(normalize_network(network_chain, value))
        except ValueError:
            return None
    if len(explicit_networks) > 1:
        return None
    explicit_network = next(iter(explicit_networks), None)

    if inferred is not None:
        if explicit_chain is not None and explicit_chain != inferred[0]:
            return None
        if explicit_network is not None and explicit_network != inferred[1]:
            return None
        return inferred
    if explicit_chain is not None and explicit_network is not None:
        return explicit_chain, explicit_network
    # Bitcoin's historical blank metadata means mainnet.  Liquid blanks are
    # not defaulted here: an elementsregtest observation must never merge with
    # liquidv1 solely because the txid happened to match.
    if explicit_chain == "bitcoin":
        return _norm_chain_network(explicit_chain, explicit_network)
    return None


def _profile_parsed_transactions(
    rows: Sequence[Mapping[str, Any]], index: Any
) -> dict[str, dict[str, Any]]:
    """Merge per-wallet observations inside strict chain/network/txid scopes."""

    parsed_rows: dict[str, dict[str, Any]] = {}
    keys: dict[str, tuple[str, str, str]] = {}
    groups: dict[tuple[str, str, str], list[dict[str, Any]]] = {}
    for row in rows:
        row_id = str(_get(row, "id"))
        parsed = _parse_onchain_tx(_get(row, "raw_json"), allow_partial=True)
        if parsed is None:
            continue
        scope = _ownership_onchain_scope(row, index=index, parsed=parsed)
        if scope is None:
            continue
        parsed_rows[row_id] = parsed
        key = (scope[0], scope[1], scope[2])
        parsed["chain"], parsed["network"] = key[0], key[1]
        keys[row_id] = key
        groups.setdefault(key, []).append(parsed)
    result: dict[str, dict[str, Any]] = {}
    for row_id, parsed in parsed_rows.items():
        key = keys.get(row_id)
        if key is None:
            result[row_id] = parsed
            continue
        # Put the row's own observation first so its per-asset ``component``
        # metadata remains authoritative while legs are filled from siblings.
        merged = merge_ownership_txs([parsed, *groups[key]]) or parsed
        merged["component"] = dict(parsed.get("component") or {})
        merged["chain"], merged["network"] = key[0], key[1]
        result[row_id] = merged
    return result


def _ownership_onchain_scope(
    row: Mapping[str, Any],
    *,
    index: Any,
    parsed: Mapping[str, Any] | None = None,
) -> tuple[str, str, str, str] | None:
    """Canonical row scope, allowing the owned index to identify its network."""

    parsed = parsed or _parse_onchain_tx(_get(row, "raw_json"), allow_partial=True)
    canonical_scope = onchain_transfer_scope(row)
    if canonical_scope is not None:
        return canonical_scope
    if parsed is not None and index is not None:
        raw_parsed_txid = str(parsed.get("txid") or "").strip()
        parsed_txid = canonical_txid(raw_parsed_txid)
        if raw_parsed_txid and parsed_txid is None:
            return None
        external_txid = canonical_txid(_get(row, "external_id"))
        if parsed_txid and external_txid and parsed_txid != external_txid:
            return None
        txid = parsed_txid or external_txid
        chain_network = _parsed_chain_network_for_row(parsed, index, row)
        if txid is not None and chain_network is not None:
            asset_id, display_asset = _component_asset(parsed, row)
            normalized_display_asset = display_asset.replace("L-BTC", "LBTC")
            if (
                normalized_display_asset == "BTC"
                and chain_network[0] != "bitcoin"
            ) or (
                normalized_display_asset == "LBTC"
                and chain_network[0] != "liquid"
            ):
                return None
            if chain_network[0] == "liquid":
                if canonical_txid(asset_id) is None:
                    return None
                asset_identity = str(asset_id).lower()
            else:
                asset_identity = normalized_display_asset
            return (chain_network[0], chain_network[1], txid, asset_identity)
    return None


def _component_asset(parsed: Mapping[str, Any], row: Mapping[str, Any]) -> tuple[str | None, str]:
    component = parsed.get("component")
    component = component if isinstance(component, Mapping) else {}
    asset_id = str(component.get("asset_id") or "").strip().lower() or None
    asset = str(component.get("asset") or _get(row, "asset") or "").strip().upper()
    return asset_id, asset


def _leg_asset_relation(
    leg: Mapping[str, Any], parsed: Mapping[str, Any], row: Mapping[str, Any]
) -> str:
    """Return ``same``, ``different``, or ``unknown`` for one component leg."""

    if str(parsed.get("chain") or "").lower() != "liquid":
        return "same"
    component_id, component_asset = _component_asset(parsed, row)
    leg_id = str(leg.get("asset_id") or "").strip().lower() or None
    leg_asset = str(leg.get("asset") or "").strip().upper()
    if component_id is not None and leg_id is not None:
        return "same" if component_id == leg_id else "different"
    if component_id is not None:
        return "unknown"
    if component_asset and leg_asset:
        return "same" if component_asset == leg_asset else "different"
    return "unknown"


def _component_inputs(
    parsed: Mapping[str, Any], row: Mapping[str, Any]
) -> tuple[list[Mapping[str, Any]], bool]:
    inputs = list(parsed.get("inputs") or ())
    if str(parsed.get("chain") or "").lower() != "liquid":
        return inputs, True
    selected: list[Mapping[str, Any]] = []
    complete = True
    for entry in inputs:
        relation = _leg_asset_relation(entry, parsed, row)
        if relation == "same":
            selected.append(entry)
        elif relation == "unknown":
            # An unidentified confidential input might carry this component;
            # excluding it would make per-asset conservation fictitious.
            complete = False
    return selected, complete and bool(selected)


def _liquid_source_conserves(
    parsed: Mapping[str, Any],
    row: Mapping[str, Any],
    index: Any,
    source_wallet_id: str,
    component_inputs: Sequence[Mapping[str, Any]],
    source_total_msat: int,
    physical_scope: tuple[str, str],
) -> tuple[bool, dict[str, Any]]:
    """Prove the wallet row's net outflow from historical valued Liquid legs."""

    if str(parsed.get("chain") or "").lower() != "liquid":
        return True, {}
    if any(entry.get("value_sats") is None for entry in component_inputs):
        return False, {"missing": "input_value_or_asset"}
    input_sats = sum(int(entry["value_sats"]) for entry in component_inputs)
    change_sats = 0
    for output in parsed.get("outputs") or ():
        owner_ids = {
            str(match.wallet_id)
            for match in _matches_in_physical_scope(
                index.lookup_script(output.get("script")), physical_scope
            )
        }
        if source_wallet_id not in owner_ids:
            continue
        relation = _leg_asset_relation(output, parsed, row)
        if relation == "different":
            continue
        if relation == "unknown" or output.get("value_sats") is None:
            return False, {"missing": "owned_change_value_or_asset"}
        change_sats += int(output["value_sats"])
    expected_msat = (input_sats - change_sats) * SATS_TO_MSAT
    return expected_msat == source_total_msat, {
        "input_sats": input_sats,
        "change_sats": change_sats,
        "expected_outflow_msat": expected_msat,
        "row_total_outflow_msat": source_total_msat,
    }


def _raw_mapping(raw_json: Any) -> Mapping[str, Any]:
    return stored_tx_mapping(raw_json) or {}


def _row_is_liquid(row: Mapping[str, Any]) -> bool:
    raw = _raw_mapping(_get(row, "raw_json"))
    component = raw.get("component")
    return (
        str(raw.get("chain") or "").strip().lower() == "liquid"
        or (isinstance(component, Mapping) and bool(component.get("asset_id")))
        or str(_get(row, "asset") or "").strip().upper() in {"LBTC", "L-BTC"}
    )


def _matches_in_physical_scope(
    matches: Sequence[Any], physical_scope: tuple[str, str]
) -> list[Any]:
    """Only ownership evidence from one canonical chain/network domain."""

    return [
        match
        for match in matches
        if _norm_chain_network(match.chain, match.network) == physical_scope
    ]


def _inputs_owned_by(
    inputs: Sequence[Mapping[str, Any]],
    index: Any,
    owner_set: set[str],
    *,
    physical_scope: tuple[str, str],
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
        owners = _input_owner_ids(index, entry, physical_scope=physical_scope)
        if not owners or not owners <= owner_set:
            return False
    return True


def _inputs_are_single_source(
    inputs: Sequence[Mapping[str, Any]],
    index: Any,
    source_wallet_id: str,
    *,
    physical_scope: tuple[str, str],
) -> bool:
    """True only when the source wallet owns every input.

    A foreign/unresolvable input (payjoin/coinjoin, or coins we do not watch)
    or an input from a *different* owned wallet (a multi-wallet consolidation)
    makes the recorded amount/fee unreliable for splitting. An input is
    acceptable only when its complete owner set is exactly the source wallet.
    A shared descriptor / reused address is ambiguous ownership evidence and
    must not let index insertion order assign basis or fees to one co-owner.
    """
    if not inputs:
        return False
    for entry in inputs:
        owners = _input_owner_ids(index, entry, physical_scope=physical_scope)
        if owners != {source_wallet_id}:
            return False
    return True


def _inputs_are_single_source_or_recorded_source(
    inputs: Sequence[Mapping[str, Any]],
    index: Any,
    source_wallet_id: str,
    row: Mapping[str, Any],
    *,
    physical_scope: tuple[str, str],
) -> bool:
    """Accept a single-input outbound row when historical input ownership is absent.

    ``record_from_bitcoin_esplora_tx`` can only create a positive outbound row
    for a wallet when tracked source value left that wallet. If the spend has
    exactly one input, the source wallet necessarily funded that input even when
    the ownership index cannot resolve the old spent outpoint (for example,
    because the wallet was first inventoried after that output was already
    spent). Keep multi-input spends on the strict index-only path.
    """
    if _inputs_are_single_source(
        inputs,
        index,
        source_wallet_id,
        physical_scope=physical_scope,
    ):
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
    # The recorded-row fallback is only for genuinely absent historical
    # ownership. Any resolved owner set that failed the strict check above is
    # conflicting or co-owned evidence and must remain manual.
    if _input_owner_ids(index, inputs[0], physical_scope=physical_scope):
        return False
    prev_txid = str(outpoint).split(":", 1)[0].lower()
    recorded_owners = {
        str(wallet_id)
        for wallet_id, _wallet_label in _lookup_txid_wallets(
            index, prev_txid, physical_scope=physical_scope
        )
    }
    return recorded_owners == {source_wallet_id}


def _input_owner_ids(
    index: Any,
    entry: Mapping[str, Any],
    *,
    physical_scope: tuple[str, str],
) -> set[str]:
    """All owned-wallet ids for an input (outpoint inventory wins; else script).

    The outpoint inventory is unambiguous (one wallet per UTXO); only the
    script fallback can map to several wallets, and we return the full set so
    callers can reason about ambiguity instead of an arbitrary first match.
    """
    outpoint = entry.get("outpoint")
    if outpoint:
        matches = _lookup_outpoint(
            index, outpoint, physical_scope=physical_scope
        )
        if matches:
            return {str(match.wallet_id) for match in matches}
    return {
        str(match.wallet_id)
        for match in _matches_in_physical_scope(
            index.lookup_script(entry.get("script")), physical_scope
        )
    }


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

    This is used only while recovering a scope for legacy graph rows that lack
    explicit network metadata.  Multiple source scopes are ambiguous and return
    ``None``; accounting paths with a canonical row group use that group directly.
    """
    scopes: set[tuple[str, str]] = set()
    for entry in inputs:
        matches: list[Any] = []
        outpoint = entry.get("outpoint")
        if outpoint:
            matches = _lookup_outpoint(index, outpoint)
        if not matches:
            matches = list(index.lookup_script(entry.get("script")))
        for match in matches:
            if str(match.wallet_id) == source_wallet_id:
                scopes.add(_norm_chain_network(match.chain, match.network))
    return next(iter(scopes)) if len(scopes) == 1 else None


def _source_input_scope_contradicts(
    inputs: Sequence[Mapping[str, Any]],
    index: Any,
    source_wallet_id: str,
    physical_scope: tuple[str, str],
) -> bool:
    """Whether present source evidence points only outside ``physical_scope``.

    Missing historical ownership remains eligible for the deliberately narrow
    one-input recorded-source fallback.  Known ownership on another network is
    different: it contradicts the canonical transaction group and must fail
    closed instead of being treated as merely missing.
    """

    for entry in inputs:
        matches: list[Any] = []
        outpoint = entry.get("outpoint")
        if outpoint:
            matches = _lookup_outpoint(index, outpoint)
        if not matches:
            matches = list(index.lookup_script(entry.get("script")))
        source_matches = [
            match
            for match in matches
            if str(match.wallet_id) == source_wallet_id
        ]
        if source_matches and not _matches_in_physical_scope(
            source_matches, physical_scope
        ):
            return True

    if len(inputs) != 1 or not inputs[0].get("outpoint"):
        return False
    prev_txid = str(inputs[0]["outpoint"]).split(":", 1)[0].lower()
    unscoped = _lookup_txid_wallets(index, prev_txid)
    if not any(str(wallet_id) == source_wallet_id for wallet_id, _ in unscoped):
        return False
    scoped = _lookup_txid_wallets(
        index, prev_txid, physical_scope=physical_scope
    )
    return not any(str(wallet_id) == source_wallet_id for wallet_id, _ in scoped)


def _lookup_outpoint(
    index: Any,
    outpoint: Any,
    *,
    physical_scope: tuple[str, str] | None = None,
) -> list[Any]:
    if hasattr(index, "lookup_outpoint"):
        if physical_scope is None:
            return list(index.lookup_outpoint(outpoint))
        try:
            return list(
                index.lookup_outpoint(
                    outpoint,
                    chain=physical_scope[0],
                    network=physical_scope[1],
                )
            )
        except TypeError:
            return _matches_in_physical_scope(
                list(index.lookup_outpoint(outpoint)), physical_scope
            )
    value = getattr(index, "by_outpoint", {}).get(str(outpoint or "").lower())
    if value is None:
        return []
    matches = value if isinstance(value, list) else [value]
    if physical_scope is not None:
        return _matches_in_physical_scope(matches, physical_scope)
    return matches


def _lookup_txid_wallets(
    index: Any,
    txid: Any,
    *,
    physical_scope: tuple[str, str] | None = None,
) -> set[tuple[str, str]]:
    if hasattr(index, "lookup_txid_wallets"):
        if physical_scope is None:
            return set(index.lookup_txid_wallets(txid))
        try:
            return set(
                index.lookup_txid_wallets(
                    txid,
                    chain=physical_scope[0],
                    network=physical_scope[1],
                )
            )
        except TypeError:
            # An older/fake index cannot prove the physical network of its txid
            # claim.  Never promote that untyped claim into accounting evidence.
            return set()
    if physical_scope is not None:
        return set()
    return set(
        getattr(index, "txid_wallets", {}).get(str(txid or "").lower(), set())
    )


def _parse_onchain_tx(
    raw_json: Any, *, allow_partial: bool = False
) -> Optional[dict[str, Any]]:
    """Parse stored Bitcoin/Liquid graph evidence.

    The default preserves the historical strict contract used by Bitcoin
    callers: every output must be valued.  Ownership reconstruction opts into
    ``allow_partial`` because a foreign confidential Liquid output can remain
    unknown while locally owned legs are filled from another wallet's stored
    observation.  Policy code must explicitly block an owned unknown leg.
    """

    if not allow_partial:
        return parse_valued_tx(raw_json)
    return parse_ownership_tx(raw_json)


def _resolve_destination_inbound(
    candidates: Sequence[Mapping[str, Any]],
    leg_msat: int,
    txid: str,
    consumed_in_ids: set[str],
    already_paired_ids: set[str],
    *,
    asset: Any,
    onchain_scope: tuple[str, str, str, str],
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

    1. One exact-value row in the same canonical chain/network/txid/asset scope
       is unambiguously this leg → reuse.
    2. Two or more such rows → ambiguous → decline.
    3. No exact reuse, but the destination has another amount-compatible inbound
       that is not provably a different on-chain transaction → decline rather
       than fabricate a duplicate.
    4. Otherwise the destination is genuinely empty for this leg → synthesize.
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
    same_txid = [
        row
        for row in exact
        if onchain_transfer_scope(row) == onchain_scope
    ]
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
    detail: Mapping[str, Any] | None = None,
    *,
    required_for: str = "ownership_transfer_review",
) -> None:
    payload = {
        "required_for": required_for,
        "wallet": _get(row, "wallet_label") or _get(row, "wallet_id"),
        "asset": _get(row, "asset"),
        "external_id": _get(row, "external_id"),
    }
    payload.update(detail or {})
    result.blocked_sources.append(
        {"row": row, "reason": reason, "detail": payload}
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
    if type(row) is dict:
        return row.get(key, default)
    getter = getattr(row, "get", None)
    if getter is not None:
        return getter(key, default)
    try:
        return row[key]
    except (KeyError, IndexError, TypeError):
        return getattr(row, key, default)
