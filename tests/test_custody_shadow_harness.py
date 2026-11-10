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


@pytest.fixture(autouse=True)
def isolated_comparison_mode():
    previous = compare._INSTALLED_MODE
    compare.uninstall_custody_mode()
    try:
        yield
    finally:
        compare.uninstall_custody_mode()
        if previous:
            compare.install_custody_mode(previous)


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
@pytest.mark.parametrize("inject", [False, "evidence", "interpretation", "gaps", "quantity", "finalized_projection", "fusion"])
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
    if inject not in {"evidence", "interpretation", "gaps", "quantity"}:
        assert "finalized_projection" in routes[0]["compared"]
    if not inject or inject == "fusion":
        assert "fusion" in routes[0]["compared"]
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


@pytest.mark.parametrize("patched", [False, True])
def test_policy_builder_patch_skips_only_fusion(tmp_path, monkeypatch, patched):
    from dataclasses import replace
    from kassiber import tax_policy
    from kassiber.core import custody_journal
    from tests.custody_snapshot_generator import snapshot

    book = snapshot(0)
    if patched:
        book["profile"]["cost_basis_pool_scope"] = "wallet"
        monkeypatch.setitem(tax_policy.POLICY_BUILDERS, "generic", lambda profile: replace(
            tax_policy.build_generic_policy(profile), allowed_cost_basis_pool_scopes=("global", "wallet"),
        ))

    class Builder:
        conn = None
        profile = book["profile"]

        def load_snapshot(self):
            return book

    monkeypatch.setattr(custody_journal, "_build_observer_factory", None)
    monkeypatch.setattr(compare, "_INSTALLED_MODE", "census")
    monkeypatch.setenv(compare.RECORDS_ENV, str(tmp_path / "records"))
    compare._build(Builder(), finalized=True)
    row, = compare.read_records(tmp_path / "records")
    assert row["outcome"] == "result"
    assert set(row["compared"]) == set(compare.STAGES) - ({"fusion"} if patched else set())
    assert row["skipped"] == (["fusion"] if patched else [])
    assert row.get("reason") == (compare.PATCHED_POLICY_REASON if patched else None)
    assert not compare.session_report([row])[1]
@pytest.mark.parametrize("injection", ["fusion", "quantity"])
def test_test_policy_skips_only_matching_fusion_and_does_not_leak(tmp_path, monkeypatch, injection):
    from kassiber.core import custody_journal
    from kassiber.tax_policy import POLICY_BUILDERS
    from tests.custody_snapshot_generator import snapshot

    book = snapshot(0)
    class Builder:
        conn = None
        profile = book["profile"]
        def load_snapshot(self):
            return book

    policy = POLICY_BUILDERS[book["profile"]["tax_country"]]
    monkeypatch.setattr(custody_journal, "_build_observer_factory", None)
    monkeypatch.setattr(compare, "_INSTALLED_MODE", "census")
    monkeypatch.setenv(compare.RECORDS_ENV, str(tmp_path / "records"))
    monkeypatch.setenv(compare.INJECT_ENV, injection)
    with compare.injected_fusion_policy(policy):
        if injection == "fusion":
            compare._build(Builder(), finalized=True)
        else:
            with pytest.raises(AssertionError, match="quantity"):
                compare._build(Builder(), finalized=True)
    with pytest.raises(AssertionError, match=injection):
        compare._build(Builder(), finalized=True)
    if injection == "fusion":
        row = compare.read_records(tmp_path / "records")[0]
        assert row["reason"] == "injected_policy"
        assert row["compared"] == list(compare.STAGES[:-1])
        assert row["skipped"] == ["fusion"]
        assert not compare.session_report([row])[1]


@pytest.mark.parametrize("patched", [False, True])
def test_patched_compute_function_skips_every_stage(tmp_path, monkeypatch, patched):
    from kassiber.core import custody_journal
    from kassiber.core import custody_tax_projection
    from kassiber.errors import AppError
    from tests.custody_snapshot_generator import snapshot

    book = snapshot(0)
    monkeypatch.setattr(compare, "_COMPUTE_FUNCTIONS", compare._capture_compute_functions())
    if patched:
        def refuse(*args, **kwargs):
            raise AppError("Transaction out: split payout allocation requires review", code="validation")

        monkeypatch.setattr(custody_tax_projection, "compile_finalized_tax_projection", refuse)

    class Builder:
        conn = None
        profile = book["profile"]

        def load_snapshot(self):
            return book

    monkeypatch.setattr(custody_journal, "_build_observer_factory", None)
    # Shadow also audits the frozen flow, which a patched build must skip;
    # the unpatched control has no database for that audit.
    monkeypatch.setattr(compare, "_INSTALLED_MODE", "shadow" if patched else "census")
    monkeypatch.setenv(compare.RECORDS_ENV, str(tmp_path / "records"))
    if patched:
        with pytest.raises(AppError, match="split payout"):
            compare._build(Builder(), finalized=True)
    else:
        compare._build(Builder(), finalized=True)
    row, = compare.read_records(tmp_path / "records")
    if patched:
        assert row["outcome"] == "patched"
        assert row["compared"] == []
        assert set(row["skipped"]) == set(compare.STAGES)
        assert row["reason"] == (
            compare.PATCHED_COMPUTE_PREFIX
            + "kassiber.core.custody_tax_projection.compile_finalized_tax_projection"
        )
    else:
        assert row["outcome"] == "result"
        assert set(row["compared"]) == set(compare.STAGES)
    lines, errors = compare.session_report([row])
    assert not errors
    assert f"patched-compute builds: {int(patched)} (all stages skipped)" in lines


@pytest.mark.parametrize("change", [{"reason": "injected"}, {"compared": ["quantity"]}, {"skipped": []}])
def test_session_rejects_invalid_patched_compute_skip(change):
    row = dict(outcome="patched", compared=[], skipped=list(compare.STAGES), reference=False,
               reason=compare.PATCHED_COMPUTE_PREFIX + "kassiber.core.custody_quantity.f")
    row.update(change)
    assert compare.session_report([row])[1]


def test_every_implemented_stage_is_compared(monkeypatch):
    import kassiber_tax
    expected = {"components": {"blockers": []}, "evidence": {}}
    response = {"schema_version": 1, "stages": {"components": {"blockers": ["injected"]}, "evidence": {}}, "skipped_stages": [stage for stage in compare.STAGES if stage not in expected]}
    monkeypatch.delenv(compare.INJECT_ENV, raising=False)
    monkeypatch.setattr(kassiber_tax, "custody", lambda request, **kwargs: json.dumps(response))
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
    def observe(request, **kwargs):
        calls.append(request)
        return original(request, **kwargs)
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
@pytest.mark.parametrize("stage", ["quantity", "finalized_projection"])
@pytest.mark.parametrize("rust_refuses", [False, True, "wrong_code", "wrong_stage"])
def test_quantity_refusal_compared_after_gaps(tmp_path, monkeypatch, mode, rust_refuses, stage):
    """Review fail-open reproducer: late Python errors cannot escape comparison."""
    import kassiber_tax
    from kassiber.core import custody_quantity_runtime, custody_tax_projection
    from kassiber.core.custody_journal import CustodyJournalBuilder
    from kassiber.db import open_db
    from tests.test_custody_native_transitions import seed_native_transition_book

    records = tmp_path / 'records.jsonl'
    monkeypatch.setenv(compare.RECORDS_ENV, str(records))
    monkeypatch.delenv(compare.INJECT_ENV, raising=False)
    def refuse(*args, **kwargs):
        raise ValueError('simulated Python quantity-stage refusal')
    monkeypatch.setattr(custody_quantity_runtime if stage == 'quantity' else custody_tax_projection,
                        'build_canonical_quantity_state' if stage == 'quantity' else 'compile_finalized_tax_projection', refuse)
    # This control compares Rust with the deliberately patched Python oracle.
    # Scope its baseline even when the enclosing session installed this mode.
    monkeypatch.setattr(compare, '_COMPUTE_FUNCTIONS', compare._capture_compute_functions())
    if rust_refuses:
        monkeypatch.setattr(kassiber_tax, 'custody', lambda _, **kwargs: json.dumps({'error': {'code': 'TypeError' if rust_refuses == 'wrong_code' else 'ValueError', 'stage': 'gaps' if rust_refuses == 'wrong_stage' else stage}}))
    monkeypatch.setattr(compare, '_COMPUTE_FUNCTIONS', compare._capture_compute_functions())
    previous = compare._INSTALLED_MODE
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
        if previous:
            compare.install_custody_mode(previous)
    assert compare._INSTALLED_MODE == previous
    rows = compare.read_records(records)
    assert rows[-1]['outcome'] == ('error' if rust_refuses is True else 'difference')
    if rust_refuses is True:
        assert rows[-1]['compared'] == [stage]
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
    monkeypatch.setattr(kassiber_tax, 'custody', lambda _, **kwargs: json.dumps({'error': {'code': 'ValueError', 'stage': 'quantity'}}))
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
@pytest.mark.parametrize('stage', ['quantity', 'finalized_projection', 'fusion'])
@pytest.mark.parametrize('injection', ['exact', 'different_exception', 'different_stage'])
def test_injected_failure_requires_exact_stage_and_exception(tmp_path, monkeypatch, mode, injection, stage):
    from kassiber.core import custody_quantity_runtime, custody_tax_projection
    from kassiber.core.custody_journal import CustodyJournalBuilder
    from tests.custody_snapshot_generator import snapshot
    book = snapshot(0)
    error = ValueError('intentional test failure')
    def refuse(*args, **kwargs):
        raise error
    if stage == 'fusion':
        from tests import custody_fusion_stage
        monkeypatch.setattr(custody_fusion_stage, 'python_fusion', refuse)
    else:
        monkeypatch.setattr(custody_quantity_runtime if stage == 'quantity' else custody_tax_projection,
                            'build_canonical_quantity_state' if stage == 'quantity' else 'compile_finalized_tax_projection', refuse)
    monkeypatch.setattr(compare, '_COMPUTE_FUNCTIONS', compare._capture_compute_functions())
    monkeypatch.setattr(CustodyJournalBuilder, 'load_snapshot', lambda _: book)
    monkeypatch.setenv(compare.RECORDS_ENV, str(tmp_path / 'records'))
    monkeypatch.setattr(compare, '_COMPUTE_FUNCTIONS', compare._capture_compute_functions())
    previous = compare._INSTALLED_MODE
    compare.install_custody_mode(mode)
    try:
        declared_error = ValueError(str(error)) if injection == 'different_exception' else error
        declared_stage = 'gaps' if injection == 'different_stage' else stage
        with compare.injected_stage_failure(declared_stage, declared_error):
            with pytest.raises(ValueError if injection == 'exact' else AssertionError):
                (CustodyJournalBuilder(None, book['profile']).build_custody_decisions() if stage == 'quantity'
                 else CustodyJournalBuilder(None, book['profile']).build_custody_projection())
        # The annotation must not leak to a later build.
        with pytest.raises(AssertionError):
            (CustodyJournalBuilder(None, book['profile']).build_custody_decisions() if stage == 'quantity'
                 else CustodyJournalBuilder(None, book['profile']).build_custody_projection())
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


def _finalized_refusal_book(root):
    """A book both runtimes still refuse at the finalized projection.

    Round 5 made an unusable transaction price a hold, so a NaN fan-out no
    longer refuses; an unreadable legacy payout value still does.
    """
    from tests.custody_projection_payout_books import books
    from tests.custody_snapshot import to_jsonable
    snapshot = next(snapshot for name, snapshot in books(root) if name == 'legacy_junk')
    return to_jsonable(snapshot, redact_config=False)


def test_real_finalized_refusal_has_complete_stage_inventory(tmp_path, monkeypatch):
    from kassiber.core import custody_journal
    from kassiber.errors import AppError
    book = _finalized_refusal_book(tmp_path / 'book')
    class Builder:
        conn = None
        profile = book['profile']
        def load_snapshot(self):
            return book
    monkeypatch.setattr(custody_journal, '_build_observer_factory', None)
    monkeypatch.setattr(compare, '_INSTALLED_MODE', 'census')
    monkeypatch.setenv(compare.RECORDS_ENV, str(tmp_path / 'records'))
    with pytest.raises(AppError):
        compare._build(Builder(), finalized=True)
    row = compare.read_records(tmp_path / 'records')[-1]
    assert row['outcome'] == 'error'
    assert row['refusing_stage'] == 'finalized_projection'
    assert row['compared'] == ['finalized_projection']
    assert set(row['skipped']) == set(compare.STAGES) - {'finalized_projection'}
    assert not compare.session_report([row])[1]


@pytest.mark.parametrize('difference', [None, 'code', 'stage', 'python_success'])
def test_earlier_stages_account_matching_finalized_refusal(tmp_path, monkeypatch, difference):
    from collections import Counter
    import kassiber_tax
    from tests.custody_snapshot_generator import snapshot
    book = _finalized_refusal_book(tmp_path / 'book')
    original = kassiber_tax.custody
    response = json.loads(original(json.dumps(book)))
    assert response['error']['stage'] == 'finalized_projection'
    assert response['error']['code'] == 'validation'
    if difference == 'code':
        response['error']['code'] = 'ValueError'
    if difference == 'stage':
        response['error']['stage'] = 'quantity'
    if difference == 'python_success':
        book = snapshot(0)
    def custody(request, *, finalized=True):
        return json.dumps(response) if finalized else original(request, finalized=False)
    monkeypatch.setattr(kassiber_tax, 'custody', custody)
    counts = Counter()
    if difference is None:
        stages, skipped = compare.rust_results(book, finalized_refusals=counts)
        assert 'interpretation' in stages and 'gaps' in stages
        assert skipped == ('finalized_projection', 'fusion')
        assert counts == {'finalized_projection:validation': 1}
    else:
        with pytest.raises(AssertionError):
            compare.rust_results(book, finalized_refusals=counts)
        assert not counts


def test_fusion_decision_only_skip_and_independent_projection():
    """Fusion consumes Rust projection, while the oracle consumes Python's."""
    import kassiber_tax
    from tests.custody_snapshot_generator import snapshot
    from tests.custody_finalized_stage import python_stages
    from tests.custody_fusion_stage import add_fusion
    book = snapshot(0)
    decisions = json.loads(kassiber_tax.custody(json.dumps(book), finalized=False))
    assert decisions['skipped_stages'] == ['finalized_projection', 'fusion']
    assert 'fusion' not in decisions['stages']
    expected = add_fusion(book, python_stages(book))
    actual = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert actual['skipped_stages'] == []
    compare.assert_same(expected['fusion'], actual['stages']['fusion'], 'fusion')
    assert expected['fusion']['outcome'] == 'request'
    assert expected['fusion']['request']['rows']


@pytest.mark.parametrize(("column", "value"), [
    ("amount", 1.5), ("fee", "unavailable"), ("amount", b"77"), ("note", b"\xff\x00"),
])
def test_legacy_scalars_compare_through_the_loaded_path(column, value):
    """Journal builds keep a book's REAL, TEXT and BLOB scalars as loaded. The
    strict v1 adapter refuses or cannot carry them, so the comparison sends
    such a book through the binding's loaded encoding instead."""
    from kassiber.core.custody_compute import compute_custody_projection

    snapshot = json.loads((ROOT / "tests/fixtures/custody_projection_stage.json").read_text())[0]["input"]
    assert compare._carried_by_v1(snapshot)
    snapshot["transactions"][0][column] = value
    assert not compare._carried_by_v1(snapshot)
    stages = {}
    try:
        compute_custody_projection(snapshot, stage_observer=lambda name, stage: stages.setdefault(name, stage))
    except Exception as error:
        stage = next(name for name in compare.STAGES if name not in stages)
        assert compare.rust_results(snapshot, python_error=error, python_stage=stage) == ({}, compare.STAGES)
    else:
        compared, _ = compare.compare_rust(snapshot, None, python_stages=stages)
        assert set(compared) == set(compare.STAGES)


def test_loaded_encoding_tags_bytes_and_escapes_literal_tags():
    value = {"note": b"\xff\x00", "raw": {"$kassiber_bytes": "ff00"}, "rows": [{"$kassiber_dict": {}}]}
    encoded = compare._loaded_encoding(value)
    assert encoded == {"note": {"$kassiber_bytes": "ff00"},
                       "raw": {"$kassiber_dict": {"$kassiber_bytes": "ff00"}},
                       "rows": [{"$kassiber_dict": {"$kassiber_dict": {}}}]}
    assert compare._loaded_decoding(json.loads(json.dumps(encoded))) == value
