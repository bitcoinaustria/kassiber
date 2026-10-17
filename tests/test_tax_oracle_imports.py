"""Only the comparison machinery may import the frozen tax oracle (plan 20).

``tests/tax_oracle/`` exists to be compared against. A product module or an
ordinary test that imported it would run on the oracle instead of
production, and its assertions would stop saying anything about production.
``tests/tax_engine_compare.py`` runs the oracle for its importer
(``run_oracle``, ``run_on_oracle_backend``), so importing it is guarded the
same way.

The product side is stricter (design §5.1): no module under ``kassiber/``
imports the test tree, RP2, or the packages only RP2 brings in, by an
``import`` statement or by a module name passed to ``import_module`` or
``__import__``. Production runs without RP2 installed; RP2 is a development
dependency of the oracle. A dynamic import whose module name is not a
literal cannot be checked here, so each one must be reviewed and listed.
``tests/test_tax_engine_backend.py`` runs production with RP2 blocked, as the
behavioral proof.
"""

from __future__ import annotations

import ast
import importlib.util
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
        # The builder and reader against the goldens and the oracle's own
        # errors and tables, and the shadow capabilities of the golden books.
        "tests/test_tax_ledger_contract.py",
        "tests/test_tax_shadow_capabilities.py",
        "tax-engine/scripts/gen_ledger_fixtures.py",
        # Tests that check the interface mirror of RP2 or the legacy-native
        # leg.
        "tests/test_tax_engine_backend.py",
        "tests/test_tax_engine_parity.py",
    }
)

# The top-level packages a product module must not import.
_PRODUCT_FORBIDDEN = frozenset({"tests", "rp2", "prezzemolo", "pycountry"})
# Calls whose first argument names a module to import.
_PRODUCT_IMPORTING_CALLS = frozenset({"import_module", "__import__"})
# What ``product_imports`` reports for a call whose module name is not a
# literal.
DYNAMIC = "<module named by a value>"
# Product modules whose dynamic imports were reviewed: the names they import.
REVIEWED_DYNAMIC_IMPORTS = {
    "kassiber/wallet_security.py": "embit's descriptor modules, from a literal tuple",
}


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


def _literal(node: ast.AST | None) -> str | None:
    return node.value if isinstance(node, ast.Constant) and isinstance(node.value, str) else None


def _imported_by_call(node: ast.Call) -> str | None:
    """The module an ``import_module``/``__import__`` call names, or ``DYNAMIC``."""
    keywords = {keyword.arg: keyword.value for keyword in node.keywords}
    argument = node.args[0] if node.args else keywords.get("name")
    name = _literal(argument)
    if name is None:
        return DYNAMIC
    if not name.startswith("."):
        return name
    package = _literal(node.args[1] if len(node.args) > 1 else keywords.get("package"))
    if package is None:
        return DYNAMIC
    try:
        return importlib.util.resolve_name(name, package)
    except ImportError:
        return DYNAMIC


def product_imports(source: str, path: Path) -> list[tuple[int, str]]:
    """``(line, module)`` for each import in ``source`` of a forbidden package,
    and ``(line, DYNAMIC)`` for each dynamic import of a name that is not a
    literal."""
    found = []
    for node in ast.walk(ast.parse(source, filename=str(path))):
        if isinstance(node, ast.Import):
            found.extend(
                (node.lineno, alias.name) for alias in node.names if alias.name.split(".")[0] in _PRODUCT_FORBIDDEN
            )
        elif isinstance(node, ast.ImportFrom):
            module = _resolve(node, path)
            if module.split(".")[0] in _PRODUCT_FORBIDDEN:
                found.append((node.lineno, module))
        elif isinstance(node, ast.Call):
            function = node.func
            name = function.attr if isinstance(function, ast.Attribute) else getattr(function, "id", None)
            if name in _PRODUCT_IMPORTING_CALLS:
                module = _imported_by_call(node)
                if module == DYNAMIC or module.split(".")[0] in _PRODUCT_FORBIDDEN:
                    found.append((node.lineno, module))
    return sorted(set(found))


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


class ProductImportTest(unittest.TestCase):
    def test_the_product_imports_neither_the_tests_nor_rp2(self):
        found = {}
        scanned = []
        for path in python_files(_ROOT / "kassiber"):
            relative = path.relative_to(_ROOT).as_posix()
            scanned.append(relative)
            imports = product_imports(path.read_text(encoding="utf-8"), path)
            if relative in REVIEWED_DYNAMIC_IMPORTS:
                # A stale entry would leave the file's next dynamic import unreviewed.
                self.assertIn(DYNAMIC, [module for _line, module in imports], relative)
                imports = [(line, module) for line, module in imports if module != DYNAMIC]
            if imports:
                found[relative] = imports
        self.assertEqual(
            {},
            found,
            "product modules must not import tests, rp2, prezzemolo, or pycountry; list a reviewed dynamic import",
        )
        # The scan sees the product, the engine package included.
        self.assertIn("kassiber/core/engines/ledger.py", scanned)
        self.assertIn("kassiber/tax_policy.py", scanned)

    def test_every_product_import_form_is_detected(self):
        path = _ROOT / "kassiber" / "core" / "example.py"
        cases = {
            "import rp2": [(1, "rp2")],
            "import rp2.plugin.country.at as at": [(1, "rp2.plugin.country.at")],
            "from rp2.rp2_decimal import RP2Decimal": [(1, "rp2.rp2_decimal")],
            "from prezzemolo.avl_tree import AVLTree": [(1, "prezzemolo.avl_tree")],
            "import pycountry": [(1, "pycountry")],
            "import tests.tax_engine_compare": [(1, "tests.tax_engine_compare")],
            "from tests import tax_oracle": [(1, "tests")],
            "from tests.tax_oracle.adapter import OracleTaxEngine": [(1, "tests.tax_oracle.adapter")],
            "importlib.import_module('rp2.plugin.country.at')": [(1, "rp2.plugin.country.at")],
            "import_module(name='pycountry')": [(1, "pycountry")],
            "importlib.import_module('.at', 'rp2.plugin.country')": [(1, "rp2.plugin.country.at")],
            "importlib.import_module('.at', package='rp2.plugin.country')": [(1, "rp2.plugin.country.at")],
            "__import__('prezzemolo')": [(1, "prezzemolo")],
            "importlib.import_module(name)": [(1, DYNAMIC)],
            "importlib.import_module('.' + name, 'kassiber')": [(1, DYNAMIC)],
            "importlib.import_module('.at', package)": [(1, DYNAMIC)],
            "from . import rp2": [],
            "from .engines import ledger": [],
            "import kassiber.core.engines": [],
            "importlib.import_module('embit.descriptor')": [],
            "importlib.import_module('.descriptor', 'embit')": [],
            "import rp2x": [],
            "import testsuite": [],
            "'rp2 is named here'": [],
        }
        for source, expected in cases.items():
            with self.subTest(source):
                self.assertEqual(product_imports(source, path), expected)


if __name__ == "__main__":
    unittest.main()
