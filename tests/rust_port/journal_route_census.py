"""Refusal-census operands for registered journal routes.

Each base is the first generated book of one custody scenario. A perturbation
then edits one stored value in a way the Python reference may accept and the
native stage transport may not: NaN or infinite JSON numbers, lone-surrogate
JSON strings and keys, undecodable TEXT, BLOB-typed profile settings, and
malformed scalars, alone and in pairs whose refusal precedence matters.
The perturbation runs after the book is created, so both runtimes open the
same bytes. Operands carry no private recording data.
"""
from __future__ import annotations

import json

# JSON text columns the journal loader, pre-build or setup resolver reads.
JSON_COLUMNS = (
    ('transactions', 'raw_json'),
    ('wallets', 'config_json'),
    ('custody_components', 'evidence_json'),
    ('custody_components', 'conversion_metadata_json'),
    ('custody_authored_evidence_snapshots', 'payload_json'),
    ('custody_authored_migration_issues', 'details_json'),
    ('custody_authored_migration_issues', 'transaction_ids_json'),
    ('profiles', 'ownership_review_counts_json'),
    ('chain_observation_provenance', 'observer_kinds_json'),
    ('chain_observation_provenance', 'observer_ids_json'),
)
JSON_VARIANTS = ('nan-member', 'nan-leaf', 'infinity-leaf', 'surrogate-member', 'surrogate-key', 'surrogate-leaf')
TEXT_COLUMNS = (
    ('transactions', 'note'), ('transactions', 'description'), ('transactions', 'kind'),
    ('transactions', 'asset'), ('wallets', 'label'), ('wallets', 'kind'),
)
PROFILE_TEXT = ('label', 'fiat_currency', 'tax_country', 'gains_algorithm', 'cost_basis_pool_scope',
                'last_processed_at', 'ownership_review_counts_json')
PROFILE_INTEGER = ('tax_long_term_days', 'require_coarse_review', 'bitcoin_rail_carrying_value',
                   'journal_input_version', 'last_processed_input_version', 'last_processed_tx_count')
SCALARS = (
    ('transactions', 'amount', ("'12a'", '1.5', "X'3132'", "'9223372036854775808'")),
    ('transactions', 'fee', ("'x'", '0.5', "X'30'")),
    ('transactions', 'occurred_at', ('20240101', "'not-a-date'", "X'32303234'")),
    ('transactions', 'fiat_rate', ("'abc'", "X'31'", '1e999', '-1e999')),
    ('transactions', 'fiat_rate_exact', ("'NaN'", "'Infinity'", "'1e999999'", "X'31'", '7')),
    ('transactions', 'excluded', ("'0'", '0.0', "X'30'")),
    ('transactions', 'amount_includes_fee', ("'1'", '2', "X'31'")),
    ('custody_components', 'revision', ("'1'", '1.0', "X'31'")),
    ('custody_components', 'expected_leg_count', ("'2'", '2.5', "X'32'")),
)
# Two independently refused values in different stages; the reported one
# must follow Python's evaluation order.
PRECEDENCE = (
    (('profiles', 'gains_algorithm', "'invalid-method'"), ('profiles', 'tax_country', "'unsupported'")),
    (('profiles', 'gains_algorithm', "'invalid-method'"), ('transactions', 'amount', "'12a'")),
    (('profiles', 'tax_country', "'unsupported'"), ('transactions', 'occurred_at', "'not-a-date'")),
    (('profiles', 'tax_country', 'CAST(tax_country AS BLOB)'), ('profiles', 'gains_algorithm', "'invalid-method'")),
    (('transactions', 'amount', "'12a'"), ('transactions', 'raw_json', "'{\"x\": NaN}'")),
    (('transactions', 'fiat_rate_exact', "'NaN'"), ('wallets', 'config_json', "'{\"\\\\ud800\": 1}'")),
    (('custody_components', 'evidence_json', "'{\"x\": NaN}'"), ('profiles', 'gains_algorithm', "'invalid-method'")),
    (('custody_components', 'revision', "'1'"), ('transactions', 'amount', '1.5')),
)


def _leaf(value, predicate, replacement):
    """Replace the first leaf satisfying predicate; return (value, replaced)."""
    if isinstance(value, dict):
        out = {}
        done = False
        for key, item in value.items():
            if not done:
                item, done = _leaf(item, predicate, replacement)
            out[key] = item
        return out, done
    if isinstance(value, list):
        out = []
        done = False
        for item in value:
            if not done:
                item, done = _leaf(item, predicate, replacement)
            out.append(item)
        return out, done
    if predicate(value):
        return replacement, True
    return value, False


class _Raw(str):
    pass


def _encode(value):
    """JSON with raw tokens (NaN, Infinity, lone-surrogate escapes) kept literal."""
    if isinstance(value, _Raw):
        return str(value)
    if isinstance(value, dict):
        return '{' + ', '.join(_encode_key(k) + ': ' + _encode(v) for k, v in value.items()) + '}'
    if isinstance(value, list):
        return '[' + ', '.join(_encode(v) for v in value) + ']'
    return json.dumps(value)


def _encode_key(key):
    return str(key) if isinstance(key, _Raw) else json.dumps(key)


def perturb_json(text, variant):
    """Return the perturbed JSON text, or None when the variant cannot apply."""
    try:
        value = json.loads(text)
    except (TypeError, ValueError):
        return None
    numeric = lambda v: isinstance(v, (int, float)) and not isinstance(v, bool)
    if variant == 'nan-member':
        if isinstance(value, dict):
            return _encode({**value, 'census': _Raw('NaN')})
        if isinstance(value, list):
            return _encode([*value, _Raw('NaN')])
        return 'NaN'
    if variant in ('nan-leaf', 'infinity-leaf'):
        changed, done = _leaf(value, numeric, _Raw('NaN' if variant == 'nan-leaf' else 'Infinity'))
        return _encode(changed) if done else None
    if variant == 'surrogate-member':
        if isinstance(value, dict):
            return _encode({**value, 'census': _Raw('"\\ud800"')})
        if isinstance(value, list):
            return _encode([*value, _Raw('"\\ud800"')])
        return '"\\ud800"'
    if variant == 'surrogate-key':
        if isinstance(value, dict):
            return _encode({**value, _Raw('"\\udc00"'): 1})
        return None
    if variant == 'surrogate-leaf':
        changed, done = _leaf(value, lambda v: isinstance(v, str), _Raw('"\\ud800"'))
        return _encode(changed) if done else None
    raise ValueError(variant)


def _first(conn, table, column):
    try:
        return conn.execute(
            f'SELECT rowid, "{column}" FROM "{table}" WHERE "{column}" IS NOT NULL ORDER BY rowid LIMIT 1').fetchone()
    except Exception:
        return None


def perturbations(conn):
    """Yield (name, sql statements) applicable to this book."""
    for table, column in JSON_COLUMNS:
        row = _first(conn, table, column)
        if row is None:
            continue
        for variant in JSON_VARIANTS:
            text = perturb_json(row[1], variant)
            if text is not None:
                yield f'json:{table}.{column}:{variant}', [
                    (f'UPDATE "{table}" SET "{column}"=? WHERE rowid=?', (text, row[0]))]
    for table, column in TEXT_COLUMNS:
        row = _first(conn, table, column)
        if row is not None:
            yield f'text:{table}.{column}:invalid-utf8', [
                (f'UPDATE "{table}" SET "{column}"=CAST(X\'EDA080\' AS TEXT) WHERE rowid=?', (row[0],))]
            yield f'text:{table}.{column}:blob', [
                (f'UPDATE "{table}" SET "{column}"=CAST("{column}" AS BLOB) WHERE rowid=?', (row[0],))]
    for column in PROFILE_TEXT:
        row = _first(conn, 'profiles', column)
        if row is not None:
            yield f'profile:{column}:blob', [
                (f'UPDATE profiles SET "{column}"=CAST("{column}" AS BLOB) WHERE rowid=?', (row[0],))]
    for column in PROFILE_INTEGER:
        for name, literal in (('text', "CAST(\"{c}\" AS TEXT)"), ('real', "\"{c}\" + 0.5"),
                              ('blob', "CAST(CAST(\"{c}\" AS TEXT) AS BLOB)")):
            yield f'profile:{column}:{name}', [
                (f'UPDATE profiles SET "{column}"={literal.format(c=column)}', ())]
    for table, column, literals in SCALARS:
        row = _first(conn, table, column)
        if row is None:
            continue
        for number, literal in enumerate(literals):
            yield f'scalar:{table}.{column}:{number}', [
                (f'UPDATE "{table}" SET "{column}"={literal} WHERE rowid=?', (row[0],))]
    for number, pair in enumerate(PRECEDENCE):
        statements = []
        for table, column, literal in pair:
            row = _first(conn, table, column)
            if row is None:
                break
            statements.append((f'UPDATE "{table}" SET "{column}"={literal} WHERE rowid=?', (row[0],)))
        else:
            yield f'precedence:{number}', statements


def bases():
    """The first generated book of every custody scenario (515 books)."""
    from tests.custody_gap_stage import suites
    for suite, scenarios, generate in suites():
        for seed in range(len(scenarios)):
            yield suite, scenarios[seed], seed, generate
