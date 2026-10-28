"""Freeze embit-backed public Liquid decoding, including its lenient edges.

Only synthetic public bytes are recorded. Expected values come exclusively
from the Python production helper, never from the candidate or a consensus
validator. This corpus covers parser equivalence classes; it is not a claim
to enumerate the infinite set of possible malformed transactions.
"""
from collections import Counter
import json
from pathlib import Path
import random
import sys

from kassiber.core.onchain import _decode_liquid_raw_hex

PATH = Path(__file__).parent / "fixtures/custody_liquid_cases.json"


def compact(value, width=0):
    if width:
        return {2: b"\xfd", 4: b"\xfe", 8: b"\xff"}[width] + value.to_bytes(width, "little")
    return bytes([value]) if value < 253 else compact(value, 2)


def parts(*, flag=0, vout=7, issuance=b"", asset_prefix=1, value_prefix=1, nonce_prefix=0, witness=False):
    # A nonuniform txid/asset and nonsymmetric amount detect byte-order drift.
    return dict(
        version=bytes.fromhex("02000000"), flag=bytes([flag]),
        input_count=b"\x01", txid=bytes(range(32)), vout=vout.to_bytes(4, "little"),
        input_script_length=b"\x02", input_script=b"\x51\x52", sequence=b"\xff" * 4,
        issuance=issuance, output_count=b"\x01", asset=bytes([asset_prefix]) + bytes(range(32, 64)),
        value=bytes([value_prefix]) + ((0x123456789ABC).to_bytes(8, "big") if value_prefix == 1 else b"\x42" * 32),
        nonce=bytes([nonce_prefix]) + (b"\x43" * 32 if nonce_prefix else b""),
        output_script_length=b"\x03", output_script=b"\x00\x51\x6a", locktime=b"\x11\x22\x33\x44",
        amount_proof_length=b"\x02" if witness else b"", amount_proof=b"ab" if witness else b"",
        token_proof_length=b"\x01" if witness else b"", token_proof=b"c" if witness else b"",
        script_witness_count=b"\x01" if witness else b"", script_witness_length=b"\x02" if witness else b"", script_witness=b"de" if witness else b"",
        pegin_witness_count=b"\x01" if witness else b"", pegin_witness_length=b"\x01" if witness else b"", pegin_witness=b"f" if witness else b"",
        surjection_length=b"\x02" if witness else b"", surjection=b"gh" if witness else b"",
        rangeproof_length=b"\x03" if witness else b"", rangeproof=b"ijk" if witness else b"",
    )


def encoded(fields):
    return b"".join(fields.values())


def vectors():
    cases = []
    def add(group, name, raw):
        value = raw.hex() if isinstance(raw, bytes) else raw
        cases.append(dict(group=group, case=name, input=value, expected=_decode_liquid_raw_hex(value)))
    variants = dict(
        explicit=parts(), confidential=parts(asset_prefix=10, value_prefix=8, nonce_prefix=2),
        explicit_blinded=parts(nonce_prefix=3), confidential_unblinded=parts(asset_prefix=11, value_prefix=9),
        pegin=parts(vout=(1 << 30) | 9), coinbase=parts(vout=0xFFFFFFFF),
        issuance_null=parts(vout=(1 << 31) | 11, issuance=b"\x12" * 32 + b"\x34" * 32 + b"\x00\x00"),
        issuance_explicit=parts(vout=(3 << 30) | 13, issuance=b"\x12" * 32 + b"\x34" * 32 + b"\x01" + (123).to_bytes(8, "big") + b"\x01" + (456).to_bytes(8, "big")),
        issuance_confidential=parts(vout=(1 << 31) | 17, issuance=b"\x12" * 32 + b"\x34" * 32 + b"\x08" + b"\x56" * 32 + b"\x09" + b"\x78" * 32),
        witness=parts(flag=1, witness=True),
    )
    # Well-formed curve commitments as well as intentionally opaque/invalid
    # point bytes. Neither production decoder performs proof verification.
    from embit.util import secp256k1
    generator=secp256k1.generator_generate(bytes(range(32)))
    committed=parts(asset_prefix=10,value_prefix=8,nonce_prefix=2)
    committed["asset"]=secp256k1.generator_serialize(generator)
    committed["value"]=secp256k1.pedersen_commitment_serialize(secp256k1.pedersen_commit(b"\x01"*32,123456,generator))
    committed["nonce"]=bytes.fromhex("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
    variants["confidential_curve_points"]=committed
    for name, fields in variants.items():
        raw = encoded(fields)
        add("valid", name, raw)
        for end in range(len(raw)):
            add("truncation", f"{name}:prefix:{end}", raw[:end])
        for suffix in (b"\x00", b"\xff", b"\x00" * 5):
            add("truncation", f"{name}:trailing:{suffix.hex()}", raw + suffix)
    explicit = encoded(variants["explicit"])
    for flag in range(256):
        add("flags", f"flag:{flag}", encoded(parts(flag=flag)))
    for field in ("asset_prefix", "value_prefix", "nonce_prefix"):
        for prefix in range(256):
            add("prefixes", f"{field}:{prefix}", encoded(parts(**{field: prefix})))
    for field,prefix in (("asset",10),("value",8),("nonce",2)):
        changed=parts();changed[field]=bytes([prefix])+b"\x00"*32
        add("prefixes","invalid_curve_point:"+field,encoded(changed))
    # Empty scripts/outputs/inputs, high flags and complete empty witnesses.
    add("valid", "empty", bytes.fromhex("02000000000000"))
    add("witness", "empty_witness_transaction", bytes.fromhex("0200000001000000000000"))
    add("witness", "superfluous_empty_witness", encoded(parts(flag=1)) + b"\x00" * 6)
    for value in (0, 1, 0xFFFFFFFFFFFFFFFF):
        fields = parts()
        fields["value"] = b"\x01" + value.to_bytes(8, "big")
        fields["output_script_length"] = b"\x00"
        fields["output_script"] = b""
        add("valid", f"fee:{value}", encoded(fields))
    for prefix in range(256):
        commit = bytes([prefix]) + (b"\x13" * (8 if prefix == 1 else 32) if prefix else b"")
        add("issuance", f"commitment:{prefix}", encoded(parts(vout=(1 << 31) | 19, issuance=b"\x12" * 64 + commit + b"\x00")))
    fields = variants["witness"]
    length_fields = [key for key in fields if key.endswith(("_count", "_length"))]
    for field in length_fields:
        for width in (2, 4, 8):
            changed = dict(fields)
            changed[field] = compact(fields[field][0], width)
            add("varints", f"nonminimal:{field}:{width}", encoded(changed))
            # Each possible short payload; also reaches accepted terminal short
            # varints for scripts/proofs and stack counts where Python permits it.
            before = encoded(dict(list(changed.items())[:list(changed).index(field)]))
            for size in range(width):
                add("varints", f"short:{field}:{width}:{size}", before + changed[field][:size + 1])
        for value in (0, 2, 252, 253, 65536, 2**63 - 1, 2**63, 2**64 - 1):
            changed = dict(fields)
            changed[field] = compact(value, 8)
            add("varints", f"length:{field}:{value}", encoded(changed))
    for field in length_fields[4:]:
        changed = dict(fields)
        changed[field] = b"\xfc"
        add("witness", f"short_payload:{field}", encoded(changed))
    # Shape with multiple inputs/outputs proves all witness loops and order.
    fields = parts(flag=1, witness=True)
    raw = encoded(fields)
    in_start = 6
    out_count = sum(len(v) for key, v in list(fields.items())[:list(fields).index("output_count")])
    out_start = out_count + 1
    locktime = sum(len(v) for key, v in list(fields.items())[:list(fields).index("locktime")])
    in_witness = encoded({key: fields[key] for key in list(fields)[list(fields).index("amount_proof_length"):list(fields).index("surjection_length")]})
    out_witness = encoded({key: fields[key] for key in list(fields)[list(fields).index("surjection_length"):]})
    add("witness", "multiple", raw[:5] + b"\x02" + raw[in_start:out_count] * 2 + b"\x02" + raw[out_start:locktime] * 2 + fields["locktime"] + in_witness * 2 + out_witness * 2)
    for name, value in (
        ("null", None), ("bool", True), ("number", 123), ("mapping", {}), ("list", []),
        ("empty", ""), ("space", " \n"), ("odd", "0"), ("bad", "zz"),
        ("prefix", "0x" + explicit.hex()), ("internal_space", explicit.hex()[:4] + " " + explicit.hex()[4:]),
        ("unicode_hex", "０２" + explicit.hex()[2:]),
        ("upper", explicit.hex().upper()), ("strip", "\x1c\u2003" + explicit.hex() + "\u0085"),
    ):
        add("hex", name, value)
    rng = random.Random(0xE1E0E17)
    for seed in range(512):
        raw = bytearray(encoded(variants[rng.choice(list(variants))]))
        for _ in range(rng.randrange(1, 5)):
            raw[rng.randrange(len(raw))] = rng.randrange(256)
        if seed % 3 == 0:
            raw = raw[:rng.randrange(len(raw) + 1)]
        add("seeded", f"seed:{seed}", bytes(raw))
    assert len({(case["group"], case["case"]) for case in cases}) == len(cases)
    return cases


def summary(cases):
    counts = Counter((case["group"], "accepted" if case["expected"] is not None else "refused") for case in cases)
    return {group: {state: counts[group, state] for state in ("accepted", "refused")} for group in sorted({case["group"] for case in cases})}


if __name__ == "__main__":
    cases = vectors()
    if "--write" in sys.argv:
        PATH.write_text(json.dumps(cases, ensure_ascii=True, indent=2) + "\n")
    else:
        assert cases == json.loads(PATH.read_text())
    print(json.dumps(summary(cases), sort_keys=True))
