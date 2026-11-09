"""CoinJoin shape rules: hand-written vectors, old-rule superset, suppression."""
import json
from itertools import product

import pytest

from kassiber.core import privacy_hygiene
from kassiber.core.chain_analysis import analyze_snapshot, build_index, run_analysis
from kassiber.core.chain_analysis import projection_store
from kassiber.core.privacy_hygiene import collaborative_transaction_evidence
from tests.coinjoin_shape_vectors import DISTINCT, EQUAL, LARGE, distinct, tx, vectors
from tests.test_chain_analysis import NOW, connection

VECTORS = vectors()


def hygiene_signal(raw, inventory=None):
    row = {"chain": "bitcoin", "network": "main", "asset": "BTC", "raw_json": json.dumps(raw),
           "external_id": raw["txid"], "privacy_boundary": None, "review_status": None, "fee": None}
    parsed = privacy_hygiene._parse_local_transaction(row, raw, inventory or {"outpoints": {}, "scripts": {}})
    return privacy_hygiene._coinjoin_signal(row, raw, parsed)


def test_vector_names_are_unique_and_cover_every_shape():
    names = [vector["name"] for vector in VECTORS]
    assert len(names) == len(set(names))
    assert {vector["expected"] for vector in VECTORS} == {EQUAL, LARGE, DISTINCT, None}
    gap = {vector["name"] for vector in VECTORS if vector["old"] is None and vector["expected"] == EQUAL}
    assert {f"gap_5in_{count}out" for count in range(13, 20)} <= gap


@pytest.mark.parametrize("vector", VECTORS, ids=[vector["name"] for vector in VECTORS])
def test_collaboration_evidence_matches_hand_written_shape(vector):
    evidence = collaborative_transaction_evidence({}, vector["raw"])
    if vector["expected"] is None:
        assert evidence is None, vector["why"]
    else:
        assert evidence == {"kind": "coinjoin", "source": vector["expected"], "evidence_level": "heuristic"}, vector["why"]


@pytest.mark.parametrize("vector", VECTORS, ids=[vector["name"] for vector in VECTORS])
def test_hygiene_finding_names_the_same_shape(vector):
    signal = hygiene_signal(vector["raw"])
    assert (signal or {}).get("pattern") == vector["expected"], vector["why"]
    if signal:
        assert signal["impact"] == privacy_hygiene.COINJOIN_SHAPE_IMPACT[vector["expected"]]
        assert signal["evidence_level"] == "heuristic"


def old_shape(inputs, outputs, top):
    if inputs >= 5 and 5 <= outputs <= 12 and top >= 5:
        return EQUAL
    if inputs >= 20 and outputs >= 20 and top >= 5:
        return LARGE
    return None


def test_every_shape_the_old_rules_recognized_keeps_its_name():
    for inputs, outputs in product(range(1, 31), range(1, 46)):
        for top in sorted({0, 2, 4, 5, 6, outputs}):
            if top > outputs:
                continue
            values = [500_000] * top + distinct(outputs - top, 610_000)
            raw = tx(1, distinct(inputs, 900_000), values)
            evidence = collaborative_transaction_evidence({}, raw)
            old = old_shape(inputs, outputs, top)
            if old is not None:
                assert evidence and evidence["source"] == old, (inputs, outputs, top)
            if 13 <= outputs <= 19 and inputs >= 5 and top >= 5:
                assert evidence and evidence["source"] == EQUAL, (inputs, outputs, top)


def test_owner_inventory_amounts_do_not_complete_the_public_ratio():
    raw = next(vector["raw"] for vector in VECTORS if vector["name"] == "wabisabi_without_input_values")
    known = next(vector["raw"] for vector in VECTORS if vector["name"] == "wabisabi_four_per_denomination")
    inventory = {"outpoints": {}, "scripts": {}}
    for entry, filled in zip(raw["vin"], known["vin"]):
        inventory["outpoints"][("bitcoin", "main", entry["txid"], entry["vout"])] = {
            "amount_sats": filled["prevout"]["value"], "address_type": "p2wpkh", "ambiguous": False}
    assert hygiene_signal(known)["pattern"] == DISTINCT
    assert hygiene_signal(raw, inventory) is None
    assert collaborative_transaction_evidence({}, raw) is None


def test_an_unreadable_input_leaves_the_ratio_unavailable():
    raw = next(vector["raw"] for vector in VECTORS if vector["name"] == "wabisabi_four_per_denomination")
    assert collaborative_transaction_evidence({}, raw)["source"] == DISTINCT
    assert collaborative_transaction_evidence({}, dict(raw, vin=[*raw["vin"], "not an input"])) is None


def test_reviewed_boundary_still_outranks_every_shape():
    raw = next(vector["raw"] for vector in VECTORS if vector["name"] == "wabisabi_four_per_denomination")
    evidence = collaborative_transaction_evidence({"privacy_boundary": "payjoin", "review_status": "accepted"}, raw)
    assert evidence == {"kind": "payjoin", "source": "reviewed_or_imported_privacy_boundary", "evidence_level": "reviewed"}


def _index(*names):
    conn = connection()
    by_name = {vector["name"]: vector for vector in VECTORS}
    for number, name in enumerate(names):
        raw = dict(by_name[name]["raw"], chain="bitcoin", network="main")
        conn.execute("INSERT INTO transactions VALUES(?,?,?,?,?,?,?,?,?,?)",
                     (str(number), "p", "w", raw["txid"], 1000, "BTC", json.dumps(raw), NOW, 0, None))
    return conn


@pytest.mark.parametrize("observer", ["owner", "public"])
def test_new_shapes_suppress_ownership_hypotheses_for_every_observer(observer):
    conn = _index("joinmarket_9_party", "gap_5in_13out", "wabisabi_four_per_denomination", "batch_5in_13out_distinct")
    try:
        index = build_index(conn, "p")
        result = analyze_snapshot(index, {"include_hypotheses": True, "observer": observer})
    finally:
        conn.close()
    boundaries = sorted(row["evidence"][0]["source"] for row in result["findings"] if row["code"] == "collaborative_boundary")
    assert boundaries == [DISTINCT, EQUAL, EQUAL]
    rules = [edge["rule"] for edge in result["edges"] if edge["kind"] == "hypothesis"]
    # Only the batch payout's five inputs are left to a common-input guess.
    assert rules == ["common_input_control"] * 4
    assert not result["clusters"] or all(len(cluster["node_ids"]) == 5 for cluster in result["clusters"])


def _boundaries(result):
    return [row["evidence"][0]["source"] for row in result["findings"] if row["code"] == "collaborative_boundary"]


def test_a_stored_older_projection_is_rebuilt_with_the_current_shapes():
    conn = _index("gap_5in_13out")
    try:
        assert _boundaries(run_analysis(conn, "p", {})) == [EQUAL]
        # What the previous rules stored: no boundary for a 13-output join.
        for column in ("tx_json", "public_tx_json"):
            ident, fact = conn.execute(f"SELECT id,{column} FROM chain_index_nodes WHERE {column} IS NOT NULL").fetchone()
            conn.execute(f"UPDATE chain_index_nodes SET {column}=? WHERE id=?",
                         (json.dumps(dict(json.loads(fact), collaboration=None)), ident))
        stale = run_analysis(conn, "p", {"include_hypotheses": True})
        assert _boundaries(stale) == [] and any(edge["kind"] == "hypothesis" for edge in stale["edges"])
        instance, revision = conn.execute("SELECT instance_id,revision FROM chain_index_state").fetchone()
        conn.execute("UPDATE chain_index_state SET version=?", (projection_store.VERSION - 1,))
        rebuilt = run_analysis(conn, "p", {"include_hypotheses": True})
        assert _boundaries(rebuilt) == [EQUAL]
        assert not [edge for edge in rebuilt["edges"] if edge["kind"] == "hypothesis"]
        state = conn.execute("SELECT instance_id,revision,version FROM chain_index_state").fetchone()
        assert tuple(state) == (instance, revision + 1, projection_store.VERSION)
    finally:
        conn.close()
