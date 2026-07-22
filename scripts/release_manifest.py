#!/usr/bin/env python3
"""Generate and offline-sign Sparrow-style Kassiber release manifests."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from kassiber.errors import AppError
from kassiber.release_verification import (
    generate_release_manifest,
    load_release_signing_policy,
    normalize_fingerprint,
    parse_release_manifest,
    signature_status_has_failure,
    valid_signature_fingerprints,
    verify_release_artifacts,
    verify_release_directory,
)


def _gpg_executable(value: str | None) -> str:
    executable = value or shutil.which("gpg")
    if not executable:
        raise AppError(
            "GnuPG is required to sign a release manifest",
            code="gpg_unavailable",
        )
    return executable


def _run_capture(command: list[str]) -> subprocess.CompletedProcess[str]:
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
            "GnuPG could not inspect the release signing key",
            code="release_signing_failed",
        ) from exc


def sign_manifest(
    manifest: Path,
    fingerprint: str,
    *,
    output: Path | None = None,
    gpg_executable: str | None = None,
    overwrite: bool = False,
) -> Path:
    """Sign a manifest with an exact full-fingerprint key selection."""

    expected = normalize_fingerprint(fingerprint)
    parse_release_manifest(manifest)
    signature = output or manifest.with_name(f"{manifest.name}.asc")
    if signature.exists() and not overwrite:
        raise AppError(
            f"Signature already exists: {signature}",
            code="release_signature_exists",
            hint="Pass --overwrite only when intentionally replacing this signature.",
        )
    gpg = _gpg_executable(gpg_executable)
    temporary = signature.with_name(f".{signature.name}.{os.getpid()}.tmp")
    temporary.unlink(missing_ok=True)
    try:
        completed = subprocess.run(
            [
                gpg,
                "--armor",
                "--detach-sign",
                "--digest-algo",
                "SHA512",
                "--local-user",
                expected,
                "--output",
                str(temporary),
                str(manifest),
            ],
            check=False,
        )
        if completed.returncode != 0:
            raise AppError(
                "GnuPG could not sign the release manifest",
                code="release_signing_failed",
            )
        verified = _run_capture(
            [
                gpg,
                "--batch",
                "--no-auto-key-retrieve",
                "--status-fd",
                "1",
                "--verify",
                str(temporary),
                str(manifest),
            ]
        )
        if (
            verified.returncode != 0
            or signature_status_has_failure(verified.stdout)
            or expected not in valid_signature_fingerprints(verified.stdout)
        ):
            raise AppError(
                "The new release signature did not verify against the expected fingerprint",
                code="release_signing_failed",
                details={"expected_fingerprint": expected},
            )
        os.replace(temporary, signature)
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

    sign = subparsers.add_parser("sign", help="Create an ASCII-armored detached OpenPGP signature")
    sign.add_argument("--manifest", required=True, type=Path)
    sign.add_argument("--fingerprint", required=True)
    sign.add_argument("--output", type=Path)
    sign.add_argument("--gpg")
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
    verify_release.add_argument("--gpg")
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
            path = sign_manifest(
                args.manifest,
                args.fingerprint,
                output=args.output,
                gpg_executable=args.gpg,
                overwrite=args.overwrite,
            )
            payload = {
                "manifest": str(args.manifest),
                "signature": str(path),
                "signer_fingerprint": normalize_fingerprint(args.fingerprint),
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
                gpg_executable=args.gpg,
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
