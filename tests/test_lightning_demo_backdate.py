"""Unit tests for the demo-only Lightning backdating helper.

These cover the pure date-assignment function and the forward_day rebucketing
without any Docker/regtest stack.
"""

from __future__ import annotations

import json
import sqlite3
import tempfile
import unittest
import json
from datetime import datetime, timezone
from pathlib import Path

from kassiber.db import open_db
from tests.integration.lightning_demo_backdate import (
    _rebucket_forward_day_records,
    _stabilize_wallet_liquidity_dates,
    assign_historical_dates,
    backdate_ln_records,
)

WINDOW_START = "2019-01-15T09:00:00Z"
WINDOW_END = "2026-07-01T00:00:00Z"


def _parse(iso: str) -> datetime:
    return datetime.fromisoformat(iso.replace("Z", "+00:00")).astimezone(timezone.utc)


class AssignHistoricalDatesTests(unittest.TestCase):
    def test_all_dates_within_window(self) -> None:
        keys = [f"hash{i:02d}" for i in range(40)]
        dates = assign_historical_dates(keys, WINDOW_START, WINDOW_END, seed=0)
        start, end = _parse(WINDOW_START), _parse(WINDOW_END)
        self.assertEqual(len(dates), len(keys))
        for iso in dates.values():
            moment = _parse(iso)
            self.assertGreaterEqual(moment, start)
            self.assertLessEqual(moment, end)

    def test_deterministic_for_same_seed(self) -> None:
        keys = [f"hash{i:02d}" for i in range(25)]
        first = assign_historical_dates(keys, WINDOW_START, WINDOW_END, seed=7)
        second = assign_historical_dates(list(reversed(keys)), WINDOW_START, WINDOW_END, seed=7)
        self.assertEqual(first, second)

    def test_seed_changes_assignment(self) -> None:
        keys = [f"hash{i:02d}" for i in range(25)]
        a = assign_historical_dates(keys, WINDOW_START, WINDOW_END, seed=0)
        b = assign_historical_dates(keys, WINDOW_START, WINDOW_END, seed=1)
        self.assertNotEqual(a, b)

    def test_business_hours(self) -> None:
        keys = [f"hash{i:02d}" for i in range(60)]
        dates = assign_historical_dates(keys, WINDOW_START, WINDOW_END, seed=3)
        for iso in dates.values():
            hour = _parse(iso).hour
            self.assertGreaterEqual(hour, 6)
            self.assertLessEqual(hour, 17)

    def test_spans_multiple_years(self) -> None:
        keys = [f"hash{i:02d}" for i in range(50)]
        dates = assign_historical_dates(keys, WINDOW_START, WINDOW_END, seed=0)
        years = {_parse(iso).year for iso in dates.values()}
        self.assertGreaterEqual(len(years), 3)

    def test_empty_keys(self) -> None:
        self.assertEqual(
            assign_historical_dates([], WINDOW_START, WINDOW_END, seed=0), {}
        )
        self.assertEqual(
            assign_historical_dates(["", None], WINDOW_START, WINDOW_END, seed=0), {}  # type: ignore[list-item]
        )

    def test_invalid_window_raises(self) -> None:
        with self.assertRaises(ValueError):
            assign_historical_dates(["a"], WINDOW_END, WINDOW_START, seed=0)
        with self.assertRaises(ValueError):
            assign_historical_dates(["a"], WINDOW_START, WINDOW_START, seed=0)

    def test_stabilizes_outbound_after_wallet_inbound_lot(self) -> None:
        date_map = {
            "pay-hash": "2019-04-19T11:18:00Z",
            "income-hash": "2019-10-19T16:40:00Z",
        }
        adjusted = _stabilize_wallet_liquidity_dates(
            [
                {
                    "id": "pay",
                    "wallet_id": "merchant",
                    "asset": "BTC",
                    "direction": "outbound",
                    "amount": 50_000_000,
                    "fee": 0,
                    "payment_hash": "pay-hash",
                },
                {
                    "id": "income",
                    "wallet_id": "merchant",
                    "asset": "BTC",
                    "direction": "inbound",
                    "amount": 50_000_000,
                    "fee": 0,
                    "payment_hash": "income-hash",
                },
            ],
            date_map,
            WINDOW_END,
        )

        self.assertEqual(adjusted, 1)
        self.assertGreater(_parse(date_map["pay-hash"]), _parse(date_map["income-hash"]))


_FORWARD_DAY_DDL = """
CREATE TABLE lightning_node_records (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    wallet_id TEXT NOT NULL,
    backend_name TEXT NOT NULL,
    node_id TEXT,
    record_type TEXT NOT NULL,
    external_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    account TEXT,
    peer_id TEXT,
    channel_id TEXT,
    direction TEXT,
    amount_msat INTEGER NOT NULL DEFAULT 0,
    fee_msat INTEGER NOT NULL DEFAULT 0,
    tag TEXT,
    status TEXT,
    currency TEXT,
    payment_hash TEXT,
    txid TEXT,
    outpoint TEXT,
    sync_id TEXT,
    raw_json TEXT NOT NULL DEFAULT '{}',
    first_seen_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, wallet_id, backend_name, record_type, external_id)
);
"""


class RebucketForwardDayTests(unittest.TestCase):
    def _conn(self) -> sqlite3.Connection:
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.executescript(_FORWARD_DAY_DDL)
        return conn

    def _insert_forward_day(
        self, conn: sqlite3.Connection, external_id: str, channel_id: str, amount: int, fee: int
    ) -> None:
        conn.execute(
            """
            INSERT INTO lightning_node_records (
                id, workspace_id, profile_id, wallet_id, backend_name, node_id,
                record_type, external_id, occurred_at, channel_id, direction,
                amount_msat, fee_msat, tag, currency, raw_json, first_seen_at, updated_at
            ) VALUES (?, 'ws', 'p1', 'w1', 'cln-merchant', 'node1', 'forward_day', ?,
                      '2026-06-30T00:00:00Z', ?, 'inbound', ?, ?, 'routed', 'bc',
                      '{"forward_count": 1}', '2026-06-30T00:00:00Z', '2026-06-30T00:00:00Z')
            """,
            (f"w1:cln-merchant:forward_day:{external_id}", external_id, channel_id, amount, fee),
        )

    def test_two_forwards_same_new_day_collapse(self) -> None:
        conn = self._conn()
        self._insert_forward_day(conn, "fwd-a", "chan-1", 10_000, 5)
        self._insert_forward_day(conn, "fwd-b", "chan-1", 20_000, 7)
        conn.commit()
        # Force both source rows onto the same new day + channel.
        date_map = {"fwd-a": "2021-03-04T00:00:00Z", "fwd-b": "2021-03-04T18:00:00Z"}

        inserted = _rebucket_forward_day_records(conn, ["p1"], date_map)
        self.assertEqual(inserted, 1)

        rows = conn.execute(
            "SELECT occurred_at, amount_msat, fee_msat, channel_id, raw_json, first_seen_at, updated_at FROM lightning_node_records"
        ).fetchall()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["occurred_at"], "2021-03-04T00:00:00Z")
        self.assertEqual(rows[0]["amount_msat"], 30_000)
        self.assertEqual(rows[0]["fee_msat"], 12)
        self.assertEqual(rows[0]["channel_id"], "chan-1")
        self.assertEqual(json.loads(rows[0]["raw_json"])["forward_count"], 2)
        self.assertEqual(rows[0]["first_seen_at"], "2026-06-30T00:00:00Z")
        self.assertEqual(rows[0]["updated_at"], "2026-06-30T00:00:00Z")

    def test_distinct_days_stay_separate(self) -> None:
        conn = self._conn()
        self._insert_forward_day(conn, "fwd-a", "chan-1", 10_000, 5)
        self._insert_forward_day(conn, "fwd-b", "chan-1", 20_000, 7)
        conn.commit()
        date_map = {"fwd-a": "2021-03-04T00:00:00Z", "fwd-b": "2022-09-09T00:00:00Z"}

        inserted = _rebucket_forward_day_records(conn, ["p1"], date_map)
        self.assertEqual(inserted, 2)
        days = {
            row["occurred_at"]
            for row in conn.execute("SELECT occurred_at FROM lightning_node_records")
        }
        self.assertEqual(days, {"2021-03-04T00:00:00Z", "2022-09-09T00:00:00Z"})


class BackdateProfileScopeTests(unittest.TestCase):
    def _write_scenario(self, root: Path) -> Path:
        scenario = root / "scenario.json"
        scenario.write_text(
            json.dumps(
                {
                    "base_time": "2020-01-01T00:00:00Z",
                    "latest_time": "2020-12-31T23:59:59Z",
                }
            ),
            encoding="utf-8",
        )
        return scenario

    def _seed_book(self, data_root: Path) -> None:
        now = "2026-06-30T00:00:00Z"
        conn = open_db(data_root)
        try:
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', ?)",
                (now,),
            )
            for profile_id, profile_label, wallet_id in (
                ("p-main", "Main", "w-main"),
                ("p-other", "Other", "w-other"),
            ):
                conn.execute(
                    """
                    INSERT INTO profiles(id, workspace_id, label, created_at)
                    VALUES(?, 'ws', ?, ?)
                    """,
                    (profile_id, profile_label, now),
                )
                conn.execute(
                    """
                    INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
                    VALUES(?, 'ws', ?, ?, 'coreln', '{}', ?)
                    """,
                    (wallet_id, profile_id, f"Wallet {profile_label}", now),
                )
                conn.execute(
                    """
                    INSERT INTO transactions(
                        id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                        occurred_at, confirmed_at, direction, asset, amount, kind,
                        raw_json, payment_hash, created_at
                    ) VALUES(?, 'ws', ?, ?, ?, ?, ?, ?, 'inbound', 'BTC', 1000,
                             'cln_invoice', '{}', ?, ?)
                    """,
                    (
                        f"tx-{profile_id}",
                        profile_id,
                        wallet_id,
                        f"ext-{profile_id}",
                        f"fp-{profile_id}",
                        now,
                        now,
                        f"hash-{profile_id}",
                        now,
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO lightning_node_records(
                        id, workspace_id, profile_id, wallet_id, backend_name, node_id,
                        record_type, external_id, occurred_at, amount_msat, currency,
                        payment_hash, raw_json, first_seen_at, updated_at
                    ) VALUES(?, 'ws', ?, ?, 'cln', 'node', 'invoice', ?, ?,
                             1000, 'bc', ?, '{}', ?, ?)
                    """,
                    (
                        f"rec-{profile_id}",
                        profile_id,
                        wallet_id,
                        f"rec-ext-{profile_id}",
                        now,
                        f"hash-{profile_id}",
                        now,
                        now,
                    ),
                )
            conn.commit()
        finally:
            conn.close()

    def _dates(self, data_root: Path) -> dict[str, str]:
        conn = open_db(data_root)
        try:
            rows = conn.execute(
                """
                SELECT id, occurred_at FROM transactions
                UNION ALL
                SELECT id, occurred_at FROM lightning_node_records
                ORDER BY id
                """
            ).fetchall()
            return {str(row["id"]): str(row["occurred_at"]) for row in rows}
        finally:
            conn.close()

    def test_valid_labels_backdate_only_the_matching_profile(self) -> None:
        with tempfile.TemporaryDirectory(prefix="kassiber-ln-backdate-") as tmp:
            root = Path(tmp)
            data_root = root / "state"
            self._seed_book(data_root)
            scenario = self._write_scenario(root)

            summary = backdate_ln_records(
                data_root,
                str(scenario),
                workspace_label="Books",
                profile_label="Main",
                seed=1,
            )

            dates = self._dates(data_root)
            self.assertEqual(summary["transactions_backdated"], 1)
            self.assertEqual(summary["node_records_backdated"], 1)
            self.assertNotEqual(dates["tx-p-main"], "2026-06-30T00:00:00Z")
            self.assertNotEqual(dates["rec-p-main"], "2026-06-30T00:00:00Z")
            self.assertEqual(dates["tx-p-other"], "2026-06-30T00:00:00Z")
            self.assertEqual(dates["rec-p-other"], "2026-06-30T00:00:00Z")

    def test_unresolved_requested_scope_raises_without_rewriting_profiles(self) -> None:
        with tempfile.TemporaryDirectory(prefix="kassiber-ln-backdate-") as tmp:
            root = Path(tmp)
            data_root = root / "state"
            self._seed_book(data_root)
            scenario = self._write_scenario(root)
            before = self._dates(data_root)

            with self.assertRaisesRegex(ValueError, "No profile matched"):
                backdate_ln_records(
                    data_root,
                    str(scenario),
                    workspace_label="Books",
                    profile_label="Typo",
                    seed=1,
                )

            self.assertEqual(self._dates(data_root), before)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
