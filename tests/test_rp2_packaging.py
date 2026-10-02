"""Regression coverage for the RP2 backend's import path.

Kassiber's own tax engine is the product backend; RP2 remains the parity
oracle that tests select (plan 20). Older ``rp2.logger`` releases bound a
``logging.FileHandler`` to ``./log/rp2_<timestamp>.log`` at import time,
which crashed with EACCES under a read-only cwd such as a macOS bundle's
``Contents/Resources``. ``_get_rp2_modules`` defends against this by priming
``rp2.logger`` under a writable scratch cwd first; these tests pin that
behavior by spawning a fresh interpreter whose cwd is a chmod-555
directory and asserting the RP2 backend still loads without a file logger.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

from kassiber.errors import AppError


_ROOT = Path(__file__).resolve().parents[1]


class Rp2ReadOnlyCwdImportTest(unittest.TestCase):
    def test_get_rp2_modules_succeeds_under_read_only_cwd(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="kassiber-ro-cwd-"))
        try:
            os.chmod(scratch, stat.S_IRUSR | stat.S_IXUSR)
            script = textwrap.dedent(
                """
                from kassiber.core.engines.rp2 import _get_rp2_modules, _use_tax_engine_backend

                with _use_tax_engine_backend("rp2"):
                    modules = _get_rp2_modules()
                assert "InTransaction" in modules, modules.keys()
                assert "Configuration" in modules, modules.keys()
                print("ok")
                """
            ).strip()
            env = os.environ.copy()
            env["PYTHONPATH"] = os.pathsep.join(
                path for path in (str(_ROOT), env.get("PYTHONPATH")) if path
            )
            result = subprocess.run(
                [sys.executable, "-c", script],
                cwd=scratch,
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=60,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=(
                    "rp2 import under a read-only cwd should succeed; "
                    f"stdout={result.stdout!r} stderr={result.stderr!r}"
                ),
            )
            self.assertIn("ok", result.stdout)
        finally:
            os.chmod(scratch, stat.S_IRWXU)
            shutil.rmtree(scratch, ignore_errors=True)

    def test_get_rp2_modules_leaves_no_file_logger_or_log_file(self) -> None:
        with tempfile.TemporaryDirectory(prefix="kassiber-rp2-log-test-") as tmp:
            script = textwrap.dedent(
                f"""
                import logging
                import pathlib
                import tempfile

                tempfile.tempdir = {str(tmp)!r}

                from kassiber.core.engines.rp2 import _get_rp2_modules, _use_tax_engine_backend

                with _use_tax_engine_backend("rp2"):
                    modules = _get_rp2_modules()
                assert "InTransaction" in modules, modules.keys()
                root = pathlib.Path(tempfile.gettempdir())
                log_files = list(root.rglob("rp2_*.log"))
                scratch_dirs = list(root.glob("kassiber-rp2-logs-*"))
                handlers = logging.getLogger("rp2").handlers
                assert not log_files, [str(path) for path in log_files]
                assert not scratch_dirs, [str(path) for path in scratch_dirs]
                assert not any(isinstance(handler, logging.FileHandler) for handler in handlers), handlers
                assert any(isinstance(handler, logging.NullHandler) for handler in handlers), handlers
                print("ok")
                """
            ).strip()
            env = os.environ.copy()
            env["PYTHONPATH"] = os.pathsep.join(
                path for path in (str(_ROOT), env.get("PYTHONPATH")) if path
            )
            result = subprocess.run(
                [sys.executable, "-c", script],
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=60,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=(
                    "rp2 import should leave no disk logger; "
                    f"stdout={result.stdout!r} stderr={result.stderr!r}"
                ),
            )
            self.assertIn("ok", result.stdout)

    def test_rp2_configuration_rejects_delimited_tokens(self) -> None:
        from kassiber.core.engines.rp2 import _rp2_configuration

        cases = [
            ("wallet_label", {"label": "Default"}, ["Cold, Savings"], ["BTC"]),
            ("profile_label", {"label": "Alice\nBob"}, ["Cold"], ["BTC"]),
            ("asset", {"label": "Default"}, ["Cold"], ["BTC=USD"]),
        ]
        for field, profile, wallet_labels, assets in cases:
            with self.subTest(field=field):
                with self.assertRaises(AppError) as raised:
                    with _rp2_configuration(profile, wallet_labels, assets):
                        pass

                self.assertEqual(raised.exception.code, "validation")
                self.assertEqual(raised.exception.details["field"], field)


if __name__ == "__main__":
    unittest.main()
