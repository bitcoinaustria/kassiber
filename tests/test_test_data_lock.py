"""The test-data lock is well formed, ignored, and fetched wherever tests run.

Generated fixtures live on the data branch tests/test-data.lock pins
(docs/reference/testing.md#generated-test-data); scripts/fetch-test-data.sh
puts them in place. These checks need no fetched data.
"""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "tests/test-data.lock"
SCRIPT = ROOT / "scripts/fetch-test-data.sh"
BEGIN = "# >>> generated test data: tests/test-data.lock (scripts/fetch-test-data.sh --ignore)"
END = "# <<< generated test data"


def _lock():
    header, entries = {}, []
    for line in LOCK.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        if re.fullmatch(r"[0-9a-f]{40} \S+", line):
            blob, path = line.split(" ", 1)
            entries.append((blob, path))
        else:
            key, _, value = line.partition(" ")
            header[key] = value
    return header, entries


def test_lock_names_a_branch_a_commit_and_sorted_unique_files():
    header, entries = _lock()
    assert header.get("branch") == "t3code/test-data"
    assert re.fullmatch(r"[0-9a-f]{40}", header.get("commit", ""))
    paths = [path for _blob, path in entries]
    assert paths, "the lock pins no files"
    assert paths == sorted(set(paths))
    assert all(not path.startswith("/") and ".." not in path.split("/") for path in paths)


def test_locked_files_are_ignored_and_not_tracked():
    _header, entries = _lock()
    paths = [path for _blob, path in entries]
    text = (ROOT / ".gitignore").read_text(encoding="utf-8")
    assert BEGIN in text and END in text
    block = text.split(BEGIN, 1)[1].split(END, 1)[0].split()
    assert block == ["/.test-data/", *("/" + path for path in paths)]
    tracked = subprocess.run(
        ["git", "ls-files", "--", *paths], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.split()
    assert tracked == [], "generated test data belongs on the data branch, not in this tree"


def test_fetch_script_is_executable_and_uses_git_only():
    assert SCRIPT.is_file() and os.access(SCRIPT, os.X_OK)
    text = SCRIPT.read_text(encoding="utf-8")
    assert "tests/test-data.lock" in text
    for tool in ("curl", "wget", "pip ", "uv ", "npm "):
        assert tool not in text


def test_ci_fetches_the_data_before_tests_and_rust_builds():
    workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))
    needs = re.compile(r"\bcargo\b|\buv sync\b|\bpytest\b|python_test_shards")
    checked = 0
    for name, job in workflow["jobs"].items():
        steps = job.get("steps") or []
        runs = [index for index, step in enumerate(steps) if needs.search(str(step.get("run") or ""))]
        if not runs:
            continue
        fetch = [index for index, step in enumerate(steps) if "scripts/fetch-test-data.sh" in str(step.get("run") or "")]
        assert fetch and fetch[0] < runs[0], name
        checked += 1
    assert checked
