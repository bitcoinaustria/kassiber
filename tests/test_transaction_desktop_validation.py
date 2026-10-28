"""Invalid desktop sort/order containers fail through the typed error contract."""
import tempfile
import unittest

from kassiber.core.ui_snapshot import build_transactions_search_snapshot, build_transactions_snapshot
from kassiber.db import open_db
from kassiber.errors import AppError


class TransactionDesktopValidationTest(unittest.TestCase):
    def test_sort_and_order_containers_are_typed_validation_refusals(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-desktop-validation-") as root:
            conn = open_db(root)
            try:
                conn.executescript("""
                    INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
                    INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at)
                    VALUES('p','w','Book','EUR','at','2025');
                    INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');
                """)
                for builder in (build_transactions_snapshot, build_transactions_search_snapshot):
                    for key in ("sort", "order"):
                        for value in ([], {}):
                            with self.subTest(kind=builder.__name__, key=key, value=value):
                                with self.assertRaises(AppError) as raised:
                                    builder(conn, {"query": "%", key: value})
                                self.assertEqual(raised.exception.code, "validation")
                                self.assertEqual(raised.exception.details, {key: value})
            finally:
                conn.close()
