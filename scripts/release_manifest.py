#!/usr/bin/env python3
"""Generate, SSH-sign, and verify Sparrow-style Kassiber release manifests."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from kassiber.errors import AppError
from kassiber.release_verification import (
    MAX_MANIFEST_BYTES,
    MAX_PUBLIC_KEY_BYTES,
    MAX_SIGNATURE_BYTES,
    RELEASE_SIGNATURE_NAMESPACE,
    RELEASE_SIGNATURE_SUFFIX,
    _require_small_regular_file,
    generate_release_manifest,
    load_release_signing_policy,
    normalize_fingerprint,
    parse_release_manifest,
    ssh_keygen_command,
    ssh_keygen_output_is_unsupported,
    ssh_public_key_fingerprint,
    verify_release_artifacts,
    verify_release_directory,
    verify_signature_bytes,
)

DEFAULT_POLICY = ROOT / "packaging" / "release" / "signing-policy.json"
# Generous: the Bitwarden SSH agent waits for a per-use approval click.
SIGNING_TIMEOUT_SECONDS = 600


def _install_signature(temporary: Path, signature: Path, *, overwrite: bool) -> None:
    if overwrite:
        os.replace(temporary, signature)
        return
    try:
        # Atomic no-clobber publication: fails if the signature appeared meanwhile.
        os.link(temporary, signature)
    except FileExistsError as exc:
        raise AppError(
            f"Signature already exists: {signature}",
            code="release_signature_exists",
            hint="Pass --overwrite only when intentionally replacing this signature.",
        ) from exc
    except OSError as exc:
        raise AppError(
            f"Could not write release signature: {signature}",
            code="release_signing_failed",
        ) from exc


def sign_manifest(
    manifest: Path,
    public_key: Path,
    expected_fingerprint: str,
    *,
    signing_key: Path | None = None,
    output: Path | None = None,
    ssh_keygen_executable: str | None = None,
    overwrite: bool = False,
) -> Path:
    """Sign a manifest with the pinned SSH release key.

    ``public_key`` must hash to ``expected_fingerprint`` before anything is
    signed. By default ``ssh-keygen -Y sign`` receives that public key and
    asks the agent in ``SSH_AUTH_SOCK`` for the matching private key.
    ``signing_key`` exists for tests that sign with a throwaway private key.
    """

    expected = normalize_fingerprint(expected_fingerprint)
    public_key_bytes = _require_small_regular_file(
        public_key,
        limit=MAX_PUBLIC_KEY_BYTES,
        label="release public key",
    )
    if ssh_public_key_fingerprint(public_key_bytes) != expected:
        raise AppError(
            "Release public key does not match the pinned policy fingerprint",
            code="release_fingerprint_mismatch",
            hint="Select the Kassiber release public key named in the signing policy.",
            details={"expected_fingerprint": expected},
        )
    parse_release_manifest(manifest)
    manifest_bytes = _require_small_regular_file(
        manifest,
        limit=MAX_MANIFEST_BYTES,
        label="release manifest",
    )
    signature = output or manifest.with_name(f"{manifest.name}{RELEASE_SIGNATURE_SUFFIX}")
    if signature.exists() and not overwrite:
        raise AppError(
            f"Signature already exists: {signature}",
            code="release_signature_exists",
            hint="Pass --overwrite only when intentionally replacing this signature.",
        )
    ssh_keygen = ssh_keygen_command(ssh_keygen_executable)
    try:
        # The manifest goes over stdin and the signature comes back on stdout,
        # so ssh-keygen never names, prompts for, or overwrites a file itself.
        completed = subprocess.run(
            [
                ssh_keygen,
                "-Y",
                "sign",
                "-n",
                RELEASE_SIGNATURE_NAMESPACE,
                "-f",
                str(signing_key or public_key),
            ],
            input=manifest_bytes,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=SIGNING_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise AppError(
            "ssh-keygen could not sign the release manifest",
            code="release_signing_failed",
        ) from exc
    signature_bytes = completed.stdout or b""
    if completed.returncode != 0 or not signature_bytes:
        stderr = (completed.stderr or b"")[:4096].decode("utf-8", errors="replace")
        if ssh_keygen_output_is_unsupported(stderr):
            raise AppError(
                "This ssh-keygen cannot create OpenSSH signatures",
                code="ssh_keygen_unsupported",
                hint="Install OpenSSH 8.1 or newer.",
            )
        raise AppError(
            "ssh-keygen could not sign the release manifest",
            code="release_signing_failed",
            hint="Check that SSH_AUTH_SOCK points at the agent holding the release key and approve the request.",
        )
    if len(signature_bytes) > MAX_SIGNATURE_BYTES:
        raise AppError(
            "ssh-keygen produced an unexpectedly large signature",
            code="release_signing_failed",
        )
    try:
        verify_signature_bytes(
            manifest_bytes,
            signature_bytes,
            public_key_bytes,
            expected,
            ssh_keygen_executable=ssh_keygen,
        )
    except AppError as exc:
        raise AppError(
            "The new release signature did not verify against the pinned fingerprint",
            code="release_signing_failed",
            details={"expected_fingerprint": expected, "verification_error": exc.code},
        ) from exc

    temporary = signature.with_name(f".{signature.name}.{os.getpid()}.tmp")
    temporary.unlink(missing_ok=True)
    try:
        with temporary.open("xb") as handle:
            handle.write(signature_bytes)
            handle.flush()
            os.fsync(handle.fileno())
        _install_signature(temporary, signature, overwrite=overwrite)
    except OSError as exc:
        raise AppError(
            f"Could not write release signature: {signature}",
            code="release_signing_failed",
        ) from exc
    finally:
        temporary.unlink(missing_ok=True)
    return signature


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    generate = subparsers.add_parser("generate", help="Create a deterministic SHA-256 manifest")
    generate.add_argument("--release-dir", required=True, type=Path)
    generate.add_argument("--version", required=True)
    generate.add_argument("--exclude", action="append", default=[])

    sign = subparsers.add_parser(
        "sign",
        help="Create a detached OpenSSH signature with the release key from SSH_AUTH_SOCK",
    )
    sign.add_argument("--manifest", required=True, type=Path)
    sign.add_argument(
        "--public-key",
        required=True,
        type=Path,
        help="Release public key (.pub); its private key must be in the SSH agent",
    )
    sign.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    sign.add_argument("--output", type=Path)
    sign.add_argument("--ssh-keygen")
    sign.add_argument("--overwrite", action="store_true")

    verify_artifacts = subparsers.add_parser(
        "verify-artifacts",
        help="Verify that a release directory exactly matches its manifest",
    )
    verify_artifacts.add_argument("--release-dir", required=True, type=Path)
    verify_artifacts.add_argument("--manifest", required=True, type=Path)
    verify_artifacts.add_argument("--allow-subset", action="store_true")

    verify_release = subparsers.add_parser(
        "verify-release",
        help="Authenticate a signed manifest and every release artifact",
    )
    verify_release.add_argument("--release-dir", required=True, type=Path)
    verify_release.add_argument("--manifest", required=True, type=Path)
    verify_release.add_argument("--signature", required=True, type=Path)
    verify_release.add_argument("--public-key", required=True, type=Path)
    verify_release.add_argument("--fingerprint", required=True)
    verify_release.add_argument("--ssh-keygen")
    verify_release.add_argument("--allow-subset", action="store_true")

    policy = subparsers.add_parser(
        "policy",
        help="Validate and print the code-reviewed release signing policy",
    )
    policy.add_argument("--policy", required=True, type=Path)
    policy.add_argument("--require-enabled", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "generate":
            path = generate_release_manifest(
                args.release_dir,
                args.version,
                excluded_names=args.exclude,
            )
            payload = {"manifest": str(path), "entries": len(parse_release_manifest(path))}
        elif args.command == "sign":
            policy = load_release_signing_policy(
                args.policy,
                repository_root=ROOT,
                require_enabled=True,
            )
            fingerprint = str(policy["fingerprint"])
            path = sign_manifest(
                args.manifest,
                args.public_key.expanduser(),
                fingerprint,
                output=args.output,
                ssh_keygen_executable=args.ssh_keygen,
                overwrite=args.overwrite,
            )
            payload = {
                "manifest": str(args.manifest),
                "signature": str(path),
                "signer_fingerprint": fingerprint,
            }
        elif args.command == "verify-artifacts":
            payload = verify_release_artifacts(
                args.release_dir,
                args.manifest,
                require_complete=not args.allow_subset,
            )
        elif args.command == "verify-release":
            payload = verify_release_directory(
                args.release_dir,
                args.manifest,
                args.signature,
                args.public_key,
                args.fingerprint,
                ssh_keygen_executable=args.ssh_keygen,
                require_complete=not args.allow_subset,
            )
        else:
            payload = load_release_signing_policy(
                args.policy,
                repository_root=ROOT,
                require_enabled=args.require_enabled,
            )
    except AppError as exc:
        print(
            json.dumps(
                {
                    "ok": False,
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "hint": exc.hint,
                        "details": exc.details,
                    },
                }
            ),
            file=sys.stderr,
        )
        return 1
    print(json.dumps({"ok": True, **payload}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
