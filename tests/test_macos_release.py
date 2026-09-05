"""Release guards run without Apple credentials or signing a real artifact."""
import argparse
import importlib.util
import json
import plistlib
import stat
import subprocess
import sys
import tarfile
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("macos_release", ROOT / "scripts/macos_release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
sys.modules["macos_release"] = release
prepare_spec = importlib.util.spec_from_file_location("prepare_macos_release", ROOT / "scripts/prepare_macos_release.py")
prepare = importlib.util.module_from_spec(prepare_spec)
prepare_spec.loader.exec_module(prepare)


@pytest.mark.parametrize("name,mode", [
    ("../outside", stat.S_IFREG), ("/outside", stat.S_IFREG),
    ("Kassiber.app/../../outside", stat.S_IFREG),
    ("Kassiber.app/link", stat.S_IFLNK), ("Kassiber.app/socket", stat.S_IFSOCK),
    ("Other.app/file", stat.S_IFREG), ("Kassiber.app\\evil", stat.S_IFREG),
])
def test_reject_archive_before_platform_extraction(tmp_path, name, mode):
    archive = tmp_path / "input.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        member = zipfile.ZipInfo(name)
        member.external_attr = mode << 16
        zf.writestr(member, "bad")
    with patch.object(release, "run") as run:
        with pytest.raises(ValueError):
            release.extract_app(archive, tmp_path / "out")
        run.assert_not_called()


def test_digest_fails_closed(tmp_path):
    artifact = tmp_path / "file"
    artifact.write_bytes(b"release")
    release.check_digest(artifact, release.sha256(artifact))
    with pytest.raises(ValueError):
        release.check_digest(artifact, "0" * 64)


def test_signing_never_runs_in_ci(tmp_path, monkeypatch):
    monkeypatch.setenv("CI", "true")
    with patch.object(release, "run") as run:
        with pytest.raises(ValueError, match="locally"):
            release.sign(argparse.Namespace())
        run.assert_not_called()


def test_embedded_build_identity(tmp_path):
    app = tmp_path / "Kassiber.app"
    contents = app / "Contents"
    contents.mkdir(parents=True)
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleIdentifier": release.APP_ID,
                      "CFBundleShortVersionString": "1.2.3"}, stream)
    metadata = contents / "BUILD_INFO.json"
    metadata.write_text(json.dumps({"commit": "a" * 40, "version": "1.2.3", "channel": "release"}))
    release.validate_app(app, "a" * 40, "1.2.3")
    with pytest.raises(ValueError):
        release.validate_app(app, "b" * 40, "1.2.3")
    with pytest.raises(ValueError):
        release.validate_app(app, "a" * 12, "1.2.3")
    metadata.unlink()
    with pytest.raises(ValueError, match="provenance"):
        release.validate_app(app, "a" * 40, "1.2.3")


def test_macho_inventory_rejects_symlinks(tmp_path):
    (tmp_path / "lib").write_bytes(bytes.fromhex("cffaedfe") + b"code")
    (tmp_path / "data").write_bytes(b"data")
    assert release.code_files(tmp_path) == [tmp_path / "lib"]
    (tmp_path / "link").symlink_to("lib")
    with pytest.raises(ValueError):
        release.code_files(tmp_path)


@pytest.mark.parametrize("field,value", [
    ("conclusion", "failure"), ("event", "pull_request"),
    ("head_sha", "b" * 40), ("path", "attacker.yml"),
    ("head_repository", {"full_name": "attacker/kassiber"}),
])
def test_untrusted_build_never_reaches_signer(tmp_path, field, value):
    build = {"conclusion": "success", "event": "push", "head_sha": "a" * 40,
             "path": ".github/workflows/prerelease-binaries.yml",
             "head_repository": {"full_name": prepare.REPO}}
    build[field] = value
    with patch.object(prepare, "run", side_effect=[
        json.dumps({"isDraft": True, "assets": []}), json.dumps(build),
        json.dumps({"sha": "a" * 40}),
    ]), patch.object(prepare, "sign") as sign:
        with pytest.raises(ValueError):
            prepare.prepare(argparse.Namespace(tag="v1.2.3", run_id="1", work_dir=tmp_path / "work"))
        sign.assert_not_called()
        assert not (tmp_path / "work").exists()


def test_workflows_keep_keys_local_and_publication_gated():
    workflows = ROOT / ".github/workflows"
    notary = (workflows / "notarize-macos.yml").read_text()
    final = (workflows / "finalize-signed-release.yml").read_text()
    build = yaml.safe_load((workflows / "prerelease-binaries.yml").read_text())
    publish_steps = build["jobs"]["publish"]["steps"]
    assert next(s for s in publish_steps if s.get("name") == "Create or update release")["with"]["draft"] is True
    assert "scripts/macos_release.py verify" in final
    assert 'case "$actual_draft" in true|false)' in final
    assert "if: steps.release.outputs.draft == 'true'" in final
    assert "--smoke" in final
    assert final.index("Require unchanged release assets and tag") < final.index("Publish the verified draft")
    assert final.index("Publish the verified draft") < final.index("Commit Homebrew tap update")
    assert final.index("Authenticate the complete release set") < final.index("Verify actual macOS release bytes")
    assert "APPLE_CERTIFICATE" not in notary
    assert "NOTARY_KEY_P8" in notary
    assert "--keychain" in notary
    assert "--identity" not in notary
    assert "release-seal-${{ inputs.tag_name }}" in notary and "release-seal-${{ inputs.tag_name }}" in final


@pytest.mark.skipif(sys.platform != "darwin", reason="Apple tool contract")
def test_real_codesign_rejects_non_bitcoin_austria_identity():
    # Apple's system binary is validly signed, but not by our Developer ID.
    # This exercises the actual platform requirement without any private key.
    with pytest.raises(subprocess.CalledProcessError):
        release.verify_code(Path("/usr/bin/true"))


@pytest.mark.parametrize("mutation", [None, "bytes", "mode", "launcher", "missing", "extra"])
def test_cli_distribution_is_exact_sealed_app(tmp_path, mutation):
    root = tmp_path / "kassiber-cli-macos-arm64"
    app = root / "Kassiber.app"
    contents = app / "Contents/Resources/bin"
    contents.mkdir(parents=True)
    binary = contents / "kassiber"
    binary.write_bytes(b"sealed launcher")
    binary.chmod(0o755)
    (root / "kassiber").symlink_to("Kassiber.app/Contents/Resources/bin/kassiber")
    if mutation == "launcher":
        (root / "kassiber").unlink()
        (root / "kassiber").symlink_to("/etc/passwd")
    if mutation == "missing":
        (root / "kassiber").unlink()
    if mutation == "extra":
        (root / "extra").write_text("unexpected")
    archive = tmp_path / "cli.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(root, arcname=root.name)
    # Change the comparison side after creating the archive.
    if mutation == "bytes":
        binary.write_bytes(b"forged launcher")
    if mutation == "mode":
        binary.chmod(0o644)
    if mutation is None:
        release.verify_cli_archive(archive, app)
    else:
        with pytest.raises(ValueError):
            release.verify_cli_archive(archive, app)


@pytest.mark.parametrize("mutation", [None, "team", "expiry", "development", "app", "group"])
def test_developer_id_profile_contract(mutation):
    data = {"TeamIdentifier": [release.TEAM], "ProvisionsAllDevices": True,
            "ExpirationDate": datetime.now(timezone.utc) + timedelta(days=10),
            "Entitlements": dict(release.APP_ENTITLEMENTS)}
    if mutation == "team":
        data["TeamIdentifier"] = ["OTHERTEAM1"]
    if mutation == "expiry":
        data["ExpirationDate"] = datetime.now(timezone.utc) - timedelta(days=1)
    if mutation == "development":
        data["ProvisionsAllDevices"] = False
    if mutation == "app":
        data["Entitlements"]["com.apple.application-identifier"] = release.TEAM + ".other"
    if mutation == "group":
        data["Entitlements"]["keychain-access-groups"] = [release.TEAM + ".other"]
    if mutation is None:
        release.validate_profile_data(data)
    else:
        with pytest.raises(ValueError):
            release.validate_profile_data(data)


def test_entitlements_are_explicit_not_inherited():
    with patch.object(release, "run", return_value=plistlib.dumps(release.APP_ENTITLEMENTS).decode()):
        release.verify_entitlements(Path("app"), release.APP_ENTITLEMENTS)
        with pytest.raises(ValueError):
            release.verify_entitlements(Path("sidecar"), {})
    for unexpected in ({}, {**release.APP_ENTITLEMENTS, "com.apple.security.get-task-allow": True}):
        with patch.object(release, "run", return_value=plistlib.dumps(unexpected).decode()):
            with pytest.raises(ValueError):
                release.verify_entitlements(Path("app"), release.APP_ENTITLEMENTS)
