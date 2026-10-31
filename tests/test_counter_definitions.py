"""Every counter a health, badge or quarantine payload reports says what it counts."""

import tempfile
import unittest

from kassiber.cli import handlers
from kassiber.core.ui_snapshot import (
    build_journals_quarantine_snapshot,
    build_review_badges_snapshot,
    build_workspace_health_snapshot,
)
from kassiber.db import set_setting
from tests.test_custody_quantity_handler import _book


class CounterDefinitionsTest(unittest.TestCase):
    def test_each_counter_says_what_it_counts(self):
        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            self.addCleanup(conn.close)
            set_setting(conn, "context_workspace", "ws")
            set_setting(conn, "context_profile", "profile")
            conn.commit()
            handlers.process_journals(conn, "Books", "Book")
            health = build_workspace_health_snapshot(conn)
            badges = build_review_badges_snapshot(conn)
            quarantine = build_journals_quarantine_snapshot(conn, {})

        self.assertTrue(set(health["counts"]) | {"review_queue"} <= set(health["definitions"]))
        self.assertEqual(
            set(badges["definitions"]),
            {key for key in badges if key != "definitions"},
        )
        for key in ("count", "attention_count", "waiting_count", "blocking_count"):
            self.assertIn(key, quarantine["summary"]["definitions"])
        for definitions in (health["definitions"], badges["definitions"]):
            for text in definitions.values():
                self.assertLess(len(text), 220)


if __name__ == "__main__":
    unittest.main()
