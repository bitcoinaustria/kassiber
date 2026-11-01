import copy
import io
import json
from contextlib import redirect_stdout
from pathlib import Path

from kassiber.cli.main import _select_document_import_rows_for_cli
from kassiber.core import document_import as reference
from crates.wallets.scripts.generate_document_vectors import base, SOURCE_HASH, error

DEST = Path(__file__).resolve().parents[2] / 'cli/tests/data/document_review.json'


def vectors():
    ready = reference._draft_rows({'rows': [base(), {**base(), 'amount_btc': '0.02'}]},
        source_hash=SOURCE_HASH, threshold=reference.DEFAULT_CONFIDENCE_THRESHOLD,
        expected_fiat_currency='EUR', allowed_pages=[1])
    draft = dict(source={'sha256': SOURCE_HASH}, rows=ready, confidence_threshold=0.78)
    scenarios = []
    for response in ['1\n', '2,1,2,01\n', ' １ , ٢ \n', '1', '\n', '', ' \t\u001c\n', '0\n', '3\n', '-1\n', '+1\n', '1,\n', '1 2\n', '1_0\n', '1.0\n', '²\n', '①\n', '½\n', '9' * 100 + '\n', '1' * 4301 + '\n']:
        scenarios.append((repr(response[:32]), draft, response, False, True, False))
    scenarios.append(('interrupt', draft, '', False, True, True))
    scenarios.append(('noninteractive', draft, '1\n', False, False, False))
    for changes in [{}, {'fee_btc': None}, {'confidence': 0.5}, {'amount_btc': '0.000000000015'}, {'fiat_currency': 'USD'}, {'cell_confidences': {}}, {'counterparty': 'Desk\ud800', 'description': 'café😀\udfff'}]:
        rows = reference._draft_rows({'rows': [{**base(), **changes}]}, source_hash=SOURCE_HASH,
            threshold=reference.DEFAULT_CONFIDENCE_THRESHOLD, expected_fiat_currency='EUR', allowed_pages=[1])
        for include in [False, True]:
            scenarios.append((repr(changes), {**draft, 'rows': rows}, '1\n', include, True, False))
    for name, changed in [('empty', {**draft, 'rows': []}), ('invalid-source', {**draft, 'source': []}), ('invalid-rows', {**draft, 'rows': {}}), ('threshold', {**draft, 'confidence_threshold': 2}), ('nonobjects', {**draft, 'rows': [None, *ready]})]:
        scenarios.append((name, changed, '2\n', False, True, False))
    for confidence in [True, False, None, '0.5', 0.785, 0.795, 0.005, float('nan'), float('inf')]:
        changed = copy.deepcopy(draft)
        changed['rows'][0]['confidence'] = confidence
        scenarios.append((f'confidence-{confidence}', changed, '1\n', True, True, False))
    result = []
    for name, draft, stdin, include, interactive, interrupt in scenarios:
        stdout, stderr = io.StringIO(), io.StringIO()
        def input_fn(prompt):
            print(prompt, end='')
            if interrupt:
                raise KeyboardInterrupt
            if not stdin:
                raise EOFError
            return stdin.rstrip('\n')
        case = dict(name=name, draft=draft, stdin=stdin, include=include, interactive=interactive, interrupt=interrupt)
        with redirect_stdout(stdout):
            try:
                case['selected'] = _select_document_import_rows_for_cli(draft,
                    expected_fiat_currency='EUR', include_quarantined=include,
                    interactive=interactive, input_fn=input_fn, output=stderr)
            except Exception as exc:
                case['error'] = error(exc)
        case.update(stdout=stdout.getvalue(), stderr=stderr.getvalue().encode('utf-8', errors='backslashreplace').decode())
        result.append(case)
    return result


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, sort_keys=True, indent=2) + '\n')
