"""Scope, resource and lifetime boundaries for analysis computations and files."""
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core.chain_analysis_runtime import AnalysisJobs, AnalysisSources
from kassiber.errors import AppError


class AnalysisRuntimeTests(unittest.TestCase):
    def test_completed_preview_is_bound_to_source_scope_recipe_hash_and_lifetime(self):
        clock = [0]
        sources = AnalysisSources(clock=lambda: clock[0], ttl_seconds=10)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source.csv"
            path.write_bytes(b"subject,entity\n")
            token = sources.stage(b"book", str(path), "dataset")["source_token"]
            other_token = sources.stage(b"book", str(path), "dataset")["source_token"]
            recipe = {"manifest": {"license": "Declared license"}, "format": "csv", "adapter": "generic"}
            result = {"sha256": "a" * 64, "row_count": 17, "manifest": recipe["manifest"]}
            sources.remember_preview(b"book", token, recipe, result)
            result["row_count"] = 999
            retrieved = sources.preview(b"book", token, recipe, "a" * 64)
            self.assertEqual(retrieved["row_count"], 17)
            retrieved["manifest"]["license"] = "Changed externally"
            self.assertEqual(sources.preview(b"book", token, recipe, "a" * 64)["manifest"]["license"], "Declared license")
            for scope, selected, changed, sha in (
                (b"other-book", token, recipe, "a" * 64),
                (b"book", other_token, recipe, "a" * 64),
                (b"book", token, {**recipe, "manifest": {"license": "Other license"}}, "a" * 64),
                (b"book", token, {**recipe, "format": "jsonl"}, "a" * 64),
                (b"book", token, {**recipe, "adapter": "am_i_exposed"}, "a" * 64),
                (b"book", token, recipe, "b" * 64),
            ):
                with self.subTest(scope=scope, token=selected, recipe=changed, sha=sha):
                    with self.assertRaises(AppError):
                        sources.preview(scope, selected, changed, sha)
            clock[0] = 10
            with self.assertRaises(AppError):
                sources.preview(b"book", token, recipe, "a" * 64)

    def test_source_changes_are_rejected_before_data_or_eof_reaches_the_parser(self):
        import os
        original_fdopen = os.fdopen
        for method in ("read", "readline"):
            for at_eof in (False, True):
                with self.subTest(method=method, at_eof=at_eof), tempfile.TemporaryDirectory() as directory:
                    path = Path(directory) / "source.csv"
                    path.write_bytes(b"subject,entity\n")
                    sources = AnalysisSources()
                    token = sources.stage(b"book", str(path), "dataset")["source_token"]

                    class ChangeDuringRead:
                        def __init__(self, stream):
                            self.stream = stream
                        def __enter__(self):
                            return self
                        def __exit__(self, *args):
                            return self.stream.__exit__(*args)
                        def fileno(self):
                            return self.stream.fileno()
                        def read_value(self, name, size):
                            value = getattr(self.stream, name)(size)
                            if bool(value) != at_eof:
                                path.write_bytes(b"changed source\nmore data\n")
                            return value
                        def read(self, size=-1):
                            return self.read_value("read", size)
                        def readline(self, size=-1):
                            return self.read_value("readline", size)

                    with patch("kassiber.core.chain_analysis_runtime.os.fdopen", side_effect=lambda *args: ChangeDuringRead(original_fdopen(*args))):
                        with sources.open(b"book", token, "dataset") as stream:
                            if at_eof:
                                self.assertEqual(getattr(stream, method)(), b"subject,entity\n")
                            with self.assertRaises(AppError) as error:
                                getattr(stream, method)()
                            self.assertEqual(error.exception.code, "chain_analysis_source_changed")

    def test_source_change_after_a_completed_read_does_not_retract_the_result(self):
        sources = AnalysisSources()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source.csv"
            path.write_bytes(b"reviewed bytes")
            token = sources.stage(b"book", str(path), "dataset")["source_token"]
            with sources.open(b"book", token, "dataset") as stream:
                self.assertEqual(stream.read(), b"reviewed bytes")
                self.assertEqual(stream.read(), b"")
                # A caller may have committed the validated bytes already.
                path.write_bytes(b"a later file revision")
            with self.assertRaises(AppError):
                sources.preview(b"book", token, {}, "a" * 64)

    def test_jobs_cancel_without_publishing_partial_certainty_and_clear_scope(self):
        jobs = AnalysisJobs(concurrent=1)
        entered, finished = threading.Event(), threading.Event()
        def compute(progress, cancelled):
            progress({"phase": "solve", "states_explored": 17})
            entered.set()
            while not cancelled():
                finished.wait(0.001)
            return {"status": "cancelled", "interpretation_count": None}
        first = jobs.start(b"book-a", {"snapshot_id": "old"}, compute)
        self.assertTrue(entered.wait(1))
        with self.assertRaises(AppError) as error:
            jobs.get(b"book-b", first["job_id"])
        self.assertEqual(error.exception.code, "chain_analysis_job_unavailable")
        with self.assertRaises(AppError) as error:
            jobs.start(b"book-b", {}, compute)
        self.assertEqual(error.exception.code, "chain_analysis_busy")
        receipt = jobs.get(b"book-a", first["job_id"], cancel=True)
        self.assertTrue(receipt["cancel_requested"])
        self.assertEqual(receipt["request"]["snapshot_id"], "old")
        jobs.clear()
        with self.assertRaises(AppError):
            jobs.get(b"book-a", first["job_id"])

    def test_error_receipts_do_not_expose_dependency_messages(self):
        jobs = AnalysisJobs()
        finished = threading.Event()
        def compute(*_):
            try:
                raise ValueError("private PSBT /Users/me/wallet.psbt")
            finally:
                finished.set()
        receipt = jobs.start(b"book", {}, compute)
        self.assertTrue(finished.wait(1))
        receipt = jobs.get(b"book", receipt["job_id"])
        self.assertNotIn("private PSBT", str(receipt))
        self.assertEqual(receipt["error_code"], "chain_analysis_computation_failed")

    def test_file_grants_are_book_purpose_and_content_bound(self):
        clock = [0]
        sources = AnalysisSources(clock=lambda: clock[0], ttl_seconds=10)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "evidence.csv"
            path.write_bytes(b"address,entity\n")
            grant = sources.stage(b"book-a", str(path), "dataset")
            self.assertNotIn(directory, str(grant))
            with sources.open(b"book-a", grant["source_token"], "dataset") as stream:
                self.assertEqual(stream.read(), b"address,entity\n")
            for book, purpose in [(b"book-b", "dataset"), (b"book-a", "psbt")]:
                with self.assertRaises(AppError):
                    with sources.open(book, grant["source_token"], purpose):
                        self.fail("scope mismatch accepted")
            path.write_bytes(b"changed")
            with self.assertRaises(AppError) as error:
                with sources.open(b"book-a", grant["source_token"], "dataset"):
                    self.fail("changed source accepted")
            self.assertEqual(error.exception.code, "chain_analysis_source_changed")
            clock[0] = 11
            with self.assertRaises(AppError) as error:
                with sources.open(b"book-a", grant["source_token"], "dataset"):
                    self.fail("expired source accepted")
            self.assertEqual(error.exception.code, "chain_analysis_source_expired")

    def test_file_grants_refuse_directory_and_symlink_replacement(self):
        sources = AnalysisSources()
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(AppError):
                sources.stage(b"book", directory, "dataset")
            path, other = Path(directory) / "a.csv", Path(directory) / "b.csv"
            path.write_text("source")
            other.write_text("private")
            grant = sources.stage(b"book", str(path), "dataset")
            path.unlink()
            path.symlink_to(other)
            with self.assertRaises(AppError):
                with sources.open(b"book", grant["source_token"], "dataset"):
                    self.fail("symlink substitution accepted")


if __name__ == "__main__":
    unittest.main()
