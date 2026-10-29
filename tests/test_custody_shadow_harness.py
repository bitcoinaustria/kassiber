"""A custody discrepancy must fail children and sessions that catch errors."""
import json
import os
from pathlib import Path
import subprocess
import sys
import textwrap

import pytest

from tests import custody_engine_compare as compare
from tests import test_tax_shadow_harness as tax_harness

ROOT = Path(__file__).resolve().parents[1]


def child_env(mode="shadow", *, inject=False):
    env = {key: value for key, value in os.environ.items() if not key.startswith("KASSIBER_TEST_CUSTODY_")}
    env[compare.MODE_ENV] = mode
    env["PYTHONPATH"] = os.pathsep.join((str(ROOT / "tests/_child_hooks"), str(ROOT)))
    if inject:
        env[compare.INJECT_ENV] = inject if isinstance(inject, str) else "evidence"
    return env


@pytest.fixture(scope="module")
def cli_book():
    cls = tax_harness.ChildProcessShadowTest
    cls.setUpClass()
    try:
        yield cls
    finally:
        cls.tearDownClass()


@pytest.mark.parametrize("mode", ["shadow", "census"])
@pytest.mark.parametrize("inject", [False, "evidence", "interpretation", "gaps", "quantity"])
def test_custody_child_compares_and_records_differences(cli_book, tmp_path, mode, inject):
    records = tmp_path / "records.jsonl"
    env = child_env(mode, inject=inject)
    env[compare.RECORDS_ENV] = str(records)
    env[compare.MODULE_ENV] = __file__
    result = cli_book._kassiber("journals", "process", "--workspace", "Main", "--profile", "Default", env=env)
    assert result.returncode == (1 if inject else 0), result.stdout[-2000:] + result.stderr[-2000:]
    routes = compare.read_records(records)
    assert len(routes) == 1
    assert routes[0]["outcome"] == ("difference" if inject else "result")
    assert routes[0]["compared"][:2] == ["components", "evidence"]
    if inject != "evidence":
        assert "interpretation" in routes[0]["compared"]
    if inject not in {"evidence", "interpretation"}:
        assert "gaps" in routes[0]["compared"]
    if inject not in {"evidence", "interpretation", "gaps"}:
        assert "quantity" in routes[0]["compared"]
    assert bool(compare.session_report(routes)[1]) == bool(inject)


def test_custody_difference_cannot_be_swallowed_by_a_test(tmp_path):
    probe = tmp_path / "test_caught_custody.py"
    probe.write_text(textwrap.dedent('''
        from kassiber.db import open_db
        from kassiber.core.custody_journal import CustodyJournalBuilder
        from tests.test_custody_native_transitions import seed_native_transition_book

        def test_catches_difference(tmp_path):
            conn = open_db(tmp_path)
            try:
                seed_native_transition_book(conn, "claim")
                profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
                try:
                    CustodyJournalBuilder(conn, profile, as_of="2026-01-01T00:00:00Z").build_custody_projection()
                except AssertionError:
                    pass
            finally:
                conn.close()
    '''))
    result = subprocess.run([sys.executable, "-m", "pytest", "-p", "tests.conftest", "-p", "no:cacheprovider", "-q", str(probe)], cwd=tmp_path, env=child_env(inject=True), capture_output=True, text=True, timeout=120)
    assert result.returncode == 1, result.stdout + result.stderr
    assert "1 passed" in result.stdout
    assert "FAILED: custody differences: 1" in result.stdout


def test_unknown_custody_modes_fail_closed():
    with pytest.raises(ValueError):
        compare.install_custody_mode("native")


def test_every_implemented_stage_is_compared(monkeypatch):
    import kassiber_tax
    expected = {"components": {"blockers": []}, "evidence": {}}
    response = {"schema_version": 1, "stages": {"components": {"blockers": ["injected"]}, "evidence": {}}, "skipped_stages": [stage for stage in compare.STAGES if stage not in expected]}
    monkeypatch.delenv(compare.INJECT_ENV, raising=False)
    monkeypatch.setattr(kassiber_tax, "custody", lambda request: json.dumps(response))
    with pytest.raises(AssertionError, match="components.blockers"):
        compare.compare_rust({}, {}, python_stages=expected)


@pytest.mark.parametrize("first", ["tests.custody_snapshot", "tests.custody_engine_compare", "kassiber.core.custody_journal"])
def test_child_import_order_installs_both_recorder_and_comparison(tmp_path, first):
    env = child_env()
    env["KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR"] = str(tmp_path)
    code = f"import {first}; from kassiber.core import custody_journal as j; print(j.CustodyJournalBuilder._compute_snapshot.__name__, j._build_observer_factory is not None)"
    result = subprocess.run([sys.executable, "-c", code], env=env, cwd=tmp_path, capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "_build True"


def test_stale_binding_cannot_pass_with_no_stages(monkeypatch):
    import kassiber_tax
    monkeypatch.delattr(kassiber_tax, "custody")
    with pytest.raises(AssertionError, match="binding"):
        compare.rust_results({})
    lines, failures = compare.session_report([{"outcome": "result", "compared": [], "skipped": compare.STAGES, "reference": False}])
    assert failures, lines


def test_comparison_does_not_hide_config_text():
    with pytest.raises(AssertionError, match="config_json"):
        compare.assert_same({"config_json": '{"network":"main"}'}, {"config_json": '{ "network": "main" }'})


def test_python_gate_refusal_is_compared_even_when_caught(cli_book, tmp_path, monkeypatch):
    from kassiber.core import custody_journal
    from kassiber.errors import AppError
    from tests.custody_snapshot_generator import snapshot
    book = snapshot(0)
    book["wallets"][0]["config_json"] = '{"network":"regtest"}'
    book["book_network"] = {"binding": None, "observations": [], "references": []}
    book["wallets"][1]["config_json"] = '{"network":"main"}'
    calls = []
    import kassiber_tax
    original = kassiber_tax.custody
    def observe(request):
        calls.append(request)
        return original(request)
    monkeypatch.setattr(kassiber_tax, "custody", observe)
    monkeypatch.setattr(custody_journal.CustodyJournalBuilder, "load_snapshot", lambda self: book)
    builder = custody_journal.CustodyJournalBuilder(None, book["profile"])
    previous = compare._INSTALLED_MODE
    compare.install_custody_mode("census")
    try:
        with pytest.raises(AppError):
            builder.build_custody_decisions()
    finally:
        compare.uninstall_custody_mode()
        if previous:
            compare.install_custody_mode(previous)
    assert len(calls) == 1


@pytest.mark.parametrize("mode", ["census", "shadow"])
@pytest.mark.parametrize("rust_refuses", [False, True, "wrong_code"])
def test_quantity_refusal_compared_after_gaps(tmp_path, monkeypatch, mode, rust_refuses):
    """Review fail-open reproducer: late Python errors cannot escape comparison."""
    import kassiber_tax
    from kassiber.core import custody_quantity_runtime
    from kassiber.core.custody_journal import CustodyJournalBuilder
    from kassiber.db import open_db
    from tests.test_custody_native_transitions import seed_native_transition_book

    records = tmp_path / 'records.jsonl'
    monkeypatch.setenv(compare.RECORDS_ENV, str(records))
    monkeypatch.delenv(compare.INJECT_ENV, raising=False)
    def refuse(*args, **kwargs):
        raise ValueError('simulated Python quantity-stage refusal')
    monkeypatch.setattr(custody_quantity_runtime, 'build_canonical_quantity_state', refuse)
    if rust_refuses:
        monkeypatch.setattr(kassiber_tax, 'custody', lambda _: json.dumps({'error': {'code': 'ValueError' if rust_refuses is True else 'TypeError'}}))
    compare.install_custody_mode(mode)
    conn = open_db(tmp_path / 'book')
    try:
        seed_native_transition_book(conn, 'claim')
        profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
        exception = ValueError if rust_refuses is True else AssertionError
        with pytest.raises(exception):
            CustodyJournalBuilder(conn, profile, as_of='2026-01-01T00:00:00Z').build_custody_projection()
    finally:
        conn.close()
        compare.uninstall_custody_mode()
    rows = compare.read_records(records)
    assert rows[-1]['outcome'] == ('error' if rust_refuses is True else 'difference')
    if rust_refuses is True:
        assert rows[-1]['compared'] == ['quantity']
        assert set(rows[-1]['compared']) | set(rows[-1]['skipped']) == set(compare.STAGES)
    assert bool(compare.session_report(rows)[1]) == (rust_refuses is not True)


def test_session_rejects_unaccounted_quantity_stage():
    record = dict(outcome='result', compared=compare.STAGES[:4], skipped=['finalized_projection'], reference=False)
    assert 'custody stage inventory is incomplete' in compare.session_report([record])[1]


@pytest.mark.parametrize('mode', ['census', 'shadow'])
def test_rust_quantity_refusal_against_python_success(tmp_path, monkeypatch, mode):
    import kassiber_tax
    from kassiber.core.custody_journal import CustodyJournalBuilder
    from tests.custody_snapshot_generator import snapshot
    book = snapshot(0)
    monkeypatch.setattr(CustodyJournalBuilder, 'load_snapshot', lambda _: book)
    monkeypatch.setattr(kassiber_tax, 'custody', lambda _: json.dumps({'error': {'code': 'ValueError'}}))
    monkeypatch.setenv(compare.RECORDS_ENV, str(tmp_path / 'records'))
    previous = compare._INSTALLED_MODE
    compare.install_custody_mode(mode)
    try:
        with pytest.raises(AssertionError, match='refused'):
            CustodyJournalBuilder(None, book['profile']).build_custody_decisions()
    finally:
        compare.uninstall_custody_mode()
        if previous:
            compare.install_custody_mode(previous)
    assert compare.read_records(tmp_path / 'records')[-1]['outcome'] == 'difference'


@pytest.mark.parametrize('mode', ['census', 'shadow'])
@pytest.mark.parametrize('injection', ['exact', 'different_exception', 'different_stage'])
def test_injected_failure_requires_exact_stage_and_exception(tmp_path, monkeypatch, mode, injection):
    from kassiber.core import custody_quantity_runtime
    from kassiber.core.custody_journal import CustodyJournalBuilder
    from tests.custody_snapshot_generator import snapshot
    book = snapshot(0)
    error = ValueError('intentional test failure')
    def refuse(*args, **kwargs):
        raise error
    monkeypatch.setattr(custody_quantity_runtime, 'build_canonical_quantity_state', refuse)
    monkeypatch.setattr(CustodyJournalBuilder, 'load_snapshot', lambda _: book)
    monkeypatch.setenv(compare.RECORDS_ENV, str(tmp_path / 'records'))
    previous = compare._INSTALLED_MODE
    compare.install_custody_mode(mode)
    try:
        declared_error = ValueError(str(error)) if injection == 'different_exception' else error
        declared_stage = 'gaps' if injection == 'different_stage' else 'quantity'
        with compare.injected_stage_failure(declared_stage, declared_error):
            with pytest.raises(ValueError if injection == 'exact' else AssertionError):
                CustodyJournalBuilder(None, book['profile']).build_custody_decisions()
        # The annotation must not leak to a later build.
        with pytest.raises(AssertionError):
            CustodyJournalBuilder(None, book['profile']).build_custody_decisions()
    finally:
        compare.uninstall_custody_mode()
        if previous:
            compare.install_custody_mode(previous)
    rows = compare.read_records(tmp_path / 'records')
    first = rows[0]
    if injection == 'exact':
        assert first['outcome'] == first['reason'] == 'injected'
        assert first['compared'] == [] and set(first['skipped']) == set(compare.STAGES)
        lines, errors = compare.session_report(rows[:1])
        assert not errors
        assert 'injected builds: 1 (all stages skipped)' in lines
    else:
        assert first['outcome'] == 'difference'
    assert rows[-1]['outcome'] == 'difference'


@pytest.mark.parametrize('change', [{'reason': None}, {'compared': ['quantity']}, {'skipped': []}])
def test_session_rejects_invalid_injected_skip(change):
    row = dict(outcome='injected', compared=[], skipped=list(compare.STAGES), reference=False, reason='injected')
    row.update(change)
    assert 'custody injected skip record is invalid' in compare.session_report([row])[1]


@pytest.mark.parametrize('mode', ['census', 'shadow'])
def test_recorder_failure_has_explicit_injected_skip(mode):
    result = subprocess.run([sys.executable, '-m', 'pytest', '-q', 'tests/test_custody_snapshot.py::test_a_failed_build_records_its_error'], cwd=ROOT, env=child_env(mode), capture_output=True, text=True, timeout=120)
    assert result.returncode == 0, result.stdout + result.stderr
    # Pytest owns and removes its private records directory. Its session
    # report validates the record and exposes the explicit skip inventory.
    assert 'injected builds: 1 (all stages skipped)' in result.stdout
    assert 'quantity: 0 compared, 1 skipped' in result.stdout
