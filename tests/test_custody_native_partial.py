"""Native transition counts are independently pinned for Python and Rust."""
from collections import Counter
import json

from tests.custody_native_generator import NATIVE_SCENARIOS,native_snapshot
from tests.custody_engine_compare import assert_same,permuted_snapshot
from tests.custody_interpretation_partial import python_results,rust_results

# matcher candidates, native pairs, proofs, native quarantines;
# composed claims, audits, non-events, blocked, transient.
COUNTS = {}
for case in ('lnd_settle','cln_settle','cln_alias_settle','reverse_settle','timeout_refund','raw_refund','raw_claim','core_attestation','utc_same_day','dismiss_expired','dismiss_equal','privacy_native','provider_same_edge','provider_different_network','policy_taxable','policy_austria','raw_reverse_prefix'):
    COUNTS[case]=(1,1,1,0,2,0,0,0,0)
for case in ('nwc_untrusted','lnd_forged','network_mismatch','network_unknown','amount_increase','amount_excess_fee','target_stale','observer_mismatch','failed_payment','refund_duplicate_funding','refund_partial_output','raw_wrong_hash'):
    COUNTS[case]=(0,0,0,0,0,0,0,0,0)
for case in ('fee_next_day','fee_unknown_time','utc_different_day','canonical_mismatch'):
    COUNTS[case]=(1,0,0,2,0,0,0,2,0)
for case in ('dismiss_active','dismiss_invalid','dismiss_future','source_untrusted','timeout_missing_proof','refund_wrong_outpoint','refund_batched','provider_only'):
    COUNTS[case]=(1,0,0,0,0,0,0,0,0)
COUNTS.update(zero_fee_next_day=(1,1,1,0,1,0,0,0,0),duplicate_ln=(0,0,0,3,0,0,0,3,0),duplicate_chain=(0,0,0,3,0,0,0,3,0),duplicate_occupied=(0,0,0,3,0,0,1,2,0),duplicate_dismissed=(0,0,0,2,0,0,0,2,0),occupied_loan=(1,1,1,0,0,0,1,0,0),occupied_channel=(1,1,1,0,0,1,1,0,0),pending_claim=(1,1,1,0,0,0,0,2,2),provider_conflict=(2,0,0,2,0,0,0,2,0),alias_asset=(1,1,1,0,0,0,0,0,0),mixed_recorded=(1,1,1,0,3,0,0,0,0),mixed_pipeline=(1,1,1,0,6,1,0,0,0),mixed_samourai=(1,1,1,0,3,0,0,0,0),mixed_channel=(1,1,1,0,2,2,2,0,0))


def test_native_generated_counts_and_permutations():
    assert set(COUNTS)==set(NATIVE_SCENARIOS)
    counts={e:{case:Counter() for case in NATIVE_SCENARIOS} for e in ('Python','Rust')}
    for seed in range(2000):
        case=NATIVE_SCENARIOS[seed%len(NATIVE_SCENARIOS)];book=native_snapshot(seed)
        expected,actual=python_results(book),rust_results(book)
        assert_same(expected,actual,'native '+case)
        for engine,result in (('Python',expected),('Rust',actual)):
            native,safety=result['native'],result['safety']
            assert 'error' not in native and 'error' not in safety,(engine,case,native.get('error'),safety.get('error'))
            observed=(*(len(native[k]) for k in ('candidates','pairs','proofs','quarantines')),*(len(safety[k]) for k in ('claims','native_audits','non_event_transaction_ids','blocked_transaction_ids','transient_blocked_transaction_ids')))
            assert observed==COUNTS[case],(engine,seed,case,observed)
            counts[engine][case].update(books=1,**dict(zip(('candidates','pairs','proofs','native_quarantines','claims','audits','non_events','blocked','transient'),observed)))
        if seed<len(NATIVE_SCENARIOS)*2:
            p=permuted_snapshot(book);assert_same(python_results(p),rust_results(p),'permuted native '+case)
            from kassiber.core.custody_compute import compute_custody_decisions
            from tests.custody_snapshot import to_jsonable
            stages={}
            compute_custody_decisions(book,stage_observer=lambda n,v:stages.setdefault(n,v))
            full=to_jsonable(stages['interpretation'],redact_config=False)
            for key in ('claims','native_audits','non_event_transaction_ids','blocked_transaction_ids','quarantines','transient_blocked_transaction_ids'):
                assert_same(full[key],expected['safety'][key],'full Python native '+case+' '+key)
    for engine in counts:
        for index,case in enumerate(NATIVE_SCENARIOS):
            books=2000//len(NATIVE_SCENARIOS)+(index<2000%len(NATIVE_SCENARIOS))
            assert counts[engine][case]['books']==books
            for key,value in zip(('candidates','pairs','proofs','native_quarantines','claims','audits','non_events','blocked','transient'),COUNTS[case]):assert counts[engine][case][key]==books*value
    assert counts['Python']==counts['Rust']
    print('Native HTLC counts: '+json.dumps(counts,sort_keys=True))


def test_native_frozen_helpers():
    from tests.custody_native_vectors import PATH,vectors
    assert json.loads(PATH.read_text())==vectors()
