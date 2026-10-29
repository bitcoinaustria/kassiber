"""The raw R2 request contains enough local facts without private configuration."""
import json

from kassiber.core.custody_snapshot import load_custody_snapshot
from tests.test_custody_observation_consistency import custody_book, _profile  # noqa: F401

AS_OF = "2026-01-01T00:00:00Z"


def test_snapshot_is_raw_json_with_only_configuration_facts(custody_book):
    conn, helper = custody_book
    helper._tx(conn, tx_id="receipt", wallet_id="wallet-a", direction="inbound", amount=100000, external_id="receipt", raw_json="{}")
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps({"chain": "bitcoin", "network": "main", "descriptor_backup": "private-policy-must-not-cross-the-seam", "samourai": {"role": "child", "group_id": "group", "section": "postmix", "xpub": "private-key"}}),))
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    encoded = json.dumps(snapshot)
    assert "private-policy" not in encoded and "private-key" not in encoded
    assert snapshot["schema_version"] == 1
    assert snapshot["as_of"] == AS_OF
    assert snapshot["owned_index"] is None
    assert snapshot["transactions"][0]["id"] == "receipt"
    assert not {"canonical_input", "interpretation", "validation", "finalized_projection"} & snapshot.keys()


def test_snapshot_runs_every_stage_with_database_access_forbidden(custody_book, monkeypatch):
    from kassiber.core.custody_compute import compute_custody_projection, STAGES
    from kassiber.core.custody_component_facts import ComponentFacts
    from tests.custody_reference import CustodyJournalBuilder as ReferenceBuilder
    from tests.custody_engine_compare import assert_same, normalized_reference
    conn, helper = custody_book
    helper._tx(conn, tx_id="receipt", wallet_id="wallet-a", direction="inbound", amount=100000, external_id="receipt", raw_json="{}")
    profile = _profile(conn)
    expected = ReferenceBuilder(conn, profile, as_of=AS_OF).build_custody_projection()
    snapshot = json.loads(json.dumps(load_custody_snapshot(conn, profile, as_of=AS_OF)))
    conn.set_authorizer(lambda *_: 1)  # SQLite denies every subsequent query.
    def forbidden(*_args, **_kwargs):
        raise AssertionError("pure custody queried a database")
    monkeypatch.setattr(ComponentFacts, "execute", forbidden)
    monkeypatch.setattr("sqlite3.connect", forbidden)
    stages = {}
    actual = compute_custody_projection(snapshot, stage_observer=lambda name, value: stages.setdefault(name, value))
    assert tuple(stages) == STAGES
    assert_same(normalized_reference(expected), actual)


def test_snapshot_retains_excluded_and_retracted_component_anchors(tmp_path, monkeypatch):
    from tests import test_custody_components as fixtures
    from kassiber.core.custody_compute import compute_custody_projection
    from kassiber.core.custody_journal import CustodyJournalBuilder
    from tests.custody_reference import CustodyJournalBuilder as ReferenceBuilder
    from tests.custody_engine_compare import assert_same, normalized_reference
    book = fixtures.CustodyComponentApiTests()
    book.setUp()
    try:
        component = fixtures.activate_component(book.conn, book._balanced_component()["id"])
        for state in ("excluded", "retracted"):
            if state == "excluded":
                book.conn.execute("UPDATE transactions SET excluded=1 WHERE id='out'")
            else:
                book.conn.execute("DELETE FROM transactions WHERE id='out'")
            profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
            snapshot = CustodyJournalBuilder(book.conn, profile, as_of=AS_OF).load_snapshot()
            assert snapshot["anchors"]["out"] is None if state == "retracted" else snapshot["anchors"]["out"]["excluded"] == 1
            expected = ReferenceBuilder(book.conn, profile, as_of=AS_OF).build_custody_projection()
            actual = compute_custody_projection(json.loads(json.dumps(snapshot)))
            assert actual.component_blockers[0]["component_id"] == component["id"]
            assert_same(normalized_reference(expected), actual)
    finally:
        book.tearDown()


def test_large_book_snapshot_retains_the_sql_worklist(custody_book, monkeypatch):
    from kassiber.core import custody_gaps
    from kassiber.core.custody_compute import compute_custody_projection
    from tests.custody_reference import CustodyJournalBuilder as ReferenceBuilder
    from tests.custody_engine_compare import assert_same, normalized_reference
    conn, helper = custody_book
    for i in range(5):
        helper._tx(conn, tx_id=f"row-{i}", wallet_id="wallet-a" if i < 2 else "wallet-b", direction="outbound" if i < 2 else "inbound", amount=100000 if i < 2 else 99000, external_id=f"row-{i}", raw_json="{}")
    monkeypatch.setattr(custody_gaps, "DEFAULT_MAX_INPUT_ROWS", 2)
    profile = _profile(conn)
    snapshot = load_custody_snapshot(conn, profile, as_of=AS_OF)
    assert snapshot["gap_worklist"]["large_book"] is True
    assert snapshot["gap_worklist"]["rows"]
    expected = ReferenceBuilder(conn, profile, as_of=AS_OF).build_custody_projection()
    conn.set_authorizer(lambda *_: 1)
    assert_same(normalized_reference(expected), compute_custody_projection(json.loads(json.dumps(snapshot))))


def test_binding_refuses_unknown_schema_fractional_msat_and_private_config(custody_book):
    import copy
    import kassiber_tax
    conn, _helper = custody_book
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    invalid = []
    for key, value in (("schema_version", 2), ("operation", "ledger")):
        book = copy.deepcopy(snapshot)
        book[key] = value
        invalid.append(book)
    book = copy.deepcopy(snapshot)
    book["transactions"] = [{"amount": 1.5, "fee": 0}]
    invalid.append(book)
    book = copy.deepcopy(snapshot)
    book["wallets"][0]["config_json"] = json.dumps({"descriptor": "private-policy"})
    invalid.append(book)
    for book in invalid:
        result = json.loads(kassiber_tax.custody(json.dumps(book)))
        assert result["error"]["code"] == "custody_snapshot_invalid"
        assert result["error"]["stage"] == "snapshot"
        assert "stages" not in result
        assert "private-policy" not in json.dumps(result)


def test_receipt_sources_include_only_the_referenced_output_scripts(custody_book):
    from kassiber.core.custody_compute import compute_custody_projection
    from kassiber.core.source_overlap import build_profile_source_index
    from tests.custody_reference import CustodyJournalBuilder as ReferenceBuilder
    from tests.custody_engine_compare import assert_same, normalized_reference
    from tests.test_source_overlap import ADDR_A, ADDR_B, ADDR_C, _script

    conn, helper = custody_book
    txid = "dd" * 32
    graph = {"chain": "bitcoin", "network": "main", "txid": txid,
             "vin": [], "vout": [{"n": i, "scriptpubkey": _script(address), "value_sats": 100}
                                  for i, address in enumerate((ADDR_A, ADDR_B))]}
    for wallet, address in (("wallet-a", ADDR_A), ("wallet-b", ADDR_B)):
        conn.execute("UPDATE wallets SET kind='address',config_json=? WHERE id=?",
                     (json.dumps({"chain": "bitcoin", "network": "main", "addresses": [address, ADDR_C]}), wallet))
        helper._tx(conn, tx_id=wallet, wallet_id=wallet, direction="inbound", amount=100000,
                   external_id=txid, raw_json=json.dumps(graph))
    profile = _profile(conn)
    assert _script(ADDR_C) in {source.script_pubkey for source in build_profile_source_index(conn, profile["id"]).sources}
    snapshot = load_custody_snapshot(conn, profile, as_of=AS_OF)
    assert {source["script_pubkey"] for source in snapshot["receipt_sources"]} == {_script(ADDR_A), _script(ADDR_B)}
    expected = ReferenceBuilder(conn, profile, as_of=AS_OF).build_custody_projection()
    conn.set_authorizer(lambda *_: 1)
    assert_same(normalized_reference(expected), compute_custody_projection(snapshot))
