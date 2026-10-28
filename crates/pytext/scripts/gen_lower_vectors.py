"""Pin CPython 3.11 lowercasing, including contextual sigma and future scalars."""
import argparse
from pathlib import Path
import sys
import unicodedata

TARGET = Path(__file__).resolve().parents[1] / "tests/data/lower.txt"


def render():
    if sys.version_info[:2] != (3, 11) or unicodedata.unidata_version != "14.0.0":
        raise SystemExit("run with locked CPython 3.11 / Unicode 14")
    texts = [chr(point) for point in range(0x110000)
             if chr(point).lower() != chr(point)]
    texts += ["ΟΣ", "ΟΣΑ", "ΟΣ\u0301", "ΟΣ\u0301Α", "Σ", "AΣ\U00010d50", "\u1c89\ua7cb\U00010d50", "Straße İ"]
    return "".join(
        " ".join(f"{ord(c):x}" for c in text) + ";" +
        " ".join(f"{ord(c):x}" for c in text.lower()) + "\n"
        for text in texts
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected = render()
    if args.check:
        if TARGET.read_text() != expected:
            raise SystemExit("lowercase vectors drifted")
    else:
        TARGET.write_text(expected)
