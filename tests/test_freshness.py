import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon as daemon_runtime
from kassiber.core import freshness
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.time_utils import now_iso


def _seed_profile(conn: sqlite3.Connection) -> str:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Main', '2026-06-04T00:00:00Z')"
    )
    conn.execute(
        """
        INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at)
        VALUES('profile', 'ws', 'Book', 'EUR', '2026-06-04T00:00:00Z')
        """
    )
    conn.commit()
    return "profile"


class FreshnessTest(unittest.TestCase):
    def _db(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-freshness-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        return conn

    def test_rate_limited_source_keeps_other_jobs_moving_and_redacts(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        first = freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            payload={"backend_url": "http://secret-node.local/path"},
            priority=10,
        )
        second = freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_MARKET_RATES,
            source_key=freshness.rate_source_key(profile_id),
            source_type=freshness.SOURCE_RATES,
            source_label="Market-rate coverage",
            priority=20,
        )
        conn.commit()
        self.assertEqual(first["status"], freshness.JOB_QUEUED)
        self.assertEqual(second["status"], freshness.JOB_QUEUED)

        calls = []

        def limited(conn, job, progress, check_cancelled):
            calls.append(job["source_key"])
            progress({"phase": freshness.PHASE_BACKEND_FETCH, "backend_url": "http://secret-node.local/path"})
            raise AppError(
                "HTTP 429 from provider",
                code="rate_limited",
                retryable=True,
                details={"retry_after_seconds": 90, "backend_url": "http://secret-node.local/path"},
            )

        def ok(conn, job, progress, check_cancelled):
            calls.append(job["source_key"])
            progress({"phase": freshness.PHASE_RATE_COVERAGE})
            return {"status": "synced", "samples": 3}

        results = freshness.run_due_jobs(
            conn,
            {
                freshness.JOB_ONCHAIN_WALLET: limited,
                freshness.JOB_MARKET_RATES: ok,
            },
            profile_id=profile_id,
            limit=2,
        )

        self.assertEqual(calls, ["onchain_wallet:cold", freshness.rate_source_key(profile_id)])
        self.assertEqual(results[0]["status"], freshness.JOB_RATE_LIMITED)
        self.assertEqual(results[1]["status"], freshness.JOB_DONE)
        snapshot = freshness.build_snapshot(conn, profile_id)
        encoded = json.dumps(snapshot, sort_keys=True)
        self.assertNotIn("secret-node.local", encoded)
        self.assertNotIn("/path", encoded)
        cold = freshness.get_source_state(conn, profile_id, "onchain_wallet:cold")
        self.assertEqual(cold["status"], freshness.STATUS_RATE_LIMITED)
        self.assertTrue(cold["blocking_reports"])
        self.assertEqual(snapshot["summary"]["rate_limited"], 1)

    def test_cancelled_job_leaves_blocking_partial_state(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        job = freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_BTCPAY_WALLET,
            source_key="btcpay_wallet:store",
            source_type=freshness.SOURCE_BTCPAY_WALLET,
            source_label="BTCPay store",
            priority=10,
        )
        cancelled = freshness.cancel_job(conn, job["id"])

        self.assertEqual(cancelled["status"], freshness.JOB_CANCELLED)
        state = freshness.get_source_state(conn, profile_id, "btcpay_wallet:store")
        self.assertEqual(state["status"], freshness.STATUS_BLOCKING_REPORTS)
        self.assertEqual(state["stale_reason"], "cancelled")
        self.assertTrue(state["blocking_reports"])

    def test_running_cancel_stays_single_flight_and_finishes_cancelled(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        job = freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            priority=10,
        )
        conn.commit()
        seen = []

        def cancellable(conn, running_job, progress, check_cancelled):
            progress({"phase": freshness.PHASE_BACKEND_FETCH})
            requested = freshness.cancel_job(conn, running_job["id"])
            self.assertEqual(requested["status"], freshness.JOB_RUNNING)
            self.assertTrue(requested["cancel_requested"])
            duplicate = freshness.enqueue_job(
                conn,
                profile_id=profile_id,
                job_type=freshness.JOB_ONCHAIN_WALLET,
                source_key="onchain_wallet:cold",
                source_type=freshness.SOURCE_ONCHAIN,
                source_label="Cold wallet",
                priority=10,
            )
            self.assertEqual(duplicate["id"], running_job["id"])
            seen.append("cancel_requested")
            check_cancelled()
            raise AssertionError("cancelled job continued after check")

        results = freshness.run_due_jobs(
            conn,
            {freshness.JOB_ONCHAIN_WALLET: cancellable},
            profile_id=profile_id,
            limit=1,
        )

        self.assertEqual(seen, ["cancel_requested"])
        self.assertEqual(results[0]["status"], freshness.JOB_CANCELLED)
        self.assertEqual(results[0]["error"], {})
        state = freshness.get_source_state(conn, profile_id, "onchain_wallet:cold")
        self.assertEqual(state["status"], freshness.STATUS_BLOCKING_REPORTS)
        self.assertEqual(state["stale_reason"], "cancelled")
        self.assertTrue(state["blocking_reports"])

    def test_paused_queued_source_does_not_run_until_resumed(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        job = freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            priority=10,
        )
        freshness.pause_source(conn, profile_id, "onchain_wallet:cold")
        conn.commit()
        calls = []

        def ok(conn, running_job, progress, check_cancelled):
            calls.append(running_job["id"])
            return {"status": "synced"}

        self.assertEqual(
            freshness.run_due_jobs(
                conn,
                {freshness.JOB_ONCHAIN_WALLET: ok},
                profile_id=profile_id,
                limit=1,
            ),
            [],
        )
        self.assertEqual(calls, [])
        queued = freshness.list_jobs(conn, profile_id, active_only=True)
        self.assertEqual(queued[0]["id"], job["id"])
        self.assertEqual(queued[0]["status"], freshness.JOB_QUEUED)

        freshness.resume_source(conn, profile_id, "onchain_wallet:cold")
        results = freshness.run_due_jobs(
            conn,
            {freshness.JOB_ONCHAIN_WALLET: ok},
            profile_id=profile_id,
            limit=1,
        )

        self.assertEqual(calls, [job["id"]])
        self.assertEqual(results[0]["status"], freshness.JOB_DONE)

    def test_recover_interrupted_running_job_requeues_with_checkpoint(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        job = freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            priority=10,
        )
        freshness.upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            status=freshness.STATUS_SYNCING,
            checkpoint={"tip_hash": "abc", "known_txids": ["tx1"]},
        )
        conn.execute(
            "UPDATE freshness_jobs SET status = 'running', phase = ? WHERE id = ?",
            (freshness.PHASE_IMPORT, job["id"]),
        )
        conn.commit()

        recovered = freshness.recover_interrupted_jobs(conn, profile_id=profile_id)

        self.assertEqual(len(recovered), 1)
        self.assertEqual(recovered[0]["status"], freshness.JOB_QUEUED)
        self.assertEqual(recovered[0]["error"]["code"], "worker_interrupted")
        state = freshness.get_source_state(conn, profile_id, "onchain_wallet:cold")
        self.assertEqual(state["status"], freshness.STATUS_BLOCKING_REPORTS)
        self.assertEqual(state["stale_reason"], "worker_interrupted")
        self.assertEqual(state["checkpoint"]["tip_hash"], "abc")

    def test_policy_preserves_legacy_auto_sync_setting(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        conn.execute(
            "INSERT INTO settings(key, value) VALUES(?, 'true')",
            (freshness.legacy_auto_sync_setting_key(profile_id),),
        )
        conn.commit()

        policy = freshness.get_policy(conn, profile_id)

        self.assertTrue(policy.report_read_sync)
        self.assertTrue(policy.source_classes[freshness.SOURCE_ONCHAIN])
        self.assertFalse(policy.background_enabled)

    def test_market_rate_job_seeds_bundled_kraken_before_live_sync(self):
        conn = self._db()
        _seed_profile(conn)
        calls = []

        def fake_seed(conn_arg, commit=True):
            self.assertIs(conn_arg, conn)
            self.assertTrue(commit)
            calls.append("seed")
            return "memory://bundled-kraken", [
                {"pair": "BTC-EUR", "samples": 2, "already_seeded": False}
            ]

        def fake_sync(conn_arg, commit=True):
            self.assertIs(conn_arg, conn)
            self.assertTrue(commit)
            calls.append("sync")
            return [{"pair": "BTC-EUR", "samples": 0}]

        progress = []
        handler = daemon_runtime._freshness_handlers({})[freshness.JOB_MARKET_RATES]
        with patch(
            "kassiber.daemon.core_rates.ensure_bundled_kraken_btc_daily_seed",
            fake_seed,
        ), patch("kassiber.daemon.core_rates.sync_rates", fake_sync):
            result = handler(
                conn,
                {},
                lambda payload: progress.append(dict(payload)),
                lambda: None,
            )

        self.assertEqual(calls, ["seed", "sync"])
        self.assertEqual(progress[0]["phase"], freshness.PHASE_RATE_COVERAGE)
        self.assertEqual(result["bundled_seed"]["path"], "memory://bundled-kraken")
        self.assertEqual(result["bundled_seed"]["summary"][0]["pair"], "BTC-EUR")
        self.assertEqual(result["sync"][0]["pair"], "BTC-EUR")

    def test_background_due_filter_skips_recent_fresh_sources(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        spec = {
            "job_type": freshness.JOB_ONCHAIN_WALLET,
            "source_key": "onchain_wallet:cold",
            "source_type": freshness.SOURCE_ONCHAIN,
            "source_label": "Cold wallet",
        }
        self.assertEqual(
            daemon_runtime._filter_freshness_specs_for_background(conn, profile_id, [spec]),
            [spec],
        )

        freshness.upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key=spec["source_key"],
            source_type=spec["source_type"],
            source_label=spec["source_label"],
            status=freshness.STATUS_FRESH,
            last_success_at=now_iso(),
        )
        conn.commit()
        self.assertEqual(
            daemon_runtime._filter_freshness_specs_for_background(conn, profile_id, [spec]),
            [],
        )

        job = freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key=spec["source_key"],
            source_type=spec["source_type"],
            source_label=spec["source_label"],
        )
        conn.commit()
        self.assertEqual(job["status"], freshness.JOB_QUEUED)
        freshness.upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key=spec["source_key"],
            source_type=spec["source_type"],
            source_label=spec["source_label"],
            status=freshness.STATUS_FAILED,
        )
        conn.commit()
        self.assertEqual(
            daemon_runtime._filter_freshness_specs_for_background(conn, profile_id, [spec]),
            [],
        )


if __name__ == "__main__":
    unittest.main()
