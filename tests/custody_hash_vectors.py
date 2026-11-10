"""Golden vectors for every custody hash and id that is stored or compared.

Stored evidence commitments, decision and issue ids, gap fingerprints and
uuid5 component ids are compared against values Kassiber recomputes on the
next build, so a port of the custody path must reproduce them byte for byte.
Each vector names the product function that produced it, the exact
serialization it hashes (with the ``json.dumps`` keyword arguments), its input
and its output. Outputs that hash a text keep that text as a ``*_preimage``
next to the hash.

    uv run --locked python -m tests.custody_hash_vectors          # check
    uv run --locked python -m tests.custody_hash_vectors --write  # regenerate

Inputs come from a seeded generator plus hand-picked edge cases (unicode,
floats including non-finite and subnormal values, big integers, nested and
unparsable ``raw_json``, missing fields), so regeneration is reproducible.
``tests/test_custody_hash_vectors.py`` recomputes every stored vector with
the product functions and fails on any difference.

Fixture encoding: one vector per line, keys sorted, ASCII only. Values that
JSON cannot carry exactly are tagged: ``{"$float": repr}`` for every float,
``{"$int": text}`` for integers outside the signed and unsigned 64-bit range,
``{"$tuple": [...]}``, ``{"$bytes": hex}``, ``{"$decimal": text}``,
``{"$event_key": [chain, network, namespace, id]}`` for a
``CanonicalEventKey`` and ``{"$gap_candidate": {...}}`` for a
``CustodyGapCandidate``.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, fields
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import random
import sys
import tempfile
from typing import Any, Callable, Iterable, Iterator
import uuid

from kassiber import fingerprints
from kassiber.core import (
    custody_ai_audit,
    custody_authored_migration,
    custody_component_planner,
    custody_evidence,
    custody_filed_reports,
    custody_gap_reviews,
    custody_gaps,
    custody_native_audit,
    custody_native_reconciliation,
    custody_native_transitions,
    custody_quantity_runtime,
    custody_quantity_store,
    custody_tax_projection,
)
from kassiber.core.chain_observer import provenance


SEED = 20261026
FIXTURE_DIR = Path(__file__).resolve().parent / "fixtures" / "custody_hash_vectors"

COMPACT_SORTED = {"sort_keys": True, "separators": [",", ":"]}
COMPACT_SORTED_UNICODE = {**COMPACT_SORTED, "ensure_ascii": False}
COMPACT_SORTED_ASCII = {**COMPACT_SORTED, "ensure_ascii": True}
COMPACT_ASCII = {"separators": [",", ":"], "ensure_ascii": True}
SHA256_HEX = "sha256 over the UTF-8 bytes, lowercase hex"


# -- encoding ---------------------------------------------------------------


_INT64_MIN = -(2**63)
_UINT64_MAX = 2**64 - 1


def encode(value: Any) -> Any:
    """Tag what plain JSON cannot carry exactly."""

    if isinstance(value, bool) or value is None or isinstance(value, str):
        return value
    if isinstance(value, int):
        if _INT64_MIN <= value <= _UINT64_MAX:
            return value
        return {"$int": str(value)}
    if isinstance(value, float):
        return {"$float": repr(value)}
    if isinstance(value, Decimal):
        return {"$decimal": str(value)}
    if isinstance(value, (bytes, bytearray)):
        return {"$bytes": bytes(value).hex()}
    if isinstance(value, custody_evidence.CanonicalEventKey):
        return {
            "$event_key": [
                value.chain,
                value.network,
                value.native_namespace,
                value.native_event_id,
            ]
        }
    if isinstance(value, custody_gaps.CustodyGapCandidate):
        return {
            "$gap_candidate": {
                item.name: encode(getattr(value, item.name)) for item in fields(value)
            }
        }
    if isinstance(value, tuple):
        return {"$tuple": [encode(item) for item in value]}
    if isinstance(value, list):
        return [encode(item) for item in value]
    if isinstance(value, dict):
        if not all(isinstance(key, str) for key in value):
            raise TypeError(f"vector dict keys must be strings: {value!r}")
        return {key: encode(item) for key, item in value.items()}
    raise TypeError(f"cannot encode {type(value).__name__} in a vector")


def decode(value: Any) -> Any:
    if isinstance(value, list):
        return [decode(item) for item in value]
    if not isinstance(value, dict):
        return value
    if len(value) == 1:
        (tag, payload), = value.items()
        if tag == "$int":
            return int(payload)
        if tag == "$float":
            return float(payload)
        if tag == "$decimal":
            return Decimal(payload)
        if tag == "$bytes":
            return bytes.fromhex(payload)
        if tag == "$tuple":
            return tuple(decode(item) for item in payload)
        if tag == "$event_key":
            return custody_evidence.CanonicalEventKey(*payload)
        if tag == "$gap_candidate":
            return custody_gaps.CustodyGapCandidate(
                **{key: decode(item) for key, item in payload.items()}
            )
    return {key: decode(item) for key, item in value.items()}


def _error(exc: BaseException) -> dict[str, str]:
    return {"type": type(exc).__name__, "message": str(exc)}


def _sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _dumps(value: Any, kwargs: dict[str, Any], **extra: Any) -> str:
    options = dict(kwargs)
    if "separators" in options:
        options["separators"] = tuple(options["separators"])
    return json.dumps(value, **options, **extra)


# -- seeded inputs ----------------------------------------------------------


TEXTS = (
    "a",
    "wallet-1",
    "Wallet One",
    "",
    " ",
    "  padded  ",
    "UPPER",
    "MiXeD",
    "\u00e9",
    "e\u0301",
    "\u00df",
    "\u1e9e",
    "\ufb01",
    "\u0130",
    "\u03a3\u0391\u03a3",
    "\u03c2",
    "\u01c5",
    "\u216b",
    "\U0001f600",
    "\U0001f469\u200d\U0001f4bb",
    "\u94b1\u5305",
    "\u05e9\u05dc\u05d5\u05dd",
    "\u0000",
    "tab\there",
    "line\nbreak",
    "\u2028",
    "\u00a0",
    "\x7f",
    '"quoted"',
    "it's",
    "both ' and \"",
    "back\\slash",
    "\ufeff",
    "\U0010ffff",
    "\ue000",
    "x" * 80,
)

FLOATS = (
    0.0,
    -0.0,
    0.1,
    1.0,
    1.5,
    -2.25,
    1e16,
    1e-7,
    1.5e-7,
    123456789.125,
    5e-324,
    2.2250738585072014e-308,
    1.7976931348623157e308,
    9007199254740993.0,
    0.30000000000000004,
    1e22,
    1e21,
    1e-5,
    0.0001,
    float("inf"),
    float("-inf"),
    float("nan"),
)

BIG_INTS = (
    0,
    1,
    -1,
    999,
    1000,
    2**31,
    2**53,
    2**53 + 1,
    2**63 - 1,
    -(2**63),
    2**64,
    10**30,
    -(10**25),
)

RAW_JSON_TEXTS: tuple[Any, ...] = (
    "",
    "{}",
    " {} ",
    "null",
    "[]",
    "[1,2]",
    '"text"',
    "true",
    "42",
    "1.50",
    "-0",
    '{"a":1,"a":2}',
    '{"n":1E5,"m":1e+5,"o":10.50,"p":-0.0,"q":0e0,"r":1.0e-7,"s":5e-324}',
    '{"big":123456789012345678901234567890,"neg":-98765432109876543210}',
    '{"inf":1e400,"ninf":-1e400}',
    '{"nan":NaN,"pinf":Infinity,"minf":-Infinity}',
    '{"lone":"\\ud800"}',
    '{"pair":"\\ud83d\\ude00"}',
    '{"esc":"\\u00e9\\n\\t\\"\\\\\\/","raw":"\u00e9\U0001f600"}',
    '{"nul":"\\u0000","del":"\x7f"}',
    '{"B":1,"a":2,"_":3,"10":4,"9":5,"\u00e9":6,"\U0001f600":7}',
    "{",
    "not json",
    "{'a': 1}",
    "\ufeff{}",
    '{"a":1} trailing',
    "  ",
    '{"nested":{"deeper":{"list":[1,2.5,{"x":null}],"t":true}}}',
    b"{}",
    b'{"bytes":"\xc3\xa9"}',
    b'{"invalid_utf8":"\xff"}',
)


class Generator:
    """A seeded source of awkward custody inputs."""

    def __init__(self, seed: int) -> None:
        self.rng = random.Random(seed)

    def pick(self, items: Iterable[Any]) -> Any:
        options = tuple(items)
        return options[self.rng.randrange(len(options))]

    def chance(self, numerator: int, denominator: int) -> bool:
        return self.rng.randrange(denominator) < numerator

    def text(self) -> str:
        return self.pick(TEXTS)

    def ident(self, prefix: str) -> str:
        if self.chance(1, 4):
            return self.text()
        return f"{prefix}-{self.rng.randrange(10**6):06d}"

    def hex64(self) -> str:
        return "".join(self.pick("0123456789abcdef") for _ in range(64))

    def txid(self) -> str:
        roll = self.rng.randrange(10)
        value = self.hex64()
        if roll == 0:
            return value.upper()
        if roll == 1:
            return value[:63]
        if roll == 2:
            return value[:62] + "zz"
        if roll == 3:
            return f"  {value}  "
        return value

    def amount(self) -> Any:
        roll = self.rng.randrange(20)
        if roll < 12:
            return self.rng.randrange(10**15)
        if roll < 16:
            return self.pick(BIG_INTS)
        if roll == 16:
            return str(self.rng.randrange(10**9))
        if roll == 17:
            return self.pick((None, "", True, False))
        if roll == 18:
            return self.pick((1.9, -2.5, 1e3))
        return self.pick(("1.5", "abc", " 7 "))

    def json_value(self, depth: int) -> Any:
        kinds = ["int", "big", "float", "str", "bool", "null"]
        if depth > 0:
            kinds += ["list", "dict", "dict"]
        kind = self.pick(kinds)
        if kind == "int":
            return self.rng.randrange(-1000, 10**9)
        if kind == "big":
            return self.pick(BIG_INTS)
        if kind == "float":
            return self.pick(FLOATS)
        if kind == "str":
            return self.text()
        if kind == "bool":
            return self.chance(1, 2)
        if kind == "null":
            return None
        if kind == "list":
            return [self.json_value(depth - 1) for _ in range(self.rng.randrange(4))]
        return {
            self.text() if self.chance(1, 3) else self.pick(("txid", "vin", "vout", "fee", "status", "value_sats", "chain", "network", "tx")): self.json_value(depth - 1)
            for _ in range(self.rng.randrange(5))
        }

    def raw_json(self, *, txid: str | None = None, chain: str | None = None) -> Any:
        if self.chance(1, 3):
            return self.pick(RAW_JSON_TEXTS)
        value = self.json_value(2)
        if not isinstance(value, dict):
            value = {"value": value}
        if txid is not None:
            value[self.pick(("txid", "TxId", "tx_hash", "transactionHash"))] = txid
        if chain is not None:
            value["chain"] = chain
        if self.chance(1, 4):
            value["tx"] = {"txid": txid or self.hex64(), "vin": [], "vout": []}
        return json.dumps(
            value,
            ensure_ascii=self.chance(1, 2),
            sort_keys=self.chance(1, 2),
            indent=self.pick((None, None, 1)),
        )

    def scope(self) -> tuple[str | None, str | None, str]:
        """(chain, network, wallet kind) with mostly supported combinations."""

        roll = self.rng.randrange(12)
        if roll < 5:
            return (
                self.pick(("bitcoin", "btc", "onchain", None, "")),
                self.pick(("main", "mainnet", "bitcoin", "test", "testnet", "regtest", "signet", None)),
                self.pick(("descriptor", "xpub", "address", "samourai")),
            )
        if roll < 8:
            return (
                self.pick(("liquid", "lbtc", "elements")),
                self.pick(("liquid", "liquidv1", "main", "liquidtestnet", "testnet", "elementsregtest", "regtest", None)),
                self.pick(("liquid-electrum", "descriptor")),
            )
        if roll < 10:
            return (
                self.pick(("lightning", "ln", None)),
                self.pick(("main", "mainnet", "bitcoin", "test", "testnet", "regtest", "signet", "", None)),
                self.pick(("lnd", "coreln", "cln", "lightning", "nwc", "phoenix")),
            )
        return (
            self.pick(("dogecoin", "LIGHTNING", " Bitcoin ")),
            self.pick(("moon", "liquidv1", "MAIN")),
            self.pick(("descriptor", "lnd", "")),
        )

    def transaction_row(self) -> dict[str, Any]:
        chain, network, wallet_kind = self.scope()
        txid = self.txid()
        config: dict[str, Any] = {}
        if self.chance(1, 2) and chain:
            config["chain"] = chain
        if self.chance(1, 2) and network:
            config["network"] = network
        raw_chain = chain if self.chance(1, 3) else None
        row: dict[str, Any] = {
            "id": self.ident("tx"),
            "profile_id": self.ident("profile"),
            "wallet_id": self.ident("wallet"),
            "wallet_kind": wallet_kind,
            "config_json": json.dumps(config) if config or self.chance(1, 2) else None,
            "external_id": txid if self.chance(2, 3) else self.ident("ext"),
            "direction": self.pick(("inbound", "outbound", "OUTBOUND", "", None)),
            "asset": self.pick(("BTC", "btc", "LBTC", "l-btc", "USDT", "", None, self.hex64())),
            "amount": self.amount(),
            "fee": self.amount() if self.chance(1, 2) else 0,
            "amount_includes_fee": self.pick((0, 1, True, False, None)),
            "raw_json": self.raw_json(txid=txid if self.chance(1, 2) else None, chain=raw_chain),
            "fingerprint": self.hex64() if self.chance(2, 3) else None,
            "occurred_at": self.pick(("2024-01-02T03:04:05Z", "1970-01-01T00:00:00Z", "2025-12-31T23:59:59.999999+01:00", "", None)),
            "confirmed_at": self.pick(("2024-01-02T03:14:05Z", None, "")),
            "kind": self.pick((None, "deposit", "withdrawal", "swap", self.text())),
            "payment_hash": self.pick((None, self.hex64(), self.hex64().upper())),
            "payment_hash_source": self.pick((None, "invoice", "chain_script_unique_outpoint")),
            "swap_refund_funding_txid": self.pick((None, self.hex64())),
            "swap_refund_funding_vout": self.pick((None, 0, 7, "3", 2**32)),
        }
        optional = {
            "external_id_kind": self.pick(("txid", "TXID", "invoice", "")),
            "native_event_id": self.pick((txid, self.ident("event"))),
            "native_namespace": self.pick(("lnd-node", "Upper-NS", "")),
            "source_ref": self.ident("source"),
            "backend_name": self.pick(("mempool", "Local Electrum")),
            "chain": chain,
            "network": network,
        }
        for key, value in optional.items():
            if self.chance(1, 4):
                row[key] = value
        return row


# -- families ---------------------------------------------------------------


@dataclass(frozen=True)
class Family:
    name: str
    function: str
    serialization: dict[str, Any]
    cases: Callable[[Generator], Iterator[tuple[str, Any]]]
    compute: Callable[[Any], Any]


def _evidence_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    base = {
        "id": "tx-1",
        "profile_id": "profile",
        "wallet_id": "wallet",
        "wallet_kind": "descriptor",
        "config_json": '{"chain":"bitcoin","network":"main"}',
        "external_id": "aa" * 32,
        "direction": "outbound",
        "asset": "BTC",
        "amount": 100_000_000,
        "fee": 1_000,
        "amount_includes_fee": 0,
        "fingerprint": "f" * 64,
        "occurred_at": "2024-01-02T03:04:05Z",
        "confirmed_at": None,
        "kind": None,
        "payment_hash": None,
        "payment_hash_source": None,
        "swap_refund_funding_txid": None,
        "swap_refund_funding_vout": None,
    }
    for index, raw_json in enumerate(RAW_JSON_TEXTS):
        yield f"raw-json-{index:02d}", {**base, "raw_json": raw_json}
    yield "missing-every-optional-field", {"id": "bare", "wallet_id": "w", "raw_json": '{"txid":"' + "bb" * 32 + '"}'}
    yield "lightning-kind-overrides-asset", {**base, "wallet_kind": "lnd", "config_json": None, "external_id": "invoice-1", "raw_json": "{}"}
    yield "liquid-asset-default", {**base, "asset": "LBTC", "config_json": None, "raw_json": "{}"}
    yield "unsupported-chain", {**base, "raw_json": '{"chain":"dogecoin"}'}
    yield "unsupported-lightning-network", {**base, "wallet_kind": "lnd", "network": "moon", "raw_json": "{}"}
    yield "txid-from-nested-ownership-graph", {**base, "external_id": "not-a-txid", "raw_json": '{"ownership_graph":{"Tx_Hash":"' + "CC" * 32 + '"}}'}
    yield "external-id-kind-txid", {**base, "external_id": "DD" * 32, "external_id_kind": "txid", "raw_json": "{}"}
    yield "namespace-from-provenance", {**base, "external_id": "order-9", "raw_json": '{"_kassiber_provenance":{"import_source":"Kraken CSV"}}'}
    yield "unicode-everywhere", {**base, "id": "\u00e9", "wallet_id": "\u94b1\u5305", "kind": "\U0001f600", "external_id": "\u0130d", "raw_json": '{"note":"\u00df\u2028"}'}
    for index in range(110):
        yield f"seeded-{index:03d}", gen.transaction_row()


def _event_key_output(row: dict[str, Any]) -> list[str]:
    key = custody_evidence.canonical_event_key(row)
    return [key.chain, key.network, key.native_namespace, key.native_event_id]


def _commitment_detail_preimage(payload_json: str) -> str:
    payload = json.loads(payload_json)
    for volatile_key in ("fingerprint", "occurred_at", "confirmed_at", "raw_json"):
        payload.pop(volatile_key, None)
    return _dumps(payload, COMPACT_SORTED_UNICODE)


def _evidence_compute(row: dict[str, Any]) -> dict[str, Any]:
    try:
        event_key = _event_key_output(row)
    except (TypeError, ValueError) as exc:
        return {"event_key_error": _error(exc)}
    output: dict[str, Any] = {"event_key": event_key}
    try:
        snapshot = custody_evidence.EvidenceSnapshot.from_transaction(row)
    except Exception as exc:  # the exact failure is part of the contract
        output["snapshot_error"] = _error(exc)
        return output
    _quantity_hash, quantity_preimage = custody_evidence._hash_payload(
        custody_evidence.canonical_quantity_payload(row)
    )
    output.update(
        quantity_hash=snapshot.quantity_hash,
        quantity_preimage=quantity_preimage,
        detail_hash=snapshot.detail_hash,
        detail_preimage=snapshot.payload_json,
        commitment_detail_hash=custody_quantity_store._commitment_detail_hash(snapshot),
        commitment_detail_preimage=_commitment_detail_preimage(snapshot.payload_json),
    )
    return output


def _graph_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index, raw_json in enumerate(RAW_JSON_TEXTS):
        yield f"text-{index:02d}", {"raw_json": raw_json}
    non_text = (
        None,
        {},
        [],
        0,
        True,
        {"a": 1.0, "b": [1e16, -0.0, 5e-324], "c": None},
        {"nan": float("nan"), "inf": float("inf")},
        {"decimal": Decimal("1.10"), "neg": Decimal("-0E-8")},
        {"bytes": b"\x00\xff"},
        {"tuple": (1, "two", 3.0)},
        {"\u00e9": "\U0001f600", "B": 1, "a": 2},
        Decimal("12.3400"),
        b"{}",
    )
    for index, value in enumerate(non_text):
        yield f"value-{index:02d}", {"raw_json": value}
    for index in range(50):
        yield f"seeded-{index:03d}", {"raw_json": gen.raw_json(txid=gen.hex64())}


def _graph_preimage(raw_json: Any) -> str:
    if isinstance(raw_json, str):
        try:
            value = json.loads(raw_json or "{}")
        except (TypeError, ValueError):
            value = {"opaque_text": raw_json}
    else:
        value = raw_json
    return _dumps(value, COMPACT_SORTED_UNICODE, default=str)


def _graph_compute(item: dict[str, Any]) -> dict[str, Any]:
    raw_json = item["raw_json"]
    try:
        graph_hash = provenance.canonical_graph_hash(raw_json)
    except Exception as exc:  # the exact failure is part of the contract
        return {"error": _error(exc)}
    return {"graph_hash": graph_hash, "graph_preimage": _graph_preimage(raw_json)}


def _observed_quantity_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    yield "empty-row", {}
    yield "unicode-ascii-escaped", {"wallet_id": "\u94b1\u5305", "external_id": " \u00c9X ", "direction": " Inbound ", "asset": "l-btc", "amount": 1, "fee": 0, "amount_includes_fee": 1}
    yield "liquid-asset-id-lowercased", {"wallet_id": "w", "external_id": "x", "asset": "AB" * 32, "amount": 5}
    for index in range(40):
        row = gen.transaction_row()
        yield f"seeded-{index:03d}", {
            key: row[key]
            for key in ("wallet_id", "external_id", "direction", "asset", "amount", "fee", "amount_includes_fee")
            if key in row
        }


def _observed_quantity_preimage(row: dict[str, Any]) -> str:
    from kassiber.wallet_descriptors import normalize_asset_code

    payload = {
        "schema_version": 1,
        "wallet_id": str(row.get("wallet_id") or ""),
        "external_id": str(row.get("external_id") or "").strip().lower(),
        "direction": str(row.get("direction") or "").strip().lower(),
        "asset": normalize_asset_code(row.get("asset")),
        "amount_msat": int(row.get("amount") or 0),
        "fee_msat": int(row.get("fee") or 0),
        "amount_includes_fee": bool(row.get("amount_includes_fee")),
    }
    return _dumps(payload, COMPACT_SORTED)


def _observed_quantity_compute(row: dict[str, Any]) -> dict[str, Any]:
    try:
        quantity_hash = provenance.canonical_observed_quantity_hash(row)
    except Exception as exc:  # the exact failure is part of the contract
        return {"error": _error(exc)}
    return {
        "quantity_hash": quantity_hash,
        "quantity_preimage": _observed_quantity_preimage(row),
    }


def _reconciled_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(25):
        txid = gen.hex64()
        row = {
            "id": f"receipt-{index}",
            "profile_id": "profile",
            "wallet_id": gen.ident("wallet"),
            "wallet_kind": "descriptor",
            "config_json": '{"chain":"bitcoin","network":"main"}',
            "external_id": txid,
            "direction": "inbound",
            "asset": "BTC",
            "amount": gen.rng.randrange(1, 10**12),
            "fee": 0,
            "amount_includes_fee": 0,
            "raw_json": gen.raw_json(txid=txid, chain="bitcoin"),
            "occurred_at": "2024-05-06T07:08:09Z",
        }
        outputs = [
            {
                "vout": vout,
                "wallet_id": gen.pick((row["wallet_id"], gen.ident("wallet"))),
                "amount_msat": gen.rng.randrange(1, 10**10) * 1000,
            }
            for vout in range(gen.rng.randrange(1, 4))
        ]
        proof = {
            "schema_version": 1,
            "amount_msat": gen.pick((row["amount"], sum(item["amount_msat"] for item in outputs), 0)),
            "outputs": outputs,
        }
        yield f"seeded-{index:03d}", {"row": row, "proof": proof}


def _reconciled_compute(item: dict[str, Any]) -> dict[str, Any]:
    row, proof = item["row"], item["proof"]
    canonical = custody_evidence.build_canonical_quantity_input(
        [row], receipt_reconciliations={str(row["id"]): proof}
    )
    observations = []
    for observation in canonical.observations:
        observed = custody_evidence.EvidenceSnapshot.from_transaction(row)
        observations.append(
            {
                "observed_quantity_hash": observed.quantity_hash,
                "quantity_hash": observation.quantity_hash,
                "quantity_preimage": _dumps(
                    {
                        "observed_quantity_hash": observed.quantity_hash,
                        "receipt_reconciliation": proof,
                    },
                    COMPACT_SORTED_UNICODE,
                ),
                "detail_hash": observation.evidence_detail_hash,
                "detail_preimage": observation.evidence_payload_json,
                "amount_msat": observation.amount_msat,
            }
        )
    return {
        "observations": observations,
        "rejected": [[issue.code, issue.message] for issue in canonical.rejected_events],
    }


def _event_keys(gen: Generator) -> list[custody_evidence.CanonicalEventKey]:
    keys = [
        custody_evidence.CanonicalEventKey("bitcoin", "main", "chain", "ab" * 32),
        custody_evidence.CanonicalEventKey("invalid", "invalid", "row", "row-3"),
        custody_evidence.CanonicalEventKey("lightning", "regtest", "lnd-node", "it's"),
        custody_evidence.CanonicalEventKey("liquid", "liquidv1", "chain", 'both \' and "'),
        custody_evidence.CanonicalEventKey("bitcoin", "test", "x\\y", "line\nbreak\ttab\x7f\u00a0"),
        custody_evidence.CanonicalEventKey("bitcoin", "signet", "\u94b1\u5305", "\U0001f600\u2028\ufeff\U0010ffff"),
    ]
    for _ in range(6):
        keys.append(
            custody_evidence.CanonicalEventKey(
                gen.pick(("bitcoin", "liquid", "lightning")),
                gen.pick(("main", "regtest", "liquidv1")),
                gen.text() or "ns",
                gen.text() or "id",
            )
        )
    return keys


def _issue_id_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    keys = _event_keys(gen)
    for index, key in enumerate(keys):
        yield f"canonical-{index:02d}", {"parts": ("canonical", key, gen.pick(("canonical_event_identity_invalid", "source_overlap_quantity_unresolved")))}
    for index in range(10):
        yield f"decision-{index:02d}", {
            "parts": ("decision", gen.hex64(), gen.rng.randrange(10**9), gen.pick(BIG_INTS), gen.pick(("custody_suspense", "conflicting")))
        }
    for index in range(10):
        yield f"single-{index:02d}", {
            "parts": (gen.pick(("component_compile", "gap_compile", "custody_gap_hold", "claim_bundle_invalid")), gen.text())
        }
    for index in range(10):
        yield f"blocked-{index:02d}", {
            "parts": ("custody_interpreter_blocked", gen.text(), gen.pick(("privacy_hop_unresolved", gen.text())), index)
        }
    yield "none-and-bool", {"parts": ("claim_bundle_invalid", None, True, 1.5)}
    yield "empty", {"parts": ()}


def _issue_id_preimage(parts: tuple[Any, ...]) -> str:
    return _dumps([str(value) for value in parts], {"separators": [",", ":"]})


def _issue_id_compute(item: dict[str, Any]) -> dict[str, Any]:
    parts = item["parts"]
    return {
        "issue_id": custody_quantity_runtime._issue_id(parts),
        "issue_preimage": _issue_id_preimage(parts),
    }


def _gap_id_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    yield "missing-profile", {"profile_id": "", "asset": "BTC", "source_ids": ("a",), "return_ids": ("b",)}
    yield "missing-returns", {"profile_id": "p", "asset": "BTC", "source_ids": ("a",), "return_ids": ()}
    yield "sorted-ids", {"profile_id": "p", "asset": "BTC", "source_ids": ("b", "a", "\u00e9", "B"), "return_ids": ("z", "10", "9")}
    for index in range(25):
        yield f"seeded-{index:03d}", {
            "profile_id": gen.ident("profile") or "p",
            "asset": gen.pick(("BTC", "LBTC", gen.text() or "X")),
            "source_ids": tuple(gen.ident("out") or "o" for _ in range(gen.rng.randrange(1, 4))),
            "return_ids": tuple(gen.ident("in") or "i" for _ in range(gen.rng.randrange(1, 5))),
        }


def _gap_id_compute(item: dict[str, Any]) -> dict[str, Any]:
    try:
        gap_id = custody_gaps.custody_gap_id(
            item["profile_id"], item["asset"], item["source_ids"], item["return_ids"]
        )
    except ValueError as exc:
        return {"error": _error(exc)}
    preimage = "\x1f".join(
        (
            item["profile_id"],
            item["asset"],
            *sorted(item["source_ids"]),
            "->",
            *sorted(item["return_ids"]),
        )
    )
    return {"gap_id": gap_id, "gap_preimage": preimage}


def _gap_candidate(gen: Generator) -> custody_gaps.CustodyGapCandidate:
    source_ids = tuple(gen.ident("out") for _ in range(gen.rng.randrange(1, 3)))
    return_ids = tuple(gen.ident("in") for _ in range(gen.rng.randrange(1, 4)))
    return custody_gaps.CustodyGapCandidate(
        gap_id="cg_" + gen.hex64()[:24],
        profile_id=gen.ident("profile"),
        asset=gen.pick(("BTC", "LBTC")),
        protocol_chain=gen.pick(("bitcoin", "liquid")),
        network=gen.pick(("main", "liquidv1", "regtest")),
        source_ids=source_ids,
        return_ids=return_ids,
        source_wallet_ids=tuple(gen.ident("wallet") for _ in source_ids),
        destination_wallet_ids=tuple(gen.ident("wallet") for _ in return_ids),
        source_wallet_labels=tuple(gen.text() for _ in source_ids),
        destination_wallet_labels=tuple(gen.text() for _ in return_ids),
        source_total_msat=gen.pick(BIG_INTS[:10]),
        source_fee_msat=gen.rng.randrange(10**6),
        source_debit_msat=gen.rng.randrange(10**12),
        return_total_msat=gen.rng.randrange(10**12),
        retained_msat=gen.rng.randrange(10**12),
        residual_msat=gen.rng.randrange(10**9),
        excess_msat=gen.rng.randrange(10**9),
        coverage_ppm=gen.rng.randrange(1_000_001),
        started_at="2021-06-01T00:00:00Z",
        ended_at=gen.pick(("2021-06-02T00:00:00Z", "")),
        elapsed_seconds=gen.rng.randrange(10**7),
        score=gen.rng.randrange(-100, 1000),
        confidence=gen.pick(("high", "medium", "low")),
        reason_codes=tuple(gen.pick(("amount_close", "same_era", "\u00e9")) for _ in range(gen.rng.randrange(4))),
        promotion_eligible=gen.chance(1, 2),
        competitor_score_margin=gen.pick((None, 0, 75, -3)),
        conflict_set_id=gen.pick(("", "set-1", gen.text())),
        conflict_size=gen.rng.randrange(1, 5),
    )


def _gap_fingerprint_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(30):
        yield f"seeded-{index:03d}", {"candidate": _gap_candidate(gen)}


_CANDIDATE_FINGERPRINT_FIELDS = (
    "gap_id", "profile_id", "asset", "protocol_chain", "network", "source_ids",
    "return_ids", "source_wallet_ids", "destination_wallet_ids",
    "source_total_msat", "source_fee_msat", "source_debit_msat",
    "return_total_msat", "retained_msat", "residual_msat", "excess_msat",
    "started_at", "ended_at",
)


def _gap_fingerprint_compute(item: dict[str, Any]) -> dict[str, Any]:
    candidate = item["candidate"]
    authored = {"schema_version": 1}
    authored.update({name: getattr(candidate, name) for name in _CANDIDATE_FINGERPRINT_FIELDS})
    full = {"schema_version": 2}
    full.update({name: getattr(candidate, name) for name in _CANDIDATE_FINGERPRINT_FIELDS})
    full.update(
        promotion_eligible=candidate.promotion_eligible,
        competitor_score_margin=candidate.competitor_score_margin,
        conflict_set_id=candidate.conflict_set_id,
        conflict_size=candidate.conflict_size,
        reason_codes=candidate.reason_codes,
    )
    return {
        "candidate_fingerprint": custody_gap_reviews.candidate_fingerprint(candidate),
        "candidate_preimage": _dumps(full, COMPACT_SORTED),
        "authored_claim_fingerprint": custody_gap_reviews.authored_claim_fingerprint(candidate),
        "authored_claim_preimage": _dumps(authored, COMPACT_SORTED),
    }


class _CommitmentRows:
    """The one query `_correction_fingerprint` runs, answered from the vector."""

    def __init__(self, rows: list[dict[str, Any]]) -> None:
        self._rows = rows

    def execute(self, _sql: str, _params: Any) -> "_CommitmentRows":
        return self

    def fetchall(self) -> list[dict[str, Any]]:
        return self._rows


def _uuid5_name(kind: str, item: dict[str, Any]) -> str:
    if kind == "gap_bridge":
        return f"kassiber:custody-gap-bridge:{item['profile_id']}:{item['gap_id']}:{item['fingerprint']}"
    if kind == "gap_revision":
        return f"kassiber:custody-gap-revision:{item['fingerprint']}"
    if kind == "gap_residual":
        return f"kassiber:custody-gap-residual:{item['fingerprint']}"
    if kind == "planned_wallet":
        return f"kassiber:planned-untracked-wallet:{item['profile_id']}:{item['label'].casefold()}"
    if kind == "deterministic":
        return f"kassiber:custody-component:{item['component_id']}:{item['kind']}:{item['ordinal']}"
    if kind == "bulk":
        return (
            "kassiber:custody-bulk:"
            f"{item['profile_id']}:{item['authored_source']}:{int(item['activate'])}:"
            f"{item['index']}:{_dumps(item['spec'], {'sort_keys': True, 'separators': [',', ':']})}"
        )
    if kind == "revision":
        return (
            "kassiber:custody-revision:"
            f"{item['profile_id']}:{item['component_id']}:{item['action']}:{item['next_revision']}:"
            f"{item['authored_source']}:{int(item['activate'])}:{item['reason'] or ''}:"
            f"{_dumps(item['spec'], {'sort_keys': True, 'separators': [',', ':']})}"
        )
    if kind == "filed_impact":
        return f"kassiber:custody-filed-impact:{item['filed_report_snapshot_id']}:{item['review_id']}"
    if kind == "filed_resolution":
        return f"kassiber:custody-filed-impact-resolution:{item['impact_id']}"
    raise KeyError(kind)


def _spec(gen: Generator) -> dict[str, Any]:
    return {
        "component_type": gen.pick(("manual_bridge", "swap")),
        "legs": [
            {"id": gen.ident("leg"), "role": gen.pick(("source", "destination")), "amount_msat": gen.pick(BIG_INTS[:9])}
            for _ in range(gen.rng.randrange(1, 3))
        ],
        "notes": gen.text(),
        "ratio": gen.pick(FLOATS[:19]),
        "nested": {"\u00e9": [1, 2.5, None], "B": True},
    }


def _gap_review_id_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(8):
        yield f"bridge-{index:02d}", {"kind": "gap_bridge", "profile_id": gen.ident("profile"), "gap_id": "cg_" + gen.hex64()[:24], "fingerprint": gen.hex64()}
        yield f"revision-{index:02d}", {"kind": "gap_revision", "fingerprint": gen.pick((gen.hex64(), gen.text()))}
        yield f"residual-{index:02d}", {"kind": "gap_residual", "fingerprint": gen.pick((gen.hex64(), gen.text()))}
    for index in range(6):
        legs = [{"id": gen.pick((gen.ident("leg"), None)), "role": "source"} for _ in range(gen.rng.randrange(1, 4))]
        leg_keys = [str(leg["id"] or f"leg:{ordinal}") for ordinal, leg in enumerate(legs)]
        allocations = [
            {"source_leg_id": gen.pick(leg_keys), "sink_leg_id": gen.pick(leg_keys)}
            for _ in range(gen.rng.randrange(3))
        ]
        yield f"plan-rows-{index:02d}", {
            "kind": "plan_rows",
            "component_id": gen.pick((str(uuid.UUID(int=gen.rng.getrandbits(128))), gen.text())),
            "spec": {"component_type": "manual_bridge", "legs": legs, "allocations": allocations},
        }
    for index in range(8):
        commitments = [
            {"ordinal": ordinal, "quantity_hash": gen.hex64(), "detail_hash": gen.hex64()}
            for ordinal in range(gen.rng.randrange(3))
        ]
        yield f"correction-{index:02d}", {
            "kind": "correction_fingerprint",
            "context": {
                "review": {
                    "gap_id": "cg_" + gen.hex64()[:24],
                    "id": gen.ident("review"),
                    "revision": gen.rng.randrange(1, 9),
                    **({"event_kind": gen.pick(("bridge_reopened", "review_decision"))} if gen.chance(1, 2) else {}),
                },
                "component": {
                    "id": gen.ident("component"),
                    "lineage_id": gen.ident("lineage"),
                    "revision": gen.rng.randrange(1, 9),
                },
            },
            "commitments": commitments,
            "operation": gen.pick(("bridge_revised", "bridge_reopened", "residual_classified")),
            "details": {"reason": gen.text(), "classification": gen.pick(("network_fee", "retained_custody"))},
        }


def _gap_review_id_compute(item: dict[str, Any]) -> dict[str, Any]:
    kind = item["kind"]
    if kind == "plan_rows":
        planned = custody_gap_reviews._stable_plan_rows(item["spec"], item["component_id"])
        return {
            "leg_ids": [leg["id"] for leg in planned["legs"]],
            "leg_names": [
                f"kassiber:custody-plan:{item['component_id']}:leg:{ordinal}"
                for ordinal in range(len(planned["legs"]))
            ],
            "allocation_ids": [row["id"] for row in planned["allocations"]],
            "allocation_names": [
                f"kassiber:custody-plan:{item['component_id']}:allocation:{ordinal}"
                for ordinal in range(len(planned["allocations"]))
            ],
            "allocation_endpoints": [
                [row["source_leg_id"], row["sink_leg_id"]] for row in planned["allocations"]
            ],
        }
    if kind == "correction_fingerprint":
        context = item["context"]
        fingerprint = custody_gap_reviews._correction_fingerprint(
            _CommitmentRows(item["commitments"]),
            context,
            operation=item["operation"],
            details=item["details"],
        )
        payload = {
            "schema_version": 1,
            "operation": item["operation"],
            "gap_id": context["review"]["gap_id"],
            "review_id": context["review"]["id"],
            "review_revision": int(context["review"]["revision"]),
            "review_event_kind": str(context["review"].get("event_kind") or "review_decision"),
            "component_id": context["component"]["id"],
            "component_lineage_id": context["component"]["lineage_id"],
            "component_revision": int(context["component"]["revision"]),
            "evidence_commitments": item["commitments"],
            "details": dict(item["details"]),
        }
        return {"fingerprint": fingerprint, "fingerprint_preimage": _dumps(payload, COMPACT_SORTED)}
    function = {
        "gap_bridge": lambda: custody_gap_reviews._gap_bridge_component_id(
            item["profile_id"], item["gap_id"], item["fingerprint"]
        ),
        "gap_revision": lambda: custody_gap_reviews._gap_revision_component_id(item["fingerprint"]),
        "gap_residual": lambda: custody_gap_reviews._gap_residual_component_id(item["fingerprint"]),
    }[kind]
    return {"id": function(), "name": _uuid5_name(kind, item)}


def _planner_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index, label in enumerate(TEXTS):
        yield f"planned-wallet-{index:02d}", {"kind": "planned_wallet", "profile_id": gen.ident("profile"), "label": label}
    for index in range(6):
        yield f"deterministic-{index:02d}", {"kind": "deterministic", "component_id": gen.ident("component"), "kind_name": gen.pick(("leg", "allocation", "term")), "ordinal": gen.pick((0, 1, 17, 2**40))}
    for index in range(8):
        yield f"bulk-{index:02d}", {
            "kind": "bulk",
            "profile_id": gen.ident("profile"),
            "authored_source": gen.pick(("cli", "desktop", "ai")),
            "activate": gen.chance(1, 2),
            "index": gen.rng.randrange(50),
            "spec": _spec(gen),
        }
    for index in range(8):
        yield f"revision-{index:02d}", {
            "kind": "revision",
            "profile_id": gen.ident("profile"),
            "component_id": gen.ident("component"),
            "action": gen.pick(("revise", "supersede", "undo")),
            "next_revision": gen.rng.randrange(2, 20),
            "authored_source": gen.pick(("cli", "desktop")),
            "activate": gen.chance(1, 2),
            "reason": gen.pick((None, "", gen.text())),
            "spec": _spec(gen),
        }
    yield "spec-not-json-safe", {"kind": "bulk", "profile_id": "p", "authored_source": "cli", "activate": False, "index": 0, "spec": {"nan": float("nan"), "bytes": b"x"}}


def _planner_compute(item: dict[str, Any]) -> dict[str, Any]:
    kind = item["kind"]
    if kind == "planned_wallet":
        return {
            "id": custody_component_planner._planned_wallet_id(item["profile_id"], item["label"]),
            "name": _uuid5_name(kind, item),
        }
    if kind == "deterministic":
        named = {**item, "kind": item["kind_name"]}
        return {
            "id": custody_component_planner._deterministic_id(
                item["component_id"], item["kind_name"], item["ordinal"]
            ),
            "name": _uuid5_name(kind, named),
        }
    try:
        canonical = custody_component_planner._canonical_spec_json(item["spec"])
    except (TypeError, ValueError) as exc:
        return {"error": _error(exc)}
    if kind == "bulk":
        component_id = custody_component_planner._bulk_component_id(
            item["profile_id"], item["authored_source"], item["activate"], item["index"], canonical
        )
    else:
        component_id = custody_component_planner._revision_component_id(
            item["profile_id"],
            item["component_id"],
            item["action"],
            item["next_revision"],
            item["authored_source"],
            item["activate"],
            item["reason"],
            canonical,
        )
    return {"canonical_spec": canonical, "id": component_id, "name": _uuid5_name(kind, item)}


def _filed_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(6):
        yield f"impact-{index:02d}", {"kind": "filed_impact", "filed_report_snapshot_id": gen.ident("snapshot"), "review_id": gen.ident("review")}
        yield f"resolution-{index:02d}", {"kind": "filed_resolution", "impact_id": gen.ident("impact")}
    yield "artifact-single", {"kind": "artifact", "files": [{"name": "report.pdf", "content": b"%PDF-1.4\n"}]}
    yield "artifact-empty-file", {"kind": "artifact", "files": [{"name": "empty.csv", "content": b""}]}
    for index in range(4):
        yield f"artifact-bundle-{index:02d}", {
            "kind": "artifact",
            "files": [
                {"name": name, "content": bytes(gen.rng.randrange(256) for _ in range(gen.rng.randrange(64)))}
                for name in gen.pick((("b.csv", "a.xlsx"), ("E1kv.pdf", "e1kv.csv", "\u00e9.xlsx"), ("z", "y", "x", "w")))
            ],
        }
    yield "artifact-duplicate-names", {"kind": "artifact", "files": [{"name": "a", "content": b"1"}, {"name": "a", "content": b"2"}]}


def _filed_compute(item: dict[str, Any]) -> dict[str, Any]:
    kind = item["kind"]
    if kind == "filed_impact":
        return {
            "id": custody_filed_reports.custody_filed_impact_id(
                item["filed_report_snapshot_id"], item["review_id"]
            ),
            "name": _uuid5_name(kind, item),
        }
    if kind == "filed_resolution":
        return {
            "id": custody_filed_reports.custody_filed_impact_resolution_id(item["impact_id"]),
            "name": _uuid5_name(kind, item),
        }
    with tempfile.TemporaryDirectory() as tmp:
        paths = []
        for index, entry in enumerate(item["files"]):
            folder = Path(tmp) / str(index)
            folder.mkdir()
            path = folder / entry["name"]
            path.write_bytes(entry["content"])
            paths.append(path)
        try:
            digest = custody_filed_reports.artifact_content_sha256(paths)
        except Exception as exc:  # the exact failure is part of the contract
            return {"error": _error(exc)}
    output: dict[str, Any] = {"sha256": digest}
    if len(item["files"]) > 1:
        manifest = [
            {
                "name": entry["name"],
                "bytes": len(entry["content"]),
                "sha256": hashlib.sha256(entry["content"]).hexdigest(),
            }
            for entry in sorted(item["files"], key=lambda entry: entry["name"])
        ]
        output["sha256_preimage"] = _dumps(manifest, COMPACT_SORTED_UNICODE)
    return output


def _native_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(10):
        yield f"pair-{index:02d}", {
            "kind": "pair_id",
            "method": gen.pick(("payment_hash", "htlc_refund", gen.text())),
            "out_id": gen.ident("send"),
            "in_id": gen.ident("claim"),
        }
    for index in range(10):
        yield f"allocation-digest-{index:02d}", {
            "kind": "allocation_digest",
            "expected": [
                [gen.hex64(), gen.pick(("fee", gen.hex64(), gen.text())), gen.pick(BIG_INTS[:10])]
                for _ in range(gen.rng.randrange(4))
            ],
        }
    for index in range(10):
        yield f"audit-issue-{index:02d}", {
            "kind": "audit_hash",
            "value": {
                "schema_version": 1,
                "kind": "native_audit_issue",
                "reason": gen.pick(("native_audit_evidence_invalid", gen.text())),
                "transaction_ids": tuple(sorted({gen.ident("tx") for _ in range(gen.rng.randrange(3))})),
                "pairing_source": gen.pick(("", "channel_lifecycle", "ownership_graph")),
            },
        }
    for index in range(10):
        yield f"audit-evidence-{index:02d}", {
            "kind": "audit_hash",
            "value": {
                "schema_version": 1,
                "kind": gen.pick(("verified_native_intra_leg", "verified_channel_close_source")),
                "pairing_source": "ownership_graph",
                "source_quantity_hash": gen.hex64(),
                "out_id": gen.ident("out"),
                "in_id": gen.ident("in"),
                "target_wallet_id": gen.ident("wallet"),
                "asset": gen.pick(("BTC", "LBTC")),
                "received_msat": gen.pick(BIG_INTS[:10]),
                "fee_msat": gen.pick((None, 0, 1000)),
                "ratio": gen.pick(FLOATS[:19]),
            },
        }


def _native_compute(item: dict[str, Any]) -> dict[str, Any]:
    kind = item["kind"]
    if kind == "pair_id":
        pair_id = custody_native_transitions.native_transition_pair_id(
            item["method"], item["out_id"], item["in_id"]
        )
        return {
            "pair_id": pair_id,
            "pair_id_preimage": _dumps([item["method"], item["out_id"], item["in_id"]], {"separators": [",", ":"]}),
        }
    if kind == "allocation_digest":
        expected: dict[tuple[str, str], int] = {}
        for source, target, amount in item["expected"]:
            expected[(source, target)] = expected.get((source, target), 0) + amount
        return {
            "digest": custody_native_reconciliation.reviewed_allocation_digest(expected),
            "digest_preimage": _dumps(
                [[source, target, amount] for (source, target), amount in sorted(expected.items())],
                {"separators": [",", ":"]},
            ),
        }
    return {
        "hash": custody_native_audit._hash_payload(item["value"]),
        "hash_preimage": _dumps(item["value"], COMPACT_SORTED_UNICODE),
    }


def _store_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(12):
        yield f"commitment-{index:02d}", {"kind": "commitment_id", "component_id": gen.ident("component"), "ordinal": gen.pick((0, 1, 9, 2**53))}
    for index in range(14):
        yield f"decision-{index:02d}", {
            "kind": "decision_id",
            "args": [
                gen.hex64(), gen.rng.randrange(10**9), gen.pick(BIG_INTS[:10]),
                gen.hex64(), gen.rng.randrange(10**9), gen.pick(BIG_INTS[:10]),
                gen.pick(("internal_verified", "internal_reviewed")),
                gen.pick(("native_htlc_transition", "ownership_graph", gen.text())),
                gen.pick((None, "pair-group:x", gen.text())),
                gen.pick((None, gen.ident("component"))),
            ],
        }
    for index in range(14):
        yield f"relation-{index:02d}", {
            "kind": "relation_id",
            "args": [
                gen.pick(("conversion", "direct_payout")),
                gen.ident("out"),
                gen.pick((None, gen.ident("in"))),
                gen.pick((None, gen.ident("component"), 7)),
                gen.pick(("BTC", "LBTC")),
                gen.pick(("USDT", "BTC", None)),
                gen.pick(BIG_INTS[:10]),
                gen.rng.randrange(1, 10**12),
            ],
        }


def _store_compute(item: dict[str, Any]) -> dict[str, Any]:
    kind = item["kind"]
    if kind == "commitment_id":
        return {
            "id": custody_quantity_store.evidence_commitment_id(item["component_id"], item["ordinal"]),
            "id_preimage": _dumps(
                ["custody-component-evidence-v1", str(item["component_id"]), int(item["ordinal"])],
                COMPACT_ASCII,
            ),
        }
    if kind == "decision_id":
        return {
            "id": custody_quantity_store.custody_decision_id(*item["args"]),
            "id_preimage": _dumps(["canonical-custody-decision-v1", *item["args"]], COMPACT_ASCII),
        }
    return {
        "id": custody_quantity_store.economic_relation_id(*item["args"]),
        "id_preimage": _dumps(["canonical-custody-economic-relation-v3", *item["args"]], COMPACT_ASCII),
    }


def _tax_row_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(8):
        yield f"slice-{index:02d}", {"kind": "slice", "args": [gen.hex64(), gen.rng.randrange(10**9), gen.pick(BIG_INTS[:10]), gen.pick(("in", "out", "source", "target"))]}
        yield f"fee-{index:02d}", {"kind": "fee", "args": [gen.hex64(), gen.pick(("fee", "out")), gen.pick(BIG_INTS[:10])]}
        yield f"payout-{index:02d}", {"kind": "payout", "args": [gen.ident("payout"), gen.pick(("in", "out"))]}


def _tax_row_compute(item: dict[str, Any]) -> dict[str, Any]:
    function = {
        "slice": custody_tax_projection.tax_slice_row_id,
        "fee": custody_tax_projection.tax_fee_row_id,
        "payout": custody_tax_projection.direct_payout_row_id,
    }[item["kind"]]
    return {"id": function(*item["args"])}


def _migration_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    float_inputs = (*FLOATS, None, "", 0, 7, -3, "1.10", "1E+2", "-0", "0.000", Decimal("2.50"), "abc", 1e100, 100.0, 1 / 3)
    for index, value in enumerate(float_inputs):
        yield f"canonical-float-{index:02d}", {"kind": "canonical_float", "value": value}
    for index in range(12):
        row = {
            "id": gen.ident("pair"),
            "workspace_id": gen.ident("workspace"),
            "profile_id": gen.ident("profile"),
            "out_transaction_id": gen.ident("out"),
            "in_transaction_id": gen.ident("in"),
            "kind": gen.pick(("manual", "swap-refund", None)),
            "policy": gen.pick(("carrying-value", "taxable")),
            "notes": gen.pick((None, gen.text())),
            "swap_fee_msat": gen.pick((None, 0, 1500, 2**63 - 1)),
            "swap_fee_kind": gen.pick((None, "provider")),
            "confidence_at_pair": gen.pick((None, "exact", 0.75)),
            "pair_source": gen.pick(("manual", "auto")),
            "out_amount": gen.pick((None, gen.rng.randrange(10**12))),
            "deleted_at": None,
            "created_at": "2023-03-04T05:06:07Z",
        }
        yield f"pair-hash-{index:02d}", {"kind": "pair_hash", "row": row}
    for index in range(10):
        row = {
            "id": gen.ident("payout"),
            "workspace_id": "w",
            "profile_id": "p",
            "out_transaction_id": gen.ident("out"),
            "kind": "direct-swap-payout",
            "policy": gen.pick(("taxable", "carrying-value")),
            "payout_asset": gen.pick(("USDT", "EUR")),
            "payout_amount": gen.rng.randrange(10**12),
            "payout_occurred_at": gen.pick(("2023-01-01T00:00:00Z", "Mon, 01 Sep 2025 12:00:30 +0000")),
            "payout_fiat_value": gen.pick((0.1, 1234.5678, 1e-7, "99.990", None, 17, Decimal("3.000"), 1e22)),
            "payout_external_id": gen.pick((None, gen.text())),
            "counterparty": gen.pick((None, gen.text())),
            "notes": None,
            "swap_fee_msat": 0,
            "swap_fee_kind": None,
            "out_amount": None,
            "deleted_at": None,
            "created_at": "2023-03-04T05:06:07Z",
        }
        yield f"payout-hash-{index:02d}", {"kind": "payout_hash", "row": row}
    for index in range(10):
        yield f"stable-id-{index:02d}", {
            "kind": "stable_id",
            "parts": [gen.pick(("transaction_pair_group", "direct_swap_payout", "custody_authored_migration_issue")), *(gen.text() for _ in range(gen.rng.randrange(1, 4)))],
        }


def _migration_compute(item: dict[str, Any]) -> dict[str, Any]:
    kind = item["kind"]
    if kind == "canonical_float":
        try:
            return {"text": custody_authored_migration._canonical_float(item["value"])}
        except Exception as exc:  # the exact failure is part of the contract
            return {"error": _error(exc)}
    if kind == "stable_id":
        return {
            "id": custody_authored_migration._stable_id(*item["parts"]),
            "namespace": str(custody_authored_migration._MIGRATION_NAMESPACE),
            "name": "\x1f".join(item["parts"]),
        }
    fields_ = (
        custody_authored_migration._PAIR_HASH_FIELDS
        if kind == "pair_hash"
        else custody_authored_migration._PAYOUT_HASH_FIELDS
    )
    payload = custody_authored_migration._canonical_payload(item["row"], fields_)
    return {
        "hash": custody_authored_migration._hash_payload(payload),
        "hash_preimage": _dumps(payload, COMPACT_SORTED_UNICODE),
    }


def _fingerprint_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    yield "decimal-exponent", {"args": ["w", "x", "2024-01-01T00:00:00Z", "inbound", "BTC", Decimal("1E+2"), Decimal("0.00000000")]}
    yield "float-amounts", {"args": ["w", "x", "2024-01-01T00:00:00Z", "outbound", "BTC", 0.1, 1e-7]}
    yield "none-fields", {"args": [None, None, None, None, None, None, None]}
    for index in range(30):
        yield f"seeded-{index:03d}", {
            "args": [
                gen.ident("wallet"),
                gen.pick((gen.txid(), gen.text())),
                gen.pick(("2024-01-02T03:04:05Z", "2024-01-02", gen.text())),
                gen.pick(("inbound", "outbound")),
                gen.pick(("BTC", "LBTC", "\u20bf")),
                gen.pick((gen.rng.randrange(10**15), Decimal("0.00012345"), Decimal("-1.5"), "12", 2**70)),
                gen.pick((0, Decimal("0"), "0", gen.rng.randrange(10**6))),
            ]
        }


def _fingerprint_compute(item: dict[str, Any]) -> dict[str, Any]:
    wallet_id, external_id, occurred_at, direction, asset, amount, fee = item["args"]
    payload = {
        "wallet_id": wallet_id,
        "external_id": external_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": str(amount),
        "fee": str(fee),
    }
    return {
        "fingerprint": fingerprints.make_transaction_fingerprint(*item["args"]),
        "fingerprint_preimage": json.dumps(payload, sort_keys=True),
    }


def _ai_audit_cases(gen: Generator) -> Iterator[tuple[str, Any]]:
    for index in range(20):
        yield f"seeded-{index:03d}", {"value": gen.json_value(3)}
    yield "proposal", {"value": {"operations": [{"type": "kind_override", "transaction_id": "\u00e9", "kind": None}], "reason": "\U0001f600"}}


def _ai_audit_compute(item: dict[str, Any]) -> dict[str, Any]:
    value = item["value"]
    try:
        digest = custody_ai_audit._sha256_json(value)
    except Exception as exc:  # the exact failure is part of the contract
        return {"error": _error(exc)}
    return {"hash": digest, "hash_preimage": _dumps(value, COMPACT_SORTED_ASCII)}


def _hash_rule(payload: str, dumps: dict[str, Any] | None, **extra: Any) -> dict[str, Any]:
    rule: dict[str, Any] = {"payload": payload, "digest": SHA256_HEX}
    if dumps is not None:
        rule["json.dumps"] = dumps
    rule.update(extra)
    return rule


def _uuid5_rule(name: str, namespace: str = "uuid.NAMESPACE_URL") -> dict[str, Any]:
    return {"uuid5": {"namespace": namespace, "name": name}, "output": "str(uuid) lowercase hyphenated"}


FAMILIES: tuple[Family, ...] = (
    Family(
        "evidence_snapshot",
        "kassiber.core.custody_evidence.EvidenceSnapshot.from_transaction; "
        "canonical_event_key; custody_quantity_store._commitment_detail_hash",
        {
            "raw_json": (
                "custody_evidence._canonical_raw_json: None, '', '{}' and b'{}' -> {}; "
                "bytes decode as UTF-8 with errors='replace'; text -> json.loads, "
                "or {'unparsed_text': text} when it does not parse; other values unchanged"
            ),
            "quantity_hash": _hash_rule(
                "canonical_quantity_payload(row)", COMPACT_SORTED_UNICODE,
                preimage="quantity_preimage",
            ),
            "detail_hash": _hash_rule(
                "canonical_evidence_payload(row), stored verbatim as payload_json",
                COMPACT_SORTED_UNICODE,
                preimage="detail_preimage",
            ),
            "commitment_detail_hash": _hash_rule(
                "json.loads(payload_json) without fingerprint, occurred_at, confirmed_at, raw_json",
                COMPACT_SORTED_UNICODE,
                preimage="commitment_detail_preimage",
            ),
        },
        _evidence_cases,
        _evidence_compute,
    ),
    Family(
        "graph_hash",
        "kassiber.core.chain_observer.provenance.canonical_graph_hash",
        {
            "graph_hash": _hash_rule(
                "text -> json.loads(text or '{}'), or {'opaque_text': text} when it does not "
                "parse; any other value is hashed as given",
                {**COMPACT_SORTED_UNICODE, "default": "str"},
                preimage="graph_preimage",
            )
        },
        _graph_cases,
        _graph_compute,
    ),
    Family(
        "observed_quantity_hash",
        "kassiber.core.chain_observer.provenance.canonical_observed_quantity_hash",
        {
            "quantity_hash": _hash_rule(
                "{schema_version: 1, wallet_id, external_id stripped+lowercased, direction "
                "stripped+lowercased, asset via normalize_asset_code, amount_msat int, fee_msat int, "
                "amount_includes_fee bool}",
                {**COMPACT_SORTED, "ensure_ascii": True},
                preimage="quantity_preimage",
            )
        },
        _observed_quantity_cases,
        _observed_quantity_compute,
    ),
    Family(
        "reconciled_observation",
        "kassiber.core.custody_evidence.build_canonical_quantity_input(receipt_reconciliations=...)",
        {
            "quantity_hash": _hash_rule(
                "{observed_quantity_hash, receipt_reconciliation: proof}",
                COMPACT_SORTED_UNICODE,
                preimage="quantity_preimage",
            ),
            "detail_hash": _hash_rule(
                "json.loads(observation payload_json) plus receipt_reconciliation: proof",
                COMPACT_SORTED_UNICODE,
                preimage="detail_preimage",
            ),
        },
        _reconciled_cases,
        _reconciled_compute,
    ),
    Family(
        "quantity_issue_id",
        "kassiber.core.custody_quantity_runtime._issue_id",
        {
            "issue_id": _hash_rule(
                "[str(part) for part in parts]; a CanonicalEventKey part is its dataclass repr "
                "(Python str repr quoting and escapes)",
                {"separators": [",", ":"], "ensure_ascii": True},
                preimage="issue_preimage",
            )
        },
        _issue_id_cases,
        _issue_id_compute,
    ),
    Family(
        "custody_gap_id",
        "kassiber.core.custody_gaps.custody_gap_id",
        {
            "gap_id": {
                "payload": "'\\x1f'.join((profile_id, asset, *sorted(source_ids), '->', *sorted(return_ids)))",
                "digest": "'cg_' + sha256 over the UTF-8 bytes, lowercase hex, first 24 characters",
                "preimage": "gap_preimage",
            }
        },
        _gap_id_cases,
        _gap_id_compute,
    ),
    Family(
        "gap_fingerprints",
        "kassiber.core.custody_gap_reviews.candidate_fingerprint; authored_claim_fingerprint",
        {
            "candidate_fingerprint": _hash_rule(
                "schema_version 2 candidate payload (tuples as lists)", {**COMPACT_SORTED, "ensure_ascii": True},
                preimage="candidate_preimage",
            ),
            "authored_claim_fingerprint": _hash_rule(
                "schema_version 1 authored-claim payload (tuples as lists)", {**COMPACT_SORTED, "ensure_ascii": True},
                preimage="authored_claim_preimage",
            ),
        },
        _gap_fingerprint_cases,
        _gap_fingerprint_compute,
    ),
    Family(
        "gap_review_ids",
        "kassiber.core.custody_gap_reviews: _gap_bridge_component_id, _gap_revision_component_id, "
        "_gap_residual_component_id, _stable_plan_rows, _correction_fingerprint",
        {
            "gap_bridge": _uuid5_rule("kassiber:custody-gap-bridge:{profile_id}:{gap_id}:{fingerprint}"),
            "gap_revision": _uuid5_rule("kassiber:custody-gap-revision:{fingerprint}"),
            "gap_residual": _uuid5_rule("kassiber:custody-gap-residual:{fingerprint}"),
            "plan_rows": _uuid5_rule(
                "kassiber:custody-plan:{component_id}:leg:{ordinal} and "
                "kassiber:custody-plan:{component_id}:allocation:{ordinal}"
            ),
            "correction_fingerprint": _hash_rule(
                "schema_version 1 correction payload; evidence_commitments ordered by ordinal, id",
                {**COMPACT_SORTED, "ensure_ascii": True},
                preimage="fingerprint_preimage",
            ),
        },
        _gap_review_id_cases,
        _gap_review_id_compute,
    ),
    Family(
        "component_planner_ids",
        "kassiber.core.custody_component_planner: _planned_wallet_id, _deterministic_id, "
        "_canonical_spec_json, _bulk_component_id, _revision_component_id",
        {
            "planned_wallet": _uuid5_rule(
                "kassiber:planned-untracked-wallet:{profile_id}:{label.casefold()} (full Unicode casefold)"
            ),
            "deterministic": _uuid5_rule("kassiber:custody-component:{component_id}:{kind}:{ordinal}"),
            "canonical_spec": {"json.dumps": {**COMPACT_SORTED, "ensure_ascii": True}},
            "bulk": _uuid5_rule(
                "kassiber:custody-bulk:{profile_id}:{authored_source}:{int(activate)}:{index}:{canonical_spec}"
            ),
            "revision": _uuid5_rule(
                "kassiber:custody-revision:{profile_id}:{component_id}:{action}:{next_revision}:"
                "{authored_source}:{int(activate)}:{reason or ''}:{canonical_spec}"
            ),
        },
        _planner_cases,
        _planner_compute,
    ),
    Family(
        "filed_report_ids",
        "kassiber.core.custody_filed_reports: custody_filed_impact_id, "
        "custody_filed_impact_resolution_id, artifact_content_sha256",
        {
            "filed_impact": _uuid5_rule("kassiber:custody-filed-impact:{filed_report_snapshot_id}:{review_id}"),
            "filed_resolution": _uuid5_rule("kassiber:custody-filed-impact-resolution:{impact_id}"),
            "artifact": _hash_rule(
                "one file: sha256 of its bytes; several: the manifest [{name, bytes, sha256}] "
                "sorted by name",
                COMPACT_SORTED_UNICODE,
                preimage="sha256_preimage",
            ),
        },
        _filed_cases,
        _filed_compute,
    ),
    Family(
        "native_ids",
        "kassiber.core.custody_native_transitions.native_transition_pair_id; "
        "custody_native_reconciliation.reviewed_allocation_digest; custody_native_audit._hash_payload",
        {
            "pair_id": _hash_rule(
                "[method, out_id, in_id]", {"separators": [",", ":"]},
                preimage="pair_id_preimage", output="'native-htlc-' + digest",
            ),
            "allocation_digest": _hash_rule(
                "[[source_hash, sink, amount_msat], ...] sorted by (source_hash, sink)",
                {"separators": [",", ":"]},
                preimage="digest_preimage",
            ),
            "audit_hash": _hash_rule("issue, proof or evidence payload", COMPACT_SORTED_UNICODE, preimage="hash_preimage"),
        },
        _native_cases,
        _native_compute,
    ),
    Family(
        "quantity_store_ids",
        "kassiber.core.custody_quantity_store: evidence_commitment_id, custody_decision_id, "
        "economic_relation_id",
        {
            "commitment_id": _hash_rule(
                "['custody-component-evidence-v1', str(component_id), int(ordinal)]", COMPACT_ASCII,
                preimage="id_preimage",
            ),
            "decision_id": _hash_rule(
                "['canonical-custody-decision-v1', source hash, start, end, target hash, start, end, "
                "state, reason, atomic_bundle_id, component_id]",
                COMPACT_ASCII,
                preimage="id_preimage",
            ),
            "relation_id": _hash_rule(
                "['canonical-custody-economic-relation-v3', relation_kind, source id, target id, "
                "component_id, source asset, target asset, source msat, target msat]",
                COMPACT_ASCII,
                preimage="id_preimage",
            ),
        },
        _store_cases,
        _store_compute,
    ),
    Family(
        "tax_projection_row_ids",
        "kassiber.core.custody_tax_projection: tax_slice_row_id, tax_fee_row_id, direct_payout_row_id",
        {
            "slice": "f'custody-tax:{quantity_hash}:{start_msat}:{end_msat}:{side}'",
            "fee": "f'custody-tax:{quantity_hash}:{side}:{amount_msat}'",
            "payout": "f'direct-payout:{payout_id}:{side}'",
        },
        _tax_row_cases,
        _tax_row_compute,
    ),
    Family(
        "authored_migration_ids",
        "kassiber.core.custody_authored_migration: _canonical_float, _canonical_payload, "
        "_hash_payload, _stable_id",
        {
            "canonical_float": "None/'' -> None; float -> format(value, '.17g'); else str(Decimal(str(value)))",
            "pair_hash": _hash_rule(
                "_canonical_payload(row, _PAIR_HASH_FIELDS)", COMPACT_SORTED_UNICODE, preimage="hash_preimage"
            ),
            "payout_hash": _hash_rule(
                "_canonical_payload(row, _PAYOUT_HASH_FIELDS); payout_fiat_value via _canonical_float",
                COMPACT_SORTED_UNICODE,
                preimage="hash_preimage",
            ),
            "stable_id": _uuid5_rule("'\\x1f'.join(parts)", namespace="95ed148a-743a-4dcc-9b55-ca8cc203d547"),
        },
        _migration_cases,
        _migration_compute,
    ),
    Family(
        "transaction_fingerprint",
        "kassiber.fingerprints.make_transaction_fingerprint",
        {
            "fingerprint": _hash_rule(
                "{wallet_id, external_id, occurred_at, direction, asset, amount: str(amount), fee: str(fee)}",
                {"sort_keys": True},
                preimage="fingerprint_preimage",
                note="default separators (', ', ': ') and ensure_ascii=True",
            )
        },
        _fingerprint_cases,
        _fingerprint_compute,
    ),
    Family(
        "custody_ai_audit_hash",
        "kassiber.core.custody_ai_audit._sha256_json",
        {"hash": _hash_rule("the bounded audit value", COMPACT_SORTED_ASCII, preimage="hash_preimage")},
        _ai_audit_cases,
        _ai_audit_compute,
    ),
)

FAMILIES_BY_NAME = {family.name: family for family in FAMILIES}


# -- fixture I/O ------------------------------------------------------------


def vector_line(vector: dict[str, Any]) -> str:
    return json.dumps(vector, sort_keys=True, ensure_ascii=True, separators=(",", ":"), allow_nan=False)


def serialization_for(family: Family, raw_input: Any) -> dict[str, Any]:
    """The rules one vector's output follows: its own kind's, or all of them."""

    kind = raw_input.get("kind") if isinstance(raw_input, dict) else None
    if kind in family.serialization:
        return {kind: family.serialization[kind]}
    return family.serialization


def generate() -> dict[str, list[dict[str, Any]]]:
    """Build every family's vectors from the fixed seed."""

    generated: dict[str, list[dict[str, Any]]] = {}
    for offset, family in enumerate(FAMILIES):
        gen = Generator(SEED + offset)
        vectors = []
        seen: set[str] = set()
        for case, raw_input in family.cases(gen):
            if case in seen:
                raise ValueError(f"{family.name} repeats case {case}")
            seen.add(case)
            # Store what the fixture will hand back, so a check recomputes
            # from exactly the decoded input.
            encoded_input = encode(raw_input)
            output = family.compute(decode(encoded_input))
            vectors.append(
                {
                    "family": family.name,
                    "case": case,
                    "function": family.function,
                    "serialization": serialization_for(family, raw_input),
                    "input": encoded_input,
                    "output": encode(output),
                }
            )
        generated[family.name] = vectors
    return generated


def fixture_path(family: str) -> Path:
    return FIXTURE_DIR / f"{family}.jsonl"


def load_vectors(path: Path) -> list[dict[str, Any]]:
    return [
        json.loads(line)
        for line in path.read_text(encoding="ascii").splitlines()
        if line.strip()
    ]


def write_fixtures() -> dict[str, int]:
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    counts = {}
    for family, vectors in generate().items():
        text = "".join(vector_line(vector) + "\n" for vector in vectors)
        fixture_path(family).write_text(text, encoding="ascii")
        counts[family] = len(vectors)
    for stale in FIXTURE_DIR.glob("*.jsonl"):
        if stale.stem not in FAMILIES_BY_NAME:
            stale.unlink()
    return counts


def recompute(vector: dict[str, Any]) -> Any:
    """Recompute one stored vector's output with the product functions."""

    family = FAMILIES_BY_NAME[vector["family"]]
    return encode(family.compute(decode(vector["input"])))


def check_fixtures() -> list[str]:
    problems = []
    for family in FAMILIES:
        path = fixture_path(family.name)
        if not path.exists():
            problems.append(f"{family.name}: fixture missing")
            continue
        for vector in load_vectors(path):
            actual = recompute(vector)
            if actual != vector["output"]:
                problems.append(f"{family.name}/{vector['case']}: output differs")
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--write", action="store_true", help="regenerate the fixtures")
    args = parser.parse_args(argv)
    if args.write:
        counts = write_fixtures()
        for family, count in counts.items():
            print(f"{family}: {count}")
        print(f"total: {sum(counts.values())}")
        return 0
    problems = check_fixtures()
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
