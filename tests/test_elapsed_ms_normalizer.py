"""The parity tests' `elapsed_ms` normalizer checks the value before fixing it."""
from __future__ import annotations

import pytest

from tests.rust_sync_parity import ELAPSED_MS_FIXED, fixed_elapsed_ms, fixed_elapsed_ms_bytes, fixed_elapsed_ms_text


def test_only_elapsed_ms_is_fixed_and_only_after_its_check():
    outcome = {"data": [{"elapsed_ms": 387, "imported": 2, "fetch_ms": 9, "nested": {"elapsed_ms": 0}}]}
    assert fixed_elapsed_ms(outcome) == {
        "data": [{"elapsed_ms": ELAPSED_MS_FIXED, "imported": 2, "fetch_ms": 9, "nested": {"elapsed_ms": ELAPSED_MS_FIXED}}]
    }
    assert outcome["data"][0]["elapsed_ms"] == 387, "the input is not modified"
    assert fixed_elapsed_ms_bytes(b'{"elapsed_ms": 41, "fetch_ms": 3}') == b'{"elapsed_ms": %d, "fetch_ms": 3}' % ELAPSED_MS_FIXED
    stored = '{"backend":"fixture","elapsed_ms":1188,"imported":0}'
    assert fixed_elapsed_ms_text(stored) == '{"backend":"fixture","elapsed_ms":%d,"imported":0}' % ELAPSED_MS_FIXED


@pytest.mark.parametrize("bad", [-1, 1.5, "12", None, True])
def test_a_negative_or_non_integer_elapsed_ms_still_fails(bad):
    with pytest.raises(AssertionError, match="elapsed_ms"):
        fixed_elapsed_ms({"data": [{"elapsed_ms": bad}]})


@pytest.mark.parametrize("raw", [b"-1", b"1.5", b'"12"', b"null", b"true"])
def test_a_negative_or_non_integer_elapsed_ms_still_fails_in_raw_output(raw):
    with pytest.raises(AssertionError, match="elapsed_ms"):
        fixed_elapsed_ms_bytes(b'{"elapsed_ms": ' + raw + b', "imported": 2}')
    with pytest.raises(AssertionError, match="elapsed_ms"):
        fixed_elapsed_ms_text('{"elapsed_ms":' + raw.decode() + ',"imported":2}')
