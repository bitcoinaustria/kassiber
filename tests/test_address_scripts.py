import unittest

from kassiber.core.address_scripts import (
    BECH32_CHARSET,
    address_to_scriptpubkey,
    bech32_hrp_expand,
    bech32_polymod,
    convertbits,
    scriptpubkey_for_address_or_none,
)
from kassiber.errors import AppError


def _bech32_address(hrp, data):
    values = bech32_hrp_expand(hrp) + data
    polymod = bech32_polymod(values + [0, 0, 0, 0, 0, 0]) ^ 1
    checksum = [(polymod >> 5 * (5 - index)) & 31 for index in range(6)]
    return f"{hrp}1{''.join(BECH32_CHARSET[item] for item in data + checksum)}"


class AddressScriptsTest(unittest.TestCase):
    def test_segwit_v0_rejects_valid_checksum_invalid_program_length(self):
        invalid_v0 = _bech32_address(
            "bc",
            [0] + convertbits(list(b"\x01\x02"), 8, 5, pad=True),
        )

        with self.assertRaises(AppError):
            address_to_scriptpubkey(invalid_v0)
        self.assertIsNone(scriptpubkey_for_address_or_none(invalid_v0))

    def test_segwit_v0_accepts_p2wpkh_program_length(self):
        valid_v0 = _bech32_address(
            "bc",
            [0] + convertbits(list(bytes(range(20))), 8, 5, pad=True),
        )

        script = address_to_scriptpubkey(valid_v0)
        self.assertEqual(script[:2], bytes([0, 20]))
        self.assertEqual(len(script), 22)


if __name__ == "__main__":
    unittest.main()
