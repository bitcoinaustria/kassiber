"""Desktop project catalog/switching: byte envelopes, catalog and 112-table books."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
from unittest import mock

import pytest

from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not BINARY, reason="set KASSIBER_RUST_BIN to the test-harness binary")
KEY = "R5 project synthetic '🔑"
NOW = "2030-01-02T03:04:05Z"
KINDS = ("ui.projects.list", "ui.projects.create", "ui.projects.select")
REFERENCE = (
    "import sys; from pathlib import Path; from kassiber.operator import project; "
    "owners=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:owners; "
    f"from kassiber import projects; projects.now_iso=lambda:{NOW!r}; "
    f"from kassiber import daemon; daemon.now_iso=lambda:{NOW!r}; "
    "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
)


def environment(tmp_path):
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith(("KASSIBER_", "SATBOOKS_"))}
    # HOME is inherited; every Kassiber location is a disposable override.
    return owners, env | {
        "KASSIBER_NO_EGRESS": "1", "KASSIBER_TEST_NO_EGRESS": "strict",
        "KASSIBER_DISABLE_UPDATE_CHECK": "1",
        # The desktop daemon starts without --data-root, so the default state
        # root must be disposable. HOME stays inherited; XDG selects the root.
        "XDG_DATA_HOME": str(tmp_path / "xdg"),
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(tmp_path / "offline.json"),
        "KASSIBER_UPDATE_PREFERENCE_FILE": str(tmp_path / "updates.json"),
        "KASSIBER_UPDATE_CACHE_FILE": str(tmp_path / "cache.json"),
        "KASSIBER_AGENT_ACCESS_FILE": str(tmp_path / "agents.json"),
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_RUST_TEST_PROJECT_TIMESTAMP": NOW,
        cli_runner.CLI_ENV: json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]),
        "KASSIBER_HYBRID_RUST_BIN": str(Path(BINARY).resolve()),
        "KASSIBER_HYBRID_RUST_DAEMON": "1",
        "KASSIBER_HYBRID_LOG": str(tmp_path / "served.jsonl"),
    }


def state_root(tmp_path):
    return tmp_path / "xdg/kassiber"


def exchange(tmp_path, requests, *, reference, argv=("daemon",)):
    owners, env = environment(tmp_path)
    # Refuse to start a daemon unless the default project catalog it will
    # write resolves inside this test's directory.
    probe = subprocess.run(
        [sys.executable, "-I", "-c", "from kassiber.db import default_state_root; print(default_state_root())"],
        cwd=ROOT, env=env, capture_output=True, text=True, check=True)
    assert Path(probe.stdout.strip()) == state_root(tmp_path), probe.stdout
    original = cli_runner.prefix

    def prefix(*, reference=False, python_options=()):
        if reference:
            return (sys.executable, "-c", REFERENCE, str(owners))
        return original(reference=reference, python_options=python_options)

    lines = [{"request_id": i, "kind": kind, "args": args} for i, (kind, args) in enumerate(requests)]
    lines.append({"request_id": "shutdown", "kind": "daemon.shutdown"})
    data = "".join(json.dumps(line) + "\n" for line in lines).encode()
    with mock.patch.object(cli_runner, "prefix", side_effect=prefix), \
            mock.patch.dict(os.environ, {cli_runner.CLI_ENV: env[cli_runner.CLI_ENV]}):
        result = cli_runner.run(list(argv), reference=reference, env=env, input=data,
                                capture_output=True, timeout=180)
    assert result.returncode == 0, result.stderr
    assert result.stderr == b""
    return result.stdout.splitlines(keepends=True)


def dump(database, key):
    if not database.exists():
        return None
    conn = (open_encrypted(database, key, enforce_operator_identity=False) if key
            else sqlite3.connect(database.as_uri() + "?mode=ro", uri=True))
    try:
        tables = [r[0] for r in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        assert len(tables) == 112
        rows = {t: sorted((tuple(r) for r in conn.execute(f'SELECT * FROM "{t}"')), key=repr) for t in tables}
    finally:
        conn.close()
    # Each new book receives random identities; compare their shape only.
    rows["settings"] = [(k, "<random>" if k in RANDOM_SETTINGS else v) for k, v in rows["settings"]]
    # Fresh-book migration audits carry wall-clock stamps; their JSON spelling
    # is the store's existing creation contract, compared here by value.
    rows["schema_migration_audits"] = sorted(
        (*row[:-2], json.loads(row[-2]), "<stamp>") for row in rows["schema_migration_audits"])
    return rows


RANDOM_SETTINGS = {"database_instance_id"}


def state(tmp_path, encrypted):
    root = state_root(tmp_path)
    catalog = root / "config/projects.json"
    books = {}
    for database in sorted(root.glob("projects/*/data/kassiber.sqlite3")):
        project = database.parent.parent.name
        books[project] = (database.read_bytes() if project == "occupied"
                          else dump(database, encrypted.get(project) if isinstance(encrypted, dict)
                                    else KEY if project in encrypted else None))
    explicit = tmp_path / "explicit/data/kassiber.sqlite3"
    if explicit.exists():
        books["<explicit>"] = dump(explicit, None)
    return catalog.read_bytes() if catalog.exists() else None, books


def compare(tmp_path, requests, *, encrypted=(), argv=("daemon",)):
    expected = exchange(tmp_path, requests, reference=True, argv=argv)
    expected_state = state(tmp_path, encrypted)
    shutil.rmtree(tmp_path / "xdg")
    shutil.rmtree(tmp_path / "owners")
    shutil.rmtree(tmp_path / "explicit", ignore_errors=True)
    (tmp_path / "served.jsonl").unlink(missing_ok=True)
    actual = exchange(tmp_path, requests, reference=False, argv=argv)
    assert actual[1:] == expected[1:]
    ready = json.loads(actual[0])["data"]["supported_kinds"]
    assert set(KINDS) <= set(ready)
    actual_state = state(tmp_path, encrypted)
    assert actual_state[0] == expected_state[0]
    assert actual_state[1].keys() == expected_state[1].keys()
    for project, rows in expected_state[1].items():
        differing = {t: (rows[t], actual_state[1][project][t]) for t in rows if rows[t] != actual_state[1][project][t]}
        assert not differing, (project, differing)
    records = [json.loads(line) for line in (tmp_path / "served.jsonl").read_text().splitlines()]
    assert records == [{"command": "daemon", "served_by": "rust"}]
    responses = [json.loads(line) for line in actual[1:-1]]
    assert all(r.get("error", {}).get("code") != "not_implemented" for r in responses)
    if witness := os.environ.get("KASSIBER_PROJECT_COUNTS"):
        with Path(witness).open("a") as handle:
            for kind, _ in requests:
                handle.write(json.dumps({"command": kind, "served_by": "rust"}) + "\n")
    return responses


def auth(secret):
    return {"auth_response": {"passphrase_secret": secret}}


def test_project_catalog_creation_and_switching_match_python(tmp_path):
    responses = compare(tmp_path, [
        ("ui.projects.list", {}),
        ("ui.projects.list", [1]),
        ("ui.projects.create", []),
        ("ui.projects.create", {"name": " \t"}),
        ("ui.projects.create", {"name": 7}),
        ("ui.projects.create", {"name": "Alpha", "project_id": 5}),
        ("ui.projects.create", {"name": "Alpha", **auth("short")}),
        ("ui.projects.create", {"name": " Alpha "}),
        ("status", {}),
        ("ui.projects.list", None),
        ("ui.projects.create", {"name": "ALPHA"}),
        ("ui.projects.create", {"name": "Beta", "select": False}),
        ("ui.projects.create", {"name": "Gamma", "project_id": "", "select": 0}),
        ("ui.projects.create", {"name": "Vault", "project_id": "Vault Book", "select": False, **auth(KEY)}),
        ("ui.projects.list", {}),
        ("ui.projects.select", {}),
        ("ui.projects.select", {"project_id": " "}),
        ("ui.projects.select", {"project_id": 3}),
        ("ui.projects.select", {"project_id": "missing"}),
        ("ui.projects.select", {"project_id": "", "id": "BETA"}),
        ("status", {}),
        ("ui.projects.select", {"project_id": "beta"}),
        ("ui.projects.select", {"project_id": "vault-book"}),
        ("ui.projects.select", {"project_id": "vault-book", **auth("wrong passphrase value")}),
        ("ui.projects.select", {"project_id": "vault-book", **auth(KEY)}),
        ("status", {}),
        ("ui.projects.select", {"project_id": "vault-book"}),
        ("daemon.lock", {}),
        ("ui.projects.select", {"project_id": "vault-book"}),
        ("ui.projects.select", {"project_id": "vault-book", **auth(KEY), "require_existing_project": True}),
        ("ui.projects.create", {"name": "Delta", **auth(KEY)}),
        ("status", {}),
        ("ui.projects.select", {"project_id": "alpha"}),
        ("ui.projects.list", {}),
    ], encrypted={"vault-book", "delta"})
    assert responses[7]["data"]["unlocked"] is True
    assert responses[22]["kind"] == "auth_required"
    assert responses[23]["error"]["code"] == "local_auth_denied"
    assert responses[24]["data"]["project"]["id"] == "vault-book"
    assert responses[30]["data"]["project"]["encrypted"] is True


def test_project_creation_refuses_existing_databases_and_unlocked_lock_cycle(tmp_path):
    occupied = state_root(tmp_path) / "projects/occupied/data"
    occupied.mkdir(parents=True)

    def requests():
        (occupied / "kassiber.sqlite3").write_bytes(b"not a database")
        return [
            ("ui.projects.create", {"name": "Occupied"}),
            ("ui.projects.list", {}),
            ("daemon.lock", {}),
            ("ui.projects.select", {"project_id": "default"}),
            ("ui.projects.create", {"name": "Next", "select": False}),
            ("ui.projects.select", {"id": "next", "require_existing_project": True}),
            ("ui.projects.select", {"id": "next"}),
            ("ui.projects.select", {"id": "default"}),
        ]

    expected = exchange(tmp_path, requests(), reference=True)
    expected_state = state(tmp_path, ())
    shutil.rmtree(tmp_path / "xdg")
    shutil.rmtree(tmp_path / "owners")
    occupied.mkdir(parents=True)
    actual = exchange(tmp_path, requests(), reference=False)
    assert actual[1:] == expected[1:]
    assert state(tmp_path, ()) == expected_state
    assert json.loads(actual[1])["error"]["code"] == "database_exists"


def test_explicit_root_switches_to_encrypted_project_and_reunlocks(tmp_path):
    exact = "abcdefghijkl"
    responses = compare(tmp_path, [
        ("ui.projects.list", {}),
        ("status", {}),
        ("ui.projects.create", {"name": "Main", "select": False, **auth(exact[:-1])}),
        ("ui.projects.create", {"name": "Main", "select": False, **auth(exact)}),
        ("ui.projects.select", {"project_id": "main"}),
        ("ui.projects.select", {"project_id": "main", **auth(exact)}),
        ("ui.projects.list", {}),
        ("status", {}),
        ("daemon.lock", {}),
        ("ui.projects.list", {}),
        ("ui.projects.select", {"project_id": "MAIN"}),
        ("ui.projects.select", {"project_id": "main", **auth(KEY)}),
        ("ui.projects.select", {"project_id": "main", **auth(exact)}),
        ("status", {}),
        ("ui.projects.create", {"name": "Main"}),
    ], encrypted={"main": exact}, argv=("--data-root", str(tmp_path / "explicit/data"), "daemon"))
    assert responses[0]["data"] == {"selected_project_id": None, "projects": []}
    assert responses[2]["error"]["code"] == "invalid_passphrase"
    assert responses[5]["data"]["project"]["encrypted"] is True
    assert responses[10]["kind"] == "auth_required"
    assert responses[11]["error"]["code"] == "local_auth_denied"
    assert responses[12]["kind"] == "ui.projects.select"


def test_require_existing_refuses_unrelated_sqlite_without_migrating_it(tmp_path):
    foreign = tmp_path / "foreign/data/kassiber.sqlite3"

    def seed():
        foreign.parent.mkdir(parents=True, exist_ok=True)
        foreign.unlink(missing_ok=True)
        conn = sqlite3.connect(foreign)
        conn.execute("CREATE TABLE unrelated(value TEXT)")
        conn.commit()
        conn.close()
        catalog = state_root(tmp_path) / "config/projects.json"
        catalog.parent.mkdir(parents=True, exist_ok=True)
        catalog.write_text(json.dumps({
            "schema_version": 1, "selected_project_id": "default",
            "projects": [
                {"id": "default", "name": "Default", "path": str(state_root(tmp_path) / "projects/default"),
                 "encrypted": False, "last_opened_at": None},
                {"id": "foreign", "name": "Foreign", "path": str(foreign.parent.parent),
                 "encrypted": False, "last_opened_at": None},
            ],
        }))
        catalog.chmod(0o600)

    requests = [
        ("ui.projects.select", {"project_id": "foreign", "require_existing_project": True}),
        ("status", {}),
        ("ui.projects.list", {}),
    ]
    seed()
    expected = exchange(tmp_path, requests, reference=True)
    expected_state = state(tmp_path, ())
    expected_foreign = foreign.read_bytes()
    shutil.rmtree(tmp_path / "xdg")
    shutil.rmtree(tmp_path / "owners")
    seed()
    actual = exchange(tmp_path, requests, reference=False)
    assert actual[1:] == expected[1:]
    assert state(tmp_path, ()) == expected_state
    assert foreign.read_bytes() == expected_foreign
    assert json.loads(actual[1])["error"]["code"] == "invalid_project_database"
