import ast
import json
import queue
import sqlite3
import tempfile
import threading
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import Mock, patch

from kassiber import daemon as daemon_runtime
from kassiber import daemon_freshness
from kassiber.cli import handlers as cli_handlers
from kassiber.core import freshness, rates as core_rates
from kassiber.core.chain_observer.provenance import canonical_graph_hash
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import require_sqlcipher, sqlcipher_available
from kassiber.time_utils import now_iso


def _minutes_ago(minutes: int) -> str:
    return (
        (datetime.now(timezone.utc) - timedelta(minutes=minutes))
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


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


class _Out:
    def __init__(self):
        self.payloads = []

    def write(self, payload):
        self.payloads.append(payload)


class BackgroundFreshnessEventEnvelopeTest(unittest.TestCase):
    def test_background_emissions_use_event_envelope_without_request_id(self):
        out = _Out()
        daemon_freshness._emit_background_freshness_event(
            out,
            "ui.freshness.worker",
            {
                "status": "error",
                "backend_url": "http://secret-node.local/path",
            },
        )
        self.assertEqual(len(out.payloads), 1)
        envelope = out.payloads[0]
        self.assertEqual(envelope["kind"], "ui.freshness.worker")
        self.assertIn("schema_version", envelope)
        # The desktop supervisor routes on this marker; a post-ready record
        # without it and without a request_id kills the daemon.
        self.assertIs(envelope["event"], True)
        self.assertNotIn("request_id", envelope)
        self.assertNotIn("backend_url", envelope["data"])
        self.assertTrue(envelope["data"]["has_backend_url"])


class FreshnessTest(unittest.TestCase):
    def _db(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-freshness-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        return conn

    def test_module_docstring_is_visible_to_ast(self):
        source = Path(freshness.__file__).read_text(encoding="utf-8")
        self.assertEqual(
            ast.get_docstring(ast.parse(source)),
            "SQLite-backed source freshness and daemon job orchestration helpers.",
        )

    def test_deprecated_wallets_skip_automatic_freshness_but_allow_explicit_refresh(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        for wallet_id, label, config in [
            ("active", "Active", {"addresses": ["bc1qactive"]}),
            ("old", "Old", {"addresses": ["bc1qold"], "deprecated": True}),
        ]:
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind,
                    config_json, created_at
                )
                VALUES(?, 'ws', ?, NULL, ?, 'address', ?, '2026-06-04T00:00:00Z')
                """,
                (wallet_id, profile_id, label, json.dumps(config, sort_keys=True)),
            )
        conn.commit()

        automatic = daemon_freshness._freshness_wallet_source_specs(
            conn,
            profile_id,
            include_rates=False,
            include_journals=False,
        )
        self.assertEqual([spec["payload"]["wallet_label"] for spec in automatic], ["Active"])

        explicit = daemon_freshness._freshness_wallet_source_specs(
            conn,
            profile_id,
            wallet_ref="Old",
            include_rates=False,
            include_journals=False,
        )
        self.assertEqual([spec["payload"]["wallet_label"] for spec in explicit], ["Old"])

    def test_cli_wallet_sync_reuses_persisted_freshness_checkpoint(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            ) VALUES(
                'wallet', 'ws', ?, NULL, 'Wallet', 'address',
                '{"addresses":["bc1qwallet"]}', '2026-06-04T00:00:00Z'
            )
            """,
            (profile_id,),
        )
        checkpoint = {"electrum_statuses": {"script": "history-hash"}}
        freshness.upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key=freshness.source_key(freshness.SOURCE_ONCHAIN, "wallet"),
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Wallet",
            status=freshness.STATUS_FRESH,
            checkpoint=checkpoint,
        )
        conn.commit()

        with patch(
            "kassiber.cli.handlers._prefetch_chain_wallets",
            return_value={"wallet": object()},
        ) as prefetch, patch(
            "kassiber.cli.handlers._apply_wallet_sync_atomically",
            return_value=[{"wallet": "Wallet", "status": "synced"}],
        ) as apply:
            result = cli_handlers.sync_wallet(
                conn,
                {},
                "ws",
                profile_id,
                wallet_ref="wallet",
            )

        self.assertEqual(result[0]["status"], "synced")
        expected = {"wallet": checkpoint}
        self.assertEqual(prefetch.call_args.kwargs["freshness_checkpoints"], expected)
        self.assertEqual(apply.call_args.kwargs["freshness_checkpoints"], expected)

    def test_electrum_prefetch_loads_only_trusted_current_stored_graphs(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        conn.execute(
            """
            INSERT INTO backends(name, kind, url, config_json, created_at, updated_at)
            VALUES('fulcrum', 'electrum', 'tcp://127.0.0.1:50001', '{}',
                   '2026-06-04T00:00:00Z', '2026-06-04T00:00:00Z')
            """
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            ) VALUES(
                'wallet', 'ws', ?, NULL, 'Wallet', 'address',
                '{"addresses":["bc1qwallet"],"backend":"fulcrum"}',
                '2026-06-04T00:00:00Z'
            )
            """,
            (profile_id,),
        )
        txid = "ab" * 32
        raw_graph = json.dumps(
            {
                "txid": txid,
                "_kassiber_electrum_graph": {
                    "kind": "bitcoin_electrum",
                    "version": 1,
                },
                "vin": [],
                "vout": [{"value": 1}],
            },
            sort_keys=True,
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint, occurred_at, direction, asset,
                amount, fee, raw_json, created_at
            ) VALUES(
                'tx', 'ws', ?, 'wallet', ?, 'txid', 'fp',
                '2026-06-04T00:00:00Z', 'inbound', 'BTC', 1000, 0, ?,
                '2026-06-04T00:00:00Z'
            )
            """,
            (profile_id, txid, raw_graph),
        )
        conn.execute(
            """
            INSERT INTO chain_observation_provenance(
                transaction_id, workspace_id, profile_id, wallet_id,
                authority_version, observer_ids_json, observer_kinds_json,
                chain, network, application_revision, graph_hash, quantity_hash,
                fee_attribution, observed_at, updated_at
            ) VALUES(
                'tx', 'ws', ?, 'wallet', 1, '["adapter:electrum:wallet"]',
                '["electrum"]', 'bitcoin', 'mainnet', 'revision', ?, 'quantity',
                'exact', '2026-06-04T00:00:00Z', '2026-06-04T00:00:00Z'
            )
            """,
            (profile_id, canonical_graph_hash(raw_graph)),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint, occurred_at, direction, asset,
                amount, fee, raw_json, created_at
            ) VALUES(
                'tx-untrusted', 'ws', ?, 'wallet', ?, 'txid', 'fp-untrusted',
                '2026-06-04T00:00:00Z', 'inbound', 'BTC', 1000, 0, ?,
                '2026-06-04T00:00:00Z'
            )
            """,
            (profile_id, "cd" * 32, raw_graph),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = 'wallet'").fetchone()

        selected = cli_handlers._electrum_stored_graph_wallets(conn, [wallet])
        traces = []
        conn.set_trace_callback(traces.append)
        history = cli_handlers._stored_wallet_chain_history(conn, profile_id, selected)
        conn.set_trace_callback(None)

        self.assertEqual([row["id"] for row in selected], ["wallet"])
        self.assertEqual(history, {"wallet": {txid: raw_graph}})
        self.assertTrue(
            any("transactions.wallet_id IN (" in statement for statement in traces)
        )

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

    def test_failed_job_is_logged_for_the_logs_screen(self):
        # A hard job failure (e.g. the RP2 tax-calc guard) must reach the RAM
        # log ring via the stdlib logging bridge, so it shows on the Logs
        # screen — not only in structured job state. We assert the ERROR log
        # carries the message + source label; RingHandler delivers it to /logs.
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Journal refresh",
            priority=10,
        )
        conn.commit()

        def boom(conn, job, progress, check_cancelled):
            raise AppError(
                "RP2 multi-asset tax calculation failed", code="tax_failed"
            )

        with self.assertLogs("kassiber.core.freshness", level="ERROR") as captured:
            results = freshness.run_due_jobs(
                conn,
                {freshness.JOB_ONCHAIN_WALLET: boom},
                profile_id=profile_id,
                limit=1,
            )

        self.assertEqual(results[0]["status"], freshness.JOB_ERROR)
        # The source label + error code reach the ring (so /logs shows which
        # source failed and why)...
        self.assertTrue(
            any("Journal refresh" in line for line in captured.output),
            captured.output,
        )
        self.assertTrue(
            any("tax_failed" in line for line in captured.output),
            captured.output,
        )
        # ...but NOT the raw exception text, which could carry operational data
        # (URLs/secrets) on sync errors that the keyed redactor would miss.
        self.assertFalse(
            any(
                "RP2 multi-asset tax calculation failed" in line
                for line in captured.output
            ),
            captured.output,
        )

    def test_failed_job_txid_and_amount_pseudonymized_on_disk(self):
        # The freshness DISK write (freshness_jobs.error_json + source-state
        # last_error_message) must pseudonymize txids/amounts in BOTH the message
        # and the structured details before persisting — not rely on read-back
        # render. (URLs are a separate policy: kept raw on the encrypted disk,
        # scrubbed at the UI boundary — see the URL test below.)
        conn = self._db()
        profile_id = _seed_profile(conn)
        txid = "a" * 64
        freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            priority=10,
        )
        conn.commit()

        def boom(conn, job, progress, check_cancelled):
            raise AppError(
                f"Liquid UTXO {txid}:3 mismatch",
                code="liquid_mismatch",
                details={
                    "stderr": f"node fee_msat=1200 utxo {txid}",
                    "response_preview": "unspent 0.5 BTC",
                },
            )

        freshness.run_due_jobs(
            conn,
            {freshness.JOB_ONCHAIN_WALLET: boom},
            profile_id=profile_id,
            limit=1,
        )

        # message persisted to source state (disk) — scrubbed before write
        state = freshness.get_source_state(conn, profile_id, "onchain_wallet:cold")
        self.assertNotIn(txid, state["last_error_message"])
        self.assertIn("txid#", state["last_error_message"])

        # structured details persisted to freshness_jobs.error_json (disk)
        row = conn.execute(
            "SELECT error_json FROM freshness_jobs WHERE source_key = ?",
            ("onchain_wallet:cold",),
        ).fetchone()
        error_json = row["error_json"]
        self.assertNotIn(txid, error_json)
        self.assertIn("txid#", error_json)
        self.assertIn("amount#", error_json)  # keyed fee_msat + standalone 0.5 BTC
        self.assertNotIn("fee_msat=1200", error_json)

    def test_swallowed_non_apperror_logs_exception_type(self):
        # A non-AppError that escapes a handler's own guards (e.g. an RP2/Liquid
        # balance error during the journal refresh) is wrapped as the opaque
        # "freshness_job_failed". The TYPE must be captured — logged and stored —
        # so the failure is diagnosable, while the raw message (which can carry
        # operational data) must still never reach the ring.
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Journal refresh",
            priority=10,
        )
        conn.commit()

        def boom(conn, job, progress, check_cancelled):
            raise ValueError("balance went negative https://user:pw@node/secret")

        with self.assertLogs("kassiber.core.freshness", level="ERROR") as captured:
            results = freshness.run_due_jobs(
                conn,
                {freshness.JOB_ONCHAIN_WALLET: boom},
                profile_id=profile_id,
                limit=1,
            )

        self.assertEqual(results[0]["status"], freshness.JOB_ERROR)
        # The fully-qualified exception type reaches the ring...
        self.assertTrue(
            any("builtins.ValueError" in line for line in captured.output),
            captured.output,
        )
        # ...the raw message (with its embedded URL/secret) does not.
        self.assertFalse(
            any("balance went negative" in line for line in captured.output),
            captured.output,
        )
        # ...and it is persisted in the job error for diagnostics/UI.
        self.assertEqual(
            results[0]["error"]["details"]["error_class"], "builtins.ValueError"
        )

    def test_database_busy_requeues_with_safe_driver_neutral_diagnostics(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            priority=10,
        )
        conn.commit()

        def busy(conn, job, progress, check_cancelled):
            exc = sqlite3.OperationalError(
                "database is locked at https://user:pw@node/secret"
            )
            exc.sqlite_errorcode = sqlite3.SQLITE_BUSY
            exc.sqlite_errorname = "SQLITE_BUSY"
            raise exc

        with self.assertLogs("kassiber.core.freshness", level="WARNING") as captured:
            results = freshness.run_due_jobs(
                conn,
                {freshness.JOB_ONCHAIN_WALLET: busy},
                profile_id=profile_id,
                limit=1,
            )

        job = results[0]
        self.assertEqual(job["status"], freshness.JOB_RATE_LIMITED)
        self.assertEqual(job["error"]["code"], "database_busy")
        self.assertTrue(job["error"]["retryable"])
        self.assertEqual(job["error"]["details"]["sqlite_error_name"], "SQLITE_BUSY")
        self.assertEqual(job["error"]["details"]["sqlite_error_code"], sqlite3.SQLITE_BUSY)
        self.assertIsNotNone(job["cooldown_until"])
        self.assertIsNotNone(job["run_after"])
        state = freshness.get_source_state(conn, profile_id, "onchain_wallet:cold")
        self.assertEqual(state["status"], freshness.STATUS_RATE_LIMITED)
        self.assertEqual(state["last_error_code"], "database_busy")
        self.assertEqual(state["cooldown_reason"], "database-busy")
        self.assertTrue(any("SQLITE_BUSY" in line for line in captured.output))
        self.assertFalse(any("user:pw" in line for line in captured.output))

    @unittest.skipUnless(sqlcipher_available(), "SQLCipher driver is unavailable")
    def test_sqlcipher_busy_uses_the_same_safe_classifier(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-freshness-sqlcipher-busy-")
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "busy.sqlite3"
        driver = require_sqlcipher()
        owner = driver.connect(path)
        contender = driver.connect(path)
        self.addCleanup(owner.close)
        self.addCleanup(contender.close)
        owner.execute("CREATE TABLE writes(id INTEGER PRIMARY KEY, value TEXT)")
        owner.execute("INSERT INTO writes(value) VALUES('before')")
        owner.commit()
        contender.execute("PRAGMA busy_timeout = 1")
        owner.execute("BEGIN IMMEDIATE")
        owner.execute("UPDATE writes SET value = 'owner'")
        try:
            with self.assertRaises(driver.OperationalError) as raised:
                contender.execute("UPDATE writes SET value = 'contender'")
            self.assertFalse(
                isinstance(raised.exception, sqlite3.OperationalError),
                "Regression requires SQLCipher's independent DB-API hierarchy",
            )
            details = freshness._safe_sqlite_error_details(raised.exception)
            self.assertTrue(freshness._sqlite_error_is_busy(details))
            self.assertEqual(details["sqlite_error_name"], "SQLITE_BUSY")
            self.assertEqual(details["sqlite_error_code"], sqlite3.SQLITE_BUSY)
            self.assertEqual(
                details["error_class"],
                "sqlcipher3.dbapi2.OperationalError",
            )
        finally:
            owner.rollback()

    def test_sqlite_schema_error_keeps_failed_state_and_logs_safe_result_code(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            priority=10,
        )
        conn.commit()

        def malformed(conn, job, progress, check_cancelled):
            conn.execute("SELECT * FROM missing_freshness_internal_table")

        with self.assertLogs("kassiber.core.freshness", level="ERROR") as captured:
            results = freshness.run_due_jobs(
                conn,
                {freshness.JOB_ONCHAIN_WALLET: malformed},
                profile_id=profile_id,
                limit=1,
            )

        job = results[0]
        self.assertEqual(job["status"], freshness.JOB_ERROR)
        self.assertEqual(job["error"]["code"], "freshness_job_failed")
        self.assertEqual(job["error"]["details"]["sqlite_error_name"], "SQLITE_ERROR")
        self.assertEqual(job["error"]["details"]["sqlite_error_code"], sqlite3.SQLITE_ERROR)
        self.assertTrue(any("SQLITE_ERROR" in line for line in captured.output))
        self.assertFalse(
            any("missing_freshness_internal_table" in line for line in captured.output)
        )

    def test_failed_job_error_message_url_is_scrubbed_in_ui_snapshot(self):
        # A backend exception message can embed the backend URL (and inline
        # credentials) — httpx ConnectError / HTTPSConnectionPool strings do.
        # build_snapshot's structured redactor only scrubs secret *keys*, so a
        # URL inside the free-text last_error_message survives. The daemon render
        # boundary (_freshness_snapshot_for_ui) must scrub it before the UI.
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=freshness.JOB_ONCHAIN_WALLET,
            source_key="onchain_wallet:cold",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Cold wallet",
            priority=10,
        )
        conn.commit()

        def boom(conn, job, progress, check_cancelled):
            raise AppError(
                "ConnectError: could not reach "
                "https://user:pass@private-node.local:50002/rpc",
                code="backend_unreachable",
                retryable=True,
            )

        freshness.run_due_jobs(
            conn,
            {freshness.JOB_ONCHAIN_WALLET: boom},
            profile_id=profile_id,
            limit=1,
        )

        # The raw URL is stored in the source state (kept for audit, encrypted
        # at rest) — this is the gap the render boundary must close.
        state = freshness.get_source_state(conn, profile_id, "onchain_wallet:cold")
        self.assertIn("private-node.local", state["last_error_message"])

        # ...but the UI-facing snapshot must NOT leak host / path / credentials.
        ui_encoded = json.dumps(
            daemon_freshness._freshness_snapshot_for_ui(conn, profile_id),
            sort_keys=True,
        )
        self.assertNotIn("private-node.local", ui_encoded)
        self.assertNotIn("user:pass", ui_encoded)
        self.assertNotIn("/rpc", ui_encoded)
        self.assertIn("<backend-url>", ui_encoded)

    def test_sync_text_scrubber_redacts_schemeless_host(self):
        # Defense in depth: an HTTP-client connection-error repr (urllib3/httpx)
        # embeds the host schemeless as host='…', which the scheme-form URL
        # pattern does not catch.
        scrubbed = daemon_freshness._redact_sync_text_for_ui(
            "HTTPSConnectionPool(host='private-node.local', port=50002): Max retries"
        )
        self.assertNotIn("private-node.local", scrubbed)
        self.assertIn("<backend-host>", scrubbed)
        # scheme-form URLs (and inline credentials) are still scrubbed.
        url_scrubbed = daemon_freshness._redact_sync_text_for_ui(
            "see https://user:pass@node.local/rpc"
        )
        self.assertNotIn("user:pass", url_scrubbed)
        self.assertNotIn("node.local", url_scrubbed)
        self.assertIn("<backend-url>", url_scrubbed)

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
        freshness.enqueue_job(
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
        self.assertFalse(policy.source_classes[freshness.SOURCE_RATES])
        self.assertFalse(policy.background_enabled)

    def test_market_rate_job_seeds_bundled_kraken_before_live_sync(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.set_policy(
            conn,
            profile_id,
            source_classes={freshness.SOURCE_RATES: True},
        )
        conn.commit()
        calls = []

        def fake_seed(conn_arg, commit=True):
            self.assertIs(conn_arg, conn)
            self.assertTrue(commit)
            calls.append("seed")
            return "memory://bundled-kraken", [
                {"pair": "BTC-EUR", "samples": 2, "already_seeded": False}
            ]

        def fake_sync(
            conn_arg,
            source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            commit=True,
            warm_cache_when_idle=True,
        ):
            self.assertIs(conn_arg, conn)
            self.assertEqual(source, core_rates.RATE_SOURCE_COINBASE_EXCHANGE)
            self.assertTrue(commit)
            self.assertFalse(warm_cache_when_idle)
            calls.append("sync")
            return [{"pair": "BTC-EUR", "samples": 0}]

        def fake_latest(
            conn_arg,
            source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            commit=True,
        ):
            self.assertIs(conn_arg, conn)
            self.assertEqual(source, core_rates.RATE_SOURCE_COINBASE_EXCHANGE)
            self.assertTrue(commit)
            calls.append("latest")
            return [{"pair": "BTC-EUR", "samples": 1, "mode": "latest_quote"}]

        progress = []
        handler = daemon_freshness._freshness_handlers({})[freshness.JOB_MARKET_RATES]
        with patch(
            "kassiber.daemon_freshness.core_rates.ensure_bundled_kraken_btc_hourly_seed",
            fake_seed,
        ), patch("kassiber.daemon_freshness.core_rates.sync_latest_rates", fake_latest), patch(
            "kassiber.daemon_freshness.core_rates.sync_rates",
            fake_sync,
        ):
            result = handler(
                conn,
                {"profile_id": profile_id},
                lambda payload: progress.append(dict(payload)),
                lambda: None,
            )

        self.assertEqual(calls, ["seed", "latest", "sync"])
        self.assertEqual(progress[0]["phase"], freshness.PHASE_RATE_COVERAGE)
        self.assertEqual(result["provider"], core_rates.RATE_SOURCE_COINBASE_EXCHANGE)
        self.assertEqual(result["bundled_seed"]["path"], "memory://bundled-kraken")
        self.assertEqual(result["bundled_seed"]["summary"][0]["pair"], "BTC-EUR")
        self.assertEqual(result["latest"][0]["mode"], "latest_quote")
        self.assertEqual(result["sync"][0]["pair"], "BTC-EUR")

    def test_market_rate_job_uses_configured_coingecko_provider_for_latest_sync(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.set_policy(
            conn,
            profile_id,
            source_classes={freshness.SOURCE_RATES: True},
        )
        core_rates.set_market_rate_provider(
            conn,
            core_rates.RATE_SOURCE_COINGECKO,
            commit=True,
        )
        calls = []

        def fake_seed(conn_arg, commit=True):
            self.assertIs(conn_arg, conn)
            self.assertTrue(commit)
            calls.append("seed")
            return "memory://bundled-kraken", []

        def fake_latest(
            conn_arg,
            source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            commit=True,
        ):
            self.assertIs(conn_arg, conn)
            self.assertEqual(source, core_rates.RATE_SOURCE_COINGECKO)
            self.assertTrue(commit)
            calls.append("latest")
            return [{"pair": "BTC-EUR", "source": source, "samples": 1}]

        progress = []
        handler = daemon_freshness._freshness_handlers({})[freshness.JOB_MARKET_RATES]
        with patch(
            "kassiber.daemon_freshness.core_rates.ensure_bundled_kraken_btc_hourly_seed",
            fake_seed,
        ), patch("kassiber.daemon_freshness.core_rates.sync_latest_rates", fake_latest), patch(
            "kassiber.daemon_freshness.core_rates.sync_rates",
        ) as sync_rates:
            result = handler(
                conn,
                {"profile_id": profile_id},
                lambda payload: progress.append(dict(payload)),
                lambda: None,
            )

        sync_rates.assert_not_called()
        self.assertEqual(calls, ["seed", "latest"])
        self.assertEqual(progress[0]["phase"], freshness.PHASE_RATE_COVERAGE)
        self.assertEqual(result["provider"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(result["latest"][0]["source"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(result["sync"], [])

    def test_market_rate_job_uses_mempool_for_transaction_backfill(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.set_policy(
            conn,
            profile_id,
            source_classes={freshness.SOURCE_RATES: True},
        )
        core_rates.set_market_rate_provider(
            conn,
            core_rates.RATE_SOURCE_MEMPOOL,
            commit=True,
        )
        calls = []

        def fake_seed(conn_arg, commit=True):
            self.assertIs(conn_arg, conn)
            self.assertTrue(commit)
            calls.append("seed")
            return "memory://bundled-kraken", []

        def fake_latest(
            conn_arg,
            source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            commit=True,
        ):
            self.assertIs(conn_arg, conn)
            self.assertEqual(source, core_rates.RATE_SOURCE_MEMPOOL)
            self.assertTrue(commit)
            calls.append("latest")
            return [{"pair": "BTC-EUR", "source": source, "samples": 1}]

        def fake_sync(
            conn_arg,
            source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            commit=True,
            warm_cache_when_idle=True,
        ):
            self.assertIs(conn_arg, conn)
            self.assertEqual(source, core_rates.RATE_SOURCE_MEMPOOL)
            self.assertTrue(commit)
            self.assertFalse(warm_cache_when_idle)
            calls.append("sync")
            return [
                {
                    "pair": "BTC-EUR",
                    "source": core_rates.RATE_SOURCE_MEMPOOL,
                    "mode": "transaction_need",
                }
            ]

        handler = daemon_freshness._freshness_handlers({})[freshness.JOB_MARKET_RATES]
        with patch(
            "kassiber.daemon_freshness.core_rates.ensure_bundled_kraken_btc_hourly_seed",
            fake_seed,
        ), patch("kassiber.daemon_freshness.core_rates.sync_latest_rates", fake_latest), patch(
            "kassiber.daemon_freshness.core_rates.sync_rates",
            fake_sync,
        ):
            result = handler(
                conn,
                {"profile_id": profile_id},
                lambda _payload: None,
                lambda: None,
            )

        self.assertEqual(calls, ["seed", "latest", "sync"])
        self.assertEqual(result["provider"], core_rates.RATE_SOURCE_MEMPOOL)
        self.assertEqual(result["latest"][0]["source"], core_rates.RATE_SOURCE_MEMPOOL)
        self.assertEqual(result["sync"][0]["source"], core_rates.RATE_SOURCE_MEMPOOL)

    def test_market_rate_job_handler_refuses_provider_when_policy_off(self):
        # Defense in depth on top of the enqueue-level gate: even if a
        # market_rates job is somehow run for a profile that disabled the
        # market_rates source class, the handler seeds the offline bundled
        # archive but never contacts a live provider.
        conn = self._db()
        profile_id = _seed_profile(conn)
        freshness.set_policy(
            conn,
            profile_id,
            source_classes={freshness.SOURCE_RATES: False},
        )
        conn.commit()

        def fake_seed(conn_arg, commit=True):
            self.assertIs(conn_arg, conn)
            self.assertTrue(commit)
            return "memory://bundled-kraken", [
                {"pair": "BTC-EUR", "samples": 2, "already_seeded": False},
                {"pair": "BTC-USD", "samples": 2, "already_seeded": False},
            ]

        latest = Mock()
        sync = Mock()
        progress = []
        handler = daemon_freshness._freshness_handlers({})[freshness.JOB_MARKET_RATES]
        with patch(
            "kassiber.daemon_freshness.core_rates.ensure_bundled_kraken_btc_hourly_seed",
            fake_seed,
        ), patch(
            "kassiber.daemon_freshness.core_rates.sync_latest_rates", latest
        ), patch(
            "kassiber.daemon_freshness.core_rates.sync_rates", sync
        ):
            result = handler(
                conn,
                {"profile_id": profile_id},
                lambda payload: progress.append(dict(payload)),
                lambda: None,
            )

        latest.assert_not_called()
        sync.assert_not_called()
        self.assertEqual(result["status"], "synced")
        self.assertFalse(result["live_refresh"])
        self.assertEqual(result["skipped_reason"], "market_rates_disabled")
        self.assertEqual(result["latest"], [])
        self.assertEqual(result["sync"], [])
        self.assertEqual(result["bundled_seed"]["path"], "memory://bundled-kraken")
        self.assertEqual(progress[0]["phase"], freshness.PHASE_RATE_COVERAGE)

    def test_freshness_configure_persists_market_rate_provider(self):
        conn = self._db()
        _seed_profile(conn)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", "profile")

        payload = daemon_freshness._freshness_configure_payload(
            conn,
            {"market_rate_provider": core_rates.RATE_SOURCE_COINGECKO},
        )

        self.assertEqual(
            payload["settings"]["market_rate_provider"],
            core_rates.RATE_SOURCE_COINGECKO,
        )
        self.assertEqual(
            core_rates.get_market_rate_provider(conn),
            core_rates.RATE_SOURCE_COINGECKO,
        )

    def test_freshness_run_honors_market_rate_source_class_off(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", profile_id)
        freshness.set_policy(
            conn,
            profile_id,
            source_classes={freshness.SOURCE_RATES: False},
        )
        conn.commit()

        payload = daemon_freshness._freshness_run_payload(
            conn,
            {},
            {"all": True, "rates": True, "journals": True, "run": False},
        )

        self.assertNotIn(
            freshness.SOURCE_RATES,
            {job["source_type"] for job in payload["enqueued"]},
        )

    def test_desktop_sync_prefetches_wallets_together_before_serial_job_apply(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", profile_id)
        for wallet_id, label in (("wallet-a", "Alpha"), ("wallet-b", "Beta")):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind,
                    config_json, created_at
                ) VALUES(?, 'ws', ?, NULL, ?, 'address', ?, '2026-06-04T00:00:00Z')
                """,
                (
                    wallet_id,
                    profile_id,
                    label,
                    json.dumps({"addresses": [f"bc1q{wallet_id}"]}),
                ),
            )
        conn.commit()
        prefetched = {"wallet-a": object(), "wallet-b": object()}
        events = []

        def prefetch(*args, **kwargs):
            events.append("prefetch")
            self.assertEqual([wallet["id"] for wallet in args[4]], ["wallet-a", "wallet-b"])
            return prefetched

        def sync(*args, **kwargs):
            events.append(f"sync:{args[4]['id']}")
            self.assertEqual(set(kwargs["prefetched"]), {args[4]["id"]})
            self.assertIs(
                kwargs["prefetched"][args[4]["id"]],
                prefetched[args[4]["id"]],
            )
            return {"freshness_checkpoint": {"wallet": args[4]["id"]}}

        with patch(
            "kassiber.daemon_freshness.prefetch_wallets_from_backend",
            side_effect=prefetch,
        ), patch(
            "kassiber.daemon_freshness.sync_wallet_from_backend",
            side_effect=sync,
        ):
            payload = daemon_freshness._freshness_run_payload(
                conn,
                {},
                {"all": True, "rates": False, "journals": False, "run": True},
            )

        self.assertEqual(events[0], "prefetch")
        self.assertEqual(set(events[1:]), {"sync:wallet-a", "sync:wallet-b"})
        self.assertEqual(len(payload["completed"]), 2)
        self.assertTrue(
            all(job["status"] == freshness.JOB_DONE for job in payload["completed"])
        )

    def test_desktop_sync_prefetches_only_jobs_selected_by_run_limit(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", profile_id)
        for wallet_id, label in (("wallet-a", "Alpha"), ("wallet-b", "Beta")):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind,
                    config_json, created_at
                ) VALUES(?, 'ws', ?, NULL, ?, 'address', ?, '2026-06-04T00:00:00Z')
                """,
                (
                    wallet_id,
                    profile_id,
                    label,
                    json.dumps({"addresses": [f"bc1q{wallet_id}"]}),
                ),
            )
        conn.commit()
        prefetched_wallet_ids = []

        def prefetch(*args, **_kwargs):
            wallet_ids = [wallet["id"] for wallet in args[4]]
            prefetched_wallet_ids.append(wallet_ids)
            return {wallet_id: object() for wallet_id in wallet_ids}

        with patch(
            "kassiber.daemon_freshness.prefetch_wallets_from_backend",
            side_effect=prefetch,
        ), patch(
            "kassiber.daemon_freshness.sync_wallet_from_backend",
            return_value={"freshness_checkpoint": {"ok": True}},
        ):
            payload = daemon_freshness._freshness_run_payload(
                conn,
                {},
                {
                    "all": True,
                    "rates": False,
                    "journals": False,
                    "run": True,
                    "limit": 1,
                },
            )

        self.assertEqual(len(prefetched_wallet_ids), 1)
        self.assertEqual(len(prefetched_wallet_ids[0]), 1)
        self.assertIn(prefetched_wallet_ids[0][0], {"wallet-a", "wallet-b"})
        self.assertEqual(len(payload["completed"]), 1)
        self.assertEqual(payload["completed"][0]["status"], freshness.JOB_DONE)

    def test_desktop_sync_does_not_prefetch_duplicate_wallet_jobs(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            ) VALUES(
                'wallet-a', 'ws', ?, NULL, 'Alpha', 'address',
                '{"addresses":["bc1qwallet-a"]}', '2026-06-04T00:00:00Z'
            )
            """,
            (profile_id,),
        )
        conn.commit()
        source_key = freshness.source_key(freshness.SOURCE_ONCHAIN, "wallet-a")
        jobs = [
            {
                "id": f"job-{index}",
                "job_type": freshness.JOB_ONCHAIN_WALLET,
                "source_key": source_key,
                "payload": {"wallet_id": "wallet-a", "force_full": True},
            }
            for index in range(2)
        ]
        profile = conn.execute(
            "SELECT * FROM profiles WHERE id = ?",
            (profile_id,),
        ).fetchone()

        with patch(
            "kassiber.daemon_freshness.prefetch_wallets_from_backend"
        ) as prefetch:
            result = daemon_freshness._prefetch_onchain_freshness_jobs(
                conn,
                {},
                profile,
                jobs,
            )

        self.assertEqual(result, {})
        prefetch.assert_not_called()

    def test_desktop_sync_falls_back_to_isolated_jobs_if_batch_prefetch_fails(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", profile_id)
        for wallet_id, label in (("wallet-a", "Alpha"), ("wallet-b", "Beta")):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind,
                    config_json, created_at
                ) VALUES(?, 'ws', ?, NULL, ?, 'address', ?, '2026-06-04T00:00:00Z')
                """,
                (
                    wallet_id,
                    profile_id,
                    label,
                    json.dumps({"addresses": [f"bc1q{wallet_id}"]}),
                ),
            )
        conn.commit()
        isolated_wallets = []

        def sync(*args, **kwargs):
            self.assertIsNone(kwargs["prefetched"])
            isolated_wallets.append(args[4]["id"])
            return {"freshness_checkpoint": {"wallet": args[4]["id"]}}

        with self.assertLogs("kassiber.daemon.freshness", level="WARNING"), patch(
            "kassiber.daemon_freshness.prefetch_wallets_from_backend",
            side_effect=RuntimeError("batch coordinator failed"),
        ), patch(
            "kassiber.daemon_freshness.sync_wallet_from_backend",
            side_effect=sync,
        ):
            payload = daemon_freshness._freshness_run_payload(
                conn,
                {},
                {"all": True, "rates": False, "journals": False, "run": True},
            )

        self.assertEqual(set(isolated_wallets), {"wallet-a", "wallet-b"})
        self.assertEqual(len(payload["completed"]), 2)
        self.assertTrue(
            all(job["status"] == freshness.JOB_DONE for job in payload["completed"])
        )

    def test_freshness_run_can_request_auto_pair_before_journals(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", profile_id)

        payload = daemon_freshness._freshness_run_payload(
            conn,
            {},
            {
                "all": True,
                "rates": False,
                "journals": True,
                "auto_pair": True,
                "run": False,
            },
        )

        journal_jobs = [
            job
            for job in payload["enqueued"]
            if job["job_type"] == freshness.JOB_JOURNAL_REFRESH
        ]
        self.assertEqual(len(journal_jobs), 1)
        self.assertEqual(journal_jobs[0]["payload"], {"auto_pair": True})

    def test_onchain_job_sync_uses_job_profile_not_active_context(self):
        conn = self._db()
        active_profile_id = _seed_profile(conn)
        job_profile_id = "second-profile"
        conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at)
            VALUES(?, 'ws', 'Second Book', 'EUR', '2026-06-04T00:00:00Z')
            """,
            (job_profile_id,),
        )
        conn.execute("INSERT INTO settings(key, value) VALUES('context_workspace', 'ws')")
        conn.execute(
            "INSERT INTO settings(key, value) VALUES('context_profile', ?)",
            (active_profile_id,),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
            )
            VALUES(
                'wallet-second', 'ws', ?, NULL, 'Second Wallet', 'address',
                '{"addresses":["bc1qsecond"],"chain":"bitcoin","network":"mainnet"}',
                '2026-06-04T00:00:00Z'
            )
            """,
            (job_profile_id,),
        )
        conn.commit()
        source_key = freshness.source_key(freshness.SOURCE_ONCHAIN, "wallet-second")
        handler = daemon_freshness._freshness_handlers({})[
            freshness.JOB_ONCHAIN_WALLET
        ]

        with patch(
            "kassiber.daemon_freshness.sync_wallet_from_backend",
            return_value={"freshness_checkpoint": {"ok": True}},
        ) as sync_mock:
            result = handler(
                conn,
                {
                    "profile_id": job_profile_id,
                    "source_key": source_key,
                    "payload": {"wallet_id": "wallet-second"},
                },
                lambda _payload: None,
                lambda: None,
            )

        self.assertEqual(result["wallet"], "Second Wallet")
        self.assertEqual(result["status"], "synced")
        call_args = sync_mock.call_args.args
        self.assertEqual(call_args[2], "ws")
        self.assertEqual(call_args[3], job_profile_id)

    def test_prefetched_onchain_fetch_is_bound_to_exact_job_contract(self):
        fetches = {"wallet-a": object()}
        checkpoint = {"tip": 123}
        job = {"id": "job-a"}
        wallet = {
            "id": "wallet-a",
            "kind": "descriptor",
            "config_json": '{"backend":"fulcrum"}',
        }
        entry = {
            "job-a": {
                "wallet_id": "wallet-a",
                "wallet_signature": daemon_freshness._wallet_sync_config_signature(wallet),
                "checkpoint": checkpoint,
                "force_full": False,
                "fetches": fetches,
            }
        }

        self.assertIs(
            daemon_freshness._prefetched_onchain_fetches_for_job(
                entry,
                job,
                wallet,
                checkpoint,
                False,
            ),
            fetches,
        )
        for mismatched_job, mismatched_checkpoint, force_full in (
            ({"id": "job-b"}, checkpoint, False),
            (job, {"tip": 124}, False),
            (job, checkpoint, True),
        ):
            self.assertIsNone(
                daemon_freshness._prefetched_onchain_fetches_for_job(
                    entry,
                    mismatched_job,
                    wallet,
                    mismatched_checkpoint,
                    force_full,
                )
            )

        changed_wallet = dict(wallet, config_json='{"backend":"core"}')
        self.assertIsNone(
            daemon_freshness._prefetched_onchain_fetches_for_job(
                entry,
                job,
                changed_wallet,
                checkpoint,
                False,
            )
        )

    def test_journal_freshness_handler_auto_pairs_before_processing(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        progress = []
        handler = daemon_freshness._freshness_handlers({})[
            freshness.JOB_JOURNAL_REFRESH
        ]

        with patch(
            "kassiber.daemon_freshness._auto_pair_before_journals",
            return_value={"enabled": True, "applied": 2, "remaining": {"total": 1}},
        ) as auto_pair, patch(
            "kassiber.daemon_freshness._journals_process_payload",
            return_value={"quarantined": 0, "entries_created": 4},
        ) as process:
            result = handler(
                conn,
                {"profile_id": profile_id, "payload": {"auto_pair": True}},
                progress.append,
                lambda: None,
            )

        self.assertEqual(
            [item["phase"] for item in progress],
            ["auto_pair", "journal_refresh"],
        )
        auto_pair.assert_called_once()
        process.assert_called_once()
        self.assertEqual(result["auto_pair"]["applied"], 2)
        self.assertEqual(result["entries_created"], 4)

    def test_journal_freshness_handler_continues_when_auto_pair_fails(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        progress = []
        handler = daemon_freshness._freshness_handlers({})[
            freshness.JOB_JOURNAL_REFRESH
        ]

        with patch(
            "kassiber.daemon_freshness._auto_pair_before_journals",
            side_effect=AppError("profile missing", code="not_found"),
        ) as auto_pair, patch(
            "kassiber.daemon_freshness._journals_process_payload",
            return_value={"quarantined": 0, "entries_created": 4},
        ) as process:
            result = handler(
                conn,
                {"profile_id": profile_id, "payload": {"auto_pair": True}},
                progress.append,
                lambda: None,
            )

        self.assertEqual(
            [item["phase"] for item in progress],
            ["auto_pair", "journal_refresh"],
        )
        auto_pair.assert_called_once()
        process.assert_called_once()
        self.assertEqual(result["entries_created"], 4)
        self.assertEqual(result["auto_pair"]["applied"], 0)
        self.assertTrue(result["auto_pair"]["skipped"])
        self.assertEqual(result["auto_pair"]["error"]["code"], "not_found")

    def test_journal_freshness_handler_rolls_back_auto_pairs_when_processing_fails(self):
        # Auto-pair inserts are commit=False (pending). If journal processing
        # then fails, run_job commits the connection on its way to marking the
        # job failed — which must NOT persist the pending pairs. The handler
        # rolls back so the pair + journal step is atomic.
        conn = self._db()
        profile_id = _seed_profile(conn)
        handler = daemon_freshness._freshness_handlers({})[
            freshness.JOB_JOURNAL_REFRESH
        ]

        def seed_pending_pair(conn_arg, _job):
            # Stand in for a commit=False auto-pair insert left pending.
            conn_arg.execute(
                "INSERT INTO settings(key, value) VALUES('pending-auto-pair', '1')"
            )
            return {"enabled": True, "applied": 1, "remaining": {"total": 0}}

        # run_job's real progress callback COMMITS the connection on every
        # phase emission. Mimic that here, so the test catches a phase commit
        # landing *after* the pending auto-pair inserts (which would make the
        # rollback unable to undo them).
        def committing_progress(_payload):
            conn.commit()

        with patch(
            "kassiber.daemon_freshness._auto_pair_before_journals",
            side_effect=seed_pending_pair,
        ), patch(
            "kassiber.daemon_freshness._journals_process_payload",
            side_effect=AppError("journal boom", code="tax_failed"),
        ):
            with self.assertRaises(AppError):
                handler(
                    conn,
                    {"profile_id": profile_id, "payload": {"auto_pair": True}},
                    committing_progress,
                    lambda: None,
                )

        # The pending auto-pair write must have been rolled back, not left for
        # run_job's error handler to commit.
        row = conn.execute(
            "SELECT value FROM settings WHERE key = 'pending-auto-pair'"
        ).fetchone()
        self.assertIsNone(row)

    def test_auto_pair_before_journals_returns_applied_and_remaining_counts(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        before = {"counts": {"total": 4, "exact": 2, "strong": 2, "conflicts": 1}}
        remaining = {"counts": {"total": 1, "exact": 0, "strong": 1, "conflicts": 1}}

        with patch(
            "kassiber.daemon_freshness.suggest_transfer_candidates",
            side_effect=[before, remaining],
        ), patch(
            "kassiber.daemon_freshness.apply_transfer_rules",
            return_value={"summary": {"count": 1, "total_swap_fee_msat": 1200}},
        ) as rules, patch(
            "kassiber.daemon_freshness.bulk_pair_transfers",
            return_value={
                "summary": {
                    "count": 2,
                    "skipped_conflicts": 1,
                    "total_swap_fee_msat": 800,
                }
            },
        ) as bulk:
            summary = daemon_freshness._auto_pair_before_journals(
                conn,
                {"profile_id": profile_id},
            )

        rules.assert_called_once_with(conn, "ws", profile_id, commit=False)
        bulk.assert_called_once_with(
            conn,
            "ws",
            profile_id,
            confidence="exact",
            commit=False,
        )
        self.assertEqual(summary["applied"], 3)
        self.assertEqual(summary["rules_applied"], 1)
        self.assertEqual(summary["bulk_exact_applied"], 2)
        self.assertEqual(summary["skipped_conflicts"], 1)
        self.assertEqual(summary["total_swap_fee_msat"], 2000)
        self.assertEqual(summary["before"]["total"], 4)
        self.assertEqual(summary["remaining"]["total"], 1)
        # The post-pairing total is cached on the profile for the side-nav badge.
        cached = conn.execute(
            "SELECT swap_candidate_count FROM profiles WHERE id = ?",
            (profile_id,),
        ).fetchone()[0]
        self.assertEqual(cached, 1)

    def test_workspace_freshness_run_honors_each_book_market_rate_policy(self):
        conn = self._db()
        first_profile = _seed_profile(conn)
        conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at)
            VALUES('second-profile', 'ws', 'Second Book', 'EUR', '2026-06-04T00:00:00Z')
            """
        )
        freshness.set_policy(
            conn,
            "second-profile",
            source_classes={freshness.SOURCE_RATES: True},
        )
        conn.commit()

        payload = daemon_freshness._workspace_freshness_run_payload(
            conn,
            {},
            {"workspace_id": "ws", "rates": True, "journals": True, "run": False},
        )

        rates_by_profile = {
            book["profile"]["id"]: [
                job for job in book["enqueued"] if job["source_type"] == freshness.SOURCE_RATES
            ]
            for book in payload["books"]
        }
        self.assertEqual(rates_by_profile[first_profile], [])
        self.assertEqual(len(rates_by_profile["second-profile"]), 1)

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
            daemon_freshness._filter_freshness_specs_for_background(conn, profile_id, [spec]),
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
            daemon_freshness._filter_freshness_specs_for_background(conn, profile_id, [spec]),
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
            daemon_freshness._filter_freshness_specs_for_background(conn, profile_id, [spec]),
            [],
        )

    def test_background_due_filter_uses_hourly_market_rate_interval(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        onchain_spec = {
            "job_type": freshness.JOB_ONCHAIN_WALLET,
            "source_key": "onchain_wallet:cold",
            "source_type": freshness.SOURCE_ONCHAIN,
            "source_label": "Cold wallet",
        }
        rate_spec = {
            "job_type": freshness.JOB_MARKET_RATES,
            "source_key": freshness.rate_source_key(profile_id),
            "source_type": freshness.SOURCE_RATES,
            "source_label": "Market-rate coverage",
        }
        freshness.upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key=onchain_spec["source_key"],
            source_type=onchain_spec["source_type"],
            source_label=onchain_spec["source_label"],
            status=freshness.STATUS_FRESH,
            last_success_at=_minutes_ago(16),
        )
        freshness.upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key=rate_spec["source_key"],
            source_type=rate_spec["source_type"],
            source_label=rate_spec["source_label"],
            status=freshness.STATUS_FRESH,
            last_success_at=_minutes_ago(30),
        )
        conn.commit()

        self.assertEqual(
            daemon_freshness._filter_freshness_specs_for_background(
                conn,
                profile_id,
                [onchain_spec, rate_spec],
            ),
            [onchain_spec],
        )

        freshness.upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key=rate_spec["source_key"],
            source_type=rate_spec["source_type"],
            source_label=rate_spec["source_label"],
            status=freshness.STATUS_FRESH,
            last_success_at=_minutes_ago(61),
        )
        conn.commit()

        self.assertEqual(
            daemon_freshness._filter_freshness_specs_for_background(
                conn,
                profile_id,
                [rate_spec],
            ),
            [rate_spec],
        )

    def test_wallet_scoped_refresh_single_flights_inside_global_refresh(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        conn.execute("INSERT INTO settings(key, value) VALUES('context_workspace', 'ws')")
        conn.execute("INSERT INTO settings(key, value) VALUES('context_profile', ?)", (profile_id,))
        now = now_iso()
        conn.executemany(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
            )
            VALUES(?, 'ws', ?, NULL, ?, 'address', ?, ?)
            """,
            [
                (
                    "wallet-cold",
                    profile_id,
                    "Cold",
                    json.dumps({"addresses": ["bc1qcold"], "chain": "bitcoin", "network": "mainnet"}),
                    now,
                ),
                (
                    "wallet-hot",
                    profile_id,
                    "Hot",
                    json.dumps({"addresses": ["bc1qhot"], "chain": "bitcoin", "network": "mainnet"}),
                    now,
                ),
            ],
        )
        freshness.set_policy(
            conn,
            profile_id,
            source_classes={freshness.SOURCE_RATES: True},
        )
        conn.commit()
        cold_key = freshness.source_key(freshness.SOURCE_ONCHAIN, "wallet-cold")
        hot_key = freshness.source_key(freshness.SOURCE_ONCHAIN, "wallet-hot")

        scoped = daemon_freshness._freshness_run_payload(
            conn,
            {},
            {"wallet": "Cold", "all": False, "rates": False, "journals": False, "run": False},
        )
        self.assertEqual([job["source_key"] for job in scoped["enqueued"]], [cold_key])
        scoped_active_keys = [
            job["source_key"] for job in freshness.list_jobs(conn, profile_id, active_only=True)
        ]
        self.assertEqual(scoped_active_keys, [cold_key])

        global_run = daemon_freshness._freshness_run_payload(
            conn,
            {},
            {"all": True, "rates": True, "journals": True, "run": False},
        )
        active = freshness.list_jobs(conn, profile_id, active_only=True)
        active_keys = [job["source_key"] for job in active]

        self.assertIn(cold_key, [job["source_key"] for job in global_run["enqueued"]])
        self.assertIn(hot_key, active_keys)
        self.assertIn(freshness.rate_source_key(profile_id), active_keys)
        self.assertIn(freshness.journal_source_key(profile_id), active_keys)
        self.assertEqual(active_keys.count(cold_key), 1)

    def test_background_worker_uses_remembered_unlock_passphrase(self):
        conn = self._db()
        profile_id = _seed_profile(conn)
        conn.execute("INSERT INTO settings(key, value) VALUES('context_workspace', 'ws')")
        conn.execute("INSERT INTO settings(key, value) VALUES('context_profile', ?)", (profile_id,))
        freshness.set_policy(conn, profile_id, background_enabled=True)
        conn.commit()

        ctx = daemon_runtime.DaemonContext(
            conn=conn,
            data_root="encrypted-data-root",
            runtime_config={},
            active_ai_chats=daemon_runtime.ActiveAiChats(),
            main_thread_tasks=queue.Queue(),
            auth_backoff=daemon_runtime.AuthAttemptBackoff(),
            input_lines=queue.Queue(),
            deferred_input_lines=[],
            out=_Out(),
            freshness_stop_event=threading.Event(),
            db_passphrase="remembered-passphrase",
        )
        captured = []
        opened = threading.Event()
        closed = threading.Event()

        class _WorkerConn:
            def close(self):
                closed.set()

        def fake_open_db(data_root, *, passphrase=None, require_existing_schema=False):
            del data_root, require_existing_schema
            captured.append(passphrase)
            opened.set()
            return _WorkerConn()

        try:
            with (
                patch.object(daemon_freshness, "open_db", side_effect=fake_open_db),
                patch.object(daemon_freshness, "merge_db_backends"),
                patch.object(daemon_freshness, "_freshness_background_tick"),
            ):
                daemon_freshness._start_freshness_background_worker(ctx)
                self.assertTrue(opened.wait(timeout=2))
                self.assertEqual(getattr(ctx.freshness_worker, "_args", ()), ())
        finally:
            ctx.freshness_stop_event.set()
            if ctx.freshness_worker is not None:
                ctx.freshness_worker.join(timeout=2)

        self.assertEqual(captured, ["remembered-passphrase"])
        self.assertTrue(closed.wait(timeout=2))

    def test_stop_worker_rebinds_event_after_join_timeout(self):
        original_event = threading.Event()

        class _HungWorker:
            def __init__(self):
                self.join_timeout = None

            def join(self, timeout=None):
                self.join_timeout = timeout

            def is_alive(self):
                return True

        worker = _HungWorker()

        class _Ctx:
            pass

        ctx = _Ctx()
        ctx.conn = None
        ctx.freshness_stop_event = original_event
        ctx.freshness_worker = worker

        daemon_freshness._stop_freshness_background_worker(ctx)

        self.assertEqual(worker.join_timeout, 2.0)
        self.assertTrue(original_event.is_set())
        self.assertIsNone(ctx.freshness_worker)
        self.assertIsNot(ctx.freshness_stop_event, original_event)
        self.assertFalse(ctx.freshness_stop_event.is_set())

    def test_daemon_lock_clears_remembered_background_passphrase(self):
        conn = self._db()
        _seed_profile(conn)
        ctx = daemon_runtime.DaemonContext(
            conn=conn,
            data_root="encrypted-data-root",
            runtime_config={},
            active_ai_chats=daemon_runtime.ActiveAiChats(),
            main_thread_tasks=queue.Queue(),
            auth_backoff=daemon_runtime.AuthAttemptBackoff(),
            input_lines=queue.Queue(),
            deferred_input_lines=[],
            out=_Out(),
            freshness_stop_event=threading.Event(),
            db_passphrase="remembered-passphrase",
        )

        response, should_shutdown = daemon_runtime.handle_request(
            ctx,
            {"request_id": "lock-1", "kind": "daemon.lock"},
            ctx.out,
        )

        self.assertFalse(should_shutdown)
        self.assertEqual(response["kind"], "daemon.lock")
        self.assertIsNone(ctx.conn)
        self.assertIsNone(ctx.db_passphrase)

    def test_rekey_error_clears_remembered_background_passphrase(self):
        conn = self._db()
        _seed_profile(conn)
        ctx = daemon_runtime.DaemonContext(
            conn=conn,
            data_root="encrypted-data-root",
            runtime_config={},
            active_ai_chats=daemon_runtime.ActiveAiChats(),
            main_thread_tasks=queue.Queue(),
            auth_backoff=daemon_runtime.AuthAttemptBackoff(),
            input_lines=queue.Queue(),
            deferred_input_lines=[],
            out=_Out(),
            freshness_stop_event=threading.Event(),
            db_passphrase="current-passphrase",
        )

        with (
            patch.object(daemon_runtime, "_database_file_is_encrypted", return_value=True),
            patch.object(daemon_runtime, "_verify_passphrase_with_backoff", return_value=True),
            patch.object(
                daemon_runtime,
                "change_database_passphrase",
                side_effect=AppError("rotation failed", code="rotation_failed"),
            ),
        ):
            with self.assertRaises(AppError):
                daemon_runtime.handle_request(
                    ctx,
                    {
                        "request_id": "rekey-1",
                        "kind": "ui.secrets.change_passphrase",
                        "args": {
                            "auth_response": {"passphrase_secret": "current-passphrase"},
                            "new_passphrase_secret": "new-passphrase-123",
                        },
                    },
                    ctx.out,
                )

        self.assertIsNone(ctx.conn)
        self.assertIsNone(ctx.db_passphrase)


if __name__ == "__main__":
    unittest.main()
