"""Direct payout times reach the tax engine as canonical RFC3339 UTC.

`transfers payouts create --payout-occurred-at` used to store the typed text,
and the projection copied it into the payout's settlement rows. RP2 parsed
those with dateutil; Kassiber's engine takes only ISO 8601, so a typed
"2025-09-01 12:00:30 +00:00" blocked every report. New payouts are stored
canonical, and the projection canonicalizes values stored before that rule.
The module is in the tax-shadow overlay, which compares each journal run here
with the frozen RP2 oracle.
"""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

from kassiber.cli.main import main
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.time_utils import parse_legacy_timestamp

TXID = "e" * 64
WORKSPACE, PROFILE = "PayoutTimes", "PT"
SPEND_CSV = f"""date,txid,direction,asset,amount,fee,fiat_rate,description
2025-06-01T10:00:00Z,payout-time-fund,inbound,BTC,0.10000000,0,30000,Funding
2025-09-01T12:00:00Z,{TXID},outbound,BTC,0.05000000,0,60000,Spend
"""
KEEP_CSV = f"""date,txid,direction,asset,amount,fee,fiat_rate,description
2025-09-01T12:00:00Z,{TXID},inbound,BTC,0.03000000,0,60000,Change
"""
JOURNAL_COLUMNS = (
    "transaction_id", "wallet_id", "occurred_at", "entry_type", "asset",
    "quantity", "fiat_value_exact", "unit_cost_exact", "cost_basis_exact",
    "proceeds_exact", "gain_loss_exact", "at_category", "at_kennzahl",
    "capital_gains_type",
)


class DirectPayoutTimestampTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-payout-times-")
        self.root = Path(self._tmp.name)
        self.data_root = self.root / "data"
        spend, keep = self.root / "spend.csv", self.root / "keep.csv"
        spend.write_text(SPEND_CSV)
        keep.write_text(KEEP_CSV)
        self._ok("init")
        self._ok("workspaces", "create", WORKSPACE)
        self._ok("profiles", "create", "--workspace", WORKSPACE,
                 "--fiat-currency", "EUR", "--tax-country", "at", PROFILE)
        for label, path in (("Spend", spend), ("Keep", keep)):
            self._ok("wallets", "create", "--workspace", WORKSPACE, "--profile", PROFILE,
                     "--label", label, "--kind", "custom")
            self._ok("wallets", "import-csv", "--workspace", WORKSPACE, "--profile", PROFILE,
                     "--wallet", label, "--file", str(path))

    def tearDown(self):
        self._tmp.cleanup()

    def _cli(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = main(["--data-root", str(self.data_root), "--machine", *args])
        return code, json.loads(out.getvalue())

    def _ok(self, *args):
        code, payload = self._cli(*args)
        self.assertEqual(code, 0, json.dumps(payload)[:600])
        return payload["data"]

    def _create_payout(self, occurred_at):
        return self._cli(
            "transfers", "payouts", "create", "--workspace", WORKSPACE, "--profile", PROFILE,
            "--tx-out", TXID, "--payout-asset", "LBTC", "--payout-amount", "0.0198",
            "--payout-fiat-value", "1188", "--policy", "carrying-value",
            "--out-amount", "0.02", "--payout-occurred-at", occurred_at,
        )

    def _store_legacy_payout_time(self, payout_id, value):
        """Write `value` the way a review authored before canonicalization kept it."""
        conn = open_db(self.data_root)
        try:
            conn.execute("DROP TRIGGER trg_custody_component_terms_immutable")
            updated = conn.execute(
                "UPDATE custody_component_economic_terms SET payout_occurred_at = ? "
                "WHERE term_kind = 'direct_swap_payout' AND legacy_source_id = ?",
                (value, payout_id),
            ).rowcount
            conn.commit()
        finally:
            conn.close()
        self.assertEqual(updated, 1)

    def _journals(self):
        code, payload = self._cli("journals", "process", "--workspace", WORKSPACE,
                                  "--profile", PROFILE)
        if code != 0:
            return payload["error"]
        conn = open_db(self.data_root)
        try:
            rows = conn.execute(
                f"SELECT {', '.join(JOURNAL_COLUMNS)} FROM journal_entries "
                "ORDER BY occurred_at, transaction_id, entry_type, asset, quantity"
            ).fetchall()
        finally:
            conn.close()
        return [dict(row) for row in rows]

    def test_create_stores_an_offset_payout_time_canonical(self):
        # RFC3339 with an offset (and a space separator), accepted by
        # parse_timestamp on every supported Python.
        self._assert_create_stores_canonical("2025-09-01 14:00:30+02:00")

    @unittest.skipIf(
        sys.version_info < (3, 11),
        "Python 3.10 fromisoformat rejects a space before the offset",
    )
    def test_create_stores_a_spaced_offset_payout_time_canonical(self):
        # Not RFC3339, but 3.11+ parse_timestamp accepts it; it must still be
        # stored canonical rather than as typed.
        self._assert_create_stores_canonical("2025-09-01 12:00:30 +00:00")

    def _assert_create_stores_canonical(self, typed):
        code, payload = self._create_payout(typed)
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["payout_occurred_at"], "2025-09-01T12:00:30Z")
        payout = self._ok("transfers", "payouts", "list", "--workspace", WORKSPACE,
                          "--profile", PROFILE)[0]
        self.assertEqual(payout["payout"]["occurred_at"], "2025-09-01T12:00:30Z")
        conn = open_db(self.data_root)
        try:
            stored = conn.execute(
                "SELECT payout_occurred_at FROM custody_component_economic_terms "
                "WHERE term_kind = 'direct_swap_payout'"
            ).fetchall()
        finally:
            conn.close()
        self.assertEqual([row[0] for row in stored], ["2025-09-01T12:00:30Z"])

    def test_create_rejects_an_unparseable_payout_time(self):
        code, payload = self._create_payout("next tuesday")
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("--payout-occurred-at", payload["error"]["message"])
        self.assertEqual(
            self._ok("transfers", "payouts", "list", "--workspace", WORKSPACE,
                     "--profile", PROFILE),
            [],
        )

    def test_legacy_stored_payout_time_books_like_the_canonical_time(self):
        payout_id = self._ok_payout("2025-09-01T12:00:30Z")
        canonical = self._journals()
        self.assertIsInstance(canonical, list)
        for legacy in ("2025-09-01 12:00:30 +00:00", "2025-09-01T12:00:30UTC",
                       " 2025-09-01t12:00:30z ", "20250901T120030Z"):
            with self.subTest(legacy):
                self._store_legacy_payout_time(payout_id, legacy)
                self.assertEqual(self._journals(), canonical)
        payout_rows = [row for row in canonical if row["asset"] == "LBTC"]
        self.assertEqual(
            {row["occurred_at"] for row in payout_rows}, {"2025-09-01T12:00:30Z"}
        )

    def test_unparseable_stored_payout_time_is_a_validation_error(self):
        payout_id = self._ok_payout("2025-09-01T12:00:30Z")
        self._store_legacy_payout_time(payout_id, "next tuesday")
        error = self._journals()
        self.assertEqual(error["code"], "validation", error)
        self.assertIn(payout_id, error["message"])
        self.assertIn("RFC3339", error["hint"])
        self.assertEqual(error["details"]["payout_id"], payout_id)

    def _ok_payout(self, occurred_at):
        code, payload = self._create_payout(occurred_at)
        self.assertEqual(code, 0, payload)
        return payload["data"]["id"]


class LegacyTimestampParserTests(unittest.TestCase):
    def test_accepts_typed_forms_and_returns_canonical_utc(self):
        cases = {
            "2025-09-01T12:00:30Z": "2025-09-01T12:00:30Z",
            "2025-09-01 12:00:30 +00:00": "2025-09-01T12:00:30Z",
            "2025-09-01T12:00:30 Z": "2025-09-01T12:00:30Z",
            "2025-09-01T12:00:30UTC": "2025-09-01T12:00:30Z",
            "2025-09-01T12:00:30 GMT": "2025-09-01T12:00:30Z",
            "2025-09-01t12:00:30z": "2025-09-01T12:00:30Z",
            "  2025-09-01T12:00:30Z\n": "2025-09-01T12:00:30Z",
            "20250301T090030Z": "2025-03-01T09:00:30Z",
            "2025-09-01T14:00:30+0200": "2025-09-01T12:00:30Z",
            "2025-09-01T07:00:30-05": "2025-09-01T12:00:30Z",
            "2025-09-01T12:00:30.987654321Z": "2025-09-01T12:00:30Z",
            "2025-09-01": "2025-09-01T00:00:00Z",
        }
        for value, expected in cases.items():
            with self.subTest(value):
                self.assertEqual(parse_legacy_timestamp(value), expected)

    def test_rejects_other_text_as_validation(self):
        for value in ("next tuesday", "2025-13-01T00:00:00Z", "Mon, 01 Sep 2025 12:00:30 +0000"):
            with self.subTest(value):
                with self.assertRaises(AppError) as raised:
                    parse_legacy_timestamp(value)
                self.assertEqual(raised.exception.code, "validation")


if __name__ == "__main__":
    unittest.main()
