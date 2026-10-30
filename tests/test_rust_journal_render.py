"""The R4-11 oracle retains every byte and each generated identity."""
import json
from pathlib import Path

from tests.rust_port.journal_render_vectors import PATH, vectors


def test_render_vectors_rebuild_from_python_build():
    assert json.loads(PATH.read_text()) == vectors()


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
