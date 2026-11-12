#!/usr/bin/env python3
"""Fixed-source component plan/apply oracle: exact outputs and complete database rows."""
from contextlib import ExitStack
import datetime
import hashlib
import json
import tempfile
import uuid
from unittest.mock import patch
from export_pairs import ROOT, STAMP, TABLES as PAIR_TABLES, seed, dump, terms, migration, components, handlers, open_db, AppError
from kassiber.core import custody_component_planner as planner, wallets

TABLES = PAIR_TABLES + ('wallets',)
ACCOUNT = "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury-account','w','p','treasury','Treasury','asset','2025-01-01T00:00:00Z')"
LIGHTNING = "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('ln','w','p','Node','lnd','2025-01-01T00:00:00Z')"
PLACEHOLDER = "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('cold','w','p','Cold Storage','untracked','2025-01-01T00:00:00Z')"


def leg(role, tx=None, amount=1000, **kw):
    out = dict(role=role, **kw)
    if amount is not None:
        out['amount_msat'] = amount
    if tx is not None:
        out['transaction'] = tx
    return out


def bridge(out='d', inbound='e', amount=1000, **kw):
    return dict(legs=[leg('source', out, amount), leg('destination', inbound, amount)], **kw)


def plan(action='create', **kw):
    return ('transfers.components.plan', dict(action=action, **kw))


def apply(action='create', version='@v', **kw):
    return ('transfers.components.apply', dict(action=action, expected_input_version=version, **kw))


def both(action='create', **kw):
    setup = kw.pop('__sql', [])
    return [plan(action, __sql=setup, **kw), apply(action, **kw)]


SPLIT = dict(
    legs=[leg('source', 'a', 1010), leg('destination', 'b', 600), leg('destination', 'c', 400), leg('fee', 'a', 10)],
    allocations=[
        dict(source_ordinal=0, sink_ordinal=1, source_amount_msat=600, sink_amount_msat=600),
        dict(source_ordinal=0, sink_ordinal=2, source_amount_msat=400, sink_amount_msat=400),
        dict(source_ordinal=0, sink_ordinal=3, source_amount_msat=10, sink_amount_msat=10),
    ],
)
COLD = dict(legs=[leg('source', 'd', 1000), leg('destination', None, 1000, untracked_wallet=' Cold Storage ', occurred_at='2025-02-01T00:00:00Z')])


def scenarios():
    result = {
        'lifecycle': [
            *both(components=[bridge(notes=' € 🚀 ')]),
            plan(components=[bridge(notes=' € 🚀 ')]),
            apply(components=[bridge(notes=' € 🚀 ')]),
            *both('supersede', component_id='@1', reason='mistake'),
            *both('supersede', component_id='@1'),
            *both('undo', component_id='@1'),
            plan('undo', component_id='@1', reason=''),
            *both('activate', component_id='@9'),
            *both('activate', component_id='@9'),
        ],
        'draft': [
            *both(components=[bridge()], activate=False),
            *both('activate', component_id='@1'),
            *both('supersede', component_id='@1', reason='  '),
        ],
        'revise': [
            *both(components=[bridge(notes='first')]),
            *both('revise', component_id='@1', spec={'notes': 'second', 'change_reason': 'typo'}),
            plan('revise', component_id='@1', spec={'notes': 'third'}),
            *both('activate', component_id='@3'),
            *both('revise', component_id='@3', spec={'legs': [leg('source', 'd', 1000), leg('destination', 'e', 1000, notes='moved')]}, activate=True, reason='relabel'),
            *both('revise', component_id='@7', spec={'evidence': {'b': 1, 'a': [1.5, None]}, 'conversion_metadata': '{"z":1}', 'evidence_kind': 'statement', 'evidence_grade': 'reviewed'}, activate=True),
            *both('revise', component_id='@9', spec={'allocations': [dict(source_leg_id='@leg0', sink_leg_id='@leg1', source_amount_msat=1000, sink_amount_msat=1000)]}, activate=True),
            *both('revise', component_id='@11', spec={'legs': [leg('source', 'd', 900), leg('destination', 'e', 900)]}, activate=True),
            *both('revise', component_id='@11', spec={'legs': [leg('source', 'd', 900), leg('destination', 'e', 900)]}),
        ],
        'batch': [
            *both(components=[bridge(), SPLIT]),
            *both(components={'components': [bridge('j', 'k')]}),
            *both(components=[bridge('f', 'e', 1010), bridge('f', 'k', 1010)]),
            *both(components=[bridge('d', 'e', notes='again'), bridge('d', 'k')], __sql=["DELETE FROM custody_component_transaction_memberships", "UPDATE custody_components SET state='superseded'"]),
            *both(components=[bridge('d', 'e'), bridge('d', 'k')], activate=False),
        ],
        'conflict': [
            *both(components=[bridge()]),
            *both(components=[bridge('j', 'e')]),
            *both(components=[bridge('j', 'e')], activate=False),
            *both('activate', component_id='@5'),
            *both('supersede', component_id='@1'),
            *both('activate', component_id='@5'),
        ],
        'untracked': [
            *both(components=[COLD], __sql=[ACCOUNT]),
            *both(components=[dict(legs=[leg('source', 'j', 1000), leg('destination', None, 1000, untracked_wallet='cold storage', occurred_at='2025-02-01T00:00:00Z')])]),
            *both(components=[dict(legs=[leg('source', 'a', 1010), leg('destination', None, 1000, untracked_wallet='Straße', occurred_at='2025-02-01T00:00:00Z'), leg('fee', 'a', 10)], allocations=[dict(source_ordinal=0, sink_ordinal=1, source_amount_msat=1000, sink_amount_msat=1000), dict(source_ordinal=0, sink_ordinal=2, source_amount_msat=10, sink_amount_msat=10)])], activate=False),
            *both(components=[dict(legs=[leg('source', 'f', 1010), leg('destination', None, 1010, untracked_wallet='STRASSE', occurred_at='2025-02-01T00:00:00Z')])], activate=False),
            plan(components=[dict(legs=[leg('source', 'd', 1000), leg('destination', None, 1000, untracked_wallet='source')])]),
            plan(components=[dict(legs=[leg('source', 'd', 1000, untracked_wallet='x')])]),
            plan(components=[dict(legs=[leg('source', None, 1000, untracked_wallet='x', wallet='source')])]),
            plan(components=[dict(legs=[leg('source', None, 1000, untracked_wallet='  ')])]),
            plan(components=[dict(legs=[leg('source', None, 1000, untracked_wallet=5)])]),
        ],
        'route-batch': [
            *both(components=[
                dict(legs=[leg('source', 'd', 1000), leg('destination', None, 1000, untracked_wallet='Vault', occurred_at='2025-02-01T00:00:00Z')]),
                dict(legs=[leg('source', None, 1000, untracked_wallet='vault', occurred_at='2025-01-01T00:00:00Z'), leg('destination', 'k', 1000)]),
            ], __sql=[ACCOUNT]),
            *both(components=[dict(legs=[leg('source', 'd', 1000), leg('destination', None, 1000, untracked_wallet='Vault', occurred_at='2025-02-01T00:00:00Z')])]),
            *both(components=[dict(legs=[leg('source', None, 1000, untracked_wallet='vault', occurred_at='2025-01-01T00:00:00Z'), leg('destination', 'k', 1000)])]),
            *both(components=[dict(legs=[leg('source', None, 1000, untracked_wallet='vault', occurred_at='2025-03-01T00:00:00Z'), leg('destination', 'k', 1000)])]),
        ],
        'competing-active': [
            *both(components=[bridge()]),
            *both(components=[bridge(notes='rival')], __sql=["UPDATE custody_components SET state='draft' WHERE state='active'"]),
            plan('supersede', component_id='@3', __sql=["UPDATE custody_components SET state='active' WHERE state='draft'"]),
            apply('supersede', component_id='@3'),
            plan('activate', component_id='@1'),
        ],
        'native': [
            *both(components=[bridge('n-out', 'n-in')], __sql=[
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at) VALUES('n-out','w','p','source','fp-n-out','n-out-ext','BTC','outbound',1000,0,0,'2025-02-01T00:00:00Z','withdrawal','{\"txid\":\"%s\"}','2025-01-01T00:00:00Z')" % ('ab' * 32),
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at) VALUES('n-in','w','p','target','fp-n-in','n-in-ext','BTC','inbound',1000,0,0,'2025-02-01T00:00:00Z','deposit','{\"txid\":\"%s\"}','2025-01-01T00:00:00Z')" % ('ab' * 32),
            ]),
            plan('supersede', component_id='@1'),
        ],
        'no-account': [*both(components=[COLD])],
        'existing-placeholder': [*both(components=[COLD], __sql=[PLACEHOLDER])],
        'wallets': [
            *both(components=[dict(legs=[leg('source', None, 1000, wallet='SOURCE', occurred_at='2025-02-01T00:00:00Z'), leg('destination', None, 1000, wallet_ref='ln', occurred_at='2025-02-02T00:00:00Z')])], activate=False, __sql=[LIGHTNING]),
            plan(components=[dict(legs=[leg('source', None, 1000, wallet_id='source'), leg('destination', None, 1000, wallet_id='missing')])], activate=False),
            plan(components=[dict(legs=[leg('source', None, 1000, wallet_id=False)])], activate=False),
            plan(components=[dict(legs=[leg('source', None, 1000, wallet='absent')])], activate=False),
            plan(components=[dict(legs=[leg('source', None, 1000, asset='lbtc', wallet='liquid', occurred_at='2025-02-01T00:00:00Z'), leg('destination', None, 1000, asset='L-BTC', rail='', exposure=None, conservation_unit='')])], activate=False),
        ],
        # A non-string action is refused without details; an unknown text action names itself.
        'actions': [
            *[plan(v, components=[COLD]) for v in ([], {}, None, 7, True, 'explode', 'Create')],
            *[apply(v, components=[COLD]) for v in ([], {'a': 1}, None, 'explode')],
        ],
        'amounts': [
            plan(components=[dict(legs=[leg('source', 'd', None, amount_btc='0.00000001'), dict(role='destination', transaction='e', amount_btc=1e-08)])]),
            *[plan(components=[dict(legs=[dict(role='source', transaction='d', amount_btc=v)])], activate=False) for v in ('NaN', 'sNaN', 'Infinity', '-Infinity', True, None, 'bad', '1e30', '-0.00000001', ' 1_0e-11 ', '0.000000000005', '0.000000000015', [1])],
            plan(components=[dict(legs=[dict(role='source', transaction='d', amount_btc='0.00000001', amount_msat=1000)])]),
            *[plan(components=[dict(legs=[dict(role='source', transaction='d', amount_msat=v)])], activate=False) for v in ('0012', '9223372036854775807', '9223372036854775808', '00000000000000000000001', '-1', -1, 1.0, '1.0', ' 1', True)],
        ],
        'validation': [
            plan(components=None),
            plan(components=[]),
            plan(components={'x': 1}),
            plan(components=[bridge()] * 51),
            plan(components=[5]),
            plan('remove'),
            plan(['create']),
            plan(components=[bridge()], reason=5),
            plan(components=[bridge()], activate='yes'),
            plan(components=[bridge()], component_id='x', spec={}, reason='r'),
            plan('revise', component_id='x'),
            plan('revise', component_id='  ', spec={}),
            plan('revise', component_id='x', spec={}, components=[]),
            plan('undo', component_id='x', spec={}),
            plan('undo', component_id='x', activate=True),
            plan('activate', component_id='x', reason='no'),
            plan('supersede', component_id='x', activate=False),
            plan('supersede', component_id='x', spec={}),
            plan('supersede', component_id='absent'),
            plan(components=[dict(legs=5)]),
            plan(components=[dict(legs=[5])]),
            plan(components=[dict(legs=[leg('source', 'absent')])]),
            plan(components=[dict(legs=[leg('source', 'j-ext')])]),
            plan(components=[dict(legs=[leg('source', 'd-ext'), leg('destination', 'e', 1000)])]),
            plan(components=[dict(legs=[leg('source', 'd', 1000, transaction_ref='e')])]),
            plan(components=[dict(legs=[leg('bogus', 'd')])]),
            plan(components=[dict(legs=[leg('source', 'd', rail='Bad Rail')])]),
            plan(components=[dict(legs=[leg('source', 'd', valuation_unit='eur')])]),
            plan(components=[dict(legs=[leg('source', 'd', valuation_unit='eur', valuation_amount='5'), leg('destination', 'e')])], activate=False),
            plan(components=[dict(legs=[leg('source', 'd', anchor_transaction_id='e')])]),
            plan(components=[dict(legs=[leg('source', 'd', occurred_at='2025-03-01T00:00:00Z')])]),
            plan(components=[dict(legs=[leg('source', 'd', occurred_at='2025-02-01T01:00:00+01:00'), leg('destination', 'e')])]),
            plan(components=[dict(legs=[leg('source', 'd', occurred_at='bad')])]),
            plan(components=[dict(legs=[leg('source', 'd', id='same'), leg('destination', 'e', id='same')])]),
            plan(components=[dict(legs=[leg('source', 'd', id=5)])]),
            plan(components=[dict(legs=[leg('source', 'd', id=None), leg('destination', 'e', id=None)])]),
            plan(components=[dict(bridge(), allocations={})]),
            plan(components=[dict(bridge(), allocations=[5])]),
            plan(components=[dict(bridge(), allocations=[dict(source_ordinal=0)])]),
            plan(components=[dict(bridge(), allocations=[dict(source_ordinal=0, sink_ordinal=9)])]),
            plan(components=[dict(bridge(), allocations=[dict(source_ordinal=1, sink_ordinal=0, source_amount_msat=1, sink_amount_msat=1)])]),
            plan(components=[dict(bridge(), allocations=[dict(source_ordinal=0, sink_ordinal=1, source_amount_msat=1000, sink_amount_msat=1000), dict(source_ordinal=0, sink_ordinal=1, source_amount_msat=1, sink_amount_msat=1)])]),
            plan(components=[dict(bridge(), allocations=[dict(source_leg_id=5, sink_ordinal=1)])]),
            plan(components=[dict(bridge(), allocations=[dict(source_ordinal=True, sink_ordinal=1)])]),
            plan(components=[bridge(conversion_reviewed='yes')]),
            plan(components=[bridge(component_type='Swap')]),
            plan(components=[bridge(component_type='ledger')]),
            plan(components=[bridge(conservation_mode='mixed')]),
            plan(components=[bridge(evidence='{bad')]),
            plan(components=[bridge(evidence='{"a": NaN}')]),
            plan(components=[bridge(evidence='{"a": 1e400}')]),
            plan(components=[bridge(evidence=[1])]),
            plan(components=[bridge(evidence='')]),
            plan(components=[bridge(conversion_metadata={'x': float('nan')})]),
            plan(components=[bridge(notes=12)]),
            plan(components=[bridge(evidence_kind='Bad Kind')]),
            plan(components=[bridge(created_at='2025-03-01')]),
            plan(components=[bridge(created_at='bad')]),
            plan(components=[bridge(created_at='')]),
            plan(components=[bridge(created_at=5)]),
            plan(components=[bridge(lineage_id=' lin ')]),
            plan(components=[bridge(amount=900)]),
            plan(components=[bridge(amount=900)], activate=False),
            plan(components=[bridge('zero', 'e', 0)]),
            plan(components=[bridge('d', 'excluded')]),
            plan(components=[bridge('d', 'net')]),
            plan(components=[bridge('d', 'h', conservation_mode='conversion')], activate=False),
            plan(components=[dict(legs=[leg('source', None, 1000, wallet='source'), leg('destination', None, 1000, wallet='target')])], activate=False),
            apply(components=[bridge()], version=-1),
            apply(components=[bridge()], version=True),
            apply(components=[bridge()], version=99),
            apply(components=[bridge()], version='0'),
        ],
        'requested-id': [
            *both(components=[bridge(component_id=' fixed ')]),
            plan(components=[bridge(component_id='fixed')]),
            *both(components=[bridge('j', 'k', lineage_id='fixed')]),
            *both(components=[bridge('j', 'k', created_at='2025-03-01T01:00:00+01:00', change_reason='authored')], activate=False),
        ],
        'locations': [
            *both(components=[dict(legs=[leg('source', 'd', location_ref='node:1'), leg('destination', 'e', location_ref=' channel ')])]),
            *both('revise', component_id='@1', spec={'notes': 'kept'}, activate=True),
            *both('revise', component_id='@3', spec={'legs': [dict(id='@leg0', role='source', transaction='d', amount_msat=1000), dict(id='@leg1', role='destination', transaction='e', amount_msat=1000, location_ref=None)]}, activate=True),
        ],
        'conversion': [
            *both(components=[dict(component_type='swap', conservation_mode='conversion', conversion_policy='taxable', conversion_reviewed=True, legs=[leg('source', 'd', 1000), leg('destination', 'h', 700, valuation_unit='eur', valuation_amount='70')])]),
            *both(components=[dict(component_type='swap', conservation_mode='conversion', conversion_policy='taxable', conversion_reviewed=True, legs=[leg('source', 'd', 1000, valuation_unit='eur', valuation_amount=70), leg('destination', 'h', 700, valuation_unit='eur', valuation_amount='70')])]),
            *both('revise', component_id='@3', spec={'conversion_reviewed': False, 'conversion_policy': 'carrying-value'}),
        ],
        'liquid': [*both(components=[dict(legs=[leg('source', 'f', 1010), leg('destination', 'g', 800), leg('external', None, 210, wallet='source', occurred_at='2025-02-01T00:00:00Z')], allocations=[dict(source_ordinal=0, sink_ordinal=1, source_amount_msat=800, sink_amount_msat=800), dict(source_ordinal=0, sink_ordinal=2, source_amount_msat=210, sink_amount_msat=210)])], activate=False)],
        'scope': [
            *both(components=[bridge()]),
            plan('supersede', component_id='@1', profile_scope='other'),
            plan('revise', component_id='@1', spec={}, profile_scope='other'),
            plan(components=[bridge()], profile_scope='other'),
            apply(components=[bridge()], profile_scope='other', version=0),
            plan(components=[bridge('d', 'e', component_id='@1')]),
        ],
        'gui': [*both(components=[bridge()], authored_source='gui'), *both('supersede', component_id='@1', authored_source='gui')],
        'stale-revision': [
            *both(components=[bridge()]),
            *both('revise', component_id='@1', spec={'notes': 'x'}),
            plan('revise', component_id='@1', spec={'notes': 'y'}),
            plan('revise', component_id='@1', spec={'bogus': 1, 'alpha': 2}),
            plan('revise', component_id='@1', spec={'legs': 5}),
            plan('revise', component_id='@1', spec={'allocations': 5}),
            plan('undo', component_id='@1'),
            *both('supersede', component_id='@3'),
            *both('revise', component_id='@1', spec={'notes': 'x'}),
            plan('revise', component_id='@1', spec={'notes': 'x'}),
        ],
        'draft-revision': [
            *both(components=[bridge()], activate=False),
            *both('revise', component_id='@1', spec={'notes': 'replaces draft'}),
        ],
    }
    result['late-failure'] = []
    result['pair-revision'] = [
        plan('revise', component_id='@pair', spec={'notes': 'reviewed'}),
        apply('revise', component_id='@pair', spec={'notes': 'reviewed'}, activate=True),
        plan('revise', component_id='@1', spec={'legs': [leg('source', 'd', 1000), leg('destination', 'e', 1000)]}),
    ]
    return result


def resolve(value, ids):
    if isinstance(value, str) and value in ids:
        return ids[value]
    if isinstance(value, list):
        return [resolve(v, ids) for v in value]
    if isinstance(value, dict):
        return {k: resolve(v, ids) for k, v in value.items()}
    return value


def main():
    states = {}
    out = []
    for name, operations in scenarios().items():
        with tempfile.TemporaryDirectory(prefix='kassiber-component-oracle-') as root:
            c = open_db(root)
            sql = seed(c)
            ids = {}
            if name == 'pair-revision':
                with ExitStack() as stack:
                    for module, field in ((terms, 'now_iso'), (migration, 'now_iso'), (components, '_now_iso')):
                        stack.enter_context(patch.object(module, field, return_value=STAMP))
                    stack.enter_context(patch.object(terms.uuid, 'uuid4', return_value=uuid.UUID('00000000-0000-4000-8000-000000000001')))
                    writes = []
                    c.set_trace_callback(lambda q: writes.append(q) if q.lstrip().split()[0].upper() in ('INSERT', 'UPDATE', 'DELETE') else None)
                    pair = handlers.create_transaction_pair(c, 'w', 'p', 'd', 'e')
                    c.set_trace_callback(None)
                for q in writes:
                    if not sql or sql[-1] != q:
                        sql.append(q)
                sql.append("UPDATE profiles SET journal_input_version=" + str(c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]) + " WHERE id='p'")
                sql.append('UPDATE chain_index_clock SET revision=' + str(c.execute('SELECT revision FROM chain_index_clock WHERE id=1').fetchone()[0]) + ' WHERE id=1')
                sql.append('DELETE FROM chain_index_dirty')
                for row in dump(c, 'chain_index_dirty'):
                    sql.append('INSERT INTO chain_index_dirty (' + ','.join(row) + ') VALUES (' + ','.join('NULL' if v is None else str(v) if isinstance(v, int) else "'" + v.replace("'", "''") + "'" for v in row.values()) + ')')
                with tempfile.TemporaryDirectory(prefix='kassiber-component-replay-') as replay_root:
                    replay = open_db(replay_root)
                    for statement in sql:
                        replay.execute(statement)
                    replay.commit()
                    for table in TABLES:
                        assert dump(replay, table) == dump(c, table), (name, table)
                    replay.close()
                ids['@pair'] = pair['component_id']
            if name == 'late-failure':
                version = c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
                operations = [apply(components=[bridge(), bridge('j', 'k')], __sql=[f"CREATE TEMP TRIGGER refuse_component_activation BEFORE UPDATE OF journal_input_version ON profiles WHEN NEW.id='p' AND NEW.journal_input_version={version + 2} BEGIN SELECT RAISE(ABORT,'late component activation invalidation'); END"]), plan(components=[bridge(), bridge('j', 'k')])]
            tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
            cases = []
            for i, (operation, raw) in enumerate(operations):
                raw = dict(raw)
                profile = raw.pop('profile_scope', 'p')
                setup = raw.pop('__sql', [])
                for statement in setup:
                    c.execute(statement)
                c.commit()
                if raw.get('expected_input_version') == '@v':
                    raw['expected_input_version'] = c.execute('SELECT journal_input_version FROM profiles WHERE id=?', (profile,)).fetchone()[0]
                args = resolve(raw, ids)
                args.setdefault('authored_source', 'cli')
                untouched = {t: dump(c, t) for t in tables if t not in TABLES}
                before = {t: dump(c, t) for t in tables}
                id = str(uuid.uuid5(uuid.NAMESPACE_DNS, f'component-oracle-{name}-{i}'))
                now = (datetime.datetime(2025, 6, 1, tzinfo=datetime.timezone.utc) + datetime.timedelta(hours=i)).isoformat().replace('+00:00', 'Z')
                with ExitStack() as stack:
                    stack.enter_context(patch.object(terms.uuid, 'uuid4', return_value=uuid.UUID(id)))
                    for module, field in [(terms, 'now_iso'), (migration, 'now_iso'), (components, '_now_iso'), (wallets, 'now_iso')]:
                        stack.enter_context(patch.object(module, field, return_value=now))
                    try:
                        review = {k: v for k, v in args.items() if k != 'expected_input_version'}
                        if operation == 'transfers.components.plan':
                            result = planner.plan_component_review(c, workspace_id='w', profile_id=profile, **review)
                        else:
                            result = planner.apply_component_review(c, workspace_id='w', profile_id=profile, expected_input_version=args['expected_input_version'], **review)
                        expected = {'result': result}
                        component = result.get('component') or (result.get('components') or [None])[0]
                        if component is not None:
                            ids[f'@{i}'] = component['id']
                            if operation == 'transfers.components.apply':
                                for n, row in enumerate(component.get('legs') or ()):
                                    ids[f'@leg{n}'] = row['id']
                    except Exception as e:
                        c.rollback()
                        expected = {'error': dict(code=e.code if isinstance(e, AppError) else 'internal_error', message=str(e), hint=getattr(e, 'hint', None), details=getattr(e, 'details', None))}
                if name == 'late-failure' and i == 0:
                    assert expected['error']['message'] == 'late component activation invalidation', expected
                    assert {t: dump(c, t) for t in tables} == before
                if operation == 'transfers.components.plan':
                    assert {t: dump(c, t) for t in tables} == before, (name, i)
                rows = {}
                for table in TABLES:
                    data = dump(c, table)
                    key = hashlib.sha256(json.dumps(data, sort_keys=True, ensure_ascii=True).encode()).hexdigest()
                    states[key] = data
                    rows[table] = key
                for table, previous in untouched.items():
                    assert dump(c, table) == previous, (name, i, table)
                cases.append(dict(operation=operation, args=args, setup=setup, profile=profile, id=id, now=now, **expected, rows=rows))
            out.append(dict(name=name, sql=sql, cases=cases))
            c.close()
    (ROOT / 'crates/transfers/tests/components.json').write_text(json.dumps(dict(tables=list(TABLES), states=states, scenarios=out), indent=2, ensure_ascii=True, allow_nan=True) + '\n')
    print({s['name']: {'operations': len(s['cases']), 'success': sum('result' in c for c in s['cases'])} for s in out})
    print('total', sum(len(s['cases']) for s in out), 'success', sum('result' in c for s in out for c in s['cases']))


if __name__ == '__main__':
    main()
