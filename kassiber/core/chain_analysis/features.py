"""Public Bitcoin structure shared by observed transactions and PSBT proposals.

Extraction is deliberately lossy: scripts, stack elements, keys, preimages and
derivations never cross this boundary. A signature encoding is examined only at
a recognized spending-script position, not by searching arbitrary hex bytes.
Rules describe observable structure and conditional interpretations, never a
wallet vendor, owner or calibrated privacy score.
"""
from __future__ import annotations

from collections import Counter
from hashlib import new as hash_new, sha256
from typing import Any, Mapping, Sequence

from ..onchain import input_script, input_value_sats, output_script, output_value_sats

FEATURE_VERSION = 1
RULE_VERSION = 1
PERSISTED_FEATURE_KEY = "chain_analysis_features"
_ORDER = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141


def _bytes(value: Any) -> bytes | None:
    if isinstance(value, bytes):
        return value
    if isinstance(value, str) and len(value) <= 200_000:
        try:
            return bytes.fromhex(value)
        except ValueError:
            pass
    return None


def script_type(value: Any) -> str:
    script = _bytes(value)
    if script is None:
        return "unknown"
    if len(script) == 25 and script[:3] == b"\x76\xa9\x14" and script[-2:] == b"\x88\xac":
        return "p2pkh"
    if len(script) == 23 and script[:2] == b"\xa9\x14" and script[-1:] == b"\x87":
        return "p2sh"
    if len(script) == 22 and script[:2] == b"\x00\x14":
        return "p2wpkh"
    if len(script) == 34 and script[:2] == b"\x00\x20":
        return "p2wsh"
    if len(script) == 34 and script[:2] == b"\x51\x20":
        return "p2tr"
    if script.startswith(b"\x6a"):
        return "op_return"
    if len(script) in (35, 67) and script[0] == len(script) - 2 and script[-1] == 0xAC:
        return "p2pk"
    if 4 <= len(script) <= 42 and (script[0] == 0 or 0x51 <= script[0] <= 0x60) and script[1] == len(script) - 2:
        return "witness_unknown"
    return "other"


def signature_encoding(signature: Any, *, schnorr: bool = False) -> dict | None:
    """Return BIP66 encoding properties; this does not verify a signature."""
    value = _bytes(signature)
    if value is None:
        return None
    if schnorr:
        if len(value) not in (64, 65) or len(value) == 65 and value[-1] not in (1, 2, 3, 0x81, 0x82, 0x83):
            return None
        flag = value[-1] if len(value) == 65 else 0
        return {"algorithm": "schnorr", "sighash": flag, "low_r": None, "low_s": None, "verified": False}
    if not 9 <= len(value) <= 73 or value[:1] != b"\x30" or value[1] != len(value) - 3:
        return None
    length_r = value[3]
    if value[2] != 2 or not length_r or 5 + length_r >= len(value) or value[4 + length_r] != 2:
        return None
    length_s = value[5 + length_r]
    if not length_s or length_r + length_s + 7 != len(value):
        return None
    r, s = value[4:4 + length_r], value[6 + length_r:-1]
    for integer in (r, s):
        if integer[0] & 0x80 or len(integer) > 1 and integer[0] == 0 and not integer[1] & 0x80:
            return None
    r_value, s_value = int.from_bytes(r, "big"), int.from_bytes(s, "big")
    if not 0 < r_value < _ORDER or not 0 < s_value < _ORDER:
        return None
    return {"algorithm": "ecdsa", "sighash": value[-1], "low_r": r_value < 2 ** 255,
            "low_s": s_value <= _ORDER // 2, "verified": False}


def _pushes(script: bytes | None) -> list[bytes] | None:
    if script is None:
        return None
    result, position = [], 0
    while position < len(script):
        opcode = script[position]
        position += 1
        if opcode <= 75:
            size = opcode
        elif opcode in (76, 77, 78):
            width = {76: 1, 77: 2, 78: 4}[opcode]
            if position + width > len(script):
                return None
            size = int.from_bytes(script[position:position + width], "little")
            position += width
        else:
            return None
        if position + size > len(script):
            return None
        result.append(script[position:position + size])
        position += size
    return result


def _hash160(value: bytes) -> bytes:
    return hash_new("ripemd160", sha256(value).digest()).digest()


def _multisig(script: bytes) -> bool:
    if len(script) < 4 or not 0x51 <= script[0] <= 0x60 or not 0x51 <= script[-2] <= 0x60 or script[-1] != 0xAE:
        return False
    keys = _pushes(script[1:-2])
    return bool(keys and len(keys) == script[-2] - 0x50 and script[0] <= script[-2]
                and all(len(key) in (33, 65) and key[0] in (2, 3, 4) for key in keys))


def _input_signatures(entry: Mapping) -> list[dict]:
    script = _bytes(input_script(entry))
    kind = script_type(script)
    script_sig = entry.get("scriptsig", entry.get("scriptSig"))
    if isinstance(script_sig, Mapping):
        script_sig = script_sig.get("hex")
    pushes = _pushes(_bytes(script_sig))
    raw_witness = entry.get("witness", entry.get("txinwitness"))
    witness = [_bytes(value) for value in raw_witness] if isinstance(raw_witness, list) else None
    if witness is not None and any(value is None for value in witness):
        witness = None
    candidates: Sequence = []
    schnorr = False
    if kind == "p2sh" and pushes:
        redeem = pushes[-1]
        if _hash160(redeem) != script[2:22]:
            return []
        nested_kind = script_type(redeem)
        if nested_kind in {"p2wpkh", "p2wsh"} and len(pushes) == 1:
            script, kind = redeem, nested_kind
        elif _multisig(redeem) and pushes[0] == b"":
            candidates = pushes[1:-1]
    if kind == "p2wpkh" and witness and len(witness) == 2 and _hash160(witness[1]) == script[2:]:
        candidates = witness[:1]
    elif kind == "p2wsh" and witness and len(witness) >= 3 and witness[0] == b"" and sha256(witness[-1]).digest() == script[2:] and _multisig(witness[-1]):
        candidates = witness[1:-1]
    elif kind == "p2pkh" and pushes and len(pushes) == 2 and _hash160(pushes[1]) == script[3:23]:
        candidates = pushes[:1]
    elif kind == "p2pk" and pushes and len(pushes) == 1:
        candidates = pushes
    elif kind == "p2tr" and witness:
        # BIP341 annex is removed only at its specified final stack position.
        stack = witness[:-1] if len(witness) >= 2 and witness[-1][:1] == b"\x50" else witness
        if len(stack) == 1:
            candidates, schnorr = stack, True
    return [value for candidate in candidates if (value := signature_encoding(candidate, schnorr=schnorr)) is not None]


def _integer(value: Any, maximum: int = 0xFFFFFFFF) -> int | None:
    return value if isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= maximum else None


def extract_transaction_features(raw: Mapping[str, Any], *, source: str = "stored_transaction",
                                 subject_id: str | None = None,
                                 signature_observations: Sequence[Mapping] = ()) -> dict:
    """Extract a persistable snapshot before the caller discards raw witnesses.

    ``signature_observations`` is the PSBT adapter's already categorized partial
    signatures. It is never read from imported transaction metadata.
    """
    features = []

    def add(code, value, *, available=True, partial=False, assumptions=()):
        features.append({"code": code, "value": value,
                         "availability": "partial" if available and partial else "observed" if available else "unavailable",
                         "source": source, "confidence": "observed" if available else "unknown",
                         "evidence": [{"source": source, "subject_id": subject_id, "field": code}],
                         "assumptions": list(assumptions)})

    vin, vout = raw.get("vin"), raw.get("vout")
    inputs = [row for row in vin if isinstance(row, Mapping)] if isinstance(vin, list) else []
    outputs = [row for row in vout if isinstance(row, Mapping)] if isinstance(vout, list) else []
    input_complete = isinstance(vin, list) and len(inputs) == len(vin)
    output_complete = isinstance(vout, list) and len(outputs) == len(vout)
    version, locktime = _integer(raw.get("version")), _integer(raw.get("locktime"))
    # Bitcoin's transaction version is a signed int32. Parser wire representations
    # may expose its unsigned bit pattern; negative versions do not activate BIP68.
    if version is not None and version >= 2 ** 31:
        version -= 2 ** 32
    if isinstance(raw.get("version"), int) and not isinstance(raw.get("version"), bool) and -(2 ** 31) <= raw["version"] < 0:
        version = raw["version"]
    add("transaction_version", version, available=version is not None)
    add("cardinality", {"inputs": len(inputs), "outputs": len(outputs)}, available=input_complete and output_complete)
    sequences = [_integer(row.get("sequence")) for row in inputs]
    sequence_complete = bool(inputs) and input_complete and all(value is not None for value in sequences)
    rbf = any(value is not None and value < 0xFFFFFFFE for value in sequences)
    relative = [{"input_index": i, "unit": "seconds" if value & (1 << 22) else "blocks",
                 "minimum": (value & 0xFFFF) * (512 if value & (1 << 22) else 1)}
                for i, value in enumerate(sequences) if value is not None and version is not None and version >= 2 and not value & (1 << 31)]
    add("sequences", {"values": sequences, "signals_bip125": rbf if rbf or sequence_complete else None,
                      "relative_locks": relative, "relative_lock_interpretation_available": version is not None},
        available=any(value is not None for value in sequences), partial=not sequence_complete,
        assumptions=("bip125_signal_is_not_mempool_acceptance_or_replaceability", "relative_lock_maturity_requires_prevout_chain_context"))
    enabled = any(value is not None and value != 0xFFFFFFFF for value in sequences)
    add("absolute_locktime", {"value": locktime, "kind": "none" if locktime == 0 else "height" if locktime is not None and locktime < 500_000_000 else "time" if locktime is not None else "unknown",
                             "enabled": enabled if enabled or sequence_complete else None}, available=locktime is not None,
        assumptions=("locktime_maturity_requires_chain_tip_and_median_time_past",))
    for side, rows, getter, complete in (("input", inputs, input_script, input_complete), ("output", outputs, output_script, output_complete)):
        kinds = [script_type(getter(row)) for row in rows]
        known = [kind for kind in kinds if kind != "unknown"]
        add(f"{side}_script_types", {"counts": dict(sorted(Counter(kinds).items())), "by_index": kinds},
            available=bool(known), partial=not complete or len(known) != len(rows))
    signatures = [dict(value, input_index=i) for i, row in enumerate(inputs) for value in _input_signatures(row)]
    # Only a narrow categorical shape can enter via the PSBT adapter.
    for row in signature_observations:
        if row.get("algorithm") in {"ecdsa", "schnorr"} and _integer(row.get("sighash"), 255) is not None:
            signatures.append({key: row.get(key) for key in ("algorithm", "sighash", "low_r", "low_s", "input_index", "verified")})
    add("signature_encodings", {"observations": signatures, "count": len(signatures)}, available=bool(signatures),
        assumptions=("encoding_observation_does_not_verify_signature_or_identify_wallet", "unrecognized_script_positions_are_not_scanned"))
    shapes = [len(value) if isinstance(value := row.get("witness", row.get("txinwitness")), list) else None for row in inputs]
    add("witness_shapes", shapes, available=any(value is not None for value in shapes), partial=any(value is None for value in shapes))
    values = [output_value_sats(row) for row in outputs]
    known_values = [value for value in values if value is not None and value >= 0]
    equal_groups = [{"amount_msat": str(value * 1000), "count": count} for value, count in sorted(Counter(known_values).items()) if value > 0 and count >= 2]
    add("output_value_pattern", {"equal_groups": equal_groups, "round_1000_sat_count": sum(value > 0 and value % 1000 == 0 for value in known_values)},
        available=bool(known_values), partial=not output_complete or len(known_values) != len(outputs))
    scripts = [_bytes(output_script(row)) for row in outputs]
    ordering_available = len(outputs) >= 2 and output_complete and len(known_values) == len(outputs) and all(value is not None for value in scripts)
    ordering = list(zip(values, scripts)) if ordering_available else []
    add("output_ordering", {"bip69_value_script_order": ordering == sorted(ordering) if ordering_available else None}, available=ordering_available,
        assumptions=("matching_sort_order_can_occur_by_chance",))
    input_values = [input_value_sats(row) for row in inputs]
    vsize = _integer(raw.get("vsize"))
    fee_available = bool(inputs) and bool(outputs) and input_complete and output_complete and all(value is not None for value in (*input_values, *values))
    fee_sats = sum(input_values) - sum(values) if fee_available else None
    fee_available = fee_available and fee_sats >= 0 and vsize is not None and vsize > 0
    add("fee_rate", {"fee_sats": fee_sats if fee_available else None, "vsize": vsize if fee_available else None},
        available=fee_available, assumptions=("observed_transaction_values_only", "rounded_rate_does_not_identify_wallet"))
    return {"schema_version": 1, "extractor_version": FEATURE_VERSION, "source": source,
            "subject_id": subject_id, "features": features}


def normalize_persisted_features(value: Any, *, subject_id: str | None, source: str) -> dict | None:
    """Rebind a closed snapshot to trusted context; reject arbitrary cache data.

    A versioned cache is evidence supplied by an observer, not authentication.
    This function prevents imported raw_json fields from becoming a free-text
    or secret-bearing channel through a supposedly public feature snapshot.
    """
    if not isinstance(value, Mapping) or value.get("schema_version") != 1 or value.get("extractor_version") != FEATURE_VERSION:
        return None
    rows = value.get("features")
    if not isinstance(rows, list) or len(rows) > 20:
        return None
    script_types = {"p2pkh", "p2sh", "p2wpkh", "p2wsh", "p2tr", "op_return", "p2pk", "witness_unknown", "other", "unknown"}

    def number(v, limit=0xFFFFFFFF):
        return _integer(v, limit) is not None

    def nullable_number(v, limit=0xFFFFFFFF):
        return v is None or number(v, limit)

    def boolean(v):
        return v is None or isinstance(v, bool)

    def exact_dict(v, fields):
        return isinstance(v, dict) and set(v) == set(fields) and all(test(v[key]) for key, test in fields.items())

    def items(v, predicate, limit=4096):
        return isinstance(v, list) and len(v) <= limit and all(predicate(item) for item in v)

    def amount(v):
        return isinstance(v, str) and len(v) <= 19 and v.isascii() and v.isdigit() and int(v) <= 2_100_000_000_000_000_000 and int(v) % 1000 == 0

    validators = {
        "transaction_version": lambda v: v is None or isinstance(v, int) and not isinstance(v, bool) and -(2 ** 31) <= v < 2 ** 31,
        "cardinality": lambda v: exact_dict(v, {"inputs": lambda n: number(n, 4096), "outputs": lambda n: number(n, 4096)}),
        "sequences": lambda v: exact_dict(v, {"values": lambda s: items(s, nullable_number), "signals_bip125": boolean,
                         "relative_lock_interpretation_available": lambda b: isinstance(b, bool),
                         "relative_locks": lambda s: items(s, lambda r: exact_dict(r, {"input_index": lambda n: number(n, 4095), "unit": lambda u: u in ("seconds", "blocks"), "minimum": lambda n: number(n, 65535 * 512)}))}),
        "absolute_locktime": lambda v: exact_dict(v, {"value": nullable_number, "kind": lambda k: k in ("none", "height", "time", "unknown"), "enabled": boolean}),
        "signature_encodings": lambda v: exact_dict(v, {"count": lambda n: number(n, 16384),
                "observations": lambda s: items(s, lambda r: exact_dict(r, {"algorithm": lambda a: a in ("ecdsa", "schnorr"),
                "sighash": lambda n: number(n, 255), "low_r": boolean, "low_s": boolean, "input_index": lambda n: number(n, 4095), "verified": lambda b: b is False}), 16384)}),
        "witness_shapes": lambda v: items(v, lambda n: nullable_number(n, 100_000)),
        "output_value_pattern": lambda v: exact_dict(v, {"equal_groups": lambda s: items(s, lambda r: exact_dict(r, {"amount_msat": amount, "count": lambda n: number(n, 4096) and n >= 2})), "round_1000_sat_count": lambda n: number(n, 4096)}),
        "output_ordering": lambda v: exact_dict(v, {"bip69_value_script_order": boolean}),
        "fee_rate": lambda v: exact_dict(v, {"fee_sats": lambda n: nullable_number(n, 2_100_000_000_000_000), "vsize": nullable_number}),
    }
    for code in ("input_script_types", "output_script_types"):
        validators[code] = lambda v: exact_dict(v, {"counts": lambda d: isinstance(d, dict) and len(d) <= len(script_types) and all(k in script_types and number(n, 4096) for k, n in d.items()), "by_index": lambda s: items(s, lambda k: isinstance(k, str) and k in script_types)})
    # Trusted template supplies categorical source, evidence and assumptions.
    template = extract_transaction_features({}, source=source, subject_id=subject_id)
    known = {row["code"]: row for row in template["features"]}
    result, seen = [], set()
    for row in rows:
        if not isinstance(row, Mapping) or not isinstance(row.get("code"), str):
            return None
        code = row["code"]
        if code not in validators or code in seen or row.get("availability") not in {"observed", "partial", "unavailable"}:
            return None
        try:
            valid = validators[code](row.get("value"))
        except (TypeError, ValueError, OverflowError):
            valid = False
        if not valid:
            return None
        seen.add(code)
        result.append(dict(known[code], value=row["value"], availability=row["availability"], confidence="unknown" if row["availability"] == "unavailable" else "observed"))
    return dict(template, features=result)


def feature_raw_from_transaction(transaction: Any, prevouts: Sequence[Mapping | None] | None = None) -> dict:
    """Transient embit adapter; callers must discard this result after extraction."""
    supplied = prevouts or ()
    return {"version": transaction.version, "locktime": transaction.locktime,
            "vin": [{"txid": row.txid.hex(), "vout": row.vout, "sequence": row.sequence,
                     "scriptsig": row.script_sig.data.hex(), "witness": [item.hex() for item in row.witness.items],
                     "prevout": dict(supplied[i]) if i < len(supplied) and isinstance(supplied[i], Mapping) else {}}
                    for i, row in enumerate(transaction.vin)],
            "vout": [{"value_sats": row.value, "scriptpubkey": row.script_pubkey.data.hex()} for row in transaction.vout]}


def evaluate_features(snapshot: Mapping, *, collaboration: Mapping | None = None) -> list[dict]:
    """Evaluate evidence-bound rules once; suppress incompatible interpretations."""
    observed = {row["code"]: row for row in snapshot.get("features", []) if isinstance(row, Mapping) and row.get("availability") in {"observed", "partial"}}
    findings = []

    def value(code, fallback=None):
        return observed.get(code, {}).get("value", fallback)

    def add(code, keys, message, *, authority="observed", assumptions=(), contradictions=(), severity="info"):
        # Merging independently stored observations can leave a derived feature
        # present while one of its prerequisites is conflicting or unavailable.
        # A rule must not publish an interpretation with missing evidence (or
        # crash while resolving that evidence).
        if any(key not in observed for key in keys):
            return
        findings.append({"code": code, "rule_version": RULE_VERSION, "severity": severity, "authority": authority,
                         "message": message, "feature_codes": list(keys), "evidence": [ref for key in keys for ref in observed[key]["evidence"]],
                         "assumptions": list(assumptions), "contradictions": list(contradictions)})

    version = value("transaction_version")
    if version is not None and version not in (1, 2):
        add("unusual_transaction_version", ["transaction_version"], "The transaction uses an uncommon version; this does not identify a wallet.")
    lock = value("absolute_locktime", {})
    if lock.get("value") and lock.get("enabled") is False:
        add("ineffective_absolute_locktime", ["absolute_locktime", "sequences"], "All input sequences are final, so the nonzero absolute locktime is disabled.")
    sequence = value("sequences", {})
    if sequence.get("signals_bip125"):
        add("explicit_rbf_signal", ["sequences"], "At least one input explicitly signals BIP125 replacement; relay and replacement depend on node policy.")
    if value("output_script_types", {}).get("counts", {}).get("op_return", 0):
        add("op_return_output", ["output_script_types"], "An observed output carries an OP_RETURN script; the payload is not included in analysis.")
    rate = value("fee_rate", {})
    fee_sats, vsize = rate.get("fee_sats"), rate.get("vsize")
    if fee_sats is not None and vsize:
        # Keep exact integer arithmetic. This is a weak structural pattern, not
        # a claim that a wallet vendor or sender has been identified.
        rounded = (fee_sats + vsize // 2) // vsize
        if rounded in {1, 2, 3, 5, 10, 15, 20, 25, 50, 100} and abs(fee_sats - rounded * vsize) * 100 < vsize and collaboration is None:
            add("rounded_fee_rate", ["fee_rate"], "The observed fee rate is close to a common integer rate; many wallets and manual settings share it.",
                authority="hypothesis", assumptions=("fee_rate_is_not_wallet_identity",), contradictions=("manual_fee_selection", "shared_node_estimator"))
    if sequence.get("relative_locks"):
        add("relative_lock_constraints", ["sequences", "transaction_version"], "Input sequences encode relative lock constraints; maturity needs prevout confirmation context.")
    for side in ("input", "output"):
        code = f"{side}_script_types"
        known = set(value(code, {}).get("counts", {})) - {"unknown", "op_return"}
        if len(known) > 1:
            add(f"mixed_{side}_script_types", [code], f"The {side} scripts use multiple templates. This is a structural distinction, not an ownership link.")
    cardinality = value("cardinality", {})
    collaborative = collaboration is not None
    pattern = value("output_value_pattern", {})
    if pattern.get("equal_groups"):
        add("equal_output_groups", ["output_value_pattern"], "Repeated output amounts provide a CoinJoin hypothesis, but also occur in ordinary batches.", authority="hypothesis",
            assumptions=("protocol_cannot_be_established_from_equal_amounts",), contradictions=("ordinary_batch_payments",))
    if cardinality.get("inputs", 0) >= 3 and cardinality.get("outputs") == 1 and not collaborative:
        add("consolidation_shape", ["cardinality"], "Several inputs create one output, consistent with consolidation or a jointly funded payment.", authority="hypothesis",
            assumptions=("single_owner_not_established",), contradictions=("collaborative_payment",))
    if cardinality.get("outputs", 0) >= 4 and not pattern.get("equal_groups") and not collaborative:
        add("batch_payment_shape", ["cardinality"], "Many outputs are consistent with a payment batch; output roles remain unknown.", authority="hypothesis",
            contradictions=("coinjoin_or_multiple_owners",))
    sigs = value("signature_encodings", {}).get("observations", [])
    ecdsa = [row for row in sigs if row.get("algorithm") == "ecdsa"]
    if len(ecdsa) >= 2 and all(row.get("low_r") for row in ecdsa):
        add("consistent_low_r_encoding", ["signature_encodings"], "All observed ECDSA signatures use low-R encodings. Several wallets share this behavior and chance can produce it.",
            authority="hypothesis", assumptions=("observed_signatures_only",), contradictions=("random_low_r_signatures", "multiple_wallets_share_encoding"))
    if any(row["sighash"] not in (0, 1) for row in sigs):
        add("nondefault_sighash", ["signature_encodings"], "An observed signature uses a sighash other than DEFAULT or ALL; its commitment scope differs.")
    return findings
