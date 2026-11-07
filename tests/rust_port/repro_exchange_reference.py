"""Check two exchange API reference bugs stay fixed; no sockets.

    uv run --locked python -m tests.rust_port.repro_exchange_reference

Kraken API error details repeated the server's text verbatim, so a server
that echoed the API key or secret put it into the error envelope. The key and
secret now read `<redacted>`, as in Rust. A Kraken sell whose trade row has no
`price` still gets a negative implicit rate (fixed separately). Exit 0 means
the credential echo is fixed.
"""
import base64
import json
from kassiber.core.exchange_imports import normalize_kraken_records, fetch_kraken_records
from kassiber.errors import AppError


def main():
    records = normalize_kraken_records({"result": {"ledger": {"sell": {
        "type": "trade", "asset": "XXBT", "amount": "-0.01", "refid": "trade", "time": "1700000000",
    }}}}, {"result": {"trades": {"trade": {"pair": "XXBTZEUR", "cost": "400", "fee": "1"}}}})
    assert records[0]["fiat_rate"] < 0

    class Response:
        def __enter__(self):
            return self

        def __exit__(self, *_):
            pass

        def read(self):
            return json.dumps({"error": ["EGeneral:Permission denied; synthetic-api-key"]}).encode()

    class Opener:
        def open(self, *_args, **_kwargs):
            return Response()

    try:
        fetch_kraken_records({"url": "https://unused.invalid", "token": "synthetic-api-key",
                              "auth_header": base64.b64encode(b"synthetic-secret").decode()}, opener=Opener())
    except AppError as error:
        details = json.dumps(error.details)
        assert "synthetic-api-key" not in details, details
        assert "<redacted>" in details, details
    else:
        raise AssertionError("Expected reference API error")
    print("Fixed: Kraken API error details withhold credentials. "
          "Still open: implicit Kraken sell rate is negative.")


if __name__ == "__main__":
    main()
