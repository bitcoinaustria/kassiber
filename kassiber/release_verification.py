"""Sparrow-style OpenPGP release manifest verification.

Kassiber signs one deterministic SHA-256 manifest rather than signing each
package separately.  The OpenPGP signature authenticates the manifest; the
manifest then authenticates the selected release artifact.

The verifier deliberately uses an isolated temporary GnuPG home and requires
an expected full fingerprint.  Importing a public key supplied alongside a
release is not a trust decision by itself.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import os
import re
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path
from typing import Iterable

from .errors import AppError
from .update_check import _has_exact_schema_version, parse_version


MAX_MANIFEST_BYTES = 1024 * 1024
MAX_SIGNATURE_BYTES = 128 * 1024
MAX_PUBLIC_KEY_BYTES = 2 * 1024 * 1024
MAX_SIGNING_POLICY_BYTES = 64 * 1024
_MANIFEST_LINE = re.compile(
    r"(?P<sha256>[0-9a-f]{64})  (?P<filename>[A-Za-z0-9][A-Za-z0-9._+-]{0,254})"
)
_FINGERPRINT = re.compile(r"(?:[0-9A-F]{40}|[0-9A-F]{64})")
_MANIFEST_HEADER = "# Kassiber release manifest v1"
_MANIFEST_VERSION_PREFIX = "# Version: "
_REJECTED_SIGNATURE_STATUSES = frozenset(
    {
        "BADSIG",
        "ERRSIG",
        "EXPSIG",
        "EXPKEYSIG",
        "REVKEYSIG",
        "KEYEXPIRED",
        "KEYREVOKED",
        "SIGEXPIRED",
    }
)
_ACCEPTED_OPENPGP_HASH_ALGORITHMS = frozenset({"8", "9", "10", "11"})


def normalize_fingerprint(value: str) -> str:
    """Normalize a displayed OpenPGP fingerprint and require its full form."""

    normalized = "".join(value.split()).upper()
    if not _FINGERPRINT.fullmatch(normalized):
        raise AppError(
            "OpenPGP fingerprint must be a full 40- or 64-character hexadecimal fingerprint",
            code="invalid_release_fingerprint",
            hint="Copy the complete Kassiber release-key fingerprint from an independent trusted source.",
        )
    return normalized


def load_release_signing_policy(
    path: str | os.PathLike[str],
    *,
    repository_root: str | os.PathLike[str],
    require_enabled: bool = False,
) -> dict[str, object]:
    """Load the code-reviewed release trust root used by publication jobs."""

    policy_path = Path(path).expanduser()
    raw = _require_small_regular_file(
        policy_path,
        limit=MAX_SIGNING_POLICY_BYTES,
        label="release signing policy",
    )
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AppError(
            "Release signing policy is not valid JSON",
            code="invalid_release_signing_policy",
        ) from exc
    if not _has_exact_schema_version(payload, 1):
        raise AppError(
            "Release signing policy has no supported schema",
            code="invalid_release_signing_policy",
        )
    enabled = payload.get("enabled")
    fingerprint = payload.get("primary_fingerprint")
    public_key_value = payload.get("public_key_path")
    if (
        not isinstance(enabled, bool)
        or not isinstance(fingerprint, str)
        or not isinstance(public_key_value, str)
        or not public_key_value
    ):
        raise AppError(
            "Release signing policy fields are invalid",
            code="invalid_release_signing_policy",
        )
    root = Path(repository_root).expanduser().resolve()
    public_key_relative = Path(public_key_value)
    if public_key_relative.is_absolute():
        raise AppError(
            "Release public key path must be repository-relative",
            code="invalid_release_signing_policy",
        )
    public_key_path = (root / public_key_relative).resolve()
    try:
        public_key_path.relative_to(root)
    except ValueError as exc:
        raise AppError(
            "Release public key path escapes the repository",
            code="invalid_release_signing_policy",
        ) from exc
    if not enabled:
        if fingerprint.strip():
            raise AppError(
                "Disabled release signing policy must not carry a fingerprint",
                code="invalid_release_signing_policy",
            )
        if require_enabled:
            raise AppError(
                "Signed release publication is not enabled",
                code="release_signing_not_enabled",
                hint="Complete the offline key ceremony and enable the code-reviewed signing policy first.",
            )
        return {
            "enabled": False,
            "primary_fingerprint": "",
            "public_key_path": str(public_key_path),
        }

    normalized = normalize_fingerprint(fingerprint)
    _require_small_regular_file(
        public_key_path,
        limit=MAX_PUBLIC_KEY_BYTES,
        label="release public key",
    )
    return {
        "enabled": True,
        "primary_fingerprint": normalized,
        "public_key_path": str(public_key_path),
    }


def _open_regular_file(path: Path, *, label: str):
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        raise AppError(
            f"Could not read {label}: {path}",
            code="release_verification_file_error",
            details={"path": str(path), "label": label},
        ) from exc
    try:
        stat_result = os.fstat(descriptor)
    except OSError as exc:
        os.close(descriptor)
        raise AppError(
            f"Could not inspect {label}: {path}",
            code="release_verification_file_error",
            details={"path": str(path), "label": label},
        ) from exc
    if not stat.S_ISREG(stat_result.st_mode):
        os.close(descriptor)
        raise AppError(
            f"{label.capitalize()} is not a regular file: {path}",
            code="release_verification_file_error",
            details={"path": str(path), "label": label},
        )
    return os.fdopen(descriptor, "rb"), stat_result


def _require_small_regular_file(path: Path, *, limit: int, label: str) -> bytes:
    handle, stat_result = _open_regular_file(path, label=label)
    if stat_result.st_size > limit:
        handle.close()
        raise AppError(
            f"{label.capitalize()} is unexpectedly large",
            code="release_verification_file_error",
            details={"path": str(path), "maximum_bytes": limit},
        )
    try:
        with handle:
            content = handle.read(limit + 1)
    except OSError as exc:
        raise AppError(
            f"Could not read {label}: {path}",
            code="release_verification_file_error",
            details={"path": str(path), "label": label},
        ) from exc
    if len(content) > limit:
        raise AppError(
            f"{label.capitalize()} is unexpectedly large",
            code="release_verification_file_error",
            details={"path": str(path), "maximum_bytes": limit},
        )
    return content


def _normalize_release_version(version: str) -> str:
    normalized = version.removeprefix("v")
    # parse_version tolerates surrounding whitespace and one leading "v";
    # manifest names must not, so reject both explicitly.
    if (
        normalized != normalized.strip()
        or normalized.startswith("v")
        or parse_version(normalized) is None
    ):
        raise AppError(
            f"Invalid release version: {version}",
            code="invalid_release_version",
        )
    return normalized


def release_manifest_name(version: str) -> str:
    return f"kassiber-{_normalize_release_version(version)}-manifest.txt"


def _release_version_from_manifest_name(name: str) -> str:
    prefix = "kassiber-"
    suffix = "-manifest.txt"
    if not name.startswith(prefix) or not name.endswith(suffix):
        raise AppError(
            f"Invalid Kassiber release manifest filename: {name}",
            code="invalid_release_manifest",
        )
    return _normalize_release_version(name[len(prefix) : -len(suffix)])


def _parse_release_manifest_bytes(
    raw: bytes,
    *,
    source: str,
    expected_version: str,
) -> dict[str, str]:
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise AppError(
            "Release manifest is not valid UTF-8",
            code="invalid_release_manifest",
            details={"path": source},
        ) from exc

    lines = text.splitlines()
    if len(lines) < 3 or lines[0] != _MANIFEST_HEADER:
        raise AppError(
            "Release manifest has no supported Kassiber format header",
            code="invalid_release_manifest",
            details={"path": source},
        )
    expected_version_line = f"{_MANIFEST_VERSION_PREFIX}{expected_version}"
    if lines[1] != expected_version_line:
        raise AppError(
            "Signed release version does not match the manifest filename",
            code="release_manifest_version_mismatch",
            hint="Do not install or run artifacts from this release.",
            details={
                "path": source,
                "expected_version": expected_version,
                "signed_version": lines[1].removeprefix(_MANIFEST_VERSION_PREFIX)[:128],
            },
        )

    entries: dict[str, str] = {}
    for line_number, line in enumerate(lines[2:], start=3):
        match = _MANIFEST_LINE.fullmatch(line)
        if match is None:
            raise AppError(
                f"Invalid release manifest line {line_number}",
                code="invalid_release_manifest",
                details={"path": source, "line": line_number},
            )
        filename = match.group("filename")
        if filename in entries:
            raise AppError(
                f"Duplicate release manifest entry: {filename}",
                code="invalid_release_manifest",
                details={"path": source, "filename": filename},
            )
        entries[filename] = match.group("sha256")
    if not entries:
        raise AppError(
            "Release manifest is empty",
            code="invalid_release_manifest",
            details={"path": source},
        )
    return entries


def parse_release_manifest(path: str | os.PathLike[str]) -> dict[str, str]:
    """Parse Kassiber's strict, GNU-compatible SHA-256 manifest format."""

    manifest_path = Path(path).expanduser()
    raw = _require_small_regular_file(
        manifest_path,
        limit=MAX_MANIFEST_BYTES,
        label="release manifest",
    )
    return _parse_release_manifest_bytes(
        raw,
        source=str(manifest_path),
        expected_version=_release_version_from_manifest_name(manifest_path.name),
    )


def sha256_file(path: str | os.PathLike[str]) -> str:
    artifact_path = Path(path).expanduser()
    handle, _ = _open_regular_file(artifact_path, label="release artifact")
    digest = hashlib.sha256()
    try:
        with handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as exc:
        raise AppError(
            f"Could not read release artifact: {artifact_path}",
            code="release_verification_file_error",
            details={"path": str(artifact_path), "label": "release artifact"},
        ) from exc
    return digest.hexdigest()


def verify_artifact_hash(
    artifact: str | os.PathLike[str],
    manifest: str | os.PathLike[str],
) -> dict[str, str]:
    artifact_path = Path(artifact).expanduser()
    manifest_path = Path(manifest).expanduser()
    entries = parse_release_manifest(manifest_path)
    return _verify_artifact_hash_entries(artifact_path, entries, manifest_path.name)


def _verify_artifact_hash_entries(
    artifact_path: Path,
    entries: dict[str, str],
    manifest_name: str,
) -> dict[str, str]:
    filename = artifact_path.name
    expected = entries.get(filename)
    if expected is None:
        raise AppError(
            f"Release manifest does not contain {filename}",
            code="artifact_not_in_release_manifest",
            details={"artifact": filename, "manifest": manifest_name},
        )
    actual = sha256_file(artifact_path)
    if not hmac.compare_digest(actual, expected):
        raise AppError(
            f"SHA-256 mismatch for {filename}",
            code="release_artifact_hash_mismatch",
            hint="Do not install or run this file. Delete it and download the release again.",
            details={"artifact": filename, "expected_sha256": expected, "actual_sha256": actual},
        )
    return {"artifact": filename, "sha256": actual}


def _verify_release_artifact_entries(
    release_dir: Path,
    entries: dict[str, str],
    manifest_name: str,
    *,
    require_complete: bool,
) -> dict[str, object]:
    if release_dir.is_symlink() or not release_dir.is_dir():
        raise AppError(
            f"Release directory does not exist: {release_dir}",
            code="release_verification_file_error",
        )

    metadata_names = {manifest_name, f"{manifest_name}.asc"}
    actual_names: set[str] = set()
    try:
        candidates = list(release_dir.iterdir())
    except OSError as exc:
        raise AppError(
            f"Could not inspect release directory: {release_dir}",
            code="release_verification_file_error",
        ) from exc
    for candidate in candidates:
        if candidate.name in metadata_names:
            continue
        if candidate.is_symlink() or not candidate.is_file():
            raise AppError(
                f"Unexpected non-regular release asset: {candidate.name}",
                code="release_artifact_set_mismatch",
                hint="Do not publish this release.",
            )
        actual_names.add(candidate.name)

    expected_names = set(entries)
    missing = expected_names - actual_names
    unexpected = actual_names - expected_names
    if unexpected or (require_complete and missing) or not actual_names:
        raise AppError(
            (
                "Release assets do not exactly match the authenticated manifest"
                if require_complete
                else "Release assets are not a non-empty subset of the authenticated manifest"
            ),
            code="release_artifact_set_mismatch",
            hint="Do not publish this release.",
            details={
                "missing": sorted(missing),
                "unexpected": sorted(unexpected),
            },
        )

    for filename in sorted(actual_names):
        _verify_artifact_hash_entries(
            release_dir / filename,
            entries,
            manifest_name,
        )
    return {
        "verified": True,
        "artifact_count": len(actual_names),
        "complete": require_complete,
        "manifest": manifest_name,
    }


def verify_release_artifacts(
    release_dir: str | os.PathLike[str],
    manifest: str | os.PathLike[str],
    *,
    require_complete: bool = True,
) -> dict[str, object]:
    """Verify that a directory exactly matches a version-bound manifest.

    This authenticates hashes only. Production publication must call
    :func:`verify_release_directory`, which authenticates the manifest first.
    """

    release_path = Path(release_dir).expanduser()
    manifest_path = Path(manifest).expanduser()
    try:
        metadata_is_local = manifest_path.parent.resolve() == release_path.resolve()
    except OSError:
        metadata_is_local = False
    if not metadata_is_local:
        raise AppError(
            "Release manifest must be inside the release directory",
            code="release_artifact_set_mismatch",
        )
    manifest_bytes = _require_small_regular_file(
        manifest_path,
        limit=MAX_MANIFEST_BYTES,
        label="release manifest",
    )
    entries = _parse_release_manifest_bytes(
        manifest_bytes,
        source=str(manifest_path),
        expected_version=_release_version_from_manifest_name(manifest_path.name),
    )
    return _verify_release_artifact_entries(
        release_path,
        entries,
        manifest_path.name,
        require_complete=require_complete,
    )


def _gpg_command(executable: str | None) -> str:
    candidate = executable or shutil.which("gpg")
    if not candidate:
        raise AppError(
            "GnuPG is required to verify this OpenPGP release signature",
            code="gpg_unavailable",
            hint="Install GnuPG, then run kassiber verify-download again.",
        )
    return candidate


def _gpgv_command(gpg_executable: str) -> str:
    gpg_path = Path(gpg_executable)
    sibling_name = "gpgv.exe" if gpg_path.suffix.lower() == ".exe" else "gpgv"
    sibling = gpg_path.with_name(sibling_name)
    if sibling.is_file():
        return str(sibling)
    candidate = shutil.which(sibling_name)
    if not candidate:
        raise AppError(
            "GnuPG's gpgv verifier is required to verify this release signature",
            code="gpg_unavailable",
            hint="Install the complete GnuPG package, then run kassiber verify-download again.",
        )
    return candidate


def _run_gpg(command: list[str], *, operation: str) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            encoding="utf-8",
            errors="replace",
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise AppError(
            f"GnuPG could not {operation}",
            code="release_signature_verification_failed",
        ) from exc


def _primary_fingerprints(colon_output: str) -> set[str]:
    fingerprints: set[str] = set()
    awaiting_primary = False
    for line in colon_output.splitlines():
        fields = line.split(":")
        record_type = fields[0] if fields else ""
        if record_type == "pub":
            awaiting_primary = True
        elif record_type == "sub":
            awaiting_primary = False
        elif record_type == "fpr" and awaiting_primary and len(fields) > 9:
            fingerprints.add(fields[9].upper())
            awaiting_primary = False
    return fingerprints


def valid_signature_fingerprints(status_output: str) -> set[str]:
    fingerprints: set[str] = set()
    for line in status_output.splitlines():
        if not line.startswith("[GNUPG:] VALIDSIG "):
            continue
        fields = line.split()
        if len(fields) >= 3:
            fingerprints.add(fields[2].upper())
        # GnuPG appends the primary-key fingerprint for subkey signatures.
        if len(fields) >= 12 and _FINGERPRINT.fullmatch(fields[-1].upper()):
            fingerprints.add(fields[-1].upper())
    return fingerprints


def signature_status_has_failure(status_output: str) -> bool:
    for line in status_output.splitlines():
        if not line.startswith("[GNUPG:] "):
            continue
        fields = line.split()
        if len(fields) >= 2 and fields[1] in _REJECTED_SIGNATURE_STATUSES:
            return True
        if (
            len(fields) >= 10
            and fields[1] == "VALIDSIG"
            and fields[9] not in _ACCEPTED_OPENPGP_HASH_ALGORITHMS
        ):
            return True
    return False


def _verify_openpgp_signature_bytes(
    manifest_bytes: bytes,
    signature_bytes: bytes,
    public_key_bytes: bytes,
    expected_fingerprint: str,
    *,
    gpg_executable: str | None = None,
) -> str:
    expected = normalize_fingerprint(expected_fingerprint)
    gpg = _gpg_command(gpg_executable)
    gpgv = _gpgv_command(gpg)

    with tempfile.TemporaryDirectory(prefix="kassiber-gpg-") as temporary_home:
        home = Path(temporary_home)
        home.chmod(0o700)
        manifest_path = home / "release-manifest.txt"
        signature_path = home / "release-manifest.txt.asc"
        public_key_path = home / "release-public-key.asc"
        keyring_path = home / "release-public-key.gpg"
        manifest_path.write_bytes(manifest_bytes)
        signature_path.write_bytes(signature_bytes)
        public_key_path.write_bytes(public_key_bytes)
        common = [
            gpg,
            "--no-options",
            "--no-auto-key-retrieve",
            "--batch",
            "--no-tty",
            "--homedir",
            str(home),
        ]
        listed = _run_gpg(
            [
                *common,
                "--with-colons",
                "--fingerprint",
                "--show-keys",
                str(public_key_path),
            ],
            operation="inspect the release public key",
        )
        if listed.returncode != 0 or expected not in _primary_fingerprints(listed.stdout):
            raise AppError(
                "Release public key does not match the expected fingerprint",
                code="release_fingerprint_mismatch",
                hint="Obtain the public key and full fingerprint again from independent trusted sources.",
                details={"expected_fingerprint": expected},
            )
        dearmored = _run_gpg(
            [
                *common,
                "--yes",
                "--dearmor",
                "--output",
                str(keyring_path),
                str(public_key_path),
            ],
            operation="prepare the release public key",
        )
        if dearmored.returncode != 0:
            raise AppError(
                "Could not prepare the release public key",
                code="invalid_release_public_key",
            )
        verified = _run_gpg(
            [
                gpgv,
                "--status-fd",
                "1",
                "--keyring",
                str(keyring_path),
                str(signature_path),
                str(manifest_path),
            ],
            operation="verify the release signature",
        )
        valid_fingerprints = valid_signature_fingerprints(verified.stdout)
        if (
            verified.returncode != 0
            or signature_status_has_failure(verified.stdout)
            or expected not in valid_fingerprints
        ):
            raise AppError(
                "Release manifest has no valid signature from the expected Kassiber release key",
                code="release_signature_verification_failed",
                hint="Do not install or run the release artifact.",
                details={"expected_fingerprint": expected},
            )
    return expected


def verify_openpgp_signature(
    manifest: str | os.PathLike[str],
    signature: str | os.PathLike[str],
    public_key: str | os.PathLike[str],
    expected_fingerprint: str,
    *,
    gpg_executable: str | None = None,
) -> str:
    """Verify a detached signature in an isolated keyring, pinned by fingerprint."""

    manifest_path = Path(manifest).expanduser()
    signature_path = Path(signature).expanduser()
    public_key_path = Path(public_key).expanduser()
    manifest_bytes = _require_small_regular_file(
        manifest_path,
        limit=MAX_MANIFEST_BYTES,
        label="release manifest",
    )
    signature_bytes = _require_small_regular_file(
        signature_path,
        limit=MAX_SIGNATURE_BYTES,
        label="release signature",
    )
    public_key_bytes = _require_small_regular_file(
        public_key_path,
        limit=MAX_PUBLIC_KEY_BYTES,
        label="release public key",
    )
    _parse_release_manifest_bytes(
        manifest_bytes,
        source=str(manifest_path),
        expected_version=_release_version_from_manifest_name(manifest_path.name),
    )
    return _verify_openpgp_signature_bytes(
        manifest_bytes,
        signature_bytes,
        public_key_bytes,
        expected_fingerprint,
        gpg_executable=gpg_executable,
    )


def verify_download(
    artifact: str | os.PathLike[str],
    manifest: str | os.PathLike[str],
    signature: str | os.PathLike[str],
    public_key: str | os.PathLike[str],
    expected_fingerprint: str,
    *,
    gpg_executable: str | None = None,
) -> dict[str, object]:
    """Authenticate the manifest first, then verify the selected artifact hash."""

    manifest_path = Path(manifest).expanduser()
    signature_path = Path(signature).expanduser()
    public_key_path = Path(public_key).expanduser()
    manifest_bytes = _require_small_regular_file(
        manifest_path,
        limit=MAX_MANIFEST_BYTES,
        label="release manifest",
    )
    signature_bytes = _require_small_regular_file(
        signature_path,
        limit=MAX_SIGNATURE_BYTES,
        label="release signature",
    )
    public_key_bytes = _require_small_regular_file(
        public_key_path,
        limit=MAX_PUBLIC_KEY_BYTES,
        label="release public key",
    )
    signer = _verify_openpgp_signature_bytes(
        manifest_bytes,
        signature_bytes,
        public_key_bytes,
        expected_fingerprint,
        gpg_executable=gpg_executable,
    )
    entries = _parse_release_manifest_bytes(
        manifest_bytes,
        source=str(manifest_path),
        expected_version=_release_version_from_manifest_name(manifest_path.name),
    )
    artifact_result = _verify_artifact_hash_entries(
        Path(artifact).expanduser(),
        entries,
        manifest_path.name,
    )
    return {
        "verified": True,
        **artifact_result,
        "manifest": manifest_path.name,
        "signature": signature_path.name,
        "signer_fingerprint": signer,
    }


def verify_release_directory(
    release_dir: str | os.PathLike[str],
    manifest: str | os.PathLike[str],
    signature: str | os.PathLike[str],
    public_key: str | os.PathLike[str],
    expected_fingerprint: str,
    *,
    gpg_executable: str | None = None,
    require_complete: bool = True,
) -> dict[str, object]:
    """Authenticate a complete release set before any publication step."""

    release_path = Path(release_dir).expanduser()
    manifest_path = Path(manifest).expanduser()
    signature_path = Path(signature).expanduser()
    public_key_path = Path(public_key).expanduser()
    try:
        metadata_is_local = (
            manifest_path.parent.resolve() == release_path.resolve()
            and signature_path.parent.resolve() == release_path.resolve()
        )
    except OSError:
        metadata_is_local = False
    if not metadata_is_local:
        raise AppError(
            "Release manifest and signature must be inside the release directory",
            code="release_artifact_set_mismatch",
        )
    if signature_path.name != f"{manifest_path.name}.asc":
        raise AppError(
            "Release signature filename does not match the manifest",
            code="release_artifact_set_mismatch",
        )
    manifest_bytes = _require_small_regular_file(
        manifest_path,
        limit=MAX_MANIFEST_BYTES,
        label="release manifest",
    )
    signature_bytes = _require_small_regular_file(
        signature_path,
        limit=MAX_SIGNATURE_BYTES,
        label="release signature",
    )
    public_key_bytes = _require_small_regular_file(
        public_key_path,
        limit=MAX_PUBLIC_KEY_BYTES,
        label="release public key",
    )
    signer = _verify_openpgp_signature_bytes(
        manifest_bytes,
        signature_bytes,
        public_key_bytes,
        expected_fingerprint,
        gpg_executable=gpg_executable,
    )
    entries = _parse_release_manifest_bytes(
        manifest_bytes,
        source=str(manifest_path),
        expected_version=_release_version_from_manifest_name(manifest_path.name),
    )
    result = _verify_release_artifact_entries(
        release_path,
        entries,
        manifest_path.name,
        require_complete=require_complete,
    )
    return {
        **result,
        "signature": signature_path.name,
        "signer_fingerprint": signer,
    }


def generate_release_manifest(
    release_dir: str | os.PathLike[str],
    version: str,
    *,
    excluded_names: Iterable[str] = (),
) -> Path:
    """Create a stable manifest for the regular release artifacts in a directory."""

    directory = Path(release_dir).expanduser()
    if not directory.is_dir():
        raise AppError(
            f"Release directory does not exist: {directory}",
            code="release_verification_file_error",
        )
    output = directory / release_manifest_name(version)
    excluded = set(excluded_names) | {output.name, f"{output.name}.asc"}
    artifacts: list[Path] = []
    for candidate in directory.iterdir():
        if candidate.name in excluded:
            continue
        if not _MANIFEST_LINE.fullmatch(f"{'0' * 64}  {candidate.name}"):
            raise AppError(
                f"Release artifact filename is not manifest-safe: {candidate.name}",
                code="invalid_release_artifact_name",
            )
        if candidate.is_symlink() or not candidate.is_file():
            raise AppError(
                f"Release artifact is not a regular non-symlink file: {candidate.name}",
                code="release_verification_file_error",
            )
        artifacts.append(candidate)
    if not artifacts:
        raise AppError("No release artifacts found", code="empty_release_artifact_set")
    normalized_version = _release_version_from_manifest_name(output.name)
    lines = [
        f"{_MANIFEST_HEADER}\n",
        f"{_MANIFEST_VERSION_PREFIX}{normalized_version}\n",
        *[
            f"{sha256_file(path)}  {path.name}\n"
            for path in sorted(artifacts, key=lambda item: item.name)
        ],
    ]
    temporary = output.with_name(f".{output.name}.{os.getpid()}.tmp")
    try:
        with temporary.open("x", encoding="utf-8", newline="\n") as handle:
            handle.writelines(lines)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, output)
    except OSError as exc:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        raise AppError(
            f"Could not write release manifest: {output}",
            code="release_verification_file_error",
        ) from exc
    return output
