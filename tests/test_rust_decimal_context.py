"""Native decimal arithmetic runs in Python's pinned 32-digit context.

`kassiber/__init__.py` sets the default context to 32 digits, half-even, for
every thread, so Python arithmetic without an explicit context never rounds
at the stdlib default of 28. Ports that built `Context::new(28, ...)` stored
fewer digits than Python (a BTCPay payout rate in the regtest lane); use
`Context::RP2` instead. Mutation catalogs keep 28 as the mutant they must kill.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
STDLIB_DEFAULT = re.compile(r"Context::new\(\s*28\b")


def test_no_native_decimal_context_rounds_at_the_stdlib_default():
    offenders = []
    for path in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if STDLIB_DEFAULT.search(line):
                offenders.append(f"{path.relative_to(ROOT)}:{number}: {line.strip()}")
    assert offenders == [], "\n".join(offenders)


def test_python_pins_the_32_digit_context():
    import decimal

    import kassiber  # noqa: F401  (the import pins the context)

    assert decimal.DefaultContext.prec == 32
    assert decimal.DefaultContext.rounding == decimal.ROUND_HALF_EVEN
