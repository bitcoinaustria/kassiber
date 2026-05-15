"""Regression coverage for the packaged-build rp2 import path.

The third-party ``rp2.logger`` binds a ``logging.FileHandler`` to
``./log/rp2_<timestamp>.log`` at module-import time. When Kassiber ships
as a macOS .app, the daemon's working directory inherits the bundle's
read-only ``Contents/Resources`` directory, so a naive rp2 import crashes
with EACCES. ``_get_rp2_modules`` defends against this by priming
``rp2.logger`` under a writable scratch cwd first; this test pins that
behavior by spawning a fresh interpreter whose cwd is a chmod-555
directory and asserting the import succeeds.
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


_ROOT = Path(__file__).resolve().parents[1]


class Rp2ReadOnlyCwdImportTest(unittest.TestCase):
    def test_get_rp2_modules_succeeds_under_read_only_cwd(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="kassiber-ro-cwd-"))
        try:
            os.chmod(scratch, stat.S_IRUSR | stat.S_IXUSR)
            script = textwrap.dedent(
                """
                from kassiber.core.engines.rp2 import _get_rp2_modules

                modules = _get_rp2_modules()
                assert "InTransaction" in modules, modules.keys()
                assert "Configuration" in modules, modules.keys()
                print("ok")
                """
            ).strip()
            env = {**os.environ, "PYTHONPATH": str(_ROOT)}
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


if __name__ == "__main__":
    unittest.main()
