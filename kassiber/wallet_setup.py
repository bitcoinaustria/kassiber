from __future__ import annotations

import hashlib
import json
import re
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
_BSMS_VERSION = "BSMS 1.0"
_BSMS_TEMPLATE_TOKEN = "/**"
_BSMS_NO_PATH_RESTRICTIONS = "no path restrictions"
_BSMS_PATH_RESTRICTION_RE = re.compile(r"^/(?:\d+/)*(\*|\d+)$")

# Descriptor templates used to disambiguate a bare xpub/tpub once the caller
# supplies the script type. {branch} is 0 for receive and 1 for change, matching
# the shape produced by _descriptors_from_slip132. wallet_descriptors reuses the
# same templates (with branch="<0;1>") to build multipath plans per script type.
BARE_XPUB_TEMPLATES = {
    "p2pkh": "pkh({key}/{branch}/*)",
    "p2sh-p2wpkh": "sh(wpkh({key}/{branch}/*))",
    "p2wpkh": "wpkh({key}/{branch}/*)",
    "p2tr": "tr({key}/{branch}/*)",
}


def normalize_script_types(values: Any) -> list[str]:
    """Validate, dedupe, and sort a set of bare-xpub script types.

    Accepts a single string or an iterable of strings. Unknown types raise a
    validation error; the result is sorted for deterministic storage so the
    config_json comparison that drives re-derivation stays stable.
    """
    if values is None:
        return []
    if isinstance(values, str):
        values = [values]
    normalized: list[str] = []
    for value in values:
        script_type = str(value or "").strip().lower()
        if not script_type:
            continue
        if script_type not in BARE_XPUB_TEMPLATES:
            raise AppError(
                f"Unsupported script type '{script_type}'",
                code="validation",
                hint="Supported script types are p2pkh, p2sh-p2wpkh, p2wpkh, and p2tr.",
            )
        if script_type not in normalized:
            normalized.append(script_type)
    return sorted(normalized)


def normalize_wallet_material(
    value: str,
    *,
    script_type: str | None = None,
    script_types: Any = None,
) -> dict[str, Any]:
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
    parsed = parse_bsms_descriptor_record(material)
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
        resolved_types = normalize_script_types(script_types)
        if resolved_types:
            # Reject a malformed key before we record it; descriptor rendering
            # is deferred to load_descriptor_plan (single source of truth).
            _base58check_decode(material)
            return {"xpub": material, "script_types": resolved_types}
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
        hint="Paste a descriptor, a BSMS descriptor record, Bitcoin Core descriptor JSON, Sparrow-style descriptor text, or a ypub/zpub/upub/vpub key.",
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


def parse_bsms_descriptor_record(material: str) -> dict[str, str] | None:
    """Extract concrete receive/change descriptors from a plaintext BSMS record.

    BIP-129 descriptor records are four logical lines:

    ``BSMS 1.0``, descriptor/template, path restrictions, and first address.
    Signer key records share the version line but put the token on line 2; those
    are intentionally rejected so users do not import the wrong BSMS artifact.
    Encrypted ``.dat`` records need a setup token and are out of scope here.
    """
    lines = _wallet_material_lines(material)
    if not lines or lines[0].casefold() != _BSMS_VERSION.casefold():
        return None
    if len(lines) < 4:
        raise AppError(
            "Incomplete BSMS descriptor record",
            code="validation",
            hint="Paste the plaintext coordinator descriptor record, not an encrypted .dat payload.",
        )
    descriptor = lines[1]
    if not _looks_like_descriptor(descriptor):
        raise AppError(
            "BSMS key records are not wallet descriptors",
            code="validation",
            hint="Paste the coordinator descriptor record, not a signer key record.",
        )
    restrictions_line = lines[2]
    if restrictions_line.casefold() == _BSMS_NO_PATH_RESTRICTIONS:
        if _BSMS_TEMPLATE_TOKEN in descriptor:
            raise AppError(
                "BSMS descriptor template requires path restrictions",
                code="validation",
            )
        return {"descriptor": descriptor}
    restrictions = _parse_bsms_path_restrictions(restrictions_line)
    if _BSMS_TEMPLATE_TOKEN not in descriptor:
        raise AppError(
            "BSMS path restrictions require a descriptor template containing /**",
            code="validation",
        )
    if len(restrictions) > 2:
        raise AppError(
            "BSMS records with more than receive/change path restrictions are not supported",
            code="validation",
        )
    expanded = [
        descriptor.replace(_BSMS_TEMPLATE_TOKEN, restriction)
        for restriction in restrictions
    ]
    result = {"descriptor": expanded[0]}
    if len(expanded) > 1:
        result["change_descriptor"] = expanded[1]
    return result


def _parse_bsms_path_restrictions(value: str) -> list[str]:
    restrictions = [part.strip() for part in value.split(",") if part.strip()]
    if not restrictions:
        raise AppError("BSMS descriptor record has no path restrictions", code="validation")
    for restriction in restrictions:
        if not _BSMS_PATH_RESTRICTION_RE.fullmatch(restriction):
            raise AppError(
                "BSMS path restrictions must be non-hardened paths like /0/*",
                code="validation",
            )
        if "*/" in restriction:
            raise AppError(
                "BSMS path restriction wildcard must be the final segment",
                code="validation",
            )
    return restrictions


def _first_string(payload: dict[str, Any], *keys: str) -> str | None:
    for key in keys:
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def _descriptors_from_text(material: str) -> dict[str, str] | None:
    lines = _wallet_material_lines(material)
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
            "elwpkh(",
            "elwsh(",
            "elsh(",
            "eltr(",
        )
    )


def _wallet_material_lines(material: str) -> list[str]:
    return [
        line.strip()
        for line in material.replace("\r", "\n").split("\n")
        if line.strip() and not line.strip().startswith("#")
    ]


def _descriptors_from_bare_xpub(material: str, script_type: str) -> dict[str, str]:
    template = BARE_XPUB_TEMPLATES.get(script_type)
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


__all__ = [
    "BARE_XPUB_TEMPLATES",
    "normalize_script_types",
    "normalize_wallet_material",
    "parse_bsms_descriptor_record",
]
