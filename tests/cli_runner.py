"""Start the product as a child process: the one place tests build that command.

Every test, integration lane, and regtest seed that runs the CLI or
`kassiber daemon` as a subprocess goes through `run` or `popen` here, so one
switch can point all of them at another executable
(`tests/test_cli_runner_sites.py` keeps it that way):

- `KASSIBER_TEST_CLI` unset: `python -m kassiber` from this checkout.
- `KASSIBER_TEST_CLI=/path/to/kassiber`: that executable.
- `KASSIBER_TEST_CLI='["/path/to/kassiber", "--flag"]'`: a JSON argv prefix,
  run without a shell.

The working directory defaults to the repository root either way.

`KASSIBER_TEST_RECORD_SESSIONS=<dir>` also records each invocation at the
process boundary (`tests/session_harness/`); unset, nothing here touches the
recorder. Both variables are test-only: product code reads neither.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
CLI_ENV = "KASSIBER_TEST_CLI"
RECORD_ENV = "KASSIBER_TEST_RECORD_SESSIONS"
REFERENCE_PREFIX: tuple[str, ...] = (sys.executable, "-m", "kassiber")


class CandidateConfigError(RuntimeError):
    """`KASSIBER_TEST_CLI` does not name a runnable executable."""


@dataclass(frozen=True)
class InputFd:
    """Synthetic bytes the child reads from an inherited descriptor.

    In an argument list it becomes the number of a pipe holding `data`, for
    flags such as `--db-passphrase-fd`. Only synthetic test values belong
    here: the session recorder keeps them as fixture content.
    """

    data: bytes


def parse_candidate(value: str) -> tuple[str, ...]:
    """The argv prefix a `KASSIBER_TEST_CLI` value names."""
    text = value.strip()
    if not text:
        raise CandidateConfigError(f"{CLI_ENV} is empty")
    if text.startswith("["):
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError as exc:
            raise CandidateConfigError(f"{CLI_ENV} is not a JSON argv list: {exc}") from exc
        if not parsed or not all(isinstance(part, str) and part for part in parsed):
            raise CandidateConfigError(f"{CLI_ENV} must be a non-empty JSON list of strings")
        prefix = tuple(parsed)
    else:
        prefix = (text,)
    executable = prefix[0]
    resolved = executable if os.path.isabs(executable) else shutil.which(executable)
    if not resolved or not os.path.isfile(resolved) or not os.access(resolved, os.X_OK):
        raise CandidateConfigError(f"{CLI_ENV} executable is not runnable: {executable!r}")
    return (resolved, *prefix[1:])


def candidate_prefix() -> tuple[str, ...] | None:
    """The configured candidate's argv prefix, or None for the Python reference."""
    value = os.environ.get(CLI_ENV)
    if value is None or not value.strip():
        return None
    return parse_candidate(value)


def prefix(*, reference: bool = False, python_options: Sequence[str] = ()) -> tuple[str, ...]:
    """The argv prefix that starts the product.

    `reference=True` always runs this checkout's Python, for tests that pin
    the Python reference itself (interpreter flags, child hooks) rather than
    the product contract. Interpreter options need the reference.
    """
    if python_options and not reference:
        raise ValueError("python_options apply to the Python reference only; pass reference=True")
    if not reference:
        configured = candidate_prefix()
        if configured is not None:
            return configured
    return (sys.executable, *python_options, "-m", "kassiber")


def argv(
    args: Iterable[str | os.PathLike[str] | InputFd],
    *,
    reference: bool = False,
    python_options: Sequence[str] = (),
) -> list[str]:
    """The full command line for `args`, for messages and inspection."""
    return [
        *prefix(reference=reference, python_options=python_options),
        *("<fd>" if isinstance(arg, InputFd) else os.fspath(arg) for arg in args),
    ]


def _with_input_fds(args, kwargs) -> tuple[list[str], list[tuple[int, bytes]], list[int]]:
    """Open a pipe per `InputFd`; return the arguments, the inputs by index, and the read ends."""
    resolved, inputs, read_ends = [], [], []
    for index, arg in enumerate(args):
        if not isinstance(arg, InputFd):
            resolved.append(os.fspath(arg))
            continue
        read_end, write_end = os.pipe()
        try:
            os.write(write_end, arg.data)
        finally:
            os.close(write_end)
        read_ends.append(read_end)
        inputs.append((index, arg.data))
        resolved.append(str(read_end))
    if read_ends:
        kwargs["pass_fds"] = (*kwargs.get("pass_fds", ()), *read_ends)
    return resolved, inputs, read_ends


def run(
    args: Iterable[str | os.PathLike[str] | InputFd],
    *,
    reference: bool = False,
    python_options: Sequence[str] = (),
    secret_input: bool = False,
    **kwargs,
) -> subprocess.CompletedProcess:
    """`subprocess.run` of the product with `args`.

    Keyword arguments pass through to `subprocess.run`; `cwd` defaults to the
    repository root. `secret_input=True` marks stdin as a credential, which
    the session recorder refuses to keep.
    """
    head = prefix(reference=reference, python_options=python_options)
    product_args, inputs, read_ends = _with_input_fds(list(args), kwargs)
    kwargs.setdefault("cwd", ROOT)
    try:
        recorder = _recorder()
        if recorder is None:
            return subprocess.run([*head, *product_args], **kwargs)
        return recorder.run(
            [*head, *product_args],
            product_args=product_args,
            python_options=tuple(python_options),
            secret_input=secret_input,
            fd_inputs=inputs,
            **kwargs,
        )
    finally:
        for read_end in read_ends:
            os.close(read_end)


def popen(
    args: Iterable[str | os.PathLike[str] | InputFd],
    *,
    reference: bool = False,
    python_options: Sequence[str] = (),
    secret_input: bool = False,
    **kwargs,
) -> subprocess.Popen:
    """`subprocess.Popen` of the product with `args`, such as a daemon session.

    Keyword arguments pass through to `subprocess.Popen`; `cwd` defaults to
    the repository root.
    """
    head = prefix(reference=reference, python_options=python_options)
    product_args, inputs, read_ends = _with_input_fds(list(args), kwargs)
    kwargs.setdefault("cwd", ROOT)
    try:
        recorder = _recorder()
        if recorder is None:
            return subprocess.Popen([*head, *product_args], **kwargs)
        return recorder.popen(
            [*head, *product_args],
            product_args=product_args,
            python_options=tuple(python_options),
            secret_input=secret_input,
            fd_inputs=inputs,
            **kwargs,
        )
    finally:
        for read_end in read_ends:
            os.close(read_end)


def _recorder():
    directory = os.environ.get(RECORD_ENV)
    if not directory:
        return None
    from tests.session_harness import recorder

    return recorder.recorder_for(directory)
