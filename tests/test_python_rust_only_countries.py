"""Python refuses a book of a tax country only the Rust backend supports.

The Rust registry (`crates/tax-countries`) adds countries Python never will
(Python keeps `generic` and `at`, `kassiber/tax_policy.py`). Should the
hybrid ever hand such a book to Python, every read and write that reaches the
tax policy must stop with a typed error naming the country, and leave the
book as it was. `docs/reference/tax-countries.md` records this.
"""

import hashlib
import json
import os
import re
import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.tax_policy import ACTIVE_TAX_COUNTRIES
from tests import cli_runner

REGISTRY = cli_runner.ROOT / "ui-tauri" / "src" / "lib" / "taxCountries.generated.ts"


def rust_only_countries() -> list[str]:
    """The registry's countries Python does not support; `us` before any
    exists, so the refusal is pinned from the start."""
    text = REGISTRY.read_text(encoding="utf-8")
    match = re.search(r"TAX_COUNTRY_REGISTRY = (\[.*?\n\]) as const;", text, re.S)
    codes = [entry["code"] for entry in json.loads(match.group(1))]
    return [code for code in codes if code not in ACTIVE_TAX_COUNTRIES] or ["us"]


COMMANDS = (
    ("profiles", "get"),
    ("profiles", "set", "--gains-algorithm", "LIFO"),
    ("journals", "process"),
    ("reports", "summary"),
    ("reports", "capital-gains"),
    ("reports", "tax-summary"),
    ("reports", "export-pdf", "--file", "{tmp}/report.pdf"),
)


class PythonRefusesRustOnlyCountriesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-rust-only-country-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        # Children must not resolve the owner's real app-data root.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(Path(self.tmp.name) / "offline-mode.json")
        self.cli("init")
        self.cli("workspaces", "create", "Main")
        self.cli("profiles", "create", "Book", "--fiat-currency", "USD")

    def cli(self, *args, check=True):
        result = cli_runner.run(
            ["--data-root", str(self.data_root), "--machine", *args],
            reference=True,
            capture_output=True,
            text=True,
            env=self.env,
            check=False,
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def database(self) -> Path:
        return self.data_root / "kassiber.sqlite3"

    def digest(self) -> str:
        with sqlite3.connect(self.database()) as conn:
            rows = [
                (table, sorted(map(repr, conn.execute(f'SELECT * FROM "{table}"'))))
                for (table,) in conn.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
                )
            ]
        return hashlib.sha256(repr(rows).encode()).hexdigest()

    def test_every_tax_read_and_write_refuses_with_a_typed_error(self):
        for code in rust_only_countries():
            with sqlite3.connect(self.database()) as conn:
                conn.execute("UPDATE profiles SET tax_country=?", (code,))
            before = self.digest()
            for command in COMMANDS:
                args = [part.format(tmp=self.tmp.name) for part in command]
                with self.subTest(country=code, command=" ".join(command)):
                    result = self.cli(*args, check=False)
                    self.assertNotEqual(result.returncode, 0, result.stdout)
                    self.assertNotIn("Traceback", result.stderr)
                    envelope = json.loads(result.stdout)
                    self.assertEqual(envelope["kind"], "error")
                    self.assertEqual(
                        envelope["error"]["message"], f"Unsupported tax country '{code}'"
                    )
            self.assertFalse((Path(self.tmp.name) / "report.pdf").exists())
            self.assertEqual(self.digest(), before, code)


if __name__ == "__main__":
    unittest.main()
