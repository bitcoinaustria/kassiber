from __future__ import annotations

"""Low-level address-to-script helpers shared by wallet source checks."""

from ..errors import AppError

B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
B58_INDEX = {char: index for index, char in enumerate(B58_ALPHABET)}
BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
BECH32_INDEX = {char: index for index, char in enumerate(BECH32_CHARSET)}


def sha256d(payload: bytes) -> bytes:
    import hashlib

    return hashlib.sha256(hashlib.sha256(payload).digest()).digest()


def base58check_decode(value: str) -> bytes:
    number = 0
    for char in value:
        if char not in B58_INDEX:
            raise AppError(f"Unsupported base58 address: {value}")
        number = number * 58 + B58_INDEX[char]
    raw = number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    leading_zeros = len(value) - len(value.lstrip("1"))
    payload = (b"\x00" * leading_zeros) + raw
    if len(payload) < 5:
        raise AppError(f"Unsupported base58 address: {value}")
    body, checksum = payload[:-4], payload[-4:]
    if sha256d(body)[:4] != checksum:
        raise AppError(f"Invalid base58 checksum for address: {value}")
    return body


def bech32_polymod(values: list[int]) -> int:
    generators = (0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)
    checksum = 1
    for value in values:
        top = checksum >> 25
        checksum = ((checksum & 0x1FFFFFF) << 5) ^ value
        for index, generator in enumerate(generators):
            if (top >> index) & 1:
                checksum ^= generator
    return checksum


def bech32_hrp_expand(hrp: str) -> list[int]:
    return [ord(char) >> 5 for char in hrp] + [0] + [ord(char) & 31 for char in hrp]


def bech32_decode(value: str) -> tuple[str, list[int], str]:
    if value.lower() != value and value.upper() != value:
        raise AppError(f"Invalid bech32 address casing: {value}")
    normalized = value.lower()
    separator = normalized.rfind("1")
    if separator < 1 or separator + 7 > len(normalized):
        raise AppError(f"Unsupported bech32 address: {value}")
    hrp = normalized[:separator]
    data = []
    for char in normalized[separator + 1 :]:
        if char not in BECH32_INDEX:
            raise AppError(f"Unsupported bech32 address: {value}")
        data.append(BECH32_INDEX[char])
    polymod = bech32_polymod(bech32_hrp_expand(hrp) + data)
    if polymod == 1:
        spec = "bech32"
    elif polymod == 0x2BC830A3:
        spec = "bech32m"
    else:
        raise AppError(f"Invalid bech32 checksum for address: {value}")
    return hrp, data[:-6], spec


def convertbits(data: list[int], from_bits: int, to_bits: int, pad: bool = True) -> list[int]:
    accumulator = 0
    bits = 0
    output = []
    max_value = (1 << to_bits) - 1
    max_accumulator = (1 << (from_bits + to_bits - 1)) - 1
    for value in data:
        if value < 0 or value >> from_bits:
            raise AppError("Invalid bit group in address encoding")
        accumulator = ((accumulator << from_bits) | value) & max_accumulator
        bits += from_bits
        while bits >= to_bits:
            bits -= to_bits
            output.append((accumulator >> bits) & max_value)
    if pad:
        if bits:
            output.append((accumulator << (to_bits - bits)) & max_value)
    elif bits >= from_bits or ((accumulator << (to_bits - bits)) & max_value):
        raise AppError("Invalid address padding")
    return output


def address_to_scriptpubkey(address: str) -> bytes:
    if address.lower().startswith(("bc1", "tb1", "bcrt1")):
        _, data, spec = bech32_decode(address)
        if not data:
            raise AppError(f"Invalid segwit address: {address}")
        version = data[0]
        if version > 16:
            raise AppError(f"Unsupported segwit witness version for address: {address}")
        program = bytes(convertbits(data[1:], 5, 8, pad=False))
        if len(program) < 2 or len(program) > 40:
            raise AppError(f"Invalid segwit program length for address: {address}")
        if version == 0 and spec != "bech32":
            raise AppError(f"Invalid bech32 checksum type for address: {address}")
        if version > 0 and spec != "bech32m":
            raise AppError(f"Invalid bech32m checksum type for address: {address}")
        opcode = 0 if version == 0 else 0x50 + version
        return bytes([opcode, len(program)]) + program
    payload = base58check_decode(address)
    version = payload[0]
    hash160 = payload[1:]
    if len(hash160) != 20:
        raise AppError(f"Unsupported base58 payload length for address: {address}")
    if version in {0x00, 0x6F}:
        return bytes.fromhex("76a914") + hash160 + bytes.fromhex("88ac")
    if version in {0x05, 0xC4}:
        return bytes.fromhex("a914") + hash160 + bytes.fromhex("87")
    raise AppError(f"Unsupported address version for address: {address}")


def scriptpubkey_for_address_or_none(address: str | None) -> str | None:
    if not address:
        return None
    try:
        return address_to_scriptpubkey(address).hex()
    except AppError:
        return None
