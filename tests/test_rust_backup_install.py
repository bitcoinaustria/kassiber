"""Restore publication oracle: real killed processes, all runtime combinations.

File transactions compare every DB/sidecar byte. Separate CLI scenarios open
both codecs and compare complete logical dumps. This is process interruption,
not a power-loss or native-platform simulation.
"""
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys

import pytest

from kassiber.backup import install
from kassiber.db import open_db
from kassiber.operator import project
from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.test_rust_backup_services import _bundle, _manifest, OUTER
from tests.test_rust_secrets_services import cli, KEY, ok  # noqa: F401
from tests.test_rust_store_migrations import normalized_dump

ROOT = Path(__file__).resolve().parents[1]
pytestmark = pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="set KASSIBER_RUST_BIN")
PROBE = ROOT / "target/debug/examples/kassiber-storage-probe"
PYTHON_PROBE = r'''
import json, sys
from pathlib import Path
from kassiber.backup import install
from kassiber.operator import project
request = json.loads(sys.stdin.readline())
data = Path(request['data_root'])
project._owner_lock_root = lambda: Path(request['owners'])
points = []
def observe(label):
    points.append(label)
    if label == request.get('kill_at'):
        print(json.dumps({'checkpoint': label}), flush=True)
        sys.stdin.readline()
try:
    with project.exclusive_project_maintenance(data, active_owner_kind=None):
        if request['op'] == 'restore-install':
            recovery = install.install_staged_backup(Path(request['staging']), data, _observer=observe)
        else:
            install.recover_staged_restore(data, _observer=observe)
            recovery = None
except install.AppError as error:
    print(json.dumps({'ok': False, 'error': {'code': error.code, 'message': str(error)}}))
    sys.exit(1)
print(json.dumps({'ok': True, 'data': {'recovery': str(recovery) if recovery else None, 'points': points}}))
'''


def _points(populated):
    result = ["restore.prepared", *[f"restore.copy_sync.{name}" for name in
              ("attachments/new", "config/backends.env", "config/settings.json", "kassiber.sqlite3")],
              "restore.copy_dir_sync.config", "restore.copy_dir_sync.attachments", "restore.copy_dir_sync.root",
              "restore.destinations_dir_sync", "restore.artifacts_dir_sync",
              "restore.pending.journal_sync", "restore.pending.journal_replace", "restore.pending.journal_dir_sync"]
    moves = ([f"save.{n}" for n in install.NAMES] if populated else []) + [f"install.{n}" for n in install.INCOMING]
    for move in moves:
        result += [f"restore.before_{move}", f"restore.after_{move}", f"restore.source_sync.{move}", f"restore.target_sync.{move}"]
    return result + ["restore.committed.journal_sync", "restore.committed.journal_replace", "restore.committed.journal_dir_sync",
                     "restore.recovery_dir_sync", "restore.journal_unlink", "restore.journal_unlink_sync", "restore.cleanup"]


def _recovery_points(populated):
    points = []
    if populated:
        for name in reversed(install.NAMES):
            points += [f"restore.rollback_remove.{name}", f"restore.before_rollback.{name}",
                       f"restore.after_rollback.{name}", f"restore.source_sync.rollback.{name}", f"restore.target_sync.rollback.{name}"]
    else:
        points += [f"restore.rollback_absent.{name}" for name in reversed(install.INCOMING)]
    return points + ["restore.recovery_dir_sync", "restore.journal_unlink", "restore.journal_unlink_sync", "restore.cleanup"]


@pytest.fixture(scope="module", params=["plaintext", "sqlcipher"])
def books(request, tmp_path_factory):
    root = tmp_path_factory.mktemp("restore-codec")
    pair = []
    for label in ("old", "new"):
        data = root / label
        conn = open_db(data, passphrase=KEY if request.param == "sqlcipher" else None)
        conn.execute("INSERT INTO settings(key,value) VALUES('restore_marker',?)", (label,))
        conn.commit()
        dump = normalized_dump(conn)
        conn.close()
        pair.append(((data / "kassiber.sqlite3").read_bytes(), dump))
    return request.param, pair


def _setup(root, books, populated):
    data, stage, owners = root / "book/data", root / "stage", root / "owners"
    data.mkdir(parents=True)
    stage.mkdir()
    owners.mkdir(mode=0o700, exist_ok=True)
    if populated:
        for name, target in install.restore_targets(data).items():
            target.parent.mkdir(parents=True, exist_ok=True)
            if name in ("attachments", "exports"):
                target.mkdir()
                target /= "old"
            target.write_bytes(books[1][0][0] if name == install.NAMES[0] else b"old " + name.encode())
    for name in install.INCOMING:
        target = stage / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if name == "attachments":
            target.mkdir()
            target /= "new"
        target.write_bytes(books[1][1][0] if name == install.NAMES[0] else b"new " + name.encode())
    return dict(data_root=str(data), staging=str(stage), owners=str(owners), op="restore-install")


def _run(runtime, request, *, kill=False):
    if runtime == "rust":
        assert PROBE.is_file(), "build kassiber-storage-probe with test-hooks"
    command = [str(PROBE)] if runtime == "rust" else [sys.executable, "-c", PYTHON_PROBE]
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               env={**os.environ, "KASSIBER_NO_EGRESS": "1"})
    try:
        process.stdin.write((json.dumps(request) + "\n").encode())
        process.stdin.flush()
        if kill:
            assert select.select([process.stdout], [], [], 30)[0], f"unreached {runtime} {request}"
            line = process.stdout.readline()
            assert json.loads(line) == {"checkpoint": request['kill_at']}, (line, request)
            process.kill()
            process.communicate(timeout=10)
            assert process.returncode < 0
            return
        out, err = process.communicate(timeout=30)
        assert process.returncode == 0 and not err, (out, err, request)
        return json.loads(out)["data"]
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate(timeout=10)


def _refusal(runtime, request):
    command = [str(PROBE)] if runtime == "rust" else [sys.executable, "-c", PYTHON_PROBE]
    result = subprocess.run(command, input=(json.dumps(request) + "\n").encode(), capture_output=True,
                            env={**os.environ, "KASSIBER_NO_EGRESS": "1"}, timeout=30)
    assert result.returncode == 1 and not result.stderr, (result.stdout, result.stderr)
    error = json.loads(result.stdout)["error"]
    return error["code"], error["message"]


def _symlinked_temp_root(root):
    # macOS stages under /var/folders/...; /var links to /private/var.
    (root / "private-var").mkdir()
    (root / "var").symlink_to("private-var")
    folders = root / "var/folders"
    folders.mkdir()
    return folders


def _assert_files(request, books, populated, committed):
    data = Path(request["data_root"])
    for name, target in install.restore_targets(data).items():
        present = name in install.INCOMING if committed else populated
        if not present:
            assert not target.exists(), name
            continue
        if name in ("attachments", "exports"):
            target /= "new" if committed else "old"
        expected = books[1][int(committed)][0] if name == install.NAMES[0] else (b"new " if committed else b"old ") + name.encode()
        assert target.read_bytes() == expected, name
    if populated and committed:
        recovery, = data.parent.glob("pre-restore-*")
        for name in install.NAMES:
            target = recovery / name
            if name in ("attachments", "exports"):
                target /= "old"
            assert target.read_bytes() == (books[1][0][0] if name == install.NAMES[0] else b"old " + name.encode())
    assert not (data.parent / install.JOURNAL).exists()


@pytest.mark.parametrize("runtime", ["python", "rust"])
@pytest.mark.parametrize("populated", [True, False])
def test_frozen_durable_step_inventory_matches_both_implementations(tmp_path, books, runtime, populated):
    request = _setup(tmp_path, books, populated)
    assert _run(runtime, request)["points"] == _points(populated)
    _assert_files(request, books, populated, True)


@pytest.mark.parametrize("runtime", ["python", "rust"])
@pytest.mark.parametrize("populated", [True, False])
def test_staging_under_a_symlinked_temp_root_installs_in_both_runtimes(tmp_path, books, runtime, populated):
    # tests/rust_port/repro_restore_staging_symlinked_tmpdir.py
    root = tmp_path.resolve()
    request = _setup(root, books, populated)
    stage = _symlinked_temp_root(root) / "kassiber-restore-x"
    Path(request["staging"]).rename(stage)
    request["staging"] = str(stage)
    assert _run(runtime, request)["points"] == _points(populated)
    _assert_files(request, books, populated, True)


@pytest.mark.parametrize("link", ["stage", "stage/kassiber.sqlite3", "stage/attachments/new"])
def test_a_link_at_or_below_the_staging_root_refuses_alike(tmp_path, books, link):
    refusals = []
    for runtime in ("python", "rust"):
        root = (tmp_path / runtime).resolve()
        root.mkdir()
        request = _setup(root, books, True)
        folders = _symlinked_temp_root(root)
        outside = root / "outside"
        outside.write_bytes(b"not staged")
        if link == "stage":
            (folders / "stage").symlink_to(request["staging"])
        else:
            Path(request["staging"]).rename(folders / "stage")
            (folders / link).unlink()
            (folders / link).symlink_to(outside)
        refusals.append(_refusal(runtime, {**request, "staging": str(folders / "stage")}))
        _assert_files(request, books, True, False)
        assert outside.read_bytes() == b"not staged"
        assert not list(Path(request["data_root"]).parent.glob(".restore-install-*"))
    assert refusals[0] == refusals[1] == ("unsafe_restore_target", "Restore destination contains a symbolic link")


@pytest.mark.parametrize("starter", ["python", "rust"])
@pytest.mark.parametrize("restarter", ["python", "rust"])
@pytest.mark.parametrize("populated,point", [(p, point) for p in (True, False) for point in _points(p)])
def test_killed_install_recovers_same_bytes_in_either_runtime(tmp_path, books, starter, restarter, populated, point):
    request = _setup(tmp_path, books, populated)
    _run(starter, {**request, "kill_at": point}, kill=True)
    _run(restarter, {**request, "op": "restore-recover"})
    _run(restarter, {**request, "op": "restore-recover"})
    committed = _points(populated).index(point) >= _points(populated).index("restore.committed.journal_replace")
    _assert_files(request, books, populated, committed)


@pytest.mark.parametrize("starter", ["python", "rust"])
@pytest.mark.parametrize("restarter", ["python", "rust"])
@pytest.mark.parametrize("populated,point", [(p, point) for p in (True, False) for point in _recovery_points(p)])
def test_second_kill_during_rollback_recovers_in_either_runtime(tmp_path, books, starter, restarter, populated, point):
    request = _setup(tmp_path, books, populated)
    _run(starter, {**request, "kill_at": "restore.committed.journal_sync"}, kill=True)
    _run(restarter, {**request, "op": "restore-recover", "kill_at": point}, kill=True)
    _run(starter, {**request, "op": "restore-recover"})
    _assert_files(request, books, populated, False)


@pytest.mark.parametrize("reference", [True, False])
@pytest.mark.parametrize("populated", [True, False])
def test_cli_installs_both_codecs_and_preserves_complete_rollback(cli, tmp_path, books, reference, populated):
    target = tmp_path / "installed/data"
    if populated:
        target.mkdir(parents=True)
        (target / install.NAMES[0]).write_bytes(books[1][0][0])
        (target.parent / "exports").mkdir()
        (target.parent / "exports/old.csv").write_bytes(b"old report")
    archive = tmp_path / "incoming.kassiber"
    _bundle(archive, [(install.NAMES[0], books[1][1][0]), ("attachments/a", b"count only")], _manifest(1))
    result = ok(cli("backup", "import", str(archive), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()),
                    "--install", "--target-data-root", str(target), reference=reference))
    assert result["staging_path"] is None and result["temporary_artifacts_cleaned"] is True
    assert result["installed_data_root"] == str(target)
    conn = open_encrypted(target / install.NAMES[0], KEY, enforce_operator_identity=False) if books[0] == "sqlcipher" else __import__("sqlite3").connect(target / install.NAMES[0])
    try:
        assert normalized_dump(conn) == books[1][1][1]
    finally:
        conn.close()
    assert (target.parent / "attachments/a").read_bytes() == b"count only"
    assert not (target.parent / "exports").exists()
    if populated:
        recovery = Path(result["pre_restore_backup"])
        assert (recovery / install.NAMES[0]).read_bytes() == books[1][0][0]
        assert (recovery / "exports/old.csv").read_bytes() == b"old report"
    else:
        assert result["pre_restore_backup"] is None


@pytest.mark.parametrize("role", ["desktop", "broker"])
def test_cli_owner_refusal_matches_python_before_decryption(cli, tmp_path, monkeypatch, role):
    target = tmp_path / "held/data"
    target.mkdir(parents=True)
    monkeypatch.setattr(project, "_owner_lock_root", lambda: cli.owners)
    lease = project.acquire_project_ownership(project.canonical_project(target), owner_kind=role, generation="held")
    args = ("backup", "import", str(tmp_path / "absent.kassiber"), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()),
            "--install", "--target-data-root", str(target))
    try:
        python, rust = cli(*args, reference=True), cli(*args)
        assert (rust.returncode, rust.stdout, rust.stderr) == (python.returncode, python.stdout, python.stderr)
        assert json.loads(rust.stdout)["error"]["code"] == "project_in_use"
    finally:
        lease.release()


PYTHON_IMPORT_PROBE = r'''
import json, sys
from pathlib import Path
from kassiber.backup import pack
from kassiber.operator import project
from kassiber.secrets.prompt import read_passphrase_from_fd
request = json.loads(sys.stdin.readline())
project._owner_lock_root = lambda: Path(request['owners'])
def observe(point):
    if point == request['kill_at']:
        print(json.dumps({'checkpoint':point}), flush=True)
        sys.stdin.readline()
original_temp = pack.tempfile.mkdtemp
def temporary(*args, **kwargs):
    result = original_temp(*args, **kwargs)
    if kwargs.get('prefix') == 'kassiber-restore-':
        observe('import.workspace')
    return result
pack.tempfile.mkdtemp = temporary
for name, point in [('decrypt_age_stream','import.decrypted'), ('extract_tar_safely','import.extracted'), ('_restore_unavailable_secret_refs','import.validated')]:
    def wrapper(*args, _function=getattr(pack,name), _point=point, **kwargs):
        result = _function(*args, **kwargs)
        observe(_point)
        return result
    setattr(pack,name,wrapper)
with project.exclusive_project_maintenance(request['data_root'], active_owner_kind=None):
    pack.import_backup(Path(request['archive']), Path(request['data_root']),
                       backup_passphrase=read_passphrase_from_fd(request['fd']), move_into_place=True)
'''


@pytest.fixture(scope="module")
def incoming_archive(books, tmp_path_factory):
    archive = tmp_path_factory.mktemp("restore-import") / "book.kassiber"
    _bundle(archive, [(install.NAMES[0], books[1][1][0])], _manifest())
    return archive


@pytest.mark.parametrize("starter", ["python", "rust"])
@pytest.mark.parametrize("restarter", ["python", "rust"])
@pytest.mark.parametrize("populated", [True, False])
@pytest.mark.parametrize("point", ["import.workspace", "import.decrypted", "import.extracted", "import.validated"])
def test_killed_import_never_publishes_before_validation(tmp_path, books, incoming_archive, starter, restarter, populated, point):
    request = _setup(tmp_path, books, populated)
    temporary = tmp_path / "temporary"
    temporary.mkdir()
    request.update(archive=str(incoming_archive), kill_at=point)
    binary = ROOT / "target/debug/examples/kassiber-backup-probe"
    assert starter != "rust" or binary.is_file(), "build kassiber-backup-probe with test-hooks"
    command = [str(binary)] if starter == "rust" else [sys.executable, "-c", PYTHON_IMPORT_PROBE]
    read, write = os.pipe()
    os.write(write, OUTER.encode())
    os.close(write)
    request['fd'] = read
    try:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   pass_fds=[read], env={**os.environ, "TMPDIR": str(temporary), "KASSIBER_NO_EGRESS": "1"})
    finally:
        os.close(read)
    try:
        process.stdin.write((json.dumps(request) + "\n").encode())
        process.stdin.flush()
        assert select.select([process.stdout], [], [], 30)[0], "import checkpoint not reached"
        assert json.loads(process.stdout.readline()) == {'checkpoint':point}
    finally:
        process.kill()
        process.communicate(timeout=10)
    request.pop('fd')
    _run(restarter, {**request, "op":"restore-recover"})
    _assert_files(request, books, populated, False)
    # Pre-intent temporary workspaces are private, unreferenced, disposable
    # input. Neither startup scans nor deletes arbitrary system-temp paths.
    shutil.rmtree(temporary)


@pytest.mark.parametrize("starter", ["python", "rust"])
@pytest.mark.parametrize("reference", [True, False])
@pytest.mark.parametrize("point,committed", [("restore.after_save.kassiber.sqlite3", False),
                                               ("restore.after_install.kassiber.sqlite3", False),
                                               ("restore.after_install.config/settings.json", False),
                                               ("restore.committed.journal_replace", True)])
def test_ordinary_cli_restart_recovers_before_layout_and_database_open(cli, tmp_path, books, starter, reference, point, committed):
    request = _setup(tmp_path, books, True)
    data = Path(request['data_root'])
    # This scenario opens the real database; use valid settings and no
    # synthetic SQLite journals. The byte matrix separately retains all eight.
    for name in install.NAMES[1:4]:
        (data / name).unlink()
    for root in (data.parent / "config", Path(request['staging']) / "config"):
        (root / "settings.json").write_text('{"schema_version":1,"app":"kassiber"}')
        (root / "backends.env").write_text('# no backend probes\n')
    if point == "restore.after_install.config/settings.json":
        (Path(request['staging']) / "config/settings.json").write_text('invalid partial incoming settings')
    # Both fixture namespaces must agree with the ordinary CLI restart.
    request['owners'] = str(cli.owners)
    _run(starter, {**request, "kill_at":point}, kill=True)
    args = (["--db-passphrase-fd", cli_runner.InputFd(KEY.encode())] if books[0] == "sqlcipher" else []) + ['status']
    result = cli(*args, reference=reference, root=data)
    payload = json.loads(result.stdout)
    assert result.returncode == 0 or payload['error']['code'] == 'not_initialized', (result.stdout,result.stderr)
    assert not (data.parent / install.JOURNAL).exists()
    conn = open_encrypted(data / install.NAMES[0], KEY, enforce_operator_identity=False) if books[0] == "sqlcipher" else __import__('sqlite3').connect(data / install.NAMES[0])
    try:
        assert conn.execute("SELECT value FROM settings WHERE key='restore_marker'").fetchone()[0] == ('new' if committed else 'old')
    finally:
        conn.close()


@pytest.mark.parametrize("populated", [True, False])
def test_install_success_wire_bytes_use_only_declared_recovery_path_mapping(cli, tmp_path, books, populated):
    target = tmp_path / "wire/data"
    archive = tmp_path / "wire.kassiber"
    _bundle(archive, [(install.NAMES[0], books[1][1][0])], _manifest())
    responses = []
    for reference in (True, False):
        if target.parent.exists():
            shutil.rmtree(target.parent)
        if populated:
            target.mkdir(parents=True)
            (target / install.NAMES[0]).write_bytes(books[1][0][0])
        result = cli("backup", "import", str(archive), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()),
                     "--install", "--target-data-root", str(target), reference=reference)
        payload = ok(result)
        raw = result.stdout
        if populated:
            recovery = Path(payload['pre_restore_backup'])
            assert recovery.parent == target.parent and recovery.name.startswith('pre-restore-')
            assert (recovery / install.NAMES[0]).read_bytes() == books[1][0][0]
            raw = raw.replace(str(recovery).encode(), b'<retained-copy>')
        responses.append((result.returncode, raw, result.stderr))
    assert responses[0] == responses[1]


@pytest.mark.parametrize("populated", [True, False])
def test_cli_restore_staged_under_a_symlinked_tmpdir_matches_wire_bytes(cli, tmp_path, books, populated):
    # backup import stages under $TMPDIR, which macOS reaches through /var.
    temporary = _symlinked_temp_root(tmp_path.resolve())
    target = tmp_path / "linked/data"
    archive = tmp_path / "linked.kassiber"
    _bundle(archive, [(install.NAMES[0], books[1][1][0]), ("attachments/a", b"count only")], _manifest(1))
    responses = []
    for reference in (True, False):
        if target.parent.exists():
            shutil.rmtree(target.parent)
        if populated:
            target.mkdir(parents=True)
            (target / install.NAMES[0]).write_bytes(books[1][0][0])
        before = sorted(temporary.iterdir())
        cli.env["TMPDIR"] = str(temporary)
        try:
            result = cli("backup", "import", str(archive), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()),
                         "--install", "--target-data-root", str(target), reference=reference)
        finally:
            del cli.env["TMPDIR"]
        payload = ok(result)
        assert payload["installed_data_root"] == str(target)
        assert sorted(temporary.iterdir()) == before
        conn = open_encrypted(target / install.NAMES[0], KEY, enforce_operator_identity=False) if books[0] == "sqlcipher" else __import__("sqlite3").connect(target / install.NAMES[0])
        try:
            dump = normalized_dump(conn)
        finally:
            conn.close()
        assert dump == books[1][1][1]
        assert (target.parent / "attachments/a").read_bytes() == b"count only"
        raw = result.stdout
        if populated:
            raw = raw.replace(str(Path(payload["pre_restore_backup"])).encode(), b"<retained-copy>")
        responses.append((result.returncode, raw, result.stderr, dump))
    assert responses[0] == responses[1]


@pytest.mark.parametrize("role", ["broker", "desktop"])
def test_startup_recovery_refuses_owner_before_any_layout_write(cli, tmp_path, books, monkeypatch, role):
    request = _setup(tmp_path, books, True)
    data = Path(request['data_root'])
    request['owners'] = str(cli.owners)
    _run('rust', {**request, 'kill_at':'restore.after_install.kassiber.sqlite3'}, kill=True)
    monkeypatch.setattr(project, '_owner_lock_root', lambda:cli.owners)
    lease = project.acquire_project_ownership(project.canonical_project(data), owner_kind=role, generation='held-recovery')
    marker = data.parent / install.JOURNAL
    before = marker.read_bytes(), (data / install.NAMES[0]).read_bytes()
    try:
        python, rust = cli('status', root=data, reference=True), cli('status', root=data)
        assert (rust.returncode, rust.stdout, rust.stderr) == (python.returncode, python.stdout, python.stderr)
        assert json.loads(rust.stdout)['error']['code'] == 'project_in_use'
        assert (marker.read_bytes(), (data / install.NAMES[0]).read_bytes()) == before
    finally:
        lease.release()
    _run('python', {**request, 'op':'restore-recover'})
    _assert_files(request, books, True, False)
