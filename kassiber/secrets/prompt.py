"""Read passphrases from a controlling TTY or a numbered file descriptor.

Argv and the process environment are deliberately never read here. The
two supported channels are:

- An interactive `getpass()` prompt that talks directly to the controlling
  terminal so it does not collide with a `--token-stdin` payload on the
  same invocation.
- A numbered file descriptor, opened by the parent process, that delivers
  raw UTF-8 bytes. The reader strips at most one trailing newline so a
  shell redirect like `3< /tmp/secret` works without touching the file.
"""

from __future__ import annotations

import getpass
import os
import sys
from typing import Optional

from ..errors import AppError


_MAX_PASSPHRASE_BYTES = 8192


class PassphraseInputError(AppError):
    """Raised when a passphrase cannot be read from the requested channel."""

    def __init__(self, message: str, *, hint: Optional[str] = None) -> None:
        super().__init__(message, code="invalid_passphrase", hint=hint, retryable=False)


def validate_passphrase(value: str) -> str:
    """Normalize a freshly-read passphrase and reject the impossible cases."""

    if value is None:
        raise PassphraseInputError("passphrase must be a string")
    if not isinstance(value, str):
        raise PassphraseInputError("passphrase must be a string")
    if value == "":
        raise PassphraseInputError("passphrase must not be empty")
    if "\x00" in value:
        raise PassphraseInputError("passphrase must not contain NUL bytes")
    return value


def read_passphrase_from_fd(fd: int) -> str:
    """Read a passphrase from an already-open numeric file descriptor.

    The parent process is expected to have written the passphrase as raw
    UTF-8 bytes and closed its end of the pipe. We accept up to
    `_MAX_PASSPHRASE_BYTES` and strip exactly one trailing `\\n` so files
    produced with `echo` or `printf '...\\n'` work without surprises.
    """

    if not isinstance(fd, int) or fd < 0:
        raise PassphraseInputError(
            f"--db-passphrase-fd expects a non-negative integer, got {fd!r}",
        )
    chunks: list[bytes] = []
    remaining = _MAX_PASSPHRASE_BYTES + 1
    try:
        while remaining > 0:
            block = os.read(fd, min(4096, remaining))
            if not block:
                break
            chunks.append(block)
            remaining -= len(block)
    except OSError as exc:
        raise PassphraseInputError(
            f"could not read passphrase from fd {fd}: {exc}",
            hint="Make sure the parent process opens the fd before exec.",
        ) from None
    finally:
        try:
            os.close(fd)
        except OSError:
            # Best-effort fd cleanup; we deliberately swallow close
            # failures so the original read error (if any) is what the
            # caller sees rather than a noisy double exception.
            pass

    raw = b"".join(chunks)
    if len(raw) > _MAX_PASSPHRASE_BYTES:
        raise PassphraseInputError(
            f"passphrase on fd {fd} exceeds {_MAX_PASSPHRASE_BYTES}-byte limit",
        )
    if raw.endswith(b"\n"):
        raw = raw[:-1]
    if raw.endswith(b"\r"):
        raw = raw[:-1]
    try:
        decoded = raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise PassphraseInputError(
            f"passphrase on fd {fd} is not valid UTF-8: {exc}",
        ) from None
    return validate_passphrase(decoded)


def _open_tty_for_prompt() -> Optional[tuple[object, object]]:
    """Return `(prompt_stream, input_stream)` bound to the controlling TTY.

    Falling back to stdout/stdin would let `--token-stdin` payloads collide
    with the passphrase prompt, so we only succeed when /dev/tty is real.
    """

    try:
        prompt = open("/dev/tty", "w", encoding="utf-8")
    except OSError:
        return None
    try:
        source = open("/dev/tty", "r", encoding="utf-8")
    except OSError:
        prompt.close()
        return None
    return prompt, source


def prompt_passphrase(label: str = "Database passphrase: ") -> str:
    """Prompt the user once for a passphrase via the controlling TTY."""

    streams = _open_tty_for_prompt()
    if streams is None:
        if sys.stdin.isatty() and sys.stderr.isatty():
            value = getpass.getpass(label, stream=sys.stderr)
            return validate_passphrase(value)
        raise PassphraseInputError(
            "no TTY available for passphrase prompt",
            hint="Run interactively or pass --db-passphrase-fd from a controlling process.",
        )
    prompt_stream, input_stream = streams
    try:
        value = getpass.getpass(label, stream=prompt_stream)
    finally:
        prompt_stream.close()
        input_stream.close()
    return validate_passphrase(value)


def prompt_passphrase_with_confirmation(
    label: str = "New database passphrase: ",
    confirm_label: str = "Confirm passphrase: ",
) -> str:
    """Prompt for a passphrase twice; refuse to continue when they differ."""

    first = prompt_passphrase(label)
    second = prompt_passphrase(confirm_label)
    if first != second:
        raise PassphraseInputError(
            "passphrases do not match",
            hint="Re-run the command and re-enter the passphrase carefully.",
        )
    return first
