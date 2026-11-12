import sys

# `python -m tests.<module>` imports this package while `sys.argv[0]` is
# still "-m": such a reference child must run under the no-egress guard when
# the test session armed it (tests/egress_guard.py). The guard hook itself
# imports this package while it arms; that import is its own.
if sys.argv[:1] == ["-m"] and not getattr(sys.modules.get("sitecustomize"), "_ARMING", False):
    from tests.egress_guard import assert_guard_active

    assert_guard_active()
