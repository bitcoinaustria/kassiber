"""The session recorder and replay harness fail what they must (plan 21).

A CLI session and a daemon session are recorded through `tests/cli_runner.py`
and replayed on fresh copies of their starting state. A fresh Python run must
agree once clock readings, generated ids, and the order of their ties are
relaxed (the diagnostic lanes of `tests/session_harness/compare.py`; the
strict lane waits for clock and entropy injection). A candidate with one
deliberate discrepancy (`tests/session_harness/inject_candidate.py`) must
fail in the strict lane and in every relaxed one: an altered output, exit
status, stored amount, link between rows, timestamp, or referenced id,
including one applied by a grandchild process, one inside a daemon session,
and one on a caught-error path. The recorder refuses what it must not keep.
"""

from __future__ import annotations

import functools
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests import cli_runner
from tests.session_harness import dbdump, policy, recorder
from tests.session_harness.compare import Difference, Mapper, Window
from tests.session_harness.replay import CHILD_HOOKS, replay, session_dirs


ROOT = Path(__file__).resolve().parents[1]
INJECT = ROOT / "tests" / "session_harness" / "inject_candidate.py"
_BOOK_CSV = (
    "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
    "2026-01-01T00:00:00Z,buy-1,inbound,BTC,0.10000000,0,60000,Buy,buy\n"
    "2026-03-01T10:00:00Z,sell-1,outbound,BTC,0.02000000,0.00001000,70000,Sell,sell\n"
)
_SCOPE = ("--workspace", "Main", "--profile", "Default")
_PASSPHRASE = "synthetic session passphrase"
# The candidate's own children run under the suite's guard too.
_GUARDED = {"PYTHONPATH": os.pathsep.join((str(CHILD_HOOKS), str(ROOT))), "KASSIBER_TEST_NO_EGRESS": "1"}
# Kept for the whole run: under xdist a module's classes can resume on a
# worker after its module teardown, so the directories clean up at exit.
_RECORDINGS: list[tempfile.TemporaryDirectory] = []


def _recording_env(directory: Path) -> mock._patch:
    environment = {key: value for key, value in os.environ.items() if key != cli_runner.CLI_ENV}
    environment[cli_runner.RECORD_ENV] = str(directory)
    return mock.patch.dict(os.environ, environment, clear=True)


def _new_recording() -> Path:
    scratch = tempfile.TemporaryDirectory(prefix="kassiber-session-recordings-")
    _RECORDINGS.append(scratch)
    return Path(scratch.name)


@functools.lru_cache(maxsize=None)
def cli_session() -> Path:
    """One recorded CLI session: a small book, a caught error, two listings."""
    recordings = _new_recording()
    with tempfile.TemporaryDirectory(prefix="kassiber-session-cli-") as sandbox, _recording_env(recordings):
        data_root = Path(sandbox) / "data"
        book = Path(sandbox) / "book.csv"
        book.write_text(_BOOK_CSV, encoding="utf-8")

        def kassiber(*args: str, expect: int = 0) -> None:
            result = cli_runner.run(["--data-root", str(data_root), "--machine", *args], capture_output=True, text=True, timeout=300)
            assert result.returncode == expect, (args, result.stdout[-2000:], result.stderr[-2000:])

        kassiber("init")
        kassiber("workspaces", "create", "Main")
        kassiber("profiles", "create", "--workspace", "Main", "--fiat-currency", "USD", "--tax-country", "generic", "Default")
        for label in ("W1", "W2"):
            kassiber("wallets", "create", *_SCOPE, "--label", label, "--kind", "custom")
        kassiber("wallets", "import-csv", *_SCOPE, "--wallet", "W1", "--file", str(book))
        kassiber("wallets", "create", *_SCOPE, "--label", "W1", "--kind", "custom", expect=1)
        kassiber("wallets", "list", *_SCOPE)
        kassiber("transactions", "list", *_SCOPE)
    (session,) = session_dirs([recordings])
    return session


@functools.lru_cache(maxsize=None)
def daemon_session() -> Path:
    """One recorded daemon session, with a control-channel exchange the test answers."""
    recordings = _new_recording()
    with tempfile.TemporaryDirectory(prefix="kassiber-session-daemon-") as sandbox, _recording_env(recordings):
        data_root = Path(sandbox) / "data"
        process = cli_runner.popen(
            ["--data-root", str(data_root), "daemon"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        def send(payload: dict) -> None:
            process.stdin.write(json.dumps(payload) + "\n")
            process.stdin.flush()

        def receive() -> dict:
            return json.loads(process.stdout.readline())

        try:
            assert receive()["kind"] == "daemon.ready"
            send({"request_id": "status-1", "kind": "status"})
            assert receive()["kind"] == "status"
            # A test that opens the book itself while the daemon runs acts
            # outside every recorded process; the recording says so.
            sqlite3.connect(data_root / "kassiber.sqlite3").close()
            send({"request_id": "provider-1", "kind": "ai.providers.create",
                  "args": {"name": "bridge", "base_url": "https://example.test/v1", "kind": "remote"}})
            assert receive()["kind"] == "ai.providers.create"
            send({"request_id": "key-1", "kind": "ai.providers.set_api_key",
                  "args": {"name": "bridge", "api_key": "synthetic-test-key", "store_id": "macos_keychain",
                           "_desktop_secret_store_bridge": True}})
            control = receive()
            assert control["kind"] == "supervisor.ai_secret_store.request", control
            # The supervisor echoes the daemon's own request id: the replay
            # has to hand the candidate its id, not the reference's.
            send({"request_id": control["request_id"], "kind": "supervisor.ai_secret_store.response",
                  "data": {"provider_name": "bridge", "state": "ok"}})
            assert receive()["kind"] == "ai.providers.set_api_key"
            send({"request_id": "shutdown-1", "kind": "daemon.shutdown"})
            assert receive()["kind"] == "daemon.shutdown"
            process.stdin.close()
            assert process.wait(timeout=60) == 0
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            for stream in (process.stdout, process.stderr):
                stream.close()
    (session,) = session_dirs([recordings])
    return session


@functools.lru_cache(maxsize=None)
def encrypted_session() -> Path:
    """A book encrypted and then read with passphrases from inherited descriptors."""
    recordings = _new_recording()
    passphrase = cli_runner.InputFd(_PASSPHRASE.encode("utf-8") + b"\n")
    with tempfile.TemporaryDirectory(prefix="kassiber-session-encrypted-") as sandbox, _recording_env(recordings):
        data_root = str(Path(sandbox) / "data")
        for args in (
            ("init",),
            ("secrets", "init", "--new-passphrase-fd", passphrase),
            ("--db-passphrase-fd", passphrase, "workspaces", "create", "Main"),
            ("--db-passphrase-fd", passphrase, "workspaces", "list"),
        ):
            result = cli_runner.run(["--data-root", data_root, "--machine", *args], capture_output=True, text=True, timeout=300)
            assert result.returncode == 0, (args, result.stdout[-2000:], result.stderr[-2000:])
    (session,) = session_dirs([recordings])
    return session


def _replay(session: Path, mode: str | None = None):
    candidate = None if mode is None else (sys.executable, str(INJECT), mode)
    return replay(session, candidate, diagnose=True, extra_env=_GUARDED)


class RecordingTest(unittest.TestCase):
    def test_a_session_keeps_what_the_process_saw_and_did(self):
        session = cli_session()
        manifest = json.loads((session / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["format"], policy.SESSION_FORMAT)
        self.assertEqual(manifest["recorder_version"], policy.RECORDER_VERSION)
        self.assertEqual(manifest["comparison_policy"], policy.COMPARISON_POLICY)
        reference = manifest["reference"]
        for key in ("git_commit", "kassiber_version", "tax_engine_version", "schema_fingerprint"):
            self.assertTrue(reference[key], key)
        self.assertTrue(manifest["synthetic"])
        self.assertEqual(manifest["steps"], 9)
        self.assertEqual(sorted(root["kind"] for root in manifest["roots"].values()), sorted(policy.STATE_DIRNAMES))
        self.assertEqual(manifest["incomplete"], [])
        events = [json.loads(line) for line in (session / "events.jsonl").read_text(encoding="utf-8").splitlines()]
        spawns = [event for event in events if event["event"] == "spawn"]
        self.assertEqual(spawns[0]["args"][-1], "init")
        # The machine's own paths are not kept, only that they were inherited.
        self.assertEqual(spawns[0]["env"].get("HOME"), {"inherit": True})
        imported = next(event for event in spawns if "import-csv" in event["args"])
        (fixture,) = [item for item in imported["inputs"] if item.get("path", "").endswith("book.csv")]
        self.assertEqual((session / "blobs" / fixture["blob"]).read_text(encoding="utf-8"), _BOOK_CSV)
        exits = [event["returncode"] for event in events if event["event"] == "exit"]
        self.assertEqual(exits, [0, 0, 0, 0, 0, 0, 1, 0, 0])

    def test_a_daemon_session_keeps_the_ordered_exchange(self):
        session = daemon_session()
        events = [json.loads(line) for line in (session / "events.jsonl").read_text(encoding="utf-8").splitlines()]
        kinds = [event["event"] for event in events]
        self.assertEqual(kinds[0], "spawn")
        self.assertEqual(kinds[-1], "exit")
        stdout = "".join(event.get("text", "") for event in events if event["event"] == "stdout")
        records = [json.loads(line)["kind"] for line in stdout.splitlines()]
        self.assertEqual(
            records,
            ["daemon.ready", "status", "ai.providers.create", "supervisor.ai_secret_store.request",
             "ai.providers.set_api_key", "daemon.shutdown"],
        )
        # Each stdin write knows how many lines the daemon had printed by then.
        writes = [event for event in events if event["event"] == "stdin"]
        self.assertEqual([event["after"]["1"] for event in writes], [1, 2, 3, 4, 5])
        manifest = json.loads((session / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["notes"], ["the test process opened kassiber.sqlite3 while a recorded process was live"])


class InheritedDescriptorTest(unittest.TestCase):
    def test_descriptor_content_is_kept_as_synthetic_fixture_data(self):
        session = encrypted_session()
        events = [json.loads(line) for line in (session / "events.jsonl").read_text(encoding="utf-8").splitlines()]
        inputs = [arg for event in events if event["event"] == "spawn" for arg in event["args"] if isinstance(arg, dict)]
        self.assertEqual(len(inputs), 3)
        for item in inputs:
            self.assertTrue(item["synthetic"])
            self.assertEqual((session / "blobs" / item["fd_input"]).read_text(encoding="utf-8"), _PASSPHRASE + "\n")

    def test_an_encrypted_book_replays_and_compares_with_the_recorded_passphrase(self):
        report = _replay(encrypted_session())
        self.assertEqual(report.diagnostics["clock+entropy+order"], 0, [str(d) for d in report.residual])
        # The comparison read the encrypted book's rows rather than its bytes.
        self.assertFalse([d for d in report.differences if d.category in ("bytes", "error")])

    def test_the_recorded_book_opens_only_with_the_recorded_passphrase(self):
        session = encrypted_session()
        manifest = json.loads((session / "manifest.json").read_text(encoding="utf-8"))
        end = json.loads((session / "states" / f"{manifest['states']['end']}.json").read_text(encoding="utf-8"))
        book = (session / "blobs" / end["r0"]["entries"]["kassiber.sqlite3"]["sha256"]).read_bytes()
        self.assertFalse(dbdump.is_sqlite(book))
        dump = dbdump.dump_files(book, keys=["wrong", _PASSPHRASE])
        self.assertEqual([row[1][dump.tables["workspaces"].columns.index("label")] for row in dump.tables["workspaces"].rows], ["Main"])
        with self.assertRaises(dbdump.UnreadableDatabase):
            dbdump.dump_files(book)


class FreshPythonReplayTest(unittest.TestCase):
    def test_cli_session_replays_clean_once_clock_and_entropy_are_relaxed(self):
        report = _replay(cli_session())
        self.assertEqual(report.diagnostics["clock+entropy+order"], 0, [str(d) for d in report.residual])
        # Whatever the strict lane finds is clock readings or generated values.
        self.assertTrue(all(d.category.split(":")[0] in {"binding", "order", "value"} for d in report.differences))

    def test_daemon_session_replays_with_the_control_request_id_translated(self):
        report = _replay(daemon_session())
        self.assertEqual(report.diagnostics["clock+entropy+order"], 0, [str(d) for d in report.residual])
        self.assertFalse([d for d in report.differences if d.category in ("timeout", "exit", "length")])

    def test_a_replay_moves_away_from_paths_still_in_use(self):
        session = cli_session()
        manifest = json.loads((session / "manifest.json").read_text(encoding="utf-8"))
        data_root = Path(manifest["roots"]["r0"]["path"])
        (anchor,) = [Path(path) for path in manifest["anchors"] if policy.inside(data_root, Path(path))]
        anchor.mkdir(parents=True)
        try:
            report = _replay(session)
        finally:
            anchor.rmdir()
        self.assertIn(str(anchor), report.relocated)
        self.assertEqual(report.diagnostics["clock+entropy+order"], 0, [str(d) for d in report.residual])


class InjectedDiscrepancyTest(unittest.TestCase):
    """Every lane, relaxed or not, must report each injected difference."""

    def assertCaught(self, mode: str, where: str, category: str | None = None, *, session=None) -> None:
        report = _replay(session or cli_session(), mode)
        self.assertEqual(report.status, "different", mode)
        for lane, found in (("strict", report.differences), ("clock+entropy+order", report.residual)):
            matching = [d for d in found if where in d.where and (category is None or d.category == category)]
            self.assertTrue(matching, f"{mode}: {lane} lane missed it: {[str(d) for d in found][:10]}")
        self.assertTrue(all(count for count in report.diagnostics.values()), report.diagnostics)

    def test_the_pass_through_candidate_is_clean(self):
        report = _replay(cli_session(), "none")
        self.assertEqual(report.diagnostics["clock+entropy+order"], 0, [str(d) for d in report.residual])

    def test_an_altered_output_amount(self):
        self.assertCaught("output-amount", "stdout.data", "value")

    def test_an_altered_exit_status(self):
        self.assertCaught("exit", "proc 8", "exit")

    def test_swapped_referenced_ids_in_output(self):
        self.assertCaught("output-id", "stdout.data", "binding:uuid")

    def test_another_error_on_a_caught_error_path(self):
        self.assertCaught("error-code", "proc 7 stdout.error.code", "value")

    def test_another_exit_status_on_a_caught_error_path(self):
        self.assertCaught("error-exit", "proc 7", "exit")

    def test_a_stored_amount(self):
        self.assertCaught("book-amount", "transactions", "value")

    def test_a_link_between_rows(self):
        self.assertCaught("book-link", ".wallet_id", "binding:uuid")

    def test_a_timestamp_boundary(self):
        self.assertCaught("book-timestamp", ".occurred_at", "value")

    def test_a_stored_amount_changed_by_a_grandchild_process(self):
        self.assertCaught("subprocess-book-amount", "transactions", "value")

    def test_a_daemon_response(self):
        self.assertCaught("daemon-response", "proc 1 stdout#2", "keys", session=daemon_session())


class ComparatorTest(unittest.TestCase):
    def setUp(self):
        self.mapper = Mapper(Window(1000.0, 1010.0), Window(5000.0, 5010.0))

    def compare(self, ref, cand) -> list[Difference]:
        out: list[Difference] = []
        self.mapper.compare_value(ref, cand, "$", out)
        return out

    def test_json_number_spelling_does_not_matter_but_type_and_order_do(self):
        out: list[Difference] = []
        self.mapper.compare_stream(b'{"a": 1e-05, "b": [1, 2]}\n', b'{"a": 1e-5, "b": [1, 2]}\n', "$", out)
        self.assertEqual(out, [])
        self.assertEqual([d.category for d in self.compare({"a": 1, "b": 2}, {"b": 2, "a": 1})], ["keys"])
        self.assertEqual([d.category for d in self.compare([1, 2], [2, 1])], ["value", "value"])
        self.assertEqual([d.category for d in self.compare({"a": 1}, {"a": 1.0})], ["type"])

    def test_generated_ids_map_bijectively(self):
        a, b = "8c1f2a4e-1d2b-4c3d-9e4f-5a6b7c8d9e0f", "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"
        x, y = "11111111-2222-4333-8444-555555555555", "66666666-7777-4888-9999-aaaaaaaaaaaa"
        self.assertEqual(self.compare([a, b, a], [x, y, x]), [])
        self.assertEqual([d.category for d in self.compare([a], [y])], ["binding:uuid"])
        self.assertEqual(self.mapper.translate(f"wallets get {a}"), f"wallets get {x}")

    def test_inputs_pin_identities_to_themselves(self):
        a, x = "8c1f2a4e-1d2b-4c3d-9e4f-5a6b7c8d9e0f", "11111111-2222-4333-8444-555555555555"
        self.mapper.pin_text(f"--reference {a}")
        self.assertEqual([d.category for d in self.compare([a], [x])], ["binding:uuid"])

    def test_clock_readings_map_inside_the_window_only(self):
        ref = "1970-01-01T00:16:45Z"  # 1005, inside the reference window
        cand = "1970-01-01T01:23:25Z"  # 5005, inside the candidate window
        self.assertEqual(self.compare(ref, cand), [])
        self.assertEqual([d.category for d in self.compare("2026-01-01T00:00:00Z", "2026-01-01T00:00:01Z")], ["value"])
        later_ref, earlier_cand = "1970-01-01T00:16:48Z", "1970-01-01T01:23:21Z"
        self.assertEqual([d.category for d in self.compare(later_ref, earlier_cand)], ["order:time"])

    def test_database_rows_compare_through_their_bound_keys(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-session-dump-") as scratch:
            paths = []
            for name, rows in (("ref", [("8c1f2a4e-1d2b-4c3d-9e4f-5a6b7c8d9e0f", 5)]),
                               ("cand", [("11111111-2222-4333-8444-555555555555", 6)])):
                path = Path(scratch) / f"{name}.sqlite3"
                conn = sqlite3.connect(path)
                conn.execute("CREATE TABLE t (id TEXT PRIMARY KEY, amount INTEGER)")
                conn.executemany("INSERT INTO t VALUES (?, ?)", rows)
                conn.commit()
                conn.close()
                paths.append(path.read_bytes())
            from tests.session_harness.compare import compare_file

            out: list[Difference] = []
            compare_file(paths[0], paths[1], "db", self.mapper, out)
            self.assertEqual([(d.where, d.category) for d in out], [("db/t[8c1f2a4e-1d2b-4c3d-9e4f-5a6b7c8d9e0f].amount", "value")])


class RefusalTest(unittest.TestCase):
    def _refusals(self, directory: Path) -> list[str]:
        path = directory / "refusals.jsonl"
        return [json.loads(line)["reason"] for line in path.read_text(encoding="utf-8").splitlines()] if path.exists() else []

    def test_a_data_root_outside_the_temp_area_is_not_recorded(self):
        recordings = _new_recording()
        outside = ROOT / ".session-refusal-probe" / "data"
        with _recording_env(recordings):
            result = cli_runner.run(["--data-root", str(outside), "--machine", "--version"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0)
        self.assertFalse(outside.parent.exists())
        self.assertEqual(self._refusals(recordings), ["data root outside the test temp area"])
        self.assertFalse((recordings / "sessions").exists())

    def test_a_secret_from_the_environment_is_never_kept(self):
        recordings = _new_recording()
        secret = "synthetic-probe-secret-value"
        with tempfile.TemporaryDirectory(prefix="kassiber-session-secret-") as sandbox, _recording_env(recordings):
            with mock.patch.dict(os.environ, {"PROBE_API_TOKEN": secret}):
                recorder._RECORDERS.pop(recordings.resolve(), None)
                cli_runner.run(["--data-root", str(Path(sandbox) / "data"), "--machine", "workspaces", "create", secret],
                               capture_output=True, text=True)
        self.assertEqual(self._refusals(recordings), ["a secret value from the test environment reached the session"])
        stored = [path for path in recordings.rglob("*") if path.is_file()]
        self.assertTrue(stored)
        for path in stored:
            self.assertNotIn(secret.encode(), path.read_bytes(), path)

    def test_secret_named_product_variables_and_marked_stdin_are_refused(self):
        recordings = _new_recording()
        with tempfile.TemporaryDirectory(prefix="kassiber-session-secret-") as sandbox, _recording_env(recordings):
            data_root = str(Path(sandbox) / "data")
            env = {**os.environ, "KASSIBER_BACKEND_PROBE_TOKEN": "x"}
            cli_runner.run(["--data-root", data_root, "--machine", "--version"], env=env, capture_output=True)
        with tempfile.TemporaryDirectory(prefix="kassiber-session-secret-") as sandbox, _recording_env(recordings):
            cli_runner.run(["--data-root", str(Path(sandbox) / "data"), "--machine", "--version"],
                           input="token", secret_input=True, capture_output=True, text=True)
        self.assertEqual(
            self._refusals(recordings),
            ["secret-named environment variable KASSIBER_BACKEND_PROBE_TOKEN is set", "stdin carries a credential"],
        )


if __name__ == "__main__":
    unittest.main()
