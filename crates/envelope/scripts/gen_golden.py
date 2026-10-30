"""Write tests/data/golden.txt: Python's envelopes and redactions.

One JSON object per line, from kassiber/envelope.py and kassiber/redaction.py
at this checkout:

- `envelope`: `build_envelope(kind, data, envelope_meta)` written as the CLI
  writes it (`json.dumps(indent=2)`) and as the daemon writes a record
  (`json.dumps(json_ready(...), separators=(",", ":"))`);
- `error`: `build_error_envelope(...)` written the same two ways;
- `secret_text` / `operational_text`: the redaction of a string;
- `details`: `redact_operational_value(redact_secret_value(details))`.

Amount pseudonyms carry a per-process salt, so `amount#<8 hex>` is written as
`amount#<salted>` on both sides before comparing. Inputs are JSON documents
written with `ensure_ascii=True`, so surrogates survive as escapes.

    .venv/bin/python crates/envelope/scripts/gen_golden.py
"""

from __future__ import annotations

import json
import random
import re
import sys
from decimal import Decimal
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.envelope import build_envelope, build_error_envelope, build_event_envelope, json_ready  # noqa: E402
from kassiber.redaction import (  # noqa: E402
    redact_operational_text,
    redact_operational_value,
    redact_secret_text,
    redact_secret_value,
)

TARGET = Path(__file__).resolve().parents[1] / "tests" / "data" / "golden.txt"
SEED = 21
SALTED = re.compile(r"amount#[0-9a-f]{8}")


def unsalt(text: str) -> str:
    return SALTED.sub("amount#<salted>", text)


def line(record: dict) -> str:
    return json.dumps(record, separators=(",", ":"), ensure_ascii=True)


def cli(value) -> str:
    return json.dumps(value, indent=2, sort_keys=False) + "\n"


def daemon(value) -> str:
    return json.dumps(json_ready(value), sort_keys=False, separators=(",", ":")) + "\n"


def tagged(value):
    """Decimals cannot cross JSON; tag them so the Rust side rebuilds them."""
    if isinstance(value, Decimal):
        return {"$decimal": str(value)}
    if isinstance(value, dict):
        return {k: tagged(v) for k, v in value.items()}
    if isinstance(value, list):
        return [tagged(v) for v in value]
    return value


ENVELOPES = [
    ("status", {"version": "0.22.78", "schema_version": 1}, None),
    ("transactions.list", {"items": [], "next_cursor": "c2", "has_more": 1}, None),
    ("transactions.list", {"items": [1], "next_cursor": None, "has_more": 0}, None),
    ("transactions.list", {"next_cursor": "x", "has_more": "", "page": {"custom": True}}, None),
    ("transactions.list", {"next_cursor": "x", "has_more": [0]}, None),
    ("transactions.list", {"next_cursor": "x", "has_more": 0.0}, None),
    ("transactions.list", {"next_cursor": "x"}, None),
    ("reports.summary", {"amount": Decimal("1.10"), "rows": [Decimal("-0"), Decimal("1E+2")]}, None),
    ("reports.summary", [{"btc": Decimal("0.00000001")}, Decimal("123456789.123456789")], None),
    ("ui.freshness.background", {"enqueued": [], "completed": []}, {"event": True}),
    ("commands.describe", {"name": "café \U0001f600 \x7f  "}, {"request_id": "r-1"}),
    ("daemon.ready", {"version": "x", "supported_kinds": ["status"]}, None),
    ("weird", None, None),
    ("weird", "text", {"extra": Decimal("2.5")}),
    ("floats", {"a": 0.1, "b": 1e16, "c": float("inf"), "d": -0.0, "e": 1e-7}, None),
]

ERRORS = [
    ("usage_error", "argument command: invalid choice: 'x'", {"usage": "usage: kassiber"}, "Hint", False, None),
    ("validation", "Invalid start", None, None, False, None),
    ("database_busy", "busy", {"n": Decimal("3.5")}, "Retry", True, "Traceback..."),
    ("internal_error", "café", ["a", {"b": None}], None, 1, None),
    ("x", "", {}, "", 0, ""),
]

SECRET_TEXTS = [
    "mnemonıc=abandon ability able",
    "blİndİng=topsecret",
    "ſecret=topsecret",
    "toKen=topsecret",
    "\u0345token=topsecret",
    "\U0001e4d0" + "xpub" + "A" * 40,
    "token=abc123 and password: hunter2",
    "Authorization: Bearer abc.def==",
    "bearer   tok~en/+",
    "seed phrase: abandon ability able, next",
    "seed_phrase: abandon ability able, next",
    "mnemonic=abandon ability able next=1",
    "Mnemonic: 'abandon ability' rest",
    "recovery-phrase = abc; x",
    "sk-abcdefgh",
    "sk-abc",
    "{\"api_key\": \"xyz\"}",
    "{'Api-Key': 'xyz', 'token':'t'}",
    "API_KEY=abc,TOKEN=def;secret=ghi}",
    "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi",
    "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8",
    "wpkh([d34db33f/84h/0h/0h]xpub6ERApfZwUNrhLCkDtcHTcxd75RbzS1ed54G1LkBUHQVHQKqhMkhgbmJbZRkrgZw4koxb5JaHWkY4ALHY2grBGRjaDMzQLcgJvLJuZZvRcEL/0/*)",
    "tr(abc)",
    "sp(spscan1qabcdefgh,spspend1qxyz)",
    "spscan1q023456789acdefgh",
    "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv",
    "tsp1q023456789acdefghjklmnpqrstuvwxyz02345",
    "descriptor: wsh(multi(2,xpubA,xpubB))",
    "passphrase=",
    "TOKEN : \"quoted-value\"",
    "my_token=abc",
    "tokenizer=abc",
    "café token=été ok",
    "اtoken=abc",
    "token\x1c=\x1cabc",
    "seed=abandon\nability",
    "seed words: abandon ability able\n",
    "",
]

OPERATIONAL_TEXTS = [
    "component 3cad1234-1234-1234-1234-123456789abc missing",
    "component 3cad1234123412341234123456789abc missing",
    "component 3CAD1234-1234-1234-1234-123456789ABC missing; paid 3 CAD",
    "amount_sat=12345678901234567890123456789012 fee_msat=12",
    "3cad1234-1234-1234-1234-123456789abc and 4cad1234123412341234123456789abc",
    "é3cad1234-1234-1234-1234-123456789abcé 3 CAD",
    "prefix_3cad1234-1234-1234-1234-123456789abc_suffix 3cad1",
    "3cad1234-1234-1234-1234-123456789ab 3cad1234123412341234123456789ab",
    "tx " + "a" * 64 + " done",
    "TX " + "AbCd" * 16 + "e",
    "two " + "f" * 128,
    "x" + "a" * 64,
    "paid 5 BTC",
    "paid 5BTC and 1,000.50 EUR",
    "BTC 0.5 and sats 1000",
    "€ 12,50 and 12 € and $5 and 3$",
    "BTC/EUR 64000.12",
    "BTC-EUR 64000.12 EUR",
    "rate EUR/BTC 1.5 BTC",
    "amount_sat=50000 fee_msat: 100000 value_sats='12'",
    "msat_total=5",
    "1 000 000 sats",
    "-5.5 LBTC",
    "x5 BTC",
    "5 BTCs",
    "٥ BTC",
    "",
]

DETAILS = [
    {"token": "abc", "nested": {"password": "x", "ok": "token=1"}},
    {"api-key": 1, "Auth_Header": None, "list": ["Bearer abc", {"seed": "x"}]},
    {"txid": "b" * 64, "amount": "5 BTC", "count": 3},
    {"deep": {"a": {"b": {"c": {"d": {"e": {"f": {"g": {"h": {"i": "token=x"}}}}}}}}}},
    ["sk-abcdefgh", ("tuple", "secret=1")],
    {"kind": "\ud800token=abc\udc00"},
    "plain string token=abc",
    {"1": "a", "SECRET_THING": [1, 2]},
]

FRAGMENTS = [
    "token", "api_key", "API-KEY", "secret", "password", "passphrase", "seed", "_", "-", "phrase",
    "words", "mnemonic", "recovery", " ", "  ", "=", ":", "'", "\"", ",", ";", "{", "}", "\n",
    "\x1c", "é", "　", "abc", "xyz123", "Bearer ", "bearer\t", "sk-", "abcdefgh", "/",
    "xpub661MyMwAqRbcFtXgS5sYJABqq", "xprv9s21ZrQH143K3QTDL4LXw2F7H", "wpkh(", "sh(", ")",
    "0123456789abcdef0123456789abcdef", "5", "1,000", ".50", "BTC", "sats", "msat", "EUR", "€",
    "$", "amount_sat", "fee_msat", "spscan1q", "023456789acdefgh", "sp1q", "٥", "x",
]


def fuzz(rng: random.Random, count: int) -> list[str]:
    return [
        "".join(rng.choice(FRAGMENTS) for _ in range(rng.randrange(1, 10)))
        for _ in range(count)
    ]


def unicode_secret_inputs():
    """Exercise every keyword with CPython case and boundary special points."""
    keys = [
        "apikey", "api_key", "api-key", "authheader", "auth_header", "auth-header",
        "blinding", "blindingkey", "blinding_key", "blinding-key", "cookie",
        "descriptor", "mnemonic", "passphrase", "password", "recoveryphrase",
        "recovery_phrase", "recovery-phrase", "scankey", "scan_key", "scan-key",
        "secret", "seed", "seedphrase", "seed_phrase", "seed-phrase", "seedwords",
        "seed_words", "seed-words", "silentpaymentscankey", "silent_payment_scan_key",
        "silent-payment-scan-key", "spdescriptor", "sp_descriptor", "sp-descriptor",
        "spscan", "spspend", "token",
    ]
    boundaries = ["", " ", "a", "_", "0", "é", "\u0345", "\u0301", "\u200c",
                  "\u0130", "\u0131", "\u017f", "\u212a", "\U0001e4d0", "\U00011f02"]
    rng = random.Random(2112)
    for key in keys:
        variants = {key, key.upper(), key.translate(str.maketrans("isk", "ıſK")),
                    key.translate(str.maketrans("i", "İ"))}
        for variant in sorted(variants):
            for boundary in boundaries:
                yield boundary + variant + "=abandon ability able"
                yield variant + boundary + "=topsecret"
                yield boundary + json.dumps({variant: "topsecret"}, ensure_ascii=False)
            for _ in range(12):
                before, after = (rng.choice(boundaries) for _ in range(2))
                yield before + variant + after + rng.choice(["=", " : '"]) + "topsecret"
    for token in ["xpub" + "A" * 40, "xprv" + "A" * 40, "sk-abcdefgh",
                  "sp1q" + "a" * 30, "spscan1q" + "a" * 20,
                  "wpkh(" + "a" * 20 + ")", "sp(" + "a" * 12 + ")", "Bearer abcdef"]:
        for before in boundaries:
            for after in boundaries:
                yield before + token + after


def main() -> int:
    if sys.version_info[:2] != (3, 11):
        raise SystemExit("run with the locked Python 3.11 interpreter")
    rng = random.Random(SEED)
    lines = []
    for kind, data, meta in ENVELOPES:
        envelope = build_envelope(kind, data, envelope_meta=meta)
        lines.append(line({
            "op": "envelope", "kind": kind, "data": tagged(data), "meta": tagged(meta),
            "cli": cli(envelope), "daemon": daemon(envelope),
        }))
    event = build_event_envelope("ui.freshness.worker", {"state": "idle"})
    lines.append(line({"op": "event", "kind": "ui.freshness.worker", "data": {"state": "idle"},
                       "daemon": daemon(event)}))
    for code, message, details, hint, retryable, debug in ERRORS:
        envelope = build_error_envelope(code, message, details=details, hint=hint,
                                        retryable=retryable, debug=debug)
        lines.append(line({
            "op": "error", "code": code, "message": message, "details": tagged(details),
            "hint": hint, "retryable": bool(retryable), "debug": debug,
            "cli": cli(envelope), "daemon": daemon(envelope),
        }))
    for text in SECRET_TEXTS + fuzz(rng, 2500) + list(unicode_secret_inputs()):
        lines.append(line({"op": "secret_text", "in": text, "out": redact_secret_text(text)}))
    for text in OPERATIONAL_TEXTS + fuzz(rng, 2500):
        lines.append(line({"op": "operational_text", "in": text,
                           "out": unsalt(redact_operational_text(text))}))
    for details in DETAILS:
        redacted = redact_operational_value(redact_secret_value(details))
        lines.append(line({"op": "details", "in": details,
                           "out": unsalt(json.dumps(redacted, separators=(",", ":")))}))
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text("\n".join(lines) + "\n", encoding="ascii")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
