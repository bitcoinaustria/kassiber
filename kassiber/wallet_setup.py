from __future__ import annotations

import hashlib
import json
from typing import Any

from .errors import AppError


_BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
_BASE58_INDEX = {char: index for index, char in enumerate(_BASE58_ALPHABET)}

_SLIP132_VERSIONS = {
    "ypub": ("xpub", "sh(wpkh({key}/{branch}/*))"),
    "zpub": ("xpub", "wpkh({key}/{branch}/*)"),
    "upub": ("tpub", "sh(wpkh({key}/{branch}/*))"),
    "vpub": ("tpub", "wpkh({key}/{branch}/*)"),
}

_XPUB_VERSION_BYTES = {
    "xpub": bytes.fromhex("0488b21e"),
    "tpub": bytes.fromhex("043587cf"),
}

# Descriptor templates used to disambiguate a bare xpub/tpub once the caller
# supplies the script type. {branch} is 0 for receive and 1 for change, matching
# the shape produced by _descriptors_from_slip132.
_BARE_XPUB_TEMPLATES = {
    "p2pkh": "pkh({key}/{branch}/*)",
    "p2sh-p2wpkh": "sh(wpkh({key}/{branch}/*))",
    "p2wpkh": "wpkh({key}/{branch}/*)",
    "p2tr": "tr({key}/{branch}/*)",
}


def normalize_wallet_material(value: str, *, script_type: str | None = None) -> dict[str, str]:
    material = value.strip()
    if not material:
        raise AppError(
            "Wallet export is required",
            code="validation",
            hint="Paste a descriptor, descriptor export, or supported extended public key.",
        )
    json_payload = _parse_json_material(material)
    if json_payload is not None:
        parsed = _descriptors_from_json(json_payload)
        if parsed:
            return parsed
    parsed = _descriptors_from_text(material)
    if parsed:
        return parsed
    parsed = _descriptors_from_slip132(material)
    if parsed:
        return parsed
    prefix = material[:4]
    if prefix in {"xpub", "tpub"}:
        if script_type:
            return _descriptors_from_bare_xpub(material, script_type)
        raise AppError(
            "Bare xpub/tpub is ambiguous",
            code="validation",
            hint="Paste an output descriptor or a wallet export that includes the script type.",
        )
    raise AppError(
        "Unsupported wallet export format",
        code="validation",
        hint="Paste a descriptor, a Bitcoin Core descriptor export, Sparrow-style descriptor text, or a ypub/zpub/upub/vpub key.",
    )


def _parse_json_material(material: str) -> Any | None:
    try:
        return json.loads(material)
    except json.JSONDecodeError:
        return None


def _descriptors_from_json(payload: Any) -> dict[str, str] | None:
    if not isinstance(payload, dict):
        return None
    receive = _first_string(
        payload,
        "descriptor",
        "receive_descriptor",
        "external_descriptor",
        "receive",
        "external",
    )
    change = _first_string(
        payload,
        "change_descriptor",
        "internal_descriptor",
        "change",
        "internal",
    )
    descriptors = payload.get("descriptors")
    if isinstance(descriptors, list):
        for item in descriptors:
            if not isinstance(item, dict):
                continue
            descriptor = _first_string(item, "desc", "descriptor")
            if not descriptor:
                continue
            internal = bool(item.get("internal"))
            active = item.get("active")
            if active is False:
                continue
            if internal and not change:
                change = descriptor
            elif not internal and not receive:
                receive = descriptor
    if receive:
        result = {"descriptor": receive}
        if change:
            result["change_descriptor"] = change
        return result
    return None


def _first_string(payload: dict[str, Any], *keys: str) -> str | None:
    for key in keys:
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def _descriptors_from_text(material: str) -> dict[str, str] | None:
    lines = [
        line.strip()
        for line in material.replace("\r", "\n").split("\n")
        if line.strip() and not line.strip().startswith("#")
    ]
    receive = None
    change = None
    descriptors = []
    for line in lines:
        key, sep, value = line.partition("=")
        if not sep:
            key, sep, value = line.partition(":")
        if sep:
            normalized_key = key.strip().lower().replace("-", "_")
            normalized_value = value.strip()
            if normalized_key in {"descriptor", "receive", "receive_descriptor", "external", "external_descriptor"}:
                receive = normalized_value
                continue
            if normalized_key in {"change", "change_descriptor", "internal", "internal_descriptor"}:
                change = normalized_value
                continue
        if _looks_like_descriptor(line):
            descriptors.append(line)
    if not receive and descriptors:
        receive = descriptors[0]
    if not change and len(descriptors) > 1:
        change = descriptors[1]
    if receive:
        result = {"descriptor": receive}
        if change:
            result["change_descriptor"] = change
        return result
    return None


def _looks_like_descriptor(value: str) -> bool:
    lowered = value.lower()
    return any(
        lowered.startswith(prefix)
        for prefix in (
            "pkh(",
            "wpkh(",
            "sh(",
            "wsh(",
            "tr(",
            "combo(",
            "addr(",
            "raw(",
            "ct(",
        )
    )


def _descriptors_from_bare_xpub(material: str, script_type: str) -> dict[str, str]:
    template = _BARE_XPUB_TEMPLATES.get(script_type)
    if template is None:
        raise AppError(
            f"Unsupported script type '{script_type}'",
            code="validation",
            hint="Supported script types are p2pkh, p2sh-p2wpkh, p2wpkh, and p2tr.",
        )
    # Reject a malformed key before we wrap it in a descriptor.
    _base58check_decode(material)
    return {
        "descriptor": template.format(key=material, branch=0),
        "change_descriptor": template.format(key=material, branch=1),
    }


def _descriptors_from_slip132(material: str) -> dict[str, str] | None:
    key = material.strip()
    prefix = key[:4]
    if prefix not in _SLIP132_VERSIONS:
        return None
    target_prefix, template = _SLIP132_VERSIONS[prefix]
    converted = _convert_extended_key_prefix(key, target_prefix)
    return {
        "descriptor": template.format(key=converted, branch=0),
        "change_descriptor": template.format(key=converted, branch=1),
    }


def _convert_extended_key_prefix(key: str, target_prefix: str) -> str:
    raw = _base58check_decode(key)
    if len(raw) < 4:
        raise AppError("Invalid extended public key", code="validation")
    payload = _XPUB_VERSION_BYTES[target_prefix] + raw[4:]
    return _base58check_encode(payload)


def _base58check_decode(value: str) -> bytes:
    number = 0
    for char in value:
        if char not in _BASE58_INDEX:
            raise AppError("Invalid base58 extended public key", code="validation")
        number = number * 58 + _BASE58_INDEX[char]
    raw = number.to_bytes((number.bit_length() + 7) // 8, "big")
    leading_zeroes = len(value) - len(value.lstrip("1"))
    decoded = b"\x00" * leading_zeroes + raw
    if len(decoded) < 5:
        raise AppError("Invalid base58check payload", code="validation")
    payload, checksum = decoded[:-4], decoded[-4:]
    if _checksum(payload) != checksum:
        raise AppError("Invalid extended public key checksum", code="validation")
    return payload


def _base58check_encode(payload: bytes) -> str:
    raw = payload + _checksum(payload)
    number = int.from_bytes(raw, "big")
    chars = []
    while number:
        number, remainder = divmod(number, 58)
        chars.append(_BASE58_ALPHABET[remainder])
    encoded = "".join(reversed(chars)) or "1"
    leading_zeroes = len(raw) - len(raw.lstrip(b"\x00"))
    return "1" * leading_zeroes + encoded


def _checksum(payload: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]


__all__ = ["normalize_wallet_material"]
