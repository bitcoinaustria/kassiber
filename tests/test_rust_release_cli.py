"""`update` and `verify-download`: native vs Python CLI bytes and files.

No case here may contact GitHub: consent, offline mode or the disabling
environment stop every update check first, and the strict test socket guard
is armed for both runtimes. Release signatures use throwaway keys only.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sys
from unittest import mock

import pytest

from tests import cli_runner
from tests.test_release_verification import MANIFEST_NAME, RELEASE_FIXTURES, _generate_key, _ssh_sign
from tests.test_rust_project_desktop import BINARY, REFERENCE, environment

pytestmark = pytest.mark.skipif(not BINARY, reason="set KASSIBER_RUST_BIN to the test-harness binary")


@pytest.fixture(autouse=True)
def _no_ssh_agent(monkeypatch):
    monkeypatch.delenv("SSH_AUTH_SOCK", raising=False)


def run(tmp_path, argv, *, reference, extra=None):
    owners, env = environment(tmp_path)
    env = {key: value for key, value in env.items() if key != "SSH_AUTH_SOCK"}
    env.pop("KASSIBER_DISABLE_UPDATE_CHECK", None)
    env |= extra or {}
    original = cli_runner.prefix

    def prefix(*, reference=False, python_options=()):
        if reference:
            return (sys.executable, "-c", REFERENCE, str(owners))
        return original(reference=reference, python_options=python_options)

    with mock.patch.object(cli_runner, "prefix", side_effect=prefix), \
            mock.patch.dict(os.environ, {cli_runner.CLI_ENV: env[cli_runner.CLI_ENV]}):
        result = cli_runner.run(list(argv), reference=reference, env=env, capture_output=True, timeout=300)
    return result.returncode, result.stdout, result.stderr


def files(tmp_path):
    root = tmp_path / "prefs"
    return {p.name: p.read_bytes() for p in sorted(root.glob("*"))} if root.exists() else {}


def compare(tmp_path, cases):
    """Run every case in Python, reset preferences, then in Rust."""
    prefs = tmp_path / "prefs"
    expected = []
    for argv, extra, setup in cases:
        shutil.rmtree(prefs, ignore_errors=True)
        prefs.mkdir()
        setup(prefs)
        expected.append((run(tmp_path, argv, reference=True, extra=extra), files(tmp_path)))
    actual = []
    for argv, extra, setup in cases:
        shutil.rmtree(prefs, ignore_errors=True)
        prefs.mkdir()
        setup(prefs)
        actual.append((run(tmp_path, argv, reference=False, extra=extra), files(tmp_path)))
    for case, (want, got) in enumerate(zip(expected, actual)):
        assert got == want, (cases[case][0], got, want)
    records = [json.loads(line) for line in (tmp_path / "served.jsonl").read_text().splitlines()]
    assert records and all(r["served_by"] == "rust" for r in records), records
    if witness := os.environ.get("KASSIBER_RELEASE_COUNTS"):
        with Path(witness).open("a") as handle:
            for record in records:
                handle.write(json.dumps(record) + "\n")
    return actual


def nothing(_prefs):
    return None


def write(name, text):
    def setup(prefs):
        (prefs / name).write_text(text)
        (prefs / name).chmod(0o600)
    return setup


@pytest.mark.parametrize("fmt", ["json", "table", "plain"])
def test_update_preferences_and_refusals_match_python_without_contacting_github(tmp_path, fmt):
    prefs = tmp_path / "prefs"
    paths = {
        "KASSIBER_UPDATE_PREFERENCE_FILE": str(prefs / "update-checks.json"),
        "KASSIBER_UPDATE_CACHE_FILE": str(prefs / "update-check.json"),
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(prefs / "offline.json"),
    }
    enabled = write("update-checks.json", '{"enabled": true, "schema_version": 1}\n')
    offline = write("offline.json", '{"enabled": true, "schema_version": 1}\n')

    def enabled_offline(prefs):
        enabled(prefs)
        offline(prefs)

    base = ["--format", fmt, "update"]
    cases = [
        ([*base, "--status"], paths, nothing),
        ([*base, "--disable-checks"], paths, enabled),
        ([*base, "--status"], paths, enabled),
        ([*base, "--status"], paths | {"KASSIBER_DISABLE_UPDATE_CHECK": " Yes "}, enabled),
        ([*base, "--status"], paths, write("update-checks.json", '{"enabled": true, "schema_version": true}')),
        ([*base, "--status"], paths, write("update-checks.json", '{"enabled": "yes", "schema_version": 1}')),
        ([*base, "--status"], paths, write("update-checks.json", "not json")),
        ([*base, "--status"], paths, write("update-checks.json", "x" * 2000)),
        (base, paths, nothing),
        (base, paths, enabled_offline),
        ([*base, "--enable-checks"], paths, offline),
        (base, paths, write("offline.json", "broken")),
        ([*base, "--status", "--disable-checks"], paths, nothing),
        (["--machine", "update", "--status"], paths, nothing),
    ]
    results = compare(tmp_path, cases)
    assert b"update checks are disabled" in results[8][0][1] + results[8][0][2]
    assert results[10][1]["update-checks.json"] == b'{"enabled": true, "schema_version": 1}\n'
    assert "update-check.json" not in results[10][1]


@pytest.fixture(scope="module")
def release(tmp_path_factory):
    root = tmp_path_factory.mktemp("release-oracle")
    keys = _generate_key(root, "kassiber-release-test")
    other = _generate_key(root, "unrelated-key")
    directory = root / "release"
    directory.mkdir()
    for name in ("kassiber-cli-linux-x64.tar.gz", "kassiber-macos-arm64.dmg", MANIFEST_NAME):
        shutil.copy2(RELEASE_FIXTURES / name, directory / name)
    manifest = directory / MANIFEST_NAME
    manifest_bytes = manifest.read_bytes()
    (directory / f"{MANIFEST_NAME}.sig").write_bytes(_ssh_sign(Path(keys["private_key"]), manifest_bytes))
    (directory / "other.sig").write_bytes(_ssh_sign(Path(other["private_key"]), manifest_bytes))
    (directory / "namespace.sig").write_bytes(_ssh_sign(Path(keys["private_key"]), manifest_bytes, namespace="other"))
    (directory / "garbage.sig").write_bytes(b"not a signature\n")
    renamed = directory / "renamed"
    renamed.mkdir()
    shutil.copy2(manifest, renamed / "kassiber-9.9.9-manifest.txt")
    shutil.copy2(manifest, renamed / "release-manifest.txt")
    (directory / "tampered.tar.gz").write_bytes(b"tampered")
    tampered = directory / "tampered"
    tampered.mkdir()
    (tampered / "kassiber-cli-linux-x64.tar.gz").write_bytes(b"tampered")
    pub = Path(keys["public_key"])
    (directory / "two.pub").write_text(pub.read_text() * 2)
    (directory / "rsa.pub").write_text("ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== x\n")
    (directory / "manifest-link.txt").symlink_to(manifest)
    return {"dir": directory, "pub": pub, "fp": keys["fingerprint"], "other": other}


def test_verify_download_matches_python_for_success_and_every_refusal(tmp_path, release):
    d, pub, fp = release["dir"], str(release["pub"]), release["fp"]
    artifact = str(d / "kassiber-cli-linux-x64.tar.gz")
    manifest = str(d / MANIFEST_NAME)
    sig = str(d / f"{MANIFEST_NAME}.sig")

    def verify(*, artifact=artifact, manifest=manifest, sig=sig, key=pub, fingerprint=fp, fmt="json"):
        return ["--format", fmt, "verify-download", artifact, "--manifest", manifest, "--signature", sig,
                "--public-key", key, "--fingerprint", fingerprint]

    no_ssh = {"PATH": str(tmp_path / "empty-bin")}
    (tmp_path / "empty-bin").mkdir()
    cases = [
        (verify(), None, nothing),
        (verify(fmt="table"), None, nothing),
        (verify(fmt="plain"), None, nothing),
        (["--machine", *verify()[2:]], None, nothing),
        (verify(fingerprint=f"  {fp}\n"), None, nothing),
        (verify(fingerprint="SHA256:short"), None, nothing),
        (verify(fingerprint=release["other"]["fingerprint"]), None, nothing),
        (verify(sig=str(d / "other.sig")), None, nothing),
        (verify(sig=str(d / "namespace.sig")), None, nothing),
        (verify(sig=str(d / "garbage.sig")), None, nothing),
        (verify(key=str(d / "two.pub")), None, nothing),
        (verify(key=str(d / "rsa.pub")), None, nothing),
        (verify(key=str(d / "missing.pub")), None, nothing),
        (verify(manifest=str(d / "manifest-link.txt")), None, nothing),
        (verify(manifest=str(d)), None, nothing),
        (verify(manifest=str(d / "renamed/kassiber-9.9.9-manifest.txt")), None, nothing),
        (verify(manifest=str(d / "renamed/release-manifest.txt")), None, nothing),
        (verify(artifact=str(d / "tampered.tar.gz")), None, nothing),
        (verify(artifact=str(d / "tampered/kassiber-cli-linux-x64.tar.gz")), None, nothing),
        (verify(artifact=str(d / "kassiber-macos-arm64.dmg")), None, nothing),
        (verify(), no_ssh, nothing),
    ]
    results = compare(tmp_path, cases)
    assert json.loads(results[0][0][1])["data"]["verified"] is True
