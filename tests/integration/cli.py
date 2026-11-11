"""Start the Rust `kassiber` binary from the harness (standard library only).

Every integration lane, regtest seed and live test that runs the CLI or
`kassiber daemon` as a child goes through `run` or `popen` here. The binary
is the first of:

- `KASSIBER_TEST_CLI=/path/to/kassiber`, or a JSON argv prefix such as
  `'["/path/to/kassiber", "--flag"]'`, run without a shell;
- `KASSIBER_RUST_BIN=/path/to/kassiber`.

`scripts/integration-harness.sh` exports both. There is no Python fallback:
the harness drives only the Rust product (docs/plan/22-python-removal.md,
decision D2). The working directory defaults to the repository root.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CLI_ENV = "KASSIBER_TEST_CLI"
BIN_ENV = "KASSIBER_RUST_BIN"


class CandidateConfigError(RuntimeError):
    """Neither variable names a runnable `kassiber` executable."""


@dataclass(frozen=True)
class InputFd:
    """Synthetic bytes the child reads from an inherited descriptor.

    In an argument list it becomes the number of a pipe holding `data`, for
    flags such as `--db-passphrase-fd` or `--username-fd`.
    """

    data: bytes


def parse_candidate(value: str, *, name: str = CLI_ENV) -> tuple[str, ...]:
    """The argv prefix a `KASSIBER_TEST_CLI` value names."""
    text = value.strip()
    if not text:
        raise CandidateConfigError(f"{name} is empty")
    if text.startswith("["):
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError as exc:
            raise CandidateConfigError(f"{name} is not a JSON argv list: {exc}") from exc
        if not parsed or not all(isinstance(part, str) and part for part in parsed):
            raise CandidateConfigError(f"{name} must be a non-empty JSON list of strings")
        head = tuple(parsed)
    else:
        head = (text,)
    executable = head[0]
    resolved = executable if os.path.isabs(executable) else shutil.which(executable)
    if not resolved or not os.path.isfile(resolved) or not os.access(resolved, os.X_OK):
        raise CandidateConfigError(f"{name} executable is not runnable: {executable!r}")
    return (resolved, *head[1:])


def prefix() -> tuple[str, ...]:
    """The argv prefix that starts the product."""
    for name in (CLI_ENV, BIN_ENV):
        value = os.environ.get(name)
        if value and value.strip():
            return parse_candidate(value, name=name)
    raise CandidateConfigError(
        f"set {CLI_ENV} or {BIN_ENV} to the Rust kassiber binary "
        "(scripts/integration-harness.sh exports both)"
    )


def binary() -> str:
    """The executable `prefix()` starts."""
    return prefix()[0]


def argv(args: Iterable[str | os.PathLike[str] | InputFd]) -> list[str]:
    """The full command line for `args`, for messages and inspection."""
    return [*prefix(), *("<fd>" if isinstance(arg, InputFd) else os.fspath(arg) for arg in args)]


def _with_input_fds(args, kwargs) -> tuple[list[str], list[int]]:
    """Open a pipe per `InputFd`; return the arguments and the read ends."""
    resolved, read_ends = [], []
    for arg in args:
        if not isinstance(arg, InputFd):
            resolved.append(os.fspath(arg))
            continue
        read_end, write_end = os.pipe()
        try:
            os.write(write_end, arg.data)
        finally:
            os.close(write_end)
        read_ends.append(read_end)
        resolved.append(str(read_end))
    if read_ends:
        kwargs["pass_fds"] = (*kwargs.get("pass_fds", ()), *read_ends)
    return resolved, read_ends


def run(args: Iterable[str | os.PathLike[str] | InputFd], **kwargs) -> subprocess.CompletedProcess:
    """`subprocess.run` of the product with `args`; keyword arguments pass through."""
    head = prefix()
    product_args, read_ends = _with_input_fds(list(args), kwargs)
    kwargs.setdefault("cwd", ROOT)
    try:
        return subprocess.run([*head, *product_args], **kwargs)
    finally:
        for read_end in read_ends:
            os.close(read_end)


def popen(args: Iterable[str | os.PathLike[str] | InputFd], **kwargs) -> subprocess.Popen:
    """`subprocess.Popen` of the product with `args`, such as a daemon session."""
    head = prefix()
    product_args, read_ends = _with_input_fds(list(args), kwargs)
    kwargs.setdefault("cwd", ROOT)
    try:
        return subprocess.Popen([*head, *product_args], **kwargs)
    finally:
        for read_end in read_ends:
            os.close(read_end)
