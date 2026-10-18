#!/usr/bin/env python3
"""Check that the tax tests fail when the rule they pin breaks (plan 20, phase 4).

``tax-engine/scripts/mutations.json`` lists mutations. Each entry has these
fields and no others:

- ``id``: a unique name;
- ``file``: the repo-relative file to mutate;
- ``find`` and ``replace``: ``find`` must occur exactly once in ``file`` and
  is replaced by ``replace``;
- ``expect_failing``: the tests that must fail once it is replaced. For a Rust
  file these are cargo test name filters, each run over ``tax-engine/``; for
  any other file, pytest node ids, run with the tax-engine extension already
  installed in this environment (a Python mutation never rebuilds it);
- ``rule``: one sentence saying what the mutation breaks, so the entry can be
  ported when the rule moves.

The check copies the working tree (tracked and untracked files that git does
not ignore) to a scratch directory under the system temporary directory and
never writes the worktree. There it runs every named test once unmutated
(each must pass and must select at least one test), then applies one mutation
at a time, runs its tests, and restores the file. Tests run on production:
``KASSIBER_TEST_TAX_*`` variables are dropped from their environment. A
mutation survives when one of its tests still passes; a Rust mutation that
does not build is an error, not a kill. Any survivor or error fails the run.

    uv run --locked python tax-engine/scripts/mutation_check.py [--only ID ...] [--keep]
    uv run --locked python tax-engine/scripts/mutation_check.py --check

``--check`` only validates the entries against the worktree; CI runs it
through ``tests/test_tax_mutations.py``. The full check runs one test process
at a time and is too slow for CI: run it whenever ``mutations.json`` or the
code its entries mutate changes.
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Optional, Sequence

REPO = Path(__file__).resolve().parents[2]
MUTATIONS = REPO / "tax-engine" / "scripts" / "mutations.json"
FIELDS = ("id", "file", "find", "replace", "expect_failing", "rule")
TEST_TIMEOUT_SECONDS = 1800
_CARGO_RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed", re.MULTILINE)


class EntryError(ValueError):
    pass


@dataclass(frozen=True)
class Mutation:
    id: str
    file: str
    find: str
    replace: str
    expect_failing: tuple[str, ...]
    rule: str

    @property
    def runner(self) -> str:
        return "cargo" if self.file.endswith(".rs") else "pytest"


def load(path: Path = MUTATIONS) -> list[Mutation]:
    """The entries of ``path``, with their shape checked."""
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise EntryError(f"{path.name} must hold a list of mutations")
    mutations = []
    for index, item in enumerate(data):
        if not isinstance(item, dict) or set(item) != set(FIELDS):
            raise EntryError(f"entry {index} must have exactly the fields {', '.join(FIELDS)}")
        for field in ("id", "file", "find", "rule"):
            if not isinstance(item[field], str) or not item[field]:
                raise EntryError(f"entry {index}: {field} must be a non-empty string")
        if not isinstance(item["replace"], str):
            raise EntryError(f"entry {index}: replace must be a string")
        tests = item["expect_failing"]
        if isinstance(tests, str):
            tests = [tests]
        if not isinstance(tests, list) or not tests or not all(isinstance(test, str) and test for test in tests):
            raise EntryError(f"entry {index}: expect_failing must name at least one test")
        mutations.append(
            Mutation(item["id"], item["file"], item["find"], item["replace"], tuple(tests), item["rule"])
        )
    return mutations


def _missing_test(root: Path, node_id: str) -> Optional[str]:
    path, _, rest = node_id.partition("::")
    if not rest:
        return f"{node_id!r} is not a pytest node id"
    source = root / path
    if not source.is_file():
        return f"{node_id!r}: no file {path}"
    scope: Sequence[ast.stmt] = ast.parse(source.read_text(encoding="utf-8")).body
    for name in rest.split("::"):
        name = name.split("[", 1)[0]
        found = next(
            (
                node
                for node in scope
                if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name
            ),
            None,
        )
        if found is None:
            return f"{node_id!r} names no test in {path}"
        scope = found.body if isinstance(found, ast.ClassDef) else ()
    return None


def _escapes(root: Path, name: str) -> bool:
    """Whether ``name`` could reach outside ``root``: an absolute path, ``..``,
    a symlink, or a path that resolves elsewhere. The check writes the mutated
    file in the scratch tree, never in the worktree."""
    relative = Path(name)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        return True
    target = root / relative
    if any(path.is_symlink() for path in (target, *target.parents) if path != root and root in path.parents):
        return True
    return not target.resolve().is_relative_to(root.resolve())


def problems(mutations: Iterable[Mutation], root: Path = REPO) -> list[str]:
    """What keeps the entries from applying to the tree at ``root``."""
    found = []
    seen: set[str] = set()
    for mutation in mutations:
        if mutation.id in seen:
            found.append(f"{mutation.id}: duplicate id")
        seen.add(mutation.id)
        if mutation.find == mutation.replace:
            found.append(f"{mutation.id}: replace equals find")
        if _escapes(root, mutation.file):
            found.append(f"{mutation.id}: file {mutation.file} is not a file inside the tree")
            continue
        path = root / mutation.file
        if not path.is_file():
            found.append(f"{mutation.id}: no file {mutation.file}")
            continue
        count = path.read_text(encoding="utf-8").count(mutation.find)
        if count != 1:
            found.append(f"{mutation.id}: find occurs {count} times in {mutation.file}, not once")
        if mutation.runner == "pytest":
            for node_id in mutation.expect_failing:
                missing = _missing_test(root, node_id)
                if missing:
                    found.append(f"{mutation.id}: {missing}")
    return found


def _worktree_files(root: Path) -> list[str]:
    listed = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        check=True,
        capture_output=True,
    ).stdout.decode("utf-8")
    return sorted({name for name in listed.split("\0") if name})


def copy_worktree(root: Path, scratch: Path) -> None:
    for name in _worktree_files(root):
        source = root / name
        if source.is_dir() and not source.is_symlink():
            continue
        if not source.exists() and not source.is_symlink():
            # Deleted in the working tree but still in the index.
            continue
        target = scratch / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target, follow_symlinks=False)


def _environment() -> dict[str, str]:
    env = {key: value for key, value in os.environ.items() if not key.startswith("KASSIBER_TEST_TAX_")}
    # Bytecode for the scratch tree would outlive it under a cache prefix.
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    return env


def _cargo_environment(env: dict[str, str]) -> dict[str, str]:
    """``env`` without this virtual environment, as the quality gate runs cargo.

    Under ``uv run`` the venv's interpreter comes first on ``PATH``; PyO3 would
    link the binding's test binary against that interpreter's library, which
    the loader cannot find outside the venv.
    """
    venv_bin = os.path.join(sys.prefix, "bin")
    cargo_env = {key: value for key, value in env.items() if key != "VIRTUAL_ENV"}
    cargo_env["PATH"] = os.pathsep.join(
        entry for entry in env.get("PATH", "").split(os.pathsep) if entry and entry != venv_bin
    )
    return cargo_env


def _run(command: Sequence[str], cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        list(command),
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        timeout=TEST_TIMEOUT_SECONDS,
    )


def _check_imports_resolve_to(scratch: Path, env: dict[str, str]) -> None:
    probe = _run(
        [sys.executable, "-c", "import kassiber, tests; print(kassiber.__file__); print(tests.__file__)"],
        scratch,
        env,
    )
    paths = [Path(line).resolve() for line in probe.stdout.split()]
    if probe.returncode != 0 or len(paths) != 2 or not all(path.is_relative_to(scratch.resolve()) for path in paths):
        raise SystemExit(f"tests in the scratch tree would not import its code: {probe.stdout}{probe.stderr}")


class _Runner:
    def __init__(self, scratch: Path, env: dict[str, str]) -> None:
        self.scratch = scratch
        self.env = env
        self.cargo_env = _cargo_environment(env)
        self.manifest = scratch / "tax-engine" / "Cargo.toml"

    def builds(self) -> tuple[bool, str]:
        result = _run(
            ["cargo", "test", "--manifest-path", str(self.manifest), "--locked", "--no-run"],
            self.scratch,
            self.cargo_env,
        )
        return result.returncode == 0, result.stderr[-2000:]

    def outcome(self, runner: str, test: str) -> tuple[str, str]:
        """``passed``, ``failed``, or ``error`` (it selected no test or could not run)."""
        if runner == "cargo":
            result = _run(
                ["cargo", "test", "--manifest-path", str(self.manifest), "--locked", "--", test],
                self.scratch,
                self.cargo_env,
            )
            counts = [(int(passed), int(failed)) for passed, failed in _CARGO_RESULT.findall(result.stdout)]
            selected = sum(passed + failed for passed, failed in counts)
            if result.returncode == 0:
                return ("passed" if selected else "error"), result.stdout[-2000:]
            return ("failed" if selected else "error"), (result.stdout + result.stderr)[-2000:]
        result = _run(
            [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider", test],
            self.scratch,
            self.env,
        )
        # 0: passed, 1: a test failed; anything else (collection, usage, nothing
        # selected) says nothing about the rule.
        status = {0: "passed", 1: "failed"}.get(result.returncode, "error")
        return status, (result.stdout + result.stderr)[-2000:]


def check(mutations: Sequence[Mutation], *, keep: bool = False) -> int:
    trouble = problems(mutations)
    if trouble:
        print("\n".join(trouble), file=sys.stderr)
        return 2
    scratch = Path(tempfile.mkdtemp(prefix="kassiber-mutation-check-"))
    try:
        copy_worktree(REPO, scratch)
        env = _environment()
        runner = _Runner(scratch, env)
        if any(mutation.runner == "pytest" for mutation in mutations):
            _check_imports_resolve_to(scratch, env)
        failures = []
        baseline = sorted({(mutation.runner, test) for mutation in mutations for test in mutation.expect_failing})
        for kind, test in baseline:
            status, output = runner.outcome(kind, test)
            if status != "passed":
                failures.append(f"baseline: {test} {status} before any mutation\n{output}")
        if failures:
            print("\n\n".join(failures), file=sys.stderr)
            return 1
        print(f"baseline: {len(baseline)} tests pass unmutated")
        for mutation in mutations:
            if _escapes(scratch, mutation.file):
                raise SystemExit(f"{mutation.id}: file {mutation.file} is not a file inside the scratch tree")
            path = scratch / mutation.file
            original = path.read_text(encoding="utf-8")
            path.write_text(original.replace(mutation.find, mutation.replace, 1), encoding="utf-8")
            try:
                if mutation.runner == "cargo":
                    ok, output = runner.builds()
                    if not ok:
                        failures.append(f"{mutation.id}: the mutated tree does not build\n{output}")
                        print(f"ERROR    {mutation.id}")
                        continue
                outcomes = {test: runner.outcome(mutation.runner, test) for test in mutation.expect_failing}
            finally:
                path.write_text(original, encoding="utf-8")
            passed = [test for test, (status, _) in outcomes.items() if status == "passed"]
            errors = [test for test, (status, _) in outcomes.items() if status == "error"]
            if passed or errors:
                print(f"SURVIVED {mutation.id}" if passed else f"ERROR    {mutation.id}")
                failures.extend(f"{mutation.id}: {test} still passes" for test in passed)
                failures.extend(f"{mutation.id}: {test} could not run\n{outcomes[test][1]}" for test in errors)
            else:
                print(f"killed   {mutation.id} ({len(outcomes)} failing)")
        if failures:
            print("\n".join(["", *failures]), file=sys.stderr)
            return 1
        print(f"all {len(mutations)} mutations killed")
        return 0
    finally:
        if keep:
            print(f"scratch tree kept at {scratch}")
        else:
            shutil.rmtree(scratch, ignore_errors=True)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--mutations", type=Path, default=MUTATIONS, help="the entries to check")
    parser.add_argument("--only", action="append", default=[], metavar="ID", help="check only this entry (repeatable)")
    parser.add_argument("--check", action="store_true", help="only validate the entries against the worktree")
    parser.add_argument("--keep", action="store_true", help="keep the scratch tree")
    args = parser.parse_args(argv)
    # One line per mutation as it finishes, also when piped to a log.
    sys.stdout.reconfigure(line_buffering=True)
    try:
        mutations = load(args.mutations)
    except (EntryError, json.JSONDecodeError) as exc:
        print(exc, file=sys.stderr)
        return 2
    if args.only:
        unknown = sorted(set(args.only) - {mutation.id for mutation in mutations})
        if unknown:
            print(f"--only names unknown entries: {', '.join(unknown)}", file=sys.stderr)
            return 2
        mutations = [mutation for mutation in mutations if mutation.id in args.only]
    if args.check:
        trouble = problems(mutations)
        print("\n".join(trouble) if trouble else f"{len(mutations)} entries apply")
        return 1 if trouble else 0
    return check(mutations, keep=args.keep)


if __name__ == "__main__":
    sys.exit(main())
