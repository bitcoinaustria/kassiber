#!/usr/bin/env python3
"""Freeze Python's release-signature verdicts for the native SSHSIG check.

Every case runs ``kassiber.release_verification.verify_signature_bytes``, the
Python reference, which hands the signature to the system ``ssh-keygen -Y
verify``. The signatures are built here from fixed seeds so the corpus is
deterministic: valid Ed25519 and Ed25519-SK signatures, every SSHSIG field the
verifier parses (version, key, namespace, reserved, hash, signature, flags),
malleated scalars, and armour/base64 variants. The plain Ed25519 case is
checked byte for byte against ``ssh-keygen -Y sign`` before anything is
written. Requires OpenSSH 8.1 or newer; no network.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.errors import AppError  # noqa: E402
from kassiber import release_verification as rv  # noqa: E402

DESTINATION = ROOT / "crates/release/tests/data/sshsig_vectors.json"
NAMESPACE = rv.RELEASE_SIGNATURE_NAMESPACE.encode()
MESSAGE = b"# Kassiber release manifest v1\n# Version: 1.2.3\n" + b"0" * 64 + b"  kassiber.dmg\n"
ED25519 = b"ssh-ed25519"
SK_ED25519 = b"sk-ssh-ed25519@openssh.com"
ORDER = 2**252 + 27742317777372353535851937790883648493
BEGIN = b"-----BEGIN SSH SIGNATURE-----"
END = b"-----END SSH SIGNATURE-----"


def string(value: bytes) -> bytes:
    return struct.pack(">I", len(value)) + value


def key(seed: int) -> Ed25519PrivateKey:
    return Ed25519PrivateKey.from_private_bytes(bytes([seed]) * 32)


def raw_public(private: Ed25519PrivateKey) -> bytes:
    return private.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def ed_blob(private: Ed25519PrivateKey) -> bytes:
    return string(ED25519) + string(raw_public(private))


def sk_blob(private: Ed25519PrivateKey, application: bytes = b"ssh:") -> bytes:
    return string(SK_ED25519) + string(raw_public(private)) + string(application)


def public_line(blob: bytes, comment: str = "release") -> bytes:
    kind = blob[4 : 4 + struct.unpack(">I", blob[:4])[0]]
    return kind + b" " + base64.b64encode(blob) + b" " + comment.encode() + b"\n"


def digest(name: bytes, message: bytes) -> bytes:
    return hashlib.new({b"sha256": "sha256", b"sha512": "sha512", b"sha384": "sha384"}.get(name.lower(), "sha512"), message).digest()


def signed_data(namespace=NAMESPACE, hashalg=b"sha512", message=MESSAGE, reserved=b"") -> bytes:
    return b"SSHSIG" + string(namespace) + string(reserved) + string(hashalg) + string(digest(hashalg, message))


def ed_signature(private, data, *, ktype=ED25519, mutate=None, trailing=b"") -> bytes:
    raw = private.sign(data)
    if mutate:
        raw = mutate(raw)
    return string(ktype) + string(raw) + trailing


def sk_signature(private, data, *, application=b"ssh:", flags=1, counter=7, ktype=SK_ED25519, trailing=b"") -> bytes:
    trailer = bytes([flags]) + struct.pack(">I", counter)
    message = hashlib.sha256(application).digest() + trailer + hashlib.sha256(data).digest()
    return string(ktype) + string(private.sign(message)) + trailer + trailing


def sshsig(public_blob, inner, *, namespace=NAMESPACE, hashalg=b"sha512", reserved=b"", version=1, magic=b"SSHSIG", trailing=b"") -> bytes:
    return (
        magic + struct.pack(">I", version) + string(public_blob) + string(namespace)
        + string(reserved) + string(hashalg) + string(inner) + trailing
    )


def armor(blob: bytes, *, width=70, newline=b"\n", final=b"\n") -> bytes:
    text = base64.b64encode(blob)
    lines = [text[i : i + width] for i in range(0, len(text), width)] if width else [text]
    return BEGIN + newline + newline.join(lines) + newline + END + final


def scalar(raw: bytes, add: int) -> bytes:
    value = int.from_bytes(raw[32:], "little") + add
    return raw[:32] + value.to_bytes(32, "little")


def cases():
    pinned, other, sk = key(1), key(2), key(3)
    pinned_blob, sk_pinned = ed_blob(pinned), sk_blob(sk)
    data, data256 = signed_data(), signed_data(hashalg=b"sha256")
    valid_blob = sshsig(pinned_blob, ed_signature(pinned, data))
    valid = armor(valid_blob)
    encoded = base64.b64encode(valid_blob)
    yield "valid_sha512", pinned_blob, valid
    yield "valid_sha256", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data256), hashalg=b"sha256"))
    yield "signed_empty_message", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(message=b""))))
    yield "other_message", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(message=MESSAGE + b"x"))))
    yield "hash_sha384", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(hashalg=b"sha384")), hashalg=b"sha384"))
    yield "hash_uppercase", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(hashalg=b"SHA512")), hashalg=b"SHA512"))
    yield "hash_signed_256_declared_512", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data256)))
    yield "namespace_other", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(namespace=b"other")), namespace=b"other"))
    yield "namespace_empty", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(namespace=b"")), namespace=b""))
    yield "namespace_trailing_space", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(namespace=NAMESPACE + b" ")), namespace=NAMESPACE + b" "))
    yield "namespace_nul", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(namespace=NAMESPACE + b"\0")), namespace=NAMESPACE + b"\0"))
    yield "namespace_declared_other_signed_expected", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data), namespace=b"other"))
    yield "reserved_ignored", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data), reserved=b"x"))
    yield "reserved_signed", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, signed_data(reserved=b"x")), reserved=b"x"))
    yield "version_0", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data), version=0))
    yield "version_2", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data), version=2))
    yield "magic_wrong", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data), magic=b"SSHSIH"))
    yield "outer_trailing_byte", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data), trailing=b"\0"))
    yield "truncated", pinned_blob, armor(valid_blob[:-1])
    yield "other_signer", pinned_blob, armor(sshsig(ed_blob(other), ed_signature(other, data)))
    yield "other_signer_claims_pinned_key", pinned_blob, armor(sshsig(pinned_blob, ed_signature(other, data)))
    yield "embedded_key_trailing_byte", pinned_blob, armor(sshsig(pinned_blob + b"\0", ed_signature(pinned, data)))
    yield "embedded_key_as_sk", pinned_blob, armor(sshsig(sk_blob(pinned), sk_signature(pinned, data)))
    yield "inner_type_sk_for_ed_key", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, ktype=SK_ED25519)))
    yield "inner_type_rsa", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, ktype=b"rsa-sha2-512")))
    yield "inner_trailing_byte", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, trailing=b"\0")))
    yield "inner_short", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, mutate=lambda r: r[:63])))
    yield "inner_long", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, mutate=lambda r: r + b"\0")))
    yield "inner_empty", pinned_blob, armor(sshsig(pinned_blob, b""))
    yield "scalar_plus_order", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, mutate=lambda r: scalar(r, ORDER))))
    yield "scalar_plus_two_orders", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, mutate=lambda r: scalar(r, 2 * ORDER))))
    yield "scalar_flipped", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, mutate=lambda r: scalar(r, 1))))
    yield "r_flipped", pinned_blob, armor(sshsig(pinned_blob, ed_signature(pinned, data, mutate=lambda r: bytes([r[0] ^ 1]) + r[1:])))
    # Ed25519-SK: the pinned key carries its FIDO application.
    yield "sk_valid", sk_pinned, armor(sshsig(sk_pinned, sk_signature(sk, data)))
    yield "sk_valid_sha256", sk_pinned, armor(sshsig(sk_pinned, sk_signature(sk, data256), hashalg=b"sha256"))
    yield "sk_no_user_presence", sk_pinned, armor(sshsig(sk_pinned, sk_signature(sk, data, flags=0)))
    yield "sk_verified_flags", sk_pinned, armor(sshsig(sk_pinned, sk_signature(sk, data, flags=5, counter=0xFFFFFFFF)))
    yield "sk_wrong_application_signed", sk_pinned, armor(sshsig(sk_pinned, sk_signature(sk, data, application=b"ssh:other")))
    yield "sk_other_application_key", sk_pinned, armor(sshsig(sk_blob(sk, b"ssh:other"), sk_signature(sk, data, application=b"ssh:other")))
    yield "sk_inner_type_ed", sk_pinned, armor(sshsig(sk_pinned, ed_signature(sk, data)))
    yield "sk_inner_trailing_byte", sk_pinned, armor(sshsig(sk_pinned, sk_signature(sk, data, trailing=b"\0")))
    yield "sk_plain_key_for_sk_pin", sk_pinned, armor(sshsig(ed_blob(sk), ed_signature(sk, data)))
    yield "sk_application_ssh_kassiber", sk_blob(sk, b"ssh:kassiber"), armor(sshsig(sk_blob(sk, b"ssh:kassiber"), sk_signature(sk, data, application=b"ssh:kassiber")))
    for name, application in (("sk_application_not_utf8", b"ssh:\xff"), ("sk_application_nul", b"ssh:\0x")):
        blob = sk_blob(sk, application)
        yield name, blob, armor(sshsig(blob, sk_signature(sk, data, application=application)))
    # Armour and base64: OpenSSH's sshsig_dearmor and b64_pton.
    yield "armor_nbsp_in_base64", pinned_blob, BEGIN + b"\n" + encoded[:10] + b"\xa0" + encoded[10:] + b"\n" + END + b"\n"
    yield "armor_nel_in_base64", pinned_blob, BEGIN + b"\n" + encoded[:10] + b"\x85" + encoded[10:] + b"\n" + END + b"\n"
    yield "armor_leading_whitespace", pinned_blob, b" \n\t" + valid
    yield "armor_crlf", pinned_blob, armor(valid_blob, newline=b"\r\n", final=b"\r\n")
    yield "armor_crlf_body_only", pinned_blob, BEGIN + b"\n" + encoded[:70] + b"\r\n" + encoded[70:] + b"\n" + END + b"\n"
    yield "armor_no_final_newline", pinned_blob, armor(valid_blob, final=b"")
    yield "armor_text_after_end", pinned_blob, valid + b"trailing text\n"
    yield "armor_second_signature_after", pinned_blob, valid + armor(sshsig(ed_blob(other), ed_signature(other, data)))
    yield "armor_invalid_first_valid_second", pinned_blob, armor(sshsig(ed_blob(other), ed_signature(other, data))) + valid
    yield "armor_width_64", pinned_blob, armor(valid_blob, width=64)
    yield "armor_single_line", pinned_blob, armor(valid_blob, width=0)
    yield "armor_width_1", pinned_blob, armor(valid_blob, width=1)
    yield "armor_spaces_in_base64", pinned_blob, BEGIN + b"\n" + b" ".join(encoded[i : i + 8] for i in range(0, len(encoded), 8)) + b"\n" + END + b"\n"
    yield "armor_tab_and_vt_in_base64", pinned_blob, BEGIN + b"\n" + encoded[:10] + b"\t\x0b\x0c" + encoded[10:] + b"\n" + END + b"\n"
    yield "armor_header_trailing_space", pinned_blob, BEGIN + b" \n" + encoded + b"\n" + END + b"\n"
    yield "armor_header_lowercase", pinned_blob, BEGIN.lower() + b"\n" + encoded + b"\n" + END + b"\n"
    yield "armor_end_missing", pinned_blob, BEGIN + b"\n" + encoded + b"\n"
    yield "armor_end_without_newline_before", pinned_blob, BEGIN + b"\n" + encoded + END + b"\n"
    yield "armor_end_lowercase", pinned_blob, BEGIN + b"\n" + encoded + b"\n" + END.lower() + b"\n"
    yield "armor_empty_body", pinned_blob, BEGIN + b"\n" + END + b"\n"
    yield "armor_blank_line_body", pinned_blob, BEGIN + b"\n\n" + END + b"\n"
    yield "armor_nul_in_body", pinned_blob, BEGIN + b"\n" + encoded[:10] + b"\0" + encoded[10:] + b"\n" + END + b"\n"
    yield "armor_nul_at_body_end", pinned_blob, BEGIN + b"\n" + encoded + b"\0\n" + END + b"\n"
    yield "armor_invalid_character", pinned_blob, BEGIN + b"\n" + encoded[:10] + b"*" + encoded[10:] + b"\n" + END + b"\n"
    # OpenSSH ignores the reserved field, so it can steer the base64 shape: a
    # body with '+' and '/', then each padding class (none, "==", "=").
    def body(text: bytes) -> bytes:
        return BEGIN + b"\n" + text + b"\n" + END + b"\n"

    signed = ed_signature(pinned, data)
    alphabet = sshsig(pinned_blob, signed, reserved=b"\xfb" * 6)
    assert b"+" in base64.b64encode(alphabet) and b"/" in base64.b64encode(alphabet)
    yield "armor_plus_slash", pinned_blob, armor(alphabet)
    yield "armor_url_safe_alphabet", pinned_blob, body(base64.urlsafe_b64encode(alphabet))
    for extra in range(3):
        blob = sshsig(pinned_blob, signed, reserved=b"r" * (extra + 3))
        text = base64.b64encode(blob)
        stripped = text.rstrip(b"=")
        padding = len(text) - len(stripped)
        label = f"pad{padding}"
        yield f"armor_{label}", pinned_blob, body(text)
        yield f"armor_{label}_padding_removed", pinned_blob, body(stripped)
        yield f"armor_{label}_extra_padding", pinned_blob, body(text + b"=")
        yield f"armor_{label}_double_extra_padding", pinned_blob, body(text + b"==")
        yield f"armor_{label}_text_after_padding", pinned_blob, body(text + b"AAAA")
        yield f"armor_{label}_space_before_padding", pinned_blob, body(stripped + b" " + b"=" * padding)
        yield f"armor_{label}_whitespace_after_padding", pinned_blob, body(text + b" \t")
        yield f"armor_{label}_nonzero_trailing_bits", pinned_blob, body(nonzero_bits(text))
        yield f"armor_{label}_partial_quantum", pinned_blob, body(stripped + b"A")
        yield f"armor_{label}_one_char_quantum_padded", pinned_blob, body(stripped + b"A===")
        if padding == 2:
            yield f"armor_{label}_split_padding", pinned_blob, body(stripped + b"=\n=")
            yield f"armor_{label}_single_of_double_padding", pinned_blob, body(stripped + b"=")
    yield "precheck_garbage", pinned_blob, b"not a signature\n"
    yield "precheck_vertical_tab", pinned_blob, b"\x0b" + valid
    yield "precheck_nbsp", pinned_blob, b"\xa0" + valid


def nonzero_bits(encoded: bytes) -> bytes:
    """Set the low bits b64_pton ignores in the last quantum."""
    alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    body = encoded.rstrip(b"=")
    padding = len(encoded) - len(body)
    if padding == 0:
        return encoded
    last = alphabet.index(body[-1:]) | (0b1111 if padding == 2 else 0b11)
    return body[:-1] + alphabet[last : last + 1] + b"=" * padding


def keygen_signature(private: Ed25519PrivateKey, directory: Path) -> bytes:
    path = directory / "release_key"
    path.write_bytes(private.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.OpenSSH, serialization.NoEncryption()))
    path.chmod(0o600)
    environment = {k: v for k, v in os.environ.items() if k != "SSH_AUTH_SOCK"}
    return subprocess.run(
        [rv.ssh_keygen_command(), "-Y", "sign", "-n", NAMESPACE.decode(), "-f", str(path)],
        input=MESSAGE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=True, env=environment,
    ).stdout


def verdict(public_key: bytes, signature: bytes) -> dict:
    fingerprint = rv.ssh_public_key_fingerprint(public_key)
    try:
        return {"signer": rv.verify_signature_bytes(MESSAGE, signature, public_key, fingerprint)}
    except AppError as error:
        return {"error": {"code": error.code, "message": str(error), "hint": error.hint, "details": error.details}}


def generate(scratch: Path) -> dict:
    produced = keygen_signature(key(1), scratch)
    rows = []
    for name, blob, signature in cases():
        if name == "valid_sha512" and signature != produced:
            raise SystemExit("ssh-keygen -Y sign output differs from the constructed signature")
        public_key = public_line(blob)
        rows.append({
            "name": name,
            "public_key": public_key.decode("latin-1"),
            "fingerprint": rv.ssh_public_key_fingerprint(public_key),
            "signature": base64.b64encode(signature).decode(),
            "result": verdict(public_key, signature),
        })
    names = [row["name"] for row in rows]
    assert len(names) == len(set(names)), "duplicate case names"
    return {"message": base64.b64encode(MESSAGE).decode(), "cases": rows}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--scratch", type=Path, help="directory for the throwaway signing key")
    args = parser.parse_args()
    if shutil.which("ssh-keygen") is None:
        raise SystemExit("ssh-keygen (OpenSSH 8.1 or newer) is required")
    with tempfile.TemporaryDirectory(prefix="kassiber-sshsig-vectors-", dir=args.scratch) as scratch:
        encoded = json.dumps(generate(Path(scratch)), indent=1, ensure_ascii=True) + "\n"
    if args.check:
        if DESTINATION.read_text() != encoded:
            raise SystemExit("SSHSIG vectors differ; rerun export_sshsig_vectors.py")
    else:
        DESTINATION.write_text(encoded)
