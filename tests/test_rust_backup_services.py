"""Native backup format interop; disposable data, no live credentials/egress.

Desktop restore remains gated pending its session and lifecycle coverage.
"""
from io import BytesIO
import json
import os
from pathlib import Path
import shutil
import sqlite3
import tarfile
import sys

import pyrage.passphrase
import pytest

from kassiber.backup.pack import import_backup
from kassiber.db import open_db
from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.test_rust_secrets_services import KEY, cli, ok, same  # noqa: F401 (fixture)
from tests.test_rust_store_migrations import normalized_dump

OUTER = "Synthetic outer backup passphrase"
pytestmark = pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="set KASSIBER_RUST_BIN to the harness binary")


def _manifest(count=0):
    return {"schema_version": 1, "entries": {"database": "kassiber.sqlite3", "attachments_files": count,
            "backends_env": False, "settings_json": False}, "secret_refs": {"ai_provider_refs": []}}


def _bundle(path, members, manifest):
    buffer = BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as tar:
        for member, content in [*members, ("manifest.json", json.dumps(manifest).encode())]:
            if isinstance(member, str):
                info = tarfile.TarInfo(member)
                info.size = len(content)
            else:
                info = member
            tar.addfile(info, BytesIO(content))
    path.write_bytes(pyrage.passphrase.encrypt(buffer.getvalue(), OUTER))


def _import(cli, path, **kwargs):
    return cli("backup", "import", str(path), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()), **kwargs)


def _source(cli):
    ok(cli("init", reference=True))
    conn = open_db(str(cli.data))
    conn.execute("INSERT INTO settings(key,value) VALUES('backup_exact','9223372036854775807 / ₿ / nonsecret')")
    conn.commit()
    conn.close()
    ok(cli("secrets", "init", "--new-passphrase-fd", cli_runner.InputFd(KEY.encode()), reference=True))
    state = cli.data.parent
    evidence = state / "attachments/nested/₿.bin"
    evidence.parent.mkdir(parents=True)
    evidence.write_bytes(b"exact attachment\x00\xff")
    (state / "config/backends.env").write_text("# literal local bootstrap\n")
    (state / "exports").mkdir(exist_ok=True)
    (state / "exports/excluded.txt").write_text("excluded export")
    return state


@pytest.mark.parametrize("rust_makes", [True, False])
def test_sqlcipher_round_trips_both_directions_preserve_every_object_and_sidecar(cli, tmp_path, rust_makes):
    state = _source(cli)
    archive = tmp_path / "round-trip.kassiber"
    result = ok(cli("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "backup", "export", "--file", str(archive),
                    "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()), reference=not rust_makes))
    assert result["age_backend"] == "pyrage"
    assert result["size_bytes"] == archive.stat().st_size
    assert archive.read_bytes().startswith(b"age-encryption.org/v1\n")
    if rust_makes:
        imported = import_backup(archive, tmp_path / "unused", backup_passphrase=OUTER)
        staging, manifest = imported.staging_path, imported.manifest
    else:
        response = ok(_import(cli, archive))
        assert response["installed_data_root"] is None
        assert response["temporary_artifacts_cleaned"] is False
        assert response["pre_restore_backup"] is None
        staging, manifest = Path(response["staging_path"]), response["manifest"]
    try:
        assert manifest == result["manifest"]
        original = open_encrypted(cli.data / "kassiber.sqlite3", KEY, enforce_operator_identity=False)
        restored = open_encrypted(staging / "kassiber.sqlite3", KEY, enforce_operator_identity=False)
        try:
            assert normalized_dump(restored) == normalized_dump(original)
        finally:
            original.close()
            restored.close()
        assert (staging / "attachments/nested/₿.bin").read_bytes() == b"exact attachment\x00\xff"
        for name in ("backends.env", "settings.json"):
            assert (staging / "config" / name).read_bytes() == (state / "config" / name).read_bytes()
        assert manifest["entries"]["attachments_files"] == 1
        assert manifest["notes"]["inner_db_encrypted"] is True
        assert not (staging / "exports").exists()
    finally:
        shutil.rmtree(staging.parent)


def test_export_manifest_matches_python_without_attachment_hashes(cli, tmp_path):
    _source(cli)
    manifests = []
    for reference in (True, False):
        archive = tmp_path / f"manifest-{reference}.kassiber"
        result = ok(cli("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "backup", "export", "--file", str(archive),
                        "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()), reference=reference))
        manifest = result["manifest"]
        # The creation clock is the only nondeterministic manifest field.
        assert manifest.pop("created_at").endswith("Z")
        manifests.append(manifest)
    assert manifests[0] == manifests[1]


def test_plaintext_export_refusal_matches_reference_bytes(cli, tmp_path):
    ok(cli("init", reference=True))
    archive = tmp_path / "retained.kassiber"
    archive.write_bytes(b"existing ciphertext")
    result = same(cli, "backup", "export", "--file", str(archive), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()))
    assert result.returncode == 1
    error = json.loads(result.stdout)["error"]
    assert error["code"] == "plaintext_database"
    assert error["hint"] == "Run `kassiber secrets init` first."
    assert archive.read_bytes() == b"existing ciphertext"


def test_import_accepts_plaintext_payload_and_count_only_evidence(cli, tmp_path):
    database = tmp_path / "plain.sqlite3"
    conn = sqlite3.connect(database)
    conn.execute("CREATE TABLE exact(value)")
    conn.execute("INSERT INTO exact VALUES(?)", (9223372036854775807,))
    conn.commit()
    expected = normalized_dump(conn)
    conn.close()
    archive = tmp_path / "plain.kassiber"
    manifest = _manifest(1)
    _bundle(archive, [("kassiber.sqlite3", database.read_bytes()), ("attachments/a", b"count only")], manifest)
    python = import_backup(archive, tmp_path / "unused", backup_passphrase=OUTER)
    rust = ok(_import(cli, archive))
    try:
        assert rust["manifest"] == python.manifest == manifest
        conn = sqlite3.connect(Path(rust["staging_path"]) / "kassiber.sqlite3")
        assert normalized_dump(conn) == expected
        conn.close()
    finally:
        shutil.rmtree(python.staging_path.parent)
        shutil.rmtree(Path(rust["staging_path"]).parent)


@pytest.mark.parametrize("case", ["absolute", "traversal", "backslash", "duplicate", "symlink", "hardlink", "device", "fifo", "socket", "unexpected-top", "surrogate-top"])
def test_archive_safety_refusals_match_reference_bytes(cli, tmp_path, case):
    archive = tmp_path / "unsafe.kassiber"
    if case in {"absolute", "traversal", "backslash", "unexpected-top", "surrogate-top"}:
        name = {"absolute": "/attachments/a", "traversal": "attachments/../a", "backslash": "attachments\\a", "unexpected-top": "exports/a", "surrogate-top": "exports\udcff/a"}[case]
        members = [(name, b"unsafe")]
    elif case == "duplicate":
        members = [("attachments/a", b"first"), ("./attachments//a", b"second")]
    else:
        info = tarfile.TarInfo("attachments/a")
        info.type = {"symlink": tarfile.SYMTYPE, "hardlink": tarfile.LNKTYPE, "device": tarfile.CHRTYPE,
                     "fifo": tarfile.FIFOTYPE, "socket": b"s"}[case]
        info.linkname = "../../outside"
        members = [(info, b"")]
    _bundle(archive, members, _manifest())
    result = same(cli, "backup", "import", str(archive), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()))
    assert json.loads(result.stdout)["error"]["code"] == "unsafe_backup_member"


@pytest.mark.parametrize("case", ["schema", "database", "missing-db", "count", "bool-count", "backends", "settings"])
def test_manifest_refusals_match_reference_bytes(cli, tmp_path, case):
    manifest = _manifest()
    members = [("kassiber.sqlite3", b"database")]
    if case == "schema":
        manifest["schema_version"] = 2
    elif case == "database":
        manifest["entries"]["database"] = "config/elsewhere"
    elif case == "missing-db":
        members = []
    elif case in {"count", "bool-count"}:
        manifest["entries"]["attachments_files"] = 1 if case == "count" else False
    else:
        manifest["entries"][{"backends": "backends_env", "settings": "settings_json"}[case]] = True
    archive = tmp_path / "invalid.kassiber"
    _bundle(archive, members, manifest)
    result = same(cli, "backup", "import", str(archive), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()))
    assert json.loads(result.stdout)["error"]["code"] == "invalid_backup"


def test_passphrase_and_missing_archive_errors_match_reference_bytes(cli, tmp_path):
    same(cli, "backup", "import", str(tmp_path / "missing.kassiber"))
    same(cli, "backup", "import", str(tmp_path / "missing.kassiber"), "--backup-passphrase-fd", cli_runner.InputFd(OUTER.encode()))
    archive = tmp_path / "valid.kassiber"
    _bundle(archive, [("kassiber.sqlite3", b"db")], _manifest())
    result = same(cli, "backup", "import", str(archive), "--backup-passphrase-fd", cli_runner.InputFd(b"wrong outer"))
    assert json.loads(result.stdout)["error"]["code"] == "age_decrypt_failed"


def test_install_requires_outer_secret_before_target_changes(cli, tmp_path):
    target = tmp_path / "untouched/data"
    result = cli("backup", "import", str(tmp_path / "missing.kassiber"), "--install", "--target-data-root", str(target))
    assert json.loads(result.stdout)["error"]["code"] == "interaction_required"
    assert not target.parent.exists()


@pytest.mark.skipif(os.name != "posix", reason="surrogateescape filesystem acceptance is Unix-specific")
def test_tar_names_preserve_surrogateescape_and_extended_paths(cli, tmp_path):
    names = ["attachments/raw-\udcff", "attachments/raw-\udcfe", "attachments/" + "long-₿/" * 40 + "file"]
    archive = tmp_path / "names.kassiber"
    _bundle(archive, [("kassiber.sqlite3", b"db"), *[(name, name.encode("utf-8", "surrogateescape")) for name in names]], _manifest(len(names)))
    python = import_backup(archive, tmp_path / "unused", backup_passphrase=OUTER)
    rust = ok(_import(cli, archive))
    try:
        for name in names:
            assert (Path(rust["staging_path"]) / name).read_bytes() == (python.staging_path / name).read_bytes()
    finally:
        shutil.rmtree(python.staging_path.parent)
        shutil.rmtree(Path(rust["staging_path"]).parent)


@pytest.mark.skipif(os.name != "posix", reason="Unix executable fixture; native Windows PATH acceptance remains open")
def test_recipient_child_discovery_invocation_and_format_match_python(cli, tmp_path):
    import pyrage.x25519
    _source(cli)
    identity = pyrage.x25519.Identity.generate()
    recipient = str(identity.to_public())
    identity_path = tmp_path / "identity.txt"
    identity_path.write_text(str(identity))
    first, second = tmp_path / "first-bin", tmp_path / "second-bin"
    first.mkdir()
    second.mkdir()
    log = tmp_path / "child.jsonl"
    # A reviewed local test child implements the age CLI's file streams using
    # pyrage. Deliberately put rage earlier in PATH: age must still win.
    script = f"#!{sys.executable}\n" + """
import json, sys
from pathlib import Path
import pyrage
from pyrage import x25519
args = sys.argv[1:]
with Path(LOG).open('a') as handle:
    handle.write(json.dumps([Path(sys.argv[0]).name, args]) + '\\n')
payload = sys.stdin.buffer.read()
if args[0] == '--decrypt':
    assert args[1] == '--identity' and len(args) == 3
    result = pyrage.decrypt(payload, [x25519.Identity.from_str(Path(args[2]).read_text().strip())])
else:
    assert all(args[i] == '--recipient' for i in range(0, len(args), 2))
    result = pyrage.encrypt(payload, [x25519.Recipient.from_str(args[i]) for i in range(1, len(args), 2)])
sys.stdout.buffer.write(result)
""".replace("LOG", repr(str(log)))
    for directory, name in [(first, "rage"), (second, "age")]:
        binary = directory / name
        binary.write_text(script)
        binary.chmod(0o700)
    cli.env["PATH"] = os.pathsep.join([str(first), str(second)])
    for reference in (True, False):
        archive = tmp_path / f"recipient-{reference}.kassiber"
        exported = ok(cli("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "backup", "export", "--file", str(archive),
                          "--recipient", recipient, "--recipient", recipient, reference=reference))
        assert exported["age_backend"] == "age"
        imported = ok(cli("backup", "import", str(archive), "--identity-file", str(identity_path), reference=not reference))
        try:
            assert imported["manifest"] == exported["manifest"]
        finally:
            shutil.rmtree(Path(imported["staging_path"]).parent)
    entries = [json.loads(line) for line in log.read_text().splitlines()]
    assert entries == [["age", args] for args in [
        ["--recipient", recipient, "--recipient", recipient], ["--decrypt", "--identity", str(identity_path)],
        ["--recipient", recipient, "--recipient", recipient], ["--decrypt", "--identity", str(identity_path)],
    ]]
