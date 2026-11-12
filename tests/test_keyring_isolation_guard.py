"""Tests that hide the keyring behind an unreachable D-Bus address run on Linux only.

Pointing `DBUS_SESSION_BUS_ADDRESS` at a missing socket makes Linux's Secret
Service unavailable to both runtimes. It does nothing on macOS or Windows,
where the same commands would write and delete real login-keychain or
Credential Manager items. A module that relies on that address must carry a
module-level `skipif(sys.platform != "linux")`. The source check here fails on
a new module that doesn't.
"""

from __future__ import annotations

import ast
import re
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCANNED = ("tests", "crates", "dev", "scripts")
_UNREACHABLE_BUS = re.compile(r"""DBUS_SESSION_BUS_ADDRESS["']?\s*[:=,]\s*f?["']unix:path=""")


def _is_linux_only_skip(node: ast.AST) -> bool:
    """`pytest.mark.skipif(sys.platform != "linux", ...)` in any spelling of the call."""
    if not isinstance(node, ast.Call) or not node.args:
        return False
    func = node.func
    if not (isinstance(func, ast.Attribute) and func.attr == "skipif"):
        return False
    test = node.args[0]
    return (
        isinstance(test, ast.Compare)
        and len(test.ops) == 1
        and isinstance(test.ops[0], ast.NotEq)
        and ast.unparse(test.left) == "sys.platform"
        and isinstance(test.comparators[0], ast.Constant)
        and test.comparators[0].value == "linux"
    )


def _module_is_linux_only(tree: ast.Module) -> bool:
    for node in tree.body:
        if not isinstance(node, ast.Assign):
            continue
        if not any(isinstance(target, ast.Name) and target.id == "pytestmark" for target in node.targets):
            continue
        marks = node.value.elts if isinstance(node.value, (ast.List, ast.Tuple)) else [node.value]
        if any(_is_linux_only_skip(mark) for mark in marks):
            return True
    return False


def unguarded_modules(root: Path = ROOT) -> list[str]:
    """Modules that set an unreachable bus without a module-level Linux-only skip."""
    found = []
    for top in SCANNED:
        for path in sorted((root / top).rglob("*.py")):
            if "__pycache__" in path.parts or "target" in path.relative_to(root).parts:
                continue
            if path.resolve() == Path(__file__).resolve():
                continue  # this file's own fixtures
            source = path.read_text(encoding="utf-8")
            if not _UNREACHABLE_BUS.search(source):
                continue
            relative = path.relative_to(root).as_posix()
            if not _module_is_linux_only(ast.parse(source, filename=relative)):
                found.append(relative)
    return found


class KeyringIsolationGuardTest(unittest.TestCase):
    def test_unreachable_bus_modules_are_linux_only(self):
        self.assertEqual(
            unguarded_modules(),
            [],
            'add `pytest.mark.skipif(sys.platform != "linux", ...)` to the module\'s pytestmark',
        )

    def test_the_secrets_lifecycle_module_is_covered(self):
        source = (ROOT / "tests/test_rust_secrets_lifecycle.py").read_text(encoding="utf-8")
        self.assertRegex(source, _UNREACHABLE_BUS)
        self.assertTrue(_module_is_linux_only(ast.parse(source)))

    def test_the_check_finds_each_spelling_and_accepts_the_guard(self):
        cases = {
            "test_kwargs.py": 'env.update(DBUS_SESSION_BUS_ADDRESS=f"unix:path={tmp}/absent")\n',
            "test_dict.py": 'env = {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/missing"}\n',
            "test_other_skip.py": (
                "import os, pytest\n"
                'pytestmark = pytest.mark.skipif(os.name == "nt", reason="posix")\n'
                'env = {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/missing"}\n'
            ),
            "test_function_skip.py": (
                "import sys, pytest\n"
                '@pytest.mark.skipif(sys.platform != "linux", reason="linux")\n'
                "def test_x():\n"
                '    env = {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/missing"}\n'
            ),
            "test_guarded.py": (
                "import sys, pytest\n"
                'pytestmark = [pytest.mark.skipif(not BIN, reason="x"), pytest.mark.skipif(sys.platform != "linux", reason="linux")]\n'
                'env = {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/missing"}\n'
            ),
            "test_reads_only.py": 'if not os.environ.get("DBUS_SESSION_BUS_ADDRESS"):\n    pass\n',
        }
        with tempfile.TemporaryDirectory(prefix="kassiber-keyring-guard-") as scratch:
            root = Path(scratch)
            (root / "tests").mkdir()
            for name, body in cases.items():
                (root / "tests" / name).write_text(textwrap.dedent(body), encoding="utf-8")
            self.assertEqual(
                unguarded_modules(root),
                [
                    "tests/test_dict.py",
                    "tests/test_function_skip.py",
                    "tests/test_kwargs.py",
                    "tests/test_other_skip.py",
                ],
            )


if __name__ == "__main__":
    unittest.main()
