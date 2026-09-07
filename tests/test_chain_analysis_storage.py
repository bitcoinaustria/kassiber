import base64
import json
import unittest

from kassiber.core.chain_analysis import build_index
from kassiber.core import chain_analysis_cases as cases
from kassiber.errors import AppError
from tests import test_privacy_hygiene as hygiene_fixture

NOW = hygiene_fixture.NOW


class ChainAnalysisStorageTests(unittest.TestCase):
    def setUp(self):
        self.book = hygiene_fixture.PrivacyHygieneTests()
        self.book.setUp()
        self.conn = self.book.conn
        self.book._insert_transaction(tx_id="tx", external_id="a" * 64, raw_json={"vin": [{"coinbase": "0101"}], "vout": [{"value": 1000}]})
        self.conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,created_at) VALUES('other','ws','Other','EUR','generic',365,'FIFO',?)", (NOW,))

    def tearDown(self):
        self.book.tearDown()

    def save(self, title="Investigation"):
        return cases.save_case(self.conn, "pf", {"title": title, "query": {}, "expected_snapshot_id": build_index(self.conn, "pf").snapshot_id})

    def label(self, **kwargs):
        return {"subject": "a" * 64, "chain": "bitcoin", "network": "main", "label": "Counterparty", "category": "exchange", "source": "Reviewed statement", "confidence": "user_confirmed", **kwargs}

    def test_case_is_frozen_compare_tracks_changes_and_other_book_cannot_read(self):
        saved = self.save()
        frozen = json.dumps(saved["result"], sort_keys=True)
        self.book._insert_transaction(tx_id="second", external_id="b" * 64, raw_json={"vin": [{"txid": "a" * 64, "vout": 0}], "vout": [{"value": 900}]})
        self.assertEqual(json.dumps(cases.get_case(self.conn, "pf", saved["id"])["result"], sort_keys=True), frozen)
        compared = cases.compare_case(self.conn, "pf", {"id": saved["id"]})
        self.assertTrue(compared["changed"])
        self.assertTrue(compared["added_nodes"])
        with self.assertRaises(AppError) as caught:
            cases.get_case(self.conn, "other", saved["id"])
        self.assertEqual(caught.exception.code, "not_found")
        self.assertEqual(cases.list_cases(self.conn, "other", {})["items"], [])
        self.assertFalse(cases.compare_case(self.conn, "pf", {"id": saved["id"], "other_id": saved["id"]})["changed"])

    def test_stale_save_refuses_all_writes(self):
        snapshot = build_index(self.conn, "pf").snapshot_id
        cases.upsert_label(self.conn, "pf", self.label())
        with self.assertRaises(AppError) as caught:
            cases.save_case(self.conn, "pf", {"title": "Stale", "query": {}, "expected_snapshot_id": snapshot})
        self.assertEqual(caught.exception.code, "chain_analysis_stale")
        self.assertEqual(self.conn.execute("SELECT count(*) FROM chain_analysis_cases").fetchone()[0], 0)

    def test_case_integrity_and_profile_bound_pagination(self):
        saved = [self.save(str(i)) for i in range(3)]
        first = cases.list_cases(self.conn, "pf", {"limit": 2})
        second = cases.list_cases(self.conn, "pf", {"limit": 2, "cursor": first["next_cursor"]})
        self.assertEqual({row["id"] for row in first["items"] + second["items"]}, {row["id"] for row in saved})
        for cursor in (first["next_cursor"], base64.urlsafe_b64encode(b'{"0":"pf","1":"x","2":"y"}').decode()):
            with self.subTest(cursor=cursor), self.assertRaises(AppError):
                cases.list_cases(self.conn, "other", {"cursor": cursor})
        self.conn.execute("UPDATE chain_analysis_cases SET result_json='{}' WHERE id=?", (saved[0]["id"],))
        with self.assertRaises(AppError) as caught:
            cases.get_case(self.conn, "pf", saved[0]["id"])
        self.assertEqual(caught.exception.code, "integrity_error")

    def test_revisioned_label_history_and_delete_are_scope_bound(self):
        first = cases.upsert_label(self.conn, "pf", self.label())
        with self.assertRaises(AppError):
            cases.upsert_label(self.conn, "other", self.label(id=first["id"], expected_revision=1))
        second = cases.upsert_label(self.conn, "pf", self.label(id=first["id"], expected_revision=1, label="Corrected"))
        self.assertEqual(second["revision"], 2)
        with self.assertRaises(AppError) as caught:
            cases.delete_label(self.conn, "pf", {"id": first["id"], "expected_revision": 1})
        self.assertEqual(caught.exception.code, "chain_analysis_stale")
        deleted = cases.delete_label(self.conn, "pf", {"id": first["id"], "expected_revision": 2})
        self.assertEqual(deleted["revision"], 3)
        self.assertTrue(deleted["deleted"])
        self.assertEqual(cases.list_labels(self.conn, "pf")["items"], [])
        history = self.conn.execute("SELECT revision,payload_json FROM chain_analysis_label_history ORDER BY revision").fetchall()
        self.assertEqual([row["revision"] for row in history], [1, 2, 3])
        self.assertEqual(json.loads(history[0]["payload_json"])["label"], "Counterparty")

    def test_import_failure_is_atomic_including_history(self):
        with self.assertRaises(AppError):
            cases.import_labels(self.conn, "pf", {"items": [self.label(), self.label(chain="unsupported")]})
        self.assertEqual(self.conn.execute("SELECT count(*) FROM chain_analysis_labels").fetchone()[0], 0)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM chain_analysis_label_history").fetchone()[0], 0)
        imported = cases.import_labels(self.conn, "pf", {"items": [self.label(), self.label(subject="b" * 64)]})
        self.assertEqual(imported["imported_count"], 2)

    def test_label_subject_validation_does_not_accept_secrets_or_wrong_domains(self):
        for args in (self.label(subject="wpkh(xprv-secret)"), self.label(chain=[]), self.label(subject="bitcoin:regtest:tx:" + "a" * 64), self.label(subject="1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa", network="test")):
            with self.subTest(args=args), self.assertRaises(AppError):
                cases.upsert_label(self.conn, "pf", args)
        liquid = cases.upsert_label(self.conn, "pf", self.label(chain="liquid", subject="liquid:liquidv1:tx:" + "a" * 64))
        self.assertEqual(liquid["network"], "main")


if __name__ == "__main__":
    unittest.main()
