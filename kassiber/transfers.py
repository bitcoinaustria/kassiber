"""Self-transfer detection for cross-wallet on-chain hops.

When the same on-chain transaction appears as outbound in one kassiber wallet
and inbound in another wallet of the same profile, it is a self-transfer:
the user moved their own coins between their own wallets. RP2 models this as
an `IntraTransaction` (MOVE), where the network fee is the only taxable
portion and the lots themselves carry their original cost basis across to
the destination wallet.

This module is the pure detection layer. Conversion of detected pairs into
RP2 `IntraTransaction` instances happens in the journal pipeline.

Detection is evidence-first and profile-wide. Exact transaction-graph and
owned-script evidence can prove same-wallet consolidations, cross-wallet
migrations, fan-out, fan-in, and N:M flows. Ordinary same-asset on-chain wallet
movements are never inferred from amount/time correlation; cross-rail and
provider heuristics remain review candidates. Accounting policy is applied
only after the custody graph has been established.
"""

import json
from collections import defaultdict
from collections.abc import Mapping

_HEX_DIGITS = frozenset("0123456789abcdefABCDEF")
_BITCOIN_CARRY_ASSETS = frozenset({"BTC", "LBTC"})
_EXTERNAL_TXID_FIELD_KEYS = frozenset(
    {
        "txid",
        "txhash",
        "transactionid",
        "transactionhash",
        "onchaintxid",
        "sendtxid",
        "lockuptxid",
        "receivetxid",
        "claimtxid",
        "refundtxid",
    }
)
_LIQUID_NETWORK_TO_BITCOIN_DOMAIN = {
    "liquidv1": "main",
    "liquidtestnet": "test",
    "elementsregtest": "regtest",
}

# Wallet-kind normalization used only for route inference. Payment-hash
# auto-pair eligibility remains the stricter source-aware predicate below.
LIGHTNING_INFERENCE_WALLET_KINDS = frozenset(
    {"cln", "coreln", "lightning", "lnd", "nwc", "phoenix"}
)
CHAIN_INFERENCE_WALLET_KINDS = frozenset(
    {"address", "descriptor", "samourai", "silent-payment", "wasabi", "xpub"}
)
_WALLET_KIND_ALIASES = {
    "core-ln": "coreln",
    "core-lightning": "coreln",
}
_SYNTHETIC_TRANSFER_ID_PREFIXES = (
    "custody:",
    "custody-tax:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
    "owned-derive:",
    "recorded-fanout:",
)


def normalize_wallet_kind_alias(value):
    """Return the canonical wallet kind used by transfer-route inference."""

    normalized = str(value or "").strip().lower().replace("_", "-")
    return _WALLET_KIND_ALIASES.get(normalized, normalized)


def canonical_txid(value):
    """Return a lowercase Bitcoin-family txid, or ``None`` for other ids."""

    text = str(value or "").strip()
    if len(text) != 64 or any(char not in _HEX_DIGITS for char in text):
        return None
    return text.lower()


def canonical_payment_hash(value):
    """Return a lowercase 32-byte Lightning payment hash, or ``None``.

    Payment hashes and Bitcoin-family txids share the same 64-hex wire shape,
    but they prove different things.  Keep the semantic entry point separate so
    callers cannot accidentally treat an arbitrary adapter/import identifier as
    a cryptographic payment commitment merely because two strings are equal.
    """

    return canonical_txid(value)


def _json_mapping(value):
    if isinstance(value, Mapping):
        return value
    if value in (None, "", "{}", b"{}"):
        return {}
    try:
        parsed = json.loads(str(value))
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return parsed if isinstance(parsed, Mapping) else {}


def _json_mapping_sequence(value):
    """Return mapping observations from a stored JSON list, if present.

    Bitcoin Core address-wallet sync stores its ``listtransactions`` receive
    observations as a JSON list. They carry explicit ``txid``/``vout`` fields
    but are not a full vin/vout graph. Keep this parser separate from
    :func:`_json_mapping` so configuration and provider payload callers do not
    accidentally reinterpret arbitrary arrays as objects.
    """

    if isinstance(value, list):
        parsed = value
    elif value in (None, "", "{}", b"{}"):
        return []
    else:
        try:
            parsed = json.loads(str(value))
        except (TypeError, ValueError, json.JSONDecodeError):
            return []
    if not isinstance(parsed, list):
        return []
    return [item for item in parsed if isinstance(item, Mapping)]


def _canonical_chain(value):
    text = str(value or "").strip().lower()
    return {
        "bitcoin": "bitcoin",
        "btc": "bitcoin",
        "onchain": "bitcoin",
        "liquid": "liquid",
        "lbtc": "liquid",
        "elements": "liquid",
    }.get(text)


def _canonical_network(chain, value):
    text = str(value or "").strip().lower()
    aliases = (
        {
            "bitcoin": "main",
            "main": "main",
            "mainnet": "main",
            "test": "test",
            "testnet": "test",
            "regtest": "regtest",
            "signet": "signet",
        }
        if chain == "bitcoin"
        else {
            "liquid": "liquidv1",
            "liquidv1": "liquidv1",
            "main": "liquidv1",
            "mainnet": "liquidv1",
            "liquidtestnet": "liquidtestnet",
            "test": "liquidtestnet",
            "testnet": "liquidtestnet",
            "elements": "elementsregtest",
            "elementsregtest": "elementsregtest",
            "regtest": "elementsregtest",
        }
    )
    return aliases.get(text)


def _normalized_identity_key(value):
    return "".join(char for char in str(value or "").lower() if char.isalnum())


def _external_id_is_typed_txid(
    row, raw, graph_mappings, external_txid
):
    """Whether a graphless ``external_id`` was actually labelled as a txid.

    A provider swap/order id can also happen to be 64 hexadecimal characters.
    Its wire shape does not turn it into physical L1 identity.  Import adapters
    retain their original field names in ``raw_json``, so accept a graphless id
    only when it came from an explicit transaction-hash field (or a future
    reserved identity-kind marker).  In particular, bare ``id``/``swap_id``
    fallbacks never qualify.
    """

    explicit_kind = str(
        _row_field(row, "external_id_kind")
        or raw.get("external_id_kind")
        or ""
    ).strip().lower()
    if explicit_kind:
        return explicit_kind in {"txid", "transaction_hash", "onchain_txid"}

    for payload in graph_mappings:
        for key, value in payload.items():
            if _normalized_identity_key(key) not in _EXTERNAL_TXID_FIELD_KEYS:
                continue
            if canonical_txid(value) == external_txid:
                return True
    return False


def _liquid_component_identity(graph_mappings):
    """Return one unambiguous ``(asset_id, display_asset)`` component."""

    asset_ids = set()
    display_assets = set()
    for payload in graph_mappings:
        candidates = [payload]
        component = payload.get("component")
        if isinstance(component, Mapping):
            candidates.append(component)
        for candidate in candidates:
            raw_asset_id = candidate.get("asset_id")
            if raw_asset_id not in (None, ""):
                asset_id = canonical_txid(raw_asset_id)
                if asset_id is None:
                    return None
                asset_ids.add(asset_id)
            raw_asset = candidate.get("asset")
            if raw_asset not in (None, ""):
                display_assets.add(
                    str(raw_asset).strip().upper().replace("L-BTC", "LBTC")
                )
    if len(asset_ids) != 1 or len(display_assets) > 1:
        return None
    return next(iter(asset_ids)), next(iter(display_assets), "")


def onchain_transfer_scope(row):
    """Return ``(chain, network, txid, asset)`` for automatic row grouping.

    ``external_id`` is a transport/import field, not necessarily an on-chain
    transaction id.  Automatic self-transfer matching therefore accepts only a
    canonical 32-byte txid and keeps it inside one normalized chain/network
    scope.  A stored transaction graph is authoritative, while wallet config is
    used to disambiguate network.  Contradictory identity metadata fails closed.

    Legacy Bitcoin rows with no network metadata retain Kassiber's established
    ``bitcoin/main`` interpretation.  Liquid never guesses a blank network:
    graphless legacy imports stay available to the manual custody-component
    resolver instead of being joined across Liquid mainnet/testnet/regtest.
    """

    row_id = str(_row_field(row, "id") or "")
    if row_id.startswith(_SYNTHETIC_TRANSFER_ID_PREFIXES):
        # Synthetic journal projections retain the real anchor graph for audit.
        # They are allocations, not fresh observations of the whole transaction,
        # and must never rejoin their residual/source row through that raw txid.
        return None

    raw_value = _row_field(row, "raw_json")
    raw = _json_mapping(raw_value)
    config = _json_mapping(
        _row_field(row, "config_json") or _row_field(row, "wallet_config_json")
    )
    graph_mappings = [raw, *_json_mapping_sequence(raw_value)]
    for key in ("tx", "ownership_graph"):
        nested = raw.get(key)
        if isinstance(nested, Mapping):
            graph_mappings.append(nested)

    raw_graph_txids = [payload.get("txid") for payload in graph_mappings]
    # A graph that explicitly names itself with a provider/import label is not
    # merely "missing" identity. Falling back to another row field would let
    # contradictory evidence drive owned-script decomposition under the wrong
    # physical transaction.
    if any(
        str(value or "").strip() and canonical_txid(value) is None
        for value in raw_graph_txids
    ):
        return None
    graph_txids = {
        txid
        for value in raw_graph_txids
        if (txid := canonical_txid(value)) is not None
    }
    if len(graph_txids) > 1:
        return None
    external_txid = canonical_txid(_row_field(row, "external_id"))
    graph_txid = next(iter(graph_txids), None)
    if graph_txid and external_txid and graph_txid != external_txid:
        return None
    if (
        graph_txid is None
        and external_txid is not None
        and not _external_id_is_typed_txid(
            row, raw, graph_mappings, external_txid
        )
    ):
        return None
    txid = graph_txid or external_txid
    if txid is None:
        return None

    has_graph = any(
        isinstance(payload.get("vin"), list)
        and isinstance(payload.get("vout"), list)
        for payload in graph_mappings
    )
    wallet_kind = normalize_wallet_kind_alias(_row_field(row, "wallet_kind"))
    if wallet_kind in LIGHTNING_INFERENCE_WALLET_KINDS and not has_graph:
        # A Lightning payment hash is also 32 bytes.  It is not an L1 txid merely
        # because an adapter stored it in external_id.
        return None

    asset = str(_row_field(row, "asset") or "").strip().upper().replace("L-BTC", "LBTC")
    raw_chain_values = [
        *(payload.get("chain") for payload in graph_mappings),
        config.get("chain"),
    ]
    if any(
        str(value or "").strip() and _canonical_chain(value) is None
        for value in raw_chain_values
    ):
        return None
    chain_values = {
        chain
        for value in raw_chain_values
        if (chain := _canonical_chain(value)) is not None
    }
    if len(chain_values) > 1:
        return None
    inferred_chain = "liquid" if asset == "LBTC" else ("bitcoin" if asset == "BTC" else None)
    chain = next(iter(chain_values), inferred_chain)
    if chain not in {"bitcoin", "liquid"}:
        return None
    if (asset == "BTC" and chain != "bitcoin") or (
        asset == "LBTC" and chain != "liquid"
    ):
        # A display symbol cannot override contradictory physical rail evidence.
        return None

    raw_network_values = [
        *(payload.get("network") for payload in graph_mappings),
        config.get("network"),
    ]
    explicit_networks = {
        network
        for value in raw_network_values
        if str(value or "").strip()
        and (network := _canonical_network(chain, value)) is not None
    }
    # A non-empty but unsupported network is not safe to silently default.
    if any(
        str(value or "").strip() and _canonical_network(chain, value) is None
        for value in raw_network_values
    ):
        return None
    if len(explicit_networks) > 1:
        return None
    network = next(iter(explicit_networks), None)
    if network is None:
        if chain != "bitcoin":
            return None
        network = "main"

    asset_identity = asset
    if chain == "liquid":
        component_identity = _liquid_component_identity(graph_mappings)
        if component_identity is None:
            # Liquid transactions are multi-asset. A display label such as LBTC
            # is not consensus identity and cannot safely group two rows.
            return None
        asset_id, component_asset = component_identity
        if component_asset and component_asset != asset:
            return None
        if asset not in {"LBTC", asset_id.upper()}:
            return None
        asset_identity = asset_id
    return (chain, network, txid, asset_identity)


def bitcoin_network_domain_evidence(row):
    """Return ``(domain, valid)`` for an L1/Liquid/Lightning row.

    ``valid=False`` distinguishes contradictory/unsupported explicit metadata
    from genuinely absent network evidence.  Write boundaries use that bit to
    reject a row whose raw observation says mainnet while its wallet says
    regtest; read-only matchers continue to treat both cases as non-exact.
    """

    scope = onchain_transfer_scope(row)
    if scope is not None:
        chain, network = scope[:2]
        if chain == "bitcoin":
            return network, True
        domain = _LIQUID_NETWORK_TO_BITCOIN_DOMAIN.get(network)
        return (domain, domain is not None)

    raw = _json_mapping(_row_field(row, "raw_json"))
    config = _json_mapping(
        _row_field(row, "config_json") or _row_field(row, "wallet_config_json")
    )
    chain_families = set()
    for payload in (raw, config):
        value = payload.get("chain")
        if value in (None, ""):
            continue
        canonical = _canonical_chain(value)
        if canonical is not None:
            chain_families.add(canonical)
            continue
        if normalize_wallet_kind_alias(value) in LIGHTNING_INFERENCE_WALLET_KINDS:
            chain_families.add("lightning")
            continue
        return None, False
    base_families = {
        "liquid" if family == "liquid" else "bitcoin"
        for family in chain_families
    }
    if len(base_families) > 1:
        return None, False
    asset = str(_row_field(row, "asset") or "").strip().upper().replace("L-BTC", "LBTC")
    if (asset == "BTC" and base_families == {"liquid"}) or (
        asset == "LBTC" and base_families == {"bitcoin"}
    ):
        return None, False
    network_values = []
    for payload in (raw, config):
        for key in ("network", "bitcoin_network", "chain_network"):
            value = payload.get(key)
            if value not in (None, ""):
                network_values.append(value)
    domains = set()
    for value in network_values:
        bitcoin_network = _canonical_network("bitcoin", value)
        liquid_network = _canonical_network("liquid", value)
        if bitcoin_network is not None:
            domains.add(bitcoin_network)
        elif liquid_network is not None:
            domain = _LIQUID_NETWORK_TO_BITCOIN_DOMAIN.get(liquid_network)
            if domain is not None:
                domains.add(domain)
        else:
            return None, False
    if len(domains) > 1:
        return None, False
    if domains:
        return next(iter(domains)), True
    return None, True


def bitcoin_network_domain(row):
    """Return the country-neutral Bitcoin exposure network, when reliable."""

    domain, valid = bitcoin_network_domain_evidence(row)
    return domain if valid else None


def is_bitcoin_rail_pair(out_asset, in_asset):
    """True for BTC/LBTC rail changes of the same Bitcoin exposure."""

    assets = {str(out_asset or "").strip().upper(), str(in_asset or "").strip().upper()}
    return assets == _BITCOIN_CARRY_ASSETS


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
    otherwise fail case-insensitive evidence joins.
    Only fold real 64-char hex ids so Lightning ``payment_hash`` values and
    synthetic CSV ids are untouched. Automatic ownership grouping uses the
    stricter :func:`onchain_transfer_scope`; this helper remains for reviewed
    evidence joins and refund-link normalization.
    """
    text = str(external_id)
    if len(text) == 64 and all(char in _HEX_DIGITS for char in text):
        return text.lower()
    return text


def _row_field(row, key):
    """Read ``key`` from a sqlite3.Row-like or dict row, ``None`` if absent."""
    if type(row) is dict:
        return row.get(key)
    getter = getattr(row, "get", None)
    if getter is not None:
        return getter(key)
    try:
        return row[key]
    except (KeyError, IndexError):
        return None


_LIGHTNING_PAYMENT_HASH_SOURCES = frozenset({"core_lightning", "lnd"})
_NON_LIGHTNING_PAYMENT_HASH_SOURCES = frozenset(
    {"chain_script", "chain_script_unique_outpoint"}
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


def is_lightning_payment_hash_row(row):
    """True when a row's payment_hash comes from a Lightning node itself.

    Shared with the swap matcher: rows this predicate accepts auto-pair in
    the journal (detect_intra_transfers hash pass), so the matcher must
    suppress the same pairs from swap review — and ONLY those (chain_script
    HTLC hashes are swap evidence and stay reviewable).
    """
    source = _normalized_lower(_row_field(row, "payment_hash_source"))
    if source in _NON_LIGHTNING_PAYMENT_HASH_SOURCES:
        return False
    kind = _normalized_lower(_row_field(row, "kind"))
    raw = _json_mapping(_row_field(row, "raw_json"))
    provenance = raw.get("_kassiber_provenance")
    import_source = (
        _normalized_lower(provenance.get("import_source"))
        if isinstance(provenance, Mapping)
        else ""
    )
    # payment_hash_source is an importable field, so the label alone is not a
    # trust boundary.  Native node sync stamps a reserved provenance marker in
    # core.imports after stripping any user-supplied copy.  Require the adapter,
    # source label and row kind to agree before automatic MOVE treatment.
    if source == "lnd":
        return import_source == "lnd" and kind in {"lnd_invoice", "lnd_pay"}
    if source == "core_lightning":
        return import_source == "core-lightning" and kind in {
            "cln_invoice",
            "cln_pay",
            "ln_invoice",
            "ln_pay",
        }
    return False



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
        scope = onchain_transfer_scope(row)
        if scope is None:
            continue
        by_key[scope].append(row)

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
        # Same-event rows are useful candidate evidence, but this grouping is
        # intentionally not an authority boundary. The custody interpreter
        # later requires closed observer provenance on both endpoints (or an
        # explicit review) before this candidate can carry basis.
        pairs.append(
            {
                "out": out_row,
                "in": in_row,
                "source": "row_matched",
            }
        )
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
        payment_hash = canonical_payment_hash(_row_field(row, "payment_hash"))
        if payment_hash is None:
            continue
        if not is_lightning_payment_hash_row(row):
            continue
        network_domain = bitcoin_network_domain(row)
        if network_domain is None:
            continue
        by_hash[(payment_hash, row["asset"], network_domain)].append(row)
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
        # LND/CLN expose the routed principal and routing fee separately. A
        # native-node self-payment therefore carries equal principal on both
        # owned legs; treating a principal shortfall as an implied MOVE fee can
        # absorb most of a small payment under the generic 2,500-sat floor.
        if int(_row_field(out_row, "amount") or 0) != int(
            _row_field(in_row, "amount") or 0
        ):
            continue
        # Same-wallet Lightning circular payments are still internal movements:
        # pairing by payment_hash prevents the outbound/inbound legs from becoming
        # a taxable disposal plus fresh acquisition. The txid path above stays
        # cross-wallet-only because same-wallet on-chain txid rows are less
        # semantically precise (change, provider artifacts, or manual repair rows).
        if out_row["id"] in matched_ids or in_row["id"] in matched_ids:
            continue
        pairs.append(
            {
                "out": out_row,
                "in": in_row,
                "source": "lightning_payment_hash",
            }
        )
        matched_ids.add(out_row["id"])
        matched_ids.add(in_row["id"])
    return pairs, matched_ids


def detect_unscoped_transfer_review_ids(rows):
    """Return cross-wallet row ids that share an unresolved import identity.

    A shared ``external_id`` is not physical proof and must never create an
    automatic MOVE.  It is still enough evidence to prevent the two sides from
    silently booking as an unrelated disposal and acquisition when the rows
    cannot produce a canonical on-chain scope.  Keep those rows in review until
    a custody component records the missing physical link (or the user excludes
    the false association).

    Blank identifiers are deliberately ignored: coalescing them would mix every
    unrelated graphless import in the profile into one review group.
    """

    groups = defaultdict(list)
    for row in rows:
        row_id = str(_row_field(row, "id") or "")
        if row_id.startswith(_SYNTHETIC_TRANSFER_ID_PREFIXES):
            # A validated custody component has already supplied the physical
            # interpretation. Its synthetic projection rows intentionally share
            # a component id and must not be sent back into unresolved review.
            continue
        if onchain_transfer_scope(row) is not None:
            continue
        external_id = str(_row_field(row, "external_id") or "").strip()
        if not external_id:
            continue
        asset = str(_row_field(row, "asset") or "").strip().upper()
        groups[(normalize_group_txid(external_id), asset)].append(row)

    review_ids = set()
    for group in groups.values():
        outs = [
            row
            for row in group
            if _row_field(row, "direction") == "outbound"
            and int(_row_field(row, "amount") or 0) > 0
        ]
        ins = [
            row
            for row in group
            if _row_field(row, "direction") == "inbound"
            and int(_row_field(row, "amount") or 0) > 0
        ]
        if not outs or not ins:
            continue
        if not any(
            _row_field(out_row, "wallet_id") != _row_field(in_row, "wallet_id")
            for out_row in outs
            for in_row in ins
        ):
            continue
        review_ids.update(str(row["id"]) for row in group)
    return review_ids
