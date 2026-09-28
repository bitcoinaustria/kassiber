"""Read the payment hash out of a BOLT11 invoice, and nothing else.

Reconciliation needs one fact from a pasted invoice: which payment it names,
so it can be matched against the book's Lightning history. The encoded string
also carries a payment secret and the issuer's route hints, which the
Lightning discard policy (docs/reference/lightning-opsec.md) says to drop
after decoding, so this module returns only the payment hash, the network and
the creation time, and callers must not keep the invoice itself.

Signatures are not verified: the result only drives a lookup in local data,
and a forged invoice can only name a payment hash, never prove one.
"""

from __future__ import annotations

from dataclasses import dataclass

_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
_GENERATOR = (0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)
_CHECKSUM_WORDS = 6
_SIGNATURE_WORDS = 104
_TIMESTAMP_WORDS = 7
_PAYMENT_HASH_TAG = 1  # "p"
_PAYMENT_HASH_WORDS = 52

# Currency prefixes after "ln", longest first so "bcrt" wins over "bc".
_NETWORKS = (
    ("bcrt", "regtest"),
    ("bc", "main"),
    ("tbs", "signet"),
    ("tb", "test"),
    ("sb", "simnet"),
)


@dataclass(frozen=True)
class Bolt11Summary:
    payment_hash: str
    network: str
    timestamp: int


def _polymod(values: list[int]) -> int:
    checksum = 1
    for value in values:
        top = checksum >> 25
        checksum = (checksum & 0x1FFFFFF) << 5 ^ value
        for bit, generator in enumerate(_GENERATOR):
            if (top >> bit) & 1:
                checksum ^= generator
    return checksum


def _hrp_expand(hrp: str) -> list[int]:
    return [ord(char) >> 5 for char in hrp] + [0] + [ord(char) & 31 for char in hrp]


def _words_to_bytes(words: list[int]) -> bytes:
    acc = 0
    bits = 0
    out = bytearray()
    for word in words:
        acc = (acc << 5) | word
        bits += 5
        while bits >= 8:
            bits -= 8
            out.append((acc >> bits) & 0xFF)
    return bytes(out)


def _network(hrp: str) -> str | None:
    currency = hrp[2:]
    for prefix, network in _NETWORKS:
        if currency.startswith(prefix):
            amount = currency[len(prefix):]
            # An optional amount: digits with an optional multiplier letter.
            digits = amount[:-1] if amount[-1:] in {"m", "u", "n", "p"} else amount
            return network if digits == "" or digits.isdigit() else None
    return None


def decode_payment_hash(invoice: str) -> Bolt11Summary | None:
    """Return the invoice's payment hash, or ``None`` if it is not a valid BOLT11.

    Accepts a ``lightning:`` URI prefix and either letter case (QR codes carry
    invoices in upper case), rejects mixed case and bad checksums.
    """
    text = str(invoice or "").strip()
    if text[:10].lower() == "lightning:":
        text = text[10:]
    if text != text.lower() and text != text.upper():
        return None
    text = text.lower()
    separator = text.rfind("1")
    hrp, data = text[:separator], text[separator + 1:]
    if separator < 3 or not hrp.startswith("ln"):
        return None
    network = _network(hrp)
    if network is None:
        return None
    if any(char not in _CHARSET for char in data):
        return None
    values = [_CHARSET.index(char) for char in data]
    if len(values) < _TIMESTAMP_WORDS + _SIGNATURE_WORDS + _CHECKSUM_WORDS:
        return None
    if _polymod(_hrp_expand(hrp) + values) != 1:
        return None
    words = values[: -(_CHECKSUM_WORDS + _SIGNATURE_WORDS)]
    timestamp = 0
    for word in words[:_TIMESTAMP_WORDS]:
        timestamp = timestamp * 32 + word
    index = _TIMESTAMP_WORDS
    payment_hash: bytes | None = None
    while index + 3 <= len(words):
        tag = words[index]
        length = words[index + 1] * 32 + words[index + 2]
        field = words[index + 3 : index + 3 + length]
        index += 3 + length
        if index > len(words):
            return None
        # A reader skips a "p" field of the wrong length, per BOLT11.
        if tag == _PAYMENT_HASH_TAG and length == _PAYMENT_HASH_WORDS:
            payment_hash = _words_to_bytes(field)[:32]
    if payment_hash is None:
        return None
    return Bolt11Summary(
        payment_hash=payment_hash.hex(),
        network=network,
        timestamp=timestamp,
    )


def display_invoice(invoice: str) -> str:
    """A short, non-reusable stand-in for an invoice in results and exports."""
    text = str(invoice or "").strip()
    if text[:10].lower() == "lightning:":
        text = text[10:]
    return f"{text[:12]}…{text[-6:]}" if len(text) > 20 else text
