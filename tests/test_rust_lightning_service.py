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


@pytest.mark.parametrize("tls,proxy,trust,success", [
    (False, None, None, True),
    (True, None, "path", True), (True, None, "inline", True),
    (True, None, "insecure", True), (True, None, None, False),
    (False, "http", None, True), (False, "socks5h", None, True),
    (True, "http", "path", True), (True, "socks5h", "path", True),
    (True, "socks5h", "wrong-name", False),
])
def test_lightning_product_lnd_tls_proxy_wire_csv_and_all_tables(tmp_path, monkeypatch, tls, proxy, trust, success):
    from tests.rust_port.lightning_transport import transport
    with transport([{"response": {}}] * 9, tls=tls, proxy=proxy, wrong_name=trust == "wrong-name") as (raw, hits, routes):
        if trust in {"path", "wrong-name"}:
            raw["certificate"] = str(ROOT / "crates/net/tests/fixtures/ca.pem")
        elif trust == "inline":
            raw["certificate"] = (ROOT / "crates/net/tests/fixtures/ca.pem").read_text()
        elif trust == "insecure":
            raw["insecure"] = True
        root, database, env, call = book(tmp_path, monkeypatch, "lnd", raw)
        before = database.read_bytes()
        initial = state(root)
        target = tmp_path / "profitability.csv"

        def sequence(reference):
            database.write_bytes(before)
            outcomes = []
            for command in ["lightning-profitability", "export-lightning-profitability-csv"]:
                hits.clear()
                routes.clear()
                target.unlink(missing_ok=True)
                args = ["--machine", "reports", command, "--connection", "cold"]
                if command.startswith("export"):
                    args += ["--file", str(target)]
                result = call(*args, reference=reference)
                payload = json.loads(result.stdout)
                if success:
                    assert result.returncode == 0, result.stdout
                    assert len(hits) == 9
                    assert all("grpc-metadata-macaroon: aabb" in h["request"].lower() for h in hits)
                    outcomes.append((result.stdout, target.read_bytes() if target.exists() else None))
                else:
                    assert payload["error"]["code"] == "backend_error"
                    assert hits == []
                    assert not target.exists()
                    error = payload["error"]
                    outcomes.append((error["code"], error["retryable"], error.get("hint"), error.get("details")))
                assert state(root) == initial
            return outcomes

        assert sequence(False) == sequence(True)
        served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
        assert len(served) == 2
        assert all(row["served_by"] == "rust" for row in served)


@pytest.mark.parametrize("status,raw,code", [
    (401, b"SYNTHETIC_RESPONSE_SECRET", "backend_error"),
    (403, b"SYNTHETIC_RESPONSE_SECRET", "backend_error"),
    (503, b"SYNTHETIC_RESPONSE_SECRET", "backend_error"),
    (200, b"SYNTHETIC_RESPONSE_SECRET", "backend_error"),
    (200, b"[]", "backend_error"), (200, b"\xff", "internal_error"),
])
def test_lightning_product_errors_preserve_category_and_discard_node_bodies(tmp_path, monkeypatch, status, raw, code):
    from tests.rust_port.lightning_transport import transport
    with transport([{"response": {}}], status=status, raw=raw) as (raw_backend, hits, _):
        root, database, env, call = book(tmp_path, monkeypatch, "lnd", raw_backend)
        before = database.read_bytes()
        initial = state(root)
        outcomes = []
        for reference in [False, True]:
            database.write_bytes(before)
            hits.clear()
            result = call("--machine", "reports", "lightning-profitability", "--connection", "cold", reference=reference)
            error = json.loads(result.stdout)["error"]
            assert error["code"] == code
            assert len(hits) == 1
            if not reference:
                assert b"SYNTHETIC_RESPONSE_SECRET" not in result.stdout
            details = error.get("details")
            if reference and status == 200 and raw == b"SYNTHETIC_RESPONSE_SECRET":
                assert details == {"response_preview": "SYNTHETIC_RESPONSE_SECRET"}
                details = None
            outcomes.append((error["code"], error["retryable"], details))
            assert state(root) == initial
        assert outcomes[0] == outcomes[1]


@pytest.mark.parametrize("change", [
    "UPDATE wallets SET kind='nwc' WHERE id='cold'",
    "UPDATE wallets SET kind='custom' WHERE id='cold'",
    "UPDATE wallets SET config_json='{}' WHERE id='cold'",
    "UPDATE wallets SET config_json='malformed' WHERE id='cold'",
    "UPDATE wallets SET config_json='{\"backend\":\"absent\"}' WHERE id='cold'",
    "UPDATE backends SET token='' WHERE name='fake'",
    "UPDATE backends SET token='bad' WHERE name='fake'",
    "UPDATE backends SET kind='coreln' WHERE name='fake'",
    "DELETE FROM settings WHERE key='context_profile'",
])
def test_lightning_product_backend_capability_and_scope_refusals_match_python(tmp_path, monkeypatch, change):
    import sqlite3
    case = cases("lnd")[0]
    with fake_node(tmp_path, "lnd", case) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, "lnd", backend(tmp_path, "lnd", url))
        conn = sqlite3.connect(database)
        conn.execute(change)
        conn.commit()
        conn.close()
        before = database.read_bytes()
        initial = state(root)
        outcomes = []
        for reference in [False, True]:
            database.write_bytes(before)
            result = call("--machine", "reports", "lightning-profitability", "--connection", "cold", reference=reference)
            assert result.returncode == 1, result.stdout
            assert json.loads(result.stdout)["kind"] == "error"
            outcomes.append((result.stdout, result.stderr))
            assert hits == []
            assert state(root) == initial
        assert outcomes[0] == outcomes[1]


@pytest.mark.parametrize("kind", ["lnd", "coreln"])
def test_assistant_node_snapshot_matches_python_without_identifiers(tmp_path, monkeypatch, kind):
    """`ui.connections.node.snapshot` as an assistant tool: consented calls
    reach the node as Python's do and answer Python's
    `snapshot_to_dict_for_ai` bytes, without the operator pubkey or any
    channel, peer or connection identifier. A call without a consent bound
    to it never reaches the node in Rust (Python's executor leaves that to
    the chat loop: docs/reference/rust-shell.md, R6-5b reviewed differences)."""
    import subprocess

    case = cases(kind)[0]
    with fake_node(tmp_path, kind, case) as (url, hits):
        root, database, env, _call = book(tmp_path, monkeypatch, kind, backend(tmp_path, kind, url))
        before = database.read_bytes()
        initial = state(root)
        on_device = {"provider_kind": "local", "provider_on_device": True}
        remote = {"provider_kind": "remote", "provider_on_device": False}

        def wire(calls):
            lines = b""
            for index, (arguments, provider, consented) in enumerate(calls):
                call_id = f"call_{index}"
                consent = {"consented_call_ids": [call_id]} if consented else {}
                request = {"request_id": index, "kind": "test.ai_tool", "args": {
                    "name": "ui_connections_node_snapshot", "arguments": arguments, "call_id": call_id,
                    "state": {**provider, **consent}}}
                lines += (json.dumps(request) + "\n").encode()
            return lines + b'{"request_id":"done","kind":"daemon.shutdown"}\n'

        # The conftest's child hooks (the Python no-egress guard) and the
        # repository stay on the children's path.
        assert str(ROOT / "tests" / "_child_hooks") in env["PYTHONPATH"].split(os.pathsep)

        def session(argv, calls):
            database.write_bytes(before)
            hits.clear()
            result = subprocess.run([*argv, "--machine", "--data-root", str(root), "daemon"], input=wire(calls),
                                    env=env, cwd=ROOT, capture_output=True, timeout=120)
            assert (result.returncode, result.stderr) == (0, b""), result.stdout[-2000:]
            assert state(root) == initial
            lines = result.stdout.splitlines()[1:-1]
            return [json.loads(json.loads(line)["data"]["output"]) for line in lines], lines, list(hits)

        rust_bin = [str(Path(BIN).resolve())]
        consented = [({"connection": "cold"}, on_device, True), ({"connection": "cold", "window_days": 7}, remote, True),
                     ({"connection": "missing"}, on_device, True)]
        results, rust_lines, rust_hits = session(rust_bin, consented)
        _, python_lines, python_hits = session([sys.executable, str(ROOT / "tests/rust_ai_tool_reference.py")], consented)
        assert rust_hits == python_hits
        assert rust_hits
        assert rust_lines == python_lines
        assert [result["ok"] for result in results] == [True, True, False]
        snapshot = results[0]["envelope"]["data"]
        assert snapshot["pubkey"] is None
        assert "id" not in snapshot["connection"]
        hidden = {"id", "shortChannelId", "fundingOutpoint", "peerAlias", "peerPubkey",
                  "inPeerAlias", "inShortChannelId", "outPeerAlias", "outShortChannelId"}
        for row in snapshot["channels"] + snapshot["closedChannels"] + snapshot["forwards"]:
            assert not hidden & set(row), row

        refused, _, refused_hits = session(rust_bin, [({"connection": "cold"}, on_device, False)])
        assert [result.get("reason") for result in refused] == ["consent_required"]
        assert refused_hits == []
