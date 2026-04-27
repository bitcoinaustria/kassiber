"""Read CLI secret-bearing values without going through argv.

Two safe paths:

- `--<name>-stdin` — read the value from the regular `stdin` channel.
  The reader strips a single trailing newline, rejects empty input, and
  rejects NUL bytes. Only one stdin-consuming option may be set per
  invocation.
- `--<name>-fd <FD>` — read the value from a numbered file descriptor
  the parent process opened. The reader closes the fd after consuming
  it. Multiple `*-fd` options may coexist on the same invocation.

The argv-value form `--<name> <value>` is intentionally discouraged
because shell history and process listings preserve it.
"""

from __future__ import annotations

import argparse
import sys
from typing import Iterable, Optional

from ..errors import AppError
from .prompt import read_passphrase_from_fd, validate_passphrase


def _read_stdin_payload(label: str) -> str:
    if sys.stdin is None:
        raise AppError(
            f"{label} requested via --*-stdin but no stdin is attached",
            code="invalid_secret_input",
            retryable=False,
        )
    raw = sys.stdin.read()
    if raw.endswith("\n"):
        raw = raw[:-1]
    if raw.endswith("\r"):
        raw = raw[:-1]
    if not raw:
        raise AppError(
            f"{label} value read from stdin is empty",
            code="invalid_secret_input",
            retryable=False,
        )
    if "\x00" in raw:
        raise AppError(
            f"{label} value contains NUL bytes",
            code="invalid_secret_input",
            retryable=False,
        )
    return raw


def add_secret_stdin_options(
    parser: argparse.ArgumentParser,
    base_name: str,
    *,
    label: Optional[str] = None,
) -> None:
    """Attach `--<name>-stdin` and `--<name>-fd FD` to `parser`.

    `base_name` uses the dashed form, e.g. `"token"` produces flags
    `--token-stdin` and `--token-fd FD`. The stored argparse `dest`
    values are `<name>_stdin` (bool) and `<name>_fd` (int|None) so that
    handlers can read them with `getattr(args, "<name>_stdin", False)`.
    """

    label = label or base_name.replace("-", " ")
    snake = base_name.replace("-", "_")
    parser.add_argument(
        f"--{base_name}-stdin",
        dest=f"{snake}_stdin",
        action="store_true",
        help=f"Read the {label} value from stdin (preferred over `--{base_name} <value>`)",
    )
    parser.add_argument(
        f"--{base_name}-fd",
        dest=f"{snake}_fd",
        type=int,
        default=None,
        metavar="FD",
        help=f"Read the {label} value from this open file descriptor",
    )


def enforce_single_stdin_consumer(
    args: argparse.Namespace,
    candidates: Iterable[str],
) -> None:
    """Reject simultaneous use of multiple `--*-stdin` options.

    `candidates` lists the snake-cased dest prefixes (e.g. `"token"`)
    whose `<name>_stdin` flag would consume normal stdin.
    """

    active = [c for c in candidates if getattr(args, f"{c}_stdin", False)]
    if len(active) > 1:
        raise AppError(
            "only one --*-stdin option may be used per invocation",
            code="invalid_secret_input",
            details={"conflicting": active},
            hint="Use --<name>-fd FD for additional secret inputs.",
            retryable=False,
        )


def read_secret_from_args(
    args: argparse.Namespace,
    base_name: str,
    *,
    legacy_attr: Optional[str] = None,
    label: Optional[str] = None,
) -> Optional[str]:
    """Resolve a secret value for `args.<base_name>` from safe channels.

    Lookup order:

    1. `--<name>-fd FD` if set.
    2. `--<name>-stdin` if set.
    3. The legacy argv attribute (deprecated; emits a warning to stderr).
    """

    snake = base_name.replace("-", "_")
    label = label or base_name.replace("-", " ")
    fd = getattr(args, f"{snake}_fd", None)
    if fd is not None:
        return read_passphrase_from_fd(int(fd))
    if getattr(args, f"{snake}_stdin", False):
        value = _read_stdin_payload(label)
        return validate_passphrase(value)
    legacy = legacy_attr or snake
    legacy_value = getattr(args, legacy, None)
    if legacy_value not in (None, ""):
        sys.stderr.write(
            f"warning: --{base_name} <value> exposes secrets in shell history; "
            f"prefer --{base_name}-stdin or --{base_name}-fd FD.\n"
        )
        return str(legacy_value)
    return None
