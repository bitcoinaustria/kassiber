"""Tiny type-coercion helpers used across kassiber.

These are the value-normalizers call sites reach for when unwrapping
potentially-empty, potentially-stringy user / env / wallet input before
handing it to typed code. They intentionally raise `AppError` (not
`ValueError`) on bad input so the CLI surfaces a structured error envelope.

- `str_or_none` strips whitespace and collapses `""` to `None`, so
  callers can do `value if value is not None else default` without
  worrying about empty strings.
- `parse_bool` accepts the common yes/no/on/off/1/0/true/false variants
  users type into env files and config.
- `parse_int` coerces anything int-ish (str, float-ish, None, `""`) into
  an int, falling back to `default` on empties.

Prefer these over the raw builtins at I/O boundaries.
"""

from .errors import AppError


def str_or_none(value):
    """Strip and return a trimmed string, or `None` when empty/whitespace/None."""
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def parse_bool(value, default=False):
    """Parse user-facing boolean-ish text. `None` -> `default`; junk -> `AppError`."""
    if value is None:
        return default
    text = str(value).strip().lower()
    if text in {"1", "true", "yes", "on"}:
        return True
    if text in {"0", "false", "no", "off"}:
        return False
    raise AppError(f"Invalid boolean value: {value}")


def parse_int(value, default):
    """Parse user-facing integer-ish text. `None`/empty -> `default`; junk -> `AppError`."""
    if value is None or value == "":
        return default
    try:
        return int(str(value).strip())
    except ValueError as exc:
        raise AppError(f"Invalid integer value: {value}") from exc
