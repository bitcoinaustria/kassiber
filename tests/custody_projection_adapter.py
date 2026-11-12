"""Independent Python compiler fixtures for the Rust raw-stage adapter."""
from copy import deepcopy
from tests.custody_projection_quarantines import operands, evaluate
from tests.custody_snapshot_generator import templates
from tests.custody_projection_composition import seed


def vectors():
    cases = []
    non_event = seed('finalize')
    non_event.update(case='interpreter_non_event_population', issues=[], pool_barrier_roots=[], interpreter_quarantines=[], reviewed=[])
    non_event['decisions'][0].update(target=None, state='external_presumed')
    non_event['non_event_transaction_ids'] = ['source', 'other']
    interpreter_component = seed('finalize')
    interpreter_component.update(case='interpreter_component_anchor',
        issues=[dict(reason='custody_component_hold', issue_type='custody_interpreter_blocked',
                     transaction_ids=['other'], issue_id='interpreter-component', details={})],
        pool_barrier_roots=[], interpreter_quarantines=[], reviewed=[])
    for original in [*operands(), non_event, interpreter_component]:
        if original['case'].startswith('valuation_'):
            # A valuation hold recomputes eligibility from the whole quantity
            # state, which these synthetic stages lack; the valuation stage
            # fixtures compare it from real books.
            continue
        v = deepcopy(original)
        # The stage reads the book's transactions in this order.
        v['row_order'] = list(v['rows'])
        for row in v['rows'].values():
            row['_closed_confirmation_observation'] = None
        book = deepcopy(templates()[0])
        book.update(transactions=list(v['rows'].values()), profile=v['profile'], channel_records=[])
        quantity = dict(
            projection=dict(observations=v['observations'], decisions=v['decisions']),
            canonical_input=dict(events=v['events']), issues=v['issues'],
            tax_eligibility=dict(eligible_decisions=[dict(source=s) for s in v['eligible']],
                                 pool_barriers=v['pool_barriers'], pool_barrier_roots=v['pool_barrier_roots']),
            reviewed_direct_payouts=v['payouts'], reviewed_conversion_pairs=v['reviewed'])
        interpretation = dict(non_event_transaction_ids=v['non_event_transaction_ids'],
                              blocked_transaction_ids=v['blocked_transaction_ids'],
                              quarantines=v['interpreter_quarantines'])
        result = evaluate(v)
        expected = result.get('expected', {'error': result.get('error')})
        if expected.get('error') == 'AppError':
            expected['error'] = 'validation'
        cases.append(dict(case=v['case'], input=book, quantity=quantity,
                          interpretation=interpretation, expected=expected))
    # Mapping construction must preserve Python's last-write row population.
    duplicate = deepcopy(cases[0])
    duplicate['case'] = 'duplicate_row_last_write'
    stale = dict(duplicate['input']['transactions'][0], adapter_stale_row=True)
    duplicate['input']['transactions'].insert(0, stale)
    cases.append(duplicate)
    # Isolate channel exclusions from an interpreter which supplied no duplicate
    # exclusion. Internal MOVE decisions deliberately precede these exclusions.
    from dataclasses import replace
    from kassiber.core.custody_compute import compute_custody_projection
    from kassiber.core.custody_tax_projection import compile_finalized_tax_projection
    from tests.custody_channel_generator import channel_snapshot
    from tests.custody_snapshot import to_jsonable
    book = channel_snapshot(0)
    result = compute_custody_projection(book)
    state = result.quantity_state
    state = replace(state, projection=replace(state.projection, decisions=()),
                    tax_eligibility=replace(state.tax_eligibility, eligible_decisions=()))
    final = compile_finalized_tax_projection(book['profile'], result.rows, state,
        non_event_transaction_ids=result.channel_non_event_ids)
    cases.append(dict(case='channel_non_event_population', input=book,
        quantity=to_jsonable(state, redact_config=False),
        interpretation=dict(non_event_transaction_ids=[], blocked_transaction_ids=[], quarantines=[]),
        expected=to_jsonable(final, redact_config=False)))
    return cases
