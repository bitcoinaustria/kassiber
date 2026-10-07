"""Only the comparison machinery may import the frozen tax oracle (plan 20).

``tests/tax_oracle/`` exists to be compared against. A product module or an
ordinary test that imported it would run on the oracle instead of
production, and its assertions would stop saying anything about production.
``tests/tax_engine_compare.py`` runs the oracle for its importer
(``run_oracle``, ``run_on_oracle_backend``), so importing it is guarded the
same way.
"""

from __future__ import annotations

import ast
import os
import unittest
from pathlib import Path

_ROOT = Path(__file__).resolve().parents[1]
# The oracle package and the harness that re-exports it.
_GUARDED_MODULES = ("tests.tax_oracle", "tests.tax_engine_compare")
_SKIPPED_DIRECTORIES = frozenset({".git", ".venv", "__pycache__", "node_modules", "target", "dist", "build"})
# Calls whose first argument names a module to import.
_IMPORTING_CALLS = frozenset({"import_module", "__import__", "patch"})

ALLOWED = frozenset(
    {
        "tests/conftest.py",
        "tests/tax_engine_compare.py",
        # Installs the session's shadow or census mode in child processes.
        "tests/_child_hooks/sitecustomize.py",
        "tests/test_tax_shadow_harness.py",
        "tests/test_tax_ledger_goldens.py",
        "tests/test_tax_ledger_prepare.py",
        "tax-engine/scripts/gen_ledger_fixtures.py",
        # Tests that check the oracle's own internals, the interface mirror
        # of RP2, or the legacy-native leg.
        "tests/test_tax_descriptions.py",
        "tests/test_tax_engine_backend.py",
        "tests/test_tax_engine_parity.py",
    }
)


def _module_name(path: Path) -> str:
    parts = list(path.relative_to(_ROOT).with_suffix("").parts)
    if parts[-1] == "__init__":
        parts.pop()
    return ".".join(parts)


def _names_oracle(module: str) -> bool:
    return any(module == guarded or module.startswith(guarded + ".") for guarded in _GUARDED_MODULES)


def _resolve(node: ast.ImportFrom, path: Path) -> str:
    if not node.level:
        return node.module or ""
    package = _module_name(path).split(".")
    if path.name != "__init__.py":
        package.pop()
    base = package[: len(package) - (node.level - 1)] if node.level > 1 else package
    return ".".join([*base, *([node.module] if node.module else [])])


def oracle_imports(source: str, path: Path) -> list[int]:
    """Line numbers in ``source`` that import ``tests.tax_oracle`` or
    ``tests.tax_engine_compare``."""
    lines = []
    for node in ast.walk(ast.parse(source, filename=str(path))):
        if isinstance(node, ast.Import):
            if any(_names_oracle(alias.name) for alias in node.names):
                lines.append(node.lineno)
        elif isinstance(node, ast.ImportFrom):
            module = _resolve(node, path)
            if _names_oracle(module) or any(_names_oracle(f"{module}.{alias.name}") for alias in node.names):
                lines.append(node.lineno)
        elif isinstance(node, ast.Call) and node.args:
            function = node.func
            name = function.attr if isinstance(function, ast.Attribute) else getattr(function, "id", None)
            argument = node.args[0]
            if (
                name in _IMPORTING_CALLS
                and isinstance(argument, ast.Constant)
                and isinstance(argument.value, str)
                and _names_oracle(argument.value)
            ):
                lines.append(node.lineno)
    return sorted(set(lines))


def python_files(root: Path = _ROOT) -> list[Path]:
    files = []
    for directory, subdirectories, names in os.walk(root):
        subdirectories[:] = [name for name in subdirectories if name not in _SKIPPED_DIRECTORIES]
        files.extend(Path(directory) / name for name in names if name.endswith(".py"))
    return sorted(files)


class TaxOracleImportTest(unittest.TestCase):
    def test_only_the_comparison_machinery_imports_the_oracle(self):
        importers = {}
        for path in python_files():
            relative = path.relative_to(_ROOT).as_posix()
            if relative.startswith("tests/tax_oracle/"):
                continue
            lines = oracle_imports(path.read_text(encoding="utf-8"), path)
            if lines:
                importers[relative] = lines
        unexpected = {path: lines for path, lines in importers.items() if path not in ALLOWED}
        self.assertEqual(
            {}, unexpected, "use the production engine class instead of tests.tax_oracle or tests.tax_engine_compare"
        )
        # The guard sees the imports it is meant to see.
        self.assertIn("tests/tax_engine_compare.py", importers)
        self.assertIn("tests/test_tax_engine_parity.py", importers)
        # conftest.py imports only the harness.
        self.assertIn("tests/conftest.py", importers)

    def test_every_import_form_is_detected(self):
        path = _ROOT / "tests" / "test_example.py"
        cases = {
            "import tests.tax_oracle.adapter": [1],
            "from tests.tax_oracle import adapter": [1],
            "from tests import tax_oracle": [1],
            "from .tax_oracle.adapter import OracleTaxEngine": [1],
            "from . import tax_oracle": [1],
            "importlib.import_module('tests.tax_oracle.policy')": [1],
            "__import__('tests.tax_oracle')": [1],
            "patch('tests.tax_oracle.adapter._get_rp2_modules')": [1],
            "from tests import tax_engine_compare": [1],
            "from tests.tax_engine_compare import OracleTaxEngine": [1],
            "import tests.tax_engine_compare as compare": [1],
            "from .tax_engine_compare import run_oracle": [1],
            "importlib.import_module('tests.tax_engine_compare')": [1],
            "from tests import test_tax_engine_parity": [],
            "import tests.tax_engine_compared": [],
            "import tests.tax_oracles": [],
            "'tests.tax_oracle is documented here'": [],
        }
        for source, expected in cases.items():
            with self.subTest(source):
                self.assertEqual(oracle_imports(source, path), expected)


if __name__ == "__main__":
    unittest.main()
