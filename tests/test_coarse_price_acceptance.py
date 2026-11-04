"""Accepting coarse prices through review plan/apply, bound to the price accepted."""
import tempfile
import unittest

from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import metadata, pricing, review_workflow as review
from kassiber.core.tax_events import _pricing_needs_review
from kassiber.db import open_db
from kassiber.errors import AppError


class CoarsePriceAcceptanceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.conn = open_db(self.tmp.name)
        self.addCleanup(self.conn.close)
        conn = self.conn
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020')")
        conn.execute(
            "INSERT INTO profiles(id,workspace_id,label,require_coarse_review,created_at) "
            "VALUES('p','w','P',1,'2020')"
        )
        conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) "
            "VALUES('wallet','w','p','Wallet','manual','2020')"
        )
        # Two receipts priced from a daily average, one from an exchange fill.
        for tx, quality, rate in (
            ("a", pricing.QUALITY_COARSE_FALLBACK, "3197.6"),
            ("b", pricing.QUALITY_COARSE_FALLBACK, "3149.2"),
            ("c", pricing.QUALITY_EXACT, "3200"),
        ):
            conn.execute(
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,"
                "direction,asset,amount,fee,kind,raw_json,fiat_currency,fiat_rate,fiat_rate_exact,"
                "pricing_source_kind,pricing_quality,created_at) "
                "VALUES(?,'w','p','wallet',?,'2019-01-15T00:00:00Z','inbound','BTC',100000000000,0,"
                "'buy','{}','EUR',?,?,?,?,'2020')",
                (tx, tx, float(rate), rate,
                 pricing.SOURCE_FMV_PROVIDER if quality != pricing.QUALITY_EXACT
                 else pricing.SOURCE_EXCHANGE_EXECUTION, quality),
            )
        conn.commit()
        self.hooks = review.ReviewHooks(metadata=_metadata_hooks())

    def profile(self):
        return self.conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()

    def version(self):
        return int(self.profile()["journal_input_version"] or 0)

    def plan(self, operation):
        return review.plan_review(
            self.conn, self.profile(), operations=[operation],
            expected_input_version=self.version(), hooks=self.hooks,
        )

    def held(self):
        state = review._build(self.conn, self.profile())
        return sorted(
            (str(q["transaction_id"]), str(q["reason"])) for q in state["quarantines"]
        )

    def test_cases_offer_the_acceptance_for_coarse_prices_only(self):
        cases = review.inspect_cases(self.conn, self.profile(), limit=10)["cases"]
        self.assertEqual(
            {case["transaction_id"]: case["supported_operations"] for case in cases},
            {
                "a": ["accept_coarse_prices", "price_override"],
                "b": ["accept_coarse_prices", "price_override"],
            },
        )

    def test_a_plan_names_every_case_and_apply_records_each_acceptance(self):
        artifact = self.plan({"type": "accept_coarse_prices", "reason": "Daily averages reviewed"})
        self.assertEqual(artifact["operations"][0]["transaction_ids"], ["a", "b"])
        self.assertEqual(artifact["before"]["quarantine_count"], 2)
        self.assertEqual(artifact["after"]["quarantine_count"], 0)

        receipt = review.apply_review(
            self.conn, self.profile(), artifact=artifact, idempotency_key="accept-1",
            hooks=self.hooks,
        )
        self.assertEqual(receipt["transaction_ids"], ["a", "b"])
        self.assertEqual(
            receipt["proposed_operations"],
            [{"type": "accept_coarse_prices", "reason": "Daily averages reviewed",
              "transaction_count": 2}],
        )
        rows = self.conn.execute(
            "SELECT id, pricing_quality, pricing_source_kind, pricing_review_accepted_price "
            "FROM transactions ORDER BY id"
        ).fetchall()
        # The price keeps its coarse provenance; only the review is recorded.
        self.assertEqual(
            [tuple(row) for row in rows],
            [
                ("a", "coarse_fallback", "fmv_provider", "rate:3197.6"),
                ("b", "coarse_fallback", "fmv_provider", "rate:3149.2"),
                ("c", "exact", "exchange_execution", None),
            ],
        )
        self.assertEqual(self.held(), [])
        fields = self.conn.execute(
            "SELECT field FROM transaction_edit_fields WHERE field = 'pricing_review_accepted'"
        ).fetchall()
        self.assertEqual(len(fields), 2)
        history = metadata.list_transaction_history(
            self.conn, "w", "p", "a", _metadata_hooks(), limit=1,
        )
        field = history["events"][0]["fields"][0]
        self.assertEqual(
            (field["label"], field["after_label"]),
            ("Approximate price accepted", "Accepted price per unit 3197.6"),
        )

    def test_a_price_stored_only_as_a_value_is_accepted_through_the_projection(self):
        # The custody projection re-slices a value-only price into a rate; the
        # acceptance still follows the transaction's own price.
        self.conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,kind,raw_json,fiat_currency,fiat_value,fiat_value_exact,"
            "pricing_source_kind,pricing_quality,created_at) "
            "VALUES('v','w','p','wallet','v','2019-01-16T00:00:00Z','inbound','BTC',50000000000,0,"
            "'buy','{}','EUR',1600,'1600',?,?,'2020')",
            (pricing.SOURCE_FMV_PROVIDER, pricing.QUALITY_COARSE_FALLBACK),
        )
        self.conn.commit()
        artifact = self.plan({"type": "accept_coarse_prices", "reason": "Daily averages reviewed"})
        self.assertEqual(artifact["operations"][0]["transaction_ids"], ["a", "b", "v"])
        self.assertEqual(artifact["after"]["quarantine_count"], 0)
        review.apply_review(
            self.conn, self.profile(), artifact=artifact, idempotency_key="accept-v",
            hooks=self.hooks,
        )
        self.assertEqual(self.held(), [])

    def test_only_legs_with_their_own_coarse_price_are_accepted(self):
        # A transfer's other leg is held for the same reason without a coarse
        # price of its own; it is left out and clears with the priced leg.
        state = {"quarantines": [
            {"transaction_id": "a", "reason": "pricing_review_required"},
            {"transaction_id": "c", "reason": "pricing_review_required"},
        ]}
        self.assertEqual(review._coarse_price_cases(self.conn, self.profile(), state), ["a"])

    def test_an_entered_price_is_stored_in_the_book_currency(self):
        # The row came in labelled USD; the book, and its journals, read EUR.
        self.conn.execute("UPDATE profiles SET fiat_currency = 'EUR' WHERE id = 'p'")
        self.conn.execute("UPDATE transactions SET fiat_currency = 'USD' WHERE id = 'a'")
        self.conn.commit()
        artifact = self.plan({"type": "price_override", "transaction_id": "a",
                              "reason": "Receipt", "fiat_value": "3000.50"})
        review.apply_review(
            self.conn, self.profile(), artifact=artifact, idempotency_key="price-a",
            hooks=self.hooks,
        )
        row = self.conn.execute(
            "SELECT fiat_currency, fiat_value_exact, pricing_quality FROM transactions WHERE id = 'a'"
        ).fetchone()
        self.assertEqual(tuple(row), ("EUR", "3000.50", "exact"))
        self.assertEqual(self.held(), [("b", "pricing_review_required")])

    def test_a_changed_price_is_reviewed_again(self):
        artifact = self.plan({"type": "accept_coarse_prices", "reason": "Daily averages reviewed"})
        review.apply_review(
            self.conn, self.profile(), artifact=artifact, idempotency_key="accept-1",
            hooks=self.hooks,
        )
        self.conn.execute(
            "UPDATE transactions SET fiat_rate = 3300, fiat_rate_exact = '3300' WHERE id = 'a'"
        )
        self.conn.commit()
        self.assertEqual(self.held(), [("a", "pricing_review_required")])

    def test_reverting_the_acceptance_holds_the_transaction_again(self):
        metadata.update_transaction_metadata(
            self.conn, "w", "p", "a", _metadata_hooks(), accept_coarse_price=True,
            reason="Daily average reviewed",
        )
        self.assertEqual(self.held(), [("b", "pricing_review_required")])
        event = self.conn.execute(
            "SELECT id FROM transaction_edit_events WHERE transaction_id = 'a'"
        ).fetchone()
        metadata.revert_transaction_edit(
            self.conn, "w", "p", "a", _metadata_hooks(), event_id=event["id"],
        )
        self.assertEqual(
            self.held(), [("a", "pricing_review_required"), ("b", "pricing_review_required")]
        )

    def test_naming_a_transaction_that_is_not_a_coarse_case_is_refused(self):
        with self.assertRaises(AppError) as error:
            self.plan({"type": "accept_coarse_prices", "reason": "Reviewed",
                       "transaction_ids": ["a", "c"]})
        self.assertEqual(error.exception.code, "review_case_changed")
        with self.assertRaises(AppError) as error:
            metadata.update_transaction_metadata(
                self.conn, "w", "p", "c", _metadata_hooks(), accept_coarse_price=True,
            )
        self.assertEqual(error.exception.code, "validation")

    def test_an_operation_without_reason_or_with_repeated_ids_is_invalid(self):
        for operation in (
            {"type": "accept_coarse_prices"},
            {"type": "accept_coarse_prices", "reason": "x", "transaction_ids": ["a", "a"]},
            {"type": "accept_coarse_prices", "reason": "x", "transaction_ids": []},
            {"type": "accept_coarse_prices", "reason": "x", "fiat_rate": "1"},
        ):
            with self.subTest(operation=operation), self.assertRaises(AppError):
                self.plan(operation)

    def test_needs_review_follows_the_price_the_journals_read(self):
        row = {"pricing_quality": "coarse_fallback", "fiat_rate_exact": "100",
               "fiat_value_exact": "5"}
        self.assertTrue(_pricing_needs_review(row))
        self.assertFalse(_pricing_needs_review({**row, "pricing_review_accepted_price": "rate:100"}))
        self.assertTrue(_pricing_needs_review({**row, "pricing_review_accepted_price": "rate:99"}))
        # Without a rate the journals price by value, and so does the review.
        no_rate = {"pricing_quality": "coarse_fallback", "fiat_value_exact": "5"}
        self.assertFalse(_pricing_needs_review({**no_rate, "pricing_review_accepted_price": "value:5"}))
        self.assertFalse(_pricing_needs_review({"pricing_quality": "exact"}))


class ReviewedPriceComparisonTest(unittest.TestCase):
    def test_trailing_zeros_name_the_same_price(self):
        self.assertTrue(pricing.same_reviewed_price("rate:3197.6", "rate:3197.60"))
        self.assertTrue(pricing.same_reviewed_price("value:3200", "value:3200.00"))

    def test_a_different_price_or_field_is_reviewed_again(self):
        self.assertFalse(pricing.same_reviewed_price("rate:3197.6", "rate:3197.61"))
        self.assertFalse(pricing.same_reviewed_price("rate:3197.6", "value:3197.6"))
        self.assertFalse(pricing.same_reviewed_price("rate:x", "rate:3197.6"))
        self.assertFalse(pricing.same_reviewed_price(None, "rate:1"))
        self.assertFalse(pricing.same_reviewed_price("rate:1", None))


if __name__ == "__main__":
    unittest.main()
