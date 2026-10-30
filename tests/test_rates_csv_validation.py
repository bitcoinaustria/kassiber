"""Reject negative Kraken prices before they enter the shared rate cache."""
import pytest

from kassiber.core.rates import _parse_kraken_csv_row
from kassiber.errors import AppError


@pytest.mark.parametrize("column", [1, 2, 3, 4])
def test_kraken_ohlc_prices_must_be_positive(column):
    row = ["1714521600", "100", "110", "90", "105", "1", "2"]
    row[column] = "-1"
    with pytest.raises(AppError, match="OHLC prices must be positive"):
        _parse_kraken_csv_row(row, "XBTEUR_1.csv", 1, 1)


@pytest.mark.parametrize("column", [1, 2, 3, 4, 5])
@pytest.mark.parametrize("value", ["NaN", "Infinity", "1e999", "1e-999999999"])
def test_kraken_prices_and_volume_are_bounded_before_comparison(column, value):
    row = ["1714521600", "100", "110", "90", "105", "1", "2"]
    row[column] = value
    with pytest.raises(AppError) as error:
        _parse_kraken_csv_row(row, "XBTEUR_1.csv", 1, 1)
    assert error.value.code == "validation"
