"""Compare a stored fixture with its Python rebuild without a quadratic diff.

pytest explains a failed `==` on two large documents with difflib, which is
quadratic in their size: a stale multi-megabyte fixture then runs for many
minutes instead of failing. These checks compare exactly like `==` and, on a
difference, report the first differing character of the two JSON texts.
"""
import json


def _text(value):
    if isinstance(value, str):
        return value
    try:
        return json.dumps(value, sort_keys=True, ensure_ascii=True, default=repr)
    except TypeError:
        return repr(value)


def assert_rebuilt(stored, rebuilt, label):
    """Fail unless `stored == rebuilt`, naming where their texts first differ."""
    if stored == rebuilt:
        return
    left, right = _text(stored), _text(rebuilt)
    at = next(
        (index for index, (a, b) in enumerate(zip(left, right)) if a != b),
        min(len(left), len(right)),
    )
    window = slice(max(0, at - 80), at + 80)
    raise AssertionError(
        f"{label} differs from its Python rebuild at character {at} "
        f"(stored {len(left)}, rebuilt {len(right)} characters):\n"
        f"  stored:  {left[window]!r}\n  rebuilt: {right[window]!r}"
    )
