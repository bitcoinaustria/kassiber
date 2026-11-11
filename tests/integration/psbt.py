"""A minimal BIP174 (PSBT version 0) codec for test fixtures, standard library only.

The live chain-analysis lane edits Core-made PSBTs the way a payjoin receiver
or a tampering peer would: it drops wallet metadata (global xpubs and BIP32 /
Taproot derivations) and reorders outputs. This parses the key-value maps,
keeps every other record byte for byte, and serializes them back. It never
signs, finalizes or validates anything; Core and the product do that.
"""
from __future__ import annotations

import base64
from dataclasses import dataclass, field

MAGIC = b"psbt\xff"
# Record types (BIP174/BIP371).
GLOBAL_UNSIGNED_TX = 0x00
GLOBAL_XPUB = 0x01
INPUT_BIP32_DERIVATION = 0x06
INPUT_TAP_BIP32_DERIVATION = 0x16
OUTPUT_BIP32_DERIVATION = 0x02
OUTPUT_TAP_BIP32_DERIVATION = 0x07


def _read_compact(data: bytes, offset: int) -> tuple[int, int]:
    first = data[offset]
    if first < 0xFD:
        return first, offset + 1
    width = {0xFD: 2, 0xFE: 4, 0xFF: 8}[first]
    return int.from_bytes(data[offset + 1:offset + 1 + width], "little"), offset + 1 + width


def _compact(value: int) -> bytes:
    if value < 0xFD:
        return bytes([value])
    if value <= 0xFFFF:
        return b"\xfd" + value.to_bytes(2, "little")
    if value <= 0xFFFFFFFF:
        return b"\xfe" + value.to_bytes(4, "little")
    return b"\xff" + value.to_bytes(8, "little")


def _read_map(data: bytes, offset: int) -> tuple[list[tuple[bytes, bytes]], int]:
    records = []
    while True:
        length, offset = _read_compact(data, offset)
        if length == 0:
            return records, offset
        key = data[offset:offset + length]
        offset += length
        size, offset = _read_compact(data, offset)
        records.append((key, data[offset:offset + size]))
        offset += size


def _write_map(records: list[tuple[bytes, bytes]]) -> bytes:
    out = bytearray()
    for key, value in records:
        out += _compact(len(key)) + key + _compact(len(value)) + value
    return bytes(out + b"\x00")


@dataclass
class UnsignedTx:
    """A non-witness serialized transaction, split into its parts."""

    version: bytes
    inputs: list[bytes]
    outputs: list[bytes]
    locktime: bytes

    @classmethod
    def parse(cls, raw: bytes) -> "UnsignedTx":
        offset = 4
        count, offset = _read_compact(raw, offset)
        inputs = []
        for _ in range(count):
            start = offset
            offset += 36
            script, offset = _read_compact(raw, offset)
            offset += script + 4
            inputs.append(raw[start:offset])
        count, offset = _read_compact(raw, offset)
        outputs = []
        for _ in range(count):
            start = offset
            offset += 8
            script, offset = _read_compact(raw, offset)
            offset += script
            outputs.append(raw[start:offset])
        if len(raw) - offset != 4:
            raise ValueError("unsigned transaction has trailing or missing bytes")
        return cls(raw[:4], inputs, outputs, raw[offset:])

    def serialize(self) -> bytes:
        return (self.version + _compact(len(self.inputs)) + b"".join(self.inputs)
                + _compact(len(self.outputs)) + b"".join(self.outputs) + self.locktime)


@dataclass
class Psbt:
    globals: list[tuple[bytes, bytes]]
    inputs: list[list[tuple[bytes, bytes]]] = field(default_factory=list)
    outputs: list[list[tuple[bytes, bytes]]] = field(default_factory=list)

    @classmethod
    def from_base64(cls, encoded: str) -> "Psbt":
        data = base64.b64decode(encoded)
        if not data.startswith(MAGIC):
            raise ValueError("not a PSBT")
        globals_, offset = _read_map(data, len(MAGIC))
        tx = cls._tx_of(globals_)
        inputs, outputs = [], []
        for _ in tx.inputs:
            record, offset = _read_map(data, offset)
            inputs.append(record)
        for _ in tx.outputs:
            record, offset = _read_map(data, offset)
            outputs.append(record)
        if offset != len(data):
            raise ValueError("PSBT has trailing bytes")
        return cls(globals_, inputs, outputs)

    @staticmethod
    def _tx_of(globals_: list[tuple[bytes, bytes]]) -> UnsignedTx:
        for key, value in globals_:
            if key == bytes([GLOBAL_UNSIGNED_TX]):
                return UnsignedTx.parse(value)
        raise ValueError("only PSBT version 0 (with a global unsigned transaction) is supported")

    def to_base64(self) -> str:
        out = bytearray(MAGIC) + _write_map(self.globals)
        for record in (*self.inputs, *self.outputs):
            out += _write_map(record)
        return base64.b64encode(bytes(out)).decode("ascii")

    def without_derivations(self) -> "Psbt":
        """Drop global xpubs and every BIP32/Taproot derivation, as BIP78 asks a receiver."""
        def keep(records, dropped):
            return [(key, value) for key, value in records if key[0] not in dropped]

        return Psbt(
            keep(self.globals, {GLOBAL_XPUB}),
            [keep(record, {INPUT_BIP32_DERIVATION, INPUT_TAP_BIP32_DERIVATION}) for record in self.inputs],
            [keep(record, {OUTPUT_BIP32_DERIVATION, OUTPUT_TAP_BIP32_DERIVATION}) for record in self.outputs],
        )

    def with_outputs_reversed(self) -> "Psbt":
        """Reverse the unsigned transaction's outputs together with their maps."""
        tx = self._tx_of(self.globals)
        tx.outputs.reverse()
        globals_ = [
            (key, tx.serialize() if key == bytes([GLOBAL_UNSIGNED_TX]) else value)
            for key, value in self.globals
        ]
        return Psbt(globals_, list(self.inputs), list(reversed(self.outputs)))
