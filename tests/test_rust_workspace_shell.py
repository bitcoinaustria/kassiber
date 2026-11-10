"""Workspace shell reads and onboarding: Python vs native bytes and books."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sqlite3
import sys
from unittest import mock

import pytest

from kassiber.db import open_db
from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.test_rust_project_desktop import BINARY, environment
from tests.test_rust_shell_parity import RUST_ONLY_BACKEND_KINDS
from crates.workspace.scripts import export_vectors as vectors

pytestmark = pytest.mark.skipif(not BINARY, reason="set KASSIBER_RUST_BIN to the test-harness binary")
KEY = "R5 workspace synthetic '🔑"
NOW = "2026-01-01T00:00:00Z"
REFERENCE = (
    "import sys, itertools, uuid; from pathlib import Path; from kassiber.operator import project; "
    "owners=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:owners; "
    "from kassiber.core import accounts; from kassiber import backends; "
    "counter=itertools.count(1); "
    "accounts.uuid.uuid4=lambda:uuid.UUID('00000000-0000-4000-8000-%012x'%next(counter)); "
    f"accounts.now_iso=lambda:{NOW!r}; backends.now_iso=lambda:{NOW!r}; "
    "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
)
CASES = {name: sql for group, name, sql in (*vectors.cases(), *vectors.overview_cases())}
SHOWN = ["empty", "unprocessed", "current", "semantics_built", "cycle", "ov_journal_rates",
         "ov_canonical_issues", "ov_inventory_current", "ov_pairs", "ov_books_current", "ov_generic"]


def run(tmp_path, argv, *, reference, data=None):
    owners, env = environment(tmp_path)
    env |= {"KASSIBER_RUST_TEST_SETUP_IDS": "sequential", "KASSIBER_RUST_TEST_SETUP_TIMESTAMP": NOW,
            "KASSIBER_RUST_TEST_SECRETS_TIME": NOW}
    original = cli_runner.prefix

    def prefix(*, reference=False, python_options=()):
        if reference:
            return (sys.executable, "-c", REFERENCE, str(owners))
        return original(reference=reference, python_options=python_options)

    with mock.patch.object(cli_runner, "prefix", side_effect=prefix), \
            mock.patch.dict(os.environ, {cli_runner.CLI_ENV: env[cli_runner.CLI_ENV]}):
        return cli_runner.run(list(argv), reference=reference, env=env, input=data,
                              capture_output=True, timeout=600)


def session(tmp_path, root, requests, *, reference, unlock=None, traceback=False):
    lines = []
    if unlock:
        lines.append({"request_id": "unlock", "kind": "daemon.unlock",
                      "args": {"auth_response": {"passphrase_secret": unlock}}})
    lines += [{"request_id": i, "kind": kind, "args": args} for i, (kind, args) in enumerate(requests)]
    lines.append({"request_id": "shutdown", "kind": "daemon.shutdown"})
    data = "".join(json.dumps(line) + "\n" for line in lines).encode()
    result = run(tmp_path, ["--data-root", str(root), "daemon"], reference=reference, data=data)
    assert result.returncode == 0, result.stderr
    # Python prints a traceback for an unhandled reference exception.
    assert result.stderr == b"" or (traceback and reference), result.stderr
    if not reference and (witness := os.environ.get("KASSIBER_WORKSPACE_COUNTS")):
        with Path(witness).open("a") as handle:
            for kind, _ in requests:
                handle.write(json.dumps({"command": kind, "served_by": "rust"}) + "\n")
    return result.stdout.splitlines(keepends=True)[1:]


def dump(database, key=None):
    conn = (open_encrypted(database, key, enforce_operator_identity=False) if key
            else sqlite3.connect(database.as_uri() + "?mode=ro", uri=True))
    try:
        tables = [r[0] for r in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        assert len(tables) == 116
        rows = {t: sorted((tuple(r) for r in conn.execute(f'SELECT * FROM "{t}"')), key=repr) for t in tables}
    finally:
        conn.close()
    # A book created during the session receives a random identity and
    # wall-clock migration audits; compare their shape only.
    rows["settings"] = [(k, "<random>" if k == "database_instance_id" else v) for k, v in rows["settings"]]
    rows["schema_migration_audits"] = sorted(
        (*row[:-2], json.loads(row[-2]), "<stamp>") for row in rows["schema_migration_audits"])
    return rows


def seeded(tmp_path, sql, key=None):
    root = tmp_path / "book/data"
    conn = open_db(root, passphrase=key)
    conn.executescript(sql)
    conn.commit()
    conn.close()
    # Settle compatibility backfills once so neither side migrates first.
    open_db(root, passphrase=key).close()
    return root


def _python_backend_kinds(line: bytes) -> bytes:
    """Rust's backend-kind hint without the Rust-only kinds
    (test_rust_shell_parity.RUST_ONLY_BACKEND_KINDS, rust-shell.md#rust-only-commands);
    every other byte must be Python's."""
    for kind in RUST_ONLY_BACKEND_KINDS:
        token = kind.encode()
        line = (line.replace(b"Choose one of: " + token + b", ", b"Choose one of: ")
                .replace(b", " + token + b",", b",")
                .replace(b", " + token + b'"', b'"'))
    return line


def compare_session(tmp_path, root, requests, key=None, traceback=False):
    database = root / "kassiber.sqlite3"
    before = database.read_bytes()
    initial = dump(database, key)
    expected = session(tmp_path, root, requests, reference=True, unlock=key, traceback=traceback)
    expected_rows = dump(database, key)
    database.write_bytes(before)
    for sidecar in ("-wal", "-shm"):
        Path(str(database) + sidecar).unlink(missing_ok=True)
    actual = session(tmp_path, root, requests, reference=False, unlock=key)
    assert [_python_backend_kinds(line) for line in actual] == expected
    assert dump(database, key) == expected_rows
    responses = [json.loads(line) for line in actual]
    assert all(r.get("error", {}).get("code") != "not_implemented" for r in responses)
    return initial, expected_rows, responses


READS = [
    ("ui.overview.snapshot", {}),
    ("ui.overview.snapshot", [1]),
    ("ui.workspace.overview.snapshot", {"workspace_id": " w "}),
    ("ui.workspace.overview.snapshot", {"workspace_id": "missing"}),
    ("ui.workspace.overview.snapshot", {"workspace_id": "w", "extra": 1, "another": 2}),
    ("ui.workspace.overview.snapshot", {"workspace_id": " "}),
    ("ui.workspace.overview.snapshot", []),
    ("ui.workspace.overview.snapshot", None),
    ("ui.workspace.health", {}),
    ("ui.workspace.health", "ignored"),
    ("ui.next_actions", {}),
    ("ui.review.badges", {"ignored": True}),
]


@pytest.mark.parametrize("name", SHOWN)
def test_workspace_reads_match_python_bytes_and_leave_books_unchanged(tmp_path, name):
    root = seeded(tmp_path, CASES[name])
    initial, after, responses = compare_session(tmp_path, root, READS)
    assert initial == after
    assert responses[0]["kind"] == "ui.overview.snapshot"


def test_workspace_reads_on_encrypted_book_require_unlock(tmp_path):
    root = seeded(tmp_path, CASES["ov_pairs"], key=KEY)
    database = root / "kassiber.sqlite3"
    before = database.read_bytes()
    locked = session(tmp_path, root, READS[:1], reference=True)
    database.write_bytes(before)
    assert session(tmp_path, root, READS[:1], reference=False) == locked
    assert json.loads(locked[0])["kind"] == "auth_required"
    initial, after, _ = compare_session(tmp_path, root, READS, key=KEY)
    assert initial == after


@pytest.mark.parametrize("fmt", ["json", "table", "plain"])
def test_health_and_next_actions_cli_match_python(tmp_path, fmt):
    root = seeded(tmp_path, CASES["ov_journal_rates"])
    database = root / "kassiber.sqlite3"
    before = database.read_bytes()
    for argv in (["health"], ["next-actions"], ["health", "--workspace", "w", "--profile", "foreign"],
                 ["next-actions", "--workspace", "Main", "--profile", "Book"], ["health", "--workspace", "w"],
                 ["next-actions", "--profile", "p"], ["health", "--workspace", "missing", "--profile", "p"]):
        args = ["--format", fmt, "--data-root", str(root), *argv]
        expected = run(tmp_path, args, reference=True)
        database.write_bytes(before)
        actual = run(tmp_path, args, reference=False)
        assert (actual.returncode, actual.stdout, actual.stderr) == (
            expected.returncode, expected.stdout, expected.stderr), argv
        assert database.read_bytes() == before
    records = [json.loads(line) for line in (tmp_path / "served.jsonl").read_text().splitlines()]
    assert records and all(r["served_by"] == "rust" for r in records), records
    if witness := os.environ.get("KASSIBER_WORKSPACE_COUNTS"):
        with Path(witness).open("a") as handle:
            for record in records:
                handle.write(json.dumps(record) + "\n")


ONBOARDING = [
    ("ui.onboarding.complete", [1]),
    ("ui.onboarding.complete", {}),
    ("ui.onboarding.complete", {"workspace_label": 5, "profile_label": "Book"}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": " "}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "tax_long_term_days": "x"}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "backend_setup_mode": "odd"}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "backend_setup_mode": "skip",
                                "backend": {}}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "include_liquid_backends": 1}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "backend": []}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "backend": {"name": "x"}}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book",
                                "backend": {"name": "x", "kind": "Gopher", "url": "u"}}),
    ("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "backend": {}}),
]
FINAL = {
    "default": {"workspace_label": " Main ", "profile_label": "Book", "tax_country": "at", "fiat_currency": "eur",
                "tax_long_term_days": "400"},
    "liquid": {"workspace_label": "Main", "profile_label": "Book", "include_liquid_backends": True,
               "gains_algorithm": "hifo", "tax_long_term_days": 7.9},
    "skip": {"workspace_label": "Main", "profile_label": "Book", "backend_setup_mode": "skip"},
    "custom": {"workspace_label": "Main", "profile_label": "Book",
               "backend": {"name": " Node ", "kind": "ESPLORA", "url": "http://127.0.0.1:9", "chain": "bitcoin",
                           "network": "regtest", "insecure": 0, "certificate": " cert ", "notes": "n"}},
}


@pytest.mark.parametrize("mode", sorted(FINAL))
def test_onboarding_matches_python_wire_and_all_tables(tmp_path, mode):
    root = tmp_path / "book/data"
    assert run(tmp_path, ["--machine", "--data-root", str(root), "init"], reference=True).returncode == 0
    requests = [*ONBOARDING, ("ui.onboarding.complete", FINAL[mode]),
                ("ui.onboarding.complete", FINAL[mode]), ("status", {}), ("ui.profiles.snapshot", {}),
                ("ui.workspace.health", {})]
    initial, after, responses = compare_session(tmp_path, root, requests)
    assert initial != after
    assert responses[len(ONBOARDING)]["kind"] == "ui.onboarding.complete"
    assert responses[len(ONBOARDING) + 1]["error"]["code"] == "conflict"
    shutil.rmtree(tmp_path / "book", ignore_errors=True)


def stamped(lines):
    import re
    return [re.sub(rb"pre-credentials-migration-\d{8}T\d{6}Z", b"pre-credentials-migration-<ts>", line)
            for line in lines]


def secrets_session(tmp_path, root, requests, *, reference, key):
    return stamped(session(tmp_path, root, requests, reference=reference))


@pytest.mark.parametrize("existing", [True, False], ids=["plaintext-book", "no-database"])
def test_secrets_init_matches_python_and_migrates_dotenv_credentials(tmp_path, existing):
    root = tmp_path / "book/data"
    env_file = tmp_path / "book/config/backends.env"
    env_file.parent.mkdir(parents=True)
    env_file.write_text("KASSIBER_BACKEND_CORE_KIND=bitcoinrpc\nKASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332\n"
                        "KASSIBER_BACKEND_CORE_RPCUSER=alice\nKASSIBER_BACKEND_CORE_RPCPASSWORD=hunter2\n")
    if existing:
        assert run(tmp_path, ["--machine", "--data-root", str(root), "init"], reference=True).returncode == 0
    else:
        root.mkdir(parents=True)
    snapshot = {p.relative_to(tmp_path): p.read_bytes() for p in (tmp_path / "book").rglob("*") if p.is_file()}
    requests = [
        ("ui.secrets.init", []),
        ("ui.secrets.init", {}),
        ("ui.secrets.init", {"auth_response": {"passphrase_secret": "short"}}),
        ("ui.secrets.init", {"auth_response": {"passphrase_secret": KEY}}),
        ("status", {}),
        ("ui.secrets.init", {"auth_response": {"passphrase_secret": "wrong passphrase!"}}),
        ("ui.secrets.init", {"auth_response": {"passphrase_secret": KEY}, "migrate_credentials": False}),
        ("ui.backends.settings.list", {}),
    ]

    def files():
        import re
        output = {}
        for path in (tmp_path / "book").rglob("*"):
            if not path.is_file() or ".operator-owner" in path.name:
                continue
            name = re.sub(r"pre-credentials-migration-\d{8}T\d{6}Z", "<ts>", path.relative_to(tmp_path).as_posix())
            # Encrypted pages carry random salts; their rows are compared instead.
            output[name] = None if "sqlite3" in path.name else path.read_bytes()
        return output

    expected = secrets_session(tmp_path, root, requests, reference=True, key=KEY)
    expected_files = files()
    expected_rows = dump(root / "kassiber.sqlite3", KEY)
    shutil.rmtree(tmp_path / "book")
    shutil.rmtree(tmp_path / "owners", ignore_errors=True)
    for path, data in snapshot.items():
        (tmp_path / path).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / path).write_bytes(data)
    root.mkdir(parents=True, exist_ok=True)
    actual = secrets_session(tmp_path, root, requests, reference=False, key=KEY)
    assert actual == expected
    assert files() == expected_files
    assert dump(root / "kassiber.sqlite3", KEY) == expected_rows
    responses = [json.loads(line) for line in actual]
    assert responses[3]["data"]["encrypted"] is True
    # Without a stored backend row Python skips the secret, as Rust does.
    assert (b"hunter2" in (tmp_path / "book/config/backends.env").read_bytes()) is not existing


def test_generic_cancel_keeps_python_unclassified_refusal(tmp_path):
    root = seeded(tmp_path, "")
    requests = [("cancel", {}), ("cancel", None), ("cancel", {"request_id": "x", "target": "1"}), ("status", {})]
    _, _, responses = compare_session(tmp_path, root, requests)
    assert [r.get("error", {}).get("code") for r in responses[:3]] == ["unsupported_kind"] * 3


def test_onboarding_unknown_tax_country_matches_python_except_traceback(tmp_path):
    """tests/rust_port/repro_onboarding_tax_country.py: Python's unhandled
    ValueError carries a traceback in `debug`; Rust leaves it empty."""
    root = tmp_path / "book/data"
    assert run(tmp_path, ["--machine", "--data-root", str(root), "init"], reference=True).returncode == 0
    database = root / "kassiber.sqlite3"
    before = database.read_bytes()
    request = [("ui.onboarding.complete", {"workspace_label": "Main", "profile_label": "Book", "tax_country": "xx"}),
               ("ui.profiles.snapshot", {})]
    expected = [json.loads(line) for line in session(tmp_path, root, request, reference=True, traceback=True)]
    rows = dump(database)
    database.write_bytes(before)
    actual = [json.loads(line) for line in session(tmp_path, root, request, reference=False)]
    assert expected[0]["error"].pop("debug").startswith("Traceback")
    assert actual[0]["error"].pop("debug") in (None, "")
    assert actual == expected
    assert dump(database) == rows
