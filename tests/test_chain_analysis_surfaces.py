"""Real shared API, CLI and AI boundaries, including adversarial scope changes."""
import contextlib
import io
import json
import queue
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon
from kassiber.ai.tools import get_tool, select_tool_capabilities, tool_capabilities
from kassiber.cli.main import build_parser, dispatch as cli_dispatch
from kassiber.core.chain_analysis_api import dispatch
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError
from tests.test_custody_component_surfaces import _fixture


@pytest.fixture
def book(tmp_path):
    _fixture(tmp_path)
    conn = open_db(str(tmp_path))
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "profile")
    for number, row_id in ((1, "out"), (2, "in")):
        raw = {"txid": f"{number:064x}", "vin": [{"coinbase": "0101"}] if number == 1 else [{"txid": f"{1:064x}", "vout": 0, "prevout": {"value": 100}}], "vout": [{"value": 100 if number == 1 else 90, "scriptpubkey": "0014" + "11" * 20}]}
        conn.execute("UPDATE transactions SET external_id=?,raw_json=? WHERE id=?", (raw["txid"], json.dumps(raw), row_id))
    conn.commit()
    yield conn, tmp_path
    conn.close()


def test_cli_daemon_and_api_return_same_indexed_evidence(book):
    conn, root = book
    request = {"mode": "path", "subject": f"{1:064x}", "target": f"{2:064x}", "direction": "forward"}
    api = dispatch(conn, "ui.chain_analysis.query", request)
    args = build_parser().parse_args(["--data-root", str(root), "--format", "json", "chain-analysis", "path", request["subject"], request["target"], "--direction", "forward"])
    with contextlib.redirect_stdout(io.StringIO()) as output:
        cli_dispatch(conn, args)
    result = json.loads(output.getvalue())
    assert result["kind"] == "chain-analysis.path"
    assert result["data"] == api
    ctx = SimpleNamespace(conn=conn, data_root=str(root), runtime_config={})
    response, stopped = daemon.handle_request(ctx, {"kind": "ui.chain_analysis.query", "request_id": "actual-query", "args": request}, None)
    assert not stopped
    assert response["request_id"] == "actual-query"
    assert response["data"] == api
    assert api["paths"]


def test_read_queries_never_refresh_or_mutate_book(book):
    conn, _ = book
    before = conn.total_changes
    with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=AssertionError("unexpected network")), patch("kassiber.core.sync_backends.http_get_json", side_effect=AssertionError("unexpected network")):
        result = dispatch(conn, "ui.chain_analysis.query", {})
        assert result["nodes"]
        dispatch(conn, "ui.chain_analysis.entropy", {"subject": f"{2:064x}"})
    assert conn.total_changes == before


def test_scope_guard_refuses_case_save_in_changed_book(book):
    conn, _ = book
    result = dispatch(conn, "ui.chain_analysis.query", {})
    with pytest.raises(AppError) as error:
        dispatch(conn, "ui.chain_analysis.cases.save", {"title": "Unsafe scope", "query": {}, "expected_snapshot_id": result["snapshot_id"], "expected_scope": {"workspace_id": "ws", "profile_id": "wrong"}})
    assert error.value.code == "scope_changed"
    assert conn.execute("SELECT COUNT(*) FROM chain_analysis_cases").fetchone()[0] == 0


def test_cli_saved_case_roundtrip_reads_machine_artifact(book):
    conn, root = book
    result = dispatch(conn, "ui.chain_analysis.query", {})
    path = root / "query.json"
    path.write_text(json.dumps({"kind": "query", "schema_version": 1, "data": result["query"]}))
    args = build_parser().parse_args(["--format", "json", "chain-analysis", "cases", "save", "--title", "Observed path", "--query", "@" + str(path), "--expected-snapshot-id", result["snapshot_id"]])
    with contextlib.redirect_stdout(io.StringIO()) as output:
        cli_dispatch(conn, args)
    saved = json.loads(output.getvalue())
    assert saved["kind"] == "chain-analysis.cases.save"
    assert saved["data"]["result"] == result


def test_agent_tools_use_pinned_scope_projection_and_same_mutation_seam(book):
    conn, root = book
    runtime = daemon.AiToolRuntime(str(root), {}, queue.Queue(), {"scope_workspace_id": "ws", "scope_profile_id": "profile", "provider_kind": "openai", "provider_on_device": False})
    query = daemon.ParsedAiToolCall("query", "ui.chain_analysis.query", {})
    with patch("kassiber.daemon._run_on_daemon_main_thread", side_effect=lambda runtime, callback: callback(conn)):
        result = daemon._execute_read_only_ai_tool(query, runtime)
        assert result["ok"], result
        data = result["envelope"]["data"]
        assert f"{1:064x}" not in json.dumps(data)
        assert data["nodes"] and data["edges"]
        save = daemon.ParsedAiToolCall("save", "ui.chain_analysis.cases.save", {"title": "Agent investigation", "query": data["query"], "expected_snapshot_id": data["snapshot_id"]})
        saved = daemon._execute_mutating_ai_tool(save, runtime)
        assert saved["ok"], saved
        assert conn.execute("SELECT COUNT(*) FROM chain_analysis_cases").fetchone()[0] == 1
        runtime.maintenance_state["scope_profile_id"] = "other-book"
        blocked = daemon._execute_read_only_ai_tool(query, runtime)
        assert not blocked["ok"]


def test_cloud_and_lan_local_models_cannot_acquire(book):
    conn, root = book
    for kind in ("openai", "local"):
        runtime = daemon.AiToolRuntime(str(root), {}, queue.Queue(), {"provider_kind": kind, "provider_on_device": False})
        with pytest.raises(AppError) as error:
            daemon._chain_analysis_ai_payload(conn, runtime, "ui.chain_analysis.acquire.apply", {"plan": {}})
        assert error.value.code == "local_provider_required"
    assert get_tool("ui.chain_analysis.acquire.apply").egresses
    assert get_tool("ui.chain_analysis.acquire.apply").requires_consent
    assert not get_tool("ui.chain_analysis.acquire.plan").requires_consent
    assert "ui.chain_analysis.acquire.apply" in daemon.AI_TOOL_ONCE_ONLY_CONSENT


def test_chain_investigation_questions_advertise_relevant_catalog():
    for text in ("Trace these transactions", "lokale Chainanalyse", "Payjoin entropy", "chainanalysis"):
        caps = select_tool_capabilities([{"role": "user", "content": text}])
        assert "privacy" in caps
    assert "privacy" in tool_capabilities(get_tool("ui.chain_analysis.query"))


def test_desktop_assistant_context_is_version_bound_and_has_no_chain_identity(book):
    conn, _ = book
    result = dispatch(conn, "ui.chain_analysis.query", {"mode": "trace", "subject": f"{1:064x}"})
    context = dispatch(conn, "ui.chain_analysis.ai_context", {"query": result["query"], "subject": result["nodes"][0]["id"], "expected_snapshot_id": result["snapshot_id"]})
    assert f"{1:064x}" not in json.dumps(context)
    assert context["query"]["subject"].startswith("ca-ref:")
    with pytest.raises(AppError) as error:
        dispatch(conn, "ui.chain_analysis.ai_context", {"query": result["query"], "expected_snapshot_id": "0" * 64})
    assert error.value.code == "chain_analysis_stale"
    assert get_tool("ui.chain_analysis.ai_context") is None  # desktop handoff only


def test_remote_error_does_not_leak_raw_subject_from_saved_recipe(book):
    conn, root = book
    runtime = daemon.AiToolRuntime(str(root), {}, queue.Queue(), {"provider_on_device": False})
    with patch("kassiber.core.chain_analysis_api.dispatch", side_effect=AppError("Missing " + "f" * 64, code="not_found", details={"subject": "f" * 64})):
        with pytest.raises(AppError) as error:
            daemon._chain_analysis_ai_payload(conn, runtime, "ui.chain_analysis.cases.compare", {"id": "case"})
    assert "f" * 64 not in str(error.value)
    assert error.value.details is None


def test_acquisition_consent_displays_only_recomputed_effects(book):
    conn, root = book
    runtime = daemon.AiToolRuntime(str(root), {}, queue.Queue(), {"provider_on_device": True})
    genuine = {"args": {"subject": f"{1:064x}"}, "effects": {"egresses": True, "max_transactions": 50}}
    fabricated = {**genuine, "effects": {"egresses": False, "max_transactions": 0}}
    with patch("kassiber.daemon._run_on_daemon_main_thread", side_effect=lambda runtime, callback: callback(conn)), patch("kassiber.core.chain_analysis_acquisition.plan_acquisition", return_value=genuine):
        assert daemon._chain_analysis_acquisition_consent_preview(runtime, {"plan": fabricated}) == {"status": "blocked", "code": "chain_analysis_stale"}
        assert daemon._chain_analysis_acquisition_consent_preview(runtime, {"plan": genuine}) == {"status": "ready", "plan": genuine}


def test_book_reset_removes_investigations_and_reference_history(book):
    from kassiber.core.maintenance import reset_current_profile_data
    conn, root = book
    result = dispatch(conn, "ui.chain_analysis.query", {})
    dispatch(conn, "ui.chain_analysis.cases.save", {"title": "Before reset", "query": {}, "expected_snapshot_id": result["snapshot_id"]})
    dispatch(conn, "ui.chain_analysis.labels.upsert", {"subject": f"{1:064x}", "chain": "bitcoin", "network": "main", "label": "Local claim", "category": "other", "source": "User supplied record", "confidence": "unverified"})
    conn.execute("INSERT INTO chain_analysis_observations VALUES(?,?,?,?,?,?,?,?)", ("profile", "bitcoin", "main", f"{1:064x}", json.dumps({"txid": f"{1:064x}", "vin": [{"is_coinbase": True}], "vout": [{"value": 100}]}), "{}", "test node", "2026-09-06T12:00:00Z"))
    conn.commit()
    reset = reset_current_profile_data(conn, str(root))
    for table in ("chain_analysis_cases", "chain_analysis_labels", "chain_analysis_label_history", "chain_analysis_observations"):
        assert reset["removed"][table] == 1
        assert conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] == 0
    assert not dispatch(conn, "ui.chain_analysis.query", {})["nodes"]
