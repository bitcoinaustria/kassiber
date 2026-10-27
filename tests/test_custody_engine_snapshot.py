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
    from tests.custody_engine_compare import assert_same
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
    assert_same(expected, actual)


def test_snapshot_retains_excluded_and_retracted_component_anchors(tmp_path, monkeypatch):
    from tests import test_custody_components as fixtures
    from kassiber.core.custody_compute import compute_custody_projection
    from kassiber.core.custody_journal import CustodyJournalBuilder
    from tests.custody_reference import CustodyJournalBuilder as ReferenceBuilder
    from tests.custody_engine_compare import assert_same
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
            assert_same(expected, actual)
    finally:
        book.tearDown()
