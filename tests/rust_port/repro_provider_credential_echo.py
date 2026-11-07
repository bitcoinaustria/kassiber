"""Python reference repro: BTCPay and LND errors repeat an echoed credential.

    uv run --locked python -m tests.rust_port.repro_provider_credential_echo

Found while withholding exchange API credentials (Kraken, Coinbase, Binance)
that a server echoes in an error. Other providers that send a credential copy
the server's error text into the error with no literal scrub: a BTCPay
Greenfield 500 whose `message` repeats the API key, and an LND HTTP error body
that repeats the macaroon (its first 200 characters). Pattern redaction does
not recognize either. CLN (`details.stderr`), Esplora and Bitcoin Core RPC
bodies (`http_client.request_with_retry` without `withheld`) and AI provider
bodies (`provider_error_body_preview`) are the same shape. BTCPay syncs run as
freshness jobs, whose failure message is written to `freshness_jobs.error_json`
after pattern redaction only, so by reading the code the key can reach the
book on disk. No sockets; not fixed here. Exit 0 means the bug still
reproduces.
"""
from email.message import Message
import io
import json
from urllib import error as urlerror

from kassiber.btcpay.client import GreenfieldClient
from kassiber.core.lightning import lnd
from kassiber.errors import AppError

BTCPAY_KEY = "0123456789abcdef0123456789abcdef01234567"
MACAROON = "0201036c6e6402" + "ab" * 40


def http_error(url, status, body):
    return urlerror.HTTPError(url, status, "error", Message(), io.BytesIO(json.dumps(body).encode()))


def main():
    class BtcpayOpener:
        def open(self, request, timeout=0):  # noqa: ARG002
            raise http_error(request.full_url, 500, {"message": f"bad key {BTCPAY_KEY}"})

    client = GreenfieldClient.from_backend(
        {"url": "https://btcpay.invalid", "token": BTCPAY_KEY}, opener=BtcpayOpener(),
    )
    try:
        client.get_json("/api/v1/stores")
    except AppError as error:
        assert BTCPAY_KEY in str(error), str(error)
        print(f"BTCPay: {error.code}: message repeats the API key")
    else:
        raise SystemExit("Reference behavior changed: BTCPay accepted")

    def lnd_open(request, **_kwargs):
        raise http_error(request.full_url, 500, {"message": f"bad macaroon {MACAROON}"})

    lnd.urlopen_with_proxy = lnd_open
    try:
        lnd.LndRestClient({"url": "https://lnd.invalid", "token": MACAROON}).request("GET", "/v1/getinfo")
    except AppError as error:
        assert MACAROON[:64] in str(error), str(error)
        print(f"LND: {error.code}: message repeats the macaroon")
    else:
        raise SystemExit("Reference behavior changed: LND accepted")


if __name__ == "__main__":
    main()
