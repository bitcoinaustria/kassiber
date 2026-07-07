"""Self-transfer detection for cross-wallet on-chain hops.

When the same on-chain transaction appears as outbound in one kassiber wallet
and inbound in another wallet of the same profile, it is a self-transfer:
the user moved their own coins between their own wallets. RP2 models this as
an `IntraTransaction` (MOVE), where the network fee is the only taxable
portion and the lots themselves carry their original cost basis across to
the destination wallet.

This module is the pure detection layer. Conversion of detected pairs into
RP2 `IntraTransaction` instances happens in the journal pipeline.

Detection rule (intentionally conservative):

    Two rows share `(external_id, asset)`, sit in different wallets,
    and form exactly one outbound + one inbound. Multi-output transactions
    that fan out to several owned wallets are skipped — they would need
    explicit user disambiguation.
"""

from collections import defaultdict

_HEX_DIGITS = frozenset("0123456789abcdefABCDEF")
_BITCOIN_CARRY_ASSETS = frozenset({"BTC", "LBTC"})


def is_bitcoin_rail_pair(out_asset, in_asset):
    """True for BTC/LBTC rail changes of the same Bitcoin exposure."""

    assets = {str(out_asset or "").strip().upper(), str(in_asset or "").strip().upper()}
    return assets == _BITCOIN_CARRY_ASSETS


def cross_asset_carrying_value_supported(tax_country, out_asset, in_asset):
    """Whether a cross-asset carrying-value pair is supported for this profile."""

    if str(tax_country or "").strip().lower() == "at":
        return True
    return is_bitcoin_rail_pair(out_asset, in_asset)


def profile_bitcoin_rail_carrying_value(profile):
    """Profile default for treating Bitcoin rail changes as carrying value."""

    try:
        return bool(profile["bitcoin_rail_carrying_value"])
    except (KeyError, IndexError, TypeError):
        return True


def normalize_group_txid(external_id):
    """Fold a 64-hex txid to lowercase for grouping; leave other ids verbatim.

    Bitcoin txids are case-insensitive hex, but ``external_id`` is stored
    verbatim, so two wallets that recorded the same self-transfer with different
    casing (e.g. one esplora-synced, one imported from an uppercase CSV) would
    otherwise land in different ``(external_id, asset)`` groups and never pair.
    Only fold real 64-char hex ids so Lightning ``payment_hash`` values and
    synthetic CSV ids are untouched. ``transfer_matching._deterministic_self_transfer_ids``
    uses the same normalization — keep both in lockstep.
    """
    text = str(external_id)
    if len(text) == 64 and all(char in _HEX_DIGITS for char in text):
        return text.lower()
    return text


def apply_manual_pairs(rows, auto_pairs, manual_pair_records):
    """Merge manual pair records with auto-detected pairs.

    Manual pairs (created via ``kassiber transfers pair``) take precedence
    over auto-detection: any auto-pair that touches a manually-paired
    transaction is dropped, and the manual pair takes its place.

    Same-asset manual pairs with ``policy=carrying-value`` feed back into
    the IntraTransaction pipeline (same shape as auto pairs). Cross-asset
    pairs are returned separately so the journal pipeline can record them
    as audit metadata without handing them to RP2. Any existing manual
    pair with a different policy still suppresses auto-detection for those
    rows, but the legs are left on the normal SELL + BUY path.

    Args:
        rows: full row list for the profile (sqlite3.Row-like).
        auto_pairs: output of ``detect_intra_transfers(rows)``.
        manual_pair_records: iterable of dicts with at least ``out_transaction_id``,
            ``in_transaction_id``, ``policy``, ``kind``.

    Returns:
        merged_pairs: list of ``{"out": out_row, "in": in_row}`` for
            same-asset pairs (manual + surviving auto), suitable for the
            existing intra path.
        cross_asset_pairs: list of dicts describing cross-asset manual
            pairs for audit purposes only.
    """
    rows_by_id = {row["id"]: row for row in rows}
    manual_same_asset = []
    cross_asset_pairs = []
    manually_paired_ids = set()
    for record in manual_pair_records:
        out_id = record["out_transaction_id"]
        in_id = record["in_transaction_id"]
        out_row = rows_by_id.get(out_id)
        in_row = rows_by_id.get(in_id)
        if out_row is None or in_row is None:
            continue
        manually_paired_ids.add(out_id)
        manually_paired_ids.add(in_id)
        if out_row["asset"] == in_row["asset"] and record["policy"] == "carrying-value":
            manual_same_asset.append(
                {
                    "out": out_row,
                    "in": in_row,
                    "pair_id": record["id"],
                    "kind": record["kind"],
                    "policy": record["policy"],
                    "source": _row_field(record, "pair_source") or "manual",
                }
            )
        elif out_row["asset"] != in_row["asset"]:
            cross_asset_pairs.append(
                {
                    "pair_id": record["id"],
                    "kind": record["kind"],
                    "policy": record["policy"],
                    "out_id": out_id,
                    "in_id": in_id,
                    "out_asset": out_row["asset"],
                    "in_asset": in_row["asset"],
                }
            )
    surviving_auto = [
        pair
        for pair in auto_pairs
        if pair["out"]["id"] not in manually_paired_ids
        and pair["in"]["id"] not in manually_paired_ids
    ]
    return manual_same_asset + surviving_auto, cross_asset_pairs


def _row_field(row, key):
    """Read ``key`` from a sqlite3.Row-like or dict row, ``None`` if absent."""
    try:
        keys = row.keys()
    except AttributeError:
        return row.get(key)
    return row[key] if key in keys else None


_LIGHTNING_PAYMENT_HASH_SOURCES = frozenset({"core_lightning", "lnd"})
_NON_LIGHTNING_PAYMENT_HASH_SOURCES = frozenset({"chain_script"})
_LIGHTNING_WALLET_KINDS = frozenset(
    {"cln", "core-ln", "coreln", "lnd", "lightning", "nwc", "phoenix"}
)
_LIGHTNING_TRANSACTION_KINDS = frozenset(
    {
        "cln_invoice",
        "cln_pay",
        "lightning_received",
        "lightning_sent",
        "ln_invoice",
        "ln_pay",
        "lnd_invoice",
        "lnd_pay",
    }
)


def _normalized_lower(value):
    return str(value or "").strip().lower()


def _is_lightning_payment_hash_row(row):
    source = _normalized_lower(_row_field(row, "payment_hash_source"))
    if source in _NON_LIGHTNING_PAYMENT_HASH_SOURCES:
        return False
    if source in _LIGHTNING_PAYMENT_HASH_SOURCES:
        return True
    kind = _normalized_lower(_row_field(row, "kind"))
    if kind in _LIGHTNING_TRANSACTION_KINDS:
        return True
    wallet_kind = _normalized_lower(_row_field(row, "wallet_kind"))
    return wallet_kind in _LIGHTNING_WALLET_KINDS


def detect_intra_transfers(rows):
    """Return ``(pairs, matched_ids)`` for the given transaction rows.

    Args:
        rows: iterable of sqlite3.Row-like records that expose
            ``id``, ``external_id``, ``asset``, ``direction``, ``amount``,
            ``wallet_id`` (and, for Lightning, ``payment_hash``).

    Returns:
        pairs: list of ``{"out": out_row, "in": in_row}`` dicts.
        matched_ids: set of transaction ids covered by any pair.
    """
    rows = list(rows)
    by_key = defaultdict(list)
    for row in rows:
        external_id = row["external_id"] if "external_id" in row.keys() else None
        if not external_id:
            continue
        by_key[(normalize_group_txid(external_id), row["asset"])].append(row)

    pairs = []
    matched_ids = set()
    for group in by_key.values():
        outs = [
            r
            for r in group
            if r["direction"] == "outbound" and (r["amount"] or 0) > 0
        ]
        # A non-positive inbound (0-value/placeholder import row sharing the
        # txid) is never a real receiving leg; counting it would push a clean
        # 1-out/1-in self-transfer into the >1-inbound "skip" branch and, via
        # _owned_fanout_row_ids, into a spurious owned_fanout_unresolved
        # quarantine. Filter it out symmetrically with the outbound filter.
        ins = [
            r
            for r in group
            if r["direction"] == "inbound" and (r["amount"] or 0) > 0
        ]
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_row, in_row = outs[0], ins[0]
        if out_row["wallet_id"] == in_row["wallet_id"]:
            continue
        pairs.append({"out": out_row, "in": in_row})
        matched_ids.add(out_row["id"])
        matched_ids.add(in_row["id"])

    # Lightning self-transfers pair by ``payment_hash``, not by txid: a payment
    # from one owned node to an invoice on another owned node shares the payment
    # hash but has distinct ``external_id`` values (``cln:pay:H`` vs
    # ``cln:income:H``, or the LND equivalents), so the txid grouping above never
    # sees them. The hash is a cryptographic commitment to the preimage, so a
    # match across two owned wallets is deterministic proof of a self-transfer —
    # the same conservative 1-out/1-in / different-wallet / same-asset rule
    # applies. External payments (only an outbound leg, no owned receiver) never
    # pair and stay real disposals.
    #
    # On-chain HTLC claim/refund rows can also expose payment_hash via
    # chain_script enrichment. Those are swap evidence, not proof that two
    # same-asset owned rows are a plain MOVE, so they stay eligible for swap
    # review instead of being auto-suppressed here.
    by_hash = defaultdict(list)
    for row in rows:
        if _row_field(row, "id") in matched_ids:
            continue
        payment_hash = _row_field(row, "payment_hash")
        if not payment_hash:
            continue
        if not _is_lightning_payment_hash_row(row):
            continue
        by_hash[(str(payment_hash), row["asset"])].append(row)
    for group in by_hash.values():
        outs = [
            r
            for r in group
            if r["direction"] == "outbound" and (r["amount"] or 0) > 0
        ]
        ins = [
            r
            for r in group
            if r["direction"] == "inbound" and (r["amount"] or 0) > 0
        ]
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_row, in_row = outs[0], ins[0]
        # Same-wallet Lightning circular payments are still internal movements:
        # pairing by payment_hash prevents the outbound/inbound legs from becoming
        # a taxable disposal plus fresh acquisition. The txid path above stays
        # cross-wallet-only because same-wallet on-chain txid rows are less
        # semantically precise (change, provider artifacts, or manual repair rows).
        if out_row["id"] in matched_ids or in_row["id"] in matched_ids:
            continue
        pairs.append({"out": out_row, "in": in_row})
        matched_ids.add(out_row["id"])
        matched_ids.add(in_row["id"])
    return pairs, matched_ids
