"""Disposable repro: a list or object review operation `type` escapes validation.

`review_workflow._operations` tests `kind in {"unpair", "repair_legacy_pair",
"confirm_distinct_import_rows"}` (fix round 8f) and `kind in
_TERM_OPERATION_FIELDS` (main) before refusing unsupported types, so an
unhashable `type` raises a bare TypeError, and a review plan answers
`internal_error`. Rust refuses with the validation error the checked-in
vectors froze before both changes ("Review operation type is unsupported").
Python product code is unchanged.
"""
from kassiber.core import review_workflow


def main():
    for value in ([], {}):
        try:
            review_workflow._operations([{"type": value, "transaction_id": "t", "reason": "r"}])
        except TypeError as error:
            print(f"Python reference bug reproduced: {type(error).__name__}: {error}")
        else:
            raise SystemExit("expected an unhandled TypeError")


if __name__ == "__main__":
    main()
