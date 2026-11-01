"""Python-derived scalar witnesses for the native journal boundary."""
import json
from pathlib import Path

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/render-scalars.json'


def vectors():
    values = [None, True, False, 0, -1, 2**128, -(2**128), 0.0, -0.0,
              1e-300, 1e300, float('nan'), float('inf'), -float('inf'),
              '', 'literal\\ud800', '\ud800', '\udfff', '\ud800x\udfff',
              '\U0001f600', "a'\"\n", {'$float': 'nan'}, {'$decimal': '1.0'},
              [None, True, -0.0, '\ud800'],
              {'\udfff': '\ud800', 'literal\\udfff': [True, 2**128]},
              {'b': 2, 'a': 1}]
    from tests.custody_snapshot import to_jsonable
    result = []
    for i, value in enumerate(values):
        result.append(dict(name=f'scalar-{i}', raw=json.dumps(value, ensure_ascii=True),
                           encoded=json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(',', ':')),
                           text_json=json.dumps(str(value), ensure_ascii=True), representation=repr(value), truth=bool(value),
                           kind=type(value).__name__,
                           snapshot=json.dumps(to_jsonable(value), sort_keys=True, ensure_ascii=True, separators=(',', ':')),
                           serde=i not in {11, 12, 13, 16, 17, 18, 23, 24, 25}))
    return result


def byte_vectors():
    from tests.custody_snapshot import to_jsonable
    result = []
    for value in (b'', b'0', b'1.5', b'\x00\xff', b"'\"\\\n"):
        try:
            json.dumps(value)
        except TypeError as exc:
            encoding_error = str(exc)
        result.append(dict(hex=value.hex(), kind=type(value).__name__, truth=bool(value),
                           text=str(value), representation=repr(value),
                           encoding_error=encoding_error,
                           snapshot=json.dumps(to_jsonable(value), sort_keys=True,
                                               ensure_ascii=True, separators=(',', ':'))))
    return result


def render_vectors():
    from dataclasses import replace
    from unittest.mock import patch
    import uuid
    from kassiber.core.custody_compute import compute_custody_projection
    from tests.custody_snapshot_generator import snapshot
    from tests.rust_port.journal_render_oracle import capture_projection
    from tests.rust_port.journal_render_vectors import clean_record, seeded_ids

    book = snapshot(0)
    custody = compute_custody_projection(book)
    response = capture_projection(book['profile'], custody)['response']
    result = []
    values = [None, False, True, 0, -0.0, 2**128, 1e-300,
              ['scalar', None, True, 2**128], {'a': None, 'b': [True, 'text']}]
    for field in ('gains_algorithm', 'journal_transaction_id'):
        for i, value in enumerate(values):
            name = f'{field}-{i}'
            profile = dict(book['profile'])
            c = custody
            if field == 'gains_algorithm':
                profile[field] = value
            else:
                c = replace(custody, finalized_tax_projection=replace(
                    custody.finalized_tax_projection,
                    rows=tuple(dict(row, journal_transaction_id=value)
                               for row in custody.finalized_tax_projection.rows)))
            with patch.object(uuid, 'uuid4', seeded_ids(name)):
                record = capture_projection(profile, c, response_override=response)
            result.append(dict(name=name, group='scalar', **clean_record(record)))
    return result


def refusal_vectors():
    result = []
    for raw in ('[1,]', '{"a":}', 'NaN garbage', '01', '"\\ud800', '9' * 4301):
        try:
            json.loads(raw)
        except Exception as exc:
            from tests.rust_port.journal_render_oracle import refusal
            result.append(dict(raw=raw, expected=refusal(exc)))
        else:
            raise AssertionError(raw)
    return result


if __name__ == '__main__':
    for name, records in [('render-scalars.json', vectors()),
                          ('render-scalar-bytes.json', byte_vectors()),
                          ('render-scalar-coercions.json', render_vectors()),
                          ('render-scalar-refusals.json', refusal_vectors())]:
        PATH.with_name(name).write_text(json.dumps(records, ensure_ascii=True, indent=2) + '\n')
