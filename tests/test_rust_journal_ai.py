"""The assistant tool's journal step (`auto_pair=False`) against unchanged Python.

`ai.chat` runs `ui.journals.process` as `_journals_refresh_payload(conn,
auto_pair=False)` on the daemon's own connection. The native entry is the same
shared refresh service the desktop route uses, reached here through a
test-only daemon kind until the native chat loop owns consent and scope.
"""
import json
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

from tests.test_rust_journal_commands import BINARY, KEY, ROOT, book_dump, seed, seed_auto
from tests.test_rust_journal_commands import environment  # noqa: F401
from tests.test_rust_journal_desktop import Journals
from tests.test_rust_journal_process import scalar_exact
from tests.rust_port.journal_refresh_oracle import SCENARIOS

pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires native harness binary")


def reference(root, env, cipher):
    env = dict(env)
    if cipher:
        env["KASSIBER_TEST_JOURNAL_AI_PASSPHRASE"] = KEY
    completed = subprocess.run([sys.executable, "-m", "tests.rust_port.journal_ai_reference", "--data-root", str(root)],
                               env=env, cwd=ROOT, capture_output=True, timeout=300, check=False)
    assert completed.returncode == 0 and completed.stderr == b"", completed
    return json.loads(completed.stdout)


def native(root, env, cipher, auto_pair=False):
    with Journals(root, env) as session:
        session.unlock(cipher)
        session.send("test.journals.refresh", "tool", {"auto_pair": auto_pair})
        row = session.until("tool", "ui.journals.process")
        session.finish()
        assert not any(r.get("kind") == "ui.journals.process.progress" for r in session.rows)
    return row


def assert_same(actual, expected):
    if "error" in expected:
        assert actual["kind"] == "error", actual
        error = actual["error"]
        assert {k: error[k] for k in ("code", "message", "hint", "details", "retryable")} == expected["error"]
    elif "exception" in expected:
        assert actual["kind"] == "error" and actual["error"]["code"] == "internal_error", actual
    else:
        assert actual["kind"] == expected["kind"] == "ui.journals.process", (actual, expected)
        assert json.dumps(actual["data"]) == json.dumps(expected["data"])


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("current", [False, True])
@pytest.mark.parametrize("scenario", SCENARIOS)
def test_ai_journal_step_never_pairs_and_matches_python(tmp_path, environment, cipher, current, scenario):
    python = seed_auto(tmp_path / "python", cipher, scenario, current)
    rust = tmp_path / "rust"
    shutil.copytree(python, rust)
    expected = reference(python, environment, cipher)
    actual = native(rust, environment, cipher)
    assert_same(actual, expected)
    assert "auto_pair" not in actual["data"]
    assert scalar_exact(book_dump(rust, cipher), book_dump(python, cipher))


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("shape", ["unchanged", "changed", "blocked", "empty", "build-refusal", "label-refusal",
                                   "admission-refusal"])
def test_ai_journal_step_plain_books_and_refusals(tmp_path, environment, cipher, shape):
    python = seed(tmp_path / "python", cipher=cipher, shape=shape)
    rust = tmp_path / "rust"
    shutil.copytree(python, rust)
    expected = reference(python, environment, cipher)
    actual = native(rust, environment, cipher)
    assert_same(actual, expected)
    assert scalar_exact(book_dump(rust, cipher), book_dump(python, cipher))


def test_ai_journal_step_differs_from_desktop_only_by_pairing(tmp_path, environment):
    """A control that the oracle can see automatic pairing at all."""
    python = seed_auto(tmp_path / "python", scenario="exact")
    rust = tmp_path / "rust"
    shutil.copytree(python, rust)
    paired = native(rust, environment, False, auto_pair=True)
    assert paired["data"]["auto_pair"]["applied"] > 0
    assert reference(python, environment, False)["data"].get("auto_pair", {}).get("applied", 0) == 0
