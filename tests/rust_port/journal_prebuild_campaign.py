"""Synthetic database operands for whole-stage pre-build comparisons.

Snapshot-only ownership indexes, receipt verdicts, and derived join columns are
not database facts. This adapter imports stored columns only; it never turns
those projections into inventory or legacy review authority. Full-book replay
uses the recorder's complete dumps instead of this adapter.
"""
from __future__ import annotations

from collections import Counter
from functools import lru_cache
import hashlib
import json
import sqlite3

from tests.rust_port.journal_pipeline_oracle import capture
from tests.rust_port.journal_prebuild_oracle import ROOT, CLOCK, seed, rate

PATH = ROOT / 'crates/journals/tests/data/prebuild-campaign.json'


@lru_cache(maxsize=2)
def template(relaxed=False):
    conn = sqlite3.connect(':memory:')
    seed(conn)
    triggers = conn.execute("SELECT name,sql FROM sqlite_master WHERE type='trigger'").fetchall()
    for name, _ in triggers:
        conn.execute(f'DROP TRIGGER "{name}"')
    if relaxed:
        # Malformed snapshot witnesses may violate the current physical schema.
        # Preserve their values instead of inventing valid economic terms or
        # changing duplicate ordinals. Both runtimes receive this same schema.
        for table in ('custody_component_legs', 'custody_component_allocations',
                      'custody_component_economic_terms', 'custody_component_evidence_commitments'):
            columns = conn.execute(f'PRAGMA table_info("{table}")').fetchall()
            fields = []
            for _, name, kind, _, default, primary in columns:
                field = f'"{name}" {kind}'
                if primary:
                    field += ' PRIMARY KEY'
                if default is not None:
                    field += ' DEFAULT ' + default
                fields.append(field)
            conn.execute(f'DROP TABLE "{table}"')
            conn.execute(f'CREATE TABLE "{table}" ({",".join(fields)})')
    for table in ('wallets', 'profiles', 'workspaces'):
        conn.execute(f'DELETE FROM {table}')
    conn.commit()
    image = conn.serialize()
    conn.close()
    return image, triggers


def materialize(snapshot, *, relaxed=False):
    conn = sqlite3.connect(':memory:')
    conn.row_factory = sqlite3.Row
    try:
        return _materialize(conn, snapshot, relaxed=relaxed)
    except BaseException:
        conn.close()
        raise


def _materialize(conn, snapshot, *, relaxed=False):
    image, triggers = template(relaxed)
    conn.deserialize(image)
    conn.execute('PRAGMA foreign_keys=OFF')
    conn.execute('PRAGMA ignore_check_constraints=ON')
    profile = snapshot['profile']
    scope = dict(workspace_id=profile['workspace_id'], profile_id=profile['id'])
    skipped = Counter()

    def insert(table, row, **defaults):
        columns = {r[1] for r in conn.execute(f'PRAGMA table_info("{table}")')}
        skipped.update(f'{table}.{key}' for key in row if key not in columns)
        values = {k: v for k, v in dict(defaults, **row).items() if k in columns}
        values = {k: json.dumps(v, ensure_ascii=True) if isinstance(v, (dict, list)) else v for k, v in values.items()}
        names = ','.join(f'"{k}"' for k in values)
        conn.execute(f'INSERT INTO "{table}"({names}) VALUES({",".join("?" for _ in values)})', tuple(values.values()))

    insert('workspaces', dict(id=scope['workspace_id'], label='Synthetic campaign', created_at=CLOCK))
    insert('profiles', profile)
    wallets = {row['id']: dict(row) for row in snapshot.get('anchor_wallets', [])}
    for row in snapshot['wallets']:
        wallets[row['id']] = dict(wallets.get(row['id'], {}), **row)
    transactions = {row['id']: row for row in snapshot['transactions']}
    for row in snapshot.get('anchors', {}).values():
        if row:
            transactions.setdefault(row['id'], row)
    for row in transactions.values():
        if row.get('wallet_id') not in wallets:
            wallets[row['wallet_id']] = dict(id=row['wallet_id'], label=row.get('wallet_label') or row['wallet_id'],
                                           kind=row.get('wallet_kind') or 'custom', config_json=row.get('config_json') or '{}')
    labels = set()
    for row in wallets.values():
        if not row.get('label') or row['label'] in labels:
            skipped['adapter.wallet_label'] += 1
            row = dict(row, label='campaign:' + row['id'])
        labels.add(row['label'])
        insert('wallets', row, **scope, created_at=CLOCK)
    fingerprints = set()
    for row in transactions.values():
        if row.get('fingerprint') in fingerprints:
            skipped['adapter.transaction_fingerprint'] += 1
            row = dict(row, fingerprint='campaign:' + row['id'])
        fingerprints.add(row.get('fingerprint'))
        insert('transactions', row)
    headers = {}
    for component in snapshot['components']:
        header = component['header']
        headers[header['id']] = header
    for header in snapshot.get('revision_headers', []):
        headers.setdefault(header['id'], header)
    for header in headers.values():
        insert('custody_components', header)
    for component in snapshot['components']:
        for key, table in (('legs', 'custody_component_legs'), ('allocations', 'custody_component_allocations'),
                           ('economic_terms', 'custody_component_economic_terms'), ('commitments', 'custody_component_evidence_commitments')):
            for ordinal, row in enumerate(component[key]):
                defaults = dict(scope, id=f'campaign:{component["header"]["id"]}:{key}:{ordinal}',
                                component_id=component['header']['id'], ordinal=ordinal, created_at=CLOCK)
                if key == 'economic_terms':
                    defaults.update(legacy_source_id=row.get('id', defaults['id']))
                insert(table, row, **defaults)
    for row in snapshot.get('evidence_snapshots', []):
        insert('custody_authored_evidence_snapshots', row, **scope, created_at=CLOCK, subject_kind='custody_component')
    for row in snapshot.get('migration_issues', []):
        insert('custody_authored_migration_issues', row, **scope, created_at=CLOCK, updated_at=CLOCK,
               legacy_table='transaction_pairs', legacy_source_id=row['id'])
    for row in snapshot.get('pair_dismissals', []):
        insert('transaction_pair_dismissals', row, **scope, created_at=CLOCK)
    for _, sql in triggers:
        conn.execute(sql)
    for currency in sorted({profile['fiat_currency'], *(row.get('fiat_currency') or profile['fiat_currency'] for row in transactions.values())}):
        rate(conn, pair='BTC-' + currency, timestamp='0001-01-01T00:00:00Z')
    conn.commit()
    conn.execute('PRAGMA ignore_check_constraints=OFF')
    return conn, skipped


def activity(case):
    result = case['result'] or {}
    repair = result.get('repair') or {}
    consolidation = result.get('consolidation') or {}
    return Counter(books=1, transactions=len(case['before']['transactions']['rows']),
                   components=len(case['before']['custody_components']['rows']),
                   auto_priced=result.get('auto_priced', 0),
                   duplicates_excluded=repair.get('duplicates_excluded', 0),
                   addresses_removed=repair.get('addresses_removed', 0),
                   legacy_activated=consolidation.get('activated', 0),
                   legacy_skipped=consolidation.get('skipped', 0),
                   warnings=int(result.get('warning') is not None), refusals=int(case['error'] is not None),
                   changed_tables=sum(case['before'][t] != data for t, data in case['expected'].items()))


def digest(case):
    return hashlib.sha256(json.dumps(case, sort_keys=True, ensure_ascii=True, allow_nan=False,
                                     separators=(',', ':')).encode()).hexdigest()


def generated(seeds=2000, permutations=True):
    from tests.custody_gap_stage import suites
    from tests.custody_engine_compare import permuted_snapshot
    for suite, scenarios, generate in suites():
        for seed_id in range(seeds):
            scenario = scenarios[seed_id % len(scenarios)]
            snapshot = generate(seed_id)
            yield suite, scenario, seed_id, False, snapshot
            if permutations and seed_id < len(scenarios) * 2:
                yield suite, scenario, seed_id, True, permuted_snapshot(snapshot)


def recorded_snapshots(directory):
    from tests.custody_engine_compare import permuted_snapshot
    from tests.custody_snapshot import load, SCHEMA, VERSION
    for path in sorted(directory.rglob('*.json')):
        document = load(path)
        if not isinstance(document, dict) or document.get('schema') != SCHEMA:
            continue
        snapshot = document.get('input') or {}
        if snapshot.get('schema_version') != VERSION:
            continue
        name = path.relative_to(directory).as_posix()
        yield 'historical-snapshots', name, 0, False, snapshot
        yield 'historical-snapshots', name, 0, True, permuted_snapshot(snapshot)


def historical_books():
    import base64
    import gzip
    import tempfile
    from pathlib import Path
    from kassiber.db import open_db
    scratch = ROOT / 'target/r4-10/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    for fixture in sorted((ROOT / 'tests/fixtures/historical').glob('*.sqlite3.gz.b64')):
        with tempfile.TemporaryDirectory(dir=scratch, prefix='historical-prebuild-') as directory:
            data = Path(directory)
            (data / 'kassiber.sqlite3').write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
            conn = open_db(data)
            try:
                for profile in conn.execute('SELECT id FROM profiles ORDER BY id').fetchall():
                    case = capture(conn, fixture.name + '/' + profile[0], profile[0])
                    case['schema'] = [list(row) for row in conn.execute(
                        "SELECT type,name,tbl_name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")]
                    yield 'historical-books', case['name'], 0, False, case
            finally:
                conn.close()


def frozen(rejections):
    from tests.custody_gap_stage import suites
    cases = []
    for suite, scenarios, generate in suites():
        for seed_id, scenario in enumerate(scenarios):
            snapshot = generate(seed_id)
            try:
                conn, skipped = materialize(snapshot)
            except sqlite3.IntegrityError as error:
                rejections.append(dict(name=f'{suite}/{scenario}', error=str(error)))
                continue
            try:
                case = capture(conn, f'{suite}/{scenario}', snapshot['profile']['id'])
                case['adapter_omitted_columns'] = dict(sorted(skipped.items()))
                cases.append(case)
            finally:
                conn.close()
    return cases


def packed(cases):
    base = cases[0]['before']
    return dict(base=base, cases=[dict(case, **{
        side: {table: item['rows'] for table, item in case[side].items() if item != base[table]}
        for side in ('before', 'expected')}) for case in cases])


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    rejections = []
    cases = frozen(rejections)
    text = json.dumps(dict(packed(cases), unrepresentable=rejections), ensure_ascii=True, separators=(',', ':'), allow_nan=False) + '\n'
    if args.write:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
    print(f'{len(cases)} whole-stage scenario vectors match Python; {len(rejections)} schema refusals')
    print(json.dumps(rejections, indent=2))
