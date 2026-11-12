import json
import os
from pathlib import Path
import subprocess
from unittest.mock import patch

import pytest

from kassiber.core import freshness
from kassiber.daemon_freshness import _due_jobs_for_refresh, _run_requested_freshness_jobs, _workspace_freshness_run_payload
from kassiber.daemon_freshness import _freshness_wallet_source_specs, _filter_freshness_specs_by_policy
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import connect, copy_book, create_book, dump_bytes
from crates.wallets.scripts.publication_books import deterministic
from tests.rust_fixture_binary import checked


FIXTURE = os.environ.get("KASSIBER_RUST_FRESHNESS_FIXTURE")
if FIXTURE is None and os.environ.get("KASSIBER_RUST_BIN"):
    FIXTURE = str(checked(Path(os.environ["KASSIBER_RUST_BIN"]).resolve().parent / "examples/freshness_fixture",
                          "cargo build -p kassiber-wallets --example freshness_fixture --features test-harness --offline --locked"))
pytestmark = pytest.mark.skipif(not FIXTURE, reason="requires the Rust freshness queue fixture")


def op(name, **args):
    return {"op": name, "args": args}


def source(key="onchain_wallet:a", profile_id="p", **fields):
    return {"profile_id": profile_id, "source_key": key, "source_type": "onchain_wallet",
            "source_label": key, **fields}


def enqueue(key="onchain_wallet:a", **fields):
    return op("enqueue", **source(key, **{"job_type": "onchain_wallet_history", **fields}))


def _reference(conn, operation):
    name, args = operation["op"], operation["args"]
    if name == "sql":
        conn.executescript(args["sql"])
    elif name == "specs":
        return _freshness_wallet_source_specs(conn, **args)
    elif name == "filter_specs":
        return _filter_freshness_specs_by_policy(args["specs"], freshness.FreshnessPolicy(**args["policy"]), force=args.get("force", False))
    elif name == "upsert":
        return freshness.upsert_source_state(conn, **args)
    elif name == "enqueue":
        return freshness.enqueue_job(conn, **args)
    elif name == "cancel":
        return freshness.cancel_job(conn, args["id"])
    elif name in {"pause", "resume"}:
        return getattr(freshness, name + "_source")(conn, args["profile_id"], args["source_key"])
    elif name == "recover":
        return freshness.recover_interrupted_jobs(conn, **args)
    elif name == "due":
        return freshness.list_due_jobs(conn, **args)
    elif name == "dispatchable":
        return freshness.get_dispatchable_job(conn, args["profile_id"], args["id"])
    elif name == "selected":
        return freshness.get_selected_jobs(conn, args["profile_id"], args["ids"])
    elif name == "snapshot":
        return freshness.build_snapshot(conn, args["profile_id"])
    elif name == "policy":
        return freshness.get_policy(conn, args["profile_id"]).to_payload()
    elif name == "set_policy":
        fields = {key: value for key, value in args.items() if key != "profile_id"}
        return freshness.set_policy(conn, args["profile_id"], **fields).to_payload()
    elif name == "reset_checkpoint":
        return freshness.reset_source_checkpoint(conn, args["profile_id"], args["source_key"], stale_reason=args["stale_reason"])
    elif name == "delete_sources":
        freshness.delete_source_records(conn, args["profile_id"], args["keys"])
    elif name == "requested_due":
        return _due_jobs_for_refresh(conn, args["profile_id"], [{"id": id} for id in args["ids"]], args["limit"])
    elif name == "dispatch_order":
        order = []
        def capture(conn, profile_id, job_id):
            order.append(job_id)
        with patch.object(freshness, "get_dispatchable_job", side_effect=capture):
            _run_requested_freshness_jobs(conn, args["profile_id"], [{"id": id} for id in args["ids"]], {}, limit=len(args["ids"]))
        return order
    elif name == "workspace_profiles":
        result = _workspace_freshness_run_payload(conn, {}, {"workspace_id": args["workspace_id"], "run": False, "rates": False, "journals": False})
        return {"workspace": result["workspace"], "profiles": [book["profile"] for book in result["books"]]}
    elif name == "progress":
        freshness.update_job_progress(conn, freshness.get_job(conn, args["id"]), args["progress"])
    elif name == "progress_removed":
        job = freshness.get_job(conn, args["id"])
        conn.executescript("DELETE FROM freshness_jobs; DELETE FROM freshness_source_states;")
        freshness.update_job_progress(conn, job, args["progress"])
    elif name == "redact":
        return freshness.redact_freshness_payload(args)
    elif name in {"run_job", "run_requested", "plan_execute", "ui_payload", "ui_snapshot", "sync_results"}:
        from tests.rust_freshness_execution import reference
        return reference(conn, name, args)
    else:
        raise AssertionError(name)


def parity(tmp_path, operations, sql=""):
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, {}, extra_sql=sql)
    copy_book(python_book, rust_book)
    conn = connect(python_book)
    expected = []
    try:
        with deterministic(freshness, prefix="job"):
            for operation in operations:
                try:
                    result = _reference(conn, operation)
                    expected.append(build_envelope(operation["op"], result))
                except AppError as exc:
                    expected.append(build_error_envelope(exc.code, str(exc), details=exc.details,
                                                         hint=exc.hint, retryable=exc.retryable))
        conn.commit()
    finally:
        conn.close()
    result = subprocess.run([str(Path(FIXTURE).resolve())],
                            input=json.dumps({"book": str(rust_book), "operations": operations}).encode(),
                            capture_output=True, timeout=30, env=child_env(tmp_path))
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    actual = json.loads(result.stdout)["results"]
    assert json.dumps(actual, sort_keys=True, separators=(",", ":")) == json.dumps(expected, sort_keys=True, separators=(",", ":"))
    assert dump_bytes(rust_book) == dump_bytes(python_book)
    return actual


def test_freshness_queue_singleflight_force_and_running_owner(tmp_path):
    parity(tmp_path, [enqueue(payload={"force_full": False}), enqueue(payload={"force_full": True}),
                     enqueue(single_flight=False, payload={"force_full": True}),
                     op("sql", sql="UPDATE freshness_jobs SET status='running' WHERE id='job-0002'"),
                     enqueue(single_flight=False, payload={"different": True}),
                     op("snapshot", profile_id="p")])


@pytest.mark.parametrize("old,new", [({}, {}), ({"auto_pair": True}, {}), ({}, {"auto_pair": True}),
                                       ({"auto_pair": 1}, {"auto_pair": True}),
                                       ({"skip_rebuild_when_current": True}, {"skip_rebuild_when_current": True}),
                                       ({"skip_rebuild_when_current": True}, {}),
                                       ({}, {"skip_rebuild_when_current": True})])
def test_freshness_queue_journal_merge(tmp_path, old, new):
    parity(tmp_path, [enqueue("journals:p", job_type="journal_refresh", payload=old),
                     enqueue("journals:p", job_type="journal_refresh", payload=new),
                     op("sql", sql="UPDATE freshness_jobs SET status='running'"),
                     enqueue("journals:p", job_type="journal_refresh", payload={"auto_pair": True}),
                     op("snapshot", profile_id="p")])


def test_freshness_queue_paused_enqueue_keeps_private_checkpoint(tmp_path):
    parity(tmp_path, [op("upsert", **source(status="fresh", checkpoint={"scripts": ["private"]},
                                           paused=True, retry_count=4, blocking_reports=True)),
                     enqueue(), op("snapshot", profile_id="p"),
                     op("resume", profile_id="p", source_key="onchain_wallet:a"), enqueue(),
                     op("snapshot", profile_id="p")])


def test_freshness_queue_recovery_excludes_graph_and_other_profiles(tmp_path):
    sql = "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','other','EUR','fixed');"
    parity(tmp_path, [enqueue(priority=40), enqueue("graph:a", job_type="wallet_graph_references", priority=20),
                     enqueue("other:a", profile_id="q", priority=10),
                     op("upsert", **source(status="syncing", checkpoint={"private": "retained"}, paused=True)),
                     op("sql", sql="UPDATE freshness_jobs SET status='running',phase='import',cancel_requested=1"),
                     op("recover", profile_id="p"), op("snapshot", profile_id="p"),
                     op("snapshot", profile_id="q"), op("recover"),
                     op("snapshot", profile_id="q"), op("recover", profile_id="missing")], sql)


@pytest.mark.parametrize("status", ["queued", "running", "rate_limited", "done", "error", "cancelled"])
def test_freshness_queue_cancel_statuses_and_graph_blocking(tmp_path, status):
    parity(tmp_path, [enqueue(), enqueue("graph:a", job_type="wallet_graph_references"),
                     op("sql", sql=f"UPDATE freshness_jobs SET status='{status}',phase='backend_fetch'"),
                     op("cancel", id="job-0001"), op("cancel", id="job-0002"),
                     op("snapshot", profile_id="p"), op("cancel", id="missing")])


def test_freshness_queue_dispatch_rechecks_pause_cooldown_profile_and_running(tmp_path):
    operations = [enqueue(), enqueue("b", priority=10), enqueue("c", priority=10),
                  op("due", profile_id="p"), op("due", profile_id="p", limit=1),
                  op("pause", profile_id="p", source_key="onchain_wallet:a"),
                  op("dispatchable", profile_id="p", id="job-0001"),
                  op("dispatchable", profile_id="q", id="job-0002"),
                  op("dispatchable", profile_id="p", id="missing"),
                  op("sql", sql="UPDATE freshness_jobs SET cooldown_until='9999-01-01T00:00:00Z' WHERE id='job-0002'; "
                                 "UPDATE freshness_jobs SET status='running' WHERE id='job-0003';"),
                  op("dispatchable", profile_id="p", id="job-0002"),
                  op("dispatchable", profile_id="p", id="job-0003"),
                  op("due", profile_id="p"), op("due", profile_id="p", limit=-1),
                  op("sql", sql="UPDATE freshness_jobs SET status='done' WHERE id='job-0001'"),
                  op("dispatchable", profile_id="p", id="job-0001"),
                  op("snapshot", profile_id="p")]
    parity(tmp_path, operations)


def test_freshness_queue_due_order_and_timestamp_boundaries(tmp_path):
    parity(tmp_path, [enqueue("a", priority=40), enqueue("b", priority=10), enqueue("c", priority=10),
                     enqueue("d", run_after="9999-01-01T00:00:00Z"), enqueue("e"),
                     op("sql", sql="UPDATE freshness_jobs SET created_at='same' WHERE id IN ('job-0002','job-0003');"
                                    "UPDATE freshness_jobs SET status='rate_limited',cooldown_until='2000-01-01T00:00:00Z' WHERE id='job-0005'"),
                     op("due"), op("due", profile_id="p", limit=2),
                     op("dispatchable", profile_id="p", id="job-0004"),
                     op("selected", profile_id="p", ids=["job-0001", "missing", "job-0001", "job-0003"]),
                     op("selected", profile_id="q", ids=["job-0001"]),
                     op("selected", profile_id="p", ids=[]), op("snapshot", profile_id="p")])


@pytest.mark.parametrize("rate_until", [None, "invalid", "2000-01-01T00:00:00Z", "9999-01-01T00:00:00Z"])
def test_freshness_queue_state_precedence_and_preservation(tmp_path, rate_until):
    operations = [op("upsert", **source(status="fresh", last_success_at="saved", retry_count=3,
                                        checkpoint={"private": [1]}, progress={"phase": "done"})),
                  op("upsert", **source(status="failed", rate_limited_until=rate_until, blocking_reports=True)),
                  op("upsert", **source(status="syncing", rate_limited_until=rate_until, paused=True)),
                  op("snapshot", profile_id="p"),
                  op("resume", profile_id="p", source_key="onchain_wallet:a"),
                  op("pause", profile_id="p", source_key="missing")]
    parity(tmp_path, operations)


@pytest.mark.parametrize("setting", [None, "[]", "invalid", "null", "{}",
                                      '{"background_enabled":"yes","report_read_sync":1,"source_classes":{"journals":false,"onchain_wallet":true,"unknown":true}}'])
def test_freshness_queue_policy_defaults_legacy_and_saved(tmp_path, setting):
    sql = "INSERT INTO settings(key,value) VALUES('ai.auto_sync_before_report_reads.profile.p','  YES  ');"
    if setting is not None:
        sql += "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p','" + setting + "');"
    parity(tmp_path, [op("policy", profile_id="p"), op("snapshot", profile_id="missing")], sql)


def test_freshness_queue_redacts_results_and_retains_private_state(tmp_path):
    private = {"checkpoint": {"raw": "private"}, "freshness_checkpoint": {"raw": "private"},
               "_freshness_checkpoint": {}, "esplora_history_memberships": {},
               "nested": [{"Descriptor": "private", "api_key": "private", "client_secret": "private",
                           "password": "", "url": "http://localhost", "backend_url": "",
                           "checkpoint_json": "private", "xpub": "private", "count": 4}],
               "MixedTOKENName": "private", "empty_token": False, "safe": [1, True, None]}
    parity(tmp_path, [op("redact", **private), enqueue(payload=private),
                     op("progress", id="job-0001", progress={"phase": "backend_fetch", **private}),
                     op("upsert", **source(status="fresh", checkpoint=private)),
                     op("snapshot", profile_id="p"), enqueue(job_type="invalid")])


def test_freshness_queue_removed_progress_cannot_recreate_source(tmp_path):
    parity(tmp_path, [enqueue(), op("progress_removed", id="job-0001", progress={"phase": "import"}),
                     op("dispatchable", profile_id="p", id="job-0001"),
                     op("cancel", id="job-0001"), op("snapshot", profile_id="p")])


def test_freshness_queue_requested_jobs_do_not_borrow_other_work(tmp_path):
    parity(tmp_path, [enqueue("a", priority=1), enqueue("b", priority=2), enqueue("c", priority=3),
                     op("requested_due", profile_id="p", ids=["job-0003", "job-0002"], limit=1),
                     op("requested_due", profile_id="p", ids=[], limit=10),
                     op("requested_due", profile_id="q", ids=["job-0001"], limit=10),
                     op("requested_due", profile_id="p", ids=["job-0001"], limit=-1)])


def test_freshness_queue_policy_updates_and_invalid_class(tmp_path):
    parity(tmp_path, [op("set_policy", profile_id="p", background_enabled=True,
                        source_classes={"onchain_wallet": True, "journals": False}),
                     op("set_policy", profile_id="p", report_read_sync=True, background_enabled=None),
                     op("set_policy", profile_id="p", source_classes={"unknown": True}),
                     op("snapshot", profile_id="p")])


def test_freshness_queue_reset_and_delete_scope(tmp_path):
    parity(tmp_path, [enqueue(), op("upsert", **source(status="fresh", checkpoint={"private": "value"},
                                                      paused=True, blocking_reports=True)),
                     op("reset_checkpoint", profile_id="p", source_key="onchain_wallet:a", stale_reason="changed"),
                     op("reset_checkpoint", profile_id="q", source_key="onchain_wallet:a", stale_reason="changed"),
                     op("delete_sources", profile_id="q", keys=["onchain_wallet:a"]),
                     op("delete_sources", profile_id="p", keys=[]), op("snapshot", profile_id="p"),
                     op("delete_sources", profile_id="p", keys=["onchain_wallet:a"]), op("snapshot", profile_id="p")])


@pytest.mark.parametrize("fields", [{"url": "private", "has_url": False}, {"has_url": False, "url": "private"},
                                     {"\ud800token": "private", "safe\ud800": "retained", "se\ud800cret": "retained"}])
def test_freshness_queue_redaction_key_collisions_and_surrogates(tmp_path, fields):
    parity(tmp_path, [op("redact", **fields), enqueue(payload=fields), op("snapshot", profile_id="p")])


def test_freshness_queue_dispatch_order_prioritizes_exact_terminal_results(tmp_path):
    parity(tmp_path, [enqueue("a", priority=30), enqueue("b", priority=20), enqueue("c", priority=10),
                     enqueue("d", priority=40), enqueue("e", priority=50),
                     op("sql", sql="UPDATE freshness_jobs SET status='done' WHERE id='job-0001';"
                                    "UPDATE freshness_jobs SET status='cancelled' WHERE id='job-0004';"
                                    "UPDATE freshness_jobs SET status='running' WHERE id='job-0005';"),
                     op("dispatch_order", profile_id="p", ids=["job-0004", "job-0002", "job-0001", "missing", "job-0003", "job-0005"]),
                     op("dispatch_order", profile_id="q", ids=["job-0001"]),
                     op("dispatch_order", profile_id="p", ids=[]),
                     op("dispatch_order", profile_id="p", ids=["job-0001", "job-0004", "job-0001"])])


def test_freshness_queue_selected_batches_and_duplicates(tmp_path):
    ids = [f"missing-{index}" for index in range(1100)]
    ids.insert(0, "job-0003")
    ids.insert(501, "job-0001")
    ids.extend(["job-0003", "job-0001", "job-0002"])
    parity(tmp_path, [enqueue("a"), enqueue("b"), enqueue("c"),
                     op("selected", profile_id="p", ids=ids)])


def test_freshness_workspace_order_preserves_active_context(tmp_path):
    sql = """
        INSERT INTO workspaces(id,label,created_at) VALUES('foreign','foreign','fixed');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES
          ('old','w','Z','EUR','earlier'), ('a','w','A','EUR','fixed'),
          ('b','w','B','EUR','fixed'), ('out','foreign','first','EUR','0');
        INSERT INTO settings(key,value) VALUES('context_workspace','foreign'),('context_profile','out');
    """
    parity(tmp_path, [op("workspace_profiles", workspace_id=" w "),
                     op("workspace_profiles", workspace_id=" missing ")], sql)
