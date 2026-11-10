"""The migration census must never silently omit a test or a wire contract."""

from __future__ import annotations

import ast
import copy
from pathlib import Path

from scripts.python_test_shards import PREFLIGHT_SHARD, shard_for
from tests.rust_port import classify_tests as census
from tests.rust_port import reviews


def test_every_test_has_a_valid_disposition():
    problems = census.check_inventory(census.load_inventory(), census.discover())
    assert not problems, "\n".join(problems)


def test_missing_modules_and_added_class_methods_fail_the_census():
    inventory = copy.deepcopy(census.load_inventory())
    path = "tests/test_ci_shards.py"
    del inventory["modules"][path]
    assert any(f"{path}: no disposition entry" in item for item in census.check_inventory(inventory, census.discover()))
    inventory = copy.deepcopy(census.load_inventory())
    inventory["modules"][path]["PythonShardContractTest"]["tests"] -= 1
    assert any("source defines" in item for item in census.check_inventory(inventory, census.discover()))


def test_only_the_five_agreed_dispositions_are_allowed():
    assert set(census.DISPOSITIONS) == {"boundary-fixture", "subprocess-scenario", "rust-test", "oracle", "python-only"}


def test_the_thirty_largest_modules_have_recorded_manual_samples():
    largest = sorted(census.discover(), key=lambda path: (
        -len((census.ROOT / path).read_text(encoding="utf-8").splitlines()), path.as_posix(),
    ))[:30]
    assert all(reviews.SAMPLES.get(path.as_posix()) for path in largest)


def test_inventory_checks_run_in_preflight():
    assert shard_for(Path("tests/rust_port/test_inventory.py")) == PREFLIGHT_SHARD


def test_counts_in_the_reference_match_the_inventory():
    reference = (census.ROOT / "docs/reference/rust-port.md").read_text(encoding="utf-8")
    table = reference.split("<!-- disposition-counts:start -->\n", 1)[1].split("<!-- disposition-counts:end -->", 1)[0]
    assert table == census.summary_markdown(census.load_inventory())


def _verdict(source):
    tree = ast.parse(source)
    index = census.ModuleIndex(
        "tests.example", census.ROOT / "tests/example.py", tree,
        census._import_map(tree, "tests.example", False), {}, {},
    )
    features, _ = census.direct_features(tree, index)
    return census.heuristic(Path("tests/example.py"), census.TestFunction("test_example", features), "cli-shell")


def test_sql_seeded_children_are_scenarios_and_not_portable_recordings():
    verdict = _verdict('''
import sqlite3
import subprocess
conn = sqlite3.connect("synthetic.sqlite3")
conn.execute("INSERT INTO transactions(id) VALUES ('synthetic')")
subprocess.run(["python", "-m", "kassiber", "--machine", "status"])
''')
    assert verdict.disposition == "subprocess-scenario"
    assert {"sql", "sql-seed", "subprocess"} <= set(verdict.flags)


def test_environment_patches_are_distinct_from_internal_patches():
    prefix = 'import subprocess\nfrom unittest.mock import patch as replace\n'
    child = '    subprocess.run(["python", "-m", "kassiber", "status"])\n'
    env = _verdict(prefix + 'with replace.dict("os.environ", {"HOME": "/tmp"}):\n' + child)
    internal = _verdict(prefix + 'with replace("kassiber.db.open_db"):\n' + child)
    assert env.disposition == "boundary-fixture"
    assert "mock-internal" not in env.flags
    assert internal.disposition == "subprocess-scenario"
    assert "mock-internal" in internal.flags
