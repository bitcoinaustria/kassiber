"""Wasabi numeric amounts have satoshi units, never type-guessed BTC units."""

from decimal import Decimal

import pytest

from kassiber.errors import AppError
from kassiber.importers import _wasabi_amount_sats, load_wasabi_bundle


@pytest.mark.parametrize("value", [0.1, -0.1, 1.5, Decimal("0.1"), float("inf"), True])
def test_noninteger_numeric_wasabi_amount_is_refused(value):
    with pytest.raises(AppError):
        _wasabi_amount_sats(value)


@pytest.mark.parametrize("value,expected", [(1, 1), (1.0, 1), (Decimal("1.0"), 1),
                                            ("1", 1), ("0.1", 10_000_000)])
def test_integral_numbers_are_sats_and_explicit_decimal_strings_remain_btc(value, expected):
    assert _wasabi_amount_sats(value) == expected


def test_file_json_fractional_amount_does_not_import_as_btc(tmp_path):
    path = tmp_path / "wasabi.json"
    path.write_text('{"gethistory":[{"txid":"test", "amount":0.1,"datetime":"2025-01-01T00:00:00Z"}]}')
    with pytest.raises(AppError):
        load_wasabi_bundle(str(path))
