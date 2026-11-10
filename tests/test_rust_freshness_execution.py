import pytest
from tests.test_rust_freshness_admission import backend, config_sql
from tests.test_rust_wallet_discovery import _config

from tests.test_rust_freshness_queue import FIXTURE, enqueue, op, parity, source

pytestmark = pytest.mark.skipif(not FIXTURE, reason="requires the Rust freshness execution fixture")


def run(steps=(), result=None, **args):
    return op("run_job", id="job-0001", handlers={"onchain_wallet_history": {
        "steps": list(steps), "result": result if result is not None else {},
    }}, **args)


def emit(phase, **values):
    return {"op": "emit", "value": {"phase": phase, **values}}


def fail(code="backend_failed", message="failed", **values):
    return {"op": "fail", "code": code, "message": message, **values}


@pytest.mark.parametrize("partial,blocked", [(False, False), (True, False), (True, True)])
def test_freshness_execution_success_checkpoint_and_progress(tmp_path, partial, blocked):
    actual = parity(tmp_path, [
        op("upsert", **source(status="fresh", checkpoint={"old": ["private"]}, retry_count=3)),
        enqueue(payload={"wallet_label": "Cold"}),
        run([emit("backend_fetch", token="secret", checkpoint={"secret": "hidden"}), emit("import")],
            {"wallet": "Cold", "status": "synced", "freshness_checkpoint": {"new": [1, 2]},
             "partial_success": partial, "blocking_reports": blocked, "backend_url": "https://hidden"}),
        op("snapshot", profile_id="p"), op("ui_snapshot", profile_id="p"),
        op("run_job", id="job-0001", handlers={}),
    ])
    output = actual[2]["data"]["output"]
    assert output["status"] == "done" and output["attempts"] == 1
    assert "freshness_checkpoint" not in output["result"]
    assert actual[3]["data"]["sources"][0]["checkpoint"] == {"new": [1, 2]}


@pytest.mark.parametrize("checkpoint", [None, [], "invalid"])
def test_freshness_execution_preserves_checkpoint_without_mapping(tmp_path, checkpoint):
    parity(tmp_path, [op("upsert", **source(status="fresh", checkpoint={"keep": True})), enqueue(),
        run(result={"freshness_checkpoint": checkpoint}), op("snapshot", profile_id="p")])


@pytest.mark.parametrize("details", [
    {"retry_after_seconds": 12}, {"retry_after": "12", "retry_reason": "provider"},
    {"retry_after_seconds": 0}, {"retry_after": 0}, {"retry_after": -12},
    {"retry_after_seconds": "invalid"}, {"retry_after": 1.9}, {"retry_after": True},
    {"retry_after_seconds": 0, "retry_after": 9},
])
def test_freshness_execution_retry_after_matches_python(tmp_path, details):
    parity(tmp_path, [enqueue(), run([fail(details=details, retryable=True)]),
                     op("snapshot", profile_id="p"), op("due", profile_id="p")])


@pytest.mark.parametrize("attempts", [0, 1, 2, 7, 64])
@pytest.mark.parametrize("code,message", [("rate_limited", "slow"), ("backend_failed", "HTTP 429")])
def test_freshness_execution_exponential_backoff(tmp_path, attempts, code, message):
    parity(tmp_path, [enqueue(), op("sql", sql=f"UPDATE freshness_jobs SET attempts={attempts}"),
                     run([fail(code, message)]), op("snapshot", profile_id="p")])


@pytest.mark.parametrize("code", [5, 6, 261, 517, 773, 262, 518])
def test_freshness_execution_database_busy_rolls_back_and_retries(tmp_path, code):
    actual = parity(tmp_path, [enqueue(), run([
        {"op": "sql", "sql": "INSERT INTO settings(key,value) VALUES('must_rollback','pending')"},
        {"op": "database", "code": code, "message": "database is locked"},
    ]), op("snapshot", profile_id="p")])
    output = actual[1]["data"]["output"]
    assert output["error"]["code"] == "database_busy" and output["status"] == "rate_limited"


def test_freshness_execution_errors_and_ui_redaction(tmp_path):
    txid = "ab" * 32
    actual = parity(tmp_path, [
        enqueue(), run([fail(message=f"failed https://user:pass@private.example/a: {txid}:0 host='private.example'",
                            hint="try tcp://secret.example!", details={"url": "https://hidden", "token": "secret",
                            "preview": f"lookup {txid}:1", "nested": ["host=private.example"]})]),
        op("ui_snapshot", profile_id="p"),
        op("ui_payload", results=[{"status": "error", "message": "rpc://secret!"}],
           backend_url="ssl://secret", checkpoint={"private": True}),
    ])
    assert "private.example" not in str(actual[2]["data"])
    assert actual[3]["data"]["ok"] is False


@pytest.mark.parametrize("kind", ["app", "unexpected", "unexpected-empty", "lookalike"])
def test_freshness_execution_generic_failure_and_handler_not_found(tmp_path, kind):
    step = (fail("not_found", "handler row missing") if kind == "app" else
            fail("state_not_ready", "Freshness job disappeared") if kind == "lookalike" else
            {"op": "unexpected", "class": "builtins.ValueError", "message": "" if kind == "unexpected-empty" else "bad handler value"})
    parity(tmp_path, [enqueue(), run([step]), op("snapshot", profile_id="p")])


@pytest.mark.parametrize("missing_ok", [False, True])
@pytest.mark.parametrize("committed", [False, True])
def test_freshness_execution_removed_job_never_resurrects_source(tmp_path, missing_ok, committed):
    parity(tmp_path, [enqueue(), run([
        {"op": "sql", "sql": "DELETE FROM freshness_jobs"},
        {"op": "sql", "sql": "DELETE FROM freshness_source_states"},
        *([{"op": "commit"}] if committed else []),
        emit("import"),
    ], missing_ok=missing_ok), op("snapshot", profile_id="p")])


@pytest.mark.parametrize("during_handler", [False, True])
def test_freshness_execution_cancellation_checked_before_and_after_handler(tmp_path, during_handler):
    control = {"op": "sql", "sql": "UPDATE freshness_jobs SET cancel_requested=1"}
    steps = [control, emit("import")] if during_handler else []
    setup = [op("sql", sql=control["sql"])] if not during_handler else []
    actual = parity(tmp_path, [enqueue(), *setup, run(steps), op("snapshot", profile_id="p")])
    assert actual[-2]["data"]["output"]["status"] == "cancelled"


def test_freshness_execution_retry_success_retains_attempt_history(tmp_path):
    actual = parity(tmp_path, [enqueue(), run([fail("rate_limited", "wait")]),
        op("sql", sql="UPDATE freshness_jobs SET run_after=NULL,cooldown_until=NULL"),
        run(result={"wallet": "Cold", "status": "synced"}), op("snapshot", profile_id="p")])
    assert actual[3]["data"]["output"]["attempts"] == 2
    assert actual[4]["data"]["sources"][0]["retry_count"] == 0


def test_freshness_execution_missing_job_and_handler_are_distinct(tmp_path):
    parity(tmp_path, [op("run_job", id="missing", handlers={}, missing_ok=True), enqueue(),
                     op("run_job", id="job-0001", handlers={}),
                     op("run_job", id="missing", handlers={})])


@pytest.mark.parametrize("control", ["cancel", "pause", "cooldown", "delete"])
def test_freshness_execution_requested_dispatch_rechecks_controls(tmp_path, control):
    sql = {
        "cancel": "UPDATE freshness_jobs SET status='cancelled',cancel_requested=1 WHERE id='job-0002'",
        "pause": "UPDATE freshness_source_states SET paused=1 WHERE source_key='onchain_wallet:b'",
        "cooldown": "UPDATE freshness_jobs SET cooldown_until='2099-01-01T00:00:00Z' WHERE id='job-0002'",
        "delete": "DELETE FROM freshness_jobs WHERE id='job-0002'",
    }[control]
    parity(tmp_path, [enqueue(), enqueue("onchain_wallet:b"), enqueue("onchain_wallet:unrelated"),
        op("run_requested", profile_id="p", ids=["job-0002", "job-0001"], limit=2,
           handlers={"onchain_wallet_history": {"jobs": {
               "job-0001": {"steps": [{"op": "sql", "sql": sql}]},
               "job-0002": {"result": {"wallet": "Second", "status": "synced"}},
           }}}), op("snapshot", profile_id="p")])


def test_freshness_execution_aggregation_matches_daemon(tmp_path):
    jobs = [{"job_type": "onchain_wallet_history", "status": "done", "result": {"wallet": "one", "status": "synced"}},
            {"job_type": "journal_refresh", "status": "error"},
            {"job_type": "btcpay_provenance", "status": "cancelled", "payload": {"wallet_label": "two"}},
            {"job_type": "btcpay_wallet_source", "status": "rate_limited", "error": {"code": "429", "message": "wait", "retryable": True}},
            {"job_type": "onchain_wallet_history", "status": "done", "result": []}]
    parity(tmp_path, [op("sync_results", jobs=jobs)])


@pytest.mark.parametrize("workspace", [False, True])
@pytest.mark.parametrize("outcome", ["success", "partial", "error", "cooldown", "queued", "paused", "recovery", "negative-limit", "zero-limit"])
def test_freshness_execution_admitted_coordinator_matches_daemon(tmp_path, workspace, outcome):
    sql = "INSERT INTO settings VALUES('context_workspace','w'); INSERT INTO settings VALUES('context_profile','p');"
    sql += config_sql(_config("wpkh-fixed", 3))
    if workspace:
        sql += "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Second','EUR','later');"
    setup = []
    if outcome == "paused":
        setup.append(op("upsert", **source(status="paused", paused=True)))
    if outcome == "recovery":
        setup += [enqueue(), op("sql", sql="UPDATE freshness_jobs SET status='running',phase='backend_fetch'")]
    handler = {"steps": [emit("backend_fetch", wallet="Cold"), emit("import")],
               "result": {"wallet": "Cold", "status": "synced", "freshness_checkpoint": {"private": True}}}
    if outcome in {"error", "cooldown"}:
        handler["steps"].append(fail("rate_limited" if outcome == "cooldown" else "fetch_failed", "failed https://private"))
    if outcome == "partial":
        handler["result"]["partial_success"] = True
        handler["result"]["blocking_reports"] = True
    args = {"rates": False, "journals": False, "backend": backend(),
            "handlers": {"onchain_wallet_history": handler}, "run": outcome != "queued"}
    if workspace:
        args["workspace_id"] = "w"
    if outcome in {"negative-limit", "zero-limit"}:
        args["limit"] = -1 if outcome == "negative-limit" else 0
    parity(tmp_path, [*setup, op("plan_execute", **args), op("ui_snapshot", profile_id="p")], sql)


def test_freshness_execution_no_active_profile_matches_daemon(tmp_path):
    parity(tmp_path, [op("plan_execute", rates=False, journals=False, backend=backend(), handlers={})])


@pytest.mark.parametrize("raw", [
    "look ssl://user:pass@[::1]:50002/wallet?!", "HOST = 'private.example'", "quote \ud800 https://private/wallet",
    "surrogate in https://private/\ud800/path done", "nothing https://private/path] trailing",
])
def test_freshness_execution_ui_text_matches_python(tmp_path, raw):
    parity(tmp_path, [op("ui_payload", message=raw, nested=[{"backend_url": raw}], results=[])])
