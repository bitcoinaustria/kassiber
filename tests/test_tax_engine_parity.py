"""Production's ledger against the frozen oracle on RP2 (plan 20).

Production's ``build_ledger_state`` runs on finalized custody projections and
must equal the oracle's exactly (``tests/tax_engine_compare.py``). The
comparison itself is pinned first: it sees a decimal's representation, a
value's type, and key and list order, and forgives exactly the two listed
error differences (LD-3, LD-4).

The lot engine's own parity with RP2 is pinned in Rust by the schema-1
fixtures ``tax-engine/scripts/gen_engine_fixtures.py`` records from RP2
(``tax-engine/core/tests/engine_fixtures.rs``).
"""

from __future__ import annotations

import unittest
from decimal import Decimal

from kassiber.core.engines import errors
from kassiber.errors import AppError
from tests import tax_engine_compare as compare
from tests.tax_ledger_generator import finalized, ledger_history, rail_history, random_inputs
from tests.tax_ledger_generator import profile as ledger_profile


def test_comparison_sees_representation_type_and_order() -> None:
    # The parity checks below are only as strict as the comparison.
    assert compare.value_differences({"a": Decimal("300")}, {"a": Decimal("3.0E+2")})
    assert compare.value_differences({"a": Decimal("0")}, {"a": Decimal("-0")})
    assert compare.value_differences({"a": Decimal("1")}, {"a": "1"})
    assert compare.value_differences({"a": 1, "b": 2}, {"b": 2, "a": 1})
    assert compare.value_differences([1, 2], [2, 1])
    assert compare.value_differences([0.0], [-0.0])
    assert not compare.value_differences({"a": [Decimal("1.50"), float("nan")]}, {"a": [Decimal("1.50"), float("nan")]})


def test_error_signatures_apply_only_the_listed_differences() -> None:
    from rp2.rp2_error import RP2ValueError

    duplicate = "field 'exchanges' in section 'general' contains duplicate elements: W1"
    # LD-3: RP2 prefixes its temporary INI path; LD-4: its classes map to the engine's.
    assert compare.error_signature(RP2ValueError(f"/tmp/tmpk3x9.ini: {duplicate}")) == compare.error_signature(
        errors.EngineValueError(duplicate)
    )
    wrapped = [AppError("RP2 input validation failed", code="rp2_input_validation") for _ in range(2)]
    wrapped[0].__cause__ = RP2ValueError(f"/tmp/a b.ini: {duplicate}")
    wrapped[1].__cause__ = errors.EngineValueError(duplicate)
    assert compare.error_signature(wrapped[0]) == compare.error_signature(wrapped[1])
    # Nothing else is forgiven: other text before the message, another class,
    # or another AppError field still differs.
    assert compare.error_signature(errors.EngineValueError(f"x: {duplicate}")) != compare.error_signature(
        errors.EngineValueError(duplicate)
    )
    assert compare.error_signature(errors.EngineTypeError(duplicate)) != compare.error_signature(
        errors.EngineValueError(duplicate)
    )
    # LD-3 masks only a leading absolute INI path or the stored placeholder,
    # not ".ini: " after other text.
    for path in ("/tmp/dir with spaces/tmpk3x9.ini", "C:\\Users\\a b\\Temp\\tmpk3x9.ini"):
        error = RP2ValueError(f"{path}: {duplicate}")
        assert compare.error_signature(error) == compare.error_signature(errors.EngineValueError(duplicate))
        stored = compare.oracle_error_signature(error)
        assert stored == ("RP2ValueError", f"{compare.STORED_INI_PATH}: {duplicate}")
        assert compare.listed_signature(stored) == compare.error_signature(error)
    unsupported = "unsupported method broken.ini: tail"
    for message in (unsupported, f"relative/tmpk3x9.ini: {duplicate}", f"/tmp/x: y.ini: {duplicate}"):
        assert compare.error_signature(errors.EngineValueError(message)) == ("EngineValueError", message)
        assert compare.oracle_error_signature(RP2ValueError(message)) == ("RP2ValueError", message)
        assert compare.listed_signature(("RP2ValueError", message)) == ("EngineValueError", message)
    assert compare.error_signature(errors.EngineValueError("unsupported method a.ini: tail")) != (
        compare.error_signature(errors.EngineValueError("unsupported method b.ini: tail"))
    )
    assert compare.error_signature(AppError("m", code="validation")) != compare.error_signature(
        AppError("m", code="unsupported")
    )


class AdapterParityTest(unittest.TestCase):
    """Production's ``build_ledger_state`` gives the oracle's journal."""

    def _assert_parity(self, profile, inputs):
        expected = compare.run_oracle(profile, inputs)
        actual = compare.run_production(profile, inputs)
        self.assertEqual([], compare.ledger_differences(expected, actual))
        return actual

    def test_generic_ledger_matches_on_each_lot_method(self):
        rows, pairs = ledger_history()
        for method in ("FIFO", "LIFO", "HIFO", "LOFO"):
            with self.subTest(method=method):
                profile = ledger_profile("generic", method)
                result, error = self._assert_parity(profile, finalized(profile, rows, pairs))
                self.assertIsNone(error)
                entry_types = {entry["entry_type"] for entry in result.entries}
                # The history reaches every journal shape the engine feeds.
                self.assertTrue(
                    {"acquisition", "disposal", "income", "transfer_fee", "transfer_out", "transfer_in"} <= entry_types,
                    entry_types,
                )
                self.assertTrue(result.tax_summary)
                self.assertTrue(result.wallet_holdings)

    def test_generic_bitcoin_rail_carry_matches(self):
        rows, pairs = rail_history()
        for method in ("FIFO", "HIFO"):
            with self.subTest(method=method):
                profile = ledger_profile("generic", method)
                result, error = self._assert_parity(profile, finalized(profile, rows, pairs))
                self.assertIsNone(error)
                self.assertIn("disposal", {entry["entry_type"] for entry in result.entries})

    def test_random_generic_projections_match(self):
        # tests/test_tax_ledger_goldens.py runs 200 seeds of each country.
        for seed in range(16):
            book = random_inputs(seed, "generic")
            with self.subTest(seed=seed, method=book.profile["gains_algorithm"], tags=book.tags):
                self._assert_parity(book.profile, book.inputs)

    def test_austrian_ledger_matches_on_lot_methods(self):
        rows, pairs = ledger_history()
        for method in ("fifo", "hifo"):
            with self.subTest(method=method):
                profile = ledger_profile("at", method)
                self._assert_parity(profile, finalized(profile, rows, pairs))

    def test_austrian_ledger_matches_on_moving_average_at(self):
        for rows, pairs in (ledger_history(), rail_history()):
            with self.subTest(first=rows[0]["id"]):
                profile = ledger_profile("at", "moving_average_at")
                self._assert_parity(profile, finalized(profile, rows, pairs))

    def test_random_austrian_projections_match(self):
        for seed in range(8):
            book = random_inputs(seed, "at")
            with self.subTest(seed=seed, method=book.profile["gains_algorithm"], tags=book.tags):
                self._assert_parity(book.profile, book.inputs)
