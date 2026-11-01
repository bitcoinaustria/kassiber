from kassiber.core.lightning import cln


def test_nested_funding_output_zero_is_an_outpoint():
    assert cln._channel_funding_outpoint({"funding": {"txid": "ab" * 32, "output": 0}}) == "ab" * 32 + ":0"
