"""Canonical physical-event and immutable evidence input for custody quantity."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Any, Mapping, Sequence

from .chain_observer.provenance import (
    AUTHORITY_VERSION,
    canonical_graph_hash,
    canonical_observed_quantity_hash,
    row_has_current_authoritative_observation,
)
from ..wallet_descriptors import normalize_chain, normalize_network


_LIGHTNING_WALLET_KINDS = frozenset(
    {"lnd", "coreln", "cln", "lightning", "nwc", "phoenix"}
)
_VERIFIED_AUTHORITY_KEY = "_kassiber_verified_chain_observation"


@dataclass(frozen=True, order=True)
class ProtocolScope:
    """One normalized physical Bitcoin protocol/network scope.

    ``protocol_chain`` names the observation namespace used by canonical event
    identity. ``rail``/``base_chain`` are the equivalent custody-component
    fields. Keeping that translation here prevents Lightning BTC from silently
    falling back to Bitcoin on-chain merely because both use the BTC asset code.
    """

    protocol_chain: str
    network: str
    rail: str
    base_chain: str


@dataclass(frozen=True)
class BoundaryAmounts:
    """Canonical principal, fee, and wallet movement for one boundary leg."""

    direction: str
    observed_amount_msat: int
    principal_msat: int
    fee_msat: int
    wallet_movement_msat: int
    wallet_delta_msat: int


def normalize_boundary_amounts(
    *,
    direction: Any,
    amount_msat: Any,
    fee_msat: Any = 0,
    amount_includes_fee: Any = False,
) -> BoundaryAmounts:
    """Normalize imported boundary arithmetic without guessing semantics.

    Some outbound observers store principal in ``amount`` while others store
    the complete wallet debit. This is the single arithmetic boundary used by
    discovery, reviewed plans, canonical observations, and component coverage.
    Validation remains at each caller's evidence boundary so malformed imports
    continue to fail closed in their existing typed issue path.
    """

    normalized_direction = str(direction or "").strip().lower()
    observed = int(amount_msat or 0)
    fee = int(fee_msat or 0)
    included = bool(amount_includes_fee)
    principal = (
        observed - fee
        if normalized_direction == "outbound" and included
        else observed
    )
    wallet_movement = (
        principal + fee
        if normalized_direction == "outbound"
        else observed
    )
    wallet_delta = (
        -wallet_movement
        if normalized_direction == "outbound"
        else wallet_movement
    )
    return BoundaryAmounts(
        direction=normalized_direction,
        observed_amount_msat=observed,
        principal_msat=principal,
        fee_msat=fee,
        wallet_movement_msat=wallet_movement,
        wallet_delta_msat=wallet_delta,
    )


def row_boundary_amounts(row: Mapping[str, Any]) -> BoundaryAmounts:
    """Normalize one imported transaction-like mapping."""

    return normalize_boundary_amounts(
        direction=_field(row, "direction"),
        amount_msat=_field(row, "amount"),
        fee_msat=_field(row, "fee"),
        amount_includes_fee=_field(row, "amount_includes_fee", False),
    )


def _field(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def _json_object(value: Any) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    try:
        payload = json.loads(value or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return dict(payload) if isinstance(payload, Mapping) else {}


def row_principal_msat(row: Mapping[str, Any]) -> int:
    """Return the spend principal represented by an imported transaction row.

    Outbound rows from some observers store the complete wallet debit in
    ``amount`` and mark that the fee is already included. Every custody review
    allocates principal, while the fee remains a separate sibling quantity.
    """

    return row_boundary_amounts(row).principal_msat


def enriched_quantity_rows(
    rows: Sequence[Mapping[str, Any]],
) -> tuple[dict[str, Any], ...]:
    """Attach canonical event scope without copying wallet config to evidence."""

    enriched = []
    for row in rows:
        item = dict(row)
        # Verify the persisted observer commitment before applying any
        # deterministic, multi-row accounting normalization below. The value
        # is a frozen Python object, never serialized input, so raw imports
        # cannot manufacture this closed authority channel.
        item.pop(_VERIFIED_AUTHORITY_KEY, None)
        authority = assess_authoritative_chain_observation(item)
        if authority.authoritative:
            item[_VERIFIED_AUTHORITY_KEY] = authority
        config = _json_object(_field(row, "config_json"))
        raw = _json_object(_field(row, "raw_json"))
        wallet_kind = str(_field(row, "wallet_kind") or "").strip().lower()
        chain = raw.get("chain") or config.get("chain")
        if not chain and wallet_kind in _LIGHTNING_WALLET_KINDS:
            chain = "lightning"
        item["chain"] = chain
        item["network"] = raw.get("network") or config.get("network")
        item.pop("config_json", None)
        enriched.append(item)

    # A single multi-wallet on-chain spend is commonly imported once per
    # source wallet, with the whole transaction fee stamped on every row. The
    # per-wallet debit (amount + fee) is still useful, but the physical event has
    # only one fee. Preserve each wallet debit while reclassifying duplicate fee
    # copies as principal on all but one deterministic source row.
    outbound_by_event: dict[CanonicalEventKey, list[dict[str, Any]]] = {}
    for item in enriched:
        if str(item.get("direction") or "") != "outbound":
            continue
        try:
            event_key = canonical_event_key(item)
        except (TypeError, ValueError):
            continue
        if event_key.native_namespace != "chain":
            continue
        outbound_by_event.setdefault(event_key, []).append(item)
    for event_rows in outbound_by_event.values():
        positive_fee_rows = [
            item for item in event_rows if int(item.get("fee") or 0) > 0
        ]
        duplicate_fees = {int(item.get("fee") or 0) for item in positive_fee_rows}
        if len(positive_fee_rows) <= 1 or len(duplicate_fees) != 1:
            continue
        keeper = min(positive_fee_rows, key=lambda item: str(item.get("id") or ""))
        for item in positive_fee_rows:
            if item is keeper:
                continue
            duplicate_fee = int(item.get("fee") or 0)
            if not bool(item.get("amount_includes_fee", False)):
                item["amount"] = int(item.get("amount") or 0) + duplicate_fee
            item["fee"] = 0
            item["custody_duplicate_event_fee_normalized"] = True
    return tuple(enriched)


def _canonical_raw_json(value: Any) -> Any:
    if value in (None, ""):
        return {}
    if isinstance(value, (bytes, bytearray)):
        value = value.decode("utf-8", errors="replace")
    if isinstance(value, str):
        try:
            return json.loads(value)
        except (TypeError, ValueError, json.JSONDecodeError):
            return {"unparsed_text": value}
    return value


def _hash_payload(payload: Mapping[str, Any]) -> tuple[str, str]:
    encoded = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest(), encoded


def _normalize_chain(value: Any, asset: str) -> str:
    text = str(value or "").strip().lower()
    if text in {"lightning", "ln"}:
        return "lightning"
    return normalize_chain(text or ("liquid" if asset == "LBTC" else "bitcoin"))


def _normalize_network(chain: str, value: Any) -> str:
    text = str(value or "").strip().lower()
    if chain == "lightning":
        network = {
            "": "main",
            "main": "main",
            "mainnet": "main",
            "bitcoin": "main",
            "test": "test",
            "testnet": "test",
            "regtest": "regtest",
            "signet": "signet",
        }.get(text)
        if network is None:
            raise ValueError(f"Unsupported lightning network '{value}'")
        return network
    return normalize_network(chain, text or None)


def resolve_protocol_scope(row: Mapping[str, Any]) -> ProtocolScope:
    """Resolve typed raw, wallet-kind, and configured scope consistently.

    Raw protocol evidence wins. Without it, a Lightning wallet kind is itself
    typed protocol evidence and takes precedence over the generic BTC asset
    default. Unsupported future protocols fail closed so callers can surface a
    canonical evidence issue instead of grouping them as Bitcoin mainnet.
    """

    raw = _json_object(_field(row, "raw_json"))
    config = _json_object(
        _field(row, "config_json") or _field(row, "wallet_config_json")
    )
    wallet_kind = str(_field(row, "wallet_kind") or "").strip().lower()
    asset = str(_field(row, "asset") or "").strip().upper()
    raw_chain = raw.get("chain")
    explicit_chain = _field(row, "chain")
    configured_chain = config.get("chain")
    if raw_chain not in (None, ""):
        chain_value = raw_chain
    elif wallet_kind in _LIGHTNING_WALLET_KINDS:
        chain_value = "lightning"
    elif explicit_chain not in (None, ""):
        chain_value = explicit_chain
    else:
        chain_value = configured_chain or ("liquid" if asset == "LBTC" else "bitcoin")
    protocol_chain = _normalize_chain(chain_value, asset)
    network_value = (
        raw.get("network")
        or _field(row, "network")
        or config.get("network")
    )
    network = _normalize_network(protocol_chain, network_value)
    return ProtocolScope(
        protocol_chain=protocol_chain,
        network=network,
        rail=protocol_chain,
        base_chain="bitcoin" if protocol_chain == "lightning" else protocol_chain,
    )


def _canonical_txid(value: Any) -> str | None:
    text = str(value or "").strip().lower()
    if len(text) != 64:
        return None
    try:
        bytes.fromhex(text)
    except ValueError:
        return None
    return text


@dataclass(frozen=True, order=True)
class CanonicalEventKey:
    chain: str
    network: str
    native_namespace: str
    native_event_id: str

    def __post_init__(self) -> None:
        if not all(
            (self.chain, self.network, self.native_namespace, self.native_event_id)
        ):
            raise ValueError("canonical events require chain, network, namespace, and id")


def canonical_event_key(row: Mapping[str, Any]) -> CanonicalEventKey:
    raw = _canonical_raw_json(_field(row, "raw_json", {}))
    raw = raw if isinstance(raw, Mapping) else {}
    scope = resolve_protocol_scope(row)
    chain = scope.protocol_chain
    network = scope.network
    provenance = raw.get("_kassiber_provenance")
    provenance = provenance if isinstance(provenance, Mapping) else {}
    native_id_candidates = (
        _field(row, "native_event_id"),
        raw.get("txid"),
        _field(row, "external_id"),
    )
    declared_native_id = next(
        (value for value in native_id_candidates if value not in (None, "")),
        None,
    )
    external_id_kind = str(
        _field(row, "external_id_kind")
        or raw.get("external_id_kind")
        or ""
    ).strip().lower()
    txid_field_names = {
        "txid",
        "txhash",
        "transactionid",
        "transactionhash",
        "onchaintxid",
    }
    txid_candidates = [
        value
        for payload in (
            raw,
            raw.get("tx") if isinstance(raw.get("tx"), Mapping) else {},
            (
                raw.get("ownership_graph")
                if isinstance(raw.get("ownership_graph"), Mapping)
                else {}
            ),
        )
        for key, value in payload.items()
        if "".join(char for char in str(key).lower() if char.isalnum())
        in txid_field_names
    ]
    if external_id_kind == "txid":
        txid_candidates.extend(
            (_field(row, "native_event_id"), _field(row, "external_id"))
        )
    txid = next(
        (
            canonical
            for canonical in map(_canonical_txid, txid_candidates)
            if canonical is not None
        ),
        None,
    )
    if chain in {"bitcoin", "liquid"} and txid is not None:
        namespace = "chain"
        native_event_id = txid
    else:
        namespace = str(
            _field(row, "native_namespace")
            or _field(row, "source_ref")
            or _field(row, "backend_name")
            or provenance.get("import_source")
            or _field(row, "wallet_id")
            or ""
        ).strip().lower()
        native_event_id = str(
            declared_native_id
            or _field(row, "fingerprint")
            or _field(row, "id")
            or ""
        ).strip().lower()
    return CanonicalEventKey(chain, network, namespace, native_event_id)


def canonical_quantity_payload(
    row: Mapping[str, Any],
    event_key: CanonicalEventKey | None = None,
) -> dict[str, Any]:
    """Stable physical-quantity identity, unaffected by graph enrichment."""

    key = event_key or canonical_event_key(row)
    return {
        "schema_version": 1,
        "event": {
            "chain": key.chain,
            "network": key.network,
            "native_namespace": key.native_namespace,
            "native_event_id": key.native_event_id,
        },
        "wallet_id": str(_field(row, "wallet_id") or ""),
        "direction": str(_field(row, "direction") or "").lower(),
        "asset": str(_field(row, "asset") or "").upper(),
        "amount_msat": int(_field(row, "amount") or 0),
        "fee_msat": int(_field(row, "fee") or 0),
        "amount_includes_fee": bool(_field(row, "amount_includes_fee", False)),
    }


def canonical_evidence_payload(
    row: Mapping[str, Any],
    event_key: CanonicalEventKey | None = None,
) -> dict[str, Any]:
    """Versioned evidence detail retained beside the stable quantity hash."""

    return {
        "schema_version": 1,
        "quantity": canonical_quantity_payload(row, event_key),
        "fingerprint": str(_field(row, "fingerprint") or ""),
        "occurred_at": str(_field(row, "occurred_at") or ""),
        "confirmed_at": str(_field(row, "confirmed_at") or ""),
        "kind": str(_field(row, "kind") or ""),
        "payment_hash": str(_field(row, "payment_hash") or ""),
        "payment_hash_source": str(_field(row, "payment_hash_source") or ""),
        "swap_refund_funding_txid": str(
            _field(row, "swap_refund_funding_txid") or ""
        ),
        "swap_refund_funding_vout": _field(row, "swap_refund_funding_vout"),
        "raw_json": _canonical_raw_json(_field(row, "raw_json", {})),
    }


def observation_hash(row: Mapping[str, Any]) -> str:
    """Compatibility name for the stable quantity-core hash."""

    return _hash_payload(canonical_quantity_payload(row))[0]


@dataclass(frozen=True)
class EvidenceSnapshot:
    quantity_hash: str
    detail_hash: str
    payload_json: str

    @classmethod
    def from_transaction(
        cls,
        row: Mapping[str, Any],
        event_key: CanonicalEventKey | None = None,
    ) -> "EvidenceSnapshot":
        key = event_key or canonical_event_key(row)
        quantity_hash, _ = _hash_payload(canonical_quantity_payload(row, key))
        detail_hash, payload_json = _hash_payload(canonical_evidence_payload(row, key))
        return cls(quantity_hash, detail_hash, payload_json)


@dataclass(frozen=True)
class ChainObservationAuthority:
    """Result of binding a current row to persisted observer provenance.

    Authority is deliberately supplied out-of-band from the SQL persistence
    boundary. Nothing inside ``raw_json`` can construct or upgrade this result;
    strings such as ``{"observer": "bdk"}`` remain ordinary imported detail.
    """

    authoritative: bool
    reason: str
    authority_version: int | None = None
    graph_hash: str | None = None
    quantity_hash: str | None = None


def assess_authoritative_chain_observation(
    row: Mapping[str, Any],
) -> ChainObservationAuthority:
    """Verify a current observation against its persisted authority record.

    The caller must provide the ledger row joined to Kassiber's closed
    ``chain_observation_provenance`` table. Hashing delegates to the observer
    boundary's canonical codecs, so authoritative apply and custody cannot
    silently diverge. This function intentionally never searches the
    transaction payload for provenance.

    Both commitments are required: the quantity hash binds physical event
    identity, wallet, direction, asset, amount, and fee semantics; the graph
    hash binds the canonical input/output evidence used for native ownership.
    A retraction, stale revision, unsupported authority version, or either hash
    mismatch fails closed.
    """

    def result(
        authoritative: bool,
        reason: str,
        *,
        version: int | None = None,
        graph_hash: str | None = None,
        quantity_hash: str | None = None,
    ) -> ChainObservationAuthority:
        return ChainObservationAuthority(
            authoritative=authoritative,
            reason=reason,
            authority_version=version,
            graph_hash=graph_hash,
            quantity_hash=quantity_hash,
        )

    raw_version = _field(row, "observation_authority_version")
    if raw_version in (None, ""):
        return result(False, "provenance_missing")
    try:
        version = int(raw_version)
    except (TypeError, ValueError):
        version = None
    if version != AUTHORITY_VERSION:
        return result(False, "authority_version_unsupported")
    try:
        event_key = canonical_event_key(row)
    except (TypeError, ValueError):
        return result(False, "canonical_event_invalid", version=version)
    if event_key.native_namespace != "chain" or event_key.chain not in {
        "bitcoin",
        "liquid",
    }:
        return result(False, "canonical_chain_scope_invalid", version=version)
    expected_quantity_hash = str(
        _field(row, "observation_quantity_hash") or ""
    ).strip().lower()
    actual_quantity_hash = canonical_observed_quantity_hash(row)
    if expected_quantity_hash != actual_quantity_hash:
        return result(
            False,
            "quantity_hash_mismatch",
            version=version,
            quantity_hash=actual_quantity_hash,
        )
    expected_graph_hash = str(
        _field(row, "observation_graph_hash") or ""
    ).strip().lower()
    actual_graph_hash = canonical_graph_hash(_field(row, "raw_json", "{}"))
    if expected_graph_hash != actual_graph_hash:
        return result(
            False,
            "graph_hash_mismatch",
            version=version,
            graph_hash=actual_graph_hash,
            quantity_hash=actual_quantity_hash,
        )
    if not row_has_current_authoritative_observation(row):
        return result(
            False,
            "authoritative_observation_invalid",
            version=version,
            graph_hash=actual_graph_hash,
            quantity_hash=actual_quantity_hash,
        )
    return result(
        True,
        "matched",
        version=version,
        graph_hash=actual_graph_hash,
        quantity_hash=actual_quantity_hash,
    )


@dataclass(frozen=True)
class QuantityObservation:
    transaction_id: str
    # Persistence/audit FK anchor.  Ordinary observations anchor themselves;
    # an engine-derived rowless destination keeps its content-addressed
    # synthetic identity above while anchoring durable postings to the real
    # physical source transaction.
    anchor_transaction_id: str
    quantity_hash: str
    evidence_detail_hash: str
    evidence_payload_json: str
    event_key: CanonicalEventKey
    # Quantity state is normally built for one profile at a time. Retaining
    # the owning profile here nevertheless makes downstream basis barriers
    # safe when a caller accidentally supplies a wider row set. Profile is
    # accounting scope, not physical event identity, so it is deliberately
    # excluded from ``canonical_quantity_payload`` and the quantity hash.
    profile_id: str
    authoritative_chain_observation: bool
    observation_authority_reason: str
    fee_attribution: str
    wallet_id: str
    asset: str
    direction: str
    amount_msat: int
    fee_msat: int
    amount_includes_fee: bool
    occurred_at: str

    @classmethod
    def from_transaction(
        cls,
        row: Mapping[str, Any],
        event_key: CanonicalEventKey | None = None,
    ) -> "QuantityObservation":
        key = event_key or canonical_event_key(row)
        snapshot = EvidenceSnapshot.from_transaction(row, key)
        verified_authority = _field(row, _VERIFIED_AUTHORITY_KEY)
        authority = (
            verified_authority
            if isinstance(verified_authority, ChainObservationAuthority)
            and verified_authority.authoritative
            else assess_authoritative_chain_observation(row)
        )
        fee_attribution = str(
            _field(row, "observation_fee_attribution") or "unknown"
        ).strip().lower()
        if not authority.authoritative or fee_attribution not in {
            "exact",
            "implicit_wallet_delta",
        }:
            fee_attribution = "unknown"
        observation = cls(
            transaction_id=str(_field(row, "id") or ""),
            anchor_transaction_id=str(
                _field(row, "journal_transaction_id")
                or _field(row, "id")
                or ""
            ),
            quantity_hash=snapshot.quantity_hash,
            evidence_detail_hash=snapshot.detail_hash,
            evidence_payload_json=snapshot.payload_json,
            event_key=key,
            profile_id=str(_field(row, "profile_id") or ""),
            authoritative_chain_observation=authority.authoritative,
            observation_authority_reason=authority.reason,
            fee_attribution=fee_attribution,
            wallet_id=str(_field(row, "wallet_id") or ""),
            asset=str(_field(row, "asset") or "").upper(),
            direction=str(_field(row, "direction") or "").lower(),
            amount_msat=int(_field(row, "amount") or 0),
            fee_msat=int(_field(row, "fee") or 0),
            amount_includes_fee=bool(
                _field(row, "amount_includes_fee", False)
            ),
            occurred_at=str(_field(row, "occurred_at") or ""),
        )
        observation._validate()
        return observation

    def _validate(self) -> None:
        if not self.transaction_id or not self.anchor_transaction_id or not self.wallet_id:
            raise ValueError(
                "quantity observations require transaction, anchor, and wallet ids"
            )
        for label, value in (
            ("quantity_hash", self.quantity_hash),
            ("evidence_detail_hash", self.evidence_detail_hash),
        ):
            if len(value) != 64 or any(
                char not in "0123456789abcdef" for char in value
            ):
                raise ValueError(f"quantity observation {label} must be lowercase SHA-256")
        if self.direction not in {"inbound", "outbound"}:
            raise ValueError("quantity observation direction must be inbound or outbound")
        if not self.asset:
            raise ValueError("quantity observations require an asset")
        if type(self.amount_msat) is not int or self.amount_msat < 0:
            raise ValueError("quantity observation amount_msat must be non-negative")
        if type(self.fee_msat) is not int or self.fee_msat < 0:
            raise ValueError("quantity observation fee_msat must be non-negative")
        if self.direction == "inbound" and self.amount_includes_fee:
            raise ValueError("inbound quantity cannot use amount_includes_fee")
        if self.amount_msat == 0 and not (
            self.direction == "outbound" and self.fee_msat > 0
        ):
            raise ValueError("zero quantity is valid only for a fee-only outbound")
        if self.amount_includes_fee and self.fee_msat > self.amount_msat:
            raise ValueError("an included fee cannot exceed the observed amount")

    @property
    def boundary_amounts(self) -> BoundaryAmounts:
        return normalize_boundary_amounts(
            direction=self.direction,
            amount_msat=self.amount_msat,
            fee_msat=self.fee_msat,
            amount_includes_fee=self.amount_includes_fee,
        )

    @property
    def principal_msat(self) -> int:
        return self.boundary_amounts.principal_msat

    @property
    def wallet_delta_msat(self) -> int:
        return self.boundary_amounts.wallet_delta_msat


@dataclass(frozen=True)
class CanonicalEventIssue:
    event_key: CanonicalEventKey
    code: str
    message: str
    transaction_ids: tuple[str, ...]
    details: Mapping[str, Any]


@dataclass(frozen=True)
class CanonicalQuantityEvent:
    event_key: CanonicalEventKey
    legs: tuple[QuantityObservation, ...]
    evidence_snapshots: tuple[EvidenceSnapshot, ...]
    source_transaction_ids: tuple[str, ...]
    observation_aliases: tuple[tuple[str, str], ...]


@dataclass(frozen=True)
class CanonicalQuantityInput:
    events: tuple[CanonicalQuantityEvent, ...]
    rejected_events: tuple[CanonicalEventIssue, ...]

    @property
    def observations(self) -> tuple[QuantityObservation, ...]:
        return tuple(leg for event in self.events for leg in event.legs)


def _fallback_event_key(row: Mapping[str, Any], ordinal: int) -> CanonicalEventKey:
    identity = str(
        _field(row, "id")
        or _field(row, "fingerprint")
        or f"row-{ordinal}"
    ).strip().lower()
    return CanonicalEventKey("invalid", "invalid", "row", identity)


def _event_issue(
    event_key: CanonicalEventKey,
    code: str,
    message: str,
    rows: Sequence[Mapping[str, Any]],
    **details: Any,
) -> CanonicalEventIssue:
    return CanonicalEventIssue(
        event_key=event_key,
        code=code,
        message=message,
        transaction_ids=tuple(
            sorted(
                {
                    str(_field(row, "id") or "")
                    for row in rows
                    if _field(row, "id") not in (None, "")
                }
            )
        ),
        details=details,
    )


def build_canonical_quantity_input(
    rows: Sequence[Mapping[str, Any]],
) -> CanonicalQuantityInput:
    """Deduplicate wallet-event aggregate rows without losing location legs.

    Kassiber import rows are wallet-event aggregates.  Therefore one canonical
    physical event may legitimately contain A/outbound and B/inbound legs, but
    one ``(wallet, direction, asset)`` slot has exactly one aggregate quantity.
    Exact repeats of that slot deduplicate.  Different aggregate semantics are
    contradictory evidence: the affected event is rejected rather than summed,
    while unrelated events remain projectable.
    """

    grouped: dict[CanonicalEventKey, list[Mapping[str, Any]]] = {}
    rejected: list[CanonicalEventIssue] = []
    for ordinal, row in enumerate(rows):
        try:
            key = canonical_event_key(row)
        except (TypeError, ValueError) as exc:
            key = _fallback_event_key(row, ordinal)
            rejected.append(
                _event_issue(
                    key,
                    "canonical_event_identity_invalid",
                    str(exc),
                    [row],
                )
            )
            continue
        grouped.setdefault(key, []).append(row)

    events: list[CanonicalQuantityEvent] = []
    for key, event_rows in sorted(grouped.items()):
        observations: list[QuantityObservation] = []
        invalid_messages: list[str] = []
        for row in event_rows:
            try:
                observations.append(QuantityObservation.from_transaction(row, key))
            except (TypeError, ValueError) as exc:
                invalid_messages.append(str(exc))
        if invalid_messages:
            rejected.append(
                _event_issue(
                    key,
                    "canonical_event_leg_invalid",
                    "one or more wallet-event aggregate legs are invalid",
                    event_rows,
                    errors=sorted(set(invalid_messages)),
                )
            )
            continue

        by_slot: dict[tuple[str, str, str], list[QuantityObservation]] = {}
        for observation in observations:
            slot = (
                observation.wallet_id,
                observation.direction,
                observation.asset,
            )
            by_slot.setdefault(slot, []).append(observation)
        contradictions = []
        for slot, slot_observations in sorted(by_slot.items()):
            semantics = {
                (
                    item.amount_msat,
                    item.fee_msat,
                    item.amount_includes_fee,
                )
                for item in slot_observations
            }
            if len(semantics) > 1:
                contradictions.append(
                    {
                        "wallet_id": slot[0],
                        "direction": slot[1],
                        "asset": slot[2],
                        "semantics": sorted(semantics),
                    }
                )
        if contradictions:
            rejected.append(
                _event_issue(
                    key,
                    "canonical_event_leg_contradiction",
                    "wallet-event aggregate observations disagree",
                    event_rows,
                    contradictory_slots=contradictions,
                )
            )
            continue

        legs = tuple(
            min(
                slot_observations,
                key=lambda item: (
                    item.occurred_at,
                    item.evidence_detail_hash,
                    item.transaction_id,
                ),
            )
            for _, slot_observations in sorted(by_slot.items())
        )
        selected_by_slot = {
            (item.wallet_id, item.direction, item.asset): item for item in legs
        }
        aliases = tuple(
            sorted(
                (
                    item.transaction_id,
                    selected_by_slot[
                        (item.wallet_id, item.direction, item.asset)
                    ].quantity_hash,
                )
                for item in observations
            )
        )
        snapshots_by_hash = {
            item.evidence_detail_hash: EvidenceSnapshot(
                item.quantity_hash,
                item.evidence_detail_hash,
                item.evidence_payload_json,
            )
            for item in observations
        }
        events.append(
            CanonicalQuantityEvent(
                event_key=key,
                legs=legs,
                evidence_snapshots=tuple(
                    snapshots_by_hash[value]
                    for value in sorted(snapshots_by_hash)
                ),
                source_transaction_ids=tuple(
                    sorted({item.transaction_id for item in observations})
                ),
                observation_aliases=aliases,
            )
        )
    return CanonicalQuantityInput(
        events=tuple(events),
        rejected_events=tuple(
            sorted(
                rejected,
                key=lambda item: (item.event_key, item.code, item.transaction_ids),
            )
        ),
    )



__all__ = [
    "BoundaryAmounts",
    "ChainObservationAuthority",
    "CanonicalEventIssue",
    "CanonicalEventKey",
    "CanonicalQuantityEvent",
    "CanonicalQuantityInput",
    "EvidenceSnapshot",
    "ProtocolScope",
    "QuantityObservation",
    "build_canonical_quantity_input",
    "assess_authoritative_chain_observation",
    "canonical_event_key",
    "canonical_evidence_payload",
    "canonical_quantity_payload",
    "enriched_quantity_rows",
    "observation_hash",
    "normalize_boundary_amounts",
    "row_boundary_amounts",
    "row_principal_msat",
    "resolve_protocol_scope",
]
