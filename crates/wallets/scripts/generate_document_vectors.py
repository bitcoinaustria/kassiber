import copy
import hashlib
import json
from pathlib import Path

from kassiber.core import document_import as ref

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_document_rows.json'
SOURCE = b'Kassiber synthetic local OCR evidence\n'
SOURCE_HASH = hashlib.sha256(SOURCE).hexdigest()


def base():
    return dict(occurred_at='2026-01-02', direction='inbound', asset='BTC',
        amount_btc='0.01000000', fee_btc='0', fiat_currency='EUR', fiat_value='500.00',
        counterparty='OTC Desk', description='Receipt row', confidence=0.94,
        cell_confidences={key: 0.99 for key in ref._OCR_CONFIDENCE_FIELDS},
        source_region=dict(page=1, x=0.1, y=0.2, width=0.7, height=0.1, unit='relative'),
        evidence_text='02.01.2026 BTC 0.01000000 EUR 500 OTC Desk')


def inputs():
    yield 'ready', {'rows': [base()]}, None
    for key in ['amount_btc', 'fee_btc']:
        yield f'{key}-fractional-msat-above-one', {'rows': [{**base(), key: '0.000000000015'}]}, None
    yield 'empty', {'rows': []}, None
    yield 'non-objects', {'rows': [None, [], 'row', 0, True, base()]}, None
    for raw in [None, {}, 3, 'row']:
        yield f'bad-rows-{raw!r}', {'rows': raw}, None
    yield 'no-rows', {'a': 1, 'z': 2}, None
    yield 'too-many', {'rows': [base()] * 501}, None
    values = [None, '', False, True, 0, 1, -1, 0.01, 1e-11, 1e-12,
        '0.00000000001', '0.000000000001', '92233720.36854775807', '92233720.36854775808',
        '-0', '0E-9999999', '1E-9999999', '1E+9999999', '1' * 513,
        'NaN', 'Infinity', '-Infinity', 'sNaN', '١.٢', '1_000.00', '0,01234', '1,234',
        '0,123', ',123', '1.234,56', '1,234.56', '1,234,567', '1\u00a0234,56', '1 234,56',
        '1\ud800', '1\u2000', [], {}, '0.' + '0' * 500 + '1']
    for key in ['amount_btc', 'fee_btc', 'fiat_value', 'fiat_rate', 'confidence']:
        for index, value in enumerate(values):
            row = base(); row[key] = value
            yield f'{key}-{index}', {'rows': [row]}, None
    for key, values in {
        'occurred_at': [None, '', 0, True, '01.02.2026', '2026-01-02T12:34:56+02:00', '20260102', '2026-W01-5', '2026-01-02T12:34:56.345Z'],
        'direction': [None, '', 'buy', 'SELL', 'received', 'withdraw', 'incoming', 'move', True, 3],
        'asset': [None, '', 'BTC', 'lbtc', 'ETH', 'eur', 'B\ud800TC', 'b' * 40],
        'fiat_currency': [None, '', 'eur', 'USD', 'E\ud800UR', 'e' * 20],
        'source_region': [None, [], {}, {'page': 2}, {'page': 0}, {'page': 1.5}, {'page': '١'}, {'page': 1, 'x': '1000000.00000000000000001', 'unit': 'x' * 40}],
        'counterparty': [None, '', True, 3, 'ü' * 600, 'Counter\ud800party'],
        'description': [None, '', {}, ['a'], 'd' * 4100],
        'evidence_text': [None, 3, 'exact\ud800text', 'e' * 4100],
        'cell_confidences': [None, [], {}, {'amount_btc': '0.77999999999999999999999'}, {'amount_btc': 5, 'secret': 1}, {key: 0 for key in ref._OCR_CONFIDENCE_FIELDS}],
    }.items():
        for index, value in enumerate(values):
            row = base(); row[key] = value
            yield f'{key}-{index}', {'rows': [row]}, None
    for index, changes in enumerate([
        {'amount_btc': None, 'amount_crypto': '0.02'},
        {'amount_btc': None, 'amount': '0.02'},
        {'amount_btc': None, 'amount': '0.02', 'asset': None},
        {'amount_btc': '0.02', 'asset': None},
        {'fee_btc': None, 'fee_crypto': '0.0001'},
        {'occurred_at': None, 'date': '2026-02-03'},
        {'id': 'model-id'}, {'id': None, 'row_id': 'fallback-id'},
    ]):
        yield f'aliases-{index}', {'rows': [{**base(), **changes}]}, None
    for index, threshold in enumerate([0, 1, '0,78', '0.940000000000000000001', -1, 'NaN', '1,234', True, False]):
        yield f'threshold-{index}', {'rows': [base()]}, threshold
    row1 = base(); row2 = base()
    row2.update(source_region={'page': 1, 'x': 0.7}, evidence_text='Second identical economic row')
    yield 'duplicates', {'rows': [row2, row1, row2]}, None
    yield 'duplicates-reordered', {'rows': [row1, row2, row2]}, None


def error(exc):
    return dict(code=getattr(exc, 'code', 'internal_error'), message=str(exc),
        hint=getattr(exc, 'hint', None), details=getattr(exc, 'details', None), retryable=getattr(exc, 'retryable', False))


def vectors():
    result = []
    for name, payload, raw_threshold in inputs():
        case = dict(name=name, payload=payload, threshold=raw_threshold, source_hash=SOURCE_HASH, currency='EUR', allowed_pages=[1])
        try:
            threshold = ref._confidence_threshold(raw_threshold)
            rows = ref._draft_rows(payload, threshold=threshold, source_hash=SOURCE_HASH, expected_fiat_currency='EUR', allowed_pages=[1])
            case['rows'] = rows
            case['reviewed'] = [ref._import_record_from_draft_row(row, source_hash=SOURCE_HASH, expected_fiat_currency='EUR') for row in rows]
            case['confidence_flags'] = [ref._confidence_review_flags(row, threshold=threshold) for row in rows]
        except Exception as exc:
            case['error'] = error(exc)
        result.append(case)
    return result



def review_vectors():
    ready = ref._draft_rows({'rows': [base()]}, source_hash=SOURCE_HASH,
        threshold=ref.DEFAULT_CONFIDENCE_THRESHOLD, expected_fiat_currency='EUR', allowed_pages=[1])[0]
    variants = [('ready', ready)]
    for key, values in {
        'id': ['', 'forged', ready['id'].replace(SOURCE_HASH[:16], '0' * 16),
            ready['id'][:-3] + '١٢٣', ready['id'][:-3] + '\U0001e4f1\U0001e4f2\U0001e4f3', ready['id'] + '\n'],
        'status': ['ready', 'quarantined'],
        'flags': [[], ['low_row_confidence'], ['invalid_source_page'], ['fee_not_representable'], 'missing_amount'],
        'confidence': [None, 0, .1, .78, 1, 'NaN'],
        'cell_confidences': [{}, {'amount_btc': 0}, {**ready['cell_confidences'], 'amount_btc': .1}],
        'import_record': [None, {'amount': '999', 'kind': 'income', 'ownership': True}],
        'source_region': [None, {'page': '1.0'}, {'page': 2}, {'page': -1}],
    }.items():
        for index, value in enumerate(values):
            row = copy.deepcopy(ready); row[key] = value
            variants.append((f'{key}-{index}', row))
    for key, values in {
        'occurred_at': [None, '2026-02-30', '2026-02-01'],
        'direction': [None, 'SELL', 'transfer'], 'asset': ['LBTC', 'ETH', None],
        'amount_btc': ['-1', '0', '0.000000000001', '0.000000000015', '92233720.36854775808', '1E-9999999'],
        'fee_btc': [None, '-1', '0.000000000001', '0.000000000015', 'NaN'],
        'fiat_currency': [None, 'USD', 'eur'], 'fiat_value': ['-1', '0', '1E9999999', '1E-9999999', 'NaN'],
        'fiat_rate': ['-1', '0', '1E9999999', '1E-9999999'], 'fee_defaulted': [True],
    }.items():
        for index, value in enumerate(values):
            row = copy.deepcopy(ready); row['record'][key] = value
            variants.append((f'record-{key}-{index}', row))
    result = []
    for name, row in variants:
        for override in [False, True]:
            case = dict(name=f'{name}-override-{override}', rows=[row], selected=None,
                include_quarantined=override, threshold=None, source_hash=SOURCE_HASH, currency='EUR')
            records, skipped = ref._import_records_from_rows(case['rows'], include_quarantined=override,
                selected_row_ids=None, source_hash=SOURCE_HASH, expected_fiat_currency='EUR')
            case.update(records=records, skipped=skipped)
            result.append(case)
    for selected in [[], ['missing'], [ready['id']], [ready['id'], ready['id']]]:
        records, skipped = ref._import_records_from_rows([ready], include_quarantined=False,
            selected_row_ids=selected, source_hash=SOURCE_HASH, expected_fiat_currency='EUR')
        result.append(dict(name=f'selection-{len(result)}', rows=[ready], selected=selected,
            include_quarantined=False, threshold=None, source_hash=SOURCE_HASH, currency='EUR',
            records=records, skipped=skipped))
    return result


if __name__ == "__main__":
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, indent=2) + "\n")
    DEST.with_name("python_document_review.json").write_text(json.dumps(review_vectors(), ensure_ascii=True, indent=2) + "\n")
