"""Two-engine lifecycle and loan controls, including complete Python composition."""
from collections import Counter
import json

from tests.custody_channel_generator import CHANNEL_SCENARIOS, channel_snapshot
from tests.custody_engine_compare import assert_same, permuted_snapshot
from tests.custody_interpretation_partial import python_results, rust_results

# roles, candidate pairs, claims, audits, non-events, blocked, transient
COUNTS = {
    "open_close": (2,2,0,2,2,0,0), "strict_lnd": (2,2,0,2,2,0,0),
    "strict_coreln": (2,2,0,2,2,0,0), "batched": (2,2,0,2,2,0,0),
    "force_trusted": (3,3,0,3,3,0,0), "force_untrusted": (3,1,0,1,1,2,0),
    "force_multiple": (4,4,0,4,4,0,0), "force_duplicate": (4,1,0,1,1,3,0),
    "peer_payment": (2,2,0,2,2,0,0), "fee_tie": (3,3,0,3,3,0,0),
    "missing_funding": (1,0,0,0,1,0,0), "open_mismatch": (2,0,0,0,1,1,0),
    "close_mismatch": (2,1,0,1,1,1,0), "strict_zero": (2,0,0,0,0,2,0),
    "owner_conflict": (2,0,0,0,1,1,0), "network_conflict": (1,1,0,1,1,0,0),
    "graph_conflict": (1,1,0,1,1,0,0), "pending_open": (2,2,0,1,2,1,1),
    "privacy_close": (2,2,0,2,2,1,0), "splice_tag": (2,2,0,2,2,0,0),
    "loan_collateral": (0,0,0,0,2,0,0), "loan_principal": (0,0,0,0,2,0,0),
    "loan_all": (0,0,0,0,4,0,0), "loan_pending": (0,0,0,0,1,1,1),
    "mixed_recorded": (2,2,1,2,2,0,0), "mixed_pipeline": (2,2,4,3,2,0,0),
    "mixed_samourai": (2,2,1,2,2,0,0), "loan_wrong_direction": (0,0,0,0,2,0,0),
    "loan_unknown_role": (0,0,0,0,1,0,0), "loan_empty_ids": (0,0,0,0,3,0,0),
    "strict_owner_conflict": (2,0,0,0,0,2,0), "multiple_close_scopes": (4,4,0,4,4,0,0),
}


def assert_channel_fee_conservation(book, channel):
    from collections import defaultdict
    from kassiber.core.lightning.channel_lifecycle import _channel_lifecycle_scope
    from kassiber.transfers import onchain_transfer_scope
    balances=defaultdict(int)
    for record in book["channel_records"]:
        if record["tag"] == "channel_close":
            scope=_channel_lifecycle_scope(record,record["txid"])
            if scope is not None and record["amount_msat"] > 0:
                balances[scope] += record["amount_msat"]
    groups=defaultdict(list)
    for pair in channel["pairs"]:
        assert pair["out"]["amount"] == pair["in"]["amount"]
        if pair["kind"] == "channel_open":
            assert pair["in"]["fee"] == 0
            continue
        inbound=pair["in"]
        scope=onchain_transfer_scope(inbound)
        raw=json.loads(inbound["raw_json"])
        scopes={scope[:3]} | {(scope[0],scope[1],vin["txid"]) for vin in raw.get("vin",[])}
        candidates=scopes & balances.keys()
        assert len(candidates)==1
        groups[next(iter(candidates))].append(pair)
    for scope,pairs in groups.items():
        receipts=sum(pair["in"]["amount"] for pair in pairs)
        expected_fee=max(0,balances[scope]-receipts)
        assert sum(pair["out"]["fee"] for pair in pairs) == expected_fee
        keeper=max(pairs,key=lambda pair:(pair["in"]["amount"],pair["in"]["id"]))
        for pair in pairs:
            assert pair["out"]["fee"] == (expected_fee if pair is keeper else 0)


def test_channel_generated_counts_and_permutations():
    assert set(COUNTS) == set(CHANNEL_SCENARIOS)
    counts = {engine: {case: Counter() for case in CHANNEL_SCENARIOS} for engine in ("Python", "Rust")}
    for seed in range(2000):
        case = CHANNEL_SCENARIOS[seed % len(CHANNEL_SCENARIOS)]
        book = channel_snapshot(seed)
        expected, actual = python_results(book), rust_results(book)
        assert_same(expected, actual, "channel/loan " + case)
        for engine, result in (("Python", expected), ("Rust", actual)):
            channel, safety = result["channels"], result["safety"]
            assert_channel_fee_conservation(book,channel)
            assert "error" not in channel and "error" not in safety, (seed,case,engine,result)
            observed = (len(channel["roles"]),len(channel["pairs"]),*(len(safety[k]) for k in ("claims","native_audits","non_event_transaction_ids","blocked_transaction_ids","transient_blocked_transaction_ids")))
            assert observed == COUNTS[case], (seed,case,engine,observed)
            counts[engine][case].update(books=1, roles=observed[0], pairs=observed[1], claims=observed[2], audits=observed[3], non_events=observed[4], blocked=observed[5], transient=observed[6])
        if seed < len(CHANNEL_SCENARIOS)*2:
            permuted=permuted_snapshot(book)
            assert_same(python_results(permuted),rust_results(permuted),"permuted channel/loan "+case)
            from kassiber.core.custody_compute import compute_custody_decisions
            from tests.custody_snapshot import to_jsonable
            stages={}
            compute_custody_decisions(book,stage_observer=lambda name,value:stages.setdefault(name,value))
            full=to_jsonable(stages["interpretation"],redact_config=False)
            for key in ("claims","native_audits","non_event_transaction_ids","blocked_transaction_ids","quarantines","transient_blocked_transaction_ids"):
                assert_same(full[key],expected["safety"][key],"full Python channel/loan "+key)
    for engine in counts:
        for index,case in enumerate(CHANNEL_SCENARIOS):
            books=2000//len(CHANNEL_SCENARIOS)+(index < 2000%len(CHANNEL_SCENARIOS))
            assert counts[engine][case]["books"] == books
            for key,value in zip(("roles","pairs","claims","audits","non_events","blocked","transient"),COUNTS[case]):
                assert counts[engine][case][key] == value*books
    assert counts["Python"] == counts["Rust"]
    print("Channel/loan producer counts: "+json.dumps(counts,sort_keys=True))


def test_channel_frozen_helpers():
    from tests.custody_channel_vectors import PATH,vectors
    assert json.loads(PATH.read_text()) == vectors()


def test_loan_authoring_directions_remain_separate_from_interpretation():
    from kassiber.core import loans
    from kassiber.errors import AppError
    import pytest
    for role, expected in (("collateral_lock","outbound"),("collateral_release","inbound"),("loan_principal_received","inbound"),("loan_principal_repaid","outbound")):
        loans._require_role(role)
        loans._require_role_direction(role,expected)
        other="inbound" if expected == "outbound" else "outbound"
        with pytest.raises(AppError) as error: loans._require_role_direction(role,other)
        assert error.value.code == "validation"
        assert error.value.details == dict(field="role",role=role,expected_direction=expected,actual_direction=other)
    # Interpretation consumes persisted marks without revalidating role/direction.
    # The two raw controls pin that legacy behavior, never grant loan ownership.
    for case in ("loan_wrong_direction","loan_unknown_role"):
        book=channel_snapshot(0,case=case)
        expected,actual=python_results(book),rust_results(book)
        assert_same(expected,actual,"persisted loan mark")
        assert set(expected["safety"]["non_event_transaction_ids"]) == {leg["transaction_id"] for leg in book["loan_legs"]}
