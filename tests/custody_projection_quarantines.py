"""Independent full Python compiler oracle for private projection operands."""
from copy import deepcopy
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace as NS

from kassiber.core import custody_tax_projection as p
from kassiber.core.custody_quantity_runtime import TaxExposurePool
from unittest.mock import patch

from tests.custody_projection_composition import environment, fixed_eligibility, seed, scenario
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / 'fixtures/custody_projection_quarantines.json'



def source_rows(v):
    """The operand's rows in its source order: `row_order` when given, else
    by id, as the Rust compiler reads a JSON object (its keys sort)."""
    return [v['rows'][key] for key in (v.get('row_order') or sorted(v['rows']))]

def evaluate(v):
    try:
        _, _, state = environment(v)
        state.issues = [NS(**i) for i in v['issues']]
        state.tax_eligibility = replace(state.tax_eligibility, pool_barrier_roots=tuple(
            (TaxExposurePool(**pool), tuple(roots)) for pool, roots in v['pool_barrier_roots']))
        state.reviewed_conversion_pairs = v['reviewed']
        with patch.object(p, 'with_tax_projection_blockers', fixed_eligibility):
            result = p.compile_finalized_tax_projection(
                v['profile'], source_rows(v), state,
                non_event_transaction_ids=v['non_event_transaction_ids'],
                blocked_transaction_ids=v['blocked_transaction_ids'],
                interpreter_quarantines=v['interpreter_quarantines'],
                direct_payout_records=v['payouts'])
        return {'expected': to_jsonable(result, redact_config=False)}
    except Exception as exc:
        return {'error': type(exc).__name__}


def issue(v, reason='custody_gap_review_required', ident='issue', tx=('source',), kind='quantity', **details):
    v['issues'].append(dict(reason=reason, issue_id=ident, transaction_ids=list(tx), issue_type=kind, details=details))


def barrier(v):
    o = v['observations'][0]
    pool = dict(profile_id=o['profile_id'], network='main', exposure='bitcoin', unit='msat')
    e = v['events'][0]['event_key']
    order = [o['occurred_at'], *[e[k] for k in ('chain','network','native_namespace','native_event_id')]]
    v['pool_barriers'] = [[pool, order]]
    v['pool_barrier_roots'] = [[pool, ['root-z', 'root-a']]]


def operands():
    def case(name, edit=lambda v: None):
        v = seed('finalize')
        v.update(case=name, issues=[], pool_barrier_roots=[], interpreter_quarantines=[], reviewed=[])
        edit(v)
        return v
    yield case('ordinary_rows')
    def roots(v):
        d = deepcopy(v['decisions'][0])
        d['source']['observation_hash'] = v['observations'][1]['quantity_hash']
        d['target']['observation_hash'] = v['observations'][2]['quantity_hash']
        v['decisions'].append(d)
        v['blocked_transaction_ids'] = ['source','target']
    yield case('origins_fixed_point', roots)
    yield case('origins_reverse_roots', lambda v: (roots(v), v['blocked_transaction_ids'].reverse()))
    yield case('origins_separate_root', lambda v: (roots(v), v['blocked_transaction_ids'].append('unconnected')))
    yield case('dependency_absent_row', lambda v: (roots(v), v['rows'].pop('other')))
    yield case('explicit_root_not_dependency', lambda v: v.update(blocked_transaction_ids=['source']))
    yield case('barrier_held_receipt', barrier)
    yield case('barrier_ineligible_source', lambda v: (barrier(v), v.update(eligible=[])))
    yield case('barrier_without_event', lambda v: (barrier(v), v.update(events=[],eligible=[])))
    yield case('ineligible_without_barrier', lambda v: v.update(eligible=[]))
    yield case('basis_anchor_alias', lambda v: (v.update(eligible=[]), v['observations'][0].update(transaction_id='virtual')))
    yield case('basis_no_anchor', lambda v: (v.update(eligible=[]), v['rows'].pop('source')))
    yield case('barrier_roots_pool_isolation', lambda v: (barrier(v), v['pool_barrier_roots'][0][0].update(profile_id='unrelated')))
    yield case('barrier_fee_reason', lambda v: (barrier(v), v.update(eligible=[]), issue(v,'transfer_fee_implausible')))
    yield case('interpreter_issue_skipped', lambda v: issue(v,'transfer_fee_implausible',kind='custody_interpreter_blocked'))
    yield case('issue_single', lambda v: issue(v))
    yield case('issue_missing_transaction', lambda v: issue(v,tx=('missing',)))
    yield case('issue_component', lambda v: issue(v,'custody_component_incomplete',tx=('other',)))
    yield case('issue_compile_failed', lambda v: issue(v,'compile_failure',kind='component_claim_compile_failed',tx=('other',)))
    yield case('issue_bundle', lambda v: issue(v,'bad_bundle',kind='quantity_claim_bundle_invalid',tx=('other',)))
    yield case('issue_fee', lambda v: issue(v,'transfer_fee_implausible'))
    def many(v):
        for i,reason in enumerate(['other','custody_component_z','capacity_source_suspense_required','search_capacity_incomplete','source_overlap_quantity_unresolved','unclaimed_source_residual','implicit_wallet_delta_unallocated','custody_gap_review_required']):
            issue(v,reason,str(i),gap_id='gap-z' if i%2 else 'gap-a')
        issue(v,'custody_gap_review_required','aaa',gap_id='gap-z')
    yield case('issue_precedence_and_sets', many)
    # Every adjacent rank is independently observable, including component/default.
    reasons=['custody_gap_review_required','implicit_wallet_delta_unallocated','unclaimed_source_residual','source_overlap_quantity_unresolved','search_capacity_incomplete','capacity_source_suspense_required','custody_component_z','other']
    for i in range(len(reasons)-1):
        yield case('rank_'+str(i),lambda v,i=i:(issue(v,reasons[i+1],'a'),issue(v,reasons[i],'z')))
    yield case('lexical_reason_tie',lambda v:(issue(v,'z-other','a'),issue(v,'a-other','z')))
    yield case('duplicate_issue_membership',lambda v:issue(v,tx=('source','source'),gap_id='gap'))
    yield case('gap_falsey',lambda v:issue(v,gap_id=0))
    yield case('gap_python_string',lambda v:issue(v,gap_id=True))
    yield case('detail_not_mapping',lambda v:(issue(v),v['issues'][0].update(details=['gap_id'])))
    def interpreter(v):
        v['interpreter_quarantines']=[dict(transaction_id='source',reason='zzz',detail_json='literal',extra='kept'),dict(transaction_id='source',reason='aaa',detail_json='first'),dict(transaction_id='source',reason='aaa',detail_json='last'),dict(transaction_id='',reason='ignored'),dict(transaction_id='other')]
    yield case('interpreter_order_and_copy',lambda v:(interpreter(v),v['interpreter_quarantines'].pop()))
    yield case('interpreter_missing_reason_refusal',interpreter)
    yield case('interpreter_before_generic',lambda v:(issue(v),v['interpreter_quarantines'].append(dict(transaction_id='source',reason='zzz',detail_json='specific'))))
    yield case('interpreter_over_issue',lambda v:(issue(v),v['interpreter_quarantines'].append(dict(transaction_id='source',reason='custody_quantity_unresolved',detail_json='specific'))))
    yield case('dependency_over_interpreter',lambda v:(roots(v),v['interpreter_quarantines'].append(dict(transaction_id='other',reason='transfer_pair_dependency_blocked',detail_json='replaced'))))
    yield case('barrier_receipt_existing_issue',lambda v:(barrier(v),issue(v,tx=('other',))))
    yield case('barrier_receipt_existing_interpreter',lambda v:(barrier(v),v['interpreter_quarantines'].append(dict(transaction_id='other',reason='specific'))))
    def payout(v):
        v['decisions'][0].update(target=None,state='external_confirmed',reason='reviewed_direct_payout_source')
        v['payouts']=[dict(id='paid',out_transaction_id='source',payout_amount=50,payout_asset='LBTC',payout_fiat_value=None,policy='carrying-value')]
    yield case('payout_price_hold',payout)
    yield case('payout_specific_hold_preserved',lambda v:(payout(v),v['interpreter_quarantines'].append(dict(transaction_id='source',reason='at_swap_price_required',detail_json='specific'))))
    yield case('setdefault_eager_profile_refusal',lambda v:(v.update(eligible=[]),v['interpreter_quarantines'].append(dict(transaction_id='source',reason='custody_basis_barrier')),v['profile'].pop('workspace_id')))
    yield case('missing_profile_key_refusal',lambda v:(issue(v),v['profile'].pop('workspace_id')))
    yield case('payout_refusal_precedes_profile',lambda v:(payout(v),v['payouts'][0].update(payout_fiat_value='bad'),v['profile'].pop('workspace_id')))
    yield case('missing_source_refusal',lambda v:v['decisions'][0]['source'].update(observation_hash='missing'))
    yield case('missing_target_refusal',lambda v:v['decisions'][0]['target'].update(observation_hash='missing'))
    yield case('duplicate_projection_refusal',lambda v:v['decisions'].append(deepcopy(v['decisions'][0])))
    yield case('issue_before_constructor_refusal',lambda v:(v['decisions'].append(deepcopy(v['decisions'][0])),issue(v),v['profile'].pop('id')))
    yield case('conversion_after_selection',lambda v:v['reviewed'].append(dict(pair_id='taxable',out_id='source',in_id='other')))
    yield case('conversion_amount_refusal',lambda v:v['reviewed'].append(dict(pair_id='taxable',out_id='source',in_id='other',out_amount='bad')))
    # Rounds 5 and 8: valuation holds and price causes retained behind barriers.
    for field, value in [('fiat_rate', '1e-14'), ('fiat_rate_exact', 'Infinity'), ('fiat_value', '1e19'),
                         ('fiat_value_exact', 'bad'), ('fiat_rate', 1e300), ('fiat_rate_exact', '9999999999999999999.5')]:
        yield case(f'valuation_invalid_{field}_{value}', lambda v, f=field, x=value: v['rows']['other'].update({f: x}))
    yield case('valuation_derived_value_bound', lambda v: v['rows']['other'].update(amount=10**30, fiat_rate='1e18'))
    yield case('valuation_derived_rate_floor', lambda v: v['rows']['other'].update(amount=10**30, fiat_value='1e-13'))
    yield case('valuation_exact_falls_back_to_legacy', lambda v: v['rows']['other'].update(fiat_rate_exact='', fiat_rate='1e-14'))
    yield case('valuation_cached_rate_range', lambda v: v['rows']['other'].update(pricing_method='unsupported_price_range'))
    yield case('valuation_held_source_dependency', lambda v: v['rows']['source'].update(fiat_rate='1e-14'))
    yield case('valuation_bad_amount_refusal', lambda v: v['rows']['other'].update(amount='bad'))
    yield case('valuation_unobserved_bad_amount_refusal', lambda v: v['rows'].update(stray=dict(v['rows']['other'], id='stray', amount='bad')))
    yield case('valuation_falsy_exact_uses_legacy', lambda v: v['rows']['other'].update(fiat_rate_exact=0, fiat_rate='1e-13'))
    yield case('valuation_after_interpreter', lambda v: (v['rows']['other'].update(fiat_rate='1e-14'),
        v['interpreter_quarantines'].append(dict(transaction_id='other', reason='invalid_transaction_price', detail_json='earlier'))))
    yield case('valuation_split_payout', lambda v: (payout(v), v['payouts'][0].update(out_amount=40)))
    yield case('valuation_split_other_component', lambda v: (payout(v), v['payouts'][0].update(out_amount=40, component_id='other')))
    def priced(v, rate='100'):
        barrier(v)
        for row in v['rows'].values():
            row.update(fiat_rate=rate)
    yield case('price_cause_priced_rows', priced)
    yield case('price_cause_value_only', lambda v: (barrier(v), v['rows']['other'].update(fiat_value='5')))
    yield case('price_cause_zero_rate_value', lambda v: (barrier(v), v['rows']['other'].update(fiat_rate='0', fiat_value='5')))
    yield case('price_cause_wallet_label', lambda v: (barrier(v), v['rows']['other'].update(wallet_label='Cold')))
    yield case('price_cause_coarse_review', lambda v: (priced(v), v['rows']['other'].update(pricing_quality='coarse_fallback'),
        v['profile'].update(require_coarse_review=' Yes ')))
    # Main #708: a coarse price the owner accepted, unchanged since, needs no review.
    yield case('price_cause_coarse_accepted', lambda v: (priced(v), v['rows']['other'].update(
        pricing_quality='coarse_fallback', pricing_review_accepted_price='rate:100.00'), v['profile'].update(require_coarse_review=1)))
    yield case('price_cause_coarse_accepted_changed', lambda v: (priced(v), v['rows']['other'].update(
        pricing_quality='coarse_fallback', pricing_review_accepted_price='rate:99'), v['profile'].update(require_coarse_review=1)))
    yield case('price_cause_coarse_not_required', lambda v: (priced(v), v['rows']['other'].update(pricing_quality='coarse_fallback'),
        v['profile'].update(require_coarse_review='0')))
    # An internal move held at the barrier needs no standalone price: barrier_ineligible_source.
    yield case('price_cause_external_source', lambda v: (barrier(v), v.update(eligible=[]),
        v['decisions'][0].update(target=None, state='external_confirmed')))
    yield case('price_cause_external_payout_source', lambda v: (barrier(v), payout(v), v.update(eligible=[])))
    yield case('price_cause_zero_outbound', lambda v: (barrier(v), v.update(eligible=[]),
        v['decisions'][0].update(target=None, state='external_presumed'), v['rows']['source'].update(amount=0, fee=0)))
