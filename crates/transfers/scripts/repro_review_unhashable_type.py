#!/usr/bin/env python3
"""Reproduce a review operation whose `type` is a list escaping as TypeError.

Before #680 `_operations` compared the type with `==` and refused anything
unknown with "Review operation type is unsupported". It now tests membership
in `_TERM_OPERATION_FIELDS`, a dict, so a list or object type raises a bare
`TypeError: unhashable type`, which the daemon answers as `internal_error`.
No book is needed. The Rust port reproduces the reference answer.
"""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import review_workflow
from kassiber.errors import AppError

for kind in ([], {}):
    try:
        review_workflow._operations([{"type": kind, "transaction_id": "tx", "reason": "r"}])
    except AppError as error:
        raise SystemExit(f"Reference behavior changed; review repro: {error}")
    except TypeError as error:
        print(f"type={kind!r}: TypeError: {error}")
        assert str(error) == f"unhashable type: '{type(kind).__name__}'"
    else:
        raise SystemExit("Reference behavior changed; review repro: accepted")
