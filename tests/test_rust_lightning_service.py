import json
import os
from pathlib import Path
import sys

import pytest

from kassiber.db import open_db
from tests import cli_runner
from tests.test_rust_cln_acquisition import fake_cli, node as cln_node
from tests.test_rust_lightning_acquisition import node as lnd_node
from tests.test_rust_cross_check import state

ROOT = Path(__file__).resolve().parents[1]
BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not BIN, reason="requires Rust test-harness binary")


def book(tmp_path, monkeypatch, kind, backend):
    root = tmp_path / "data"
    conn = open_db(root)
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text())
    conn.execute("UPDATE wallets SET kind=?,config_json=? WHERE id='cold'", (kind, json.dumps({"backend": "fake"})))
    conn.execute("INSERT INTO backends(name,kind,url,token,timeout,config_json,created_at,updated_at) VALUES('fake',?,?,?,?,?,'fixed','fixed')",
                 (kind, backend.get("url", ""), backend.get("token", ""), 2, json.dumps({k: v for k, v in backend.items() if k not in {"name", "kind", "url", "token", "timeout"}})))
    conn.commit()
    database = Path(conn.execute("PRAGMA database_list").fetchone()[2])
    conn.close()
    conn = open_db(root)
    conn.close()
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env = {k: v for k, v in os.environ.items() if not k.startswith("KASSIBER_")}
    env.update(KASSIBER_RUST_TEST_OWNER_ROOT=str(owners), KASSIBER_HYBRID_RUST_BIN=str(Path(BIN).resolve()),
               KASSIBER_HYBRID_LOG=str(tmp_path / "served.jsonl"), KASSIBER_HYBRID_RUST_DAEMON="1",
               KASSIBER_TEST_NO_EGRESS="1", KASSIBER_DISABLE_UPDATE_CHECK="1",
               KASSIBER_OFFLINE_PREFERENCE_FILE=str(tmp_path / "offline.json"),
               XDG_CONFIG_HOME=str(tmp_path / "config"), XDG_DATA_HOME=str(tmp_path / "xdg-data"),
               XDG_STATE_HOME=str(tmp_path / "state"))
    if kind == "coreln":
        env["KASSIBER_RUST_TEST_CLN_FIXTURE"] = backend["lightning_cli"]
    monkeypatch.setenv("KASSIBER_TEST_CLI", json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]))
    original = cli_runner.prefix
    reference = '''import os,sys
from pathlib import Path
from kassiber.operator import project
project._owner_lock_root=lambda:Path(os.environ['KASSIBER_RUST_TEST_OWNER_ROOT'])
from kassiber.cli.main import main
raise SystemExit(main(sys.argv[1:]))
'''
    monkeypatch.setattr(cli_runner, "prefix", lambda *, reference=False, python_options=():
                        (sys.executable, "-c", reference_code) if reference else original(python_options=python_options))
    reference_code = reference

    def call(*args, reference=False, wire=None):
        return cli_runner.run(["--data-root", root, *args], reference=reference, env=env,
                              input=wire, capture_output=True, timeout=60)
    return root, database, env, call


def cases(kind):
    filename = "python_cln.json" if kind == "coreln" else "python_lightning.json"
    all_cases = json.loads((ROOT / "crates/chain/fixtures" / filename).read_text())
    good = [c for c in all_cases if "expected" in c and not c.get("failures")]
    return [good[0], good[len(good) // 2], good[-1]]


def fake_node(tmp_path, kind, case, **kwargs):
    if kind == "coreln":
        return cln_node(case["data"], **kwargs)
    return lnd_node(case["calls"], **kwargs)


def backend(tmp_path, kind, url):
    if kind == "coreln":
        return dict(kind=kind, name="fake", url="cln://local", rpc_file=url, lightning_cli=str(fake_cli(tmp_path)))
    return dict(kind=kind, name="fake", url=url, token="aabb")


@pytest.mark.parametrize("kind", ["lnd", "coreln"])
@pytest.mark.parametrize("case_index", [0, 1, 2])
def test_lightning_product_cli_wire_csv_all_112_tables_and_zero_fallback(tmp_path, monkeypatch, kind, case_index):
    case = cases(kind)[case_index]
    with fake_node(tmp_path, kind, case) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, kind, backend(tmp_path, kind, url))
        before = database.read_bytes()
        initial = state(root)
        target = tmp_path / "profitability.csv"

        def sequence(reference):
            database.write_bytes(before)
            outcomes = []
            for command in ["lightning-profitability", "export-lightning-profitability-csv"]:
                for flags in [[], ["--machine"], ["--format", "json"], ["--format", "table"], ["--format", "plain"], ["--format", "csv"]]:
                    hits.clear()
                    target.unlink(missing_ok=True)
                    args = ["reports", command, "--connection", "cold", "--window-days", str(case["window"])]
                    if "export" in command:
                        args += ["--file", str(target)]
                    result = call(*flags, *args, reference=reference)
                    assert result.returncode == 0, result.stdout
                    assert hits
                    assert state(root) == initial
                    outcomes.append((result.returncode, result.stdout, result.stderr, target.read_bytes() if target.exists() else None, list(hits)))
            return outcomes

        assert sequence(False) == sequence(True)
        served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
        assert len(served) == 12
        assert all(row["served_by"] == "rust" for row in served)
        assert {row["command"] for row in served} == {"reports.lightning-profitability", "reports.export-lightning-profitability-csv"}


@pytest.mark.parametrize("kind", ["lnd", "coreln"])
def test_lightning_product_daemon_wire_all_tables_and_zero_fallback(tmp_path, monkeypatch, kind):
    case = cases(kind)[0]
    with fake_node(tmp_path, kind, case) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, kind, backend(tmp_path, kind, url))
        before = database.read_bytes()
        initial = state(root)
        args = [dict(connection="cold", window_days=w) for w in [None, 1, 365, "7", "bad", True, -1, 900]]
        args += [dict(wallet="Cold"), dict(label="cold", workspace="Main", profile="Book"), dict(connection="missing"), {}, [], None]
        wire = b"".join((json.dumps(dict(request_id=i, kind="ui.reports.lightning_profitability", args=a)) + "\n").encode() for i, a in enumerate(args))
        wire += b'{"request_id":"done","kind":"daemon.shutdown"}\n'

        def session(reference):
            database.write_bytes(before)
            hits.clear()
            result = call("daemon", reference=reference, wire=wire)
            assert (result.returncode, result.stderr) == (0, b"")
            assert state(root) == initial
            return result.stdout.splitlines()[1:], list(hits)

        assert session(False) == session(True)
        assert Path(env["KASSIBER_HYBRID_LOG"]).read_text() == '{"command": "daemon", "served_by": "rust"}\n'


@pytest.mark.parametrize("kind", ["lnd", "coreln"])
def test_lightning_product_config_display_and_refusals_make_no_node_traffic(tmp_path, monkeypatch, kind):
    case = cases(kind)[0]
    with fake_node(tmp_path, kind, case) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, kind, backend(tmp_path, kind, url))
        initial = state(root)
        for args in [("--machine", "wallets", "list"), ("--machine", "backends", "list"), ("--machine", "status"),
                     ("--machine", "reports", "lightning-profitability", "--connection", "missing")]:
            call(*args)
        assert hits == []
        assert state(root) == initial

        Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).write_text('{"enabled":true}')
        result = call("--machine", "reports", "lightning-profitability", "--connection", "cold")
        assert json.loads(result.stdout)["error"]["code"] == "network_egress_disabled"
        Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).unlink()
        env["KASSIBER_NO_EGRESS"] = "1"
        result = call("--machine", "reports", "export-lightning-profitability-csv", "--connection", "cold", "--file", str(tmp_path / "out.csv"))
        assert json.loads(result.stdout)["error"]["code"] == "network_egress_disabled"
        assert not (tmp_path / "out.csv").exists()
        assert hits == []
        assert state(root) == initial


@pytest.mark.parametrize("kind", ["lnd", "coreln"])
@pytest.mark.parametrize("change", [
    "UPDATE settings SET value='changed' WHERE key='database_instance_id'",
    "UPDATE settings SET value='other' WHERE key='context_profile'",
    "UPDATE wallets SET config_json='{}' WHERE id='cold'",
    "UPDATE backends SET token='ccdd' WHERE name='fake'",
])
def test_lightning_product_rechecks_scope_during_acquisition_before_csv(tmp_path, monkeypatch, kind, change):
    import sqlite3
    case = cases(kind)[0]
    database = None

    def mutate(index):
        if index == 0:
            conn = sqlite3.connect(database)
            conn.execute(change)
            conn.commit()
            conn.close()

    with fake_node(tmp_path, kind, case, before_reply=mutate) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, kind, backend(tmp_path, kind, url))
        before = database.read_bytes()
        conn = sqlite3.connect(database)
        conn.execute(change)
        conn.commit()
        conn.close()
        expected = state(root)
        database.write_bytes(before)
        target = tmp_path / "profitability.csv"
        result = call("--machine", "reports", "export-lightning-profitability-csv", "--connection", "cold", "--file", str(target))
        assert json.loads(result.stdout)["error"]["code"] == "interaction_required", result.stdout
        assert len(hits) == 1
        assert not target.exists()
        assert state(root) == expected
