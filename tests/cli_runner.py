"""Start the product as a child process: the one place tests build that command.

Every test, integration lane, and regtest seed that runs the CLI or
`kassiber daemon` as a subprocess goes through `run` or `popen` here, so one
switch can point all of them at another executable
(`tests/test_cli_runner_sites.py` keeps it that way):

- `KASSIBER_TEST_CLI` unset: `python -m kassiber` from this checkout.
- `KASSIBER_TEST_CLI=/path/to/kassiber`: that executable.
- `KASSIBER_TEST_CLI='["/path/to/kassiber", "--flag"]'`: a JSON argv prefix,
  run without a shell.

The working directory defaults to the repository root either way. The
variable is test-only: product code never reads it.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from collections.abc import Iterable, Sequence
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
CLI_ENV = "KASSIBER_TEST_CLI"
REFERENCE_PREFIX: tuple[str, ...] = (sys.executable, "-m", "kassiber")


class CandidateConfigError(RuntimeError):
    """`KASSIBER_TEST_CLI` does not name a runnable executable."""


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
    args: Iterable[str | os.PathLike[str]],
    *,
    reference: bool = False,
    python_options: Sequence[str] = (),
) -> list[str]:
    """The full command line for `args`, for messages and inspection."""
    return [*prefix(reference=reference, python_options=python_options), *(os.fspath(arg) for arg in args)]


def run(
    args: Iterable[str | os.PathLike[str]],
    *,
    reference: bool = False,
    python_options: Sequence[str] = (),
    secret_input: bool = False,
    **kwargs,
) -> subprocess.CompletedProcess:
    """`subprocess.run` of the product with `args`.

    Keyword arguments pass through to `subprocess.run`; `cwd` defaults to the
    repository root. `secret_input=True` marks stdin as a credential.
    """
    command = argv(args, reference=reference, python_options=python_options)
    kwargs.setdefault("cwd", ROOT)
    return subprocess.run(command, **kwargs)


def popen(
    args: Iterable[str | os.PathLike[str]],
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
    command = argv(args, reference=reference, python_options=python_options)
    kwargs.setdefault("cwd", ROOT)
    return subprocess.Popen(command, **kwargs)
