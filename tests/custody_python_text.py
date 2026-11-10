"""Generate the custody identity table with the pinned CPython oracle."""
from pathlib import Path
import sys
import unicodedata

PATH = Path(__file__).resolve().parents[1] / "crates/custody/src/python_text.rs"


def render():
    assert unicodedata.unidata_version == "14.0.0", "regenerate with CPython 3.11"
    def table(name, predicate):
        ranges = []
        start = None
        for n in range(0x110001):
            yes = n < 0x110000 and predicate(chr(n))
            if yes and start is None:
                start = n
            if not yes and start is not None:
                ranges.append((start, n - 1))
                start = None
        return f"const {name}: &[(u32, u32)] = &[\n" + "".join(f"    (0x{a:x}, 0x{b:x}),\n" for a, b in ranges) + "];\n"
    # rustfmt only expands the strip predicate; its checked-in spelling stays
    # the template while the generated table is replaced below.
    header = PATH.read_text().split("const ALNUM:", 1)[0]
    # Probe CPython's Final_Sigma context rather than importing a different
    # Unicode database. Cased+ignorable scalars belong to the skipped class;
    # CASED only needs the non-ignorable cased scalars used by the algorithm.
    cased = lambda c: (c + "Σ").lower().endswith("ς")
    ignored = lambda c: not cased(c) and ("A" + c + "Σ").lower().endswith("ς")
    return header + table("ALNUM", str.isalnum) + table("CASED", cased) + table("CASE_IGNORABLE", ignored) + table("DECIMAL", str.isdecimal) + table("DIGIT", str.isdigit)


if __name__ == "__main__":
    text = render()
    if "--write" in sys.argv:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
