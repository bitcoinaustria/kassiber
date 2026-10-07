"""The tax mutation entries stay applicable (plan 20, phase 4).

``tax-engine/scripts/mutation_check.py`` breaks one rule at a time and needs
the tests ``tax-engine/scripts/mutations.json`` names to fail. It is too slow
for CI, so this module keeps its entries honest in between: each one still
applies to exactly one place in the code, and every test it names exists.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

_ROOT = Path(__file__).resolve().parents[1]
_SCRIPT = _ROOT / "tax-engine" / "scripts" / "mutation_check.py"


def _load_checker():
    spec = importlib.util.spec_from_file_location("tax_mutation_check", _SCRIPT)
    module = importlib.util.module_from_spec(spec)
    # Its dataclasses look their module up while the class is built.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


checker = _load_checker()


class MutationEntriesTest(unittest.TestCase):
    def test_every_entry_applies_once_and_names_existing_tests(self):
        mutations = checker.load()
        self.assertTrue(mutations)
        self.assertEqual(checker.problems(mutations), [])

    def test_stale_entries_are_reported(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-mutations-") as tmp:
            root = Path(tmp)
            (root / "tests").mkdir()
            (root / "rule.py").write_text("limit = 1\nlimit = 1\nfloor = 0\n", encoding="utf-8")
            (root / "tests" / "test_rule.py").write_text(
                "class RuleTest:\n    def test_limit(self):\n        pass\n\n\ndef test_floor(case):\n    pass\n",
                encoding="utf-8",
            )

            def entry(**fields):
                return checker.Mutation(
                    **{
                        "id": "m",
                        "file": "rule.py",
                        "find": "floor = 0",
                        "replace": "floor = 1",
                        "expect_failing": ("tests/test_rule.py::RuleTest::test_limit",),
                        "rule": "r",
                        **fields,
                    }
                )

            parametrized = entry(id="n", expect_failing=("tests/test_rule.py::test_floor[x]",))
            self.assertEqual(checker.problems([entry(), parametrized], root), [])
            cases = {
                "twice": (entry(find="limit = 1", replace="limit = 2"), "find occurs 2 times"),
                "gone": (entry(find="ceiling = 9"), "find occurs 0 times"),
                "same": (entry(replace="floor = 0"), "replace equals find"),
                "no file": (entry(file="missing.py"), "no file missing.py"),
                "no test": (entry(expect_failing=("tests/test_rule.py::RuleTest::test_gone",)), "names no test"),
                "no class": (entry(expect_failing=("tests/test_rule.py::Gone::test_limit",)), "names no test"),
                "not a node id": (entry(expect_failing=("tests/test_rule.py",)), "is not a pytest node id"),
            }
            for name, (mutation, message) in cases.items():
                with self.subTest(name):
                    found = checker.problems([mutation], root)
                    self.assertEqual(len(found), 1, found)
                    self.assertIn(message, found[0])
            self.assertEqual(checker.problems([entry(), entry()], root), ["m: duplicate id"])
            # A Rust entry names cargo filters, which are not checked here.
            (root / "engine.rs").write_text("let x = 1;\n", encoding="utf-8")
            rust = entry(file="engine.rs", find="x = 1", replace="x = 2", expect_failing=("ledger",))
            self.assertEqual(checker.problems([rust], root), [])

    def test_entries_need_exactly_the_documented_fields(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-mutations-") as tmp:
            path = Path(tmp) / "mutations.json"
            good = {
                "id": "m",
                "file": "rule.py",
                "find": "a",
                "replace": "",
                "expect_failing": "ledger",
                "rule": "r",
            }
            path.write_text(json.dumps([good]), encoding="utf-8")
            [mutation] = checker.load(path)
            self.assertEqual(mutation.expect_failing, ("ledger",))
            for broken in (
                {key: value for key, value in good.items() if key != "rule"},
                {**good, "extra": 1},
                {**good, "expect_failing": []},
                {**good, "find": ""},
            ):
                with self.subTest(broken):
                    path.write_text(json.dumps([broken]), encoding="utf-8")
                    with self.assertRaises(checker.EntryError):
                        checker.load(path)


if __name__ == "__main__":
    unittest.main()
