"""Handled freshness failures remain diagnosable without wallet data in RAM logs."""
import json
import logging
from unittest.mock import Mock, patch

import pytest

from kassiber import daemon_freshness
from kassiber.core import freshness
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError
from kassiber.log_ring import LogRing, RingHandler, current_request_id


@pytest.mark.parametrize("failure", ["typed", "untyped", "custom_type", "unknown_code"])
def test_failed_decode_logs_categorical_context_without_private_values(tmp_path, failure):
    conn = open_db(tmp_path)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Private','2026-01-01')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','Book','EUR','2026-01-01')")
    wallet = "private-wallet-label"
    secret = "https://user:password@private-node.example/rpc"
    txid = "a" * 64
    freshness.enqueue_job(
        conn, profile_id="p", job_type=freshness.JOB_ONCHAIN_WALLET,
        source_key="onchain_wallet:" + wallet, source_type=freshness.SOURCE_ONCHAIN,
        source_label=wallet, priority=10,
    )
    conn.commit()

    def fail(conn, job, progress, check_cancelled):
        progress({"phase": freshness.PHASE_DECODE_ENRICH, "wallet": wallet})
        progress({"phase": wallet})  # Unknown phase text must not enter diagnostics.
        if failure == "untyped":
            raise ValueError(secret + txid)
        if failure == "custom_type":
            raise type(wallet, (ValueError,), {})(secret + txid)
        raise AppError(secret + txid, code=wallet if failure == "unknown_code" else "observer_state_invalid", details={
            "error_class": wallet,
            "sqlite_error_name": secret,
            "traceback": txid,
        })

    ring = LogRing()
    handler = RingHandler(ring)
    logger = logging.getLogger("kassiber.core.freshness")
    logger.addHandler(handler)
    token = current_request_id.set("refresh-fixture")
    try:
        jobs = freshness.run_due_jobs(conn, {freshness.JOB_ONCHAIN_WALLET: fail}, profile_id="p", limit=1)
    finally:
        current_request_id.reset(token)
        logger.removeHandler(handler)
        conn.close()
    assert jobs[0]["status"] == freshness.JOB_ERROR
    records = ring.snapshot()["records"]
    assert len(records) == 1
    record = records[0]
    fields = {key: value["value"] for key, value in record["fields"].items()}
    assert fields["phase"] == "decode_enrich"
    assert fields["job_type"] == freshness.JOB_ONCHAIN_WALLET
    assert fields["error_code"] == ("observer_state_invalid" if failure == "typed" else "freshness_job_failed")
    assert fields["error_class"] == ("builtins.ValueError" if failure in {"untyped", "custom_type"} else "kassiber.errors.AppError")
    assert fields["request_id"] == "refresh-fixture"
    encoded = json.dumps(records)
    for private in (wallet, secret, "private-node", txid, "password"):
        assert private not in encoded


def test_refresh_that_becomes_ineligible_after_prefetch_logs_deferred_summary(tmp_path):
    conn = open_db(tmp_path)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Private','2026-01-01')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','Book','EUR','2026-01-01')")
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('w','ws','p','private-wallet','address','{\"addresses\":[\"bc1qfixture\"]}','2026-01-01')")
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "p")
    conn.commit()
    events = []

    def prefetch(*args, **kwargs):
        daemon_freshness.sync_progress_emitter.get()({"phase": freshness.PHASE_DECODE_ENRICH})
        freshness.pause_source(conn, "p", freshness.source_key(freshness.SOURCE_ONCHAIN, "w"))
        conn.commit()
        return {"w": object()}

    ring = LogRing()
    handler = RingHandler(ring)
    logger = logging.getLogger("kassiber.daemon.freshness")
    original_level = logger.level
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)
    try:
        with patch.object(daemon_freshness, "prefetch_wallets_from_backend", side_effect=prefetch), patch.object(daemon_freshness, "sync_wallet_from_backend") as apply:
            payload = daemon_freshness._freshness_run_payload(conn, {}, {"all": True, "rates": False, "journals": False}, progress_observer=events.append)
        apply.assert_not_called()
    finally:
        logger.removeHandler(handler)
        logger.setLevel(original_level)
        conn.close()
    assert events[-1]["phase"] == "decode_enrich"
    assert payload["completed"] == []
    records = ring.snapshot()["records"]
    assert len(records) == 1
    fields = {key: item["value"] for key, item in records[0]["fields"].items()}
    assert fields["jobs_selected"] == "1"
    assert fields["jobs_completed"] == "0"
    assert fields["jobs_deferred"] == "1"
    assert "private-wallet" not in json.dumps(records)


def _refresh_fixture(tmp_path):
    conn = open_db(tmp_path)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Private','2026-01-01')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','Book','EUR','2026-01-01')")
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('w','ws','p','private-wallet','address','{\"addresses\":[\"bc1qfixture\"]}','2026-01-01')")
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "p")
    conn.commit()
    return conn


@pytest.mark.parametrize("status", ["done", "error", "cancelled"])
def test_refresh_returns_requested_job_finished_during_prefetch(tmp_path, status):
    conn = _refresh_fixture(tmp_path)

    def finish(*args):
        if status != "done":
            raise AppError("Fixture failure", code="cancelled" if status == "cancelled" else "observer_state_invalid")
        return {"wallet": "private-wallet", "status": "synced"}

    def prefetch(*args, **kwargs):
        # A separate executor finishes the committed queued job while the
        # foreground caller is waiting for network I/O.
        jobs = freshness.run_due_jobs(conn, {freshness.JOB_ONCHAIN_WALLET: finish}, profile_id="p", limit=1)
        assert jobs[0]["status"] == status
        return {"w": object()}

    try:
        with patch.object(daemon_freshness, "prefetch_wallets_from_backend", side_effect=prefetch), patch.object(daemon_freshness, "sync_wallet_from_backend") as apply:
            result = daemon_freshness._freshness_run_payload(conn, {}, {"all": True, "rates": False, "journals": False})
        apply.assert_not_called()
        assert len(result["completed"]) == 1
        assert result["completed"][0]["id"] == result["enqueued"][0]["id"]
        assert result["completed"][0]["status"] == status
        assert result["results"][0]["status"] == ("synced" if status == "done" else "error")
        assert result["ok"] is (status == "done")
    finally:
        conn.close()


def test_cached_prefetch_failure_logs_its_origin_not_apply_phase(tmp_path):
    conn = _refresh_fixture(tmp_path)
    failure = AppError("Private decode failure", code="observer_state_invalid")
    ring = LogRing()
    handler = RingHandler(ring)
    logger = logging.getLogger("kassiber.core.freshness")
    logger.addHandler(handler)

    def prefetch(*args, **kwargs):
        daemon_freshness.sync_progress_emitter.get()({"phase": freshness.PHASE_DECODE_ENRICH})
        return {"w": failure}

    def apply(*args, **kwargs):
        raise kwargs["prefetched"]["w"]

    try:
        with patch.object(daemon_freshness, "prefetch_wallets_from_backend", side_effect=prefetch), patch.object(daemon_freshness, "sync_wallet_from_backend", side_effect=apply):
            result = daemon_freshness._freshness_run_payload(conn, {}, {"all": True, "rates": False, "journals": False}, progress_observer=lambda event: None)
        assert result["ok"] is False
        record = ring.snapshot()["records"][0]
        assert record["fields"]["phase"]["value"] == "batch_prefetch"
        assert "Private decode failure" not in json.dumps(record)
    finally:
        logger.removeHandler(handler)
        conn.close()


@pytest.mark.parametrize("stage", ["prefetch", "running"])
def test_active_refresh_excludes_background_recovery_and_foreground_dispatch(tmp_path, stage):
    import threading

    conn = _refresh_fixture(tmp_path)
    freshness.set_policy(conn, "p", background_enabled=True)
    conn.commit()
    checked = threading.Event()
    failures = []

    def contender():
        other = open_db(tmp_path)
        try:
            with patch.object(daemon_freshness, "_enqueue_freshness_jobs") as enqueue, patch.object(freshness, "recover_interrupted_jobs") as recover:
                daemon_freshness._freshness_background_tick(other, {}, Mock())
                enqueue.assert_not_called()
                recover.assert_not_called()
                for invoke in (
                    lambda: daemon_freshness._freshness_run_payload(other, {}, {"all": True}),
                    lambda: daemon_freshness._workspace_freshness_run_payload(other, {}, {"workspace_id": "ws"}),
                ):
                    with pytest.raises(AppError) as error:
                        invoke()
                    assert error.value.code == "project_operation_in_progress"
                    assert error.value.retryable is True
                state = {}
                # Opted-out report reads remain a no-op, even during refresh.
                freshness.set_policy(other, "p", report_read_sync=False)
                other.commit()
                assert daemon_freshness._auto_sync_wallets_if_enabled(other, {}, state=state) is None
                assert state == {}
                result = daemon_freshness._auto_sync_wallets_if_enabled(other, {}, state=state, force=True)
                assert result["ok"] is False
                assert result["reason"] == "project_operation_in_progress"
                assert state["auto_sync"] == result
        except BaseException as exc:
            failures.append(exc)
        finally:
            other.close()
            checked.set()

    def apply(*args, **kwargs):
        before = freshness.list_jobs(conn, "p")[0]
        expected_status = freshness.JOB_RUNNING if stage == "running" else freshness.JOB_QUEUED
        expected_attempts = 1 if stage == "running" else 0
        assert before["status"] == expected_status
        worker = threading.Thread(target=contender)
        worker.start()
        assert checked.wait(timeout=10), "competing refresh must return without waiting for network I/O"
        worker.join(timeout=1)
        if failures:
            raise failures[0]
        after = freshness.list_jobs(conn, "p")[0]
        assert (after["status"], after["attempts"]) == (expected_status, expected_attempts)
        return {}

    try:
        with patch.object(daemon_freshness, "prefetch_wallets_from_backend", side_effect=apply if stage == "prefetch" else None, return_value={}), patch.object(daemon_freshness, "sync_wallet_from_backend", side_effect=apply if stage == "running" else None, return_value={}):
            result = daemon_freshness._freshness_run_payload(conn, {}, {"all": True, "rates": False, "journals": False})
        assert not failures
        assert result["ok"] is True
        assert result["completed"][0]["attempts"] == 1
        # A subsequent request succeeds, proving the execution slot was released.
        with patch.object(daemon_freshness, "prefetch_wallets_from_backend", return_value={}), patch.object(daemon_freshness, "sync_wallet_from_backend", return_value={}):
            assert daemon_freshness._freshness_run_payload(conn, {}, {"all": True, "rates": False, "journals": False})["ok"] is True
    finally:
        conn.close()


def test_completed_collection_respects_requested_ids_limit_and_running_state(tmp_path):
    conn = _refresh_fixture(tmp_path)
    jobs = [freshness.enqueue_job(
        conn, profile_id="p", job_type=freshness.JOB_ONCHAIN_WALLET,
        source_key=f"onchain_wallet:{number}", source_type=freshness.SOURCE_ONCHAIN,
        source_label=f"Source {number}", priority=number,
    ) for number in range(4)]
    handlers = {freshness.JOB_ONCHAIN_WALLET: lambda *args: {"status": "synced"}}
    try:
        for job in jobs[:3]:
            freshness.run_job(conn, job["id"], handlers)
        conn.execute("UPDATE freshness_jobs SET status='running' WHERE id=?", (jobs[3]["id"],))
        conn.commit()
        with patch.object(freshness, "run_job") as run:
            completed = daemon_freshness._run_requested_freshness_jobs(conn, "p", jobs[1:], handlers, limit=1)
            assert [job["id"] for job in completed] == [jobs[1]["id"]]
            assert daemon_freshness._run_requested_freshness_jobs(conn, "p", jobs[3:], handlers, limit=1) == []
            run.assert_not_called()
    finally:
        conn.close()
