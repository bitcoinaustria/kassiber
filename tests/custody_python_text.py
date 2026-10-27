"""Generate the custody identity table with the pinned CPython oracle."""
from pathlib import Path
import sys
import unicodedata

PATH = Path(__file__).resolve().parents[1] / "crates/custody/src/python_text.rs"


def render():
    assert unicodedata.unidata_version == "14.0.0", "regenerate with CPython 3.11"
    ranges = []
    start = None
    for n in range(0x110001):
        yes = n < 0x110000 and chr(n).isalnum()
        if yes and start is None:
            start = n
        if not yes and start is not None:
            ranges.append((start, n - 1))
            start = None
    # rustfmt only expands the strip predicate; its checked-in spelling stays
    # the template while the generated table is replaced below.
    header = PATH.read_text().split("const ALNUM:", 1)[0]
    return header + "const ALNUM: &[(u32, u32)] = &[\n" + "".join(f"    (0x{a:x}, 0x{b:x}),\n" for a, b in ranges) + "];\n"


if __name__ == "__main__":
    text = render()
    if "--write" in sys.argv:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
