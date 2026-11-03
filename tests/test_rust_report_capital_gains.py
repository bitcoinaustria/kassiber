import json
import re
import unittest

from kassiber.db import open_db
from tests.rust_desktop_parity import ROOT, RUST_BIN, DesktopParity, run_candidate
from crates.reports.scripts.gen_gain_vectors import BASE

BOOK = (ROOT / "crates/reports/tests/data/book.sql").read_text() + BASE
GENERATED = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z")


def generated(line):
    return GENERATED.sub("<generated>", line.decode() if isinstance(line, bytes) else line)


def journal_shape(snapshot):
    """Rebuilt journals compared without generated identifiers and clocks."""
    return {table: sorted(generated(repr(row)) for row in rows) for table, rows in snapshot.items()}


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustReportCapitalGainsTest(DesktopParity, unittest.TestCase):
    KIND = "ui.reports.capital_gains"
    SEED = None

    def seed(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(BOOK)

    def test_current_book_years_aliases_and_refusals_match_wire_bytes(self):
        self.compare([..., None, {}, {"year": 2024}, {"tax_year": 2025}, {"year": "2024"},
                      {"year": 2024, "tax_year": 2025}, {"year": None, "tax_year": 2024},
                      {"year": 1999}, {"year": "x"}, {"year": True}, {"year": 2024.0},
                      [], 3, "x", {"unknown": 1}])
        self.change("UPDATE profiles SET tax_country='at' WHERE id='p';")
        self.compare([{}, {"year": 2024}, {"year": 2025}])
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([..., {}, {"year": 2024}, []])

    def test_stale_book_rebuilds_journals_before_the_read(self):
        self.change("UPDATE profiles SET journal_input_version=8 WHERE id='p';")
        responses = self.compare([{}, {"year": 2024}, {"year": "bad"}], mutates=True,
                                 mask=generated, tables=journal_shape)
        self.assertEqual(responses[0]["kind"], self.KIND)
        with open_db(self.root, passphrase=self.passphrase) as conn:
            versions = conn.execute(
                "SELECT journal_input_version,last_processed_input_version FROM profiles WHERE id='p'").fetchone()
        self.assertEqual(versions[0], versions[1])

    def test_report_read_sync_policy_fails_closed_in_rust(self):
        self.change("UPDATE profiles SET journal_input_version=8 WHERE id='p';"
                    "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',"
                    "'{\"report_read_sync\": true}');")
        requests = self.requests([{}])
        wire = "".join(json.dumps(request) + "\n" for request in [*requests, dict(kind="daemon.shutdown")])
        result = run_candidate(["--machine", "--data-root", str(self.root), "daemon"],
                               env=self.env, input=wire.encode())
        response = json.loads(result.stdout.splitlines()[1])
        self.assertEqual(response["error"]["code"], "not_implemented")
        with open_db(self.root, passphrase=self.passphrase) as conn:
            self.assertEqual(conn.execute(
                "SELECT last_processed_input_version FROM profiles WHERE id='p'").fetchone()[0], 7)

    def test_sqlcipher_capital_gains_matches_wire_bytes(self):
        self.encrypt("synthetic gains key")
        self.compare([{}, {"year": 2024}, {"year": "bad"}])
