"""Streaming on-chain verification: progress, cancel, and errors, no network."""

import threading
import unittest
from contextlib import contextmanager
from types import SimpleNamespace
from unittest import mock

from kassiber import daemon_ownership_verify as verify
from kassiber.core import ownership
from kassiber.errors import AppError
from tests.test_ownership import _engine_conn

_TXIDS = ["ab" * 32, "cd" * 32]


class _Out:
    def __init__(self):
        self.records = []
        self.finished = threading.Event()

    def write(self, record):
        self.records.append(record)
        if record.get("kind") in (verify.KIND, "error"):
            self.finished.set()


def _ctx(out):
    return SimpleNamespace(
        conn=None, runtime_config={}, out=out, active_verifications=verify.ActiveVerifications()
    )


def _plan(txids=_TXIDS):
    return ownership.prepare_identify(_engine_conn(), "p1", candidates=txids, verify=True)


def _session(fetcher):
    @contextmanager
    def session(_backend):
        yield fetcher

    return session


class FinishIdentifyTests(unittest.TestCase):
    def test_reports_progress_after_each_lookup(self):
        seen = []
        ownership.finish_identify(_plan(), lambda *_: None, progress=lambda *p: seen.append(p))
        self.assertEqual(seen, [(1, 2), (2, 2)])

    def test_cancel_leaves_the_rest_unknown_and_says_so(self):
        calls = []
        report = ownership.finish_identify(
            _plan(), lambda txid, _chain: calls.append(txid), cancelled=lambda: bool(calls)
        )
        self.assertEqual(calls, [_TXIDS[0]])
        self.assertEqual([row["status"] for row in report["results"]], ["unknown", "unknown"])
        self.assertIn("Stopped after 1 of 2", " ".join(report["warnings"]))


class StreamingVerifyTests(unittest.TestCase):
    def _start(self, fetcher, plan=None, request_id="verify-1"):
        out = _Out()
        ctx = _ctx(out)
        prepared = (plan or _plan(), {"name": "b", "kind": "esplora"}, {"workspace": None, "profile": "p"})
        with mock.patch.object(verify, "prepare_wallet_identify_onchain", return_value=prepared), \
             mock.patch.object(verify.core_sync_backends, "verify_session", _session(fetcher)):
            reply = verify.start(ctx, out, request_id, request_id, {})
            self.assertIsNone(reply)
            return ctx, out

    def test_streams_progress_then_the_report(self):
        ctx, out = self._start(lambda *_: None)
        self.assertTrue(out.finished.wait(5))
        kinds = [record["kind"] for record in out.records]
        self.assertEqual(kinds, [verify.PROGRESS_KIND] * 3 + [verify.KIND])
        self.assertEqual(out.records[0]["data"], {"checked": 0, "total": 2})
        self.assertTrue(all(record["request_id"] == "verify-1" for record in out.records))
        self.assertFalse(out.records[-1]["data"]["cancelled"])

    def test_stop_cancels_between_lookups(self):
        release = threading.Event()
        started = threading.Event()

        def fetcher(_txid, _chain):
            started.set()
            release.wait(5)

        ctx, out = self._start(fetcher)
        self.assertTrue(started.wait(5))
        reply = verify.cancel(ctx, "cancel-1", {"target_request_id": "verify-1"})
        self.assertEqual(reply["data"], {"cancelled": True, "running": True})
        release.set()
        self.assertTrue(out.finished.wait(5))
        report = out.records[-1]["data"]
        self.assertTrue(report["cancelled"])
        self.assertIn("Stopped after 1 of 2", " ".join(report["warnings"]))

    def test_a_cancel_that_arrives_first_still_applies(self):
        out = _Out()
        ctx = _ctx(out)
        verify.cancel(ctx, "cancel-1", {"target_request_id": "verify-1"})
        calls = []
        prepared = (_plan(), {"name": "b"}, {})
        with mock.patch.object(verify, "prepare_wallet_identify_onchain", return_value=prepared), \
             mock.patch.object(verify.core_sync_backends, "verify_session", _session(lambda *a: calls.append(a))):
            verify.start(ctx, out, "verify-1", "verify-1", {})
        self.assertTrue(out.finished.wait(5))
        self.assertEqual(calls, [])
        self.assertTrue(out.records[-1]["data"]["cancelled"])

    def test_a_broken_backend_ends_in_an_error_record(self):
        def fetcher(_txid, _chain):
            raise AppError("Electrum needs a chain", code="validation")

        ctx, out = self._start(fetcher)
        self.assertTrue(out.finished.wait(5))
        last = out.records[-1]
        self.assertEqual((last["kind"], last["request_id"]), ("error", "verify-1"))
        self.assertEqual(last["error"]["code"], "validation")
        self.assertEqual(ctx.active_verifications._running, {})

    def test_nothing_to_look_up_answers_at_once(self):
        out = _Out()
        plan = ownership.prepare_identify(_engine_conn(), "p1", candidates=["bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"], verify=True)
        with mock.patch.object(verify, "prepare_wallet_identify_onchain", return_value=(plan, {}, {})):
            reply = verify.start(_ctx(out), out, "verify-1", "verify-1", {})
        self.assertEqual(reply["kind"], verify.KIND)
        self.assertEqual(out.records, [])

    def test_cancel_requires_a_target(self):
        with self.assertRaises(AppError):
            verify.cancel(_ctx(_Out()), "cancel-1", {})


if __name__ == "__main__":
    unittest.main()
