"""Test-only CLI reference that checks each report workbook's Rust rendering.

Use as KASSIBER_TEST_CLI='[".../python", "-m", "tests.rust_port.xlsx_reference"]'
with KASSIBER_XLSX_RENDER_BIN set. Load the same module as a pytest plugin to
check in-process writers too. It is never a hybrid candidate or evidence of
native service routing.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import xlsxwriter

from tests.rust_port.xlsx import RecordingWorkbook, canonical, first_difference


class CheckingWorkbook(RecordingWorkbook):
    def __init__(self, path, options=None):
        super().__init__(path, options)
        self.path = Path(path)

    def close(self):
        super().close()
        with tempfile.TemporaryDirectory(dir=os.environ["TMPDIR"], prefix="xlsx-compare-") as scratch:
            candidate = Path(scratch) / "rust.xlsx"
            request = json.dumps(dict(document=self.document, path=str(candidate))) + "\n"
            result = subprocess.run([os.environ["KASSIBER_XLSX_RENDER_BIN"]], input=request,
                                    text=True, capture_output=True, check=True)
            assert json.loads(result.stdout) == {"ok": True}, result.stdout
            assert not result.stderr
            assert not (difference := first_difference(canonical(self.path), canonical(candidate))), difference
        if summary := os.environ.get("KASSIBER_XLSX_COMPARISON_LOG"):
            with Path(summary).open("a") as handle:
                handle.write(json.dumps(dict(file=self.path.name, sheets=[s["name"] for s in self.document["sheets"]])) + "\n")


def pytest_configure(config):
    xlsxwriter.Workbook = CheckingWorkbook


def main():
    from kassiber.operator import project
    from kassiber.cli.main import main as cli_main
    owners = Path(os.environ["TMPDIR"]) / "xlsx-reference-owners"
    owners.mkdir(mode=0o700, exist_ok=True)
    project._owner_lock_root = lambda: owners
    xlsxwriter.Workbook = CheckingWorkbook
    return cli_main(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())
