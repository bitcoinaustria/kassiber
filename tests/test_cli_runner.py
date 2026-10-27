"""Tests start the product only through `tests/cli_runner.py` (plan 21).

`KASSIBER_TEST_CLI` can point every CLI and daemon subprocess the suite, the
integration lanes, and the regtest seeds start at another executable only if
none of them builds the `python -m kassiber` command itself. The source check
here fails on a new construction site; the rest pins how the switch reads.
"""

from __future__ import annotations

import ast
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

from tests import cli_runner


ROOT = Path(__file__).resolve().parents[1]
SCANNED = ("tests", "dev", "scripts")
# Files that may build the Python command themselves, and why.
ALLOWED = {
    "tests/cli_runner.py": "the shared runner",
    "scripts/benchmark-wallet-sync.py": "profiles the Python reference under cProfile",
    "scripts/generate-source-funds-demo-report.py": "renders documentation assets, not a test",
}
_SUBPROCESS_CALLS = {"run", "Popen", "call", "check_call", "check_output", "execv", "execvp", "execve", "spawnv"}


def _constant(node) -> str | None:
    return node.value if isinstance(node, ast.Constant) and isinstance(node.value, str) else None


def construction_sites(root: Path = ROOT) -> list[str]:
    """Places outside the runner that start the product by its module or name."""
    sites = []
    for top in SCANNED:
        for path in sorted((root / top).rglob("*.py")):
            relative = path.relative_to(root).as_posix()
            if "__pycache__" in path.parts or relative in ALLOWED:
                continue
            tree = ast.parse(path.read_text(encoding="utf-8"), filename=relative)
            for node in ast.walk(tree):
                if isinstance(node, (ast.List, ast.Tuple)):
                    values = [_constant(element) for element in node.elts]
                    for first, second in zip(values, values[1:]):
                        if first == "-m" and second and second.split(".")[0] == "kassiber":
                            sites.append(f"{relative}:{node.lineno}: python -m {second}")
                elif isinstance(node, ast.Call):
                    name = node.func.attr if isinstance(node.func, ast.Attribute) else getattr(node.func, "id", None)
                    if name not in _SUBPROCESS_CALLS or not node.args:
                        continue
                    command = node.args[0]
                    if isinstance(command, (ast.List, ast.Tuple)) and command.elts and _constant(command.elts[0]) == "kassiber":
                        sites.append(f"{relative}:{node.lineno}: kassiber from PATH")
    return sorted(sites)


class ConstructionSiteTest(unittest.TestCase):
    def test_no_test_builds_the_product_command_itself(self):
        self.assertEqual(
            construction_sites(),
            [],
            "start the CLI or daemon with tests.cli_runner.run/popen so KASSIBER_TEST_CLI reaches it",
        )

    def test_the_check_finds_each_spelling(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-cli-runner-sites-") as scratch:
            root = Path(scratch)
            (root / "tests").mkdir()
            (root / "tests" / "test_probe.py").write_text(
                textwrap.dedent(
                    """
                    import subprocess, sys
                    subprocess.run([sys.executable, "-m", "kassiber", "status"])
                    cmd = [sys.executable, "-X", "dev", "-m", "kassiber.cli.main"]
                    subprocess.Popen(["kassiber", "daemon"])
                    """
                ),
                encoding="utf-8",
            )
            self.assertEqual(
                construction_sites(root),
                [
                    "tests/test_probe.py:3: python -m kassiber",
                    "tests/test_probe.py:4: python -m kassiber.cli.main",
                    "tests/test_probe.py:5: kassiber from PATH",
                ],
            )

    def test_allowed_files_exist(self):
        for relative in ALLOWED:
            self.assertTrue((ROOT / relative).is_file(), relative)


class CandidateSwitchTest(unittest.TestCase):
    def test_default_is_this_checkouts_python(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop(cli_runner.CLI_ENV, None)
            self.assertEqual(cli_runner.prefix(), cli_runner.REFERENCE_PREFIX)
            self.assertEqual(cli_runner.argv(["--machine", Path("/x")]), [*cli_runner.REFERENCE_PREFIX, "--machine", "/x"])

    def test_a_path_or_a_json_argv_selects_the_candidate(self):
        with mock.patch.dict(os.environ, {cli_runner.CLI_ENV: sys.executable}):
            self.assertEqual(cli_runner.prefix(), (sys.executable,))
        with mock.patch.dict(os.environ, {cli_runner.CLI_ENV: json.dumps([sys.executable, "-I", "x.py"])}):
            self.assertEqual(cli_runner.prefix(), (sys.executable, "-I", "x.py"))
            # The reference ignores the switch, and only it takes interpreter options.
            self.assertEqual(
                cli_runner.prefix(reference=True, python_options=("-X", "dev")),
                (sys.executable, "-X", "dev", *cli_runner.REFERENCE_PREFIX[1:]),
            )
        with self.assertRaises(ValueError):
            cli_runner.prefix(python_options=("-X", "dev"))

    def test_unusable_values_fail_closed(self):
        for value in ("   ", "[]", '["", "x"]', "[1]", "[not json", "/nonexistent/kassiber"):
            with self.subTest(value=value), mock.patch.dict(os.environ, {cli_runner.CLI_ENV: value}):
                if value.strip():
                    with self.assertRaises(cli_runner.CandidateConfigError):
                        cli_runner.prefix()
                else:
                    self.assertEqual(cli_runner.prefix(), cli_runner.REFERENCE_PREFIX)

    def test_run_and_popen_start_the_candidate_with_the_product_arguments(self):
        probe = "import json, os, sys; print(json.dumps([sys.argv[1:], os.getcwd()]))"
        candidate = json.dumps([sys.executable, "-c", probe])
        with mock.patch.dict(os.environ, {cli_runner.CLI_ENV: candidate}):
            os.environ.pop(cli_runner.RECORD_ENV, None)
            result = cli_runner.run(["--machine", "status"], capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(result.stdout), [["--machine", "status"], str(cli_runner.ROOT)])
            with cli_runner.popen(["daemon"], stdout=subprocess.PIPE, text=True) as process:
                output, _ = process.communicate(timeout=60)
            self.assertEqual(json.loads(output), [["daemon"], str(cli_runner.ROOT)])

    def test_an_input_fd_reaches_the_child_as_a_readable_descriptor(self):
        probe = "import os, sys; print(os.read(int(sys.argv[2]), 100).decode())"
        with mock.patch.dict(os.environ, {cli_runner.CLI_ENV: json.dumps([sys.executable, "-c", probe])}):
            os.environ.pop(cli_runner.RECORD_ENV, None)
            result = cli_runner.run(
                ["--db-passphrase-fd", cli_runner.InputFd(b"synthetic")], capture_output=True, text=True, check=True
            )
        self.assertEqual(result.stdout.strip(), "synthetic")
        self.assertEqual(cli_runner.argv(["--x", cli_runner.InputFd(b"s")])[-1], "<fd>")


if __name__ == "__main__":
    unittest.main()
