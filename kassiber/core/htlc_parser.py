"""HTLC script and witness parsing for cross-asset swap detection.

When a Lightning↔on-chain (or Lightning↔Liquid) submarine swap settles
on-chain, the funding output and the spending claim transaction reveal
the swap's hashlock — and, in the claim case, the preimage itself. This
module turns raw script/witness bytes into a ``payment_hash`` so the
matcher can deterministically pair the two legs of the swap across
asset boundaries.

Scope (intentionally narrow for the first cut):

* P2WSH HTLCs whose redeem script matches the Boltz v1 submarine /
  reverse-submarine template (optionally prefixed with an
  ``OP_SIZE 0x20 OP_EQUALVERIFY`` preimage-length check, which Boltz uses
  on the reverse-swap side).
* Liquid HTLCs share the same script grammar (Elements is a superset of
  Bitcoin Script), so the same parser covers them.
* Pure functions — no I/O, no SQLite, no logging. Callers feed in raw
  bytes; the parser returns a frozen ``HtlcExtraction`` or ``None``.

Out of scope here:

* Boltz v2 Taproot key-path cooperative spends — those reveal only a
  Schnorr signature on-chain, with no script or preimage. Detection is
  impossible from chain data alone; the matcher must fall back to its
  time / amount heuristic.
* Boltz v2 Taproot script-path spends — feasible but bytecode-shaped
  differently; out of scope for this PR.

Hash relationships:

* ``payment_hash`` (Lightning) ``= SHA256(preimage)`` — 32 bytes.
* ``hashlock160`` (in the HTLC redeem script) ``= HASH160(preimage) =
  RIPEMD160(SHA256(preimage)) = RIPEMD160(payment_hash)`` — 20 bytes.

So a recovered preimage gives the payment_hash directly. A funding
script alone yields only the hashlock160; the matcher must cross-check
it against a candidate ``payment_hash`` known from the Lightning leg.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Any, Mapping, Optional, Sequence

from embit import hashes as _embit_hashes


# Bitcoin Script opcodes used in the Boltz v1 HTLC redeem script.
_OP_0 = 0x00
_OP_PUSHBYTES_1 = 0x01
_OP_PUSHBYTES_75 = 0x4B
_OP_1 = 0x51
_OP_16 = 0x60
_OP_DROP = 0x75
_OP_EQUAL = 0x87
_OP_EQUALVERIFY = 0x88
_OP_SIZE = 0x82
_OP_HASH160 = 0xA9
_OP_CHECKSIG = 0xAC
_OP_CHECKLOCKTIMEVERIFY = 0xB1
_OP_IF = 0x63
_OP_ELSE = 0x67
_OP_ENDIF = 0x68

_HASHLOCK_LEN = 20
_PUBKEY_LEN = 33
_PREIMAGE_LEN = 32

_TEMPLATE_SUBMARINE = "boltz_v1_submarine"
_TEMPLATE_REVERSE = "boltz_v1_reverse"


@dataclass(frozen=True)
class HtlcExtraction:
    """Outcome of decoding an HTLC redeem script or claim witness.

    Attributes:
        template: Identifier of the script template that matched, e.g.
            ``"boltz_v1_submarine"`` or ``"boltz_v1_reverse"``.
        role: ``"fund"`` when only the HTLC-output redeem script was
            seen (no preimage available), ``"claim"`` when a spending
            witness revealed the preimage, or ``"refund"`` when a spending
            witness took the CLTV timeout branch (no preimage revealed).
        hashlock160: 40-char lowercase hex of the ``HASH160(preimage)``
            embedded in the redeem script.
        payment_hash: 64-char lowercase hex of ``SHA256(preimage)``,
            set iff the preimage was recovered (role ``"claim"``).
        preimage: 64-char lowercase hex of the preimage, set iff
            recovered.
    """

    template: str
    role: str
    hashlock160: str
    payment_hash: Optional[str] = None
    preimage: Optional[str] = None


def parse_htlc_redeem_script(script_bytes: bytes) -> Optional[HtlcExtraction]:
    """Match the Boltz v1 submarine / reverse-submarine redeem script.

    Returns an :class:`HtlcExtraction` with ``role="fund"`` on match (the
    script alone does not reveal the preimage). Returns ``None`` if the
    bytes do not parse as a recognized template.
    """
    if not script_bytes:
        return None

    cursor = 0
    template = _TEMPLATE_SUBMARINE

    # Optional ``OP_SIZE 0x20 OP_EQUALVERIFY`` prefix (reverse-swap variant).
    if (
        len(script_bytes) >= 4
        and script_bytes[cursor] == _OP_SIZE
        and script_bytes[cursor + 1] == _OP_PUSHBYTES_1
        and script_bytes[cursor + 2] == _PREIMAGE_LEN
        and script_bytes[cursor + 3] == _OP_EQUALVERIFY
    ):
        template = _TEMPLATE_REVERSE
        cursor += 4

    # OP_HASH160 <20 bytes> OP_EQUAL
    if cursor + 1 + _HASHLOCK_LEN + 1 > len(script_bytes):
        return None
    if script_bytes[cursor] != _OP_HASH160:
        return None
    cursor += 1
    if script_bytes[cursor] != _HASHLOCK_LEN:
        return None
    cursor += 1
    hashlock160_bytes = script_bytes[cursor : cursor + _HASHLOCK_LEN]
    cursor += _HASHLOCK_LEN
    if script_bytes[cursor] != _OP_EQUAL:
        return None
    cursor += 1

    # OP_IF <33 byte pubkey>
    if cursor + 2 + _PUBKEY_LEN > len(script_bytes):
        return None
    if script_bytes[cursor] != _OP_IF:
        return None
    cursor += 1
    if script_bytes[cursor] != _PUBKEY_LEN:
        return None
    cursor += 1 + _PUBKEY_LEN

    # OP_ELSE <CLTV push>
    if cursor >= len(script_bytes) or script_bytes[cursor] != _OP_ELSE:
        return None
    cursor += 1

    cltv_consumed = _consume_minimal_integer_push(script_bytes, cursor)
    if cltv_consumed is None:
        return None
    cursor += cltv_consumed

    # OP_CHECKLOCKTIMEVERIFY OP_DROP <33 byte pubkey> OP_ENDIF OP_CHECKSIG
    if cursor + 2 + 1 + _PUBKEY_LEN + 2 > len(script_bytes):
        return None
    if script_bytes[cursor] != _OP_CHECKLOCKTIMEVERIFY:
        return None
    cursor += 1
    if script_bytes[cursor] != _OP_DROP:
        return None
    cursor += 1
    if script_bytes[cursor] != _PUBKEY_LEN:
        return None
    cursor += 1 + _PUBKEY_LEN
    if script_bytes[cursor] != _OP_ENDIF:
        return None
    cursor += 1
    if script_bytes[cursor] != _OP_CHECKSIG:
        return None
    cursor += 1

    if cursor != len(script_bytes):
        return None

    return HtlcExtraction(
        template=template,
        role="fund",
        hashlock160=hashlock160_bytes.hex(),
    )


def extract_from_claim_witness(witness_items: Sequence[bytes]) -> Optional[HtlcExtraction]:
    """Decode a P2WSH HTLC claim witness and return the payment_hash.

    A successful preimage claim for a Boltz v1 HTLC stack-pushes (in
    spend order) the signature, the preimage, the truthy IF selector,
    and the redeem script. The witness items array exposes them in the
    same order, with the redeem script as the last entry per BIP141.

    Returns an :class:`HtlcExtraction` with ``role="claim"``,
    ``payment_hash`` set, and ``preimage`` set when a 32-byte preimage
    in the witness matches the script's embedded hashlock. Returns
    ``None`` when the witness is not a recognized HTLC claim.
    """
    if len(witness_items) < 2:
        return None

    redeem_script = bytes(witness_items[-1])
    fund = parse_htlc_redeem_script(redeem_script)
    if fund is None:
        return None

    expected_hash160 = bytes.fromhex(fund.hashlock160)
    for item in witness_items[:-1]:
        item_bytes = bytes(item)
        if len(item_bytes) != _PREIMAGE_LEN:
            continue
        if _embit_hashes.hash160(item_bytes) != expected_hash160:
            continue
        payment_hash = hashlib.sha256(item_bytes).digest()
        return HtlcExtraction(
            template=fund.template,
            role="claim",
            hashlock160=fund.hashlock160,
            payment_hash=payment_hash.hex(),
            preimage=item_bytes.hex(),
        )
    return None


def extract_from_refund_witness(witness_items: Sequence[bytes]) -> Optional[HtlcExtraction]:
    """Decode a P2WSH HTLC refund (CLTV timeout) witness.

    The refund branch of a Boltz v1 HTLC is spent after the timelock
    expires with a witness shaped ``<sig> <empty-selector> <redeem_script>``:
    the spender pushes an empty (falsy) item where a successful claim
    would push the 32-byte preimage, so the script's ``OP_IF`` falls
    through to the ``OP_ELSE`` timeout branch. No preimage is revealed,
    so there is no recoverable ``payment_hash`` — but recognizing the
    spend as a refund lets the matcher link an inbound refund back to the
    on-chain funding leg that paid into the HTLC.

    Returns an :class:`HtlcExtraction` with ``role="refund"`` and the
    embedded ``hashlock160`` when ``witness_items`` spends a recognized
    HTLC via its timeout branch. Returns ``None`` when the witness is not
    a recognized HTLC spend, or when it actually reveals the preimage (a
    claim — that is :func:`extract_from_claim_witness`'s job).
    """
    if len(witness_items) < 2:
        return None

    redeem_script = bytes(witness_items[-1])
    fund = parse_htlc_redeem_script(redeem_script)
    if fund is None:
        return None

    expected_hash160 = bytes.fromhex(fund.hashlock160)
    for item in witness_items[:-1]:
        item_bytes = bytes(item)
        if len(item_bytes) != _PREIMAGE_LEN:
            continue
        if _embit_hashes.hash160(item_bytes) == expected_hash160:
            # The preimage is present: this is a claim, not a refund.
            return None
    return HtlcExtraction(
        template=fund.template,
        role="refund",
        hashlock160=fund.hashlock160,
    )


def refund_funding_outpoint_from_tx_mapping(
    payload: Mapping[str, Any],
) -> tuple[str, int] | None:
    """Recover one unique v1 refund funding outpoint from stored tx JSON.

    This is the replay/backfill path for transactions imported before the
    dedicated refund-link columns existed. It accepts the public Esplora and
    decoded Electrum/Core witness keys, requires a canonical 32-byte txid plus
    output index, and declines batched refunds rather than choosing one HTLC.
    """

    nested = payload.get("tx")
    if isinstance(nested, Mapping):
        payload = nested
    vin = payload.get("vin")
    if not isinstance(vin, list):
        return None
    matches: list[tuple[str, int]] = []
    for entry in vin:
        if not isinstance(entry, Mapping):
            continue
        raw_witness = entry.get("witness")
        if raw_witness is None:
            raw_witness = entry.get("txinwitness")
        if not isinstance(raw_witness, list):
            continue
        witness_items: list[bytes] = []
        valid = True
        for item in raw_witness:
            if isinstance(item, str):
                try:
                    witness_items.append(bytes.fromhex(item))
                except ValueError:
                    valid = False
                    break
            elif isinstance(item, (bytes, bytearray)):
                witness_items.append(bytes(item))
            else:
                valid = False
                break
        if not valid or extract_from_refund_witness(witness_items) is None:
            continue
        txid = str(entry.get("txid") or "").strip().lower()
        if len(txid) != 64:
            continue
        try:
            bytes.fromhex(txid)
            vout = int(entry.get("vout"))
        except (TypeError, ValueError):
            continue
        if vout < 0:
            continue
        matches.append((txid, vout))
    unique = list(dict.fromkeys(matches))
    return unique[0] if len(unique) == 1 else None


def script_matches_payment_hash(script_bytes: bytes, payment_hash_hex: str) -> bool:
    """Verify whether an HTLC redeem script's hashlock matches a candidate
    Lightning ``payment_hash``.

    Returns ``True`` iff ``script_bytes`` parses as a recognized HTLC
    redeem script and ``RIPEMD160(payment_hash) == hashlock160``.
    """
    fund = parse_htlc_redeem_script(script_bytes)
    if fund is None:
        return False
    try:
        payment_hash_bytes = bytes.fromhex(payment_hash_hex)
    except ValueError:
        return False
    if len(payment_hash_bytes) != 32:
        return False
    computed = _embit_hashes.ripemd160(payment_hash_bytes)
    return computed.hex() == fund.hashlock160


def _consume_minimal_integer_push(script_bytes: bytes, cursor: int) -> Optional[int]:
    """Return the byte length of a minimal-encoded integer push at ``cursor``.

    Accepts the Bitcoin Script encodings used for a CLTV expiration:
    ``OP_0``, ``OP_1``..``OP_16``, or a ``OP_PUSHBYTES_N`` push of 1..5
    bytes (block height or timestamp). Returns ``None`` if the byte is
    not a minimal integer push.
    """
    if cursor >= len(script_bytes):
        return None
    head = script_bytes[cursor]
    if head == _OP_0:
        return 1
    if _OP_1 <= head <= _OP_16:
        return 1
    if _OP_PUSHBYTES_1 <= head <= _OP_PUSHBYTES_75:
        push_len = head
        # CLTV values fit comfortably in 5 bytes; reject longer pushes so we
        # do not accidentally match unrelated script shapes.
        if push_len > 5:
            return None
        if cursor + 1 + push_len > len(script_bytes):
            return None
        return 1 + push_len
    return None
