"""Byte-exact local setup daemon sessions, with identities injected before use."""
import json
import os
from pathlib import Path
import subprocess
import sys
from unittest import mock

import pytest
from tests import cli_runner
from tests import test_rust_shell_parity as shell
from tests.test_rust_shell_parity import RUST_BIN, RUST_SKIP, ROOT, TIMEOUT

pytestmark = pytest.mark.skipif(not RUST_BIN, reason=RUST_SKIP)

@pytest.fixture
def book():
    harness = shell.RustRealStoreParityTest()
    harness.setUp()
    try:
        harness.ok(harness.call(False, ["init"]))
        harness.ok(harness.call(False, ["workspaces", "create", "Main"]))
        profile = harness.ok(harness.call(False, ["profiles", "create", "Book"]))
        harness.ok(harness.call(False, ["backends", "create", "private", "--kind", "bitcoinrpc", "--url", "http://127.0.0.1:8332", "--token", "synthetic-token"]))
        harness.profile = profile
        yield harness
    finally:
        harness.tearDown()

def same_session(book, requests):
    database = book.root / "kassiber.sqlite3"
    initial = database.read_bytes()
    log = book.home / "desktop-served.jsonl"
    env = book.env | {
        cli_runner.CLI_ENV: json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]),
        "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
        "KASSIBER_HYBRID_RUST_DAEMON": "1",
        "KASSIBER_HYBRID_LOG": str(log),
        "KASSIBER_RUST_TEST_SETUP_IDS": "sequential",
        "KASSIBER_RUST_TEST_SETUP_TIMESTAMP": "2026-01-01T00:00:00Z",
    }
    script = (
        "import sys,uuid,itertools; from pathlib import Path; "
        "from kassiber.operator import project; "
        "project._owner_lock_root=lambda:Path(sys.argv.pop(1)); "
        "from kassiber.core import accounts,ui_snapshot; from kassiber import backends; "
        "from crates.setup.scripts.export_desktop_vectors import FixedDateTime; "
        "ui_snapshot.datetime=FixedDateTime; "
        "counter=itertools.count(1); "
        "accounts.uuid.uuid4=lambda:uuid.UUID('00000000-0000-4000-8000-%012x'%next(counter)); "
        "accounts.now_iso=lambda:'2026-01-01T00:00:00Z'; "
        "backends.now_iso=lambda:'2026-01-01T00:00:00Z'; "
        "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
    )
    # Capture the ownership root once; the ownership resolver is called repeatedly.
    script = script.replace("project._owner_lock_root=lambda:Path(sys.argv.pop(1));", "owners=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:owners;")
    original = cli_runner.prefix
    def prefix(*, reference=False, python_options=()):
        return (sys.executable, "-c", script, str(book.owners)) if reference else original(reference=reference, python_options=python_options)
    lines = [{"request_id": i, "kind": kind, "args": args} for i, (kind, args) in enumerate(requests)]
    lines.append({"request_id": "shutdown", "kind": "daemon.shutdown"})
    data = "".join(json.dumps(line) + "\n" for line in lines).encode()
    def exchange(reference):
        with mock.patch.object(cli_runner, "prefix", side_effect=prefix), mock.patch.dict(os.environ, env):
            result = cli_runner.run(["--data-root", str(book.root), "daemon"], reference=reference, env=env, input=data, capture_output=True, timeout=TIMEOUT)
        assert result.returncode == 0, result.stderr
        assert result.stderr == b""
        return result.stdout.splitlines(keepends=True)
    expected = exchange(True)
    database.write_bytes(initial)
    actual = exchange(False)
    assert actual[1:] == expected[1:]
    expected_ready = json.loads(expected[0])
    actual_ready = json.loads(actual[0])
    supported = set(actual_ready["data"]["supported_kinds"])
    expected_ready["data"]["supported_kinds"] = [k for k in expected_ready["data"]["supported_kinds"] if k in supported] + [
        k for k in actual_ready["data"]["supported_kinds"] if k.startswith("test.")]
    assert actual[0] == (json.dumps(expected_ready, separators=(",", ":")) + "\n").encode()
    records = [json.loads(line) for line in log.read_text().splitlines()]
    assert records == [{"command": "daemon", "served_by": "rust"}]
    responses = [json.loads(line) for line in actual[1:-1]]
    assert len(responses) == len(requests)
    assert all(r.get("error", {}).get("code") != "not_implemented" for r in responses)
    if os.environ.get("KASSIBER_HYBRID_LOG"):
        with Path(os.environ["KASSIBER_HYBRID_LOG"]).open("a") as witness:
            for kind, _ in requests:
                witness.write(json.dumps({"command": kind, "served_by": "rust"}) + "\n")
    return responses

def test_setup_desktop_mutations_snapshots_and_plaintext_authorization(book):
    wid = book.profile["workspace_id"]
    ack = {"plaintext_delete_ack": "DELETE LOCAL DATA"}
    reset = {"confirm": "RESET", "confirm_profile": "Book"}
    secondary = "00000000-0000-4000-8000-000000000001"
    responses = same_session(book, [
        ("ui.profiles.snapshot", {}),
        ("ui.workspace.rename", {"workspace_id": "Main", "label": "Wrong"}),
        ("ui.workspace.rename", {"workspace_id": wid, "label": "Renamed main"}),
        ("ui.profiles.reset_data", {}),
        ("ui.profiles.reset_data", reset | {"clear_shared_rates": 1}),
        ("ui.profiles.reset_data", reset),
        ("backends.reveal_token", {"name": "private"}),
        ("backends.reveal_token", {"name": "private", "auth_response": {"plaintext_reveal_ack": "copy local secret"}}),
        ("backends.reveal_token", {"backend": "private", "auth_response": {"plaintext_reveal_ack": "COPY LOCAL SECRET"}}),
        ("ui.profiles.reset_data", reset | {"auth_response": ack}),
        ("ui.profiles.reset_data", reset | {"clear_shared_rates": True, "auth_response": ack}),
        ("ui.profiles.snapshot", {}),
        ("ui.workspace.create", {"label": " Secondary "}),
        ("ui.workspace.rename", {"workspace_id": secondary, "label": "Renamed secondary"}),
        ("ui.profiles.snapshot", [1]),
        ("ui.workspace.delete", {"confirm": "DELETE", "confirm_workspace": "wrong", "auth_response": ack}),
        ("ui.workspace.delete", {"confirm": "DELETE", "confirm_workspace": "Renamed secondary", "auth_response": ack}),
        ("ui.profiles.snapshot", {}),
        ("backends.reveal_token", []),
        ("backends.reveal_token", 0),
        ("backends.reveal_token", ""),
        ("backends.reveal_token", False),
    ])
    assert responses[8]["data"]["token"] == "synthetic-token"
    assert responses[9]["data"]["reset"] is True
    assert responses[16]["data"]["deleted"] is True

def test_owner_reveal_and_reset_reauthorize_the_unlocked_encrypted_book(book):
    book.ok(book.call(False, ["secrets", "init"], secret=book.PASSPHRASE, new_secret=True))
    auth = {"passphrase_secret": book.PASSPHRASE}
    reset = {"confirm": "RESET", "confirm_profile": "Book"}
    responses = same_session(book, [
        ("daemon.unlock", {"auth_response": auth}),
        ("backends.reveal_token", {"name": "private"}),
        ("backends.reveal_token", {"name": "private", "auth_response": {"passphrase_secret": "wrong"}}),
        ("backends.reveal_token", {"name": "private", "auth_response": auth}),
        ("ui.profiles.reset_data", reset),
        ("ui.profiles.reset_data", reset | {"auth_response": auth}),
        ("ui.workspace.delete", {"confirm": "DELETE", "confirm_workspace": "Main"}),
        ("ui.workspace.delete", {"confirm": "DELETE", "confirm_workspace": "Main", "auth_response": auth}),
    ])
    assert responses[1]["kind"] == "auth_required"
    assert responses[2]["error"]["code"] == "local_auth_denied"
    assert responses[3]["data"]["token"] == "synthetic-token"
    assert responses[4]["kind"] == "auth_required"
    assert responses[5]["data"]["reset"] is True
    assert responses[7]["data"]["deleted"] is True

def test_core_credential_references_merge_only_current_local_candidates(book):
    from kassiber.daemon import _core_candidate_credential_ref, _core_local_probe_candidates
    core = book.home / ".bitcoin"
    core.mkdir()
    (core / "bitcoin.conf").write_text("rpcuser=alice\nrpcpassword=synthetic-password\n[main]\nrpcport=18444\n[test]\nrpcconnect=example.test\n")
    (core / ".cookie").write_text("synthetic-user:synthetic-cookie")
    found = _core_local_probe_candidates(core)
    basic = next(c for c in found if c["network"] == "main" and c["auth_source"] == "basic")
    cookie = next(c for c in found if c["network"] == "main" and c["auth_source"] == "cookiefile")
    responses = same_session(book, [
        ("ui.backends.create", {"name": "bad-kind", "kind": "electrum", "credential_ref": _core_candidate_credential_ref(basic)}),
        ("ui.backends.create", {"name": "missing", "kind": "bitcoinrpc", "credential_ref": "local-core:missing"}),
        ("ui.backends.create", {"name": "basic", "kind": "bitcoinrpc", "credential_ref": _core_candidate_credential_ref(basic), "url": "https://example.test", "config": {"cookiefile": "/synthetic/wrong"}}),
        ("ui.backends.create", {"name": "cookie", "kind": "bitcoinrpc", "credential_ref": _core_candidate_credential_ref(cookie), "config": {"username": "wrong", "password": "synthetic-wrong"}}),
        ("backends.reveal_token", {"name": "basic", "auth_response": {"plaintext_reveal_ack": "COPY LOCAL SECRET"}}),
        ("backends.reveal_token", {"name": "cookie", "auth_response": {"plaintext_reveal_ack": "COPY LOCAL SECRET"}}),
    ])
    assert responses[2]["data"]["url"] == "http://127.0.0.1:18444"
    assert responses[4]["data"]["password"] == "synthetic-password"
    assert responses[3]["data"]["has_cookiefile"] is True
    assert responses[5]["data"]["username"] is None
