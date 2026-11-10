import sqlite3
from unittest.mock import patch

from kassiber import daemon_freshness
from kassiber.core import freshness
from kassiber.daemon_freshness import (
    _freshness_payload_for_ui, _freshness_snapshot_for_ui,
    _run_requested_freshness_jobs, _sync_results_from_freshness_jobs,
)
from kassiber.errors import AppError
from crates.wallets.scripts.publication_books import timestamp


def reference(conn, name, args):
    if name == "ui_payload":
        return _freshness_payload_for_ui(args)
    if name == "ui_snapshot":
        return _freshness_snapshot_for_ui(conn, args["profile_id"])
    if name == "sync_results":
        return _sync_results_from_freshness_jobs(args["jobs"])

    def make_handler(script):
        def handler(conn, job, progress, check):
            action = script.get("jobs", {}).get(job["id"], script)
            for step in action.get("steps", []):
                op = step["op"]
                if op == "emit":
                    progress(step["value"])
                elif op == "check":
                    check()
                elif op == "commit":
                    conn.commit()
                elif op == "sql":
                    conn.execute(step["sql"])
                elif op == "fail":
                    raise AppError(step["message"], code=step["code"], hint=step.get("hint"),
                                   details=step.get("details"), retryable=step.get("retryable", False))
                elif op == "database":
                    error = sqlite3.OperationalError(step["message"])
                    error.sqlite_errorcode = step["code"]
                    error.sqlite_errorname = {
                        5: "SQLITE_BUSY", 6: "SQLITE_LOCKED", 261: "SQLITE_BUSY_RECOVERY",
                        517: "SQLITE_BUSY_SNAPSHOT", 773: "SQLITE_BUSY_TIMEOUT",
                        262: "SQLITE_LOCKED_SHAREDCACHE", 518: "SQLITE_LOCKED_VTAB",
                        19: "SQLITE_CONSTRAINT",
                    }[step["code"]]
                    raise error
                elif op == "unexpected":
                    assert step["class"] == "builtins.ValueError"
                    raise ValueError(step["message"])
                else:
                    raise AssertionError(op)
            return action.get("result", {})
        return handler

    handlers = {kind: make_handler(script) for kind, script in args.get("handlers", {}).items()}
    events = []
    with patch.object(freshness, "_iso_from_now_plus", side_effect=lambda seconds: timestamp(max(0, seconds))):
        if name == "run_job":
            output = freshness.run_job(conn, args["id"], handlers, progress_observer=events.append,
                                       missing_ok=args.get("missing_ok", False))
        elif name == "run_requested":
            output = _run_requested_freshness_jobs(conn, args["profile_id"],
                [{"id": id} for id in args["ids"]], handlers, limit=args["limit"], progress_observer=events.append)
        else:
            request = {key: value for key, value in args.items() if key not in {"handlers", "backend"}}
            route = (daemon_freshness._workspace_freshness_run_payload if "workspace_id" in request
                     else daemon_freshness._freshness_run_payload)
            with patch.object(daemon_freshness, "_freshness_handlers", return_value=handlers), \
                 patch.object(daemon_freshness, "_prefetch_onchain_freshness_jobs", return_value={}):
                output = route(conn, {}, request, progress_observer=events.append)
    return {"output": output, "events": events}
