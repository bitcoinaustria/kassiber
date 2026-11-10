"""Write tests/data/vectors.txt: CPython's answers for kassiber-pytext.

One JSON object per line. Inputs are lists of code points so surrogates and
control characters survive any transport; outputs are what CPython 3.11
returns or raises:

- `repr`: `repr(s)`;
- `strip`: `s.strip()` as code points;
- `int` / `float`: `repr(int(s))` / `repr(float(s))`, or `null` on
  `ValueError`;
- `loads`: `json.dumps(json.loads(s), separators=(",", ":"))`, or the
  exception's class name and `str(exc)`.

    .venv/bin/python crates/pytext/scripts/gen_vectors.py
"""

from __future__ import annotations

import json
import random
import sys
from pathlib import Path

TARGET = Path(__file__).resolve().parents[1] / "tests" / "data" / "vectors.txt"
SEED = 21


def points(text: str) -> list[int]:
    return [ord(c) for c in text]


def case(op: str, text: str, out) -> str:
    return json.dumps({"op": op, "in": points(text), "out": out}, separators=(",", ":"))


def outcome(func, text):
    try:
        return {"ok": func(text)}
    except (ValueError, RecursionError) as exc:
        return {"error": type(exc).__name__, "message": str(exc)}


def repr_cases(rng: random.Random) -> list[str]:
    singles = list(range(0x0, 0x300)) + list(range(0xD7F0, 0xE010))
    singles += [rng.randrange(0x300, 0x110000) for _ in range(6000)]
    texts = [chr(p) for p in singles]
    quotes = ["'", '"', "\\", "a'b", 'a"b', "'\"", "it's", 'say "x"', ""]
    alphabet = "ab'\"\\\n\t\r\x00\x7f\xa0\xe9\u200b\u2028\ud800\udfff\U0001f600\U000e0001 "
    texts += quotes
    texts += ["".join(rng.choice(alphabet) for _ in range(rng.randrange(1, 12))) for _ in range(800)]
    return [case("repr", t, repr(t)) for t in texts]


def strip_cases(rng: random.Random) -> list[str]:
    spaces = [chr(p) for p in range(0x110000) if chr(p).isspace()]
    near = ["\x08", "\x0e", "\x1b", "\u180e", "\u200b", "\ufeff", "x", "\ud800", "\u2060"]
    alphabet = spaces + near
    texts = ["".join(rng.choice(alphabet) for _ in range(rng.randrange(0, 8))) for _ in range(1500)]
    return [case("strip", t, points(t.strip())) for t in texts]


NUMBER_ALPHABET = list("0123456789_+-.eE ") + [
    "\t", "\x0b", "\x1c", "\x85", "\xa0", "\u3000", "\u0663", "\uff15", "\U0001d7d9",
    "\xb2", "x", "inf", "nan", "Infinity", "0x", "\x00",
]
NUMBER_TEXTS = [
    "", " ", "0", "-0", "+0", "007", "1_0", "_1", "1_", "1__0", "0_0", "+_1", "- 1", "1e5", "1.5",
    "1.", ".5", ".", "e5", "1e", "1e+", "1E-5", " -1.5e3 ", "1_e5", "1e_5", "1e1_0", "in_f", "inf",
    "-Infinity", "+nan", "nAn", "infinit", "0x1p3", "1d5", "\x1c5", "5\x1c", "\x855", "\u30005",
    "\u0661.\u0665", "\u0661_\u0662", "\uff15", "\xb2", "9" * 4300, "9" * 4301, "1" + "0" * 400,
    "1e400", "-1e400", "1e-400", "4.9e-324", "2.4703282292062327e-324", "0.1", "1.7976931348623157e308",
]


def number_cases(rng: random.Random) -> list[str]:
    texts = list(NUMBER_TEXTS)
    texts += [
        "".join(rng.choice(NUMBER_ALPHABET) for _ in range(rng.randrange(1, 7)))
        for _ in range(3000)
    ]
    lines = []
    for text in texts:
        for op, func in (("int", int), ("float", float)):
            try:
                out = repr(func(text))
            except ValueError:
                out = None
            lines.append(case(op, text, out))
    return lines


LOADS_TEXTS = [
    "", " ", "null", "nul", "true", "false", "NaN", "Infinity", "-Infinity", "-Inf", "-", "-a",
    "0", "-0", "01", "1.", "1.5", "1e5", "1E+5", "1e", "1e+", "1.5e-3", "-0.0", "1" * 4301,
    "\"abc\"", "\"abc", "\"a\nb\"", "\"\\x\"", "\"\\u12\"", "\"\\u1234", "\"\\u1234\"",
    "\"\\ud83d\\ude00\"", "\"\\ud800\"", "\"\\ud800\\u0041\"", "\"\\ud800\\uzzzz\"",
    "\"\\/\\b\\f\\n\\r\\t\\\"\\\\\"", "{}", "[]", "{ }", "[ ]", "{\"a\": }", "[1,]", "{\"a\":1,}",
    "{\"a\" 1}", "{\"a\":1 \"b\":2}", "{1:2}", "[1 2]", "{} x", "[\n1 2]", "\ufeff{}", "  {}  ",
    "{\"a\":1,\"a\":2,\"b\":3}", "{\"kind\":\"status\",\"request_id\":\"r-1\"}", "[[[]]]",
    "\"\u00e9\U0001f600\"", "{\"a\":[1,{\"b\":null}]}", "\t[1]\r\n", "[1]\x0b", "\"\x7f\"",
    "[" * 990 + "]" * 990, "[" * 1001, "{\"a\":" * 1001,
]
LOADS_ALPHABET = list("{}[]:,\"\\ 0123456789-+.eEtrufalsenNI") + [
    "\n", "\t", "\x00", "\x1f", "u", "\u00e9", "null", "true", "\"k\"", "1.5", "\\u00e9", "\\ud800",
]


def loads_cases(rng: random.Random) -> list[str]:
    texts = list(LOADS_TEXTS)
    texts += [
        "".join(rng.choice(LOADS_ALPHABET) for _ in range(rng.randrange(1, 14)))
        for _ in range(6000)
    ]
    lines = []
    for text in texts:
        try:
            value = json.loads(text)
            out = {"ok": json.dumps(value, separators=(",", ":"))}
        except ValueError as exc:
            out = {"error": type(exc).__name__, "message": str(exc)}
        except RecursionError as exc:
            out = {"error": "RecursionError", "message": str(exc)}
        lines.append(case("loads", text, out))
    return lines


def main() -> int:
    if sys.version_info[:2] != (3, 11):
        raise SystemExit("run with the locked Python 3.11 interpreter")
    rng = random.Random(SEED)
    lines = repr_cases(rng) + strip_cases(rng) + number_cases(rng) + loads_cases(rng)
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text("\n".join(lines) + "\n", encoding="ascii")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
