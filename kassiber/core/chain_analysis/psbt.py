"""Read-only, local PSBT v0/v2 analysis using the shared feature/rule engine.

BIP174/370 framing and cross-field constraints are checked before embit decodes
transactions and UTXOs. We do not use embit's PSBT transaction reconstruction:
its sequence-zero fallback and missing BIP370 required-locktime resolution are
not suitable for evidence analysis. No signing, key derivation or broadcasting
API is called. Raw maps exist only for the duration of this call.
"""
from __future__ import annotations

import base64
import binascii
from collections import Counter
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from hashlib import new as hash_new, sha256
from typing import Any, Mapping

from embit.base import EmbitError
from embit.ec import PublicKey, Signature
from embit.script import Script, Witness
from embit.transaction import Transaction, TransactionInput, TransactionOutput

from ...errors import AppError
from .features import extract_transaction_features, evaluate_features, signature_encoding, script_type, _pushes

MAX_PSBT_BYTES = 2 * 1024 * 1024
MAX_LEGS = 4096
MAX_MONEY = 21_000_000 * 100_000_000


def _invalid(reason: str, *, input_index: int | None = None):
    details = {"reason": reason}
    if input_index is not None:
        details["input_index"] = input_index
    raise AppError("The PSBT cannot be analyzed safely.", code="chain_analysis_invalid_psbt", details=details) from None


class _Reader:
    def __init__(self, data: bytes):
        self.data, self.offset = data, 0

    def take(self, size: int) -> bytes:
        if size < 0 or self.offset + size > len(self.data):
            _invalid("truncated_encoding")
        value = self.data[self.offset:self.offset + size]
        self.offset += size
        return value

    def read(self, size: int) -> bytes:
        return self.take(size)

    def compact(self) -> int:
        marker = self.take(1)[0]
        if marker < 253:
            return marker
        width = {253: 2, 254: 4, 255: 8}[marker]
        result = int.from_bytes(self.take(width), "little")
        if result < {253: 253, 254: 65536, 255: 2 ** 32}[marker]:
            _invalid("noncanonical_compact_size")
        return result

    def mapping(self) -> dict[bytes, bytes]:
        values = {}
        while True:
            size = self.compact()
            if size == 0:
                return values
            key = self.take(size)
            _Reader(key).compact()  # Key type is itself a canonical compact-size.
            if key in values:
                _invalid("duplicate_map_key")
            if len(values) >= 10_000:
                _invalid("map_entry_limit")
            values[key] = self.take(self.compact())


def _number(mapping: Mapping, key: bytes, width: int, *, default=None, required=False):
    value = mapping.get(key)
    if value is None:
        if required:
            _invalid("missing_required_field")
        return default
    if len(value) != width:
        _invalid("invalid_field_length")
    return int.from_bytes(value, "little")


def _count(mapping: Mapping, key: bytes) -> int:
    if key not in mapping:
        _invalid("missing_required_field")
    reader = _Reader(mapping[key])
    result = reader.compact()
    if reader.offset != len(reader.data) or result > MAX_LEGS:
        _invalid("invalid_or_excessive_leg_count")
    return result


def _decode(cls, value: bytes, reason: str):
    try:
        decoded = cls.parse(value)
        # embit accepts some truncated fixed-width values as zero. Exact
        # reserialization also rejects noncanonical transaction compact sizes.
        if decoded.serialize() != value:
            _invalid(reason)
        return decoded
    except AppError:
        raise
    except (EmbitError, ValueError, IndexError, TypeError, OverflowError, AttributeError):
        _invalid(reason)


def _typed_map(mapping: Mapping, scope: str, version: int):
    singleton = {
        "global": {0, 2, 3, 4, 5, 6, 0xFB},
        "input": {0, 1, 3, 4, 5, 7, 8, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x17, 0x18},
        "output": {0, 1, 3, 4, 5, 6},
    }[scope]
    v2_only = {"global": {2, 3, 4, 5, 6}, "input": set(range(0x0E, 0x13)), "output": {3, 4}}[scope]
    for key in mapping:
        key_type = _Reader(key).compact()
        if key_type in singleton and len(key) != 1:
            _invalid("unexpected_key_data")
        if version == 0 and key_type in v2_only or version == 2 and scope == "global" and key_type == 0:
            _invalid("field_forbidden_in_psbt_version")
        if (scope == "input" and key_type in {2, 6}) or (scope == "output" and key_type == 2):
            try:
                PublicKey.parse(key[1:])
            except (EmbitError, ValueError, IndexError):
                _invalid("invalid_public_key_encoding")
        if (scope == "input" and key_type == 6) or (scope == "output" and key_type == 2):
            if len(mapping[key]) < 4 or len(mapping[key]) % 4:
                _invalid("invalid_derivation_encoding")


def _unsigned_transaction(value: bytes) -> Transaction:
    """Decode explicitly legacy serialization, including a valid empty PSBTv0.

    embit's generic network decoder interprets zero input count as a witness
    marker. BIP174 explicitly permits a zero-input construction-stage PSBT.
    All individual transaction fields remain embit decoded and round-tripped.
    """
    reader = _Reader(value)
    version = int.from_bytes(reader.take(4), "little")

    def legs(cls):
        count = reader.compact()
        if count > MAX_LEGS:
            _invalid("invalid_or_excessive_leg_count")
        rows = []
        for _ in range(count):
            start = reader.offset
            try:
                row = cls.read_from(reader)
            except (EmbitError, ValueError, IndexError, TypeError, OverflowError):
                _invalid("invalid_unsigned_transaction")
            if row.serialize() != reader.data[start:reader.offset]:
                _invalid("invalid_unsigned_transaction")
            rows.append(row)
        return rows

    inputs, outputs = legs(TransactionInput), legs(TransactionOutput)
    locktime = int.from_bytes(reader.take(4), "little")
    if reader.offset != len(value):
        _invalid("invalid_unsigned_transaction")
    return Transaction(version=version, vin=inputs, vout=outputs, locktime=locktime)


@dataclass
class _Parsed:
    psbt_version: int
    tx: Transaction
    globals: dict
    inputs: list[dict]
    outputs: list[dict]
    content_sha256: str


def _parse(text: str | bytes) -> _Parsed:
    if isinstance(text, bytes) and text.startswith(b"psbt\xff"):
        data = text
    else:
        if isinstance(text, bytes):
            if len(text) > MAX_PSBT_BYTES * 3:
                _invalid("payload_limit")
            try:
                text = text.decode("ascii")
            except UnicodeError:
                _invalid("invalid_binary_or_text_encoding")
        if not isinstance(text, str):
            _invalid("invalid_payload_type")
        if len(text) > MAX_PSBT_BYTES * 3:
            _invalid("payload_limit")
        compact = "".join(text.split())
        try:
            data = bytes.fromhex(compact) if compact.startswith("70736274ff") else base64.b64decode(compact, validate=True)
        except (ValueError, binascii.Error):
            _invalid("invalid_base64_or_hex")
    if len(data) > MAX_PSBT_BYTES:
        _invalid("payload_limit")
    reader = _Reader(data)
    if reader.take(5) != b"psbt\xff":
        _invalid("invalid_magic")
    global_map = reader.mapping()
    version = _number(global_map, b"\xfb", 4, default=0)
    if version not in (0, 2):
        _invalid("unsupported_psbt_version")
    _typed_map(global_map, "global", version)
    if version == 0:
        if b"\x00" not in global_map:
            _invalid("missing_unsigned_transaction")
        raw_tx = global_map[b"\x00"]
        tx = _unsigned_transaction(raw_tx)
        if tx.is_segwit or any(row.script_sig.data for row in tx.vin):
            _invalid("unsigned_transaction_contains_scripts_or_witness")
        input_count, output_count = len(tx.vin), len(tx.vout)
        if max(input_count, output_count) > MAX_LEGS:
            _invalid("invalid_or_excessive_leg_count")
    else:
        tx_version = _number(global_map, b"\x02", 4, required=True)
        input_count, output_count = _count(global_map, b"\x04"), _count(global_map, b"\x05")
        fallback_locktime = _number(global_map, b"\x03", 4, default=0)
        _number(global_map, b"\x06", 1, default=0)  # Validate optional flags before building the transaction.
    input_maps = [reader.mapping() for _ in range(input_count)]
    output_maps = [reader.mapping() for _ in range(output_count)]
    if reader.offset != len(data):
        _invalid("trailing_bytes_or_extra_maps")
    for row in input_maps:
        _typed_map(row, "input", version)
    for row in output_maps:
        _typed_map(row, "output", version)
    if version == 2:
        inputs, outputs, time_locks, height_locks, allowed = [], [], [], [], {"time", "height"}
        for row in input_maps:
            txid = row.get(b"\x0e")
            if txid is None or len(txid) != 32:
                _invalid("missing_or_invalid_previous_txid")
            vout = _number(row, b"\x0f", 4, required=True)
            sequence = _number(row, b"\x10", 4, default=0xFFFFFFFF)
            inputs.append(TransactionInput(txid[::-1], vout, sequence=sequence))
            time_lock, height_lock = _number(row, b"\x11", 4), _number(row, b"\x12", 4)
            if time_lock is not None and time_lock < 500_000_000 or height_lock is not None and not 0 < height_lock < 500_000_000:
                _invalid("invalid_required_locktime")
            if time_lock is not None or height_lock is not None:
                allowed &= ({"time"} if time_lock is not None else set()) | ({"height"} if height_lock is not None else set())
            if time_lock is not None:
                time_locks.append(time_lock)
            if height_lock is not None:
                height_locks.append(height_lock)
        if not allowed:
            _invalid("incompatible_required_locktime_types")
        locktime = (max(height_locks) if "height" in allowed and height_locks else max(time_locks) if time_locks else fallback_locktime)
        for row in output_maps:
            amount = _number(row, b"\x03", 8, required=True)
            if b"\x04" not in row:
                _invalid("missing_output_script")
            outputs.append(TransactionOutput(amount, Script(row[b"\x04"])))
        tx = Transaction(version=tx_version, locktime=locktime, vin=inputs, vout=outputs)
    if any(row.value > MAX_MONEY for row in tx.vout) or sum(row.value for row in tx.vout) > MAX_MONEY:
        _invalid("output_amount_out_of_range")
    points = [(row.txid, row.vout) for row in tx.vin]
    if len(set(points)) != len(points):
        _invalid("duplicate_input_outpoint")
    return _Parsed(version, tx, global_map, input_maps, output_maps, sha256(data).hexdigest())


def _validate_scripts(row: Mapping, utxo: TransactionOutput, index: int):
    outer = utxo.script_pubkey.data
    redeem, witness_script = row.get(b"\x04"), row.get(b"\x05")
    if redeem is not None:
        digest = hash_new("ripemd160", sha256(redeem).digest()).digest()
        if script_type(outer) != "p2sh" or digest != outer[2:22]:
            _invalid("redeem_script_mismatch", input_index=index)
    effective = redeem if redeem is not None else outer
    if witness_script is not None:
        if script_type(effective) != "p2wsh" or sha256(witness_script).digest() != effective[2:]:
            _invalid("witness_script_mismatch", input_index=index)
    if b"\x01" in row and script_type(outer) not in {"p2sh", "p2wpkh", "p2wsh", "p2tr", "witness_unknown"}:
        _invalid("witness_utxo_for_nonsegwit_script", input_index=index)
    if b"\x01" in row and redeem is not None and script_type(redeem) not in {"p2wpkh", "p2wsh"}:
        _invalid("witness_utxo_for_nonsegwit_redeem_script", input_index=index)


def _input_utxo(row: Mapping, vin: TransactionInput, index: int) -> tuple[TransactionOutput | None, str]:
    previous, witness = None, None
    if b"\x00" in row:
        transaction = _decode(Transaction, row[b"\x00"], "invalid_non_witness_utxo")
        if transaction.txid() != vin.txid:
            _invalid("non_witness_utxo_txid_mismatch", input_index=index)
        if vin.vout >= len(transaction.vout):
            _invalid("non_witness_utxo_outpoint_out_of_range", input_index=index)
        if any(output.value > MAX_MONEY for output in transaction.vout) or sum(output.value for output in transaction.vout) > MAX_MONEY:
            _invalid("previous_transaction_amount_out_of_range", input_index=index)
        previous = transaction.vout[vin.vout]
    if b"\x01" in row:
        witness = _decode(TransactionOutput, row[b"\x01"], "invalid_witness_utxo")
        if witness.value > MAX_MONEY:
            _invalid("input_amount_out_of_range", input_index=index)
    if previous is not None and witness is not None and previous.serialize() != witness.serialize():
        _invalid("witness_non_witness_utxo_conflict", input_index=index)
    utxo = previous if previous is not None else witness
    if utxo is not None:
        _validate_scripts(row, utxo, index)
    return utxo, "previous_transaction_hash_verified" if previous is not None else "supplied_witness_utxo" if witness is not None else "missing"


def _analyze(parsed: _Parsed, *, network: str, subject_id: str) -> dict:
    tx, inputs, raw_inputs, signatures, metadata = parsed.tx, [], [], [], []
    finalized = []
    for i, (vin, row) in enumerate(zip(tx.vin, parsed.inputs)):
        utxo, evidence = _input_utxo(row, vin, i)
        point = f"{vin.txid.hex()}:{vin.vout}"
        inputs.append({"output_id": f"bitcoin:{network}:out:{point}", "amount_msat": str(utxo.value * 1000) if utxo is not None else None,
                       "input_index": i, "sequence": vin.sequence, "utxo_evidence": evidence})
        raw = {"txid": vin.txid.hex(), "vout": vin.vout, "sequence": vin.sequence,
               "prevout": {"value_sats": utxo.value, "scriptpubkey": utxo.script_pubkey.data.hex()} if utxo is not None else {}}
        if b"\x07" in row:
            raw["scriptsig"] = row[b"\x07"].hex()
        if b"\x08" in row:
            witness = _decode(Witness, row[b"\x08"], "invalid_final_witness")
            raw["witness"] = [item.hex() for item in witness.items]
        raw_inputs.append(raw)
        is_final = b"\x07" in row or b"\x08" in row
        finalized.append(is_final)
        declared_sighash = _number(row, b"\x03", 4)
        partial_count = 0
        for key, data in row.items():
            if key[0] not in {2, 0x13, 0x14}:
                continue
            if key[0] == 2 and (len(key) not in (34, 66) or key[1] not in (2, 3, 4)) or key[0] == 0x13 and len(key) != 1 or key[0] == 0x14 and len(key) != 65:
                _invalid("invalid_partial_signature_key", input_index=i)
            observed = signature_encoding(data, schnorr=key[0] != 2)
            if observed is None:
                _invalid("invalid_partial_signature_encoding", input_index=i)
            if declared_sighash is not None and observed["sighash"] != declared_sighash:
                _invalid("partial_signature_sighash_mismatch", input_index=i)
            # A finalized input may retain a redundant partial signature. Count
            # only its final stack for fingerprint purposes, avoiding duplicates.
            if not is_final:
                signatures.append(dict(observed, input_index=i))
            partial_count += 1
        metadata.append({"input_index": i, "finalized": is_final, "partial_signature_count": partial_count,
                         "derivation_metadata_present": any(key[0] in (6, 0x16) for key in row),
                         "sighash_declared": declared_sighash})
    outputs = [{"output_id": f"{subject_id}:out:{i}", "output_index": i, "amount_msat": str(row.value * 1000),
                "script_type": script_type(row.script_pubkey.data)} for i, row in enumerate(tx.vout)]
    raw = {"version": tx.version, "locktime": tx.locktime, "vin": raw_inputs,
           "vout": [{"value_sats": row.value, "scriptpubkey": row.script_pubkey.data.hex()} for row in tx.vout]}
    features = extract_transaction_features(raw, source="psbt_supplied", subject_id=subject_id, signature_observations=signatures)
    known = bool(inputs) and all(row["amount_msat"] is not None for row in inputs)
    total_in = sum(int(row["amount_msat"]) for row in inputs) if known else None
    total_out = sum(int(row["amount_msat"]) for row in outputs)
    if total_in is not None and total_in > MAX_MONEY * 1000:
        _invalid("total_input_amount_out_of_range")
    fee = total_in - total_out if total_in is not None else None
    if fee is not None and fee < 0:
        _invalid("negative_fee")
    # Exact final vsize only when every input has a supplied final stack. It
    # remains a size observation, not script/signature/broadcast verification.
    vsize = None
    if inputs and all(finalized):
        final_tx = Transaction(version=tx.version, locktime=tx.locktime,
                               vin=[TransactionInput(vin.txid, vin.vout, sequence=vin.sequence,
                                      script_sig=Script(row.get(b"\x07", b"")),
                                      witness=_decode(Witness, row[b"\x08"], "invalid_final_witness") if b"\x08" in row else Witness([]))
                                    for vin, row in zip(tx.vin, parsed.inputs)], vout=tx.vout)
        stripped = len(Transaction(version=final_tx.version, locktime=final_tx.locktime,
                        vin=[TransactionInput(row.txid, row.vout, sequence=row.sequence, script_sig=row.script_sig) for row in final_tx.vin], vout=final_tx.vout).serialize())
        vsize = (stripped * 3 + len(final_tx.serialize()) + 3) // 4
    facts = {"inputs": inputs, "outputs": outputs, "fee_msat": str(fee) if fee is not None else None,
             "complete": bool(inputs and outputs) and known, "collaboration": None,
             "amount_provenance": "psbt_supplied", "amounts_chain_verified": False}
    return {"schema_version": 1, "network": network, "chain": "bitcoin", "psbt_version": parsed.psbt_version,
            "subject_id": subject_id, "content_sha256": parsed.content_sha256,
            "transaction_facts": facts, "features": features,
            "findings": evaluate_features(features), "totals": {"input_msat": str(total_in) if total_in is not None else None,
            "output_msat": str(total_out), "fee_msat": facts["fee_msat"], "final_vsize": vsize,
            "final_fee_rate_sat_vb": str(Decimal(fee) / 1000 / vsize) if fee is not None and vsize else None},
            "coverage": {"known_input_amounts": sum(row["amount_msat"] is not None for row in inputs), "input_count": len(inputs),
                         "previous_transaction_hashes_verified": sum(row["utxo_evidence"] == "previous_transaction_hash_verified" for row in inputs),
                         "missing_input_indices": [row["input_index"] for row in inputs if row["amount_msat"] is None]},
            "validation": {"status": "structurally_valid", "network_source": "caller_selected", "network_verified": False,
                           "chain_membership_verified": False, "unspentness_verified": False, "signatures_verified": False,
                           "input_metadata": metadata, "output_derivation_metadata_present": any(any(key[0] in (2, 7) for key in row) for row in parsed.outputs),
                           "global_xpubs_present": any(key[0] == 1 for key in parsed.globals),
                           "transaction_modifiable_flags": _number(parsed.globals, b"\x06", 1, default=0),
                           "validated_fields": ["map_framing", "transaction_structure", "prevout_consistency", "provided_redeem_and_witness_scripts", "partial_signature_encodings"],
                           "discarded_metadata_is_not_authenticated": True,
                           "limitations": ["supplied_utxo_amounts_are_not_chain_verified", "network_not_encoded_in_script_pubkeys",
                                           "no_signing_or_script_execution", "no_broadcastability_claim", "unsigned_size_not_a_final_fee_rate"]}}


def _network(network: str) -> str:
    if not isinstance(network, str) or network not in {"main", "test", "signet", "regtest"}:
        raise AppError("Select an explicit Bitcoin network.", code="chain_analysis_invalid_network")
    return network


def analyze_psbt(psbt_text: str | bytes, *, network: str, subject_id: str = "psbt:transaction") -> dict:
    """Analyze local data. The returned shape contains no raw PSBT or scripts."""
    return _analyze(_parse(psbt_text), network=_network(network), subject_id=subject_id)


def decode_psbt_structure(psbt_text: str | bytes) -> dict:
    """Transient public transaction structure for the inventory linkage adapter.

    Unlike analysis results, this internal-consumer contract includes output
    scripts so local ownership can be matched. It contains no PSBT metadata,
    signatures or witness bytes, and must not be persisted or sent to AI.
    Networks are deliberately absent: scripts do not identify a network.
    """
    parsed = _parse(psbt_text)
    for i, (vin, row) in enumerate(zip(parsed.tx.vin, parsed.inputs)):
        _input_utxo(row, vin, i)
    return {"version": parsed.tx.version, "locktime": parsed.tx.locktime,
            "inputs": [{"prev_txid": row.txid.hex(), "vout": row.vout, "sequence": row.sequence} for row in parsed.tx.vin],
            "outputs": [{"value_msat": row.value * 1000, "script_key": row.script_pubkey.data.hex(), "is_op_return": row.script_pubkey.data.startswith(b"\x6a")} for row in parsed.tx.vout],
            "unsigned_tx_clean": True,
            "signature_material_present": any(any(key[0] in {2, 7, 8, 0x13, 0x14} for key in row) for row in parsed.inputs)}


def compare_psbts(before_text: str | bytes, after_text: str | bytes, *, network: str,
                  payjoin: Mapping[str, Any] | None = None) -> dict:
    """Compare supplied proposals, optionally applying explicit BIP78 checks."""
    network = _network(network)
    before, after = _parse(before_text), _parse(after_text)
    left = _analyze(before, network=network, subject_id="psbt:before")
    right = _analyze(after, network=network, subject_id="psbt:after")
    left_features = {row["code"]: row["value"] for row in left["features"]["features"]}
    right_features = {row["code"]: row["value"] for row in right["features"]["features"]}
    left_ids = {row["output_id"] for row in left["transaction_facts"]["inputs"]}
    right_ids = {row["output_id"] for row in right["transaction_facts"]["inputs"]}
    left_outputs = Counter(row.script_pubkey.data for row in before.tx.vout)
    right_outputs = Counter(row.script_pubkey.data for row in after.tx.vout)
    left_codes, right_codes = {row["code"] for row in left["findings"]}, {row["code"] for row in right["findings"]}
    fee_a, fee_b = left["totals"]["fee_msat"], right["totals"]["fee_msat"]
    result = {"schema_version": 1, "before": left, "after": right,
              "delta": {"inputs_added": sorted(right_ids - left_ids), "inputs_removed": sorted(left_ids - right_ids),
                        "output_scripts_added": sum((right_outputs - left_outputs).values()),
                        "output_scripts_removed": sum((left_outputs - right_outputs).values()),
                        "fee_msat": str(int(fee_b) - int(fee_a)) if fee_a is not None and fee_b is not None else None,
                        "features_changed": [{"code": key, "before": value, "after": right_features.get(key)} for key, value in left_features.items() if right_features.get(key) != value],
                        "findings_added": sorted(right_codes - left_codes), "findings_removed": sorted(left_codes - right_codes)}}
    if payjoin is not None:
        result["payjoin"] = _payjoin_checks(before, after, left, right, payjoin)
    return result


def _payjoin_checks(before: _Parsed, after: _Parsed, left: dict, right: dict, constraints: Mapping) -> dict:
    allowed = {"payment_output_index", "additional_fee_output_index", "max_additional_fee_contribution_msat", "allow_output_substitution", "minimum_fee_rate_sat_vb"}
    if not isinstance(constraints, Mapping) or set(constraints) - allowed:
        raise AppError("Invalid Payjoin comparison constraints.", code="chain_analysis_invalid_payjoin_constraints")
    payment = constraints.get("payment_output_index")
    fee_output = constraints.get("additional_fee_output_index")
    if isinstance(payment, bool) or not isinstance(payment, int) or not 0 <= payment < len(before.tx.vout):
        raise AppError("Select the original payment output for Payjoin comparison.", code="chain_analysis_invalid_payjoin_constraints")
    if fee_output is not None and (isinstance(fee_output, bool) or not isinstance(fee_output, int) or not 0 <= fee_output < len(before.tx.vout) or fee_output == payment):
        raise AppError("Invalid additional fee output.", code="chain_analysis_invalid_payjoin_constraints")
    maximum = constraints.get("max_additional_fee_contribution_msat", "0")
    if not isinstance(maximum, str) or not maximum.isascii() or not maximum.isdigit() or len(maximum) > 19 or int(maximum) > MAX_MONEY * 1000 or int(maximum) % 1000:
        raise AppError("Fee contribution must be an exact nonnegative whole-satoshi msat string.", code="chain_analysis_invalid_payjoin_constraints")
    substitute = constraints.get("allow_output_substitution", False)
    if not isinstance(substitute, bool):
        raise AppError("Output substitution must be a boolean.", code="chain_analysis_invalid_payjoin_constraints")
    checks = []

    def check(code, passed, **details):
        checks.append({"code": code, "status": "unavailable" if passed is None else "passed" if passed else "failed", "details": details})

    a_inputs = {(row.txid, row.vout): row for row in before.tx.vin}
    b_inputs = {(row.txid, row.vout): (i, row) for i, row in enumerate(after.tx.vin)}
    check("original_inputs_preserved", set(a_inputs) <= set(b_inputs))
    # BIP78 permits inserting receiver inputs at arbitrary positions, while
    # preserving the relative order of all original inputs. Duplicate outpoints
    # were already rejected by the shared decoder.
    check("original_input_order_preserved", [point for point in b_inputs if point in a_inputs] == list(a_inputs))
    check("transaction_version_unchanged", before.tx.version == after.tx.version)
    check("locktime_unchanged", before.tx.locktime == after.tx.locktime)
    check("sender_sequences_unchanged", all(point in b_inputs and row.sequence == b_inputs[point][1].sequence for point, row in a_inputs.items()))
    check("proposal_sequences_uniform", len({row.sequence for row in after.tx.vin}) <= 1)
    metadata = right["validation"]["input_metadata"]
    check("no_input_derivations", not any(row["derivation_metadata_present"] for row in metadata))
    check("no_output_derivations", not right["validation"]["output_derivation_metadata_present"])
    check("no_partial_signatures", not any(row["partial_signature_count"] for row in metadata))
    check("sender_inputs_unfinalized", not any(metadata[i]["finalized"] for point, (i, _) in b_inputs.items() if point in a_inputs))
    receiver_indices = [i for point, (i, _) in b_inputs.items() if point not in a_inputs]
    check("receiver_inputs_finalized", all(metadata[i]["finalized"] for i in receiver_indices))
    check("receiver_utxos_supplied", all(right["transaction_facts"]["inputs"][i]["amount_msat"] is not None for i in receiver_indices))
    receiver_signatures = [_verify_segwit_keyhash_signature(after, i) for i in receiver_indices]
    check("receiver_signature_commitments_valid", False if False in receiver_signatures else None if None in receiver_signatures else True,
          supported_templates=["p2wpkh", "p2sh-p2wpkh"])
    # Same outpoint must not change its supplied value or script. This is checked
    # privately; neither script bytes nor hashes are returned.
    consistent = True
    for point, vin in a_inputs.items():
        if point not in b_inputs:
            continue
        i = next(index for index, row in enumerate(before.tx.vin) if (row.txid, row.vout) == point)
        j = b_inputs[point][0]
        a, _ = _input_utxo(before.inputs[i], vin, i)
        b, _ = _input_utxo(after.inputs[j], b_inputs[point][1], j)
        if a is None or b is None:
            consistent = None if consistent is True else consistent
        elif a.serialize() != b.serialize():
            consistent = False
    check("sender_prevouts_unchanged", consistent)
    fee_a, fee_b = left["totals"]["fee_msat"], right["totals"]["fee_msat"]
    delta = int(fee_b) - int(fee_a) if fee_a is not None and fee_b is not None else None
    check("absolute_fee_not_decreased", delta >= 0 if delta is not None else None)
    # The explicitly substitutable payment output may disappear or be replaced;
    # every other original output must survive in its original relative order.
    # Added outputs are insertions, not permission to shuffle protected outputs.
    protected = [(i, row) for i, row in enumerate(before.tx.vout) if not (i == payment and substitute)]
    original_scripts = Counter(row.script_pubkey.data for _, row in protected)
    proposal_scripts = Counter(row.script_pubkey.data for row in after.tx.vout)
    counts_preserved = all(proposal_scripts[script] >= count for script, count in original_scripts.items())
    required_values, proposed_values = Counter(), Counter()
    for i, row in protected:
        # With duplicate scripts this aggregate is a necessary lower bound,
        # not proof that each individual original output is preserved.
        required_values[row.script_pubkey.data] += row.value * 1000 - (min(int(maximum), row.value * 1000) if i == fee_output else 0)
    for row in after.tx.vout:
        proposed_values[row.script_pubkey.data] += row.value * 1000
    values_preserved = all(proposed_values[script] >= amount for script, amount in required_values.items())
    check("protected_output_cardinality_preserved", counts_preserved)
    check("protected_output_aggregate_value_preserved", values_preserved)
    # Repeated scripts do not justify picking an arbitrary correspondence or
    # counting a single proposal output for several original outputs.
    unique = all(count == 1 and proposal_scripts[script] <= 1 for script, count in original_scripts.items())
    check("output_correspondence_unambiguous", True if unique else None)
    contribution = 0
    if unique:
        positions = {row.script_pubkey.data: i for i, row in enumerate(after.tx.vout)}
        original_positions = [positions[row.script_pubkey.data] for _, row in protected if row.script_pubkey.data in positions]
        check("original_output_order_preserved", counts_preserved and original_positions == sorted(original_positions),
              payment_output_substitution_allowed=substitute)
        output_checks = []
        for i, output in protected:
            position = positions.get(output.script_pubkey.data)
            match = after.tx.vout[position] if position is not None else None
            if match is None:
                output_checks.append(False)
            elif i == fee_output:
                contribution = max(0, output.value - match.value) * 1000
                output_checks.append(contribution <= int(maximum))
            else:
                output_checks.append(match.value >= output.value)
        check("sender_outputs_preserved_and_values_valid", all(output_checks))
        check("fee_contribution_within_budget", contribution <= int(maximum), contribution_msat=str(contribution), maximum_msat=maximum)
        check("fee_contribution_only_pays_fee_increase", contribution <= delta if delta is not None else None)
        # Use each actual original finalized input size; no fixed 68-vbyte
        # fallback. Heterogeneous original input templates leave BIP78's singular
        # sender_input_type premise unresolved rather than picking the largest.
        original_sizes = [_final_input_vsize(before, i) for i in range(len(before.tx.vin))]
        rate = left["totals"]["final_fee_rate_sat_vb"]
        cap = (Decimal(rate) * original_sizes[0] * len(receiver_indices) * 1000
               if rate is not None and original_sizes and None not in original_sizes and len(set(original_sizes)) == 1 else None)
        check("fee_contribution_added_input_cost_cap", contribution <= cap if cap is not None else True if contribution == 0 else None,
              maximum_msat=str(cap) if cap is not None else None)
    else:
        check("original_output_order_preserved", None, payment_output_substitution_allowed=substitute)
        check("sender_outputs_preserved_and_values_valid", None if counts_preserved and values_preserved else False)
    minimum = constraints.get("minimum_fee_rate_sat_vb")
    if minimum is not None:
        try:
            if not isinstance(minimum, str) or len(minimum) > 40:
                raise ValueError
            min_rate = Decimal(minimum)
            if not min_rate.is_finite() or min_rate < 0:
                raise ValueError
        except (ValueError, InvalidOperation):
            raise AppError("Minimum fee rate must be a nonnegative decimal string.", code="chain_analysis_invalid_payjoin_constraints") from None
        rate = right["totals"]["final_fee_rate_sat_vb"]
        check("minimum_final_fee_rate", Decimal(rate) >= min_rate if rate is not None else None)
    return {"protocol": "bip78", "status": "failed" if any(row["status"] == "failed" for row in checks) else "incomplete" if any(row["status"] == "unavailable" for row in checks) else "checks_passed",
            "checks": checks, "additional_input_count": len(receiver_indices),
            "receiver_contribution_msat": str(sum(int(right["transaction_facts"]["inputs"][i]["amount_msat"]) for i in receiver_indices)) if all(right["transaction_facts"]["inputs"][i]["amount_msat"] is not None for i in receiver_indices) else None,
            "negotiation_verified": False, "safe_to_sign": None,
            "limitations": ["local_supplied_psbts_only", "signature_checks_are_not_full_consensus_script_execution", "ownership_unspentness_and_negotiation_not_verified", "final_feerate_requires_signed_proposal"]}


def _final_input_vsize(parsed: _Parsed, index: int) -> int | None:
    row, vin = parsed.inputs[index], parsed.tx.vin[index]
    utxo, _ = _input_utxo(row, vin, index)
    if utxo is None or not (b"\x07" in row or b"\x08" in row):
        return None
    kind = script_type(utxo.script_pubkey.data)
    if kind not in {"p2pkh", "p2wpkh", "p2sh", "p2tr"}:
        return None
    scriptsig = row.get(b"\x07", b"")
    if kind == "p2sh":
        pushes = _pushes(scriptsig)
        if not pushes or len(pushes) != 1 or script_type(pushes[0]) != "p2wpkh":
            return None
    if kind in {"p2wpkh", "p2tr", "p2sh"} and b"\x08" not in row:
        return None
    base = TransactionInput(vin.txid, vin.vout, sequence=vin.sequence, script_sig=Script(scriptsig))
    witness_size = len(row[b"\x08"]) if b"\x08" in row else 0
    return (len(base.serialize()) * 4 + witness_size + 3) // 4


def _verify_segwit_keyhash_signature(parsed: _Parsed, index: int) -> bool | None:
    """Verify supported receiver signature commitments, without signing.

    This is intentionally narrower than executing consensus scripts. Missing
    prevouts or unsupported templates stay unavailable; invalid supported
    commitments fail explicitly. No key or signature is retained.
    """
    row, vin = parsed.inputs[index], parsed.tx.vin[index]
    utxo, _ = _input_utxo(row, vin, index)
    if utxo is None:
        return None
    outer = utxo.script_pubkey.data
    if script_type(outer) == "p2sh":
        pushes = _pushes(row.get(b"\x07"))
        if not pushes or len(pushes) != 1:
            return None
        redeem = pushes[0]
        if hash_new("ripemd160", sha256(redeem).digest()).digest() != outer[2:22]:
            return False
        outer = redeem
    elif row.get(b"\x07", b""):
        return False if script_type(outer) == "p2wpkh" else None
    if script_type(outer) != "p2wpkh":
        return None
    if b"\x08" not in row:
        return False
    witness = _decode(Witness, row[b"\x08"], "invalid_final_witness").items
    if len(witness) != 2 or hash_new("ripemd160", sha256(witness[1]).digest()).digest() != outer[2:]:
        return False
    signature = signature_encoding(witness[0])
    if signature is None or signature["sighash"] not in (1, 2, 3, 0x81, 0x82, 0x83):
        return False
    try:
        public_key = PublicKey.parse(witness[1])
        digest = parsed.tx.sighash_segwit(index, Script(b"\x76\xa9\x14" + outer[2:] + b"\x88\xac"), utxo.value, sighash=signature["sighash"])
        return public_key.verify(Signature.parse(witness[0][:-1]), digest)
    except (EmbitError, ValueError, IndexError):
        return False
