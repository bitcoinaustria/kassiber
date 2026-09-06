"""Independent BIP fixtures and hostile proposals for the public-feature seam."""
from copy import deepcopy
from hashlib import new as hash_new, sha256
import json
from pathlib import Path

from embit import compact
from embit.psbt import PSBT
from embit.script import Script, Witness
from embit.transaction import Transaction, TransactionInput, TransactionOutput
import pytest

from kassiber.core.chain_analysis.features import (
    extract_transaction_features, evaluate_features, feature_raw_from_transaction,
    normalize_persisted_features, signature_encoding,
)
from kassiber.core.chain_analysis.psbt import analyze_psbt, compare_psbts
from kassiber.errors import AppError

FIXTURES = Path(__file__).parent / "fixtures" / "chain_analysis"
VECTORS = json.loads((FIXTURES / "psbt_bip_vectors.json").read_text())
PAYJOIN = json.loads((FIXTURES / "payjoin_bip78_vectors.json").read_text())
KEY = bytes.fromhex("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
SCRIPT = b"\x00\x14" + hash_new("ripemd160", sha256(KEY).digest()).digest()
SIG = bytes.fromhex("30440220" + "12" * 32 + "0220" + "34" * 32 + "01")


def value(snapshot, code):
    return next(row["value"] for row in snapshot["features"] if row["code"] == code)


def sample(*, sequence=0xFFFFFFFD, previous=True, witness=True):
    prev = Transaction(vin=[TransactionInput(b"\x99" * 32, 0)], vout=[TransactionOutput(100_000, Script(SCRIPT))])
    tx = Transaction(vin=[TransactionInput(prev.txid(), 0, sequence=sequence)],
                     vout=[TransactionOutput(99_000, Script(SCRIPT))], locktime=100)
    psbt = PSBT(tx)
    if previous:
        psbt.inputs[0].non_witness_utxo = prev
    if witness:
        psbt.inputs[0].witness_utxo = prev.vout[0]
    return psbt


@pytest.mark.parametrize("vector", VECTORS, ids=lambda row: f"BIP{row['bip']}: {row['case']}")
def test_official_bip174_and_bip370_vectors(vector):
    if not vector["valid"]:
        with pytest.raises(AppError) as error:
            analyze_psbt(vector["psbt"], network="test")
        assert error.value.code == "chain_analysis_invalid_psbt"
        return
    result = analyze_psbt(vector["psbt"], network="test")
    assert result["validation"]["status"] == "structurally_valid"
    assert result["validation"]["signatures_verified"] is False
    if "locktime" in vector:
        assert value(result["features"], "absolute_locktime")["value"] == vector["locktime"]


def test_prevout_hash_value_and_script_crosschecks_and_missing_values():
    psbt = sample()
    result = analyze_psbt(psbt.to_base64(), network="regtest")
    assert result["transaction_facts"]["fee_msat"] == "1000000"
    assert result["coverage"]["previous_transaction_hashes_verified"] == 1
    assert result["transaction_facts"]["inputs"][0]["sequence"] == 0xFFFFFFFD
    assert result["validation"]["chain_membership_verified"] is False
    # Same named outpoint cannot carry a different witness value or script.
    psbt.inputs[0].witness_utxo = TransactionOutput(100_001, Script(SCRIPT))
    with pytest.raises(AppError, match="cannot be analyzed") as error:
        analyze_psbt(psbt.to_base64(), network="regtest")
    assert error.value.details["reason"] == "witness_non_witness_utxo_conflict"
    psbt = sample()
    psbt.inputs[0].txid = b"\x88" * 32
    with pytest.raises(AppError) as error:
        analyze_psbt(psbt.to_base64(), network="regtest")
    assert error.value.details["reason"] == "non_witness_utxo_txid_mismatch"
    psbt = sample(previous=False, witness=False)
    result = analyze_psbt(psbt.to_base64(), network="main")
    assert result["transaction_facts"]["complete"] is False
    assert result["transaction_facts"]["inputs"][0]["amount_msat"] is None
    assert result["totals"]["input_msat"] is None and result["totals"]["fee_msat"] is None
    assert result["totals"]["final_fee_rate_sat_vb"] is None


def encode_map(mapping):
    return b"".join(compact.to_bytes(len(key)) + key + compact.to_bytes(len(val)) + val for key, val in mapping.items()) + b"\x00"


def test_v2_preserves_zero_sequence_and_resolves_required_locktime():
    globals = {b"\xfb": (2).to_bytes(4, "little"), b"\x02": (2).to_bytes(4, "little"),
               b"\x03": (55).to_bytes(4, "little"), b"\x04": b"\x01", b"\x05": b"\x01"}
    input_map = {b"\x0e": b"\x11" * 32, b"\x0f": bytes(4), b"\x10": bytes(4), b"\x12": (100).to_bytes(4, "little")}
    output = {b"\x03": (1000).to_bytes(8, "little"), b"\x04": SCRIPT}
    result = analyze_psbt(b"psbt\xff" + encode_map(globals) + encode_map(input_map) + encode_map(output), network="signet")
    assert result["transaction_facts"]["inputs"][0]["sequence"] == 0
    assert value(result["features"], "absolute_locktime") == {"value": 100, "kind": "height", "enabled": True}
    assert value(result["features"], "sequences")["relative_locks"] == [{"input_index": 0, "unit": "blocks", "minimum": 0}]


@pytest.mark.parametrize("change,reason", [
    (lambda data: data + b"\x00", "trailing_bytes_or_extra_maps"),
    (lambda data: data[:-1], "truncated_encoding"),
    (lambda data: data[:5] + b"\xfd\x01\x00" + data[6:], "noncanonical_compact_size"),
])
def test_strict_framing_and_no_raw_payload_in_errors(change, reason):
    with pytest.raises(AppError) as error:
        analyze_psbt(change(sample().serialize()), network="main")
    assert error.value.details["reason"] == reason
    assert "psbt" not in json.dumps(error.value.details)


def test_every_payload_truncation_is_a_typed_error():
    data = sample().serialize()
    for size in range(len(data)):
        with pytest.raises(AppError) as error:
            analyze_psbt(data[:size], network="main")
        assert error.value.code == "chain_analysis_invalid_psbt"


def test_legacy_inventory_linkage_uses_the_v2_and_evidence_decoder():
    from kassiber.core.privacy_linkage import _decode_psbt

    vector = next(row for row in VECTORS if row["bip"] == 370 and row["valid"])
    decoded = _decode_psbt(vector["psbt"])
    assert decoded.version == 2 and len(decoded.inputs) == 1 and len(decoded.outputs) == 2
    conflicting = sample()
    conflicting.inputs[0].witness_utxo = TransactionOutput(100_000, Script(b"\x00\x14" + b"\x55" * 20))
    with pytest.raises(ValueError, match="supplied prevout evidence is invalid"):
        _decode_psbt(conflicting.to_base64())


def test_hygiene_uses_shared_evidence_without_wallet_fingerprint_penalties():
    from kassiber.core.privacy_hygiene import _structural_feature_findings

    raw = {"version": 3, "locktime": 100, "vin": [{"sequence": 0xFFFFFFFF}], "vout": []}
    findings = _structural_feature_findings(raw, collaboration=None)
    assert {row["code"] for row in findings} == {"unusual_transaction_version", "ineffective_absolute_locktime"}
    assert all(row["impact"] == 0 for row in findings)
    assert all(row["details"]["rule_version"] == 1 for row in findings)


def test_explicit_network_and_psbt_never_export_secrets():
    psbt = sample()
    psbt.inputs[0].unknown = {b"\x0b" + b"\x44" * 32: b"secret-preimage-canary", b"\xfcvendor": b"private-proprietary-canary"}
    output = analyze_psbt(psbt.to_base64(), network="test")
    serialized = json.dumps(output)
    assert "secret-preimage-canary" not in serialized and "private-proprietary-canary" not in serialized
    assert SCRIPT.hex() not in serialized and KEY.hex() not in serialized
    assert output["network"] == "test"
    with pytest.raises(AppError) as error:
        analyze_psbt(psbt.to_base64(), network="auto")
    assert error.value.code == "chain_analysis_invalid_network"


def test_content_commitment_binds_scripts_and_private_metadata_without_returning_them():
    original = sample()
    before = analyze_psbt(original.to_base64(), network="test")
    assert before["content_sha256"] == sha256(original.serialize()).hexdigest()
    changed = sample()
    changed.outputs[0].script_pubkey = Script(b"\x00\x14" + b"\x67" * 20)
    after = analyze_psbt(changed.to_base64(), network="test")
    assert before["content_sha256"] != after["content_sha256"]
    assert before["transaction_facts"] == after["transaction_facts"]
    changed.inputs[0].unknown = {b"\xfcprivate": b"metadata-canary"}
    metadata = analyze_psbt(changed.to_base64(), network="test")
    assert after["content_sha256"] != metadata["content_sha256"]
    assert "metadata-canary" not in json.dumps(metadata)


def test_binary_base64_and_hex_files_share_decoded_content_commitment():
    psbt = sample()
    binary = psbt.serialize()
    expected = analyze_psbt(binary, network="test")
    for transport in (psbt.to_base64(), (psbt.to_base64() + "\n").encode("ascii"), binary.hex(), binary.hex().encode("ascii")):
        assert analyze_psbt(transport, network="test") == expected


def test_signature_features_only_examine_recognized_valid_locations():
    raw = {"version": 2, "locktime": 0, "vin": [{"prevout": {"scriptpubkey": SCRIPT.hex()},
           "sequence": 0xFFFFFFFD, "witness": [SIG.hex(), KEY.hex()]}], "vout": [{"value": 1000, "scriptpubkey": SCRIPT.hex()}]}
    facts = extract_transaction_features(raw)
    assert value(facts, "signature_encodings")["observations"][0]["low_r"] is True
    assert value(facts, "signature_encodings")["observations"][0]["sighash"] == 1
    assert SCRIPT.hex() not in json.dumps(facts) and SIG.hex() not in json.dumps(facts)
    forged = deepcopy(raw)
    forged["vin"][0]["witness"] = ["ff" + SIG.hex(), KEY.hex()]
    assert value(extract_transaction_features(forged), "signature_encodings")["count"] == 0
    forged["vin"][0]["prevout"]["scriptpubkey"] = "6a" + SIG.hex()
    forged["vin"][0]["witness"] = [SIG.hex(), KEY.hex()]
    assert value(extract_transaction_features(forged), "signature_encodings")["count"] == 0
    del raw["vin"][0]["prevout"]
    assert value(extract_transaction_features(raw), "signature_encodings")["count"] == 0
    assert signature_encoding(b"\x30" + SIG[1:-1] + b"\x01\x02") is None


def test_relative_locks_version_and_missing_sequence_are_honest():
    raw = {"version": 1, "locktime": 123, "vin": [{"sequence": 17}, {}], "vout": []}
    facts = extract_transaction_features(raw)
    assert value(facts, "sequences")["relative_locks"] == []
    raw["version"] = 2
    raw["vin"][0]["sequence"] = (1 << 22) | 17
    assert value(extract_transaction_features(raw), "sequences")["relative_locks"][0] == {"input_index": 0, "unit": "seconds", "minimum": 8704}
    raw["vin"] = [{}]
    assert value(extract_transaction_features(raw), "sequences")["signals_bip125"] is None
    assert value(extract_transaction_features(raw), "absolute_locktime")["enabled"] is None
    raw["vin"] = [{"sequence": 0xFFFFFFFF}]
    assert "ineffective_absolute_locktime" in {row["code"] for row in evaluate_features(extract_transaction_features(raw))}


def test_collaboration_suppresses_incompatible_shape_hypotheses():
    raw = {"vin": [{}, {}, {}], "vout": [{"value": 1000}]}
    facts = extract_transaction_features(raw)
    assert "consolidation_shape" in {row["code"] for row in evaluate_features(facts)}
    for kind in ("coinjoin", "payjoin", "unknown"):
        assert "consolidation_shape" not in {row["code"] for row in evaluate_features(facts, collaboration={"kind": kind})}


@pytest.mark.parametrize("availability", ["conflicting", "unavailable", "missing"])
@pytest.mark.parametrize("prerequisite", ["transaction_version", "sequences", "absolute_locktime"])
def test_rules_suppress_conflicting_or_unavailable_prerequisites(availability, prerequisite):
    facts = extract_transaction_features({"version": 2, "locktime": 100,
        "vin": [{"sequence": 5}], "vout": []})
    # Exercise both two-prerequisite rules even if independent observations
    # have left their derived values inconsistent at merge time.
    value(facts, "absolute_locktime")["enabled"] = False
    if availability == "missing":
        facts["features"] = [row for row in facts["features"] if row["code"] != prerequisite]
    else:
        next(row for row in facts["features"] if row["code"] == prerequisite)["availability"] = availability
    findings = evaluate_features(facts)
    assert all(prerequisite not in row["feature_codes"] for row in findings)
    codes = {row["code"] for row in findings}
    if prerequisite in {"transaction_version", "sequences"}:
        assert "relative_lock_constraints" not in codes
    if prerequisite in {"absolute_locktime", "sequences"}:
        assert "ineffective_absolute_locktime" not in codes


def test_persisted_features_rebind_and_reject_smuggling():
    facts = extract_transaction_features(feature_raw_from_transaction(sample().tx), subject_id="old-id")
    facts["features"][0]["evidence"] = [{"private": "witness-canary"}]
    facts["features"][0]["source"] = "source-secret"
    facts["features"][0]["assumptions"] = ["private-assumption"]
    result = normalize_persisted_features(facts, subject_id="new-id", source="reference_cache")
    assert result is not None
    assert "canary" not in json.dumps(result) and "source-secret" not in json.dumps(result) and "private-assumption" not in json.dumps(result)
    assert result["features"][0]["evidence"][0]["subject_id"] == "new-id"
    for malicious in ({"private": "secret"}, "xpub-canary", ["witness-canary"]):
        bad = deepcopy(facts)
        bad["features"][0]["value"] = malicious
        assert normalize_persisted_features(bad, subject_id="new-id", source="reference_cache") is None


def payjoin_constraints(**extra):
    return {"payment_output_index": 1, "additional_fee_output_index": 0, "max_additional_fee_contribution_msat": "182000", **extra}


def test_official_bip78_success_and_precise_fee_contribution_cap():
    result = compare_psbts(PAYJOIN["Original PSBT"], PAYJOIN["payjoin proposal"], network="test", payjoin=payjoin_constraints())
    assert result["delta"]["fee_msat"] == "182000"
    assert result["payjoin"]["status"] == "checks_passed"
    assert result["payjoin"]["safe_to_sign"] is None
    checks = {row["code"]: row for row in result["payjoin"]["checks"]}
    assert checks["receiver_signature_commitments_valid"]["status"] == "passed"
    assert checks["fee_contribution_added_input_cost_cap"]["details"]["maximum_msat"] == "182000"
    limited = compare_psbts(PAYJOIN["Original PSBT"], PAYJOIN["payjoin proposal"], network="test", payjoin=payjoin_constraints(max_additional_fee_contribution_msat="181000"))
    assert limited["payjoin"]["status"] == "failed"
    unfinished = compare_psbts(PAYJOIN["Original PSBT"], PAYJOIN["payjoin proposal"], network="test", payjoin=payjoin_constraints(minimum_fee_rate_sat_vb="2"))
    assert unfinished["payjoin"]["status"] == "incomplete"


def test_payjoin_detects_changed_sender_input_and_invalid_receiver_signature():
    proposal = PSBT.from_base64(PAYJOIN["payjoin proposal"])
    # Corrupt the finalized receiver signature without changing its DER shape.
    receiver = next(row for row in proposal.inputs if row.final_scriptwitness is not None)
    signature = bytearray(receiver.final_scriptwitness.items[0])
    signature[10] ^= 1
    receiver.final_scriptwitness.items[0] = bytes(signature)
    result = compare_psbts(PAYJOIN["Original PSBT"], proposal.to_base64(), network="test", payjoin=payjoin_constraints())
    checks = {row["code"]: row["status"] for row in result["payjoin"]["checks"]}
    assert checks["receiver_signature_commitments_valid"] == "failed"
    proposal = PSBT.from_base64(PAYJOIN["payjoin proposal"])
    original = PSBT.from_base64(PAYJOIN["Original PSBT"])
    sender = next(row for row in proposal.inputs if row.txid == original.inputs[0].txid)
    sender.sequence = 0xFFFFFFFD
    result = compare_psbts(PAYJOIN["Original PSBT"], proposal.to_base64(), network="test", payjoin=payjoin_constraints())
    assert {row["code"]: row["status"] for row in result["payjoin"]["checks"]}["sender_sequences_unchanged"] == "failed"


def ordered_payjoin_pair(*, duplicate_outputs=False):
    scripts = [Script(b"\x00\x14" + bytes([byte]) * 20) for byte in (31, 32, 33)]
    if duplicate_outputs:
        scripts[1] = scripts[0]
    tx = Transaction(vin=[TransactionInput(bytes([byte]) * 32, 0, sequence=0xFFFFFFFD) for byte in (11, 12)],
                     vout=[TransactionOutput(amount, script) for amount, script in zip((49_000, 50_000, 100_000), scripts)])
    original = PSBT(tx)
    for row in original.inputs:
        row.witness_utxo = TransactionOutput(100_000, Script(SCRIPT))
        row.final_scriptwitness = Witness([SIG, KEY])
    proposal = PSBT.from_base64(original.to_base64())
    for row in proposal.inputs:
        row.final_scriptwitness = None
    return original, proposal


@pytest.mark.parametrize("side,code", [("inputs", "original_input_order_preserved"), ("outputs", "original_output_order_preserved")])
def test_payjoin_rejects_shuffled_original_inputs_and_outputs(side, code):
    original, proposal = ordered_payjoin_pair()
    getattr(proposal, side).reverse()
    result = compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin={"payment_output_index": 0})
    checks = {row["code"]: row["status"] for row in result["payjoin"]["checks"]}
    assert checks[code] == "failed"
    assert result["payjoin"]["status"] == "failed"
    assert result["payjoin"]["additional_input_count"] == 0


def test_payjoin_duplicate_input_cannot_satisfy_original_input_presence_or_order():
    original, proposal = ordered_payjoin_pair()
    proposal.inputs[1] = deepcopy(proposal.inputs[0])
    with pytest.raises(AppError) as error:
        compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin={"payment_output_index": 0})
    assert error.value.details["reason"] == "duplicate_input_outpoint"


def test_payjoin_allows_receiver_insertions_preserving_original_subsequence_order():
    from embit.ec import PrivateKey

    original, _ = ordered_payjoin_pair()
    tx = original.tx
    tx.vin.insert(1, TransactionInput(bytes([13]) * 32, 0, sequence=0xFFFFFFFD))
    tx.vout.insert(1, TransactionOutput(100_000, Script(b"\x00\x14" + b"\x44" * 20)))
    proposal = PSBT(tx)
    for row in proposal.inputs:
        row.witness_utxo = TransactionOutput(100_000, Script(SCRIPT))
    # Synthetic receiver signs only the inserted input. Production comparison
    # verifies this commitment and never invokes a signer.
    digest = tx.sighash_segwit(1, Script(b"\x76\xa9\x14" + SCRIPT[2:] + b"\x88\xac"), 100_000)
    signature = PrivateKey((1).to_bytes(32, "big")).sign(digest).serialize() + b"\x01"
    proposal.inputs[1].final_scriptwitness = Witness([signature, KEY])
    result = compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin={"payment_output_index": 0})
    checks = {row["code"]: row["status"] for row in result["payjoin"]["checks"]}
    assert checks["original_input_order_preserved"] == "passed"
    assert checks["original_output_order_preserved"] == "passed"
    assert checks["receiver_signature_commitments_valid"] == "passed"
    assert result["payjoin"]["status"] == "checks_passed"


def test_payjoin_output_substitution_does_not_allow_shuffling_sender_outputs():
    original, proposal = ordered_payjoin_pair()
    payment = proposal.outputs.pop(0)
    payment.script_pubkey = Script(b"\x00\x14" + b"\x45" * 20)
    proposal.outputs.append(payment)
    constraints = {"payment_output_index": 0, "allow_output_substitution": True}
    result = compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin=constraints)
    assert result["payjoin"]["status"] == "checks_passed"
    proposal.outputs[0], proposal.outputs[1] = proposal.outputs[1], proposal.outputs[0]
    result = compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin=constraints)
    assert {row["code"]: row["status"] for row in result["payjoin"]["checks"]}["original_output_order_preserved"] == "failed"


def test_payjoin_duplicate_scripts_keep_order_unknown_but_enforce_count_and_total_value():
    original, proposal = ordered_payjoin_pair(duplicate_outputs=True)
    constraints = {"payment_output_index": 2}
    result = compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin=constraints)
    checks = {row["code"]: row["status"] for row in result["payjoin"]["checks"]}
    assert checks["original_output_order_preserved"] == "unavailable"
    assert checks["protected_output_cardinality_preserved"] == "passed"
    assert checks["protected_output_aggregate_value_preserved"] == "passed"
    assert result["payjoin"]["status"] == "incomplete"
    proposal.outputs.pop(0)
    proposal.outputs[0].value = 99_000
    result = compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin=constraints)
    checks = {row["code"]: row["status"] for row in result["payjoin"]["checks"]}
    assert checks["protected_output_cardinality_preserved"] == "failed"
    assert checks["protected_output_aggregate_value_preserved"] == "passed"
    assert result["payjoin"]["status"] == "failed"
    _, proposal = ordered_payjoin_pair(duplicate_outputs=True)
    proposal.outputs[0].value = proposal.outputs[1].value = 1
    result = compare_psbts(original.to_base64(), proposal.to_base64(), network="test", payjoin=constraints)
    checks = {row["code"]: row["status"] for row in result["payjoin"]["checks"]}
    assert checks["protected_output_cardinality_preserved"] == "passed"
    assert checks["protected_output_aggregate_value_preserved"] == "failed"
    assert result["payjoin"]["status"] == "failed"
