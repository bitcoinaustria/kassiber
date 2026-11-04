"""Native review workflow sessions compared with the unchanged Python daemon.

Each session runs the same JSONL requests against Python and the Rust binary
on copies of one disposable book, requiring identical response bytes and
identical rows in all 112 logical tables. Applied sessions mask only generated
UUIDs (through one consistent bijection) and the clocks sampled by each run.
"""
import base64
from datetime import datetime, timedelta, timezone
import json
import os
import re
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests.rust_desktop_parity import DesktopParity

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
REQUIRE_RUST = os.environ.get("KASSIBER_REQUIRE_RUST_PARITY") == "1"
NOW = "2026-01-01T00:00:00Z"

TRANSACTIONS = (
    # id, wallet, direction, amount (msat), fiat rate, occurred
    ("out", "source-wallet", "outbound", 100_000, None, NOW),
    ("in", "sink-wallet", "inbound", 99_000, None, NOW),
    ("buy", "sink-wallet", "inbound", 5_000_000, None, "2024-02-01T00:00:00Z"),
    ("sell", "sink-wallet", "outbound", 1_000_000, 50_000, "2024-03-01T00:00:00Z"),
    ("gift", "source-wallet", "inbound", 7_000_000, 40_000, "2023-06-01T00:00:00Z"),
)


def cursor(scope, after):
    return base64.urlsafe_b64encode(json.dumps(
        {"scope": scope, "after": after}, sort_keys=True, separators=(",", ":"),
    ).encode()).decode()


_UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
_STAMP = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,6})?(?:Z|\+00:00)")


class Masker:
    """Map each generated UUID to its order of first use and recent clocks to
    one token. Response lines are walked first, then tables in name order with
    rows ordered by their masked text, so the mapping never depends on the
    random values themselves."""

    def __init__(self, since):
        self.since = since
        self.ids = {}

    def _clock(self, match):
        text = match.group(0)
        try:
            stamp = datetime.fromisoformat(text.replace("Z", "+00:00"))
        except ValueError:
            return text
        return "<now>" if stamp >= self.since else text

    def _blank(self, text):
        return _STAMP.sub(self._clock, _UUID.sub("<uuid>", text))

    def text(self, text):
        def identity(match):
            return self.ids.setdefault(match.group(0), f"<uuid:{len(self.ids)}>")
        return _STAMP.sub(self._clock, _UUID.sub(identity, text))

    def value(self, value):
        return self.text(value) if isinstance(value, str) else value

    def tables(self, snapshot):
        result = {}
        for table in sorted(snapshot):
            rows = sorted(snapshot[table], key=lambda row: repr(tuple(
                self._blank(value) if isinstance(value, str) else value for value in row)))
            result[table] = sorted((tuple(self.value(value) for value in row) for row in rows), key=repr)
        return result


@unittest.skipUnless(RUST_BIN or REQUIRE_RUST, "KASSIBER_RUST_BIN names the Rust binary")
class ReviewParity(DesktopParity):
    SEED = None
    PAIR = False

    def setUp(self):
        self.started = datetime.now(timezone.utc) - timedelta(seconds=1)
        super().setUp()

    def seed(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main',?)", (NOW,))
            conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book',?)", (NOW,))
            conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) "
                         "VALUES('treasury','ws','profile','treasury','Treasury','asset',?)", (NOW,))
            for wallet_id, label in (("source-wallet", "Source"), ("sink-wallet", "Sink")):
                conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                             "VALUES(?,'ws','profile',?,'custom','{}',?)", (wallet_id, label, NOW))
            for tx_id, wallet_id, direction, amount, rate, occurred in TRANSACTIONS:
                conn.execute(
                    "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
                    "occurred_at,direction,asset,amount,fee,fiat_rate,created_at) "
                    "VALUES(?,'ws','profile',?,?,?,?,?,'BTC',?,0,?,?)",
                    (tx_id, wallet_id, f"fingerprint-{tx_id}", f"external-{tx_id}", occurred,
                     direction, amount, rate, occurred))
            conn.execute("INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile')")
            conn.commit()
        if self.PAIR:
            self.python("transfers", "pair", "--tx-out", "out", "--tx-in", "in")

    def python(self, *argv):
        args = ["--machine", "--data-root", str(self.root)]
        if self.passphrase:
            args.extend(["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())])
        result = cli_runner.run([*args, *argv], reference=True, env=self.env, capture_output=True, timeout=300)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)["data"]

    def profile_scope(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            row = conn.execute("SELECT id, journal_input_version, last_processed_at FROM profiles").fetchone()
        return [row[0], int(row[1] or 0), row[2]]

    def python_cases(self):
        from kassiber.core import review_workflow
        with open_db(self.root, passphrase=self.passphrase) as conn:
            profile = conn.execute("SELECT * FROM profiles").fetchone()
            return review_workflow.inspect_cases(conn, profile, limit=100)

    def masked(self, args, **options):
        masker = Masker(self.started)
        self.compare(args, mutates=True, mask=masker.text, tables=masker.tables, **options)


class ReviewReadParity(ReviewParity, unittest.TestCase):
    KIND = "ui.review.cases"

    def test_cases_pages_cursors_and_refusals_match_python(self):
        scope = self.profile_scope()
        self.compare([
            ..., None, {}, {"limit": 1}, {"limit": 2, "cursor": cursor(scope, "buy")},
            {"limit": 100, "cursor": cursor(scope, "")}, {"cursor": None},
            {"limit": 0}, {"limit": 101}, {"limit": "5"}, {"limit": True}, {"limit": 1.0},
            {"cursor": "bad"}, {"cursor": 5}, {"cursor": cursor([scope[0], scope[1] + 1, scope[2]], "buy")},
            {"cursor": base64.urlsafe_b64encode(b'{"scope":' + json.dumps(scope).encode() + b',"after":7}').decode()},
            {"cursor": "%%%"}, {"limit": 1, "extra": 1}, [], "text",
        ])

    def test_cases_after_processing_report_fresh_receipts_and_waiting_roots(self):
        self.python("journals", "process")
        scope = self.profile_scope()
        self.compare([{}, {"limit": 1}, {"cursor": cursor(scope, "in")}])

    def test_receipts_match_python_by_id_and_key(self):
        self.python("journals", "process")
        version = self.profile_scope()[1]
        operations = [{"type": "exclude", "transaction_id": "sell", "reason": "Duplicate import"}]
        with open_db(self.root, passphrase=self.passphrase) as conn:
            from kassiber.core import review_workflow
            from kassiber.cli.handlers import _metadata_hooks
            profile = conn.execute("SELECT * FROM profiles").fetchone()
            hooks = review_workflow.ReviewHooks(metadata=_metadata_hooks())
            artifact = review_workflow.plan_review(conn, profile, operations=operations,
                                                   expected_input_version=version, hooks=hooks)
            receipt = review_workflow.apply_review(conn, profile, artifact=artifact,
                                                   idempotency_key="key-1", hooks=hooks, authored_source="cli")
        self.compare([
            {"receipt_id": receipt["id"]}, {"idempotency_key": "key-1"},
            {"receipt_id": receipt["id"], "idempotency_key": "key-1"}, {}, ...,
            {"receipt_id": None, "idempotency_key": "key-1"}, {"receipt_id": "missing"},
            {"idempotency_key": 7}, {"receipt_id": "x", "workspace": "ws"},
        ], kind="ui.review.receipt")
        self.compare([{}, {"limit": 1}])

    def test_encrypted_book_reads_match_python(self):
        self.encrypt("correct horse battery staple")
        self.compare([{}, {"limit": 2}])


class ReviewPairParity(ReviewParity, unittest.TestCase):
    KIND = "ui.review.cases"
    PAIR = True

    def test_pair_residual_cases_name_their_pair(self):
        self.compare([{}, {"limit": 1}])
        self.python("journals", "process")
        self.compare([{}])


if __name__ == "__main__":
    unittest.main()
