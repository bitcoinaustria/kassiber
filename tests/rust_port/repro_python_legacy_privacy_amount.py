from contextlib import closing

from tests.integration.env import no_egress_guard
from tests.test_rust_legacy_privacy import REFERENCE
from kassiber.core.privacy_hygiene import build_privacy_hygiene_snapshot


def main():
    with no_egress_guard(enabled=True,allow_loopback=False):
        with closing(REFERENCE.connection("DELETE FROM transactions;" + REFERENCE.coin(amount="bad"))) as conn:
            before = REFERENCE.tables(conn)
            try:
                build_privacy_hygiene_snapshot(conn)
            except ValueError as exc:
                assert str(exc) == "invalid literal for int() with base 10: 'bad'"
                assert REFERENCE.tables(conn) == before
                print("Confirmed: an invalid stored active amount escapes the Python legacy scorer as ValueError; all 114 tables are unchanged.")
            else:
                raise AssertionError("The Python reference behavior changed")


if __name__ == "__main__":
    main()
