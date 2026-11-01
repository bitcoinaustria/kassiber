"""Desktop journal registration against the consent-independent Python reference."""
import json
import copy
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

from tests.rust_freshness_session import Session
from kassiber.db import open_db
from tests.test_rust_journal_commands import (
    BINARY, KEY, ROOT, book_dump, invoke, seed, seed_auto,
)
from tests.test_rust_journal_commands import environment  # noqa: F401
from tests.test_rust_journal_process import scalar_exact
from tests.rust_port.journal_refresh_oracle import SCENARIOS

pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires native harness binary")
KIND = "ui.journals.process"


class Journals(Session):
    def __init__(self, root, env, *, native=True, reference_exception=None):
        prefix = [str(BINARY)] if native else [sys.executable, "-m", "tests.rust_port.journal_cli_reference"]
        self.process = subprocess.Popen(
            [*prefix, "--data-root", str(root), "daemon"], cwd=ROOT, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        self.buffer = b""
        self.rows = []
        self.reference_exception = reference_exception
        self.stderr_path = root.parent / (root.name + "-stderr.txt")

    def finish(self):
        self.send("daemon.shutdown", "stop")
        assert self.until("stop")["kind"] == "daemon.shutdown"
        self.process.stdin.close()
        self.process.wait(timeout=30)
        assert self.process.returncode == 0
        stderr = self.process.stderr.read()
        if self.reference_exception:
            self.stderr_path.write_bytes(stderr)
            assert stderr.startswith(b"Traceback (most recent call last):")
            assert stderr.rstrip().endswith(self.reference_exception.encode()), stderr
        else:
            assert stderr == b"", stderr

    def unlock(self, cipher):
        if cipher:
            self.send("daemon.unlock", "unlock", {"auth_response": {"passphrase_secret": KEY}})
            assert self.until("unlock")["kind"] == "daemon.unlock"
        else:
            self.send("status", "open")
            assert self.until("open")["kind"] == "status"


def run(root, env, *, cipher=False, native=True, args=None, reference_exception=None):
    with Journals(root, env, native=native, reference_exception=reference_exception) as session:
        assert KIND in session.rows[0]["data"]["supported_kinds"]
        session.unlock(cipher)
        if native:
            before = book_dump(root, cipher)
            session.send("test.freshness.select", "select", {"kind": KIND, "args": args, "request_id": "process"})
            assert session.until("select")["data"]["supported"]
            assert scalar_exact(book_dump(root, cipher), before)
        session.send(KIND, "process", args)
        terminal = session.until("process", KIND)
        session.finish()
        return terminal, [row for row in session.rows if row.get("request_id") == "process"]


def assert_wire(actual, expected):
    """Apply only rust-shell.md's interpreter-specific traceback allowance."""
    normalized = copy.deepcopy(expected[0]), copy.deepcopy(expected[1])
    for row in [normalized[0], *normalized[1]]:
        if (row["kind"] == "error" and row["error"]["code"] == "internal_error"
                and isinstance(row["error"]["debug"], str)):
            assert row["error"]["debug"].startswith("Traceback (most recent call last):")
            row["error"]["debug"] = None
    assert json.dumps(actual) == json.dumps(normalized)


def test_desktop_refusal_comparison_preserves_every_nontraceback_field():
    expected = {"kind": "error", "request_id": "process", "schema_version": 1, "error": {
        "code": "internal_error", "message": "refused", "hint": None, "details": {"count": 3},
        "retryable": False, "debug": "Traceback (most recent call last):\nValueError: refused\n",
    }}
    actual = copy.deepcopy(expected)
    actual["error"]["debug"] = None
    assert_wire((actual, [actual]), (expected, [expected]))
    for key, value in [("message", "changed"), ("retryable", True), ("details", {"count": 4}),
                       ("debug", "made-up stack"), ("code", "validation")]:
        changed = copy.deepcopy(actual)
        changed["error"][key] = value
        with pytest.raises(AssertionError):
            assert_wire((changed, [changed]), (expected, [expected]))
    assert expected["error"]["debug"].startswith("Traceback")


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("current", [False, True])
@pytest.mark.parametrize("scenario,country", [(s, "generic") for s in SCENARIOS] + [("exact", "at")])
def test_desktop_journals_match_ordered_envelopes_events_and_all_tables(tmp_path, environment, cipher, current, scenario, country):
    reference = seed_auto(tmp_path / "python", cipher, scenario, current, country)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    expected = run(reference, environment, cipher=cipher, native=False)
    actual = run(native, environment, cipher=cipher)
    (tmp_path / "parity.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=2))
    assert actual[0]["kind"] == KIND, actual
    assert_wire(actual, expected)
    assert scalar_exact(book_dump(native, cipher), book_dump(reference, cipher))
    assert not Path(environment["KASSIBER_HYBRID_LOG"]).exists()
    if scenario == "exact":
        assert actual[0]["data"]["auto_pair"]["applied"] > 0


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("shape", ["unchanged", "changed", "blocked", "empty", "build-refusal", "admission-refusal"])
def test_desktop_journals_plain_books_and_refusal_envelopes(tmp_path, environment, cipher, shape):
    reference = seed(tmp_path / "python", cipher=cipher, shape=shape)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    exception = "ValueError: Unsupported tax country 'unsupported'" if shape == "admission-refusal" else None
    expected = run(reference, environment, cipher=cipher, native=False, reference_exception=exception)
    actual = run(native, environment, cipher=cipher)
    assert_wire(actual, expected)
    assert scalar_exact(book_dump(native, cipher), book_dump(reference, cipher))


@pytest.mark.parametrize("context", ["missing-profile", "foreign-workspace-profile", "no-context", "label-only"])
def test_desktop_journals_unresolved_active_profile_matches_python_plain_resolution(tmp_path, environment, context):
    """Python falls back to plain processing when the active profile does not resolve."""
    reference = seed_auto(tmp_path / "python")
    with open_db(reference) as conn:
        if context == "missing-profile":
            conn.execute("UPDATE settings SET value='gone' WHERE key='context_profile'")
        elif context == "foreign-workspace-profile":
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w2','Other','2020')")
            conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p2','w2','Elsewhere','2020')")
            conn.execute("UPDATE settings SET value='p2' WHERE key='context_profile'")
        elif context == "no-context":
            conn.execute("DELETE FROM settings WHERE key IN ('context_workspace','context_profile')")
        else:
            conn.execute("UPDATE settings SET value=(SELECT label FROM profiles WHERE id='p') WHERE key='context_profile'")
    conn.close()
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    expected = run(reference, environment, native=False)
    actual = run(native, environment)
    assert_wire(actual, expected)
    assert scalar_exact(book_dump(native), book_dump(reference))


@pytest.mark.parametrize("args", [[], False, 7, "text", {"auto_pair": False}, {"profile": "foreign"},
                                  {"wallet": "foreign", "all": True}, {"consent": True}])
def test_desktop_journal_arguments_cannot_change_scope_or_pairing_authority(tmp_path, environment, args):
    reference = seed_auto(tmp_path / "python")
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    before = book_dump(native)
    assert run(native, environment, args=args) == run(reference, environment, args=args, native=False)
    assert scalar_exact(book_dump(native), before)


@pytest.mark.parametrize("scope", [{"workspace_id": "w", "profile_id": "foreign"},
                                   {"workspace_id": "foreign", "profile_id": "p"}, [], {}])
def test_desktop_journals_refuse_stale_and_malformed_expected_scope_before_progress(tmp_path, environment, scope):
    reference = seed_auto(tmp_path / "python")
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    before = book_dump(native)
    actual = run(native, environment, args={"expected_scope": scope})
    expected = run(reference, environment, native=False, args={"expected_scope": scope})
    assert actual == expected
    assert len(actual[1]) == 1
    assert scalar_exact(book_dump(native), before)


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("current", [False, True])
def test_desktop_second_rebuild_failure_retains_exact_python_first_commit(tmp_path, environment, cipher, current):
    reference = seed_auto(tmp_path / "python", cipher, current=current)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    before = book_dump(native, cipher)
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_FAIL_PAIRED_STORE="1")
    actual = run(native, env, cipher=cipher)
    expected = run(reference, env, cipher=cipher, native=False,
                   reference_exception="IntegrityError: second rebuild failed")
    assert_wire(actual, expected)
    assert scalar_exact(book_dump(native, cipher), book_dump(reference, cipher))
    assert scalar_exact(book_dump(native, cipher), before) == current


@pytest.mark.parametrize("cipher", [False, True])
def test_desktop_journal_cancellation_before_commit_rolls_back_pairs_and_every_journal_write(tmp_path, environment, cipher):
    root = seed_auto(tmp_path / "rust", cipher, current=True)
    before = book_dump(root, cipher)
    terminal, _ = run(root, dict(environment, KASSIBER_RUST_TEST_JOURNAL_CANCEL_BEFORE_COMMIT="1"), cipher=cipher)
    assert terminal["error"]["code"] == "cancelled", terminal
    assert scalar_exact(book_dump(root, cipher), before)


@pytest.mark.parametrize("cipher", [False, True])
def test_desktop_journal_context_change_before_writer_reservation_refuses_without_domain_writes(tmp_path, environment, cipher):
    root = seed_auto(tmp_path / "rust", cipher)
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE="writer_wait")
    with Journals(root, env) as session:
        session.unlock(cipher)
        session.send(KIND, "process", {"expected_scope": {"workspace_id": "w", "profile_id": "p"}})
        first = session.read()
        assert first["data"]["phase"] == "writer_wait", first
        with open_db(root, passphrase=KEY if cipher else None) as conn:
            conn.execute("UPDATE settings SET value='foreign' WHERE key='context_profile'")
        conn.close()
        before = book_dump(root, cipher)
        terminal = session.until("process", KIND)
        assert terminal["error"]["code"] == "stale_context", terminal
        session.finish()
    assert scalar_exact(book_dump(root, cipher), before)


@pytest.mark.parametrize("identity", [None, 7, False, [], {}, "", "\ud800"])
def test_desktop_journal_hybrid_selection_refuses_unsupported_request_ids_without_effects(tmp_path, environment, identity):
    root = seed_auto(tmp_path / "rust")
    before = book_dump(root)
    with Journals(root, environment) as session:
        session.unlock(False)
        session.send("test.freshness.select", "select", {"kind": KIND, "args": {}, "request_id": identity})
        assert session.until("select")["data"]["supported"] is False
        session.finish()
    assert scalar_exact(book_dump(root), before)


@pytest.mark.parametrize("cipher", [False, True])
def test_desktop_journal_early_cancel_leaves_every_table_unchanged(tmp_path, environment, cipher):
    root = seed_auto(tmp_path / "rust", cipher)
    before = book_dump(root, cipher)
    with Journals(root, environment) as session:
        session.unlock(cipher)
        session.send("test.freshness.request.cancel", "control", {"target_request_id": "process"})
        assert session.until("control")["data"]["queued"]
        session.send(KIND, "process")
        assert session.until("process", KIND)["error"]["code"] == "cancelled"
        session.finish()
        assert not any(row["kind"] == KIND + ".progress" for row in session.rows)
    assert scalar_exact(book_dump(root, cipher), before)


@pytest.mark.parametrize("cipher", [False, True])
def test_desktop_journal_profile_switch_joins_worker_before_changing_context(tmp_path, environment, cipher):
    root = seed_auto(tmp_path / "rust", cipher)
    with open_db(root, passphrase=KEY if cipher else None) as conn:
        conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p2','w','Other','2020')")
    conn.close()
    expected = book_dump(root, cipher)
    settings = expected["settings"]
    key, value = (settings["columns"].index(column) for column in ("key", "value"))
    for row in settings["rows"]:
        if row[key] == "context_profile":
            row[value] = "p2"
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE="building")
    with Journals(root, env) as session:
        session.unlock(cipher)
        session.send(KIND, "process")
        while session.read().get("data", {}).get("phase") != "building":
            pass
        session.send("ui.profiles.switch", "switch", {"profile_id": "p2"})
        result = session.until("switch")
        assert result["kind"] == "ui.profiles.switch", result
        assert result["data"]["activeProfileId"] == "p2"
        terminal = next(row for row in session.rows if row.get("request_id") == "process" and row["kind"] == "error")
        assert terminal["error"]["code"] == "cancelled"
        session.finish()
    assert scalar_exact(book_dump(root, cipher), expected)


@pytest.mark.parametrize("cipher", [False, True])
def test_desktop_journal_eof_cancels_and_joins_before_exit(tmp_path, environment, cipher):
    root = seed_auto(tmp_path / "rust", cipher)
    before = book_dump(root, cipher)
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE="building")
    with Journals(root, env) as session:
        session.unlock(cipher)
        session.send(KIND, "process")
        while session.read().get("data", {}).get("phase") != "building":
            pass
        session.process.stdin.close()
        terminal = session.until("process", KIND)
        assert terminal["error"]["code"] == "cancelled", terminal
        session.process.wait(timeout=30)
        assert session.process.returncode == 0
        assert session.process.stderr.read() == b""
    assert scalar_exact(book_dump(root, cipher), before)


@pytest.mark.parametrize("cipher", [False, True])
def test_desktop_journal_closed_terminal_output_preserves_committed_book_and_stops(tmp_path, environment, cipher):
    reference = seed_auto(tmp_path / "python", cipher, current=True)
    root = tmp_path / "rust"
    shutil.copytree(reference, root)
    expected = run(reference, environment, cipher=cipher, native=False)
    assert expected[0]["kind"] == KIND
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE="complete")
    with Journals(root, env) as session:
        session.unlock(cipher)
        session.send(KIND, "process")
        while session.read().get("data", {}).get("phase") != "complete":
            pass
        session.process.stdout.close()
        session.process.wait(timeout=30)
        assert session.process.returncode == 1
        assert session.process.stderr.read() == b""
    assert scalar_exact(book_dump(root, cipher), book_dump(reference, cipher))


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("phase", ["writer_wait", "preparing", "repairing", "pricing", "building", "storing", "auto_pair"])
@pytest.mark.parametrize("action", ["cancel", "lock", "shutdown"])
def test_desktop_journal_cancellation_and_retirement_join_the_connection_owner(tmp_path, environment, cipher, phase, action):
    root = seed_auto(tmp_path / "rust", cipher)
    before = book_dump(root, cipher)
    reference = tmp_path / "first-commit"
    shutil.copytree(root, reference)
    failed = invoke(reference, dict(environment, KASSIBER_RUST_TEST_JOURNAL_FAIL_PAIRED_STORE="1"),
                    cipher=cipher, extra=("--auto-pair",))
    assert failed.returncode == 1
    retained = book_dump(reference, cipher)
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE=phase)
    with Journals(root, env) as session:
        session.unlock(cipher)
        session.send(KIND, "process")
        while True:
            row = session.read()
            if row.get("data", {}).get("phase") == phase:
                break
        session.send("ui.freshness.run", "compete", {"rates": False, "journals": True})
        busy = session.until("compete")
        assert busy["error"]["code"] == "project_operation_in_progress", busy
        if action == "cancel":
            session.send("test.freshness.request.cancel", "control", {"target_request_id": "process"})
            assert session.until("control")["data"]["cancelled"]
            terminal = session.until("process", KIND)
            assert terminal["error"]["code"] == "cancelled", terminal
            session.finish()
        else:
            session.send("daemon." + action, "retire")
            result = session.until("retire")
            assert result["kind"] == "daemon." + action
            terminal = next(row for row in session.rows if row.get("request_id") == "process" and row["kind"] == "error")
            assert terminal["error"]["code"] == "cancelled", terminal
            if action == "lock":
                session.finish()
            else:
                session.process.stdin.close()
                session.process.wait(timeout=30)
                assert session.process.returncode == 0
                assert session.process.stderr.read() == b""
    assert scalar_exact(book_dump(root, cipher), retained if phase == "auto_pair" else before)
