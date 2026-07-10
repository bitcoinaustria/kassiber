"""Key generation, canonical signing, hashing, and wire identifiers."""

from __future__ import annotations

import base64
from dataclasses import dataclass
import hashlib
import hmac
import json
import secrets
from typing import Any, Mapping

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)


def canonical_json_bytes(value: Any) -> bytes:
    return json.dumps(
        value,
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def _b64encode(value: bytes) -> str:
    return base64.b64encode(value).decode("ascii")


def _b64decode(value: str) -> bytes:
    return base64.b64decode(value.encode("ascii"), validate=True)


@dataclass(frozen=True)
class SigningKeyPair:
    private_key_b64: str
    public_key_b64: str


@dataclass(frozen=True)
class DeviceKeyPair:
    age_identity: str
    recipient: str


def generate_signing_keypair() -> SigningKeyPair:
    private_key = Ed25519PrivateKey.generate()
    private_bytes = private_key.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    public_bytes = private_key.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    return SigningKeyPair(_b64encode(private_bytes), _b64encode(public_bytes))


def generate_device_keypair() -> DeviceKeyPair:
    from pyrage import x25519

    identity = x25519.Identity.generate()
    return DeviceKeyPair(age_identity=str(identity), recipient=str(identity.to_public()))


def sign_bytes(private_key_b64: str, payload: bytes) -> str:
    private_key = Ed25519PrivateKey.from_private_bytes(_b64decode(private_key_b64))
    return _b64encode(private_key.sign(payload))


def verify_bytes(public_key_b64: str, payload: bytes, signature_b64: str) -> bool:
    try:
        public_key = Ed25519PublicKey.from_public_bytes(_b64decode(public_key_b64))
        public_key.verify(_b64decode(signature_b64), payload)
    except (InvalidSignature, ValueError, TypeError):
        return False
    return True


def sign_canonical(private_key_b64: str, payload: Mapping[str, Any]) -> str:
    return sign_bytes(private_key_b64, canonical_json_bytes(payload))


def verify_canonical(
    public_key_b64: str,
    payload: Mapping[str, Any],
    signature_b64: str,
) -> bool:
    return verify_bytes(public_key_b64, canonical_json_bytes(payload), signature_b64)


def _domain_payload(domain: str, payload: bytes) -> bytes:
    normalized = str(domain or "").strip()
    if not normalized or "\x00" in normalized:
        raise ValueError("signature domain must be non-empty and cannot contain NUL")
    return b"kassiber-sync-signature\x00" + normalized.encode("ascii") + b"\x00" + payload


def sign_domain_bytes(private_key_b64: str, domain: str, payload: bytes) -> str:
    return sign_bytes(private_key_b64, _domain_payload(domain, payload))


def verify_domain_bytes(
    public_key_b64: str,
    domain: str,
    payload: bytes,
    signature_b64: str,
) -> bool:
    try:
        framed = _domain_payload(domain, payload)
    except (UnicodeEncodeError, ValueError):
        return False
    return verify_bytes(public_key_b64, framed, signature_b64)


def sign_domain_canonical(
    private_key_b64: str,
    domain: str,
    payload: Mapping[str, Any],
) -> str:
    return sign_domain_bytes(private_key_b64, domain, canonical_json_bytes(payload))


def verify_domain_canonical(
    public_key_b64: str,
    domain: str,
    payload: Mapping[str, Any],
    signature_b64: str,
) -> bool:
    return verify_domain_bytes(
        public_key_b64,
        domain,
        canonical_json_bytes(payload),
        signature_b64,
    )


def sha256_hex(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def event_hash(event_core: Mapping[str, Any]) -> str:
    return sha256_hex(canonical_json_bytes(event_core))


def random_book_key() -> bytes:
    return secrets.token_bytes(32)


def encode_secret(value: bytes) -> str:
    return _b64encode(value)


def decode_secret(value: str) -> bytes:
    return _b64decode(value)


def hmac_identifier(book_key: bytes, namespace: str, raw_value: str) -> str:
    message = f"{namespace}\x00{raw_value}".encode("utf-8")
    return hmac.new(book_key, message, hashlib.sha256).hexdigest()
