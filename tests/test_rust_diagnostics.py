"""Byte parity of public reports, with an explicit runtime-provenance exception."""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import sqlite3
import sys
from unittest.mock import patch

import pytest

from tests import cli_runner

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
STAMP = "2026-10-29T12:34:56Z"
# This is the whole exception: never add user data or timestamps here.
NATIVE_FIELDS = (
    ("environment", "python_version"),
    ("environment", "python_implementation"),
    ("environment", "sqlite_version"),
    ("stack",),
)


def normalized(report):
    result = copy.deepcopy(report)
    for path in NATIVE_FIELDS:
        parent = result
        for key in path[:-1]:
            parent = parent[key]
        if path[-1] in parent:
            parent[path[-1]] = [] if path == ("stack",) else "<native-runtime>"
    return result


def test_normalization_changes_only_named_runtime_and_stack_values():
    report = {"environment": {"python_version": "3.11", "python_implementation": "CPython",
                              "sqlite_version": "old", "platform": {"machine": "planted"}},
              "stack": [{"module": "planted", "function": "planted", "line": 123}],
              "state": {"secret": "planted"}, "generated_at": "planted"}
    result = normalized(report)
    assert result["state"] == report["state"]
    assert result["generated_at"] == report["generated_at"]
    assert result["environment"]["platform"] == report["environment"]["platform"]
    # Every non-allowlisted leaf must remain observable by the byte comparison.
    for key in ("state", "generated_at"):
        changed = copy.deepcopy(report)
        changed[key] = "changed"
        assert normalized(changed) != result
    assert list(result) == list(report)
    assert NATIVE_FIELDS == (("environment", "python_version"),
                             ("environment", "python_implementation"),
                             ("environment", "sqlite_version"), ("stack",))


@pytest.fixture
def cli(tmp_path):
    if not RUST_BIN:
        pytest.skip("requires Rust test-harness binary")
    data = tmp_path / "book" / "data"
    env = {k: v for k, v in os.environ.items() if not k.startswith(("KASSIBER_", "SATBOOKS_"))}
    env.update({"KASSIBER_TEST_NO_EGRESS": "1", "KASSIBER_DISABLE_UPDATE_CHECK": "1",
                "KASSIBER_RUST_TEST_DIAGNOSTICS_TIME": STAMP,
                "XDG_CONFIG_HOME": str(tmp_path / "config"),
                "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
                "XDG_STATE_HOME": str(tmp_path / "state"),
                "KASSIBER_OFFLINE_PREFERENCE_FILE": str(tmp_path / "offline.json"),
                "KASSIBER_AGENT_ACCESS_FILE": str(tmp_path / "agents.json"),
                "KASSIBER_UPDATE_PREFERENCE_FILE": str(tmp_path / "updates.json")})

    served=Path(os.environ.get("KASSIBER_HYBRID_LOG",str(tmp_path / "served.jsonl")))

    def run(*args, reference=False):
        child_env = dict(env)
        options = {}
        if reference:
            code = ("import sys; import kassiber.diagnostics as d; "
                    f"d.now_iso=lambda: {STAMP!r}; "
                    "from kassiber.cli.main import main; sys.exit(main(sys.argv[3:]))")
            options["python_options"] = ("-c", code)
        else:
            child_env["KASSIBER_HYBRID_RUST_BIN"] = RUST_BIN
            child_env["KASSIBER_HYBRID_LOG"] = str(served)
        candidate=json.dumps([sys.executable,str(Path(__file__).parent / "rust_port" / "hybrid_cli.py")])
        with patch.dict(os.environ, {"KASSIBER_TEST_CLI": candidate}):
            result=cli_runner.run(["--machine", "--data-root", str(data), *args],
                                 reference=reference, env=child_env, capture_output=True, **options)
        if not reference:
            assert json.loads(served.read_text().splitlines()[-1])["served_by"] == "rust"
        return result

    assert run("init", reference=True).returncode == 0
    run.data = data
    return run


def canonical_bytes(raw):
    obj = json.loads(raw)
    assert raw == (json.dumps(obj, indent=2) + "\n").encode()
    return obj


def assert_report_bytes(reference, native):
    reference = canonical_bytes(reference)
    native = canonical_bytes(native)
    assert native["environment"]["python_implementation"] == "Rust"
    assert native["environment"]["python_version"].startswith("rustc ")
    assert json.dumps(normalized(native), indent=2) == json.dumps(normalized(reference), indent=2)


@pytest.mark.parametrize("save", [False, True])
def test_collect_report_bytes_and_export_modes(cli, save):
    args = ["diagnostics", "collect", *(["--save"] if save else [])]
    reference = cli(*args, reference=True)
    if save:
        payload = json.loads(reference.stdout)
        (cli.data.parent / payload["data"]["saved"]["relative_path"]).unlink()
    native = cli(*args)
    assert reference.returncode == native.returncode == 0, (reference.stderr, native.stderr)
    py = canonical_bytes(reference.stdout)
    rs = canonical_bytes(native.stdout)
    assert_report_bytes((json.dumps(py["data"]["report"], indent=2)+"\n").encode(),
                        (json.dumps(rs["data"]["report"], indent=2)+"\n").encode())
    py["data"]["report"] = normalized(py["data"]["report"])
    rs["data"]["report"] = normalized(rs["data"]["report"])
    assert json.dumps(py, indent=2) == json.dumps(rs, indent=2)
    if save:
        path = cli.data.parent / rs["data"]["saved"]["relative_path"]
        assert_report_bytes((json.dumps(json.loads(reference.stdout)["data"]["report"], indent=2)+"\n").encode(), path.read_bytes())
        assert path.stat().st_mode & 0o777 == 0o644


@pytest.mark.parametrize("target", ["auto", "custom"])
def test_error_reports_and_envelopes_match_bytes(cli, tmp_path, target):
    destination = "auto" if target == "auto" else str(tmp_path / "custom.json")
    args = ["--diagnostics-out", destination, "rates", "range", "BTC-USD", "--start", "not-a-date"]
    reference = cli(*args, reference=True)
    path = (cli.data.parent / "exports" / "diagnostics" / "kassiber-diagnostics-2026-10-29T12-34-56Z.json"
            if target == "auto" else Path(destination))
    expected = path.read_bytes()
    path.unlink()
    native = cli(*args)
    assert native.returncode == reference.returncode == 1
    assert native.stdout == reference.stdout
    assert native.stderr == reference.stderr
    assert_report_bytes(expected, path.read_bytes())


def test_planted_wallet_and_backend_fields_match_reference(cli):
    assert cli("workspaces", "create", "private-workspace", reference=True).returncode == 0
    assert cli("profiles", "create", "--workspace", "private-workspace", "private-profile", reference=True).returncode == 0
    assert cli("wallets", "create", "--workspace", "private-workspace", "--profile", "private-profile",
               "--kind", "custom", "--label", "private-wallet", reference=True).returncode == 0
    secret = "planted-never-public"
    config = {key: secret for key in ["descriptor", "change_descriptor", "addresses", "source_file",
                                    "sp_descriptor", "backend", "api_key", "password", "notes", "scan_key"]}
    config.update({"chain": "bitcoin", "network": "mainnet"})
    conn = sqlite3.connect(cli.data / "kassiber.sqlite3")
    conn.execute("UPDATE wallets SET config_json=?", (json.dumps(config),))
    conn.commit()
    conn.close()
    assert cli("backends", "create", "private-backend", "--kind", "electrum", "--url",
               f"ssl://private-user:{secret}@private.example:50002/private?token={secret}",
               "--token", secret, "--notes", secret, reference=True).returncode == 0
    reference = cli("diagnostics", "collect", reference=True)
    native = cli("diagnostics", "collect")
    py = json.loads(reference.stdout)["data"]["report"]
    rs = json.loads(native.stdout)["data"]["report"]
    assert secret.encode() not in native.stdout
    assert b"private-user" not in native.stdout
    assert b"private.example" not in native.stdout
    assert json.dumps(normalized(py), indent=2) == json.dumps(normalized(rs), indent=2)

# Only the unhandled diagnostics-export class may normalize envelope debug.
UNHANDLED_ENVELOPE_FIELDS = (("error", "debug"),)


def normalized_unhandled_envelope(raw):
    envelope = canonical_bytes(raw)
    assert envelope["kind"] == "error"
    assert envelope["error"]["code"] == "internal_error"
    assert isinstance(envelope["error"]["debug"], str)
    result = copy.deepcopy(envelope)
    for parent, key in UNHANDLED_ENVELOPE_FIELDS:
        result[parent][key] = "<native-stack>"
    return json.dumps(result, indent=2)


def test_unhandled_envelope_normalization_preserves_every_other_byte():
    assert UNHANDLED_ENVELOPE_FIELDS == (("error", "debug"),)
    envelope = {"kind": "error", "schema_version": 1, "error": {
        "code": "internal_error", "message": "planted", "hint": None,
        "details": {"planted": True}, "retryable": False, "debug": "stack"}}
    encode = lambda value: (json.dumps(value, indent=2) + "\n").encode()
    expected = normalized_unhandled_envelope(encode(envelope))
    for field in ("message", "hint", "details", "retryable"):
        changed = copy.deepcopy(envelope)
        changed["error"][field] = "changed"
        assert normalized_unhandled_envelope(encode(changed)) != expected
    changed = copy.deepcopy(envelope)
    changed["error"]["code"] = "validation"
    with pytest.raises(AssertionError):
        normalized_unhandled_envelope(encode(changed))


@pytest.mark.parametrize("debug", [False, True])
def test_collect_save_unhandled_io_failure_debug_and_report_bytes(cli, tmp_path, debug):
    # Put planted private paths in the exception and Python traceback.
    blocked = cli.data.parent / "exports" / "diagnostics"
    blocked.write_text("planted-sensitive-file")
    destination = tmp_path / "home-planted-user" / "private-build" / "failure.json"
    args = [*(["--debug"] if debug else []), "--diagnostics-out", str(destination),
            "diagnostics", "collect", "--save"]
    reference = cli(*args, reference=True)
    expected = destination.read_bytes()
    native = cli(*args)
    assert reference.returncode == native.returncode == 1
    if debug:
        assert normalized_unhandled_envelope(reference.stdout) == normalized_unhandled_envelope(native.stdout)
        assert json.loads(native.stdout)["error"]["debug"] == ""
    else:
        assert reference.stdout == native.stdout
        assert json.loads(native.stdout)["error"]["debug"] is None
    assert_report_bytes(expected, destination.read_bytes())
    assert json.loads(destination.read_bytes())["error"]["type"] == "FileExistsError"
    assert json.loads(destination.read_bytes())["error"]["unhandled"] is True
    assert b"home-planted-user" not in destination.read_bytes()
    assert b"private-build" not in destination.read_bytes()


@pytest.mark.parametrize("failure", ["directory", "blocked_parent"])
def test_best_effort_error_report_write_failure_preserves_original_error(cli, tmp_path, failure):
    blocker = tmp_path / "private-user" / "private-path"
    blocker.mkdir(parents=True)
    if failure == "blocked_parent":
        blocker = blocker / "file"
        blocker.write_text("private-secret")
        destination = blocker / "report.json"
    else:
        destination = blocker
    args = ["--diagnostics-out", str(destination), "rates", "range", "BTC-USD", "--start", "bad-date"]
    reference = cli(*args, reference=True)
    native = cli(*args)
    assert reference.returncode == native.returncode == 1
    assert native.stdout == reference.stdout
    assert native.stderr == reference.stderr
    assert b"private-user" not in native.stderr
    assert b"private-secret" not in native.stderr


def test_global_flag_success_and_usage_error_do_not_write_error_reports(cli, tmp_path):
    destination = tmp_path / "unexpected.json"
    for args in [
        ["--diagnostics-out", str(destination), "diagnostics", "collect"],
        ["--diagnostics-out", str(destination), "diagnostics", "bogus"],
    ]:
        reference = cli(*args, reference=True)
        native = cli(*args)
        assert native.returncode == reference.returncode
        if native.returncode == 0:
            py, rs = canonical_bytes(reference.stdout), canonical_bytes(native.stdout)
            py["data"]["report"] = normalized(py["data"]["report"])
            rs["data"]["report"] = normalized(rs["data"]["report"])
            assert json.dumps(py, indent=2) == json.dumps(rs, indent=2)
        else:
            assert native.stdout == reference.stdout
        assert native.stderr == reference.stderr
        assert not destination.exists()


def test_automatic_reports_keep_timestamp_and_collisions(cli):
    for reference in (True, False):
        existing = set((cli.data.parent / "exports" / "diagnostics").glob("*.json"))
        for _ in range(3):
            assert cli("--diagnostics-out", "auto", "rates", "range", "BTC-USD",
                       "--start", "not-a-date", reference=reference).returncode == 1
        directory = cli.data.parent / "exports" / "diagnostics"
        reports = set(directory.glob("*.json")) - existing
        assert len(reports) == 3
        assert {path.name for path in reports} == {
            "kassiber-diagnostics-2026-10-29T12-34-56Z.json",
            "kassiber-diagnostics-2026-10-29T12-34-56Z-2.json",
            "kassiber-diagnostics-2026-10-29T12-34-56Z-3.json"}
        for path in reports:
            assert json.loads(path.read_text())["generated_at"] == STAMP
            path.unlink()
