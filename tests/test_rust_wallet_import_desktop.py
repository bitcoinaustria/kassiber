"""Desktop file imports and analysis compared byte for byte with Python.

`ui.wallets.import_file` and `ui.wallets.import_samourai` commit an import,
run the local journal step and end the daemon session, so every import is
its own session on its own copy of the book. Generated identifiers and the
current clock legitimately differ between the two runs; both responses and
all 112 tables are compared after masking only those values.
"""
import base64
import datetime
import json
import os
import re
import unittest

from kassiber.db import open_db
from tests.rust_desktop_parity import DESKTOP_SQL, RUST_BIN
from tests.test_rust_wallet_onboarding import UnlockingParity

UUID = re.compile(rb"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
XPUB = (
    "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189o"
    "LKnC5fSwqPfgyP3hooxujYzAu3fDVmz"
)
XPRV = (
    "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5k"
    "ejMRNNU3TGtRBeJgk33yuGBxrMPHi"
)
JSON_ROWS = [
    {"txid": "funding-001", "occurred_at": "2025-01-03T10:00:00Z", "direction": "inbound", "asset": "BTC",
     "amount": 0.5, "fee": 0, "fiat_rate": 44000, "description": "Initial funding", "kind": "deposit"},
    {"txid": "payment-001", "occurred_at": "2025-02-10T14:30:00Z", "direction": "outbound", "asset": "BTC",
     "amount": 0.1, "fee": 0.0002, "fiat_rate": 51000, "description": "Vendor payment", "kind": "withdrawal"},
]
CSV = ("txid,occurred_at,direction,asset,amount,fee,fiat_rate,description\n"
       "csv-1,2025-03-01T00:00:00Z,inbound,BTC,0.25,0,40000,First\n"
       "csv-2,2025-03-02T00:00:00Z,outbound,BTC,0.05,0.0001,41000,Second\n")
LEDGER = "Wann,Menge,Typ\n2025-04-01,0.5,Kauf\n2025-04-02,0.1,Verkauf\n"
WASABI = {
    "gethistory": [{"datetime": "2025-05-01T00:00:00+00:00", "height": 800000, "amount": 150000,
                    "label": "Salary", "tx": "e" * 64, "islikelycoinjoin": False}],
    "listkeys": [{"keyState": 2}, {"keyState": 0}],
}


def _times():
    today = datetime.datetime.now(datetime.timezone.utc).date()
    days = [today + datetime.timedelta(days=offset) for offset in (-1, 0, 1)]
    dates = b"|".join(day.isoformat().encode() for day in days)
    return re.compile(rb"(?:" + dates + rb")[T ][0-9:.]+(?:Z|[+-]\d{2}:\d{2})?")


ROOTS = re.compile(rb"/(?:candidate-\d+|encrypted)(?=[/\"])")


def mask_line(line, *, times=_times()):
    return ROOTS.sub(b"/<book>", times.sub(b"<now>", UUID.sub(b"<uuid>", line)))


def mask_value(value):
    if isinstance(value, str):
        return mask_line(value.encode()).decode()
    if isinstance(value, bytes):
        return mask_line(value)
    return value


def mask_tables(tables):
    return {name: sorted((tuple(mask_value(v) for v in row) for row in rows), key=repr)
            for name, rows in tables.items()}


def record_served(kind):
    """The harness skips a session's last line, which ends an import session."""
    if path := os.environ.get("KASSIBER_DESKTOP_COUNTS"):
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(dict(kind=kind, served_by="rust", ok=True)) + "\n")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustDesktopImportTest(UnlockingParity, unittest.TestCase):
    KIND = "ui.wallets.import_file"

    def seed(self):
        files = self.base / "files"
        files.mkdir(exist_ok=True)
        self.files = {}
        for name, text in (("rows.json", json.dumps(JSON_ROWS)), ("rows.csv", CSV), ("ledger.csv", LEDGER),
                           ("wasabi.json", json.dumps({**WASABI, "gethistory": [{**WASABI["gethistory"][0], "tx": "f" * 64}]})), ("notes.pdf", "%PDF-1.4\n")):
            (files / name).write_text(text)
            self.files[name] = str(files / name)
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(DESKTOP_SQL.read_text())
            conn.executescript("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                               "VALUES('imports','w','p','Imports','custom','{}','2025');")
        self.change("")

    def imports(self, args, **kwargs):
        """One import session; the import response is the session's last line."""
        before = self.count()
        self.compare(args, mutates=True, mask=mask_line, tables=mask_tables, **kwargs)
        self.assertGreater(self.count(), before, "the reference imported rows")
        record_served(self.KIND)

    def count(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            return conn.execute("SELECT COUNT(*) FROM transactions WHERE wallet_id='imports'").fetchone()[0]

    def test_refusals_keep_the_session_and_match_python(self):
        files = self.files
        self.compare([
            ..., None, [], {}, {"source_format": 3}, {"source_format": " "},
            {"source_format": "csv"}, {"source_format": "csv", "source_file": 4},
            {"source_format": "csv", "source_file": "/no/such/export.csv"},
            {"source_format": "pdf", "source_file": files["rows.csv"], "wallet": "imports"},
            {"source_format": "csv", "source_bundle": {}},
            {"source_format": "wasabi_bundle", "source_bundle": {}},
            {"source_format": "wasabi_bundle", "source_bundle": {}, "wallet": "absent"},
            {"source_format": "csv", "source_file": files["rows.csv"]},
            {"source_format": "csv", "source_file": files["rows.csv"], "wallet": "absent"},
            {"source_format": "generic_ledger", "source_file": files["ledger.csv"], "wallet": "imports",
             "column_map": []},
            {"source_format": "generic_ledger", "source_file": files["ledger.csv"], "wallet": "imports",
             "column_map": {"date": "Nope", "amount": "Menge"}},
            {"source_format": "bullbitcoin_csv", "source_file": files["rows.csv"], "mode": "bogus"},
            {"source_format": "strike_csv", "source_file": files["rows.csv"], "wallet": "imports"},
        ])

    def non_json_files(self):
        files = self.base / "files"
        (files / "latin.json").write_bytes(b'[{"description": "caf\xe9"}]')
        (files / "bom.json").write_bytes(b"\xef\xbb\xbf[]")
        return [{"source_format": source_format, "source_file": path, "wallet": "imports"}
                for path in (self.files["rows.csv"], str(files / "latin.json"), str(files / "bom.json"))
                for source_format in ("json", "btcpay_json", "wasabi_bundle")]

    def test_non_json_files_under_json_formats_are_validation_refusals(self):
        # tests/rust_port/repro_import_json_format_on_csv.py
        responses = self.compare(self.non_json_files())
        self.assertEqual({(r["error"]["code"], r["error"]["hint"]) for r in responses},
                         {("validation", "Choose the import format that matches the file.")})

    def test_sqlcipher_non_json_files_are_validation_refusals(self):
        self.encrypt("synthetic import key")
        responses = self.compare(self.non_json_files()[:3])
        self.assertEqual([r["error"]["code"] for r in responses if r["kind"] == "error"], ["validation"] * 3)

    def test_json_import_runs_the_journal_step(self):
        self.imports([{"source_format": "json", "source_file": self.files["rows.json"], "wallet": " Imports "}])

    def test_csv_import_with_journals_disabled(self):
        self.change("INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',"
                    "'{\"source_classes\": {\"journals\": false}}');")
        self.imports([{"source_format": "csv", "source_file": self.files["rows.csv"], "wallet": "imports"}])

    def test_generic_ledger_import_with_a_column_map(self):
        self.imports([{"source_format": "generic_ledger", "source_file": self.files["ledger.csv"],
                       "wallet": "imports", "column_map": {"date": "Wann", "amount": "Menge", "type": "Typ",
                                                           "type_map": {"Kauf": "Buy", "Verkauf": "Sell"}}}])

    def test_inline_wasabi_bundle(self):
        self.imports([{"source_format": "wasabi_bundle", "source_bundle": WASABI, "wallet": "imports"}])

    def test_wasabi_bundle_file(self):
        self.imports([{"source_format": "wasabi_bundle", "source_file": self.files["wasabi.json"],
                       "wallet": "imports"}])

    def test_sqlcipher_import_matches_python(self):
        self.encrypt("synthetic import key")
        self.imports([{"source_format": "json", "source_file": self.files["rows.json"], "wallet": "imports"}])


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustDesktopSamouraiTest(UnlockingParity, unittest.TestCase):
    KIND = "ui.wallets.import_samourai"

    def seed(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(DESKTOP_SQL.read_text())
            conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) "
                         "VALUES('treasury','w','p','treasury','Treasury','asset','2025')")
        self.change("")

    def test_refusals_match_python(self):
        self.compare([
            ..., None, {}, {"label": 3}, {"label": "S", "mnemonic": "abandon"},
            {"label": "S", "backup_file": "", "source_file": "/x"}, {"label": "S", "source_set": []},
            {"label": "S", "gap_limit": "20"}, {"label": "S", "gap_limit": 2.5}, {"label": "S", "account": 3},
            {"label": "S"}, {"label": "S", "source_set": {"deposit": {"bip84": XPRV}}},
            {"label": "S", "source_set_file": "/no/such/set.json"},
            {"label": "S", "source_set": {"deposit": {"bip84": XPUB}}},
            {"label": "S", "source_set": {"xpubs": [{"xpub": XPUB}]}},
        ])

    def test_watch_only_group_import_matches_python(self):
        self.compare([{"label": "Samourai", "network": "main", "gap_limit": 5,
                       "source_set": {"xpubs": [{"section": "deposit", "script_type": "p2wpkh",
                                                 "root_path": "m/84'/0'/0'", "xpub": XPUB,
                                                 "fingerprint": "D34DB33F"}]}}],
                     mutates=True, mask=mask_line, tables=mask_tables)
        with open_db(self.root) as conn:
            self.assertTrue(conn.execute("SELECT COUNT(*) FROM wallets WHERE label LIKE 'Samourai%'").fetchone()[0])
        record_served(self.KIND)


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustAnalyzeFileTest(UnlockingParity, unittest.TestCase):
    KIND = "ui.wallets.analyze_file"

    def upload(self, text, filename="export.csv", **extra):
        return {"source_bytes_base64": base64.b64encode(text.encode()).decode(), "filename": filename, **extra}

    def test_uploads_tokens_and_refusals_match_python(self):
        self.compare([
            ..., None, [], {}, {"token": 3}, {"token": "missing-grant"}, {"sample_rows": "x"},
            {"sample_rows": True}, {"column_map": "date=Wann"}, {"source_bytes_base64": ""},
            self.upload("x", filename="export.doc"), self.upload("x", filename="README"),
            {"source_bytes_base64": "AA=A", "filename": "a.csv"},
            self.upload("Timestamp,Side,BTC Amount,Fee,Currency,Total\n2024-03-01T10:00:00Z,Buy,0.5,0.0001,EUR,21000\n"
                        "2024-04-02T10:00:00Z,Sell,0.25,0.0001,EUR,12000\n"),
            self.upload("Timestamp;Side;BTC Amount\n2024-03-01T10:00:00Z;Buy;0,5\n", filename="semi.TSV"),
            self.upload("Ausführung,Richtung,Stück\n2024-03-01T10:00:00Z,Buy,0.5\n"),
            self.upload("Ausführung,Richtung,Stück\n2024-03-01T10:00:00Z,Buy,0.5\n",
                        column_map={"date": "Ausführung", "type": "Richtung", "amount": "Stück"}),
            self.upload("Trade Date,Action,Qty,Currency,Total\n2024-05-01T09:00:00Z,ACQ-MKT,1.5,EUR,60000\n",
                        column_map={"date": "Trade Date", "type": "Action", "amount": "Qty",
                                    "fiat_currency": "Currency", "fiat_value": "Total",
                                    "type_map": {"ACQ-MKT": "Buy", "acq mkt": "Sell"}}),
            self.upload("Date,Amount\n2024-01-01,250000\n2024-01-02,-1000\n", sample_rows=0),
            self.upload("Date,Amount\n2024-01-01,0.5\n", sample_rows=99),
            self.upload("Account holder,Max Mustermann\nDate,Amount,Fee\n2024-01-01,1,0\n2024-01-02,2,0\n"),
            self.upload("just one column\nno delimiter\n", filename="notes.txt"),
            self.upload("%PDF-1.4\n", filename="statement.pdf"), self.upload("x", filename="photo.JPG"),
            self.upload("", filename="empty.csv"),
        ])

    def test_sqlcipher_analysis_matches_python(self):
        self.encrypt("synthetic analysis key")
        self.compare([self.upload("Date,Amount\n2024-01-01,0.5\n")])
