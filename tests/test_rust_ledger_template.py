"""The generic-ledger template, the last desktop kind to go native.

`ui.transactions.ledger_template` and `wallets ledger-template` write the
blank fill-in ledger; the Rust daemon and CLI must write the same CSV bytes
and the same workbook (cells, styles, widths, frozen header and the Type
dropdown) as Python, and answer the same envelopes. Only the file's own clock
differs: the managed file name and the workbook's creation time.
"""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

from tests import cli_runner
from tests.rust_port import xlsx as workbook_oracle
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
KIND = "ui.transactions.ledger_template"
STAMP = re.compile(r"kassiber-ledger-template-\d{8}-\d{6}(-\d+)?\.")


def _workbook(path):
    if path.suffix.lower() != ".xlsx":
        # openpyxl reads a workbook only by its extension.
        named = path.with_name(path.name + ".xlsx")
        named.write_bytes(path.read_bytes())
        path = named
    canonical = workbook_oracle.canonical(path)
    # The creation time is the writer's clock.
    tag, attributes, text, children = canonical["properties"]
    canonical["properties"] = [tag, attributes, text, [
        child for child in children if not str(child[0]).endswith(("}created", "}modified"))
    ]]
    return canonical


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustLedgerTemplateTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-ledger-template-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.env = _environment(self.base / "home")

    def daemon(self, root, requests, rust):
        wire = "".join(
            json.dumps({"request_id": str(index), **request}) + "\n"
            for index, request in enumerate([*requests, {"kind": "daemon.shutdown"}])
        ).encode()
        argv = ["--machine", "--data-root", str(root), "daemon"]
        options = dict(input=wire, capture_output=True, timeout=120, env=self.env, cwd=ROOT)
        result = (subprocess.run([RUST_BIN, *argv], **options) if rust
                  else cli_runner.run(argv, reference=True, **options))
        self.assertEqual((result.returncode, result.stderr), (0, b""))
        return [json.loads(line) for line in result.stdout.splitlines()[1:-1]]

    def test_daemon_kind_writes_python_template_and_refusals(self):
        requests = [
            {"kind": KIND, "args": {"format": "csv"}},
            {"kind": KIND},
            {"kind": KIND, "args": {"format": " XLSM "}},
            {"kind": KIND, "args": {"format": "  "}},
            {"kind": KIND, "args": {"format": "CSV"}},
            {"kind": KIND, "args": {"format": "ods"}},
            {"kind": KIND, "args": {"format": "Ödf"}},
            {"kind": KIND, "args": {"format": 5}},
            {"kind": KIND, "args": [1]},
            {"kind": KIND, "args": None},
        ]
        python_root, rust_root = self.base / "python", self.base / "rust"
        reference = self.daemon(python_root, requests, rust=False)
        candidate = self.daemon(rust_root, requests, rust=True)
        self.assertEqual(len(candidate), len(reference))
        written = []
        for request, ours, theirs in zip(requests, candidate, reference):
            with self.subTest(request=request):
                if theirs["kind"] == "error":
                    self.assertEqual(ours, theirs)
                    continue
                ours_file, theirs_file = Path(ours["data"].pop("file")), Path(theirs["data"].pop("file"))
                ours_name, theirs_name = ours["data"].pop("filename"), theirs["data"].pop("filename")
                self.assertEqual(ours, theirs)
                self.assertEqual((ours_file.name, theirs_file.name), (ours_name, theirs_name))
                self.assertEqual(STAMP.sub("S.", ours_name), STAMP.sub("S.", theirs_name))
                # Both land in the book's own exports directory.
                self.assertEqual(ours_file.parent, (rust_root / "exports" / "reports").resolve())
                self.assertEqual(theirs_file.parent, (python_root / "exports" / "reports").resolve())
                written.append((ours_file, theirs_file))
        self.assertEqual(len(written), 6)
        for ours, theirs in written:
            with self.subTest(file=theirs.name):
                if theirs.suffix == ".csv":
                    self.assertEqual(ours.read_bytes(), theirs.read_bytes())
                else:
                    difference = workbook_oracle.first_difference(_workbook(ours), _workbook(theirs))
                    self.assertIsNone(difference)
        # Two requests in the same second get distinct names, never overwrite.
        names = sorted(path.name for path in (rust_root / "exports" / "reports").iterdir())
        self.assertEqual(len(names), 6)

    def test_cli_leaf_writes_the_same_template_without_a_book(self):
        for name in ("template.csv", "Template.CSV", "template.xlsx", "template"):
            with self.subTest(name=name):
                results = []
                for side in ("python", "rust"):
                    target = self.base / side / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    argv = ["--machine", "--data-root", str(self.base / side / "root"),
                            "wallets", "ledger-template", "--file", str(target)]
                    options = dict(env=self.env, capture_output=True, cwd=ROOT, timeout=120)
                    result = (subprocess.run([RUST_BIN, *argv], **options) if side == "rust"
                              else cli_runner.run(argv, reference=True, **options))
                    self.assertEqual(result.returncode, 0, result.stderr)
                    payload = json.loads(result.stdout)
                    self.assertEqual(payload["data"].pop("file"), str(target))
                    results.append((payload, target))
                (theirs, theirs_file), (ours, ours_file) = results
                self.assertEqual(ours, theirs)
                if theirs["data"]["format"] == "csv":
                    self.assertEqual(ours_file.read_bytes(), theirs_file.read_bytes())
                else:
                    self.assertIsNone(workbook_oracle.first_difference(_workbook(ours_file), _workbook(theirs_file)))
                # Neither side created a book for a template.
                for side in ("python", "rust"):
                    self.assertFalse((self.base / side / "root" / "kassiber.sqlite3").exists())


if __name__ == "__main__":
    unittest.main()
