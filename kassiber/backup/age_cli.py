"""Encrypt / decrypt a backup stream through age.

We prefer the `age` (or `rage`) binary on `PATH` because it has a stable
streaming interface that handles arbitrary archive sizes without
buffering. When no binary is available we fall back to the in-memory
`pyrage` library, but only for passphrase-mode flows; recipient-mode
backups must use the binary so that the streaming behavior the plan
calls out is preserved.

The backend is chosen lazily so test environments without `age` can still
exercise the round-trip via `pyrage`.
"""

from __future__ import annotations

import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Iterable, Optional

from ..errors import AppError


class AgeUnavailableError(AppError):
    def __init__(self, message: str, *, hint: Optional[str] = None) -> None:
        super().__init__(message, code="age_unavailable", hint=hint, retryable=False)


@dataclass
class AgeBackend:
    """Resolved age backend description."""

    flavor: str  # "age", "rage", or "pyrage"
    binary_path: Optional[str] = None


def select_age_backend(
    prefer_binary: bool = True,
    *,
    mode: Optional[str] = None,
) -> AgeBackend:
    """Pick the best available age implementation.

    `mode="passphrase"` prefers `pyrage` even when an `age`/`rage`
    binary is on PATH, because `encrypt_age_stream`/`decrypt_age_stream`
    do not currently route passphrase-mode flows through the binary
    backends. Without this preference the default `kassiber backup
    export` (passphrase mode) would fail on hosts that happen to have
    `age` installed even though `pyrage` is also available.
    """

    if mode == "passphrase":
        try:
            import pyrage  # noqa: F401  - import probe only
        except ModuleNotFoundError:
            pass
        else:
            return AgeBackend(flavor="pyrage")

    if prefer_binary:
        for binary in ("age", "rage"):
            located = shutil.which(binary)
            if located:
                return AgeBackend(flavor=binary, binary_path=located)
    try:
        import pyrage  # noqa: F401  - import probe only
    except ModuleNotFoundError:
        raise AgeUnavailableError(
            "no age implementation available",
            hint="Install `age` or `rage` on PATH, or `pip install pyrage`.",
        ) from None
    return AgeBackend(flavor="pyrage")


def _spawn_age_process(
    backend: AgeBackend,
    args: Iterable[str],
    *,
    stdin: Optional[IO[bytes]] = None,
    stdout: Optional[IO[bytes]] = None,
    extra_env: Optional[dict] = None,
) -> subprocess.CompletedProcess:
    if backend.binary_path is None:
        raise AgeUnavailableError("no age binary configured for this backend")
    return subprocess.run(
        [backend.binary_path, *args],
        stdin=stdin,
        stdout=stdout,
        check=True,
        env=extra_env,
    )


def encrypt_age_stream(
    source: IO[bytes],
    destination: IO[bytes],
    *,
    passphrase: Optional[str] = None,
    recipients: Optional[Iterable[str]] = None,
    backend: Optional[AgeBackend] = None,
) -> None:
    """Read raw bytes from `source` and write age ciphertext into `destination`."""

    if (passphrase is None) == (recipients is None):
        raise AppError(
            "encrypt_age_stream requires exactly one of `passphrase` or `recipients`",
            code="invalid_age_call",
            retryable=False,
        )
    backend = backend or select_age_backend(
        mode="passphrase" if passphrase is not None else "recipient",
    )

    if backend.flavor in ("age", "rage"):
        args: list[str] = []
        if passphrase is not None:
            args.append("--passphrase")
        if recipients is not None:
            for r in recipients:
                args.extend(["--recipient", r])
        # Force a passphrase mode through stdin via a helper file; age's
        # `--passphrase` mode reads from /dev/tty by default, which is not
        # available in our subprocess.
        if passphrase is not None:
            # We use the documented `AGE_PASSPHRASE` env var by writing
            # the passphrase via a dedicated pipe instead of argv. This
            # avoids the secret showing up in `ps`.
            # `age` does not honor an env var, so emulate it by using `--passphrase`
            # in an environment with an inherited input pipe.
            raise AppError(
                "age binary passphrase mode is not safely supported via subprocess yet; install pyrage or use --recipient mode",
                code="age_passphrase_mode_unsupported",
                hint="Install pyrage (`pip install pyrage`) or use age recipient mode for now.",
                retryable=False,
            )
        _spawn_age_process(backend, args, stdin=source, stdout=destination)
        return

    # pyrage fallback (in-memory).
    payload = source.read()
    if passphrase is not None:
        import pyrage.passphrase

        ciphertext = pyrage.passphrase.encrypt(payload, passphrase)
    else:
        import pyrage
        from pyrage import x25519

        recipient_objs = [x25519.Recipient.from_str(r) for r in (recipients or ())]
        ciphertext = pyrage.encrypt(payload, recipient_objs)
    destination.write(ciphertext)


def decrypt_age_stream(
    source: IO[bytes],
    destination: IO[bytes],
    *,
    passphrase: Optional[str] = None,
    identity_file: Optional[Path] = None,
    backend: Optional[AgeBackend] = None,
) -> None:
    """Read age ciphertext from `source` and write plaintext into `destination`."""

    if (passphrase is None) == (identity_file is None):
        raise AppError(
            "decrypt_age_stream requires exactly one of `passphrase` or `identity_file`",
            code="invalid_age_call",
            retryable=False,
        )
    backend = backend or select_age_backend(
        mode="passphrase" if passphrase is not None else "recipient",
    )

    if backend.flavor in ("age", "rage"):
        args: list[str] = ["--decrypt"]
        if identity_file is not None:
            args.extend(["--identity", str(identity_file)])
        if passphrase is not None:
            raise AppError(
                "age binary passphrase mode is not safely supported via subprocess yet; install pyrage",
                code="age_passphrase_mode_unsupported",
                retryable=False,
            )
        _spawn_age_process(backend, args, stdin=source, stdout=destination)
        return

    payload = source.read()
    if passphrase is not None:
        import pyrage.passphrase

        try:
            plaintext = pyrage.passphrase.decrypt(payload, passphrase)
        except Exception as exc:
            raise AppError(
                f"age passphrase decryption failed: {exc}",
                code="age_decrypt_failed",
                retryable=False,
            ) from None
    else:
        import pyrage
        from pyrage import ssh, x25519

        with open(identity_file, "r", encoding="utf-8") as handle:
            identity_text = handle.read()
        identities = []
        try:
            identities.append(x25519.Identity.from_str(identity_text.strip()))
        except Exception:
            identities.append(ssh.Identity.from_str(identity_text))
        try:
            plaintext = pyrage.decrypt(payload, identities)
        except Exception as exc:
            raise AppError(
                f"age identity decryption failed: {exc}",
                code="age_decrypt_failed",
                retryable=False,
            ) from None
    destination.write(plaintext)
