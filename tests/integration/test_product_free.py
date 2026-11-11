"""The harness stays product-free (docs/plan/22-python-removal.md, decision D2).

Every module under `tests/integration/` and `dev/regtest/` must import under
`python3 -I` with the standard library alone, and must not load the Python
product, a third-party package, or a test module outside those two trees.
"""
from __future__ import annotations

import json
import subprocess
import sys
import sysconfig
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HARNESS_DIRS = (ROOT / "tests" / "integration", ROOT / "dev" / "regtest")

PROBE = r"""
import importlib, json, sys, sysconfig
root, name = sys.argv[1], sys.argv[2]
sys.path.insert(0, root)
before = set(sys.modules)
importlib.import_module(name)
stdlib = sysconfig.get_paths()["stdlib"]
outside = []
for module_name, module in sorted(sys.modules.items()):
    if module_name in before or module is None:
        continue
    top = module_name.split(".", 1)[0]
    if top in ("tests", "dev"):
        if not (module_name in ("tests", "dev", "tests.integration", "dev.regtest")
                or module_name.startswith(("tests.integration.", "dev.regtest."))):
            outside.append(module_name)
        continue
    if top in sys.stdlib_module_names or top.startswith("_"):
        continue
    origin = getattr(module, "__file__", None) or ""
    if origin and not origin.startswith(stdlib):
        outside.append(module_name)
print(json.dumps(outside))
"""


def harness_modules() -> list[str]:
    names = []
    for directory in HARNESS_DIRS:
        for path in sorted(directory.glob("*.py")):
            if path.name == "__init__.py" or "-" in path.stem:
                continue
            names.append(".".join(path.relative_to(ROOT).with_suffix("").parts))
    return names


class HarnessProductFreeTest(unittest.TestCase):
    maxDiff = None

    def test_every_module_imports_with_the_standard_library_alone(self):
        modules = harness_modules()
        self.assertIn("tests.integration.regtest_demo", modules)
        for name in modules:
            with self.subTest(module=name):
                result = subprocess.run(
                    [sys.executable, "-I", "-c", PROBE, str(ROOT), name],
                    capture_output=True, text=True, timeout=120, cwd=ROOT,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout.splitlines()[-1]), [], name)

    def test_no_module_names_the_python_product(self):
        offenders = []
        for directory in HARNESS_DIRS:
            for path in sorted(directory.glob("*.py")):
                if path == Path(__file__).resolve():
                    continue
                for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                    stripped = line.strip()
                    # Lazy imports inside functions escape the import probe.
                    product = stripped.startswith(("import kassiber", "from kassiber"))
                    other_tests = stripped.startswith(("from tests", "import tests")) and not stripped.startswith(
                        ("from tests.integration", "import tests.integration", "from tests import integration")
                    )
                    uv = "uv run --locked" in stripped or '"uv", "run"' in stripped
                    if product or other_tests or uv:
                        offenders.append(f"{path.relative_to(ROOT)}:{number}: {stripped}")
        self.assertEqual(offenders, [])

    def test_harness_scripts_need_no_python_environment(self):
        scripts = (
            "scripts/integration-harness.sh",
            "scripts/chain-analysis-regtest.sh",
            "scripts/benchmark-wallet-sync.py",
            "scripts/bootstrap-dev-env.sh",
            "scripts/smoke-packaged-cli.sh",
            "scripts/check-packaged-cli-no-egress.sh",
        )
        offenders = []
        for name in scripts:
            for number, line in enumerate((ROOT / name).read_text(encoding="utf-8").splitlines(), 1):
                stripped = line.strip()
                if any(token in stripped for token in ("uv run", "uv sync", ".venv/", "-m kassiber")) or \
                        stripped.startswith(("import kassiber", "from kassiber")):
                    offenders.append(f"{name}:{number}: {stripped}")
        self.assertEqual(offenders, [])

    def test_runs_on_the_interpreter_the_harness_requires(self):
        self.assertGreaterEqual(sys.version_info[:2], (3, 11))
        self.assertTrue(sysconfig.get_paths()["stdlib"])


if __name__ == "__main__":
    unittest.main()
