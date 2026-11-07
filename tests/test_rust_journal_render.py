"""The R4-11 oracle retains every byte and each generated identity."""
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_port.fixture_compare import assert_rebuilt

from tests.rust_port.journal_scalar_vectors import PATH as SCALAR_PATH, vectors as scalar_vectors, byte_vectors, render_vectors as scalar_render_vectors, refusal_vectors as scalar_refusal_vectors
from tests.rust_port.journal_render_vectors import PATH, vectors, machine_msat_vectors, legacy_float_vectors, response_refusal_vectors
from tests.rust_port.journal_native_builder_vectors import PATH as BUILDER_PATH, vectors as builder_vectors
from tests.rust_port.journal_native_graph_vectors import PATH as GRAPH_PATH, vectors as graph_vectors
from tests.rust_port.journal_native_vectors import PATH as NATIVE_PATH, vectors as native_vectors
from tests.rust_port.journal_nonfinite_vectors import PATH as NONFINITE_PATH, vectors as nonfinite_vectors


def test_render_vectors_rebuild_from_python_build():
    assert_rebuilt(json.dumps(json.loads(NONFINITE_PATH.read_text()), sort_keys=True, ensure_ascii=True), json.dumps(nonfinite_vectors(), sort_keys=True, ensure_ascii=True), NONFINITE_PATH.name)
    assert_rebuilt(json.dumps(json.loads(GRAPH_PATH.read_text()), sort_keys=True, ensure_ascii=True), json.dumps(graph_vectors(), sort_keys=True, ensure_ascii=True), GRAPH_PATH.name)
    assert_rebuilt(json.dumps(json.loads(BUILDER_PATH.read_text()), sort_keys=True, ensure_ascii=True), json.dumps(builder_vectors(), sort_keys=True, ensure_ascii=True), BUILDER_PATH.name)
    assert_rebuilt(json.loads(NATIVE_PATH.read_text()), native_vectors(), NATIVE_PATH.name)
    assert_rebuilt(json.loads(SCALAR_PATH.read_text()), scalar_vectors(), SCALAR_PATH.name)
    assert_rebuilt(json.loads(SCALAR_PATH.with_name("render-scalar-bytes.json").read_text()), byte_vectors(), 'render-scalar-bytes.json')
    assert_rebuilt(json.loads(SCALAR_PATH.with_name("render-scalar-refusals.json").read_text()), scalar_refusal_vectors(), 'render-scalar-refusals.json')
    assert_rebuilt(json.loads(SCALAR_PATH.with_name("render-scalar-coercions.json").read_text()), scalar_render_vectors(), 'render-scalar-coercions.json')
    assert_rebuilt(json.loads(PATH.read_text()), vectors(), PATH.name)
    assert_rebuilt(json.loads(PATH.with_name('msat-machine.json').read_text()), [list(v) for v in machine_msat_vectors()], 'msat-machine.json')
    assert_rebuilt(json.loads(PATH.with_name('render-legacy-floats.json').read_text()), legacy_float_vectors(), 'render-legacy-floats.json')
    assert_rebuilt(json.loads(PATH.with_name('render-response-refusals.json').read_text()), response_refusal_vectors(), 'render-response-refusals.json')


def test_uuid_tapes_are_bijections_and_cover_every_rendered_identity():
    for case in json.loads(PATH.read_text()):
        pairs = case['uuid_mapping']
        assert len({p['python'] for p in pairs}) == len(pairs)
        assert len({p['rust'] for p in pairs}) == len(pairs)
        assert case['entry_ids'] == [p['python'] for p in pairs]
        if 'state' in case['expected']:
            state = json.loads(case['expected']['state'])
            assert [e['id'] for e in state['entries']] == case['entry_ids']
            assert 'created_at' not in state  # Created only by R4-12 persistence.
            assert case['clock'] is not None


@pytest.mark.parametrize('mode', ['census', 'shadow'])
@pytest.mark.parametrize('first_import', [
    'tests.rust_port.journal_render_oracle', 'kassiber.core.custody_journal',
])
def test_render_capture_child_import_orders(tmp_path, mode, first_import):
    root = Path(__file__).resolve().parents[1]
    env = dict(os.environ, KASSIBER_TEST_CUSTODY_ENGINE=mode,
               KASSIBER_TEST_JOURNAL_RENDER_DIR=str(tmp_path),
               PYTHONPATH=os.pathsep.join([str(root / 'tests/_child_hooks'), str(root)]))
    script = f'''
import importlib
import json
importlib.import_module({first_import!r})
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.core.custody_compute import compute_custody_projection
from tests.custody_snapshot_generator import snapshot
book = snapshot(0)
custody = compute_custody_projection(book)
from tests.rust_port.journal_render_oracle import capture_projection
captured = capture_projection(book['profile'], custody)
assert len(captured['entry_ids']) == len(json.loads(captured['expected']['state'])['entries'])
builder = CustodyJournalBuilder(None, book['profile'], as_of=custody.as_of)
builder.build_custody_projection = lambda: custody
builder.build()
'''
    child = subprocess.run([sys.executable, '-c', script], env=env, cwd=root,
                           capture_output=True, text=True, timeout=60)
    assert child.returncode == 0, child.stderr
    records = list(tmp_path.glob('*.json'))
    assert len(records) == 1
    record = json.loads(records[0].read_text())
    assert record['response'] is not None
    assert record['entry_ids']
    assert 'state' in record['expected']
