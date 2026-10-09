"""The quarantine snapshot explains cause, root, blocking state and next step."""

import json
import tempfile
import unittest
from unittest import mock

from kassiber.ai.tools import get_tool
from kassiber.cli import handlers
from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import custody_component_planner, quarantine_review, review_workflow
from kassiber.core.custody_components import activate_component, create_component, get_component
from kassiber.core.ui_snapshot import (
    build_journals_quarantine_snapshot,
    build_review_badges_snapshot,
)
from kassiber.db import set_setting
from kassiber.errors import AppError
from kassiber.mcp.tools import run_tool
from tests.test_custody_quantity_handler import (
    BTC,
    SOURCE_AT,
    _book,
    _leg,
    _seed_transaction,
)


def _activate_residual_component(conn):
    component = create_component(
        conn,
        workspace_id="ws",
        profile_id="profile",
        component_type="manual_bridge",
        evidence_kind="manual_reconstruction",
        evidence_grade="reviewed",
        legs=[
            {
                **_leg("source", 10 * BTC, transaction_id="out", wallet_id="a", occurred_at=SOURCE_AT),
                "id": "source",
            },
            {
                **_leg(
                    "destination", 990_000_000_000,
                    transaction_id="in", wallet_id="c",
                    occurred_at="2025-01-01T00:00:00Z",
                ),
                "id": "destination",
            },
            {**_leg("suspense", 10_000_000_000, occurred_at=SOURCE_AT), "id": "suspense"},
        ],
        allocations=[
            {
                "source_leg_id": "source",
                "sink_leg_id": "destination",
                "source_amount_msat": 990_000_000_000,
                "sink_amount_msat": 990_000_000_000,
            },
            {
                "source_leg_id": "source",
                "sink_leg_id": "suspense",
                "source_amount_msat": 10_000_000_000,
                "sink_amount_msat": 10_000_000_000,
            },
        ],
    )
    activate_component(conn, component["id"])


def _fingerprint(conn, pair_id):
    return quarantine_review.pair_fingerprint(conn, quarantine_review.pairs_by_id(conn, "profile")[pair_id])


def _unpair(pair_id, fingerprint, reason="The owner chose to unpair this pair"):
    return {"type": "unpair", "pair_id": pair_id, "expected_fingerprint": fingerprint, "reason": reason}


def _unrelated_pair(conn):
    """A pair of two transactions with different txids that leaves a suspense."""
    conn.execute("UPDATE transactions SET external_id = ? WHERE id = 'out'", ("a" * 64,))
    conn.execute("UPDATE transactions SET external_id = ? WHERE id = 'in'", ("b" * 64,))
    conn.commit()
    pair = handlers.create_transaction_pair(conn, "Books", "Book", "out", "in")
    handlers.process_journals(conn, "Books", "Book")
    return pair


class QuarantineReviewTest(unittest.TestCase):
    def _open(self, root):
        conn = _book(root, "FIFO")
        self.addCleanup(conn.close)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", "profile")
        conn.commit()
        return conn

    def test_downstream_rows_group_under_the_custody_problem_that_blocks_them(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")

            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 50})

        items = {item["transaction_id"]: item for item in snapshot["items"]}
        root_item = items["out"]
        self.assertEqual(root_item["reason"], "custody_quantity_unresolved")
        self.assertFalse(root_item["is_downstream"])
        self.assertTrue(root_item["blocks_reports"])
        self.assertEqual(root_item["reasons"], ["custody_quantity_unresolved", "custody_basis_barrier"])
        self.assertEqual(root_item["evidence"]["blocker_code"], "reviewed_residual_suspense")

        later = items["later-sale"]
        self.assertEqual(later["reason"], "custody_basis_barrier")
        self.assertEqual(later["category"], "downstream")
        self.assertTrue(later["is_downstream"])
        self.assertFalse(later["blocks_reports"])
        self.assertEqual(later["root"]["transaction_id"], "out")
        self.assertEqual(later["root"]["reason"], "custody_quantity_unresolved")
        self.assertEqual(later["actions"], [{"kind": "resolve_root", "transaction_id": "out"}])

        # Roots come first; the queue reads from the fix to its consequences.
        self.assertEqual(snapshot["items"][0]["transaction_id"], "out")
        summary = snapshot["summary"]
        self.assertTrue(summary["reports_blocked"])
        self.assertEqual(summary["blocking_count"], 1)
        self.assertFalse(summary["freshness"]["needs_processing"])
        self.assertIsNone(summary["freshness"]["last_error"])
        group = summary["groups"][0]
        self.assertEqual(group["root_transaction_id"], "out")
        self.assertEqual(group["count"], 2)
        self.assertEqual(group["downstream_count"], 1)
        self.assertTrue(group["blocks_reports"])
        self.assertEqual(summary["group_count"], 1)

    def test_missing_acquisition_history_names_amounts_and_wallet(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            conn.execute("DELETE FROM transactions WHERE id IN ('out', 'in', 'later-sale')")
            _seed_transaction(
                conn, "oversell", "a", "outbound", 12 * BTC, "2024-06-01T00:00:00Z", 30_000
            )
            handlers.process_journals(conn, "Books", "Book")

            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 50})

        item = next(item for item in snapshot["items"] if item["transaction_id"] == "oversell")
        self.assertEqual(item["reason"], "insufficient_lots")
        self.assertEqual(item["category"], "missing_acquisition_history")
        self.assertEqual(item["evidence"]["wallet_label"], "A")
        self.assertEqual(item["evidence"]["required_msat"], 12 * BTC)
        self.assertEqual(item["evidence"]["available_msat"], 10 * BTC)
        self.assertEqual(
            [action["kind"] for action in item["actions"]],
            ["import_history", "connect_wallet"],
        )
        self.assertEqual(item["actions"][0]["wallet_label"], "A")

    def test_presumed_disposals_and_unclassified_receipts_are_listed_as_assumptions(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            conn.execute("DELETE FROM transactions WHERE id IN ('out', 'in', 'later-sale')")
            conn.execute("UPDATE transactions SET kind = 'buy' WHERE id = 'buy-old'")
            _seed_transaction(
                conn, "payment", "a", "outbound", BTC, "2024-06-01T00:00:00Z", 30_000
            )
            _seed_transaction(
                conn, "sale", "a", "outbound", BTC, "2024-07-01T00:00:00Z", 30_000
            )
            conn.execute("UPDATE transactions SET kind = 'sell' WHERE id = 'sale'")
            handlers.process_journals(conn, "Books", "Book")

            summary = build_journals_quarantine_snapshot(conn, {"limit": 5})["summary"]

        outbound = summary["assumptions"]["presumed_external_outbound"]
        self.assertEqual([item["transaction_id"] for item in outbound["items"]], ["payment"])
        self.assertEqual(outbound["amount_msat"], BTC)
        inbound = summary["assumptions"]["unclassified_inbound"]
        # The explicit purchase is evidence; the kind-less receipt is presumed.
        self.assertEqual([item["transaction_id"] for item in inbound["items"]], ["buy-new"])

    def test_stale_projection_and_paging_are_reported(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")
            conn.execute(
                "UPDATE profiles SET journal_input_version = journal_input_version + 1"
            )
            conn.commit()

            page = build_journals_quarantine_snapshot(conn, {"limit": 1, "offset": 1})
            with self.assertRaises(AppError):
                build_journals_quarantine_snapshot(conn, {"offset": -1})

        self.assertTrue(page["summary"]["freshness"]["needs_processing"])
        self.assertEqual(page["summary"]["offset"], 1)
        self.assertEqual([item["transaction_id"] for item in page["items"]], ["later-sale"])


    def test_scopes_list_what_needs_the_user_apart_from_what_waits(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")

            attention = build_journals_quarantine_snapshot(conn, {"limit": 50, "scope": "attention"})
            waiting = build_journals_quarantine_snapshot(conn, {"limit": 50, "scope": "waiting"})
            everything = build_journals_quarantine_snapshot(conn, {"limit": 50})
            badges = build_review_badges_snapshot(conn)
            with self.assertRaises(AppError):
                build_journals_quarantine_snapshot(conn, {"scope": "roots"})

        self.assertEqual([item["transaction_id"] for item in attention["items"]], ["out"])
        self.assertEqual([item["transaction_id"] for item in waiting["items"]], ["later-sale"])
        self.assertEqual(len(everything["items"]), 2)
        self.assertEqual(
            (everything["summary"]["workspace_id"], everything["summary"]["profile_id"]),
            ("ws", "profile"),
        )
        for page, scope, listed in ((attention, "attention", 1), (waiting, "waiting", 1), (everything, "all", 2)):
            summary = page["summary"]
            self.assertEqual(summary["scope"], scope)
            self.assertEqual(summary["scope_count"], listed)
            # The summary always covers the whole book.
            self.assertEqual(summary["count"], 2)
            self.assertEqual(summary["attention_count"], 1)
            self.assertEqual(summary["waiting_count"], 1)
        group = everything["summary"]["groups"][0]
        self.assertEqual(group["root_transaction_ids"], ["out"])
        self.assertEqual(group["root_count"], 1)
        self.assertEqual({item["group_key"] for item in everything["items"]}, {group["key"]})
        # The side-nav counts causes, not every row that follows one.
        self.assertEqual(badges["quarantine"], 2)
        self.assertEqual(badges["quarantine_attention"], 1)

    def test_review_cases_list_causes_and_count_what_waits_on_them(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")
            profile = conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone()

            cases = review_workflow.inspect_cases(conn, profile, limit=100)

        # The sale only waits on the custody problem: repairing that clears it,
        # so a reviewer pages through causes, not their consequences.
        self.assertEqual([case["transaction_id"] for case in cases["cases"]], ["out"])
        self.assertEqual(cases["cases"][0]["waiting_count"], 1)
        self.assertEqual(cases["waiting_count"], 1)
        self.assertIsNone(cases["next_cursor"])

    def test_the_badge_counts_unresolved_downstream_rows_like_the_attention_scope(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")
            # A second BTC root at the same time as "out", and a row whose
            # lots turned uncertain then: two roots fit, so none is named.
            # "later-sale" still follows its named root.
            _seed_transaction(conn, "twin", "a", "outbound", BTC, SOURCE_AT, 30_000)
            _seed_transaction(
                conn, "tainted", "a", "outbound", BTC, "2025-02-01T00:00:00Z", 30_000
            )
            conn.executemany(
                """
                INSERT INTO journal_quarantines(
                    transaction_id, workspace_id, profile_id, reason, detail_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, 'now')
                """,
                [
                    ("twin", "missing_spot_price", "{}"),
                    (
                        "tainted",
                        "basis_provenance_incomplete",
                        json.dumps({"lot_state_uncertain_since": SOURCE_AT}),
                    ),
                ],
            )
            conn.commit()

            attention = build_journals_quarantine_snapshot(conn, {"limit": 50, "scope": "attention"})
            waiting = build_journals_quarantine_snapshot(conn, {"limit": 50, "scope": "waiting"})
            badges = build_review_badges_snapshot(conn)

        self.assertEqual(
            sorted(item["transaction_id"] for item in attention["items"]),
            ["out", "tainted", "twin"],
        )
        self.assertEqual([item["transaction_id"] for item in waiting["items"]], ["later-sale"])
        self.assertEqual(attention["summary"]["attention_count"], 3)
        self.assertEqual(badges["quarantine"], 4)
        self.assertEqual(badges["quarantine_attention"], 3)

    def test_the_ai_tool_accepts_the_scope_and_offset_it_is_told_to_send(self):
        tool = get_tool("ui.journals.quarantine")
        self.assertEqual(
            tool.parameters["properties"]["scope"]["enum"],
            list(quarantine_review.SCOPES),
        )
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")

            def call(arguments):
                return run_tool(
                    conn,
                    data_root=root,
                    runtime_config={},
                    name="journals_quarantine",
                    arguments=arguments,
                    workspace="ws",
                    profile="profile",
                    project_name=None,
                )

            attention = call({"scope": "attention", "offset": 0, "limit": 10})["data"]
            waiting = call({"scope": "waiting", "offset": 0, "limit": 10})["data"]
            past_the_end = call({"scope": "attention", "offset": 1, "limit": 10})["data"]
            for bad in ({"scope": "roots"}, {"offset": -1}):
                with self.assertRaises(AppError) as raised:
                    call(bad)
                self.assertEqual(raised.exception.code, "validation")

        self.assertEqual([item["transaction_id"] for item in attention["items"]], ["out"])
        self.assertEqual(attention["summary"]["scope"], "attention")
        self.assertEqual([item["transaction_id"] for item in waiting["items"]], ["later-sale"])
        self.assertEqual(past_the_end["items"], [])
        self.assertEqual(past_the_end["summary"]["offset"], 1)

    def test_a_pair_that_leaves_a_suspense_points_at_the_pair(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            # Two unrelated on-chain transactions: different txids, and the
            # receipt is older than the spend it is paired with.
            conn.execute("UPDATE transactions SET external_id = ? WHERE id = 'out'", ("a" * 64,))
            conn.execute(
                "UPDATE transactions SET external_id = ?, occurred_at = ? WHERE id = 'in'",
                ("b" * 64, "2023-12-31T21:00:00Z"),
            )
            conn.commit()
            pair = handlers.create_transaction_pair(conn, "Books", "Book", "out", "in")
            handlers.process_journals(conn, "Books", "Book")

            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 50, "scope": "attention"})

        roots = [item for item in snapshot["items"] if item["evidence"].get("blocker_code") == "reviewed_residual_suspense"]
        self.assertTrue(roots)
        root_item = roots[0]
        evidence = root_item["evidence"]
        self.assertEqual(evidence["pair_id"], pair["id"])
        self.assertIn(evidence["pair_counterpart_transaction_id"], {"out", "in"})
        self.assertNotEqual(evidence["pair_counterpart_transaction_id"], root_item["transaction_id"])
        self.assertTrue(evidence["pair_txids_differ"])
        self.assertTrue(evidence["pair_receipt_before_spend"])
        legs = evidence["pair_legs"]
        self.assertEqual(
            (legs["out"]["transaction_id"], legs["out"]["wallet"], legs["out"]["amount_msat"]),
            ("out", "A", 10 * BTC),
        )
        self.assertEqual(
            (legs["in"]["transaction_id"], legs["in"]["wallet"], legs["in"]["external_id"]),
            ("in", "C", "b" * 64),
        )
        self.assertEqual(legs["in"]["occurred_at"], "2023-12-31T21:00:00Z")
        review = evidence["pair_review"]
        self.assertEqual(review["kind"], pair["kind"])
        self.assertEqual(set(review), {"kind", "policy", "out_amount_msat", "in_amount_msat"})
        self.assertIn(
            {"kind": "review_pair", "transaction_id": root_item["transaction_id"], "pair_id": pair["id"]},
            root_item["actions"],
        )
        group = next(group for group in snapshot["summary"]["groups"] if group["key"] == root_item["group_key"])
        self.assertEqual(group["actions"][0]["kind"], "review_pair")

    def test_every_pair_of_a_shared_leg_can_be_found_by_id(self):
        # Pair terms A->X, A->Y and B->X: A->X comes last, after A->Y has
        # claimed A and B->X has claimed X in the one-pair-per-leg map.
        records = [
            {"id": "a-y", "out_transaction_id": "A", "in_transaction_id": "Y"},
            {"id": "b-x", "out_transaction_id": "B", "in_transaction_id": "X"},
            {"id": "a-x", "out_transaction_id": "A", "in_transaction_id": "X"},
        ]
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            with mock.patch(
                "kassiber.core.custody_authored_migration.list_pair_review_records",
                return_value=records,
            ):
                by_leg = quarantine_review.pairs_by_transaction(conn, "profile")
                by_id = quarantine_review.pairs_by_id(conn, "profile")

        self.assertNotIn("a-x", {str(pair["id"]) for pair in by_leg.values()})
        self.assertEqual(set(by_id), {"a-y", "b-x", "a-x"})
        self.assertEqual(by_id["a-x"]["in_transaction_id"], "X")

    def test_unpair_refuses_a_pair_that_no_longer_holds_a_suspense(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            # A pair that adds up: both legs of one movement, nothing held.
            conn.execute("UPDATE transactions SET external_id = ? WHERE id IN ('out', 'in')", ("a" * 64,))
            conn.execute("UPDATE transactions SET amount = ? WHERE id = 'in'", (10 * BTC,))
            conn.commit()
            pair = handlers.create_transaction_pair(conn, "Books", "Book", "out", "in")
            handlers.process_journals(conn, "Books", "Book")
            profile = conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone()
            hooks = review_workflow.ReviewHooks(metadata=_metadata_hooks())
            cases = review_workflow.inspect_cases(conn, profile, limit=100)

            with self.assertRaises(AppError) as raised:
                review_workflow.plan_review(
                    conn, profile,
                    operations=[_unpair(pair["id"], _fingerprint(conn, pair["id"]))],
                    expected_input_version=cases["input_version"], hooks=hooks,
                )
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}

        self.assertEqual(raised.exception.code, "review_case_changed")
        self.assertIn(pair["id"], pairs_after)

    def test_a_pair_left_suspense_is_fixed_by_one_reviewed_unpair(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            conn.execute("UPDATE transactions SET external_id = ? WHERE id = 'out'", ("a" * 64,))
            conn.execute(
                "UPDATE transactions SET external_id = ?, occurred_at = ? WHERE id = 'in'",
                ("b" * 64, "2023-12-31T21:00:00Z"),
            )
            conn.commit()
            pair = handlers.create_transaction_pair(conn, "Books", "Book", "out", "in")
            handlers.process_journals(conn, "Books", "Book")
            profile = conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone()
            hooks = review_workflow.ReviewHooks(metadata=_metadata_hooks())

            cases = review_workflow.inspect_cases(conn, profile, limit=100)
            paired = [case for case in cases["cases"] if "unpair" in case["supported_operations"]]
            operations = [_unpair(pair["id"], paired[0]["pair"]["pair_fingerprint"])]
            artifact = review_workflow.plan_review(
                conn, profile, operations=operations,
                expected_input_version=cases["input_version"], hooks=hooks,
            )
            # The preview changed nothing yet.
            self.assertIn(pair["id"], {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")})
            receipt = review_workflow.apply_review(
                conn, profile, artifact=artifact, idempotency_key="fix-1", hooks=hooks,
            )
            remaining = build_journals_quarantine_snapshot(conn, {"limit": 50})["summary"]["count"]
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}
            with self.assertRaises(AppError):
                review_workflow.plan_review(
                    conn, profile,
                    operations=[_unpair("missing", "0" * 64, "x")],
                    expected_input_version=receipt["result_input_version"], hooks=hooks,
                )

        self.assertTrue(paired)
        self.assertEqual(paired[0]["pair"]["pair_id"], pair["id"])
        self.assertTrue(paired[0]["pair"]["pair_txids_differ"])
        # The server-computed preview says what the fix does to the book.
        self.assertGreater(artifact["before"]["quarantine_count"], 0)
        self.assertEqual(artifact["after"]["quarantine_count"], 0)
        self.assertEqual(receipt["status"], "verified")
        self.assertEqual(receipt["verification"]["quarantine_count"], 0)
        self.assertEqual(set(receipt["transaction_ids"]), {"out", "in"})
        self.assertNotIn(pair["id"], pairs_after)
        self.assertEqual(remaining, 0)

    def _profile_and_hooks(self, conn):
        profile = conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone()
        return profile, review_workflow.ReviewHooks(metadata=_metadata_hooks())

    def test_unpair_refuses_a_pair_revised_since_it_was_confirmed(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            pair = _unrelated_pair(conn)
            confirmed = _fingerprint(conn, pair["id"])
            # Another session revises the same pair id while it still holds
            # its suspense.
            handlers.update_transaction_pair(conn, "Books", "Book", pair["id"], kind="coinjoin")
            handlers.process_journals(conn, "Books", "Book")
            profile, hooks = self._profile_and_hooks(conn)
            version = review_workflow.inspect_cases(conn, profile, limit=100)["input_version"]

            with self.assertRaises(AppError) as raised:
                review_workflow.plan_review(
                    conn, profile, operations=[_unpair(pair["id"], confirmed)],
                    expected_input_version=version, hooks=hooks,
                )
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}

        self.assertEqual(raised.exception.code, "review_case_changed")
        self.assertIn(pair["id"], pairs_after)

    def test_unpair_refuses_a_replacement_pair_on_the_old_confirmation(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            first = _unrelated_pair(conn)
            confirmed = _fingerprint(conn, first["id"])
            # Another session replaces it with a pair of the same legs.
            handlers.delete_transaction_pair(conn, "Books", "Book", first["id"])
            second = handlers.create_transaction_pair(conn, "Books", "Book", "out", "in")
            handlers.process_journals(conn, "Books", "Book")
            profile, hooks = self._profile_and_hooks(conn)
            version = review_workflow.inspect_cases(conn, profile, limit=100)["input_version"]

            errors = []
            for operation in (_unpair(first["id"], confirmed), _unpair(second["id"], confirmed)):
                with self.assertRaises(AppError) as raised:
                    review_workflow.plan_review(
                        conn, profile, operations=[operation],
                        expected_input_version=version, hooks=hooks,
                    )
                errors.append(raised.exception.code)
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}

        self.assertEqual(errors, ["not_found", "review_case_changed"])
        self.assertIn(second["id"], pairs_after)

    def test_apply_refuses_a_confirmation_that_no_longer_matches(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            pair = _unrelated_pair(conn)
            profile, hooks = self._profile_and_hooks(conn)
            version = review_workflow.inspect_cases(conn, profile, limit=100)["input_version"]
            artifact = review_workflow.plan_review(
                conn, profile, operations=[_unpair(pair["id"], _fingerprint(conn, pair["id"]))],
                expected_input_version=version, hooks=hooks,
            )
            # A forged artifact with another reading of the same pair id.
            forged = dict(artifact, operations=[_unpair(pair["id"], "f" * 64)])
            forged["digest"] = review_workflow._digest(
                {key: value for key, value in forged.items() if key != "digest"}
            )
            with self.assertRaises(AppError) as raised:
                review_workflow.apply_review(
                    conn, profile, artifact=forged, idempotency_key="forged", hooks=hooks,
                )
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}

        self.assertEqual(raised.exception.code, "review_case_changed")
        self.assertIn(pair["id"], pairs_after)

    def test_a_batch_judges_an_unpair_after_the_operations_before_it(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            pair = _unrelated_pair(conn)
            record = quarantine_review.pairs_by_id(conn, "profile")[pair["id"]]
            component = get_component(conn, record["component_id"])
            profile, hooks = self._profile_and_hooks(conn)
            version = review_workflow.inspect_cases(conn, profile, limit=100)["input_version"]
            # First revise the pair's component so the residual is a reviewed
            # fee (the pair term stays), then unpair it in the same batch.
            legs = [
                {
                    **{key: value for key, value in leg.items()
                       if key not in ("component_id", "ordinal", "created_at")},
                    "role": "fee" if leg["role"] == "suspense" else leg["role"],
                }
                for leg in component["legs"]
            ]
            allocations = [
                {key: allocation[key] for key in
                 ("source_leg_id", "sink_leg_id", "source_amount_msat", "sink_amount_msat")}
                for allocation in component["allocations"]
            ]
            revise = {"type": "custody_component", "request": {
                "action": "revise", "component_id": component["id"],
                "spec": {"legs": legs, "allocations": allocations},
                "activate": True, "reason": "The difference was the network fee",
            }}

            with self.assertRaises(AppError) as raised:
                review_workflow.plan_review(
                    conn, profile,
                    operations=[revise, _unpair(pair["id"], quarantine_review.pair_fingerprint(conn, record))],
                    expected_input_version=version, hooks=hooks,
                )
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}
            # The suspense guard on its own: before the revision the pair's
            # component holds an open suspense slice, after it none.
            held_before = review_workflow._holds_residual_suspense(review_workflow._build(conn, profile), record)
            custody_component_planner.apply_component_review(
                conn, workspace_id="ws", profile_id="profile",
                expected_input_version=version, **revise["request"],
            )
            revised = quarantine_review.pairs_by_id(conn, "profile")[pair["id"]]
            held_after = review_workflow._holds_residual_suspense(review_workflow._build(conn, profile), revised)

        self.assertEqual(raised.exception.code, "review_case_changed")
        self.assertIn(pair["id"], pairs_after)
        self.assertTrue(held_before)
        self.assertFalse(held_after)

    def test_an_allocation_only_revision_since_confirmation_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            pair = _unrelated_pair(conn)
            profile, hooks = self._profile_and_hooks(conn)
            record = quarantine_review.pairs_by_id(conn, "profile")[pair["id"]]
            confirmed = quarantine_review.pair_fingerprint(conn, record)
            component = get_component(conn, record["component_id"])
            # Between the owner's look and the version read, another session
            # books half of the suspense as paid out to someone else: same
            # id, kind, policy, term amount and legs; only the allocations
            # differ.
            legs = [
                {key: value for key, value in leg.items()
                 if key not in ("component_id", "ordinal", "created_at")}
                for leg in component["legs"]
            ]
            suspense = next(leg for leg in legs if leg["role"] == "suspense")
            half = suspense["amount_msat"] // 2
            suspense["amount_msat"] = half
            legs.append({**suspense, "id": "paid-out", "role": "external", "amount_msat": half})
            source = next(leg for leg in legs if leg["role"] == "source")
            allocations = [
                {key: allocation[key] for key in
                 ("source_leg_id", "sink_leg_id", "source_amount_msat", "sink_amount_msat")}
                for allocation in component["allocations"]
            ]
            for allocation in allocations:
                if allocation["sink_leg_id"] == suspense["id"]:
                    allocation["source_amount_msat"] = allocation["sink_amount_msat"] = half
            allocations.append({
                "source_leg_id": source["id"], "sink_leg_id": "paid-out",
                "source_amount_msat": half, "sink_amount_msat": half,
            })
            version = review_workflow.inspect_cases(conn, profile, limit=100)["input_version"]
            custody_component_planner.apply_component_review(
                conn, workspace_id="ws", profile_id="profile", expected_input_version=version,
                action="revise", component_id=component["id"],
                spec={"legs": legs, "allocations": allocations}, activate=True,
                reason="Half of the difference paid a third party",
            )
            handlers.process_journals(conn, "Books", "Book")
            revised = quarantine_review.pairs_by_id(conn, "profile")[pair["id"]]
            version = review_workflow.inspect_cases(conn, profile, limit=100)["input_version"]

            with self.assertRaises(AppError) as raised:
                review_workflow.plan_review(
                    conn, profile, operations=[_unpair(pair["id"], confirmed)],
                    expected_input_version=version, hooks=hooks,
                )
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}

        # The review fields alone did not change; the allocations did.
        self.assertEqual(
            (revised["kind"], revised["policy"], revised["out_amount"]),
            (record["kind"], record["policy"], record["out_amount"]),
        )
        self.assertEqual(raised.exception.code, "review_case_changed")
        self.assertIn(pair["id"], pairs_after)

    def test_a_shared_leg_batch_survives_its_own_sibling_unpair(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            # out (10 BTC) pays in (5 BTC) and in2 (4 BTC) under one group:
            # removing one pair re-slices the other's allocation.
            conn.execute("UPDATE transactions SET external_id = ? WHERE id = 'out'", ("a" * 64,))
            conn.execute("UPDATE transactions SET amount = ?, external_id = ? WHERE id = 'in'", (5 * BTC, "b" * 64))
            _seed_transaction(conn, "in2", "c", "inbound", 4 * BTC, "2025-01-02T00:00:00Z", 30_000)
            conn.execute("UPDATE transactions SET external_id = ? WHERE id = 'in2'", ("c" * 64,))
            conn.commit()
            first = handlers.create_transaction_pair(conn, "Books", "Book", "out", "in")
            second = handlers.create_transaction_pair(conn, "Books", "Book", "out", "in2")
            handlers.process_journals(conn, "Books", "Book")
            profile, hooks = self._profile_and_hooks(conn)
            version = review_workflow.inspect_cases(conn, profile, limit=100)["input_version"]
            operations = [
                _unpair(first["id"], _fingerprint(conn, first["id"])),
                _unpair(second["id"], _fingerprint(conn, second["id"])),
            ]
            artifact = review_workflow.plan_review(
                conn, profile, operations=operations, expected_input_version=version, hooks=hooks,
            )
            receipt = review_workflow.apply_review(
                conn, profile, artifact=artifact, idempotency_key="siblings", hooks=hooks,
            )
            pairs_after = {item["id"] for item in handlers.list_transaction_pairs(conn, "Books", "Book")}

        self.assertEqual(receipt["status"], "verified")
        self.assertNotIn(first["id"], pairs_after)
        self.assertNotIn(second["id"], pairs_after)

    def test_a_suspense_without_a_pair_offers_no_pair_review(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")

            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 50})

        root_item = next(item for item in snapshot["items"] if item["transaction_id"] == "out")
        self.assertNotIn("pair_id", root_item["evidence"])
        self.assertEqual(root_item["actions"], [])



class QuarantineEvidenceAssetTest(unittest.TestCase):
    def test_non_bitcoin_quantities_are_not_rendered_as_msat(self):
        from kassiber.core.quarantine_review import _evidence

        row = {"wallet": "Hot", "asset": "USDT", "transaction_id": "t"}
        evidence = _evidence(
            "insufficient_lots", {"asset": "USDT", "required": 5.0, "available": 1.0}, row, {}
        )
        self.assertNotIn("required_msat", evidence)
        btc = _evidence(
            "insufficient_lots", {"asset": "BTC", "required": 0.5, "available": 0.1},
            {**row, "asset": "BTC"}, {},
        )
        self.assertEqual(btc["required_msat"], 50_000_000_000)



class QuarantineChainedRootTest(unittest.TestCase):
    def test_dependency_chains_resolve_to_their_ultimate_root(self):
        import json

        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            self.addCleanup(conn.close)
            set_setting(conn, "context_workspace", "ws")
            set_setting(conn, "context_profile", "profile")
            for tx_id, reason, detail in (
                ("out", "pending_onchain_confirmation", {}),
                ("in", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["out"]}),
                ("later-sale", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["in"]}),
            ):
                conn.execute(
                    "INSERT INTO journal_quarantines(transaction_id, workspace_id, profile_id, "
                    "reason, detail_json, created_at) VALUES(?, 'ws', 'profile', ?, ?, 'now')",
                    (tx_id, reason, json.dumps(detail)),
                )
            conn.commit()
            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 10})

        items = {item["transaction_id"]: item for item in snapshot["items"]}
        self.assertEqual(items["in"]["root"]["transaction_id"], "out")
        self.assertEqual(items["later-sale"]["root"]["transaction_id"], "out")
        self.assertNotIn("wallet_id", items["out"])
        self.assertEqual(snapshot["summary"]["group_count"], 1)


if __name__ == "__main__":
    unittest.main()
