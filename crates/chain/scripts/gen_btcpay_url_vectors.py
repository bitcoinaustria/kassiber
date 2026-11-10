"""Freeze Python's BTCPay server-URL normalization for the native port.

`normalize_server_url` and `server_url_identity` follow CPython's `urlsplit`
(3.11), including its control-character, bracket and NFKC netloc checks.
"""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.backends import redact_backend_url
from kassiber.btcpay.client import is_loopback_url, is_onion_url, normalize_server_url, server_url_identity
from kassiber.errors import AppError

OUTPUT = ROOT / "crates/chain/fixtures/python_btcpay_urls.json"

INPUTS = [
    None, "", " ", "\t", "btcpay.example.com", "https://btcpay.example.com", "https://btcpay.example.com/",
    "HTTPS://BTCPay.Example.COM:443/", "http://127.0.0.1:1", "http://127.0.0.1:1/prefix/api/v1",
    "https://example.invalid/api/v1", "https://example.invalid/API/V1/stores", "https://merchant.invalid/proxy/WALLETS/w",
    "HTTPS://EXAMPLE.invalid:443/shop/stores/s?secret=discard#fragment", "https://merchant.invalid/proxy/invoices",
    "https://merchant.invalid/i/abc", "https://merchant.invalid/apps/x/pos", "https://merchant.invalid/account/apikeys",
    "https://merchant.invalid/login?ReturnUrl=%2F", "https://merchant.invalid/docs#tag/Stores",
    "https://merchant.invalid/server/users", "https://merchant.invalid/payment-requests/x",
    "https://merchant.invalid/pull-payments/x", "https://merchant.invalid/api-keys/authorize?x=1",
    "https://merchant.invalid/a/stores/b/stores/c", "https://merchant.invalid//double//", "https://merchant.invalid/x///",
    "https://[::1]:443/prefix/docs/", "https://[::1]", "https://[fe80::1%25eth0]/", "https://[v1.example]/stores/s",
    "https://[::1]unexpected/", "http://[broken/", "ftp://[broken/", "https://[]/", "https://[1.2.3.4]/",
    "missing-scheme.invalid", "ftp://merchant.invalid", "mailto:x@y", "http:///missing", "http://", "https://:443",
    "https://user:secret@merchant.invalid", "https://user@merchant.invalid", "https://:@merchant.invalid",
    "https://::@merchant.invalid", "https://@merchant.invalid", "https://a@b@merchant.invalid",
    "\x01https://merchant.invalid", " https://merchant.invalid ", "https://merch\tant.invalid/pa\nth",
    "https://merchant／invalid/", "https://merchant℀invalid/", "https://merchant.invalid。com/",
    "https://straße.example/", "https://xn--strae-oqa.example/", "http://btcpayabcdefghijklmnop.onion/",
    "http://BTCPAY.ONION:80", "https://localhost:23001", "http://127.5.5.5/", "http://[::1]:8080/x",
    "https://merchant.invalid:abc/", "https://merchant.invalid:99999/", "https://merchant.invalid:0080/",
    "https://merchant.invalid?query", "https://merchant.invalid#frag", "HtTp://Mixed.Case/Path/STORES/x",
    "https://merchant.invalid/%2Fapi/v1", "https://merchant.invalid/café/", 12345, True, "1",
    "https://merchant.invalid/\ud800", "https://merchant.invalid:443/", "http://merchant.invalid:80/",
    "https://merchant.invalid:80/", "http://merchant.invalid:443/",
    "localhost:3000", "[::1]:80", "bad host", "host:123456", "user:pw@host", "-x", "x-", "a.b_c",
    "https://u:p@h.example:0080/p?q=1#f", "https://u@[::1]:1/p", "http://h/a?b", "socks5h://127.0.0.1:9050",
    "http://12.0.0.1/", "http://128.0.0.1/", "http://btcpayonion/", "http://merchant.notonion:80/",
]


def outcome(func, value):
    try:
        return {"ok": func(value)}
    except AppError as error:
        return {"error": {"code": error.code, "message": str(error), "hint": error.hint, "details": error.details}}
    except Exception as error:  # noqa: BLE001 - frozen as the reference behaves
        return {"exception": type(error).__name__}


def render():
    cases = [{
        "input": value,
        "normalized": outcome(normalize_server_url, value),
        "identity": outcome(server_url_identity, value),
        "onion": is_onion_url(value),
        "loopback": is_loopback_url(value),
        "redacted": outcome(redact_backend_url, value),
    } for value in INPUTS]
    return json.dumps(cases, ensure_ascii=True, indent=1) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    rendered = render()
    if parser.parse_args().check:
        if OUTPUT.read_text() != rendered:
            raise SystemExit("BTCPay URL vectors differ from Python")
    else:
        OUTPUT.write_text(rendered)
    print(f"BTCPay URLs: {len(json.loads(rendered))} reference cases")


if __name__ == "__main__":
    main()
