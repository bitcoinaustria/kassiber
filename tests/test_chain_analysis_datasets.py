import hashlib
import io
import json
import time
import unittest
import uuid
from unittest.mock import patch

from kassiber.core import chain_analysis_datasets as datasets
from kassiber.core.maintenance import reset_current_profile_data
from kassiber.errors import AppError
from tests import test_privacy_hygiene as fixture


ADDRESS = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
SCRIPT = "76a91462e907b15cbf27d5425399ebf6f0fb50ebb88f1888ac"
TXID = "a" * 64


def manifest(**changes):
    return {"dataset_key": "local-sources", "name": "Local research", "version": "2026-01", "chain": "bitcoin", "network": "main", "source": "Public source", "license": "LicenseRef-Unverified", "attribution_method": "published_claim", "visibility": "public", **changes}


def binary(*rows):
    return io.BytesIO(("\n".join(json.dumps(row) for row in rows) + "\n").encode())


class GeneratedCSV:
    """Non-seekable generated input: tests must not materialize large packs."""
    def __init__(self, count):
        self.count, self.position, self.largest_request = count, -1, 0

    def readline(self, size):
        self.largest_request = max(self.largest_request, size)
        self.position += 1
        if self.position == 0:
            return b"subject,entity,source\n"
        if self.position > self.count:
            return b""
        return f"{self.position:064x},Entity {self.position % 13},Source\n".encode()


class ChainAnalysisDatasetTests(unittest.TestCase):
    def setUp(self):
        self.book = fixture.PrivacyHygieneTests()
        self.book.setUp()
        self.conn = self.book.conn
        self.conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,created_at) VALUES('other','ws','Other','EUR','generic',365,'FIFO',?)", (fixture.NOW,))
        self.conn.commit()

    def tearDown(self):
        self.book.tearDown()

    def pack(self, *, changes=None, rows=None, **kwargs):
        return datasets.import_dataset(self.conn, "pf", manifest(**(changes or {})), binary(*(rows or [{"subject": TXID, "label": "Exchange"}])), format="jsonl", **kwargs)

    def match(self, subject=TXID, **kwargs):
        return datasets.match_subjects(self.conn, "pf", [("bitcoin", "main", subject)], **kwargs)

    def test_preview_and_import_bind_bytes_and_preserve_csv_quotes_and_conflicts(self):
        data = f'\ufeffaddress,entity,source,source_id\r\n{ADDRESS},"Example, Inc.",Source A,row-a\r\n{ADDRESS},Conflicting label,Source B,row-b\r\n'.encode()
        preview = datasets.preview_dataset(manifest(), io.BytesIO(data), adapter="am_i_exposed")
        self.assertEqual(preview["row_count"], 2)
        self.assertEqual(preview["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(datasets.list_datasets(self.conn, "pf")["items"], [])
        pack = datasets.import_dataset(self.conn, "pf", manifest(), io.BytesIO(data), adapter="am_i_exposed", expected_sha256=preview["sha256"])
        matched = self.match("script:" + SCRIPT)
        self.assertEqual({row["label"] for row in matched["claims"]}, {"Example, Inc.", "Conflicting label"})
        self.assertEqual({row["source_record"]["source_id"] for row in matched["claims"]}, {"row-a", "row-b"})
        self.assertEqual({row["source"] for row in matched["claims"]}, {"Source A", "Source B"})
        self.assertTrue(all(not row["cluster_defining"] and not row["accounting_authority"] and row["confidence"] == "imported" for row in matched["claims"]))
        self.assertEqual(matched["datasets"][0]["id"], pack["id"])
        self.assertEqual(pack["content_sha256"], preview["sha256"])
        self.assertFalse(self.conn.in_transaction)

    def test_maru92_adapter_preserves_historical_method_date_and_category(self):
        data = f"hashAdd,date_first_tx,exchange,add_type,add_num\n{ADDRESS},2012-03-04,example.com,5,987\n".encode()
        pack = datasets.import_dataset(self.conn, "pf", manifest(attribution_method="common_input_heuristic", observed_at="2018-04-01"), io.BytesIO(data), adapter="maru92")
        match = self.match(ADDRESS)["claims"][0]
        self.assertEqual(match["attribution_method"], "common_input_heuristic")
        self.assertEqual(match["source_record"]["date_first_tx"], "2012-03-04T00:00:00Z")
        self.assertEqual(match["source_record"]["original_category"], "exchange")
        self.assertEqual(pack["manifest"]["observed_at"], "2018-04-01T00:00:00Z")
        # Extra arbitrary data is never persisted as a raw row blob.
        self.assertNotIn("add_num", match["source_record"])

    def test_jsonl_aliases_and_noncanonical_subjects(self):
        self.pack(rows=[{"hashAdd": ADDRESS, "entity": "Listed", "private_key": "DO-NOT-STORE", "source_id": "entry-1"}])
        self.assertEqual(self.match(ADDRESS)["claims"][0]["source_record"]["original_subject"], ADDRESS)
        self.assertNotIn("DO-NOT-STORE", self.conn.execute("SELECT source_record_json FROM chain_analysis_dataset_claims").fetchone()[0])
        self.assertEqual(datasets.canonical_subject("bitcoin", "testnet", "bitcoin:test:tx:" + TXID), ("bitcoin", "test", "tx:" + TXID))
        self.assertEqual(datasets.canonical_subject("liquid", "main", "liquid:liquidv1:out:" + TXID + ":2"), ("liquid", "liquidv1", "out:" + TXID + ":2"))
        for value in ("tx:out:" + TXID, "out:" + TXID, TXID + ":4294967296", "bitcoin:test:tx:" + TXID, "wpkh(xprv-secret)", "script:xz"):
            with self.subTest(subject=value), self.assertRaises(AppError):
                datasets.canonical_subject("bitcoin", "main", value)
        with self.assertRaises(AppError):
            datasets.canonical_subject("bitcoin", "test", ADDRESS)

    def test_large_output_scripts_preserve_graph_and_exact_attribution(self):
        from kassiber.core.chain_analysis.index import build_index
        # A data output can exceed the old attribution pack's 1,000-byte
        # limit. Loading physical evidence must work even without any packs.
        for size in (1101, datasets.MAX_SCRIPT_BYTES, datasets.MAX_SCRIPT_BYTES + 1):
            with self.subTest(size=size):
                script = "6a" + "00" * (size - 1)
                self.conn.execute("DELETE FROM transactions")
                self.book._insert_transaction(tx_id="large", external_id=TXID, raw_json={
                    "txid": TXID, "vin": [{"coinbase": "0101"}],
                    "vout": [{"value": 0, "scriptpubkey": script}],
                })
                self.conn.commit()
                index = build_index(self.conn, "pf")
                self.assertTrue(index.nodes)
                self.assertEqual(index.coverage["datasets"]["scripts_skipped"], int(size > datasets.MAX_SCRIPT_BYTES))
                if size <= datasets.MAX_SCRIPT_BYTES:
                    self.pack(changes={"dataset_key": f"large-{size}"}, rows=[{"subject": "script:" + script, "label": "Published data output"}])
                    index = build_index(self.conn, "pf")
                    self.assertTrue(any(row["subject"] == "script:" + script for row in index.labels))
                else:
                    self.pack(changes={"dataset_key": "large-outpoint"}, rows=[{"subject": TXID + ":0", "label": "Known output"}])
                    index = build_index(self.conn, "pf")
                    self.assertTrue(any(row["subject"] == "out:" + TXID + ":0" for row in index.labels))

    def test_liquid_confidential_and_unconfidential_addresses_share_script_only_in_domain(self):
        from embit import ec, script
        from embit.liquid.addresses import address
        from embit.liquid.networks import NETWORKS
        sc = script.Script(bytes.fromhex("0014" + "45" * 20))
        blind = ec.PrivateKey(bytes.fromhex("12" * 32)).get_public_key()
        for network in ("liquidv1", "liquidtestnet", "elementsregtest"):
            with self.subTest(network=network):
                plain = address(sc, network=NETWORKS[network])
                confidential = address(sc, blind, network=NETWORKS[network])
                a = datasets.canonical_subject("liquid", network, plain)
                self.assertEqual(a, datasets.canonical_subject("liquid", network, confidential))
                self.assertEqual(a, ("liquid", network, "script:" + sc.data.hex()))
                wrong = "liquidv1" if network != "liquidv1" else "liquidtestnet"
                with self.assertRaises(AppError):
                    datasets.canonical_subject("liquid", wrong, confidential)

    def test_profile_network_and_visibility_isolation_with_no_match_manifest_binding(self):
        public = self.pack()
        hidden = self.pack(changes={"dataset_key": "private", "visibility": "private"})
        self.pack(changes={"dataset_key": "testnet", "network": "test"})
        owner = self.match()
        self.assertEqual({row["dataset_id"] for row in owner["claims"]}, {public["id"], hidden["id"]})
        public_view = self.match(observer="public")
        self.assertEqual([row["dataset_id"] for row in public_view["claims"]], [public["id"]])
        self.assertNotIn(hidden["id"], json.dumps(public_view))
        other = datasets.match_subjects(self.conn, "other", [("bitcoin", "main", TXID)])
        self.assertEqual(other["claims"], [])

        self.assertEqual(other["datasets"], [])
        empty = self.match("b" * 64)
        self.assertEqual(empty["claims"], [])
        self.assertEqual(len(empty["datasets"]), 3)
        with self.assertRaises(AppError):
            datasets.revoke_dataset(self.conn, "other", public["id"], 1)
        datasets.revoke_dataset(self.conn, "pf", public["id"], 1)
        self.assertNotEqual(self.match("b" * 64)["dataset_state_digest"], empty["dataset_state_digest"])

    def test_public_snapshot_handoff_uses_same_commitment_with_private_packs(self):
        from kassiber.core.chain_analysis import analyze_snapshot, build_index, prepare_entropy, run_analysis
        from kassiber.core.chain_analysis.projection import read_index
        self.book._insert_transaction(tx_id="snapshot", external_id=TXID, raw_json={
            "txid": TXID, "vin": [{"coinbase": "0101"}], "vout": [{"value": 1000, "scriptpubkey": SCRIPT}],
        })
        self.conn.commit()
        self.pack(changes={"dataset_key": "public-claim"}, rows=[{"subject": TXID, "label": "Public claim"}])
        self.pack(changes={"dataset_key": "private-claim", "visibility": "private"}, rows=[{"subject": TXID, "label": "PRIVATE ANCHOR"}])
        args = {"observer": "public", "include_hypotheses": True}
        with read_index(self.conn, "pf") as index:
            mirror = analyze_snapshot(index, args)
        workbench = run_analysis(self.conn, "pf", args)
        entropy_context, _, _ = prepare_entropy(self.conn, "pf", {"subject": TXID, "observer": "public"})
        self.assertEqual(mirror, workbench)
        self.assertEqual(mirror["snapshot_id"], entropy_context["snapshot_id"])
        self.assertNotIn("PRIVATE ANCHOR", json.dumps(mirror))
        self.assertIn("Public claim", json.dumps(mirror))
    def test_replacement_is_atomic_and_old_versions_are_immutable(self):
        old = self.pack()
        seen = []
        def progress(event):
            self.assertFalse(self.conn.in_transaction)
            seen.append([row["dataset_id"] for row in self.match()["claims"]])
        new = self.pack(changes={"version": "v2", "expected_active_id": old["id"]}, rows=[{"subject": TXID, "label": "Corrected"}], progress=progress)
        self.assertEqual(seen, [[old["id"]]])
        self.assertEqual(self.match()["claims"][0]["label"], "Corrected")
        self.assertEqual(datasets.get_dataset(self.conn, "pf", old["id"])["status"], "superseded")
        self.assertEqual(self.conn.execute("SELECT label FROM chain_analysis_dataset_claims WHERE dataset_id=?", (old["id"],)).fetchone()[0], "Exchange")
        with self.assertRaises(AppError):
            self.pack(changes={"version": "2026-01", "expected_active_id": new["id"]})
        with self.assertRaises(AppError):
            self.pack(changes={"version": "v3", "expected_active_id": old["id"]})
        with self.assertRaises(AppError):
            datasets.revoke_dataset(self.conn, "pf", old["id"], 1)
        self.assertEqual(datasets.revoke_dataset(self.conn, "pf", new["id"], 1)["status"], "revoked")
        self.assertEqual(self.match()["claims"], [])

    def test_hash_mismatch_invalid_rows_and_cancellation_never_activate_partial_pack(self):
        old = self.pack()
        changes = {"version": "v2", "expected_active_id": old["id"]}
        with self.assertRaises(AppError) as caught:
            self.pack(changes=changes, expected_sha256="0" * 64)
        self.assertEqual(caught.exception.code, "chain_analysis_stale")
        with patch.object(datasets, "BATCH_SIZE", 1), self.assertRaises(AppError):
            self.pack(changes=changes, rows=[{"subject": TXID, "label": "First"}, {"subject": "malformed", "label": "Bad"}])
        def cancel(_):
            raise InterruptedError("Cancelled")
        with self.assertRaises(InterruptedError):
            self.pack(changes=changes, progress=cancel)
        self.assertEqual([row["dataset_id"] for row in self.match()["claims"]], [old["id"]])
        failed = [row for row in datasets.list_datasets(self.conn, "pf")["items"] if row["status"] == "failed"]
        self.assertEqual(len(failed), 3)
        for row in failed:
            datasets.discard_dataset(self.conn, "pf", row["id"])
        self.assertEqual(len(datasets.list_datasets(self.conn, "pf")["items"]), 1)
        with self.assertRaises(AppError):
            datasets.discard_dataset(self.conn, "pf", old["id"])

    def test_concurrent_replacement_revalidated_at_activation(self):
        old = self.pack()
        second = None
        def replace(_):
            nonlocal second
            second = self.pack(changes={"version": "v3", "expected_active_id": old["id"]})
        with self.assertRaises(AppError) as caught:
            self.pack(changes={"version": "v2", "expected_active_id": old["id"]}, progress=replace)
        self.assertEqual(caught.exception.code, "chain_analysis_stale")
        self.assertEqual(self.match()["claims"][0]["dataset_id"], second["id"])

    def test_pack_and_record_validity_intersection(self):
        self.pack(changes={"valid_from": "2020-01-01", "valid_until": "2030-01-01"}, rows=[{"subject": TXID, "label": "Earlier", "valid_until": "2025-01-01"}, {"subject": TXID, "label": "Later", "valid_from": "2025-01-01"}])
        self.assertEqual(self.match(as_of="2019-12-31")["claims"], [])
        self.assertEqual([row["label"] for row in self.match(as_of="2024-12-31")["claims"]], ["Earlier"])
        self.assertEqual([row["label"] for row in self.match(as_of="2025-01-01")["claims"]], ["Later"])
        self.assertEqual(self.match(as_of="2030-01-01")["claims"], [])

    def test_malformed_inputs_are_bounded_and_do_not_expose_values_in_errors(self):
        for content in (b"address,ADDRESS\nx,y\n", b"subject,label\nx,y,z\n", b'subject,label\nx,"unterminated\n', b"subject,label\n\xff,label\n", b"subject,label\n", b"x" * (datasets.MAX_RECORD_BYTES + 1)):
            with self.subTest(content=content[:30]), self.assertRaises(AppError):
                datasets.preview_dataset(manifest(), io.BytesIO(content))
        for content in (b'{"subject":"x","Subject":"y","label":"label"}\n', b'[]\n', b'{"subject":"bad-secret-phrase","label":"label"}\n'):
            with self.subTest(content=content), self.assertRaises(AppError) as caught:
                datasets.preview_dataset(manifest(), io.BytesIO(content), format="jsonl")
            self.assertNotIn("bad-secret-phrase", str(caught.exception))
        for changes in ({"network": "wrong"}, {"visibility": "unknown"}, {"source_url": "https://secret@example.org/"}, {"source_url": "https://example.org/?token=secret"}, {"valid_from": "2026-01-01", "valid_until": "2020-01-01"}, {"unknown": "value"}):
            with self.subTest(changes=changes), self.assertRaises(AppError):
                self.pack(changes=changes)
        with self.assertRaises(AppError):
            datasets.preview_dataset(manifest(), io.StringIO("subject,label\n"))

    def test_import_refuses_to_commit_caller_transaction(self):
        self.conn.execute("UPDATE profiles SET label='Pending' WHERE id='pf'")
        with self.assertRaises(AppError) as caught:
            self.pack()
        self.assertEqual(caught.exception.code, "chain_analysis_dataset_busy")
        self.assertTrue(self.conn.in_transaction)
        self.conn.rollback()
        self.assertEqual(datasets.list_datasets(self.conn, "pf")["items"], [])

    def test_pagination_scope_and_profile_reset_cascade(self):
        ids = {self.pack(changes={"dataset_key": str(number)})["id"] for number in range(3)}
        other = datasets.import_dataset(self.conn, "other", manifest(), binary({"subject": TXID, "label": "Other"}), format="jsonl")
        page1 = datasets.list_datasets(self.conn, "pf", {"limit": 2})
        page2 = datasets.list_datasets(self.conn, "pf", {"limit": 2, "cursor": page1["next_cursor"]})
        self.assertEqual({row["id"] for row in page1["items"] + page2["items"]}, ids)
        with self.assertRaises(AppError):
            datasets.list_datasets(self.conn, "other", {"cursor": page1["next_cursor"]})
        result = reset_current_profile_data(self.conn, self.book._tmp.name)
        self.assertEqual(result["removed"]["chain_analysis_datasets"], 3)
        self.assertEqual(result["removed"]["chain_analysis_dataset_claims"], 3)
        self.assertEqual(datasets.list_datasets(self.conn, "pf")["items"], [])
        self.assertEqual(datasets.get_dataset(self.conn, "other", other["id"])["status"], "active")
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM chain_analysis_dataset_claims").fetchone()[0], 1)

    def test_large_stream_uses_bounded_batches_and_indexed_observed_subject_lookup(self):
        source = GeneratedCSV(100_000)
        batches = []
        started = time.monotonic()
        pack = datasets.import_dataset(self.conn, "pf", manifest(), source, progress=lambda event: batches.append(event["row_count"]))
        self.assertEqual(pack["row_count"], 100_000)
        self.assertEqual(len(batches), 50)
        self.assertEqual(source.largest_request, datasets.MAX_RECORD_BYTES + 1)
        queries = []
        self.conn.set_trace_callback(queries.append)
        result = datasets.match_subjects(self.conn, "pf", [("bitcoin", "main", f"{value:064x}") for value in (1, 50_000, 100_000)])
        self.conn.set_trace_callback(None)
        self.assertEqual(len(result["claims"]), 3)
        self.assertEqual(result["coverage"]["lookup_queries"], 1)
        plan = self.conn.execute("EXPLAIN QUERY PLAN SELECT * FROM chain_analysis_dataset_claims INDEXED BY idx_chain_analysis_dataset_subject WHERE dataset_id=? AND subject IN (?,?)", (pack["id"], "tx:" + f"{1:064x}", "tx:" + f"{100_000:064x}")).fetchall()
        self.assertTrue(any("idx_chain_analysis_dataset_subject" in row[3] and "SEARCH" in row[3] for row in plan), plan)
        self.assertFalse(any("SCAN chain_analysis_dataset_claims" in row[3] for row in plan))
        self.assertEqual(sum("SELECT * FROM chain_analysis_dataset_claims" in query for query in queries), 1)
        self.assertLess(time.monotonic() - started, 60, "The 100k streaming fixture should finish within a generous regression bound")

    def test_match_limit_keeps_conflicting_claims_and_reports_truncation(self):
        self.pack(rows=[{"subject": TXID, "label": str(value)} for value in range(5)])
        result = self.match(limit=2)
        self.assertEqual(len(result["claims"]), 2)
        self.assertTrue(result["coverage"]["truncated"])
        self.assertEqual(len(self.match(limit=5)["claims"]), 5)
        self.assertFalse(self.match(limit=5)["coverage"]["truncated"])

    def test_private_match_budget_cannot_starve_public_snapshot_or_mirror(self):
        from kassiber.core.chain_analysis import analyze_snapshot, build_index, run_analysis
        from kassiber.core.chain_analysis_ai import project_ai_result
        from kassiber.core.privacy_mirror import build_privacy_mirror
        self.book._insert_transaction(tx_id="visible", external_id=TXID, raw_json={
            "txid": TXID, "vin": [{"coinbase": "0101"}], "vout": [{"value": 1000, "scriptpubkey": SCRIPT}],
        })
        self.book._insert_utxo(utxo_id="visible", txid=TXID, vout=0, sats=1000, address=ADDRESS, script=SCRIPT)
        self.conn.commit()
        with patch.object(datasets.uuid, "uuid4", return_value=uuid.UUID(int=1)):
            self.pack(changes={"dataset_key": "private-many", "visibility": "private"}, rows=[{"subject": TXID, "label": f"Private claim {number}"} for number in range(5001)])
        with patch.object(datasets.uuid, "uuid4", return_value=uuid.UUID(int=2)):
            self.pack(changes={"dataset_key": "public-one"}, rows=[{"subject": TXID, "label": "Public claim"}])
        index = build_index(self.conn, "pf")
        self.assertTrue(index.coverage["datasets"]["visibility_coverage"]["private"]["truncated"])
        self.assertEqual(index.coverage["datasets"]["visibility_coverage"]["private"]["match_count"], 5000)
        args = {"observer": "public", "include_hypotheses": True}
        from kassiber.core.chain_analysis.projection import read_index
        with read_index(self.conn, "pf") as current:
            snapshot = analyze_snapshot(current, args)
            snapshot_id = current.snapshot_id
        workbench = run_analysis(self.conn, "pf", args)
        self.assertEqual(snapshot, workbench)
        self.assertEqual(snapshot["snapshot_id"], snapshot_id)
        self.assertEqual(snapshot["coverage"]["analytics"]["label_count"], 1)
        self.assertEqual(snapshot["coverage"]["datasets"]["match_count"], 1)
        self.assertEqual(snapshot["coverage"]["datasets"]["active_dataset_count"], 1)
        self.assertFalse(snapshot["coverage"]["datasets"]["truncated"])
        self.assertNotIn("visibility_coverage", snapshot["coverage"]["datasets"])
        self.assertTrue(any(row["claim"]["label"] == "Public claim" for row in snapshot["exposure"]))
        projected = project_ai_result(self.conn, "pf", snapshot)
        self.assertEqual(projected["coverage"]["analytics"]["label_count"], 1)
        mirror = build_privacy_mirror(self.conn, "pf", redacted=False)
        check = next(row for row in mirror["coverage"]["checks"] if row["code"] == "public_attribution")
        self.assertEqual((check["evaluated"], check["eligible"]), (1, 1))
        self.assertNotEqual(check.get("reason"), "no_local_public_claims")
        self.assertEqual(mirror["investigation"]["snapshot_id"], snapshot_id)

    def test_public_and_private_match_limits_are_independent_in_owner_view(self):
        self.pack(rows=[{"subject": TXID, "label": f"Public {number}"} for number in range(3)])
        self.pack(changes={"dataset_key": "private", "visibility": "private"}, rows=[{"subject": TXID, "label": "Private"}])
        result = self.match(limit=2)
        self.assertEqual(len(result["claims"]), 3)
        self.assertEqual(sum(row["visibility"] == "public" for row in result["claims"]), 2)
        self.assertEqual(sum(row["visibility"] == "private" for row in result["claims"]), 1)
        self.assertTrue(result["coverage"]["visibility_coverage"]["public"]["truncated"])
        self.assertFalse(result["coverage"]["visibility_coverage"]["private"]["truncated"])

    def test_exact_entity_and_subject_query_pages_are_scope_bound_and_historical(self):
        rows = [{"subject": f"{value:064x}", "label": "Same Entity", "source_id": str(value)} for value in range(1, 5)]
        first = self.pack(rows=rows)
        self.pack(changes={"dataset_key": "second"}, rows=[{"subject": f"{1:064x}", "label": "Disagreement"}])
        args = {"dataset_id": first["id"], "label": "Same Entity", "limit": 2}
        page1 = datasets.query_claims(self.conn, "pf", args)
        page2 = datasets.query_claims(self.conn, "pf", {**args, "cursor": page1["next_cursor"]})
        self.assertEqual([row["source_record"]["source_id"] for row in page1["items"] + page2["items"]], ["1", "2", "3", "4"])
        self.assertIsNone(page2["next_cursor"])
        self.assertEqual(datasets.query_claims(self.conn, "pf", {**args, "label": "Same"})["items"], [])
        subject = {"subject": f"{1:064x}", "chain": "bitcoin", "network": "main", "limit": 1}
        matches1 = datasets.query_claims(self.conn, "pf", subject)
        matches2 = datasets.query_claims(self.conn, "pf", {**subject, "cursor": matches1["next_cursor"]})
        self.assertEqual({row["label"] for row in matches1["items"] + matches2["items"]}, {"Same Entity", "Disagreement"})
        self.assertEqual(datasets.query_claims(self.conn, "other", subject)["items"], [])
        with self.assertRaises(AppError):
            datasets.query_claims(self.conn, "pf", {**args, "label": "Different", "cursor": page1["next_cursor"]})
        datasets.revoke_dataset(self.conn, "pf", first["id"], 1)
        with self.assertRaises(AppError):
            datasets.query_claims(self.conn, "pf", {**args, "cursor": page1["next_cursor"]})
        historical = datasets.query_claims(self.conn, "pf", args)
        self.assertTrue(all(row["dataset_status"] == "revoked" for row in historical["items"]))
        self.assertTrue(historical["coverage"]["historical_inspection"])
        self.assertEqual(datasets.query_claims(self.conn, "pf", subject)["items"][0]["label"], "Disagreement")

    def test_query_rejects_private_pack_and_incomplete_pack_for_public_view(self):
        private = self.pack(changes={"visibility": "private"})
        with self.assertRaises(AppError) as caught:
            datasets.query_claims(self.conn, "pf", {"dataset_id": private["id"], "label": "Exchange", "observer": "public"})
        self.assertEqual(caught.exception.code, "not_found")
        with self.assertRaises(AppError):
            self.pack(changes={"version": "v2", "expected_active_id": private["id"]}, expected_sha256="0" * 64)
        failed = next(row for row in datasets.list_datasets(self.conn, "pf")["items"] if row["status"] == "failed")
        with self.assertRaises(AppError):
            datasets.query_claims(self.conn, "pf", {"dataset_id": failed["id"], "label": "Exchange"})
        for args in ({"label": "Missing pack"}, {"subject": TXID}, {"subject": TXID, "label": "Both"}, {"subject": TXID, "chain": "bitcoin", "network": "main", "observer": []}):
            with self.subTest(args=args), self.assertRaises(AppError):
                datasets.query_claims(self.conn, "pf", args)

    def test_cancellation_and_input_budgets_stop_before_more_stream_reads(self):
        source = GeneratedCSV(100)
        with self.assertRaises(AppError) as caught:
            datasets.preview_dataset(manifest(), source, cancelled=lambda: source.position >= 2)
        self.assertEqual(caught.exception.code, "cancelled")
        self.assertEqual(source.position, 2)
        with patch.object(datasets, "MAX_ROWS", 2), self.assertRaises(AppError) as caught:
            datasets.import_dataset(self.conn, "pf", manifest(), GeneratedCSV(100))
        self.assertEqual(caught.exception.code, "chain_analysis_dataset_limit")
        self.assertEqual(self.match()["claims"], [])
        with patch.object(datasets, "MAX_BYTES", 20), self.assertRaises(AppError):
            datasets.preview_dataset(manifest(), GeneratedCSV(100))


if __name__ == "__main__":
    unittest.main()
