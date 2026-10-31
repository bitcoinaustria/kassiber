"""Every journal-meaning write rides one previewed, consented, atomic review batch.

Each operation type is planned on an in-memory copy (the book stays unchanged),
applied atomically with the rest of its batch, and refused once the book's
input version moved. The artifact also carries the per-year tax result before
and after, and their difference, so "what does this do to my taxes?" has an
answer before anything is written.
"""
from __future__ import annotations

import copy
import tempfile
import unittest

from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import custody_components, custody_gaps, review_workflow as review
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError

BTC = 100_000_000_000


class _Book(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.conn = open_db(self.tmp.name)
        self.addCleanup(self.conn.close)
        self.conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020')")
        self.conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','P','2020')")
        set_setting(self.conn, "context_workspace", "w")
        set_setting(self.conn, "context_profile", "p")
        self.hooks = review.ReviewHooks(metadata=_metadata_hooks())
        self.seed()
        self.conn.commit()

    def seed(self):
        raise NotImplementedError

    def wallet(self, wallet_id, kind="manual", config="{}"):
        self.conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
            "VALUES(?,'w','p',?,?,?,'2020')", (wallet_id, wallet_id.title(), kind, config),
        )

    def tx(self, tx_id, wallet_id, direction, amount, occurred_at, *, kind=None, rate=None,
           fee=0, privacy_boundary=None):
        value = None if rate is None else rate * amount / BTC
        self.conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,"
            "occurred_at,direction,asset,amount,fee,kind,fiat_rate,fiat_rate_exact,fiat_value,"
            "privacy_boundary,raw_json,created_at) "
            "VALUES(?,'w','p',?,?,?,?,?,'BTC',?,?,?,?,?,?,?,'{}','2020')",
            (tx_id, wallet_id, tx_id, "fp-" + tx_id, occurred_at, direction, amount, fee, kind,
             rate, None if rate is None else str(rate), value, privacy_boundary),
        )

    @property
    def profile(self):
        return self.conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()

    def version(self):
        return int(self.profile["journal_input_version"] or 0)

    def plan(self, operations):
        return review.plan_review(self.conn, self.profile, operations=operations,
                                  expected_input_version=self.version(), hooks=self.hooks)

    def apply(self, artifact, key="batch"):
        return review.apply_review(self.conn, self.profile, artifact=artifact,
                                   idempotency_key=key, hooks=self.hooks)

    def assert_plan_leaves_book_unchanged(self, operations):
        changes = self.conn.total_changes
        artifact = self.plan(operations)
        self.assertEqual(self.conn.total_changes, changes)
        self.assertEqual(review.validate_review(self.conn, self.profile, artifact=artifact,
                                                hooks=self.hooks), artifact)
        return artifact

    def assert_stale_version_fails_closed(self, artifact):
        self.conn.execute("UPDATE profiles SET journal_input_version = journal_input_version + 1")
        self.conn.commit()
        with self.assertRaises(AppError) as error:
            self.apply(artifact, key="stale")
        self.assertEqual(error.exception.code, "custody_review_plan_stale")
        self.conn.execute("UPDATE profiles SET journal_input_version = journal_input_version - 1")
        self.conn.commit()


class AustrianTaxDeltaTest(_Book):
    """A Neubestand gain whose price is missing until the batch supplies it."""

    def seed(self):
        self.conn.execute("UPDATE profiles SET tax_country='at', fiat_currency='EUR'")
        self.wallet("wallet")
        self.tx("buy", "wallet", "inbound", BTC, "2024-01-10T10:00:00Z", kind="buy", rate=40000)
        self.tx("early", "wallet", "outbound", BTC // 10, "2024-03-10T10:00:00Z", kind="sell", rate=50000)
        self.tx("sell", "wallet", "outbound", BTC // 2, "2024-06-10T10:00:00Z", kind="sell")

    def price(self, rate="60000"):
        return {"type": "price_override", "transaction_id": "sell", "fiat_rate": rate,
                "reason": "Exchange statement reviewed"}

    def year(self, effects, year=2024):
        return next(item for item in effects["tax"]["years"] if item["year"] == year)

    def test_price_makes_the_gain_reportable_and_the_tax_indicative(self):
        artifact = self.assert_plan_leaves_book_unchanged([self.price()])
        self.assertEqual(artifact["before"]["tax"]["jurisdiction"], "AT")
        before, after = self.year(artifact["before"]), self.year(artifact["after"])
        # A quarantined book withholds the figure instead of estimating it.
        self.assertEqual(before["indicative_tax"]["status"], "withheld")
        self.assertIsNone(before["indicative_tax"]["estimated_tax_eur_cents"])
        self.assertIn("journal_quarantine", before["indicative_tax"]["withheld_by"])
        self.assertEqual(before["realized_gain_loss"], "1000.00")
        self.assertEqual(before["kennzahlen"], {"174": 100_000})
        self.assertEqual(after["realized_gain_loss"], "11000.00")
        self.assertEqual(after["kennzahlen"], {"174": 1_100_000})
        self.assertEqual(after["indicative_tax"], {
            "status": "indicative", "special_rate_base_eur_cents": 1_100_000,
            "estimated_tax_eur_cents": 302_500, "withheld_by": [],
        })
        self.assertEqual(artifact["delta"], {"years": [{
            "year": 2024, "realized_gain_loss": "10000.00", "income": "0.00",
            "kennzahlen": {"174": 1_000_000}, "estimated_tax_eur_cents": None,
            "indicative_status": "indicative",
        }]})
        receipt = self.apply(artifact)
        self.assertEqual(receipt["delta"], artifact["delta"])
        self.assertEqual(receipt["verification"]["tax"], artifact["after"]["tax"])

    def test_reclassifying_an_acquisition_reports_the_income_difference(self):
        self.apply(self.plan([self.price()]), key="first")
        artifact = self.plan([{"type": "kind_override", "transaction_id": "buy", "kind": "income",
                               "reason": "Mining payout per pool statement"}])
        delta = artifact["delta"]["years"]
        self.assertEqual(len(delta), 1)
        self.assertEqual(delta[0]["income"], "40000.00")
        self.assertEqual(delta[0]["realized_gain_loss"], "0.00")
        # Both sides are reportable, so the indicative tax difference is a number.
        before = self.year(artifact["before"])["indicative_tax"]
        after = self.year(artifact["after"])["indicative_tax"]
        self.assertEqual((before["status"], after["status"]), ("indicative", "indicative"))
        self.assertEqual(delta[0]["estimated_tax_eur_cents"],
                         after["estimated_tax_eur_cents"] - before["estimated_tax_eur_cents"])
        self.assertEqual(delta[0]["kennzahlen"], {"172": 4_000_000})
        self.assertEqual(delta[0]["estimated_tax_eur_cents"], 1_100_000)

    def test_tampered_delta_is_refused(self):
        artifact = self.plan([self.price()])
        tampered = copy.deepcopy(artifact)
        tampered["delta"] = {"years": []}
        from kassiber.core.review_workflow import _digest
        tampered["digest"] = _digest({k: v for k, v in tampered.items() if k != "digest"})
        with self.assertRaises(AppError) as error:
            self.apply(tampered)
        self.assertEqual(error.exception.code, "review_artifact_invalid")

    def test_another_book_withholds_rather_than_borrowing_its_blockers(self):
        set_setting(self.conn, "context_profile", "elsewhere")
        self.conn.commit()
        artifact = self.plan([self.price()])
        self.assertEqual(self.year(artifact["after"])["indicative_tax"]["withheld_by"],
                         ["report_scope_not_active"])

    def test_years_are_bounded_to_the_latest(self):
        for year in range(2009, 2009 + review.MAX_TAX_YEARS + 2):
            self.tx(f"gift-{year}", "wallet", "inbound", BTC // 100, f"{year}-03-01T00:00:00Z",
                    kind="income", rate=1000)
        self.conn.commit()
        artifact = self.plan([self.price()])
        years = [item["year"] for item in artifact["after"]["tax"]["years"]]
        self.assertEqual(len(years), review.MAX_TAX_YEARS)
        self.assertEqual(years[-1], 2024)
        self.assertGreater(artifact["after"]["tax"]["years_omitted"], 0)


class PairBatchTest(_Book):
    def seed(self):
        self.wallet("hot")
        self.wallet("cold")
        self.tx("fund", "hot", "inbound", BTC, "2021-01-01T00:00:00Z", kind="buy", rate=30000)
        self.tx("out", "hot", "outbound", BTC // 2, "2022-01-01T00:00:00Z", rate=40000)
        self.tx("in", "cold", "inbound", BTC // 2, "2022-01-01T01:00:00Z", rate=40000)

    def pair(self, **extra):
        return {"type": "pair", "out_transaction_id": "out", "in_transaction_id": "in",
                "reason": "User confirmed the cold-storage move", **extra}

    def pairs(self):
        return self.conn.execute("SELECT count(*) FROM transaction_pairs").fetchone()[0]

    def test_pair_plans_on_a_copy_then_applies_and_rejects_stale_versions(self):
        before_pairs = self.pairs()
        artifact = self.assert_plan_leaves_book_unchanged([self.pair(policy="carrying-value")])
        self.assertEqual(self.pairs(), before_pairs)
        before = artifact["before"]["tax"]["years"]
        after = artifact["after"]["tax"]["years"]
        # Pairing the move removes the disposal it was booked as.
        self.assertNotEqual(before, after)
        self.assert_stale_version_fails_closed(artifact)
        receipt = self.apply(artifact)
        self.assertEqual(receipt["status"], "verified")
        self.assertEqual(receipt["transaction_ids"], ["in", "out"])
        self.assertEqual(receipt["operations"][0]["result"]["policy"], "carrying-value")

    def test_batch_is_atomic_when_a_later_operation_fails(self):
        artifact = self.plan([self.pair()])
        bad = copy.deepcopy(artifact)
        # Swap in an artifact whose second write cannot be applied.
        operations = [self.pair(), {"type": "dismiss_pair", "out_transaction_id": "out",
                                    "in_transaction_id": "missing", "reason": "Not one movement"}]
        with self.assertRaises(AppError) as error:
            self.plan(operations)
        self.assertEqual(error.exception.code, "not_found")
        bad["operations"] = operations
        bad["digest"] = review._digest({k: v for k, v in bad.items() if k != "digest"})
        with self.assertRaises(AppError):
            self.apply(bad)
        self.assertEqual(self.pairs(), 0)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM review_workflow_receipts").fetchone()[0], 0)

    def test_dismissal_is_bounded_reviewed_and_atomic(self):
        operation = {"type": "dismiss_pair", "out_transaction_id": "out", "in_transaction_id": "in",
                     "reason": "User says these are unrelated", "expires_in_days": 30}
        artifact = self.assert_plan_leaves_book_unchanged([operation])
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transaction_pair_dismissals").fetchone()[0], 0)
        self.assert_stale_version_fails_closed(artifact)
        receipt = self.apply(artifact)
        row = self.conn.execute("SELECT reason, expires_at FROM transaction_pair_dismissals").fetchone()
        self.assertEqual(row["reason"], "User says these are unrelated")
        self.assertIsNotNone(row["expires_at"])
        self.assertEqual(receipt["operations"][0]["result"]["transaction_ids"], ["out", "in"])
        for days in (0, 3651, True):
            with self.subTest(days=days), self.assertRaises(AppError):
                self.plan([{**operation, "expires_in_days": days}])

    def test_undismissal_takes_a_dismissal_back_in_the_same_batch_path(self):
        dismiss = {"type": "dismiss_pair", "out_transaction_id": "out", "in_transaction_id": "in",
                   "reason": "User says these are unrelated", "expires_in_days": 30}
        self.apply(self.plan([dismiss]))
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transaction_pair_dismissals").fetchone()[0], 1)
        undo = {"type": "undismiss_pair", "out_transaction_id": "out", "in_transaction_id": "in",
                "reason": "User wants the pair suggested again"}
        artifact = self.assert_plan_leaves_book_unchanged([undo])
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transaction_pair_dismissals").fetchone()[0], 1)
        receipt = self.apply(artifact, key="undo")
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transaction_pair_dismissals").fetchone()[0], 0)
        self.assertEqual(receipt["operations"][0]["result"]["transaction_ids"], ["out", "in"])
        # Nothing left to undo: the batch fails closed.
        with self.assertRaises(AppError) as error:
            self.plan([undo])
        self.assertEqual(error.exception.code, "not_found")

    def test_pair_fields_are_validated_before_planning(self):
        for operation in (
            self.pair(pair_kind="unknown"),
            self.pair(policy="maybe"),
            self.pair(out_amount_msat=0),
            self.pair(in_transaction_id="out"),
            {**self.pair(), "kind": "manual"},
            {"type": "pair", "out_transaction_id": "out", "in_transaction_id": "in"},
        ):
            with self.subTest(operation=operation), self.assertRaises(AppError):
                self.plan([operation])
        with self.assertRaises(AppError):
            self.plan([self.pair(out_transaction_id="in", in_transaction_id="out")])


class ComponentBatchTest(_Book):
    def seed(self):
        self.wallet("hot")
        self.wallet("cold")
        self.tx("fund", "hot", "inbound", BTC, "2021-01-01T00:00:00Z", kind="buy", rate=30000)
        self.tx("out", "hot", "outbound", BTC // 2, "2022-01-01T00:00:00Z", rate=40000,
                privacy_boundary="coinjoin")
        self.tx("in", "cold", "inbound", BTC // 2, "2022-01-02T00:00:00Z", rate=40000,
                privacy_boundary="coinjoin")

    def create(self, note="first reading"):
        return {"type": "custody_component", "request": {"action": "create", "activate": True, "components": [{
            "component_type": "manual_bridge", "evidence_kind": "manual_claim",
            "evidence_grade": "reviewed", "change_reason": note, "notes": note,
            "legs": [{"role": "source", "transaction": "out", "amount_msat": BTC // 2},
                     {"role": "destination", "transaction": "in", "amount_msat": BTC // 2}],
        }]}}

    def active(self):
        return [row["id"] for row in self.conn.execute(
            "SELECT id FROM custody_components WHERE state='active' ORDER BY id")]

    def test_supersede_and_recreate_on_the_same_anchors_in_one_batch(self):
        self.apply(self.plan([self.create()]), key="first")
        [old] = self.active()
        operations = [
            {"type": "custody_component", "request": {
                "action": "supersede", "component_id": old, "reason": "The first reading was wrong"}},
            self.create("corrected reading"),
        ]
        artifact = self.assert_plan_leaves_book_unchanged(operations)
        self.assertEqual(self.active(), [old])
        self.assert_stale_version_fails_closed(artifact)
        receipt = self.apply(artifact, key="second")
        self.assertEqual(receipt["status"], "verified")
        [new] = self.active()
        self.assertNotEqual(new, old)
        self.assertEqual(custody_components.get_component(self.conn, old)["state"], "superseded")
        self.assertEqual(custody_components.get_component(self.conn, new)["notes"], "corrected reading")

    def test_revise_and_undo_ride_the_batch(self):
        self.apply(self.plan([self.create()]), key="first")
        [old] = self.active()
        revise = {"type": "custody_component", "request": {
            "action": "revise", "component_id": old, "activate": True,
            "spec": {"notes": "revised", "change_reason": "Owner corrected the note"}}}
        artifact = self.assert_plan_leaves_book_unchanged([revise])
        self.apply(artifact, key="revise")
        [new] = self.active()
        self.assertEqual(custody_components.get_component(self.conn, new)["notes"], "revised")
        undo = {"type": "custody_component", "request": {"action": "undo", "component_id": old,
                                                         "reason": "Back to the first reading"}}
        self.apply(self.assert_plan_leaves_book_unchanged([undo]), key="undo")
        # Undo restores the superseded reading as a draft for the owner to activate.
        drafts = [row["notes"] for row in self.conn.execute(
            "SELECT notes FROM custody_components WHERE state='draft'")]
        self.assertEqual(drafts, ["first reading"])


class CustodyGapBatchTest(_Book):
    def seed(self):
        for wallet_id in ("a", "c"):
            self.wallet(wallet_id, kind="descriptor", config='{"chain":"bitcoin","network":"main"}')
        self.tx("out", "a", "outbound", 10 * BTC, "2020-01-01T00:00:00Z", fee=10_000_000,
                privacy_boundary="coinjoin")
        self.tx("return", "c", "inbound", 99 * BTC // 10, "2021-01-01T00:00:00Z")

    def gap_id(self):
        result, _ = custody_gaps.load_gap_search_result(self.conn, "p")
        [candidate] = list(result.candidates)
        return candidate.gap_id

    def test_bridge_and_residual_classification_apply_together(self):
        gap_id = self.gap_id()
        operations = [
            {"type": "custody_gap", "action": "create", "gap_id": gap_id, "reason": "Owner confirmed the bridge"},
            {"type": "custody_gap", "action": "classify_residual", "gap_id": gap_id,
             "classification": "external_payment", "reason": "Paid a merchant from the mix"},
        ]
        artifact = self.assert_plan_leaves_book_unchanged(operations)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM custody_components").fetchone()[0], 0)
        self.assert_stale_version_fails_closed(artifact)
        receipt = self.apply(artifact)
        results = [item["result"] for item in receipt["operations"]]
        self.assertEqual([item["status"] for item in results], ["resolved", "resolved"])
        self.assertEqual(results[1]["classification"], "external_payment")
        self.assertEqual(self.conn.execute(
            "SELECT count(*) FROM custody_components WHERE state='active'").fetchone()[0], 1)

    def test_gap_fields_are_validated(self):
        gap_id = self.gap_id()
        for operation in (
            {"type": "custody_gap", "action": "activate", "gap_id": gap_id, "reason": "r"},
            {"type": "custody_gap", "action": "create", "gap_id": gap_id, "reason": "r",
             "classification": "external_payment"},
            {"type": "custody_gap", "action": "classify_residual", "gap_id": gap_id, "reason": "r"},
            {"type": "custody_gap", "action": "dismiss", "gap_id": gap_id, "reason": "r" * 501},
        ):
            with self.subTest(operation=operation), self.assertRaises(AppError):
                self.plan([operation])


if __name__ == "__main__":
    unittest.main()
