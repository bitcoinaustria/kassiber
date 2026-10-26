"""One explicit clock per custody build.

Swap-dismissal expiry is the only time-dependent input of the custody path.
The journal builder judges it against one ``as_of`` instant, so the same
inputs at the same ``as_of`` give the same decisions no matter when the build
runs.
"""

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core import custody_journal
from kassiber.core.custody_evidence import build_canonical_quantity_input, enriched_quantity_rows
from kassiber.core.custody_interpreters import compile_custody_interpreters
from kassiber.db import open_db
from tests.test_custody_native_transitions import (
    PROFILE,
    native_transition_rows,
    seed_native_transition_book,
)


EXPIRES_AT = "2026-01-01T00:00:00Z"


class _WallClockForbidden:
    """Stands in for ``datetime`` where reading the wall clock is a bug."""

    @staticmethod
    def now(*_args, **_kwargs):
        raise AssertionError("the custody path read the wall clock")


def _has_native_claim(claims) -> bool:
    return any(claim.reason == "native_htlc_transition" for claim in claims)


class InterpreterAsOfTest(unittest.TestCase):
    def compile(self, *, as_of, expires_at=EXPIRES_AT):
        rows, refs = native_transition_rows()
        canonical = build_canonical_quantity_input(enriched_quantity_rows(rows))
        return compile_custody_interpreters(
            rows,
            canonical,
            profile=PROFILE,
            wallet_refs_by_id=refs,
            swap_dismissals=[{
                "out_transaction_id": "send",
                "in_transaction_id": "claim",
                "expires_at": expires_at,
            }],
            as_of=as_of,
        )

    def test_dismissal_is_judged_against_as_of_not_the_wall_clock(self):
        cases = (
            # A dismissal holds until the instant it expires...
            ("2025-12-31T23:59:59Z", False),
            ("2025-12-31T23:59:59.999999Z", False),
            # ...and has expired from that instant on (`expires_at <= as_of`).
            (EXPIRES_AT, True),
            ("2026-01-01T00:00:00.000001Z", True),
            ("2030-06-01T12:00:00+02:00", True),
        )
        with patch("kassiber.core.transfer_matching.datetime", _WallClockForbidden):
            for as_of, native_claim in cases:
                with self.subTest(as_of=as_of):
                    compiled = self.compile(as_of=as_of)
                    self.assertEqual(_has_native_claim(compiled.claims), native_claim)

    def test_a_dismissal_without_expiry_never_lapses(self):
        with patch("kassiber.core.transfer_matching.datetime", _WallClockForbidden):
            compiled = self.compile(as_of="2999-01-01T00:00:00Z", expires_at=None)
        self.assertFalse(_has_native_claim(compiled.claims))

    def test_without_as_of_the_interpreter_still_reads_the_wall_clock(self):
        # Direct callers that pass no clock keep the old behavior.
        with patch("kassiber.core.transfer_matching.datetime", _WallClockForbidden):
            with self.assertRaisesRegex(AssertionError, "wall clock"):
                self.compile(as_of=None)


class BuilderAsOfTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.conn = open_db(Path(self._tmp.name) / "data")
        seed_native_transition_book(self.conn, "claim")
        self.conn.execute(
            "INSERT INTO transaction_pair_dismissals("
            "id, workspace_id, profile_id, out_transaction_id, in_transaction_id,"
            " created_at, expires_at"
            ") VALUES('dismissal', 'workspace', 'profile', 'send', 'claim',"
            " '2025-06-01T00:00:00Z', ?)",
            (EXPIRES_AT,),
        )
        self.profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def decisions(self, as_of):
        builder = custody_journal.CustodyJournalBuilder(
            self.conn, self.profile, as_of=as_of,
        )
        return builder.build_custody_decisions()

    def test_same_inputs_and_as_of_give_the_same_decisions(self):
        with patch("kassiber.core.transfer_matching.datetime", _WallClockForbidden):
            active = self.decisions("2025-12-31T23:59:59Z")
            expired = self.decisions(EXPIRES_AT)
            repeated = self.decisions(EXPIRES_AT)
        self.assertEqual(active.as_of, "2025-12-31T23:59:59Z")
        self.assertEqual(expired.as_of, EXPIRES_AT)
        self.assertFalse(_has_native_claim(active.interpretation.claims))
        self.assertTrue(_has_native_claim(expired.interpretation.claims))
        self.assertEqual(
            repeated.quantity_state.projection.decisions,
            expired.quantity_state.projection.decisions,
        )

    def test_projection_and_ledger_state_carry_the_explicit_as_of(self):
        with patch("kassiber.core.transfer_matching.datetime", _WallClockForbidden):
            projection = custody_journal.CustodyJournalBuilder(
                self.conn, self.profile, as_of=EXPIRES_AT,
            ).build_custody_projection()
            state = custody_journal.build_ledger_state(
                self.conn, self.profile, as_of="2025-12-31T23:59:59Z",
            )
        self.assertEqual(projection.as_of, EXPIRES_AT)
        self.assertTrue(_has_native_claim(projection.interpretation.claims))
        self.assertFalse(any(
            decision.reason == "native_htlc_transition"
            for decision in state["custody_quantity"].projection.decisions
        ))

    def test_without_as_of_each_build_reads_the_clock_once(self):
        clock = iter(["2025-12-31T23:59:59.5Z", "2026-01-01T00:00:00.5Z"])
        with patch.object(
            custody_journal, "custody_build_clock", side_effect=lambda: next(clock)
        ) as build_clock, patch(
            "kassiber.core.transfer_matching.datetime", _WallClockForbidden
        ):
            builder = custody_journal.CustodyJournalBuilder(self.conn, self.profile)
            first = builder.build_custody_decisions()
            second = builder.build_custody_decisions()
        self.assertEqual(build_clock.call_count, 2)
        self.assertEqual(first.as_of, "2025-12-31T23:59:59.5Z")
        self.assertEqual(second.as_of, "2026-01-01T00:00:00.5Z")
        self.assertFalse(_has_native_claim(first.interpretation.claims))
        self.assertTrue(_has_native_claim(second.interpretation.claims))

    def test_build_clock_keeps_microseconds(self):
        value = custody_journal.custody_build_clock()
        self.assertTrue(value.endswith("Z"))
        self.assertIsNotNone(custody_journal.parse_iso_datetime_or_none(value))

    def test_malformed_as_of_is_refused(self):
        for as_of in ("", "yesterday", "2026-13-01T00:00:00Z"):
            with self.subTest(as_of=as_of):
                with self.assertRaises(ValueError):
                    custody_journal.CustodyJournalBuilder(
                        self.conn, self.profile, as_of=as_of,
                    )


if __name__ == "__main__":
    unittest.main()
