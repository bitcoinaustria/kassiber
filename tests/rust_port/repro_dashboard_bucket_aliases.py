"""Check every accepted period spelling sizes dashboard buckets alike.

    uv run --locked python -m tests.rust_port.repro_dashboard_bucket_aliases

`_dashboard_bucket_size` compared the coerced period with the canonical names
only, so the aliases `_coerce_ui_transaction_period` accepts bucketed by
month: `4y`, `4year` and `4 Year` got monthly buckets while `4years` got
quarters, and `30d`/`3m` got months instead of days/weeks. It now sizes by the
canonical name. The bucket size also read the wall clock rather than `until`;
tests/test_review_regressions.py covers that half. Rust mirrors the old
behaviour (crates/transactions/src/desktop/dashboard.rs) until it follows.
Exit 0 means fixed.
"""
from kassiber.core import ui_snapshot as u


def main():
    expected = {
        "4years": "quarter", "4y": "quarter", "4year": "quarter", "4 Year": "quarter",
        "30days": "day", "30d": "day", "3months": "week", "3m": "week",
        "1year": "month", "1y": "month",
    }
    actual = {
        period: u._dashboard_bucket_size(u._coerce_ui_transaction_period(period), 1.0)
        for period in expected
    }
    print(actual)
    if actual != expected:
        raise SystemExit("aliases still bucket differently from their canonical names")


if __name__ == "__main__":
    main()
