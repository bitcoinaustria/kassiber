"""User bounds are instants, including fractions between stored seconds."""
import tempfile
import unittest
from pathlib import Path

from kassiber.cli import handlers
from kassiber.core import metadata, rates, transaction_history, ui_snapshot
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.time_utils import timestamp_microseconds, timestamp_predicate


class TimestampFilterTest(unittest.TestCase):
    def setUp(self):
        root = tempfile.TemporaryDirectory(prefix="kassiber-time-filter-")
        self.addCleanup(root.cleanup)
        self.conn = open_db(Path(root.name) / "data")
        self.addCleanup(self.conn.close)
        self.conn.executescript("""
            INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
            INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at)
                VALUES('p','w','Book','EUR','at','2025');
            INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
                VALUES('wal','w','p','Cold','address','{}','2025');
            INSERT OR REPLACE INTO settings(key,value)
                VALUES('context_workspace','w'),('context_profile','p');
        """)
        for ident, stamp in (("before", "2025-02-01T00:00:00Z"),
                             ("after", "2025-02-01T00:00:01Z")):
            self.conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,
                fingerprint,asset,direction,amount,fee,occurred_at,created_at,kind)
                VALUES(?,'w','p','wal',?,'BTC','inbound',100000000,0,?,?,'deposit')""",
                (ident, ident, stamp, stamp))
            self.conn.execute("""INSERT INTO journal_entries(id,workspace_id,profile_id,
                transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,created_at)
                VALUES(?,'w','p',?,'wal',?,'acquisition','BTC',100000000,?)""",
                (ident, ident, stamp, stamp))
            self.conn.execute("""INSERT INTO transaction_edit_events(id,workspace_id,profile_id,
                transaction_id,wallet_id,source,changed_at)
                VALUES(?,'w','p',?,'wal','cli',?)""", (ident, ident, stamp))
            self.conn.execute("""INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at)
                VALUES('BTC-EUR',?,1,'manual',?)""", (stamp, stamp))
        self.conn.commit()

    def assert_boundaries(self, read):
        # Include sub-millisecond bounds: SQLite julianday/strftime round these.
        for bound in ("2025-02-01T00:00:00.5Z", "2025-02-01T00:00:00.000001Z",
                      "2025-02-01T00:00:00.999999Z", "2025-02-01T01:00:00.5+01:00"):
            with self.subTest(bound=bound):
                self.assertEqual(read(start=bound), ["after"])
                self.assertEqual(read(end=bound), ["before"])
        self.assertEqual(read(start="2025-02-01T00:00:00.000000Z",
                              end="2025-02-01T00:00:00Z"), ["before"])

    def test_observed_transaction_bounds_are_instants(self):
        self.assert_boundaries(lambda **kw: [r["id"] for r in
            handlers.list_transactions(self.conn, "w", "p", **kw)[0]])

    def test_metadata_record_bounds_are_instants(self):
        self.assert_boundaries(lambda **kw: [r["transaction_id"] for r in metadata.list_transaction_records(
            self.conn, "w", "p", handlers._metadata_hooks(), **kw)["records"]])

    def test_history_bounds_are_instants(self):
        self.assert_boundaries(lambda **kw: [r["id"] for r in transaction_history.list_history(
            self.conn, "w", "p", handlers._metadata_hooks(), **kw)["events"]])

    def test_journal_list_bounds_are_instants(self):
        self.assert_boundaries(lambda **kw: [r["id"] for r in
            handlers.list_journal_events(self.conn, "w", "p", **kw)["events"]])

    def test_rate_range_and_asof_bounds_are_instants(self):
        def read(**kw):
            return ["before" if r["timestamp"].endswith("00Z") else "after"
                    for r in rates.get_rate_range(self.conn, "BTC-EUR", **kw)]
        self.assert_boundaries(read)
        self.assertEqual(rates.get_cached_rate_at_or_before(
            self.conn, "BTC-EUR", "2025-02-01T00:00:00.000001Z")["timestamp"],
            "2025-02-01T00:00:00Z")

    def test_desktop_list_and_review_queue_bounds_are_instants(self):
        self.assert_boundaries(lambda **kw: [r["id"] for r in
            ui_snapshot.build_transactions_snapshot(self.conn, kw)["txs"]])
        for name, expected in (("since", "after"), ("until", "before")):
            clauses, params = ui_snapshot._review_queue_leg_filters(
                self.conn, "p", **{name: "2025-02-01T00:00:00.5Z"})
            rows = self.conn.execute("SELECT t.id FROM transactions t JOIN wallets w "
                                    "ON w.id=t.wallet_id WHERE " + " AND ".join(clauses), params)
            self.assertEqual([r[0] for r in rows], [expected])

    def test_dashboard_validates_and_filters_fractional_windows_as_instants(self):
        args = {"period": "all", "since": "2025-02-01T00:00:00Z",
                "until": "2025-02-01T00:00:00.5Z"}
        result = ui_snapshot.build_transactions_dashboard_snapshot(self.conn, args)
        self.assertEqual(result["counts"]["all"], 1)
        with self.assertRaises(AppError) as error:
            ui_snapshot.build_transactions_dashboard_snapshot(
                self.conn, {**args, "since": args["until"], "until": args["since"]})
        self.assertEqual(error.exception.code, "validation")

    def test_audit_changes_since_counts_instants(self):
        result = ui_snapshot.build_audit_changes_since_last_answer_snapshot(
            self.conn, {"since": "2025-02-01T00:00:00.5Z"})
        self.assertEqual(result["counts_since"]["transactions"], 1)
        self.assertEqual(result["counts_since"]["transaction_edit_events"], 1)
        self.assertEqual(result["counts_since"]["rates"], 1)

    def test_instant_key_preserves_microseconds_and_excludes_unknown_values(self):
        base = timestamp_microseconds("2025-02-01T00:00:00Z")
        self.assertEqual(timestamp_microseconds("2025-02-01T01:00:00.000001+01:00"), base + 1)
        for raw in (None, "", "bad", "0001-01-01T00:00:00+01:00"):
            self.assertIsNone(timestamp_microseconds(raw))

    def test_repeated_filtered_reads_allow_an_active_cursor(self):
        predicate = timestamp_predicate(self.conn, "occurred_at", ">=")
        active = self.conn.execute(f"SELECT id FROM transactions WHERE {predicate}",
                                   ("2025-02-01",))
        self.addCleanup(active.close)
        self.assertIsNotNone(active.fetchone())
        rows, _ = handlers.list_transactions(self.conn, "w", "p", start="2025-02-01")
        self.assertEqual(len(rows), 2)
