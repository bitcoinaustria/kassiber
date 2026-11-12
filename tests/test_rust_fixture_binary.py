"""The stale-binary guard reads cargo's dep-info beside each test binary."""
import json
import os

import pytest

from tests.rust_fixture_binary import checked, main, record, stale, stale_environment_binaries


def _tree(tmp_path, *, sources=("a.rs", "with space.rs")):
    root = tmp_path / "root"
    (root / "src").mkdir(parents=True)
    (root / "Cargo.lock").write_text("")
    paths = [root / "src" / name for name in sources]
    for path in paths:
        path.write_text("")
    binary = tmp_path / "debug" / "fixture"
    binary.parent.mkdir()
    binary.write_text("")
    deps = " ".join(str(path).replace(" ", "\\ ") for path in paths)
    binary.with_name("fixture.d").write_text(f"{binary}: {deps}\n\n")
    for path in [root / "Cargo.lock", *paths]:
        os.utime(path, (1_000, 1_000))
    os.utime(binary, (2_000, 2_000))
    return root, binary, paths


def test_a_binary_newer_than_its_sources_is_fresh(tmp_path):
    root, binary, _ = _tree(tmp_path)
    assert stale(binary, root) is None


def test_a_newer_source_it_was_built_from_makes_it_stale(tmp_path):
    root, binary, paths = _tree(tmp_path)
    os.utime(paths[1], (3_000, 3_000))
    assert stale(binary, root) == f"predates {paths[1]}"


def test_an_unrelated_newer_source_does_not(tmp_path):
    root, binary, _ = _tree(tmp_path)
    other = root / "src" / "other.rs"
    other.write_text("")
    os.utime(other, (3_000, 3_000))
    assert stale(binary, root) is None


def test_a_newer_lockfile_makes_it_stale(tmp_path):
    root, binary, _ = _tree(tmp_path)
    os.utime(root / "Cargo.lock", (3_000, 3_000))
    assert stale(binary, root) == f"predates {root / 'Cargo.lock'}"


def test_a_removed_source_or_missing_dep_info_makes_it_stale(tmp_path):
    root, binary, paths = _tree(tmp_path)
    paths[0].unlink()
    assert stale(binary, root) == f"was built from {paths[0]}, which no longer exists"
    binary.with_name("fixture.d").unlink()
    assert "has no cargo dep-info (fixture.d)" in stale(binary, root)


def test_checked_refuses_with_the_build_and_passes_a_missing_binary(tmp_path, monkeypatch):
    root, binary, paths = _tree(tmp_path)
    monkeypatch.setattr("tests.rust_fixture_binary.ROOT", root)
    os.utime(paths[0], (3_000, 3_000))
    with pytest.raises(AssertionError, match="rebuild it: cargo build -p x"):
        checked(binary, "cargo build -p x")
    missing = tmp_path / "debug" / "missing"
    assert checked(missing, "cargo build -p x") == missing


def test_the_session_names_each_stale_environment_binary(tmp_path, monkeypatch):
    root, binary, paths = _tree(tmp_path)
    monkeypatch.setattr("tests.rust_fixture_binary.ROOT", root)
    os.utime(paths[0], (3_000, 3_000))
    environ = {"KASSIBER_RUST_BIN": str(binary), "KASSIBER_PSBT_BIN": str(tmp_path / "absent")}
    assert stale_environment_binaries(environ) == [("KASSIBER_RUST_BIN", str(binary), f"predates {paths[0]}")]


def test_a_recorded_build_stays_fresh_until_another_build_relinks_it(tmp_path):
    root, binary, _ = _tree(tmp_path)
    record(binary, "-p kassiber --features test-harness")
    assert stale(binary, root) is None
    # A different cargo invocation replaces the file; its sources stay older.
    binary.unlink()
    binary.write_text("relinked with unified workspace features")
    os.utime(binary, (2_500, 2_500))
    assert stale(binary, root) == (
        "was relinked after its build with '-p kassiber --features test-harness' was recorded "
        "(another cargo invocation, such as a workspace test, built a different feature set)"
    )
    record(binary, "-p kassiber --features test-harness")
    assert stale(binary, root) is None


def test_an_unrecorded_binary_keeps_the_dep_info_check(tmp_path):
    root, binary, _ = _tree(tmp_path)
    assert not binary.with_name("fixture.build.json").exists()
    assert stale(binary, root) is None


def test_record_command_writes_one_record_per_binary(tmp_path):
    root, binary, _ = _tree(tmp_path)
    other = binary.with_name("other")
    other.write_text("")
    assert main(["record", "--features", "-p x --features y", str(binary), str(other)]) == 0
    assert json.loads(binary.with_name("fixture.build.json").read_text())["features"] == "-p x --features y"
    assert json.loads(other.with_name("other.build.json").read_text())["features"] == "-p x --features y"

