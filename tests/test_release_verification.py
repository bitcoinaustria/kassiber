from __future__ import annotations

import contextlib
import io
import json
import shutil
import subprocess
from pathlib import Path
from unittest import mock

import pytest

from kassiber.cli.main import main
from kassiber.errors import AppError
from kassiber.release_verification import (
    generate_release_manifest,
    load_release_signing_policy,
    normalize_fingerprint,
    parse_release_manifest,
    signature_status_has_failure,
    verify_download,
    verify_openpgp_signature,
    verify_release_artifacts,
    verify_release_directory,
)
from scripts import release_manifest as release_manifest_script


ROOT = Path(__file__).resolve().parent.parent
RELEASE_FIXTURES = ROOT / "tests" / "fixtures" / "release_signing"
TEST_RELEASE_FINGERPRINT = "51A005054F536D083E3DB9877034D4B74447F840"


@pytest.fixture(scope="module")
def signed_release() -> dict[str, object]:
    if shutil.which("gpg") is None or shutil.which("gpgv") is None:
        pytest.skip("GnuPG and gpgv are required for release verification integration tests")
    return {
        "artifact": RELEASE_FIXTURES / "kassiber-cli-linux-x64.tar.gz",
        "manifest": RELEASE_FIXTURES / "kassiber-9.8.7-rc.1-manifest.txt",
        "signature": RELEASE_FIXTURES / "kassiber-9.8.7-rc.1-manifest.txt.asc",
        "public_key": RELEASE_FIXTURES / "kassiber-release-test.asc",
        "fingerprint": TEST_RELEASE_FINGERPRINT,
    }


def test_manifest_is_stable_sorted_and_does_not_hash_itself(tmp_path: Path) -> None:
    (tmp_path / "kassiber-cli-linux-x64.tar.gz").write_bytes(b"cli")
    (tmp_path / "kassiber-macos-arm64.dmg").write_bytes(b"desktop")
    manifest = generate_release_manifest(tmp_path, "v9.8.7-rc.1")
    before = Path(manifest).read_text(encoding="utf-8")
    regenerated = generate_release_manifest(tmp_path, "9.8.7-rc.1")
    assert regenerated == manifest
    assert regenerated.read_text(encoding="utf-8") == before
    lines = before.splitlines()
    assert lines[:2] == [
        "# Kassiber release manifest v1",
        "# Version: 9.8.7-rc.1",
    ]
    assert [line.split("  ", 1)[1] for line in lines[2:]] == sorted(
        ["kassiber-cli-linux-x64.tar.gz", "kassiber-macos-arm64.dmg"]
    )


def test_offline_signing_helper_pins_key_and_uses_sha512(tmp_path: Path) -> None:
    (tmp_path / "artifact.zip").write_bytes(b"artifact")
    manifest = generate_release_manifest(tmp_path, "1.2.3")
    signing_subkey = "B" * 40
    commands: list[list[str]] = []

    def fake_gpg(command, **kwargs):
        commands.append(command)
        if "--detach-sign" in command:
            output = Path(command[command.index("--output") + 1])
            output.write_text("test detached signature\n", encoding="utf-8")
            return subprocess.CompletedProcess(command, 0, "", "")
        status = (
            f"[GNUPG:] VALIDSIG {signing_subkey} 2026-01-01 1 0 4 0 22 10 00 "
            f"{TEST_RELEASE_FINGERPRINT}\n"
        )
        return subprocess.CompletedProcess(command, 0, status, "")

    with mock.patch.object(
        release_manifest_script.subprocess,
        "run",
        side_effect=fake_gpg,
    ):
        signature = release_manifest_script.sign_manifest(
            manifest,
            TEST_RELEASE_FINGERPRINT,
            gpg_executable="/test/bin/gpg",
        )

    assert signature.read_text(encoding="utf-8") == "test detached signature\n"
    signing_command = commands[0]
    assert signing_command[signing_command.index("--digest-algo") + 1] == "SHA512"
    assert signing_command[signing_command.index("--local-user") + 1] == TEST_RELEASE_FINGERPRINT


def test_verify_download_authenticates_signature_then_artifact(signed_release) -> None:
    result = verify_download(
        signed_release["artifact"],
        signed_release["manifest"],
        signed_release["signature"],
        signed_release["public_key"],
        signed_release["fingerprint"],
    )
    assert result["verified"] is True
    assert result["artifact"] == "kassiber-cli-linux-x64.tar.gz"
    assert result["signer_fingerprint"] == signed_release["fingerprint"]


def test_signature_only_verification_remains_available(signed_release) -> None:
    assert (
        verify_openpgp_signature(
            signed_release["manifest"],
            signed_release["signature"],
            signed_release["public_key"],
            signed_release["fingerprint"],
        )
        == signed_release["fingerprint"]
    )


def test_release_directory_requires_the_exact_authenticated_artifact_set(
    signed_release,
    tmp_path: Path,
) -> None:
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    for source in (
        signed_release["artifact"],
        RELEASE_FIXTURES / "kassiber-macos-arm64.dmg",
        signed_release["manifest"],
        signed_release["signature"],
    ):
        shutil.copy2(source, release_dir / Path(source).name)

    result = verify_release_directory(
        release_dir,
        release_dir / Path(signed_release["manifest"]).name,
        release_dir / Path(signed_release["signature"]).name,
        signed_release["public_key"],
        signed_release["fingerprint"],
    )
    assert result["verified"] is True
    assert result["artifact_count"] == 2
    assert result["complete"] is True

    (release_dir / "unexpected.zip").write_bytes(b"unexpected")
    with pytest.raises(AppError) as raised:
        verify_release_artifacts(
            release_dir,
            release_dir / Path(signed_release["manifest"]).name,
        )
    assert raised.value.code == "release_artifact_set_mismatch"


def test_channel_verification_can_authenticate_a_nonempty_manifest_subset(
    signed_release,
    tmp_path: Path,
) -> None:
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    for source in (
        signed_release["artifact"],
        signed_release["manifest"],
        signed_release["signature"],
    ):
        shutil.copy2(source, release_dir / Path(source).name)

    result = verify_release_directory(
        release_dir,
        release_dir / Path(signed_release["manifest"]).name,
        release_dir / Path(signed_release["signature"]).name,
        signed_release["public_key"],
        signed_release["fingerprint"],
        require_complete=False,
    )
    assert result["artifact_count"] == 1
    assert result["complete"] is False


def test_verify_download_rejects_tampered_artifact(signed_release, tmp_path: Path) -> None:
    artifact = tmp_path / Path(signed_release["artifact"]).name
    artifact.write_bytes(b"tampered artifact\n")
    with pytest.raises(AppError) as raised:
        verify_download(
            artifact,
            signed_release["manifest"],
            signed_release["signature"],
            signed_release["public_key"],
            signed_release["fingerprint"],
        )
    assert raised.value.code == "release_artifact_hash_mismatch"


def test_verify_download_rejects_tampered_manifest_before_hashing(
    signed_release,
    tmp_path: Path,
) -> None:
    manifest = tmp_path / Path(signed_release["manifest"]).name
    manifest.write_bytes(Path(signed_release["manifest"]).read_bytes() + b"\n")
    with pytest.raises(AppError) as raised:
        verify_download(
            tmp_path / "does-not-exist.tar.gz",
            manifest,
            signed_release["signature"],
            signed_release["public_key"],
            signed_release["fingerprint"],
        )
    assert raised.value.code == "release_signature_verification_failed"


def test_verify_download_hashes_against_the_exact_authenticated_manifest_snapshot(
    signed_release,
    tmp_path: Path,
) -> None:
    manifest = tmp_path / Path(signed_release["manifest"]).name
    manifest.write_bytes(Path(signed_release["manifest"]).read_bytes())

    def swap_original_after_snapshot(*args, **kwargs):
        manifest.write_text(f"{'0' * 64}  unrelated.zip\n", encoding="utf-8")
        return signed_release["fingerprint"]

    with mock.patch(
        "kassiber.release_verification._verify_openpgp_signature_bytes",
        side_effect=swap_original_after_snapshot,
    ):
        result = verify_download(
            signed_release["artifact"],
            manifest,
            signed_release["signature"],
            signed_release["public_key"],
            signed_release["fingerprint"],
        )
    assert result["verified"] is True
    assert result["artifact"] == Path(signed_release["artifact"]).name


def test_verify_download_rejects_unexpected_full_fingerprint(signed_release) -> None:
    fingerprint = str(signed_release["fingerprint"])
    wrong = fingerprint[:-1] + ("0" if fingerprint[-1] != "0" else "1")
    with pytest.raises(AppError) as raised:
        verify_download(
            signed_release["artifact"],
            signed_release["manifest"],
            signed_release["signature"],
            signed_release["public_key"],
            wrong,
        )
    assert raised.value.code == "release_fingerprint_mismatch"


def test_cli_verify_download_returns_machine_envelope(signed_release) -> None:
    output = io.StringIO()
    with (
        contextlib.redirect_stdout(output),
        mock.patch("kassiber.cli.main._configure_cli_logging"),
    ):
        exit_code = main(
            [
                "--machine",
                "verify-download",
                str(signed_release["artifact"]),
                "--manifest",
                str(signed_release["manifest"]),
                "--signature",
                str(signed_release["signature"]),
                "--public-key",
                str(signed_release["public_key"]),
                "--fingerprint",
                str(signed_release["fingerprint"]),
            ]
        )
    assert exit_code == 0
    envelope = json.loads(output.getvalue())
    assert envelope["kind"] == "verify-download"
    assert envelope["data"]["verified"] is True


def test_manifest_parser_rejects_paths_and_duplicate_entries(tmp_path: Path) -> None:
    invalid_dir = tmp_path / "invalid"
    invalid_dir.mkdir()
    invalid_path = invalid_dir / "kassiber-1.2.3-manifest.txt"
    invalid_path.write_text(
        f"# Kassiber release manifest v1\n# Version: 1.2.3\n{'a' * 64}  ../artifact\n",
        encoding="utf-8",
    )
    with pytest.raises(AppError, match="line 3") as path_error:
        parse_release_manifest(invalid_path)
    assert path_error.value.code == "invalid_release_manifest"

    duplicate_dir = tmp_path / "duplicate"
    duplicate_dir.mkdir()
    duplicate = duplicate_dir / "kassiber-1.2.3-manifest.txt"
    duplicate.write_text(
        "# Kassiber release manifest v1\n"
        "# Version: 1.2.3\n"
        f"{'a' * 64}  artifact.zip\n{'b' * 64}  artifact.zip\n",
        encoding="utf-8",
    )
    with pytest.raises(AppError, match="Duplicate") as duplicate_error:
        parse_release_manifest(duplicate)
    assert duplicate_error.value.code == "invalid_release_manifest"


def test_manifest_signed_version_must_match_its_filename(signed_release, tmp_path: Path) -> None:
    replayed = tmp_path / "kassiber-9.8.8-manifest.txt"
    replayed.write_bytes(Path(signed_release["manifest"]).read_bytes())
    with pytest.raises(AppError) as raised:
        parse_release_manifest(replayed)
    assert raised.value.code == "release_manifest_version_mismatch"


def test_fingerprint_requires_full_hex_value() -> None:
    with pytest.raises(AppError) as raised:
        normalize_fingerprint("DEAD BEEF")
    assert raised.value.code == "invalid_release_fingerprint"


def test_release_signing_policy_is_code_reviewed_and_fail_closed(tmp_path: Path) -> None:
    disabled = load_release_signing_policy(
        ROOT / "packaging" / "release" / "signing-policy.json",
        repository_root=ROOT,
    )
    assert disabled["enabled"] is False
    with pytest.raises(AppError) as raised:
        load_release_signing_policy(
            ROOT / "packaging" / "release" / "signing-policy.json",
            repository_root=ROOT,
            require_enabled=True,
        )
    assert raised.value.code == "release_signing_not_enabled"

    key = tmp_path / "release-key.asc"
    shutil.copy2(RELEASE_FIXTURES / "kassiber-release-test.asc", key)
    policy = tmp_path / "policy.json"
    policy.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "enabled": True,
                "primary_fingerprint": TEST_RELEASE_FINGERPRINT,
                "public_key_path": key.name,
            }
        ),
        encoding="utf-8",
    )
    enabled = load_release_signing_policy(
        policy,
        repository_root=tmp_path,
        require_enabled=True,
    )
    assert enabled["enabled"] is True
    assert enabled["primary_fingerprint"] == TEST_RELEASE_FINGERPRINT


def test_expired_or_revoked_signature_status_is_rejected() -> None:
    assert signature_status_has_failure("[GNUPG:] EXPKEYSIG DEADBEEF signer")
    assert signature_status_has_failure("[GNUPG:] REVKEYSIG DEADBEEF signer")
    assert signature_status_has_failure(
        "[GNUPG:] VALIDSIG " + "A" * 40 + " 2026-01-01 1 0 4 0 22 2 00"
    )
    assert not signature_status_has_failure("[GNUPG:] VALIDSIG " + "A" * 40)


def test_missing_gpg_fails_without_attempting_signature_verification(
    signed_release,
) -> None:
    with mock.patch("kassiber.release_verification.shutil.which", return_value=None):
        with pytest.raises(AppError) as raised:
            verify_download(
                signed_release["artifact"],
                signed_release["manifest"],
                signed_release["signature"],
                signed_release["public_key"],
                signed_release["fingerprint"],
            )
    assert raised.value.code == "gpg_unavailable"
