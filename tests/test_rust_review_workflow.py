"""Native review workflow sessions compared with the unchanged Python daemon.

Each session runs the same JSONL requests against Python and the Rust binary
on copies of one disposable book, requiring identical response bytes and
identical rows in all 112 logical tables. Applied sessions mask only generated
UUIDs (through one consistent bijection) and the clocks sampled by each run.
"""
import base64
from contextlib import closing
from datetime import datetime, timedelta, timezone
import json
import os
import re
import shutil
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests.rust_desktop_parity import DesktopParity, run_candidate

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
    one token. The harness normalizes the candidate, then the reference, for
    every line and then for the tables, so calls alternate between two
    independent maps. Tables are walked in name order with rows ordered by
    their masked text (identities the responses named keep their tokens), so
    no mapping depends on the random values themselves."""

    def __init__(self, since):
        self.since = since
        self.maps = ({}, {})
        self.calls = 0

    def _side(self):
        self.calls += 1
        return self.maps[(self.calls - 1) % 2]

    def _clock(self, match):
        text = match.group(0)
        try:
            stamp = datetime.fromisoformat(text.replace("Z", "+00:00"))
        except ValueError:
            return text
        return "<now>" if stamp >= self.since else text

    def _blank(self, ids, text):
        # Identities already mapped (by the responses) break ties between rows.
        return _STAMP.sub(self._clock, _UUID.sub(lambda match: ids.get(match.group(0), "<uuid>"), text))

    def _text(self, ids, text):
        def identity(match):
            return ids.setdefault(match.group(0), f"<uuid:{len(ids)}>")
        return _STAMP.sub(self._clock, _UUID.sub(identity, text))

    def text(self, line, side=None):
        ids = self._side() if side is None else self.maps[side]
        return self._text(ids, line.decode() if isinstance(line, bytes) else line)

    def tables(self, snapshot, side=None):
        ids = self._side() if side is None else self.maps[side]
        result = {}
        for table in sorted(snapshot):
            rows = sorted(snapshot[table], key=lambda row: repr(tuple(
                self._blank(ids, value) if isinstance(value, str) else value for value in row)))
            result[table] = sorted((tuple(self._text(ids, value) if isinstance(value, str) else value
                                          for value in row) for row in rows), key=repr)
        return result


@unittest.skipUnless(RUST_BIN or REQUIRE_RUST, "KASSIBER_RUST_BIN names the Rust binary")
class ReviewParity(DesktopParity):
    SEED = None
    PAIR = False

    def setUp(self):
        self.started = datetime.now(timezone.utc) - timedelta(seconds=1)
        super().setUp()

    def seed(self):
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
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

    def change(self, sql):
        # A connection left open across a daemon session that removes the WAL
        # makes this process's next read fail, so close each one explicitly.
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
            conn.executescript(sql)
            conn.commit()

    def python(self, *argv):
        args = ["--machine", "--data-root", str(self.root)]
        if self.passphrase:
            args.extend(["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())])
        result = cli_runner.run([*args, *argv], reference=True, env=self.env, capture_output=True, timeout=300)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)["data"]

    def profile_scope(self):
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
            row = conn.execute("SELECT id, journal_input_version, last_processed_at FROM profiles").fetchone()
        return [row[0], int(row[1] or 0), row[2]]

    def python_cases(self):
        from kassiber.core import review_workflow
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
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
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
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


def component_spec():
    return {
        "component_type": "manual_bridge", "evidence_kind": "manual_claim",
        "evidence_grade": "reviewed", "notes": "migration",
        "legs": [
            {"role": "source", "transaction": "external-out", "amount_msat": 100_000},
            {"role": "destination", "transaction": "external-in", "amount_msat": 99_000},
            {"role": "fee", "transaction": "external-out", "amount_msat": 1_000},
        ],
    }


def price(tx="buy", **fields):
    return {"type": "price_override", "transaction_id": tx, "reason": "Verified exchange rate",
            **(fields or {"fiat_rate": "45000"})}


class ReviewPlanParity(ReviewParity, unittest.TestCase):
    KIND = "ui.review.plan"

    def request_inputs(self, version):
        return [
            {"action": "connect_wallet", "case_ids": ["quarantine:out", "quarantine:buy"],
             "expected_input_version": version},
            {"action": "import_history", "case_ids": ["quarantine:sell"], "expected_input_version": version,
             "explanation": "  Older exchange history\n\tis missing  "},
            {"action": "attach_evidence", "case_ids": ["quarantine:buy"], "expected_input_version": version,
             "explanation": None},
            {"action": "attach_evidence", "case_ids": ["quarantine:buy", "quarantine:out"],
             "expected_input_version": version},
            {"action": "export", "case_ids": ["quarantine:buy"], "expected_input_version": version},
            {"action": ["connect_wallet"], "case_ids": ["quarantine:buy"], "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": "quarantine:buy", "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": [], "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": [f"quarantine:{i}" for i in range(21)],
             "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy", "quarantine:buy"],
             "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": ["buy"], "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": ["quarantine:" + "x" * 290], "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": [7], "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": version,
             "explanation": "   "},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": version,
             "explanation": "bell\x07"},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": version,
             "explanation": "y" * 1001},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": version,
             "explanation": 5},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": version + 1},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": str(version)},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": True},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": -1},
            {"action": "connect_wallet", "case_ids": ["quarantine:gift"], "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": ["quarantine:missing"], "expected_input_version": version},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"]},
            {"action": "connect_wallet", "case_ids": ["quarantine:buy"], "expected_input_version": version,
             "extra": True},
        ]

    def plans(self, version):
        return [
            {"operations": [price()], "expected_input_version": version},
            {"operations": [price(fiat_value="12.50")], "expected_input_version": version},
            {"operations": [price(fiat_rate="4.5E+4")], "expected_input_version": version},
            {"operations": [{"type": "exclude", "transaction_id": " sell ", "reason": " Duplicate import "}],
             "expected_input_version": version},
            {"operations": [{"type": "kind_override", "transaction_id": "buy", "kind": "income",
                             "reason": "Mining payout"}], "expected_input_version": version},
            {"operations": [{"type": "kind_override", "transaction_id": "gift", "kind": None,
                             "valuation_mode": "market_value", "reason": "Not income"}],
             "expected_input_version": version},
            {"operations": [{"type": "custody_component",
                             "request": {"action": "create", "components": [component_spec()]}}],
             "expected_input_version": version},
            {"operations": [price("out", fiat_rate="50000"), price("in", fiat_rate="50000"),
                            {"type": "custody_component",
                             "request": {"action": "create", "components": [component_spec()]}},
                            {"type": "exclude", "transaction_id": "sell", "reason": "Duplicate"}],
             "expected_input_version": version},
            {"operations": [], "expected_input_version": version},
            {"operations": [price()] * 51, "expected_input_version": version},
            {"operations": "nope", "expected_input_version": version},
            {"operations": [price(fiat_rate="1", fiat_value="2")], "expected_input_version": version},
            {"operations": [price(fiat_rate="0")], "expected_input_version": version},
            {"operations": [price(fiat_value="-1")], "expected_input_version": version},
            {"operations": [price(fiat_rate="NaN")], "expected_input_version": version},
            {"operations": [price(fiat_rate=45000)], "expected_input_version": version},
            {"operations": [{"type": "transfer", "transaction_id": "buy", "reason": "x"}],
             "expected_input_version": version},
            {"operations": [{"type": "exclude", "transaction_id": "buy", "reason": "x", "note": 1}],
             "expected_input_version": version},
            {"operations": [{"type": "exclude", "transaction_id": "buy", "reason": "x" * 2001}],
             "expected_input_version": version},
            {"operations": [{"type": "exclude", "transaction_id": "gift", "reason": "Not a case"}],
             "expected_input_version": version},
            {"operations": [{"type": "exclude", "transaction_id": "missing", "reason": "Gone"}],
             "expected_input_version": version},
            {"operations": [{"type": "kind_override", "transaction_id": "sell", "kind": "income",
                             "reason": "Wrong direction"}], "expected_input_version": version},
            {"operations": [{"type": "kind_override", "transaction_id": "buy", "kind": "sell",
                             "reason": "Outbound kind"}], "expected_input_version": version},
            {"operations": [{"type": "kind_override", "transaction_id": "buy", "kind": "income",
                             "valuation_mode": "zero_basis", "reason": "x"}], "expected_input_version": version},
            {"operations": [{"type": "custody_component", "request": {"action": "explode"}}],
             "expected_input_version": version},
            {"operations": [{"type": "custody_component",
                             "request": {"action": "create", "components": [component_spec()], "x": 1}}],
             "expected_input_version": version},
            {"operations": [{"type": "unpair", "pair_id": "p", "expected_fingerprint": "f"}],
             "expected_input_version": version},
            {"operations": [price()], "expected_input_version": version + 1},
            {"operations": [price()], "expected_input_version": 1.0},
            {"operations": [price()]},
            {"operations": [price()], "expected_input_version": version, "workspace": "ws"},
        ]

    def test_input_requests_bind_current_cases_and_version(self):
        self.compare(self.request_inputs(self.profile_scope()[1]), kind="ui.review.request_input")

    def test_plans_are_read_only_previews_matching_python(self):
        self.compare(self.plans(self.profile_scope()[1]))

    def test_processed_book_plans_and_requests_match_python(self):
        self.python("journals", "process")
        version = self.profile_scope()[1]
        self.compare(self.plans(version)[:9] + self.plans(version)[-3:])
        self.compare(self.request_inputs(version)[:4], kind="ui.review.request_input")

    def test_encrypted_book_plans_in_ram_like_python(self):
        self.encrypt("correct horse battery staple")
        version = self.profile_scope()[1]
        self.compare(self.plans(version)[:8])
        self.compare(self.request_inputs(version)[:3], kind="ui.review.request_input")


def planned(case, operations):
    """Python's own artifact for ``operations`` on the seeded book (read-only)."""
    from kassiber.cli.handlers import _metadata_hooks
    from kassiber.core import review_workflow
    with closing(open_db(case.root, passphrase=case.passphrase)) as conn:
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        return review_workflow.plan_review(
            conn, profile, operations=operations, hooks=review_workflow.ReviewHooks(metadata=_metadata_hooks()),
            expected_input_version=int(profile["journal_input_version"] or 0))


def redigest(artifact, **changes):
    from kassiber.core.review_workflow import _digest
    changed = {key: value for key, value in {**artifact, **changes}.items() if key != "digest"}
    return {**changed, "digest": _digest(changed)}


MIXED = [price("out", fiat_rate="50000"), price("in", fiat_rate="50000"),
         {"type": "custody_component", "request": {"action": "create", "components": [component_spec()]}},
         {"type": "exclude", "transaction_id": "sell", "reason": "Duplicate"}]


class ReviewApplyParity(ReviewParity, unittest.TestCase):
    KIND = "ui.review.apply"

    def apply_session(self, operations, key="key-1"):
        artifact = planned(self, operations)
        other = planned(self, [{"type": "exclude", "transaction_id": "buy", "reason": "Not mine"}])
        self.masked([
            {"artifact": artifact, "idempotency_key": key},
            {"artifact": artifact, "idempotency_key": key},
            {"artifact": other, "idempotency_key": key},
        ])
        self.compare([{"idempotency_key": key}], kind="ui.review.receipt")
        self.compare([{}], kind="ui.review.cases")

    def test_price_apply_retry_and_conflict_match_python(self):
        self.apply_session([price()])

    def test_exclusion_and_acquisition_applies_match_python(self):
        self.apply_session([{"type": "exclude", "transaction_id": "sell", "reason": "Duplicate import"},
                            {"type": "kind_override", "transaction_id": "gift", "kind": "income",
                             "reason": "Mining payout"}])

    def test_mixed_component_batch_apply_matches_python(self):
        self.apply_session(MIXED)

    def test_processed_book_apply_replaces_journals_like_python(self):
        self.python("journals", "process")
        self.apply_session(MIXED, key="processed")

    def test_encrypted_apply_matches_python(self):
        self.encrypt("correct horse battery staple")
        self.apply_session([price(), {"type": "exclude", "transaction_id": "sell", "reason": "Duplicate"}])

    def test_refused_applies_leave_every_table_unchanged(self):
        artifact = planned(self, [price()])
        baseline = self.snapshot()
        spaced = redigest(artifact, operations=[{**artifact["operations"][0], "reason": " Verified exchange rate "}])
        self.masked([
            {"artifact": {**artifact, "digest": "0" * 64}, "idempotency_key": "k"},
            {"artifact": {key: value for key, value in artifact.items() if key != "after"}, "idempotency_key": "k"},
            {"artifact": redigest(artifact, profile_id="other"), "idempotency_key": "k"},
            {"artifact": redigest(artifact, schema_version=2), "idempotency_key": "k"},
            {"artifact": spaced, "idempotency_key": "k"},
            {"artifact": redigest(artifact, base_input_version=artifact["base_input_version"] + 1),
             "idempotency_key": "k"},
            {"artifact": redigest(artifact, after={**artifact["after"], "quarantine_count": 0}),
             "idempotency_key": "k"},
            {"artifact": redigest(artifact, before={**artifact["before"], "entries_count": 99}),
             "idempotency_key": "k"},
            {"artifact": artifact, "idempotency_key": ""},
            {"artifact": artifact, "idempotency_key": "   "},
            {"artifact": artifact, "idempotency_key": "x" * 201},
            {"artifact": artifact, "idempotency_key": 7},
            {"artifact": [artifact], "idempotency_key": "k"},
            {"artifact": artifact},
        ])
        self.assertEqual(self.snapshot(), baseline)

    def test_changed_book_refuses_stale_plans_atomically(self):
        artifact = planned(self, [price()])
        self.change("UPDATE profiles SET journal_input_version = journal_input_version + 1")
        self.masked([{"artifact": artifact, "idempotency_key": "stale-version"}])
        self.change("UPDATE profiles SET journal_input_version = journal_input_version - 1;"
                    "UPDATE transactions SET fiat_rate = 41000 WHERE id = 'sell'")
        baseline = self.snapshot()
        self.masked([{"artifact": artifact, "idempotency_key": "stale-effects"}])
        self.assertEqual(self.snapshot(), baseline)

    def test_historical_key_returns_its_receipt_after_the_book_changes(self):
        artifact = planned(self, [price()])
        other = planned(self, [{"type": "exclude", "transaction_id": "sell", "reason": "x"}])
        self.masked([{"artifact": artifact, "idempotency_key": "kept"}])
        self.change("UPDATE profiles SET journal_input_version = journal_input_version + 5")
        self.masked([{"artifact": artifact, "idempotency_key": "kept"},
                     {"artifact": other, "idempotency_key": "kept"},
                     {"artifact": other, "idempotency_key": "new"}])


class ReviewPairParity(ReviewParity, unittest.TestCase):
    KIND = "ui.review.cases"
    PAIR = True

    def unpair(self, **changes):
        pair = next(case["pair"] for case in self.python_cases()["cases"] if "pair" in case)
        return {"type": "unpair", "pair_id": pair["pair_id"],
                "expected_fingerprint": pair["pair_fingerprint"], "reason": "Two separate payments", **changes}

    def test_pair_residual_cases_name_their_pair(self):
        self.compare([{}, {"limit": 1}])
        self.python("journals", "process")
        self.compare([{}])

    def test_unpair_plans_check_the_confirmed_fingerprint(self):
        version = self.profile_scope()[1]
        self.compare([
            {"operations": [self.unpair()], "expected_input_version": version},
            {"operations": [self.unpair(pair_id=" " + self.unpair()["pair_id"] + " ")],
             "expected_input_version": version},
            {"operations": [self.unpair(expected_fingerprint="stale")], "expected_input_version": version},
            {"operations": [self.unpair(pair_id="missing")], "expected_input_version": version},
            {"operations": [price(), self.unpair()], "expected_input_version": version},
            {"operations": [self.unpair(), self.unpair()], "expected_input_version": version},
        ], kind="ui.review.plan")

    def test_unpair_apply_matches_python(self):
        artifact = planned(self, [self.unpair()])
        self.masked([{"artifact": artifact, "idempotency_key": "unpair"},
                     {"artifact": artifact, "idempotency_key": "unpair"}], kind="ui.review.apply")
        self.compare([{}], kind="ui.review.cases")

    def test_unpair_after_metadata_apply_matches_python(self):
        artifact = planned(self, [price(), self.unpair()])
        self.masked([{"artifact": artifact, "idempotency_key": "batch"}], kind="ui.review.apply")


class ReviewCliParity(ReviewParity, unittest.TestCase):
    """The `review` leaves run in Rust on a copy of the book Python uses."""

    def cli(self, commands):
        """Run each argv on Python and on the candidate copy, in order."""
        candidate = self.base / "cli-candidate" / "data"
        shutil.copytree(self.root, candidate)
        masker = Masker(self.started)
        for argv in commands:
            results = []
            for side, root in ((0, candidate), (1, self.root)):
                args = ["--machine", "--data-root", str(root)]
                if self.passphrase:
                    args.extend(["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())])
                args.extend(argv)
                if side == 0:
                    result = run_candidate(args, env=self.env, input=b"")
                else:
                    result = cli_runner.run(args, reference=True, env=self.env, input=b"",
                                            capture_output=True, timeout=300)
                results.append((result.returncode, masker.text(result.stdout, side),
                                masker.text(result.stderr, side)))
            self.assertEqual(results[0], results[1], argv)
            if os.environ.get("KASSIBER_DESKTOP_COUNTS"):
                with open(os.environ["KASSIBER_DESKTOP_COUNTS"], "a", encoding="utf-8") as handle:
                    handle.write(json.dumps(dict(
                        kind=".".join(argv[:2]), served_by="rust", cli=True,
                        ok=results[0][0] == 0, error_code=None if results[0][0] == 0 else
                        json.loads(results[0][1] or "{}").get("error", {}).get("code"))) + "\n")
        self.assertEqual(masker.tables(self.snapshot(candidate), 0), masker.tables(self.snapshot(), 1))

    def write(self, name, payload):
        path = self.base / "inputs" / name
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(payload if isinstance(payload, bytes) else json.dumps(payload).encode())
        return str(path)

    def test_cases_and_receipt_commands_match_python(self):
        scope = self.profile_scope()
        self.cli([
            ["review", "cases"], ["review", "cases", "--limit", "1"],
            ["review", "cases", "--cursor", cursor(scope, "buy")], ["review", "cases", "--limit", "0"],
            ["review", "cases", "--limit", "x"], ["review", "cases", "--cursor", "bad"],
            ["review", "cases", "--workspace", "ws", "--profile", "profile"],
            ["review", "cases", "--profile", "missing"],
            ["review", "receipt", "--idempotency-key", "none"], ["review", "receipt"],
            ["review", "receipt", "--receipt-id", "a", "--idempotency-key", "b"],
        ])

    def test_plan_files_and_refusals_match_python(self):
        version = str(self.profile_scope()[1])
        operations = [price(), {"type": "exclude", "transaction_id": "sell", "reason": "Duplicate"}]
        array = self.write("array.json", operations)
        self.cli([
            ["review", "plan", "--operations-file", array, "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("object.json", {"operations": operations}),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("bom.json", b"\xef\xbb\xbf" + json.dumps(operations).encode()),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("utf16.json", json.dumps(operations).encode("utf-16")),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("other.json", {"ops": operations}),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("text.json", "text"), "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("bad.json", b"[1,"), "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("binary.json", b"\xff\xfe\x00"),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("empty.json", b""), "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("large.json", b" " * 1_000_001),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", self.write("exact.json", json.dumps(operations).encode().ljust(1_000_000)),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", str(self.base / "inputs" / "missing.json"),
             "--expected-input-version", version],
            ["review", "plan", "--operations-file", str(self.base / "inputs"), "--expected-input-version", version],
            ["review", "plan", "--operations-file", array, "--expected-input-version", "9"],
            ["review", "plan", "--operations-file", array, "--expected-input-version", "x"],
            ["review", "plan", "--operations-file", array],
            ["review", "plan", "--operations-file", array, "--expected-input-version", version, "--profile", "missing"],
        ])

    def test_apply_artifacts_envelopes_and_retries_match_python(self):
        artifact = planned(self, [price(), {"type": "exclude", "transaction_id": "sell", "reason": "Duplicate"}])
        other = planned(self, [{"type": "exclude", "transaction_id": "buy", "reason": "Not mine"}])
        raw = self.write("artifact.json", artifact)
        self.cli([
            ["review", "apply", "--artifact-file", self.write("list.json", [artifact]), "--idempotency-key", "k"],
            ["review", "apply", "--artifact-file", self.write("unhashable.json", {"kind": ["review.plan"], "data": artifact}),
             "--idempotency-key", "k"],
            ["review", "apply", "--artifact-file", self.write("empty-envelope.json", {"kind": "review.plan"}),
             "--idempotency-key", "k"],
            ["review", "apply", "--artifact-file", self.write("tampered.json", {**artifact, "digest": "0" * 64}),
             "--idempotency-key", "k"],
            ["review", "apply", "--artifact-file", raw, "--idempotency-key", " "],
            ["review", "apply", "--artifact-file", raw, "--idempotency-key", "cli-key"],
            ["review", "apply", "--artifact-file", self.write("envelope.json", {"kind": "review.plan", "data": artifact}),
             "--idempotency-key", "cli-key"],
            ["review", "apply", "--artifact-file", self.write("desktop.json", {"kind": "ui.review.plan", "data": other}),
             "--idempotency-key", "cli-key"],
            ["review", "apply", "--artifact-file", self.write("other.json", other), "--idempotency-key", "second"],
            ["review", "receipt", "--idempotency-key", "cli-key"],
            ["review", "cases"],
        ])

    def test_encrypted_cli_plan_and_apply_match_python(self):
        self.encrypt("correct horse battery staple")
        artifact = planned(self, [price()])
        self.cli([
            ["review", "plan", "--operations-file", self.write("ops.json", [price()]),
             "--expected-input-version", str(self.profile_scope()[1])],
            ["review", "apply", "--artifact-file", self.write("artifact.json", artifact), "--idempotency-key", "e"],
            ["review", "receipt", "--idempotency-key", "e"],
        ])


if __name__ == "__main__":
    unittest.main()
