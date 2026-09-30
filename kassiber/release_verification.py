"""Sparrow-style release manifest verification with OpenSSH signatures.

Kassiber signs one deterministic SHA-256 manifest rather than signing each
package separately.  A detached OpenSSH signature (``ssh-keygen -Y sign``)
authenticates the manifest; the manifest then authenticates the selected
release artifact.

The verifier requires an expected full SHA256 key fingerprint and compares it
to the supplied public key in Python before it runs ``ssh-keygen``.  It never
passes a caller-supplied ``allowed_signers`` file to ``ssh-keygen``: it writes
one canonical entry for the pinned key, principal, and namespace into a private
temporary directory.  A public key shipped next to a download is therefore
never a trust decision by itself.
"""

from __future__ import annotations

import base64
import binascii
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
MAX_SIGNATURE_BYTES = 16 * 1024
MAX_PUBLIC_KEY_BYTES = 16 * 1024
MAX_SIGNING_POLICY_BYTES = 64 * 1024
MAX_SSH_KEYGEN_OUTPUT_BYTES = 16 * 1024
SIGNING_POLICY_SCHEMA_VERSION = 2
RELEASE_SIGNATURE_NAMESPACE = "kassiber-release"
RELEASE_SIGNER_PRINCIPAL = "release@kassiber"
RELEASE_SIGNATURE_SUFFIX = ".sig"
MINIMUM_OPENSSH_VERSION = "8.1"
SSH_KEYGEN_TIMEOUT_SECONDS = 30
_MANIFEST_LINE = re.compile(
    r"(?P<sha256>[0-9a-f]{64})  (?P<filename>[A-Za-z0-9][A-Za-z0-9._+-]{0,254})"
)
_FINGERPRINT = re.compile(r"SHA256:[A-Za-z0-9+/]{43}")
_MANIFEST_HEADER = "# Kassiber release manifest v1"
_MANIFEST_VERSION_PREFIX = "# Version: "
_SSH_SIGNATURE_HEADER = b"-----BEGIN SSH SIGNATURE-----"
# Ed25519 now; an ed25519-sk hardware key later needs only new key material.
_ACCEPTED_SSH_KEY_TYPES = frozenset({"ssh-ed25519", "sk-ssh-ed25519@openssh.com"})
_ED25519_PUBLIC_KEY_BYTES = 32
_GOOD_SIGNATURE = re.compile(
    r"Good "
    + re.escape(f'"{RELEASE_SIGNATURE_NAMESPACE}"')
    + r" signature for "
    + re.escape(RELEASE_SIGNER_PRINCIPAL)
    + r" with [A-Z0-9-]+ key (?P<fingerprint>SHA256:[A-Za-z0-9+/]{43})"
)
# ssh-keygen releases before 8.1 reject -Y as an unknown option and print usage.
_UNSUPPORTED_SSH_KEYGEN = re.compile(
    r"(?:unknown|illegal|invalid|unrecognized) option|usage: ssh-keygen",
    re.IGNORECASE,
)


def normalize_fingerprint(value: str) -> str:
    """Require a complete OpenSSH ``SHA256:`` key fingerprint."""

    normalized = value.strip() if isinstance(value, str) else ""
    if not _FINGERPRINT.fullmatch(normalized):
        raise AppError(
            "Release-key fingerprint must be a complete OpenSSH SHA256 fingerprint",
            code="invalid_release_fingerprint",
            hint=(
                "Copy the complete SHA256:... Kassiber release-key fingerprint "
                "from an independent trusted source."
            ),
        )
    return normalized


def _read_ssh_string(blob: bytes, offset: int) -> tuple[bytes, int]:
    if offset + 4 > len(blob):
        raise ValueError("truncated SSH key length")
    length = int.from_bytes(blob[offset : offset + 4], "big")
    start = offset + 4
    end = start + length
    if end > len(blob):
        raise ValueError("truncated SSH key field")
    return blob[start:end], end


def _parse_key_fields(key_type: str, encoded: str) -> tuple[str, bytes]:
    if key_type not in _ACCEPTED_SSH_KEY_TYPES:
        raise ValueError("unsupported SSH key type")
    try:
        blob = base64.b64decode(encoded.encode("ascii"), validate=True)
    except (UnicodeEncodeError, binascii.Error) as exc:
        raise ValueError("invalid SSH key encoding") from exc
    fields: list[bytes] = []
    offset = 0
    while offset < len(blob):
        field, offset = _read_ssh_string(blob, offset)
        fields.append(field)
    expected_fields = 2 if key_type == "ssh-ed25519" else 3
    if (
        len(fields) != expected_fields
        or fields[0] != key_type.encode("ascii")
        or len(fields[1]) != _ED25519_PUBLIC_KEY_BYTES
        or (expected_fields == 3 and not fields[2].startswith(b"ssh:"))
    ):
        raise ValueError("malformed SSH key")
    return key_type, blob


def _single_key_line(raw: bytes, *, label: str, code: str) -> str:
    try:
        text = raw.decode("ascii")
    except UnicodeDecodeError as exc:
        raise AppError(
            f"{label.capitalize()} is not an OpenSSH text file",
            code=code,
        ) from exc
    lines = [
        line.strip()
        for line in text.splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    if len(lines) != 1:
        raise AppError(
            f"{label.capitalize()} must contain exactly one key entry",
            code=code,
        )
    return lines[0]


def parse_ssh_public_key(raw: bytes) -> tuple[str, bytes]:
    """Parse one ``<type> <base64> [comment]`` OpenSSH public-key line."""

    tokens = _single_key_line(
        raw, label="release public key", code="invalid_release_public_key"
    ).split()
    if len(tokens) < 2:
        raise AppError(
            "Release public key is not an OpenSSH public key",
            code="invalid_release_public_key",
        )
    try:
        return _parse_key_fields(tokens[0], tokens[1])
    except ValueError as exc:
        raise AppError(
            "Release public key must be one OpenSSH Ed25519 or Ed25519-SK public key",
            code="invalid_release_public_key",
        ) from exc


def ssh_key_fingerprint(blob: bytes) -> str:
    """Return the OpenSSH ``SHA256:`` fingerprint of a public-key blob."""

    digest = hashlib.sha256(blob).digest()
    return "SHA256:" + base64.b64encode(digest).decode("ascii").rstrip("=")


def ssh_public_key_fingerprint(raw: bytes) -> str:
    return ssh_key_fingerprint(parse_ssh_public_key(raw)[1])


def allowed_signers_entry(key_type: str, blob: bytes) -> str:
    """Render the only allowed_signers entry Kassiber trusts for releases."""

    encoded = base64.b64encode(blob).decode("ascii")
    return (
        f'{RELEASE_SIGNER_PRINCIPAL} namespaces="{RELEASE_SIGNATURE_NAMESPACE}" '
        f"{key_type} {encoded}\n"
    )


def _parse_allowed_signers(raw: bytes) -> tuple[str, bytes]:
    tokens = _single_key_line(
        raw, label="release allowed_signers file", code="invalid_release_signing_policy"
    ).split()
    if (
        len(tokens) < 4
        or tokens[0] != RELEASE_SIGNER_PRINCIPAL
        or tokens[1] != f'namespaces="{RELEASE_SIGNATURE_NAMESPACE}"'
    ):
        raise AppError(
            "Release allowed_signers entry must name only the release principal and namespace",
            code="invalid_release_signing_policy",
        )
    try:
        return _parse_key_fields(tokens[2], tokens[3])
    except ValueError as exc:
        raise AppError(
            "Release allowed_signers entry has no valid OpenSSH Ed25519 key",
            code="invalid_release_signing_policy",
        ) from exc


def _policy_path(root: Path, value: str, *, label: str) -> Path:
    relative = Path(value)
    if relative.is_absolute():
        raise AppError(
            f"Release {label} path must be repository-relative",
            code="invalid_release_signing_policy",
        )
    resolved = (root / relative).resolve()
    try:
        resolved.relative_to(root)
    except ValueError as exc:
        raise AppError(
            f"Release {label} path escapes the repository",
            code="invalid_release_signing_policy",
        ) from exc
    return resolved


def load_release_signing_policy(
    path: str | os.PathLike[str],
    *,
    repository_root: str | os.PathLike[str],
    require_enabled: bool = False,
) -> dict[str, object]:
    """Load the code-reviewed release trust root used by publication jobs.

    An enabled policy must pin a fingerprint that matches both the committed
    public key and the committed ``allowed_signers`` entry.
    """

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
    if not _has_exact_schema_version(payload, SIGNING_POLICY_SCHEMA_VERSION):
        raise AppError(
            "Release signing policy has no supported schema",
            code="invalid_release_signing_policy",
        )
    enabled = payload.get("enabled")
    fingerprint = payload.get("fingerprint")
    namespace = payload.get("namespace")
    principal = payload.get("principal")
    public_key_value = payload.get("public_key_path")
    allowed_signers_value = payload.get("allowed_signers_path")
    if (
        not isinstance(enabled, bool)
        or not isinstance(fingerprint, str)
        or not isinstance(public_key_value, str)
        or not public_key_value
        or not isinstance(allowed_signers_value, str)
        or not allowed_signers_value
    ):
        raise AppError(
            "Release signing policy fields are invalid",
            code="invalid_release_signing_policy",
        )
    if namespace != RELEASE_SIGNATURE_NAMESPACE or principal != RELEASE_SIGNER_PRINCIPAL:
        raise AppError(
            "Release signing policy must use the Kassiber release namespace and principal",
            code="invalid_release_signing_policy",
            details={
                "expected_namespace": RELEASE_SIGNATURE_NAMESPACE,
                "expected_principal": RELEASE_SIGNER_PRINCIPAL,
            },
        )
    root = Path(repository_root).expanduser().resolve()
    public_key_path = _policy_path(root, public_key_value, label="public key")
    allowed_signers_path = _policy_path(
        root, allowed_signers_value, label="allowed_signers"
    )
    result: dict[str, object] = {
        "enabled": enabled,
        "fingerprint": "",
        "namespace": RELEASE_SIGNATURE_NAMESPACE,
        "principal": RELEASE_SIGNER_PRINCIPAL,
        "public_key_path": str(public_key_path),
        "allowed_signers_path": str(allowed_signers_path),
    }
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
                hint="Publish the release key and enable the code-reviewed signing policy first.",
            )
        return result

    normalized = normalize_fingerprint(fingerprint)
    public_key_type, public_key_blob = parse_ssh_public_key(
        _require_small_regular_file(
            public_key_path,
            limit=MAX_PUBLIC_KEY_BYTES,
            label="release public key",
        )
    )
    if not hmac.compare_digest(ssh_key_fingerprint(public_key_blob), normalized):
        raise AppError(
            "Release public key does not match the pinned policy fingerprint",
            code="invalid_release_signing_policy",
            details={"expected_fingerprint": normalized},
        )
    allowed_type, allowed_blob = _parse_allowed_signers(
        _require_small_regular_file(
            allowed_signers_path,
            limit=MAX_PUBLIC_KEY_BYTES,
            label="release allowed_signers file",
        )
    )
    if allowed_type != public_key_type or not hmac.compare_digest(
        allowed_blob, public_key_blob
    ):
        raise AppError(
            "Release allowed_signers key does not match the pinned public key",
            code="invalid_release_signing_policy",
            details={"expected_fingerprint": normalized},
        )
    result["fingerprint"] = normalized
    return result


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

    metadata_names = {manifest_name, f"{manifest_name}{RELEASE_SIGNATURE_SUFFIX}"}
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


def ssh_keygen_command(executable: str | None = None) -> str:
    candidate = executable or shutil.which("ssh-keygen")
    if not candidate:
        raise AppError(
            "OpenSSH's ssh-keygen is required to verify this release signature",
            code="ssh_keygen_unavailable",
            hint=(
                f"Install OpenSSH {MINIMUM_OPENSSH_VERSION} or newer, "
                "then run kassiber verify-download again."
            ),
        )
    return candidate


def _bounded_text(value: bytes | None) -> str:
    return (value or b"")[: MAX_SSH_KEYGEN_OUTPUT_BYTES].decode("utf-8", errors="replace")


def ssh_keygen_output_is_unsupported(output: str) -> bool:
    """Whether ssh-keygen rejected ``-Y`` because it predates OpenSSH 8.1."""

    return _UNSUPPORTED_SSH_KEYGEN.search(output) is not None


def _unsupported_ssh_keygen_error() -> AppError:
    return AppError(
        f"This ssh-keygen cannot verify OpenSSH signatures (OpenSSH {MINIMUM_OPENSSH_VERSION} or newer is required)",
        code="ssh_keygen_unsupported",
        hint=(
            f"Install OpenSSH {MINIMUM_OPENSSH_VERSION} or newer, "
            "then run kassiber verify-download again."
        ),
    )


def verify_signature_bytes(
    manifest_bytes: bytes,
    signature_bytes: bytes,
    public_key_bytes: bytes,
    expected_fingerprint: str,
    *,
    ssh_keygen_executable: str | None = None,
) -> str:
    """Verify a detached OpenSSH signature over manifest bytes.

    The supplied key must hash to ``expected_fingerprint`` before
    ``ssh-keygen`` sees it; the fingerprint is the root of trust.
    """

    expected = normalize_fingerprint(expected_fingerprint)
    key_type, blob = parse_ssh_public_key(public_key_bytes)
    if not hmac.compare_digest(ssh_key_fingerprint(blob), expected):
        raise AppError(
            "Release public key does not match the expected fingerprint",
            code="release_fingerprint_mismatch",
            hint="Obtain the public key and full fingerprint again from independent trusted sources.",
            details={"expected_fingerprint": expected},
        )
    if not signature_bytes.lstrip().startswith(_SSH_SIGNATURE_HEADER):
        raise AppError(
            "Release signature is not a detached OpenSSH signature",
            code="release_signature_verification_failed",
            hint="Do not install or run the release artifact.",
            details={"expected_fingerprint": expected},
        )
    ssh_keygen = ssh_keygen_command(ssh_keygen_executable)

    with tempfile.TemporaryDirectory(prefix="kassiber-sshsig-") as temporary:
        workspace = Path(temporary)
        workspace.chmod(0o700)
        allowed_signers_path = workspace / "allowed_signers"
        signature_path = workspace / f"release-manifest.txt{RELEASE_SIGNATURE_SUFFIX}"
        allowed_signers_path.write_text(
            allowed_signers_entry(key_type, blob),
            encoding="ascii",
            newline="\n",
        )
        signature_path.write_bytes(signature_bytes)
        try:
            completed = subprocess.run(
                [
                    ssh_keygen,
                    "-Y",
                    "verify",
                    "-f",
                    str(allowed_signers_path),
                    "-I",
                    RELEASE_SIGNER_PRINCIPAL,
                    "-n",
                    RELEASE_SIGNATURE_NAMESPACE,
                    "-s",
                    str(signature_path),
                ],
                input=manifest_bytes,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=SSH_KEYGEN_TIMEOUT_SECONDS,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise AppError(
                "ssh-keygen could not verify the release signature",
                code="release_signature_verification_failed",
                hint="Do not install or run the release artifact.",
                details={"expected_fingerprint": expected},
            ) from exc

    stdout = _bounded_text(completed.stdout)
    stderr = _bounded_text(completed.stderr)
    signed_by = {
        match.group("fingerprint")
        for line in stdout.splitlines()
        if (match := _GOOD_SIGNATURE.fullmatch(line.strip()))
    }
    if completed.returncode == 0 and signed_by == {expected}:
        return expected
    if completed.returncode != 0 and ssh_keygen_output_is_unsupported(f"{stdout}\n{stderr}"):
        raise _unsupported_ssh_keygen_error()
    raise AppError(
        "Release manifest has no valid signature from the expected Kassiber release key",
        code="release_signature_verification_failed",
        hint="Do not install or run the release artifact.",
        details={"expected_fingerprint": expected},
    )


def _read_signed_inputs(
    manifest_path: Path,
    signature_path: Path,
    public_key_path: Path,
) -> tuple[bytes, bytes, bytes]:
    return (
        _require_small_regular_file(
            manifest_path,
            limit=MAX_MANIFEST_BYTES,
            label="release manifest",
        ),
        _require_small_regular_file(
            signature_path,
            limit=MAX_SIGNATURE_BYTES,
            label="release signature",
        ),
        _require_small_regular_file(
            public_key_path,
            limit=MAX_PUBLIC_KEY_BYTES,
            label="release public key",
        ),
    )


def verify_manifest_signature(
    manifest: str | os.PathLike[str],
    signature: str | os.PathLike[str],
    public_key: str | os.PathLike[str],
    expected_fingerprint: str,
    *,
    ssh_keygen_executable: str | None = None,
) -> str:
    """Verify a detached manifest signature, pinned by key fingerprint."""

    manifest_path = Path(manifest).expanduser()
    manifest_bytes, signature_bytes, public_key_bytes = _read_signed_inputs(
        manifest_path,
        Path(signature).expanduser(),
        Path(public_key).expanduser(),
    )
    _parse_release_manifest_bytes(
        manifest_bytes,
        source=str(manifest_path),
        expected_version=_release_version_from_manifest_name(manifest_path.name),
    )
    return verify_signature_bytes(
        manifest_bytes,
        signature_bytes,
        public_key_bytes,
        expected_fingerprint,
        ssh_keygen_executable=ssh_keygen_executable,
    )


def _signature_result(signer: str) -> dict[str, str]:
    return {
        "signer_fingerprint": signer,
        "signer_principal": RELEASE_SIGNER_PRINCIPAL,
        "signature_namespace": RELEASE_SIGNATURE_NAMESPACE,
    }


def verify_download(
    artifact: str | os.PathLike[str],
    manifest: str | os.PathLike[str],
    signature: str | os.PathLike[str],
    public_key: str | os.PathLike[str],
    expected_fingerprint: str,
    *,
    ssh_keygen_executable: str | None = None,
) -> dict[str, object]:
    """Authenticate the manifest first, then verify the selected artifact hash."""

    manifest_path = Path(manifest).expanduser()
    signature_path = Path(signature).expanduser()
    manifest_bytes, signature_bytes, public_key_bytes = _read_signed_inputs(
        manifest_path,
        signature_path,
        Path(public_key).expanduser(),
    )
    signer = verify_signature_bytes(
        manifest_bytes,
        signature_bytes,
        public_key_bytes,
        expected_fingerprint,
        ssh_keygen_executable=ssh_keygen_executable,
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
        **_signature_result(signer),
    }


def verify_release_directory(
    release_dir: str | os.PathLike[str],
    manifest: str | os.PathLike[str],
    signature: str | os.PathLike[str],
    public_key: str | os.PathLike[str],
    expected_fingerprint: str,
    *,
    ssh_keygen_executable: str | None = None,
    require_complete: bool = True,
) -> dict[str, object]:
    """Authenticate a complete release set before any publication step."""

    release_path = Path(release_dir).expanduser()
    manifest_path = Path(manifest).expanduser()
    signature_path = Path(signature).expanduser()
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
    if signature_path.name != f"{manifest_path.name}{RELEASE_SIGNATURE_SUFFIX}":
        raise AppError(
            "Release signature filename does not match the manifest",
            code="release_artifact_set_mismatch",
        )
    manifest_bytes, signature_bytes, public_key_bytes = _read_signed_inputs(
        manifest_path,
        signature_path,
        Path(public_key).expanduser(),
    )
    signer = verify_signature_bytes(
        manifest_bytes,
        signature_bytes,
        public_key_bytes,
        expected_fingerprint,
        ssh_keygen_executable=ssh_keygen_executable,
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
        **_signature_result(signer),
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
    excluded = set(excluded_names) | {output.name, f"{output.name}{RELEASE_SIGNATURE_SUFFIX}"}
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
            # Preserve and report the original manifest write failure below.
            pass
        raise AppError(
            f"Could not write release manifest: {output}",
            code="release_verification_file_error",
        ) from exc
    return output
