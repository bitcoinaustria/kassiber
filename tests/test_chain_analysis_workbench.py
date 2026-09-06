"""Evidence survives the complete desktop/CLI/agent workbench boundary."""
import contextlib
import hashlib
import io
import json
import queue
import threading
import time
from unittest.mock import patch

import pytest

from kassiber import daemon
from kassiber.cli.main import build_parser, dispatch as cli_dispatch
from kassiber.core.chain_analysis_api import dispatch
from kassiber.core.chain_analysis_ai import project_ai_result
from kassiber.core.chain_analysis_runtime import SOURCES, scope_key, clear_runtime
from kassiber.core import chain_analysis_datasets as datasets
from kassiber.daemon_chain_analysis import job_starter
from kassiber.errors import AppError
from tests.test_chain_analysis_surfaces import book
from tests.test_chain_analysis_datasets import manifest
from tests.test_chain_analysis_psbt_features import sample


@pytest.fixture(autouse=True)
def isolated_runtime():
    clear_runtime()
    yield
    clear_runtime()


def terminal(conn, receipt):
    deadline = time.monotonic() + 5
    while receipt["status"] == "running" and time.monotonic() < deadline:
        time.sleep(.005)
        receipt = dispatch(conn, "ui.chain_analysis.jobs.get", {"job_id": receipt["job_id"]})
    assert receipt["status"] != "running", receipt
    return receipt


def done(conn, receipt):
    receipt = terminal(conn, receipt)
    assert receipt["status"] == "completed", receipt
    return receipt["result"]


def test_psbt_file_cli_native_grant_agent_projection_and_shared_entropy(book):
    conn, root = book
    path = root / "private-wallet-proposal.psbt"
    path.write_text(sample().to_base64())
    grant = SOURCES.stage(scope_key(conn, "profile"), str(path), "psbt")
    args = {"psbt_token": grant["source_token"], "network": "regtest"}
    result = dispatch(conn, "ui.chain_analysis.psbt.analyze", args)
    assert result["totals"]["fee_msat"] == "1000000"
    parsed = build_parser().parse_args(["--format", "json", "chain-analysis", "psbt", "analyze", "--file", str(path), "--network", "regtest"])
    with contextlib.redirect_stdout(io.StringIO()) as output:
        cli_dispatch(conn, parsed)
    assert json.loads(output.getvalue())["data"] == result
    entropy = done(conn, dispatch(conn, "ui.chain_analysis.psbt.entropy.start", {**args, "max_states": 2000000}))
    assert entropy["status"] == "exact" and entropy["interpretation_count"] == "1"
    projected = project_ai_result(conn, "profile", result)
    assert projected["totals"]["fee_msat"] == "1000000"
    assert projected["features"]["features"]
    assert str(path) not in json.dumps(projected) and path.read_text() not in json.dumps(projected)
    assert result["transaction_facts"]["inputs"][0]["output_id"] not in json.dumps(projected)
    with pytest.raises(AppError) as error:
        dispatch(conn, "ui.chain_analysis.psbt.analyze", {**args, "expected_scope": {"workspace_id": "ws", "profile_id": "other"}})
    assert error.value.code == "scope_changed"
    clear_runtime()


def test_background_dataset_preview_import_public_exposure_and_case_revocation(book):
    conn, root = book
    path = root / "research.csv"
    path.write_text(f"subject,entity,source\n{1:064x},Public Exchange,Published source\n")
    grant = SOURCES.stage(scope_key(conn, "profile"), str(path), "dataset")
    args = {"source_token": grant["source_token"], "manifest": manifest()}
    starter = job_starter(str(root))
    preview = done(conn, dispatch(conn, "ui.chain_analysis.datasets.preview.start", args, job_starter=starter))
    assert preview["row_count"] == 1
    assert conn.execute("SELECT count(*) FROM chain_analysis_datasets").fetchone()[0] == 0
    pack = done(conn, dispatch(conn, "ui.chain_analysis.datasets.import.start", {**args, "expected_sha256": preview["sha256"]}, job_starter=starter))
    assert pack["status"] == "active"
    query = {"mode": "trace", "subject": f"{1:064x}", "observer": "public"}
    with patch("kassiber.core.chain_analysis_datasets.now_iso", return_value="2026-09-06T10:00:00Z"):
        first = dispatch(conn, "ui.chain_analysis.query", query)
    with patch("kassiber.core.chain_analysis_datasets.now_iso", return_value="2026-09-06T10:00:02Z"):
        second = dispatch(conn, "ui.chain_analysis.query", query)
    assert first["snapshot_id"] == second["snapshot_id"]
    assert first["exposure"][0]["claim"]["dataset_id"] == pack["id"]
    saved = dispatch(conn, "ui.chain_analysis.cases.save", {"query": query, "title": "Sourced contact", "expected_snapshot_id": first["snapshot_id"]})
    dispatch(conn, "ui.chain_analysis.datasets.revoke", {"id": pack["id"], "expected_revision": pack["revision"]})
    changed = dispatch(conn, "ui.chain_analysis.cases.compare", {"id": saved["id"]})
    assert changed["changed"] and changed["removed_exposure"]
    assert dispatch(conn, "ui.chain_analysis.cases.get", {"id": saved["id"]})["result"]["exposure"]
    with pytest.raises(AppError):
        dispatch(conn, "ui.chain_analysis.datasets.discard", {"id": pack["id"]})
    clear_runtime()


def test_native_analysis_stage_is_scope_bound_and_never_a_renderer_path_tool(book):
    conn, root = book
    from types import SimpleNamespace
    ctx = SimpleNamespace(conn=conn, data_root=str(root), runtime_config={})
    path = root / "source.psbt"
    path.write_bytes(sample().serialize())
    response, _ = daemon.handle_request(ctx, {"kind": "internal.chain_analysis.stage", "request_id": "stage", "args": {"source_file": str(path), "purpose": "psbt", "expected_scope": {"workspace_id": "ws", "profile_id": "profile"}}}, None)
    assert response["data"]["source_token"]
    assert str(path) not in json.dumps(response)
    from kassiber.ai.tools import get_tool
    assert get_tool("internal.chain_analysis.stage") is None
    clear_runtime()


def dataset_source(conn, root):
    path = root / "research.csv"
    content = f"subject,entity,source\n{1:064x},Public Exchange,Published source\n".encode()
    path.write_bytes(content)
    token = SOURCES.stage(scope_key(conn, "profile"), str(path), "dataset")["source_token"]
    return path, {"source_token": token, "manifest": manifest(), "expected_sha256": hashlib.sha256(content).hexdigest()}


def on_device_runtime(root, starter=None):
    return daemon.AiToolRuntime(str(root), {}, queue.Queue(), {
        "scope_workspace_id": "ws", "scope_profile_id": "profile", "provider_kind": "local",
        "provider_on_device": True, "chain_analysis_job_starter": starter,
    })


@pytest.mark.parametrize("operation", ["datasets.import", "datasets.import.start"])
def test_ai_import_without_completed_preview_is_blocked_before_dispatch(book, operation):
    conn, root = book
    _, args = dataset_source(conn, root)
    runtime = on_device_runtime(root)
    with patch("kassiber.daemon._run_on_daemon_main_thread", side_effect=lambda runtime, callback: callback(conn)):
        assert daemon._chain_analysis_dataset_consent_preview(runtime, args) == {"status": "blocked", "code": "chain_analysis_preview_required"}
    with patch("kassiber.core.chain_analysis_api.dispatch", side_effect=AssertionError("unreviewed import dispatched")):
        with pytest.raises(AppError) as error:
            daemon._chain_analysis_ai_payload(conn, runtime, f"ui.chain_analysis.{operation}", args)
    assert error.value.code == "chain_analysis_preview_required"
    assert conn.execute("SELECT count(*) FROM chain_analysis_datasets").fetchone()[0] == 0


def test_ai_consent_and_start_reuse_exact_completed_preview_without_reading_source(book):
    conn, root = book
    _, args = dataset_source(conn, root)
    preview_args = {key: value for key, value in args.items() if key != "expected_sha256"}
    preview = done(conn, dispatch(conn, "ui.chain_analysis.datasets.preview.start", preview_args, job_starter=job_starter(str(root))))
    calls = []
    def start(connection, operation, submitted, *, profile_id):
        calls.append((connection, operation, submitted, profile_id))
        return {"job_id": "background-work", "status": "running"}
    runtime = on_device_runtime(root, start)
    original_open = SOURCES.open
    @contextlib.contextmanager
    def metadata_only(*values):
        with original_open(*values):
            class NoReads:
                def read(self, *_):
                    raise AssertionError("consent read source bytes")
                def readline(self, *_):
                    raise AssertionError("consent scanned source rows")
            yield NoReads()
    with patch.object(SOURCES, "open", metadata_only), patch.object(datasets, "preview_dataset", side_effect=AssertionError("consent repeated full preview")), patch("kassiber.daemon._run_on_daemon_main_thread", side_effect=lambda runtime, callback: callback(conn)):
        consent = daemon._chain_analysis_dataset_consent_preview(runtime, args)
        assert consent["status"] == "ready" and consent["sha256"] == preview["sha256"]
        assert consent["row_count"] == 1
        result = daemon._chain_analysis_ai_payload(conn, runtime, "ui.chain_analysis.datasets.import.start", args)
    assert result["status"] == "running"
    assert calls == [(conn, "datasets.import.start", args, "profile")]


@pytest.mark.parametrize("change", ["manifest", "format", "adapter", "hash", "source", "bytes"])
def test_ai_preview_and_execution_reject_changed_reviewed_inputs(book, change):
    conn, root = book
    path, args = dataset_source(conn, root)
    dispatch(conn, "ui.chain_analysis.datasets.preview", {key: value for key, value in args.items() if key != "expected_sha256"})
    if change == "manifest":
        args["manifest"] = manifest(license="Changed declaration")
    elif change in {"format", "adapter"}:
        args[change] = "jsonl" if change == "format" else "am_i_exposed"
    elif change == "hash":
        args["expected_sha256"] = "0" * 64
    elif change == "source":
        args["source_token"] = SOURCES.stage(scope_key(conn, "profile"), str(path), "dataset")["source_token"]
    else:
        path.write_text("changed file contents\n")
    runtime = on_device_runtime(root)
    with patch("kassiber.daemon._run_on_daemon_main_thread", side_effect=lambda runtime, callback: callback(conn)):
        consent = daemon._chain_analysis_dataset_consent_preview(runtime, args)
    assert consent["status"] == "blocked"
    with patch("kassiber.core.chain_analysis_api.dispatch", side_effect=AssertionError("changed import dispatched")):
        with pytest.raises(AppError):
            daemon._chain_analysis_ai_payload(conn, runtime, "ui.chain_analysis.datasets.import.start", args)


def test_source_change_during_eof_read_prevents_dataset_activation(book):
    import os
    conn, root = book
    path, args = dataset_source(conn, root)
    original_fdopen = os.fdopen

    class ChangeAtEOF:
        def __init__(self, stream):
            self.stream = stream
        def __enter__(self):
            return self
        def __exit__(self, *values):
            return self.stream.__exit__(*values)
        def fileno(self):
            return self.stream.fileno()
        def readline(self, size=-1):
            value = self.stream.readline(size)
            if not value:
                path.write_bytes(b"changed immediately before EOF verification")
            return value

    with patch("kassiber.core.chain_analysis_runtime.os.fdopen", side_effect=lambda *values: ChangeAtEOF(original_fdopen(*values))):
        with pytest.raises(AppError) as error:
            dispatch(conn, "ui.chain_analysis.datasets.import", args)
    assert error.value.code == "chain_analysis_source_changed"
    assert conn.execute("SELECT status FROM chain_analysis_datasets").fetchone()[0] == "failed"
    assert conn.execute("SELECT count(*) FROM chain_analysis_datasets WHERE status='active'").fetchone()[0] == 0


@pytest.mark.parametrize("cancel_after_commit", [False, True])
def test_async_import_cancellation_reports_durable_outcome_and_failed_staging_can_be_discarded(book, cancel_after_commit):
    conn, root = book
    path, args = dataset_source(conn, root)
    entered, release = threading.Event(), threading.Event()
    original_import = datasets.import_dataset
    def controlled_import(*values, **options):
        if not cancel_after_commit:
            original_progress = options["progress"]
            def progress(value):
                original_progress(value)
                entered.set()
                assert release.wait(5), "test did not release staged import"
            options["progress"] = progress
        result = original_import(*values, **options)
        if cancel_after_commit:
            # Commit already happened. A later source revision and cancel must
            # not retroactively turn that durable success into an error.
            path.write_bytes(b"next source revision")
            entered.set()
            assert release.wait(5), "test did not release committed import"
        return result
    starter = job_starter(str(root))
    with patch.object(datasets, "import_dataset", controlled_import):
        receipt = dispatch(conn, "ui.chain_analysis.datasets.import.start", args, job_starter=starter)
        try:
            assert entered.wait(5), receipt
            active = conn.execute("SELECT count(*) FROM chain_analysis_datasets WHERE status='active'").fetchone()[0]
            assert active == int(cancel_after_commit)
            requested = dispatch(conn, "ui.chain_analysis.jobs.cancel", {"job_id": receipt["job_id"]})
            assert requested["cancel_requested"]
        finally:
            release.set()
        finished = terminal(conn, receipt)
    if cancel_after_commit:
        assert finished["status"] == "completed" and finished["result"]["status"] == "active"
        assert conn.execute("SELECT count(*) FROM chain_analysis_datasets WHERE status='active'").fetchone()[0] == 1
    else:
        assert finished["status"] == "cancelled"
        pack = conn.execute("SELECT id,status,row_count FROM chain_analysis_datasets").fetchone()
        assert pack["status"] == "failed" and pack["row_count"] == 1
        assert conn.execute("SELECT count(*) FROM chain_analysis_datasets WHERE status='active'").fetchone()[0] == 0
        cleanup = terminal(conn, dispatch(conn, "ui.chain_analysis.datasets.discard.start", {"id": pack["id"]}, job_starter=starter))
        assert cleanup["status"] == "completed" and cleanup["result"]["discarded"]
        assert cleanup["progress"]["phase"] == "discarding" and cleanup["progress"]["rows_deleted"] == 1
        assert conn.execute("SELECT count(*) FROM chain_analysis_dataset_claims").fetchone()[0] == 0
