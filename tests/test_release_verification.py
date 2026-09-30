from __future__ import annotations

import base64
import contextlib
import io
import json
import os
import shutil
import subprocess
from pathlib import Path
from unittest import mock

import pytest

from kassiber.cli.main import main
from kassiber.errors import AppError
from kassiber.release_verification import (
    RELEASE_SIGNATURE_NAMESPACE,
    RELEASE_SIGNER_PRINCIPAL,
    generate_release_manifest,
    load_release_signing_policy,
    normalize_fingerprint,
    parse_release_manifest,
    parse_ssh_public_key,
    ssh_key_fingerprint,
    ssh_public_key_fingerprint,
    verify_download,
    verify_manifest_signature,
    verify_release_artifacts,
    verify_release_directory,
    verify_signature_bytes,
)
from scripts import release_manifest as release_manifest_script


ROOT = Path(__file__).resolve().parent.parent
RELEASE_FIXTURES = ROOT / "tests" / "fixtures" / "release_signing"
MANIFEST_NAME = "kassiber-9.8.7-rc.1-manifest.txt"
PUBLISHED_RELEASE_FINGERPRINT = "SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M"


@pytest.fixture(autouse=True)
def _never_contact_a_real_ssh_agent(monkeypatch: pytest.MonkeyPatch) -> None:
    # Tests sign only with throwaway private-key files; a developer's agent
    # (for example the Bitwarden agent holding the real release key) must not
    # even be asked to list identities.
    monkeypatch.delenv("SSH_AUTH_SOCK", raising=False)


def _require_ssh_keygen() -> str:
    executable = shutil.which("ssh-keygen")
    if executable is None:
        pytest.skip("OpenSSH ssh-keygen is required for release signature tests")
    return executable


def _generate_key(directory: Path, name: str) -> dict[str, object]:
    ssh_keygen = _require_ssh_keygen()
    private_key = directory / name
    subprocess.run(
        [ssh_keygen, "-q", "-t", "ed25519", "-N", "", "-C", f"{name}-throwaway", "-f", str(private_key)],
        check=True,
        stdin=subprocess.DEVNULL,
        env={key: value for key, value in os.environ.items() if key != "SSH_AUTH_SOCK"},
    )
    public_key = private_key.with_name(f"{name}.pub")
    return {
        "private_key": private_key,
        "public_key": public_key,
        "fingerprint": ssh_public_key_fingerprint(public_key.read_bytes()),
    }


def _ssh_sign(private_key: Path, data: bytes, *, namespace: str = RELEASE_SIGNATURE_NAMESPACE) -> bytes:
    return subprocess.run(
        [_require_ssh_keygen(), "-Y", "sign", "-n", namespace, "-f", str(private_key)],
        input=data,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=True,
        env={key: value for key, value in os.environ.items() if key != "SSH_AUTH_SOCK"},
    ).stdout


@pytest.fixture(scope="module")
def signed_release(tmp_path_factory: pytest.TempPathFactory) -> dict[str, object]:
    root = tmp_path_factory.mktemp("signed-release")
    keys = _generate_key(root, "kassiber-release-test")
    other = _generate_key(root, "unrelated-key")
    release = root / "release"
    release.mkdir()
    for name in ("kassiber-cli-linux-x64.tar.gz", "kassiber-macos-arm64.dmg", MANIFEST_NAME):
        shutil.copy2(RELEASE_FIXTURES / name, release / name)
    manifest = release / MANIFEST_NAME
    signature = release / f"{MANIFEST_NAME}.sig"
    signature.write_bytes(_ssh_sign(Path(keys["private_key"]), manifest.read_bytes()))
    return {
        "root": root,
        "artifact": release / "kassiber-cli-linux-x64.tar.gz",
        "release_dir": release,
        "manifest": manifest,
        "signature": signature,
        "public_key": keys["public_key"],
        "private_key": keys["private_key"],
        "fingerprint": keys["fingerprint"],
        "other": other,
    }


def _write_policy(
    root: Path,
    *,
    public_key: Path,
    fingerprint: str,
    allowed_signers: str | None = None,
    **overrides: object,
) -> Path:
    shutil.copy2(public_key, root / "release.pub")
    key_type, blob = parse_ssh_public_key(public_key.read_bytes())
    encoded = base64.b64encode(blob).decode("ascii")
    (root / "allowed_signers").write_text(
        allowed_signers
        if allowed_signers is not None
        else f'{RELEASE_SIGNER_PRINCIPAL} namespaces="{RELEASE_SIGNATURE_NAMESPACE}" {key_type} {encoded}\n',
        encoding="ascii",
    )
    payload: dict[str, object] = {
        "schema_version": 2,
        "enabled": True,
        "fingerprint": fingerprint,
        "namespace": RELEASE_SIGNATURE_NAMESPACE,
        "principal": RELEASE_SIGNER_PRINCIPAL,
        "public_key_path": "release.pub",
        "allowed_signers_path": "allowed_signers",
    }
    payload.update(overrides)
    policy = root / "policy.json"
    policy.write_text(json.dumps(payload), encoding="utf-8")
    return policy


def _fresh_manifest(directory: Path) -> Path:
    directory.mkdir()
    (directory / "artifact.zip").write_bytes(b"artifact")
    return generate_release_manifest(directory, "1.2.3")


def test_manifest_is_stable_sorted_and_does_not_hash_itself(tmp_path: Path) -> None:
    (tmp_path / "kassiber-cli-linux-x64.tar.gz").write_bytes(b"cli")
    (tmp_path / "kassiber-macos-arm64.dmg").write_bytes(b"desktop")
    manifest = generate_release_manifest(tmp_path, "v9.8.7-rc.1")
    (tmp_path / f"{manifest.name}.sig").write_bytes(b"not hashed")
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


def test_python_fingerprint_matches_ssh_keygen(signed_release) -> None:
    listed = subprocess.run(
        [_require_ssh_keygen(), "-E", "sha256", "-lf", str(signed_release["public_key"])],
        stdout=subprocess.PIPE,
        check=True,
        text=True,
    ).stdout.split()
    assert listed[1] == signed_release["fingerprint"]


def test_repository_policy_pins_the_published_release_key() -> None:
    policy = load_release_signing_policy(
        ROOT / "packaging" / "release" / "signing-policy.json",
        repository_root=ROOT,
        require_enabled=True,
    )
    assert policy["enabled"] is True
    assert policy["fingerprint"] == PUBLISHED_RELEASE_FINGERPRINT
    assert policy["namespace"] == "kassiber-release"
    assert policy["principal"] == "release@kassiber"
    public_key = Path(str(policy["public_key_path"])).read_bytes()
    assert ssh_public_key_fingerprint(public_key) == PUBLISHED_RELEASE_FINGERPRINT
    assert parse_ssh_public_key(public_key)[0] == "ssh-ed25519"


def test_signing_helper_signs_verifies_and_writes_signature(signed_release, tmp_path: Path) -> None:
    manifest = _fresh_manifest(tmp_path / "release")
    signature = release_manifest_script.sign_manifest(
        manifest,
        Path(signed_release["public_key"]),
        str(signed_release["fingerprint"]),
        signing_key=Path(signed_release["private_key"]),
    )
    assert signature == manifest.with_name(f"{manifest.name}.sig")
    assert signature.read_bytes().startswith(b"-----BEGIN SSH SIGNATURE-----")
    assert (
        verify_manifest_signature(
            manifest,
            signature,
            signed_release["public_key"],
            signed_release["fingerprint"],
        )
        == signed_release["fingerprint"]
    )
    assert not list(manifest.parent.glob(".*.tmp"))


def test_signing_helper_uses_the_release_namespace_and_public_key_for_the_agent(
    signed_release,
    tmp_path: Path,
) -> None:
    manifest = _fresh_manifest(tmp_path / "release")
    commands: list[list[str]] = []
    real_run = subprocess.run

    def record(command, **kwargs):
        commands.append(list(command))
        if command[1:3] == ["-Y", "sign"]:
            # Stand in for the agent: sign with the throwaway private key.
            command = [*command[:-1], str(signed_release["private_key"])]
        return real_run(command, **kwargs)

    with mock.patch.object(release_manifest_script.subprocess, "run", side_effect=record):
        release_manifest_script.sign_manifest(
            manifest,
            Path(signed_release["public_key"]),
            str(signed_release["fingerprint"]),
        )
    signing = commands[0]
    assert signing[1:5] == ["-Y", "sign", "-n", "kassiber-release"]
    assert signing[signing.index("-f") + 1] == str(signed_release["public_key"])


def test_signing_helper_refuses_to_overwrite_an_existing_signature(
    signed_release,
    tmp_path: Path,
) -> None:
    manifest = _fresh_manifest(tmp_path / "release")
    existing = manifest.with_name(f"{manifest.name}.sig")
    existing.write_bytes(b"operator signature\n")
    with mock.patch.object(release_manifest_script.subprocess, "run") as run:
        with pytest.raises(AppError) as raised:
            release_manifest_script.sign_manifest(
                manifest,
                Path(signed_release["public_key"]),
                str(signed_release["fingerprint"]),
                signing_key=Path(signed_release["private_key"]),
            )
    assert raised.value.code == "release_signature_exists"
    run.assert_not_called()
    assert existing.read_bytes() == b"operator signature\n"

    replaced = release_manifest_script.sign_manifest(
        manifest,
        Path(signed_release["public_key"]),
        str(signed_release["fingerprint"]),
        signing_key=Path(signed_release["private_key"]),
        overwrite=True,
    )
    assert replaced.read_bytes().startswith(b"-----BEGIN SSH SIGNATURE-----")


def test_signing_helper_refuses_a_key_that_does_not_match_the_policy(
    signed_release,
    tmp_path: Path,
) -> None:
    manifest = _fresh_manifest(tmp_path / "release")
    other = signed_release["other"]
    with mock.patch.object(release_manifest_script.subprocess, "run") as run:
        with pytest.raises(AppError) as raised:
            release_manifest_script.sign_manifest(
                manifest,
                Path(other["public_key"]),
                str(signed_release["fingerprint"]),
                signing_key=Path(other["private_key"]),
            )
    assert raised.value.code == "release_fingerprint_mismatch"
    run.assert_not_called()
    assert not manifest.with_name(f"{manifest.name}.sig").exists()


def test_signing_helper_rejects_a_signature_from_a_different_private_key(
    signed_release,
    tmp_path: Path,
) -> None:
    manifest = _fresh_manifest(tmp_path / "release")
    with pytest.raises(AppError) as raised:
        release_manifest_script.sign_manifest(
            manifest,
            Path(signed_release["public_key"]),
            str(signed_release["fingerprint"]),
            signing_key=Path(signed_release["other"]["private_key"]),
        )
    assert raised.value.code == "release_signing_failed"
    assert not manifest.with_name(f"{manifest.name}.sig").exists()


def test_signing_cli_takes_the_fingerprint_from_the_enabled_policy(tmp_path: Path) -> None:
    manifest = _fresh_manifest(tmp_path / "release")
    public_key = ROOT / "packaging" / "release" / "kassiber-release.pub"
    with mock.patch.object(
        release_manifest_script,
        "sign_manifest",
        return_value=manifest.with_name(f"{manifest.name}.sig"),
    ) as sign:
        with contextlib.redirect_stdout(io.StringIO()) as output:
            exit_code = release_manifest_script.main(
                ["sign", "--manifest", str(manifest), "--public-key", str(public_key)]
            )
    assert exit_code == 0
    assert sign.call_args.args[2] == PUBLISHED_RELEASE_FINGERPRINT
    assert json.loads(output.getvalue())["signer_fingerprint"] == PUBLISHED_RELEASE_FINGERPRINT


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
    assert result["signer_principal"] == "release@kassiber"
    assert result["signature_namespace"] == "kassiber-release"


def test_release_directory_requires_the_exact_authenticated_artifact_set(
    signed_release,
    tmp_path: Path,
) -> None:
    release_dir = tmp_path / "release"
    shutil.copytree(signed_release["release_dir"], release_dir)

    result = verify_release_directory(
        release_dir,
        release_dir / MANIFEST_NAME,
        release_dir / f"{MANIFEST_NAME}.sig",
        signed_release["public_key"],
        signed_release["fingerprint"],
    )
    assert result["verified"] is True
    assert result["artifact_count"] == 2
    assert result["complete"] is True

    # A leftover OpenPGP signature is an unexpected asset, not metadata.
    (release_dir / f"{MANIFEST_NAME}.asc").write_bytes(b"stray")
    with pytest.raises(AppError) as raised:
        verify_release_artifacts(release_dir, release_dir / MANIFEST_NAME)
    assert raised.value.code == "release_artifact_set_mismatch"


def test_release_directory_requires_the_manifest_signature_name(
    signed_release,
    tmp_path: Path,
) -> None:
    release_dir = tmp_path / "release"
    shutil.copytree(signed_release["release_dir"], release_dir)
    renamed = release_dir / f"{MANIFEST_NAME}.asc"
    (release_dir / f"{MANIFEST_NAME}.sig").rename(renamed)
    with pytest.raises(AppError) as raised:
        verify_release_directory(
            release_dir,
            release_dir / MANIFEST_NAME,
            renamed,
            signed_release["public_key"],
            signed_release["fingerprint"],
        )
    assert raised.value.code == "release_artifact_set_mismatch"


def test_channel_verification_can_authenticate_a_nonempty_manifest_subset(
    signed_release,
    tmp_path: Path,
) -> None:
    release_dir = tmp_path / "release"
    shutil.copytree(signed_release["release_dir"], release_dir)
    (release_dir / "kassiber-macos-arm64.dmg").unlink()

    result = verify_release_directory(
        release_dir,
        release_dir / MANIFEST_NAME,
        release_dir / f"{MANIFEST_NAME}.sig",
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
    manifest = tmp_path / MANIFEST_NAME
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


def test_signature_in_another_namespace_is_rejected(signed_release, tmp_path: Path) -> None:
    manifest_bytes = Path(signed_release["manifest"]).read_bytes()
    for namespace in ("file", "git", "kassiber-release-test"):
        signature = _ssh_sign(Path(signed_release["private_key"]), manifest_bytes, namespace=namespace)
        with pytest.raises(AppError) as raised:
            verify_signature_bytes(
                manifest_bytes,
                signature,
                Path(signed_release["public_key"]).read_bytes(),
                str(signed_release["fingerprint"]),
            )
        assert raised.value.code == "release_signature_verification_failed"


def test_signature_from_an_unpinned_key_is_rejected(signed_release) -> None:
    manifest_bytes = Path(signed_release["manifest"]).read_bytes()
    other = signed_release["other"]
    forged = _ssh_sign(Path(other["private_key"]), manifest_bytes)
    with pytest.raises(AppError) as raised:
        verify_signature_bytes(
            manifest_bytes,
            forged,
            Path(signed_release["public_key"]).read_bytes(),
            str(signed_release["fingerprint"]),
        )
    assert raised.value.code == "release_signature_verification_failed"


def test_key_bundled_with_a_download_must_match_the_pinned_fingerprint(signed_release) -> None:
    # An attacker can ship their own key and a valid signature; only the
    # independently obtained fingerprint decides.
    other = signed_release["other"]
    manifest_bytes = Path(signed_release["manifest"]).read_bytes()
    forged = _ssh_sign(Path(other["private_key"]), manifest_bytes)
    with mock.patch("kassiber.release_verification.subprocess.run") as run:
        with pytest.raises(AppError) as raised:
            verify_signature_bytes(
                manifest_bytes,
                forged,
                Path(other["public_key"]).read_bytes(),
                str(signed_release["fingerprint"]),
            )
    assert raised.value.code == "release_fingerprint_mismatch"
    run.assert_not_called()


def test_verify_download_rejects_unexpected_full_fingerprint(signed_release) -> None:
    fingerprint = str(signed_release["fingerprint"])
    wrong = fingerprint[:-1] + ("A" if fingerprint[-1] != "A" else "B")
    with pytest.raises(AppError) as raised:
        verify_download(
            signed_release["artifact"],
            signed_release["manifest"],
            signed_release["signature"],
            signed_release["public_key"],
            wrong,
        )
    assert raised.value.code == "release_fingerprint_mismatch"


def _completed(returncode: int, stdout: str = "", stderr: str = "") -> subprocess.CompletedProcess:
    return subprocess.CompletedProcess([], returncode, stdout.encode(), stderr.encode())


def test_good_line_for_another_principal_or_key_is_not_accepted(signed_release) -> None:
    manifest_bytes = Path(signed_release["manifest"]).read_bytes()
    signature = Path(signed_release["signature"]).read_bytes()
    public_key = Path(signed_release["public_key"]).read_bytes()
    fingerprint = str(signed_release["fingerprint"])
    wrong_principal = f'Good "kassiber-release" signature for mallory@kassiber with ED25519 key {fingerprint}\n'
    wrong_namespace = f'Good "file" signature for release@kassiber with ED25519 key {fingerprint}\n'
    wrong_key = f'Good "kassiber-release" signature for release@kassiber with ED25519 key {signed_release["other"]["fingerprint"]}\n'
    for completed in (
        _completed(0, wrong_principal),
        _completed(0, wrong_namespace),
        _completed(0, wrong_key),
        _completed(0, ""),
        _completed(255, f'Good "kassiber-release" signature for release@kassiber with ED25519 key {fingerprint}\n'),
    ):
        with (
            mock.patch("kassiber.release_verification.subprocess.run", return_value=completed),
            mock.patch("kassiber.release_verification.shutil.which", return_value="/test/ssh-keygen"),
        ):
            with pytest.raises(AppError) as raised:
                verify_signature_bytes(manifest_bytes, signature, public_key, fingerprint)
        assert raised.value.code == "release_signature_verification_failed"


def test_verifier_runs_ssh_keygen_without_a_shell_and_with_a_canonical_signer(
    signed_release,
) -> None:
    captured: dict[str, object] = {}
    real_run = subprocess.run

    def record(command, **kwargs):
        allowed = Path(command[command.index("-f") + 1])
        captured["command"] = list(command)
        captured["kwargs"] = kwargs
        captured["allowed_signers"] = allowed.read_text(encoding="ascii")
        return real_run(command, **kwargs)

    with mock.patch("kassiber.release_verification.subprocess.run", side_effect=record):
        verify_manifest_signature(
            signed_release["manifest"],
            signed_release["signature"],
            signed_release["public_key"],
            signed_release["fingerprint"],
        )
    command = captured["command"]
    assert command[1:3] == ["-Y", "verify"]
    assert command[command.index("-I") + 1] == "release@kassiber"
    assert command[command.index("-n") + 1] == "kassiber-release"
    assert "shell" not in captured["kwargs"]
    assert captured["kwargs"]["input"] == Path(signed_release["manifest"]).read_bytes()
    assert captured["allowed_signers"].startswith(
        'release@kassiber namespaces="kassiber-release" ssh-ed25519 '
    )
    assert captured["allowed_signers"].count("\n") == 1


def test_verify_download_hashes_against_the_exact_authenticated_manifest_snapshot(
    signed_release,
    tmp_path: Path,
) -> None:
    manifest = tmp_path / MANIFEST_NAME
    manifest.write_bytes(Path(signed_release["manifest"]).read_bytes())

    def swap_original_after_snapshot(*args, **kwargs):
        manifest.write_text(f"{'0' * 64}  unrelated.zip\n", encoding="utf-8")
        return signed_release["fingerprint"]

    with mock.patch(
        "kassiber.release_verification.verify_signature_bytes",
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
    assert envelope["data"]["signer_fingerprint"] == signed_release["fingerprint"]


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


def test_manifest_signed_version_must_match_its_filename(tmp_path: Path) -> None:
    replayed = tmp_path / "kassiber-9.8.8-manifest.txt"
    replayed.write_bytes((RELEASE_FIXTURES / MANIFEST_NAME).read_bytes())
    with pytest.raises(AppError) as raised:
        parse_release_manifest(replayed)
    assert raised.value.code == "release_manifest_version_mismatch"


@pytest.mark.parametrize(
    "value",
    [
        "DEAD BEEF",
        "51A005054F536D083E3DB9877034D4B74447F840",
        "UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M",
        "SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9",
        "SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M=",
        "MD5:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff",
    ],
)
def test_fingerprint_requires_complete_sha256_value(value: str) -> None:
    with pytest.raises(AppError) as raised:
        normalize_fingerprint(value)
    assert raised.value.code == "invalid_release_fingerprint"


def test_fingerprint_tolerates_only_surrounding_whitespace() -> None:
    assert normalize_fingerprint(f"  {PUBLISHED_RELEASE_FINGERPRINT}\n") == PUBLISHED_RELEASE_FINGERPRINT


def test_public_key_parser_accepts_only_one_ed25519_family_key() -> None:
    ed25519 = (ROOT / "packaging" / "release" / "kassiber-release.pub").read_bytes()
    assert parse_ssh_public_key(ed25519)[0] == "ssh-ed25519"

    sk_blob = b"".join(
        len(field).to_bytes(4, "big") + field
        for field in (b"sk-ssh-ed25519@openssh.com", b"\x01" * 32, b"ssh:")
    )
    sk_line = f"sk-ssh-ed25519@openssh.com {base64.b64encode(sk_blob).decode()} hardware\n"
    key_type, blob = parse_ssh_public_key(sk_line.encode())
    assert key_type == "sk-ssh-ed25519@openssh.com"
    assert ssh_key_fingerprint(blob).startswith("SHA256:")

    mislabeled = ed25519.replace(b"ssh-ed25519 ", b"sk-ssh-ed25519@openssh.com ", 1)
    truncated_blob = base64.b64encode(base64.b64decode(ed25519.split()[1])[:-1])
    rsa_line = b"ssh-rsa " + base64.b64encode(b"\x00\x00\x00\x07ssh-rsa") + b" rsa\n"
    for invalid in (
        b"",
        ed25519 + ed25519,
        mislabeled,
        b"ssh-ed25519 " + truncated_blob + b"\n",
        b"ssh-ed25519 not*base64\n",
        rsa_line,
        b'release@kassiber namespaces="kassiber-release" ' + ed25519,
    ):
        with pytest.raises(AppError) as raised:
            parse_ssh_public_key(invalid)
        assert raised.value.code == "invalid_release_public_key"


def test_release_signing_policy_accepts_a_consistent_throwaway_key(
    signed_release,
    tmp_path: Path,
) -> None:
    policy = _write_policy(
        tmp_path,
        public_key=Path(signed_release["public_key"]),
        fingerprint=str(signed_release["fingerprint"]),
    )
    enabled = load_release_signing_policy(policy, repository_root=tmp_path, require_enabled=True)
    assert enabled["enabled"] is True
    assert enabled["fingerprint"] == signed_release["fingerprint"]
    assert enabled["allowed_signers_path"] == str((tmp_path / "allowed_signers").resolve())


def test_release_signing_policy_disabled_state_fails_closed(signed_release, tmp_path: Path) -> None:
    policy = _write_policy(
        tmp_path,
        public_key=Path(signed_release["public_key"]),
        fingerprint="",
        enabled=False,
    )
    assert load_release_signing_policy(policy, repository_root=tmp_path)["enabled"] is False
    with pytest.raises(AppError) as raised:
        load_release_signing_policy(policy, repository_root=tmp_path, require_enabled=True)
    assert raised.value.code == "release_signing_not_enabled"

    policy = _write_policy(
        tmp_path,
        public_key=Path(signed_release["public_key"]),
        fingerprint=str(signed_release["fingerprint"]),
        enabled=False,
    )
    with pytest.raises(AppError) as raised:
        load_release_signing_policy(policy, repository_root=tmp_path)
    assert raised.value.code == "invalid_release_signing_policy"


def _policy_error(signed_release, root: Path, **kwargs: object) -> str:
    kwargs.setdefault("fingerprint", str(signed_release["fingerprint"]))
    policy = _write_policy(root, public_key=Path(signed_release["public_key"]), **kwargs)
    with pytest.raises(AppError) as raised:
        load_release_signing_policy(policy, repository_root=root, require_enabled=True)
    return raised.value.code


@pytest.mark.parametrize(
    "overrides",
    [
        {"schema_version": 1},
        {"schema_version": True},
        {"schema_version": 2.0},
        {"namespace": "file"},
        {"principal": "mallory@kassiber"},
        {"public_key_path": "../release.pub"},
        {"allowed_signers_path": "/etc/allowed_signers"},
        {"allowed_signers_path": ""},
        {"enabled": "true"},
    ],
)
def test_release_signing_policy_rejects_invalid_fields(
    signed_release,
    tmp_path: Path,
    overrides: dict[str, object],
) -> None:
    assert _policy_error(signed_release, tmp_path, **overrides) == "invalid_release_signing_policy"


def test_release_signing_policy_rejects_a_mismatched_fingerprint(
    signed_release,
    tmp_path: Path,
) -> None:
    code = _policy_error(
        signed_release,
        tmp_path,
        fingerprint=str(signed_release["other"]["fingerprint"]),
    )
    assert code == "invalid_release_signing_policy"


def test_release_signing_policy_rejects_inconsistent_allowed_signers(
    signed_release,
    tmp_path: Path,
) -> None:
    key_type, blob = parse_ssh_public_key(Path(signed_release["public_key"]).read_bytes())
    encoded = base64.b64encode(blob).decode("ascii")
    other_type, other_blob = parse_ssh_public_key(
        Path(signed_release["other"]["public_key"]).read_bytes()
    )
    other_encoded = base64.b64encode(other_blob).decode("ascii")
    for allowed_signers in (
        f'mallory@kassiber namespaces="kassiber-release" {key_type} {encoded}\n',
        f'release@kassiber namespaces="file" {key_type} {encoded}\n',
        f'release@kassiber namespaces="kassiber-release,file" {key_type} {encoded}\n',
        f"release@kassiber {key_type} {encoded}\n",
        f'release@kassiber cert-authority namespaces="kassiber-release" {key_type} {encoded}\n',
        f'release@kassiber namespaces="kassiber-release" {other_type} {other_encoded}\n',
        (
            f'release@kassiber namespaces="kassiber-release" {key_type} {encoded}\n'
            f'release@kassiber namespaces="kassiber-release" {other_type} {other_encoded}\n'
        ),
    ):
        code = _policy_error(signed_release, tmp_path, allowed_signers=allowed_signers)
        assert code == "invalid_release_signing_policy", allowed_signers


def test_committed_allowed_signers_verifies_with_plain_ssh_keygen(signed_release, tmp_path: Path) -> None:
    # Users may run ssh-keygen directly with the committed allowed_signers file;
    # prove that shape works with a throwaway key.
    policy = _write_policy(
        tmp_path,
        public_key=Path(signed_release["public_key"]),
        fingerprint=str(signed_release["fingerprint"]),
    )
    load_release_signing_policy(policy, repository_root=tmp_path, require_enabled=True)
    completed = subprocess.run(
        [
            _require_ssh_keygen(),
            "-Y",
            "verify",
            "-f",
            str(tmp_path / "allowed_signers"),
            "-I",
            "release@kassiber",
            "-n",
            "kassiber-release",
            "-s",
            str(signed_release["signature"]),
        ],
        input=Path(signed_release["manifest"]).read_bytes(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    assert completed.returncode == 0
    assert completed.stdout.decode().startswith('Good "kassiber-release" signature for release@kassiber')


def test_missing_ssh_keygen_fails_without_attempting_verification(signed_release) -> None:
    with (
        mock.patch("kassiber.release_verification.shutil.which", return_value=None),
        mock.patch("kassiber.release_verification.subprocess.run") as run,
    ):
        with pytest.raises(AppError) as raised:
            verify_download(
                signed_release["artifact"],
                signed_release["manifest"],
                signed_release["signature"],
                signed_release["public_key"],
                signed_release["fingerprint"],
            )
    assert raised.value.code == "ssh_keygen_unavailable"
    run.assert_not_called()


@pytest.mark.parametrize(
    "stderr",
    [
        "ssh-keygen: illegal option -- Y\nusage: ssh-keygen [-q] [-b bits] ...\n",
        "unknown option -- Y\nusage: ssh-keygen [options]\n",
        "ssh-keygen: invalid option -- 'Y'\n",
    ],
)
def test_ssh_keygen_older_than_8_1_fails_closed(signed_release, stderr: str) -> None:
    with (
        mock.patch("kassiber.release_verification.shutil.which", return_value="/test/ssh-keygen"),
        mock.patch(
            "kassiber.release_verification.subprocess.run",
            return_value=_completed(1, stderr=stderr),
        ),
    ):
        with pytest.raises(AppError) as raised:
            verify_download(
                signed_release["artifact"],
                signed_release["manifest"],
                signed_release["signature"],
                signed_release["public_key"],
                signed_release["fingerprint"],
            )
    assert raised.value.code == "ssh_keygen_unsupported"


def test_ssh_keygen_timeout_or_launch_failure_fails_closed(signed_release) -> None:
    for failure in (subprocess.TimeoutExpired(["ssh-keygen"], 30), OSError("exec format error")):
        with (
            mock.patch("kassiber.release_verification.shutil.which", return_value="/test/ssh-keygen"),
            mock.patch("kassiber.release_verification.subprocess.run", side_effect=failure),
        ):
            with pytest.raises(AppError) as raised:
                verify_manifest_signature(
                    signed_release["manifest"],
                    signed_release["signature"],
                    signed_release["public_key"],
                    signed_release["fingerprint"],
                )
        assert raised.value.code == "release_signature_verification_failed"


def test_non_ssh_signature_is_rejected_before_running_ssh_keygen(signed_release) -> None:
    with mock.patch("kassiber.release_verification.subprocess.run") as run:
        with pytest.raises(AppError) as raised:
            verify_signature_bytes(
                Path(signed_release["manifest"]).read_bytes(),
                b"-----BEGIN PGP SIGNATURE-----\n",
                Path(signed_release["public_key"]).read_bytes(),
                str(signed_release["fingerprint"]),
            )
    assert raised.value.code == "release_signature_verification_failed"
    run.assert_not_called()
