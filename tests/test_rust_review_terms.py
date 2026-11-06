"""The review workflow's term operations, tax delta and review counts (main sync S4).

Pair creation, candidate dismissal and its undo, custody-gap actions and
coarse-price acceptance ride ``ui.review.plan`` and ``ui.review.apply``; plans,
applies and receipts carry per-year tax figures and their delta; cases name
the book's currency; badges add ``review`` and ``definitions``; a rebuild
without automatic pairing, and an apply that pairs or dismisses, recount the
cached transfer candidates. #684 makes ``transfers unpair`` and ``transfers
rules delete`` return what they removed. Each case runs the Python reference
and the Rust binary on copies of one book and compares the bytes and all 113
tables, plaintext and SQLCipher.
"""
from contextlib import closing
import json
import shutil
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests.rust_desktop_parity import RUST_BIN, run_candidate
from tests.rust_port.sync_parity import SyncParity
from tests.test_rust_review_workflow import NOW, ReviewParity, planned, redigest

PASSPHRASE = "sync-s4-passphrase"


def pair(out="out", inbound="in", **extra):
    return {"type": "pair", "out_transaction_id": out, "in_transaction_id": inbound,
            "reason": "Same owner", **extra}


def dismiss(out="out", inbound="in", **extra):
    return {"type": "dismiss_pair", "out_transaction_id": out, "in_transaction_id": inbound,
            "reason": "Two payments", **extra}


def undismiss(out="out", inbound="in"):
    return {"type": "undismiss_pair", "out_transaction_id": out, "in_transaction_id": inbound,
            "reason": "Suggest again"}


def accept(**extra):
    return {"type": "accept_coarse_prices", "reason": "Daily averages reviewed", **extra}


class TermParity(ReviewParity):
    """`ReviewParity` books, masked with every UUID version (pair components
    are UUIDv5 of a fresh pair id) and compared on CLI leaves too."""

    COARSE = False
    masker = SyncParity.masker
    roots = SyncParity.roots

    def setUp(self):
        super().setUp()
        if self.PASSPHRASE:
            self.encrypt(self.PASSPHRASE)

    PASSPHRASE = None

    def seed(self):
        super().seed()
        if self.COARSE:
            self.change(
                "UPDATE profiles SET require_coarse_review=1;"
                "UPDATE transactions SET fiat_rate=40000.0,fiat_rate_exact='40000',"
                "pricing_quality='coarse_fallback',pricing_source_kind='fmv_provider',"
                "pricing_granularity='daily' WHERE id IN ('buy','gift');"
            )

    def requests(self, args, kind=None):
        requests = super().requests(args, kind)
        if self.passphrase:
            # Without an unlock the daemon answers every kind auth_required.
            requests.insert(0, {
                "request_id": "unlock",
                "kind": "daemon.unlock",
                "args": {"auth_response": {"passphrase_secret": self.passphrase}},
            })
        return requests

    def compare(self, args, **options):
        responses = super().compare(args, **options)
        if self.passphrase:
            self.assertEqual(responses[0]["kind"], "daemon.unlock", responses[0])
            responses = responses[1:]
        return responses

    def masked(self, args, **options):
        # Pair components and their commitments derive from generated ids.
        masker = self.masker(hashes=True)
        return self.compare(args, mutates=True, mask=lambda line: masker.text(self.roots(line)),
                            tables=masker.tables, **options)

    def plan(self, operations):
        return {"operations": operations, "expected_input_version": self.profile_scope()[1]}

    def cli(self, commands):
        candidate = self.base / f"cli-{len(list(self.base.glob('cli-*')))}" / "data"
        shutil.copytree(self.root, candidate)
        masker = self.masker(hashes=True)
        envelopes = []
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
            envelopes.append(json.loads(results[1][1]) if results[1][1] else None)
        self.assertEqual(masker.tables(self.snapshot(candidate), 0), masker.tables(self.snapshot(), 1))
        return envelopes

    def write(self, name, payload):
        path = self.base / "inputs" / name
        path.parent.mkdir(exist_ok=True)
        path.write_text(json.dumps(payload))
        return str(path)

    def swap_count(self):
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
            return conn.execute("SELECT swap_candidate_count FROM profiles").fetchone()[0]


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class ReviewTermPlanParity(TermParity, unittest.TestCase):
    KIND = "ui.review.plan"

    def test_term_plans_and_refusals_match_python(self):
        responses = self.compare([
            self.plan([pair()]),
            self.plan([pair(pair_kind="coinjoin")]),
            self.plan([pair(policy="taxable")]),
            self.plan([pair(out_amount_msat=50_000)]),
            self.plan([pair("in", "out")]),
            self.plan([pair("missing")]),
            self.plan([pair(pair_kind="direct-swap-payout")]),
            self.plan([dismiss()]),
            self.plan([dismiss(expires_in_days=3650)]),
            self.plan([dismiss(expires_in_days=0)]),
            self.plan([dismiss(), undismiss()]),
            self.plan([undismiss()]),
            self.plan([dismiss(), pair()]),
            self.plan([pair(), dismiss("sell", "buy")]),
            self.plan([{"type": "custody_gap", "action": "dismiss", "gap_id": "missing", "reason": "r"}]),
            self.plan([{"type": "custody_gap", "action": "reopen", "gap_id": "missing", "reason": "r"}]),
            self.plan([{"type": "custody_gap", "action": "classify_residual", "gap_id": "missing",
                        "reason": "r", "classification": "fee"}]),
            self.plan([accept()]),
            self.plan([accept(transaction_ids=["buy"])]),
            self.plan([pair(extra=1)]),
        ])
        self.assertIn("delta", responses[0]["data"])
        self.assertEqual(responses[17]["error"]["code"], "review_case_changed")

    def test_processed_book_plans_carry_the_tax_delta(self):
        self.python("journals", "process")
        responses = self.compare([
            self.plan([pair()]),
            self.plan([{"type": "exclude", "transaction_id": "sell", "reason": "Duplicate"}]),
            self.plan([{"type": "price_override", "transaction_id": "buy", "reason": "r",
                        "fiat_rate": "45000"}]),
        ])
        self.assertTrue(responses[2]["data"]["delta"]["years"], responses[2])


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class ReviewTermApplyParity(TermParity, unittest.TestCase):
    KIND = "ui.review.apply"

    def apply_session(self, operations, key):
        artifact = planned(self, operations)
        self.masked([{"artifact": artifact, "idempotency_key": key},
                     {"artifact": artifact, "idempotency_key": key}])
        self.compare([{"idempotency_key": key}], kind="ui.review.receipt")
        self.compare([{}], kind="ui.review.cases")
        self.compare([{}], kind="ui.review.badges")
        return artifact

    def test_pair_apply_recounts_candidates_like_python(self):
        self.apply_session([pair()], "pair")
        self.assertIsNotNone(self.swap_count())

    def test_dismissal_and_its_undo_apply_like_python(self):
        self.apply_session([dismiss(expires_in_days=30)], "dismiss")
        self.apply_session([undismiss()], "undismiss")

    def test_tampered_delta_is_refused_like_python(self):
        artifact = planned(self, [dismiss()])
        baseline = self.snapshot()
        years = [{"year": 2024, "realized_gain_loss": "1.00", "income": "0.00"}]
        self.masked([
            {"artifact": redigest(artifact, delta={"years": years}), "idempotency_key": "k"},
            {"artifact": redigest(artifact, delta=None), "idempotency_key": "k"},
            {"artifact": redigest(artifact, before={**artifact["before"], "tax": None}),
             "idempotency_key": "k"},
            {"artifact": redigest(artifact, before={**artifact["before"], "tax": {"years": [
                {"year": 2024, "realized_gain_loss": "x", "income": "0"}]}}),
             "idempotency_key": "k"},
            {"artifact": {key: value for key, value in artifact.items() if key != "delta"},
             "idempotency_key": "k"},
        ])
        self.assertEqual(self.snapshot(), baseline)


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class ReviewCoarseParity(TermParity, unittest.TestCase):
    KIND = "ui.review.plan"
    COARSE = True

    def test_cases_offer_acceptance_and_plans_name_the_cases(self):
        cases = self.compare([{}], kind="ui.review.cases")
        self.assertEqual(cases[0]["data"]["fiat_currency"], "USD")
        responses = self.compare([
            self.plan([accept()]),
            self.plan([accept(transaction_ids=["buy"])]),
            self.plan([accept(transaction_ids=["buy", "sell"])]),
            self.plan([{"type": "exclude", "transaction_id": "buy", "reason": "x"}, accept()]),
            self.plan([accept(), accept()]),
            self.plan([{"type": "price_override", "transaction_id": "buy", "reason": "x",
                        "fiat_rate": "41000"}, accept(transaction_ids=["buy"])]),
        ])
        self.assertIn("transaction_ids", responses[0]["data"]["operations"][0])

    def test_acceptance_applies_like_python(self):
        artifact = planned(self, [accept()])
        self.masked([{"artifact": artifact, "idempotency_key": "accept"}], kind="ui.review.apply")
        self.compare([{"idempotency_key": "accept"}], kind="ui.review.receipt")
        self.compare([{}], kind="ui.review.cases")


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class ReviewCountParity(TermParity, unittest.TestCase):
    KIND = "ui.review.badges"
    PAIR = True

    def test_journal_runs_recount_and_badges_match_python(self):
        self.compare([{}, ...])
        self.cli([["journals", "process"], ["review", "cases"]])
        self.compare([{}])
        self.cli([["journals", "process", "--auto-pair"]])
        self.compare([{}])

    def test_desktop_process_takes_only_a_boolean_auto_pair(self):
        # The runs themselves are compared in test_rust_journal_desktop.py,
        # whose sessions wait for the result before shutting down.
        self.compare([{"auto_pair": "yes"}, {"auto_pair": None}, {"extra": 1, "auto_pair": True},
                      {"auto_pair": 1}, []], kind="ui.journals.process")

    def test_unpair_and_rule_delete_return_what_they_removed(self):
        pair_id = self.python("transfers", "list")[0]["id"]
        envelopes = self.cli([
            ["transfers", "rules", "create", "--name", "Cold", "--predicate", '{"out_asset": "BTC"}'],
            ["transfers", "rules", "list"],
            ["transfers", "unpair", "--pair-id", pair_id],
            ["transfers", "unpair", "--pair-id", pair_id],
        ])
        self.assertEqual(sorted(envelopes[2]["data"]), ["deleted", "pair"])
        self.assertNotIn("pair", envelopes[3]["data"])
        rule_id = self.python("transfers", "rules", "list")[0]["id"]
        envelopes = self.cli([["transfers", "rules", "delete", "--rule-id", rule_id],
                              ["transfers", "rules", "delete", "--rule-id", rule_id]])
        self.assertEqual(envelopes[0]["data"]["rule"]["name"], "Cold")
        self.assertEqual(envelopes[1]["error"]["code"], "not_found")

    def test_cli_term_plans_and_applies_match_python(self):
        version = str(self.profile_scope()[1])
        artifact = planned(self, [dismiss("sell", "buy")])
        self.cli([
            ["review", "plan", "--operations-file", self.write("ops.json", [dismiss("sell", "buy")]),
             "--expected-input-version", version],
            ["review", "apply", "--artifact-file", self.write("artifact.json", artifact),
             "--idempotency-key", "cli"],
            ["review", "receipt", "--idempotency-key", "cli"],
        ])


AUSTRIAN = (
    # id, direction, amount (msat), fiat rate, occurred
    ("alt-buy", "inbound", 50_000_000_000, 8000, "2020-01-01T10:00:00Z"),
    ("alt-sell", "outbound", 50_000_000_000, 15000, "2020-12-01T10:00:00Z"),
    ("buy", "inbound", 100_000_000_000, 30000, "2021-06-01T10:00:00Z"),
    ("early", "outbound", 20_000_000_000, 40000, "2022-02-15T10:00:00Z"),
    ("loss", "outbound", 30_000_000_000, 25000, "2022-06-01T10:00:00Z"),
    ("unpriced", "inbound", 5_000_000_000, None, "2023-01-01T10:00:00Z"),
    ("gain", "outbound", 10_000_000_000, 50000, "2023-05-01T10:00:00Z"),
    ("new-year", "outbound", 1_000_000_000, 60000, "2024-12-31T23:30:00Z"),
)


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class AustrianTaxDeltaParity(TermParity, unittest.TestCase):
    """Kennzahlen, the indicative tax withheld by report blockers and by rows
    before 1 March 2022, and Vienna tax years, before and after a review."""

    KIND = "ui.review.plan"

    def seed(self):
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main',?)", (NOW,))
            conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) "
                         "VALUES('profile','ws','Book','EUR','at',?)", (NOW,))
            conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) "
                         "VALUES('treasury','ws','profile','treasury','Treasury','asset',?)", (NOW,))
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES('wallet','ws','profile','Main','custom','{}',?)", (NOW,))
            for tx_id, direction, amount, rate, occurred in AUSTRIAN:
                conn.execute(
                    "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
                    "occurred_at,direction,asset,amount,fee,fiat_rate,fiat_currency,created_at) "
                    "VALUES(?,'ws','profile','wallet',?,?,?,?,'BTC',?,0,?,'EUR',?)",
                    (tx_id, f"fingerprint-{tx_id}", f"external-{tx_id}", occurred, direction, amount,
                     rate, occurred))
            conn.execute("INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile')")
            conn.commit()

    def price(self, rate):
        return {"type": "price_override", "transaction_id": "unpriced", "reason": "Receipt",
                "fiat_rate": rate}

    def test_austrian_figures_and_delta_match_python(self):
        responses = self.compare([self.plan([self.price("20000")]), self.plan([self.price("0.01")])])
        tax = responses[0]["data"]["after"]["tax"]
        self.assertEqual(tax["jurisdiction"], "AT")
        self.assertIn("kennzahlen", tax["years"][0])
        self.assertTrue(responses[0]["data"]["delta"]["years"])
        self.python("journals", "process")
        self.compare([self.plan([self.price("20000")])])
        artifact = planned(self, [self.price("20000")])
        self.masked([{"artifact": artifact, "idempotency_key": "at"}], kind="ui.review.apply")
        self.compare([{"idempotency_key": "at"}], kind="ui.review.receipt")

    def test_generic_vienna_and_currency_variants_match_python(self):
        self.change("UPDATE profiles SET fiat_currency=' eur '")
        self.compare([self.plan([self.price("20000")])])
        self.change("UPDATE profiles SET fiat_currency='USD'")
        self.compare([self.plan([self.price("20000")])])
        self.change("UPDATE profiles SET tax_country='generic', fiat_currency='EUR'")
        self.compare([self.plan([self.price("20000")])])


class EncryptedAustrianTaxDeltaParity(AustrianTaxDeltaParity):
    PASSPHRASE = PASSPHRASE


class EncryptedReviewTermPlanParity(ReviewTermPlanParity):
    PASSPHRASE = PASSPHRASE


class EncryptedReviewTermApplyParity(ReviewTermApplyParity):
    PASSPHRASE = PASSPHRASE


class EncryptedReviewCoarseParity(ReviewCoarseParity):
    PASSPHRASE = PASSPHRASE


class EncryptedReviewCountParity(ReviewCountParity):
    PASSPHRASE = PASSPHRASE


if __name__ == "__main__":
    unittest.main()
