from __future__ import annotations

"""Fail-closed wallet-material checks shared by every descriptor ingress."""

from importlib import import_module
from typing import Any

from .errors import AppError


SPENDING_PRIVATE_MATERIAL_CODE = "wallet_spending_private_material"
SPENDING_PRIVATE_MATERIAL_HINT = (
    "Export a watch-only descriptor or extended public key. "
    "Liquid blinding/view keys may remain private because they cannot spend."
)


def spending_private_material_error() -> AppError:
    """Return the stable, secret-free error used for all spending-key rejects."""

    return AppError(
        "Spending-private wallet material is not accepted",
        code=SPENDING_PRIVATE_MATERIAL_CODE,
        hint=SPENDING_PRIVATE_MATERIAL_HINT,
        retryable=False,
    )


def reject_spending_private_material() -> None:
    raise spending_private_material_error()


def assert_descriptor_is_watch_only(descriptor: Any) -> None:
    """Reject parsed descriptor spending keys while ignoring Liquid view keys."""

    for key in tuple(getattr(descriptor, "keys", None) or ()):
        if bool(getattr(key, "is_private", False)):
            reject_spending_private_material()


def assert_descriptor_text_is_watch_only(value: Any) -> None:
    """Reject private keys in any descriptor text that parses successfully.

    Wallet-export normalization historically preserves syntactically shaped
    descriptors with placeholder public keys.  Those remain the strict live
    loader's responsibility.  This preflight only claims the narrower security
    property: whenever embit can parse the value, no spending key may be
    private.
    """

    text = str(value or "").strip()
    if not text:
        return
    liquid_text = text
    if liquid_text.startswith("ct("):
        liquid_text = f"blinded({liquid_text[3:]}"
    for source, target in {
        "elwpkh(": "wpkh(",
        "elwsh(": "wsh(",
        "elsh(": "sh(",
        "eltr(": "tr(",
    }.items():
        liquid_text = liquid_text.replace(source, target)
    for module_name, class_name, candidate in (
        ("embit.descriptor", "Descriptor", text),
        ("embit.liquid.descriptor", "LDescriptor", liquid_text),
    ):
        try:
            descriptor_class = getattr(import_module(module_name), class_name)
            descriptor = descriptor_class.from_string(candidate)
        except Exception:
            continue
        assert_descriptor_is_watch_only(descriptor)
        return


def assert_standalone_key_is_watch_only(value: Any) -> None:
    """Reject a valid standalone WIF or extended private key.

    Invalid/non-key text is left to the caller's format-specific parser. This
    helper exists so a bare xprv/tprv/WIF gets the same typed error as a key
    nested in a parsed descriptor, without relying on textual prefix checks.
    """

    text = str(value or "").strip()
    if not text or any(char.isspace() for char in text):
        return
    try:
        descriptor_key = import_module("embit.descriptor.arguments").Key.from_string(
            text
        )
    except Exception:
        return
    if bool(getattr(descriptor_key, "is_private", False)):
        reject_spending_private_material()


__all__ = [
    "SPENDING_PRIVATE_MATERIAL_CODE",
    "SPENDING_PRIVATE_MATERIAL_HINT",
    "assert_descriptor_is_watch_only",
    "assert_descriptor_text_is_watch_only",
    "assert_standalone_key_is_watch_only",
    "reject_spending_private_material",
    "spending_private_material_error",
]
