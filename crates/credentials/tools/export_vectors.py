"""Export kassiber/redaction.py's credential scrub outputs as Rust test vectors.

Run from the repository root:

    uv run --locked python crates/credentials/tools/export_vectors.py [--check]

A test/development tool, not a runtime dependency. The inputs are seeded, so
the file only changes when the Python scrub does.
"""
from __future__ import annotations

import argparse
import base64
import json
import random
from pathlib import Path
from urllib.parse import quote

from kassiber.redaction import (
    authorization_credentials,
    backend_credentials,
    credential_forms,
    header_credentials,
    request_credentials,
    url_credentials,
    withhold_credentials,
)

DESTINATION = Path(__file__).resolve().parents[1] / "tests/data/vectors.json"
ALPHABET = "abcXYZ019+/=_.-~ %:@?#[]&\"\\'\t\né€😀\x1c\x7f"


def _text(rng, low=1, high=12, alphabet=ALPHABET):
    return "".join(rng.choice(alphabet) for _ in range(rng.randint(low, high)))


def _credential(rng):
    kind = rng.randrange(5)
    if kind == 0:
        return "".join(rng.choice("0123456789abcdefABCDEF") for _ in range(rng.randint(1, 20)))
    if kind == 1:
        return base64.b64encode(rng.randbytes(rng.randint(1, 30))).decode()
    if kind == 2:
        return _text(rng, 0, 3)
    return _text(rng)


def _spellings(credential):
    escaped = json.dumps(credential)[1:-1]
    return [credential, quote(credential, safe=""), escaped, credential.upper(), escaped.replace("/", "\\/")]


def _url(rng):
    scheme = rng.choice(["https", "http", "socks5h", "", "1x", "a+b", "HTTP"])
    user = rng.choice(["", _text(rng), "user", "%41%zz", "u%C3%A9", "t%FF%FE"])
    password = rng.choice(["", _text(rng), "p%2Fw", "pa:ss", "%E2%82"])
    userinfo = rng.choice(["", user, f"{user}:{password}", f"{user}:", f":{password}"])
    host = rng.choice(["host", "127.0.0.1", "[::1]", "[v1.x]", "[1.2.3.4]", "[fe80::1%eth0]", "[::1", "h]",
                       "b\u00fccher.de", "a\uff0fb", "x\u2100y", "[g::1]", "h:80", ""])
    query = "&".join(
        f"{rng.choice(['api_key', 'apikey', 'Token', 'x-api-key', 'addr', 'sig', 'AccessKey', 'auth+x', 'k%65y', ''])}"
        f"={rng.choice(['', _text(rng), 'v%41+1', 'q'])}"
        for _ in range(rng.randint(0, 3))
    )
    url = f"{scheme}://" if scheme else rng.choice(["", "//"])
    if userinfo:
        url += f"{userinfo}@"
    url += host + rng.choice(["", "/api", "/p?x#f"])
    if query:
        url += "?" + query
    if rng.random() < 0.2:
        url += "#" + _text(rng)
    if rng.random() < 0.1:
        url = rng.choice([" \x01", "\t"]) + url + "\n"
    return url


def _authorization(rng):
    pair = f"{_text(rng, 0, 6)}:{_text(rng, 0, 6)}"
    encoded = base64.b64encode(pair.encode()).decode()
    return rng.choice([
        f"Basic {encoded}", f"basic  {encoded} ", f"BASIC {encoded.rstrip('=')}", f"Basic {encoded}=",
        f"Basic {encoded[:-1]}", f"Bearer {_text(rng)}", _text(rng), f"Basic {_text(rng)}",
        f"Basic {base64.b64encode(b'no-colon').decode()}", f"Basic {base64.b64encode(bytes([255, 58, 65])).decode()}",
        "Basic YWJj=", "Basic YQ=", "Basic YWI", "",
    ])


def vectors():
    rng = random.Random(20261107)
    cases = {"forms": [], "withhold": [], "authorization": [], "url": [], "headers": [], "backend": []}
    for _ in range(400):
        credentials = [_credential(rng) for _ in range(rng.randint(0, 4))]
        cases["forms"].append({"credentials": credentials, "forms": list(credential_forms(credentials))})
        spellings = [s for credential in credentials for s in _spellings(credential)]
        parts = [_text(rng, 0, 6) for _ in range(len(spellings) + 1)]
        text = parts[0] + "".join(s + p for s, p in zip(spellings, parts[1:]))
        cases["withhold"].append({"credentials": credentials, "text": text, "result": withhold_credentials(text, credentials)})
    for _ in range(400):
        value = _authorization(rng)
        cases["authorization"].append({"value": value, "result": authorization_credentials(value)})
    for _ in range(1200):
        url = _url(rng)
        default = rng.choice([None, "socks5h"])
        cases["url"].append({"url": url, "default_scheme": default, "result": url_credentials(url, default_scheme=default)})
    for _ in range(200):
        headers = [
            [rng.choice(["Authorization", "proxy-AUTHORIZATION", "X-MBX-APIKEY", "Accept", "X-Secret-Thing",
                         "Grpc-Metadata-macaroon", "Cookie", "User-Agent", "x_api-key", "Rune"]), _authorization(rng)]
            for _ in range(rng.randint(0, 4))
        ]
        url = _url(rng) if rng.random() < 0.5 else None
        proxy = _url(rng) if rng.random() < 0.5 else None
        cases["headers"].append({
            "headers": headers, "url": url, "proxy_url": proxy,
            "header_result": header_credentials(headers),
            "request_result": list(request_credentials(url=url, headers=headers, proxy_url=proxy)),
        })
    for _ in range(300):
        backend = {}
        for key in ("token", "auth_header", "username", "rpcuser", "rpc_user", "password", "rpcpassword",
                    "rpc_password", "url", "tor_proxy", "proxy"):
            if rng.random() < 0.35:
                backend[key] = rng.choice([_text(rng), f"  {_text(rng)} ", _authorization(rng), _url(rng), 12345, None, True, 1.5, ""])
        cases["backend"].append({"backend": backend, "result": list(backend_credentials(backend))})
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(vectors(), indent=1, ensure_ascii=True, sort_keys=True) + "\n"
    if args.check:
        assert DESTINATION.read_text() == encoded, "Python credential scrub changed; review the Rust port"
    else:
        DESTINATION.write_text(encoded)


if __name__ == "__main__":
    main()
