"""The test-only custody snapshot recorder (tests/custody_snapshot.py)."""

import json
from unittest.mock import patch

import pytest

from kassiber.core import custody_gaps, custody_journal
from tests import custody_snapshot
from tests.test_custody_observation_consistency import (  # noqa: F401 - fixture
    SCRIPT_A,
    SCRIPT_B,
    _graph,
    _native_leg,
    _profile,
    _source_inventory,
    custody_book,
)


AS_OF = "2026-03-04T05:06:07.123456Z"
SECRETS = (
    "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz",
    "wpkh([d34db33f/84h/0h/0h]tpubD6NzVbkrYhZ4XgiXtGrdW5XDAPFCL9h7we1vwNCpn8tGbBcgfVYjXyhWo4E1xkh56hjod1RhGjxbaTLV3X4FyWuejifB9jusQ46QzG87VKp/0/*)",
    "an-api-token-that-must-stay-local",
)


@pytest.fixture
def recorder_dir(tmp_path, monkeypatch):
    """Record into a fresh directory, whatever the session installed.

    Tests that patch the build in-process cannot be rebuilt elsewhere, so
    verification is off unless a test turns it on.
    """

    monkeypatch.delenv(custody_snapshot.VERIFY_ENV_VAR, raising=False)
    directory = tmp_path / "snapshots"
    directory.mkdir()
    previous = custody_snapshot.install(directory)
    try:
        yield directory
    finally:
        custody_journal.install_build_observer_factory(previous)


@pytest.fixture
def owned_book(custody_book):  # noqa: F811 - pytest fixture
    """A receipt whose own input proves a missing debit: the owned index is consulted."""

    conn, helper = custody_book
    txid = "dd" * 32
    _source_inventory(conn, helper, txid)
    graph = _graph(txid, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)], outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.commit()
    return conn


def _build(conn, as_of=AS_OF):
    return custody_journal.CustodyJournalBuilder(conn, _profile(conn), as_of=as_of).build_custody_projection()


def _snapshots(directory):
    return sorted(directory.glob("*.json"))


def test_without_a_recorder_a_build_observes_and_writes_nothing(owned_book, tmp_path, monkeypatch):
    previous = custody_journal.install_build_observer_factory(None)
    try:
        # Product code never reads the variable; a bogus value changes nothing.
        monkeypatch.setenv(custody_snapshot.ENV_VAR, str(tmp_path / "missing"))
        projection = _build(owned_book)
    finally:
        custody_journal.install_build_observer_factory(previous)
    assert projection.finalized_tax_projection.rows
    assert not (tmp_path / "missing").exists()


def test_each_build_writes_one_snapshot_that_round_trips(owned_book, recorder_dir):
    first = _build(owned_book)
    second = _build(owned_book)
    assert first.quantity_state == second.quantity_state

    paths = _snapshots(recorder_dir)
    assert len(paths) == 2
    assert custody_snapshot.check(recorder_dir) == []
    documents = [custody_snapshot.load(path) for path in paths]
    for path, document in zip(paths, documents):
        assert path.read_text(encoding="ascii") == custody_snapshot._canonical_text(document) + "\n"
        assert document["schema"] == custody_snapshot.SCHEMA
        assert document["input"]["as_of"] == AS_OF
        assert len(document["input"]["transactions"]) == len(first.rows)
        assert "output" in document and "error" not in document
    assert documents[0]["input_digest"] == documents[1]["input_digest"]
    assert custody_snapshot._comparable(documents[0]) == custody_snapshot._comparable(documents[1])


def test_the_restricted_owned_index_answers_every_lookup(owned_book, recorder_dir):
    _build(owned_book)
    (document,) = [custody_snapshot.load(path) for path in _snapshots(recorder_dir)]
    lookups = document["observed"]["owned_index_lookups"]
    assert any(result for _method, _args, result in lookups)
    assert document["input"]["owned_index"]["by_script"]
    assert custody_snapshot.uncovered_lookups(document) == []


def test_wallet_configuration_is_reduced_to_the_facts_custody_reads(owned_book, recorder_dir):
    config = {
        "chain": "bitcoin",
        "network": "main",
        "addresses": ["bc1qexample"],
        "xpub": SECRETS[0],
        "descriptor_backup": SECRETS[1],
        "api_token": SECRETS[2],
    }
    owned_book.execute("UPDATE wallets SET config_json = ? WHERE id = 'wallet-a'", (json.dumps(config),))
    _build(owned_book)
    (path,) = _snapshots(recorder_dir)
    text = path.read_text(encoding="ascii")
    for secret in SECRETS:
        assert secret not in text
    document = json.loads(text)
    wallet = next(item for item in document["input"]["wallets"] if item["id"] == "wallet-a")
    assert json.loads(wallet["config_json"]) == {"chain": "bitcoin", "network": "main"}
    assert wallet["config_text_mentions_samourai"] is False
    # Tax rows are copies of the transaction rows; their configuration goes too.
    rows = document["output"]["finalized_tax_projection"]["rows"]
    assert rows and all(
        "config_json" not in row or set(row["config_json"]) == {"$wallet_config"} for row in rows
    )


def test_a_snapshot_with_wallet_key_material_is_never_written(owned_book, recorder_dir):
    owned_book.execute(
        "UPDATE wallets SET config_json = ? WHERE id = 'wallet-a'",
        (json.dumps({"chain": "bitcoin", "xpub": SECRETS[0]}),),
    )
    real_document = custody_snapshot.SnapshotRecorder._document

    def leak_at_serialization(recorder, **result):
        document = real_document(recorder, **result)
        document["injected_wallet_material"] = SECRETS[0]
        return document

    # Inject at the recorder's final barrier: the computation still receives
    # the same valid, reduced request that Rust compares.
    with patch.object(custody_snapshot.SnapshotRecorder, "_document", leak_at_serialization):
        with pytest.raises(AssertionError, match="key material"):
            _build(owned_book)
    assert _snapshots(recorder_dir) == []


def test_a_failed_build_records_its_error(owned_book, recorder_dir):
    from tests.custody_engine_compare import injected_stage_failure
    error = RuntimeError("boom")
    with injected_stage_failure("gaps", error), patch.object(custody_gaps, "compute_gap_search_result", side_effect=error):
        with pytest.raises(RuntimeError, match="boom"):
            _build(owned_book)
    (document,) = [custody_snapshot.load(path) for path in _snapshots(recorder_dir)]
    assert document["error"] == {"type": "RuntimeError", "message": "boom"}
    assert "output" not in document
    assert custody_snapshot.check(recorder_dir) == []


def test_verification_rebuilds_in_a_fresh_process(owned_book, recorder_dir, monkeypatch):
    monkeypatch.setenv(custody_snapshot.VERIFY_ENV_VAR, "1")
    _build(owned_book)
    assert len(_snapshots(recorder_dir)) == 1
    assert list(recorder_dir.glob("*.replay")) == []


def test_verification_fails_on_a_different_rebuild(owned_book, recorder_dir, monkeypatch):
    monkeypatch.setenv(custody_snapshot.VERIFY_ENV_VAR, "1")
    real = custody_snapshot._replay_elsewhere

    def differing(builder, document):
        replay = real(builder, document)
        replay["output"]["warnings"] = [{"code": "only_in_the_replay"}]
        return replay

    with patch.object(custody_snapshot, "_replay_elsewhere", side_effect=differing):
        with pytest.raises(AssertionError, match="fresh process"):
            _build(owned_book)
    assert len(list(recorder_dir.glob("*.replay"))) == 1


def test_builds_pair_across_runs_by_inputs_and_only_then_by_clock():
    timeless = {"input_digest": "abc", "input": {"as_of": "x", "pair_dismissals": [{"expires_at": None}]}}
    assert custody_snapshot.pairing_key(timeless) == "abc"
    expiring = {"input_digest": "abc", "input": {"as_of": "x", "pair_dismissals": [{"expires_at": "2026-01-01T00:00:00Z"}]}}
    assert custody_snapshot.pairing_key(expiring) == "abc@x"


def test_configuration_facts_keep_only_what_custody_reads():
    facts = custody_snapshot.config_facts(json.dumps({
        "chain": "bitcoin",
        "descriptor": SECRETS[1],
        "samourai": {"role": "child", "group_id": "g", "section": "postmix", "note": "free text"},
    }))
    assert facts == {
        "chain": "bitcoin",
        "samourai": {"role": "child", "group_id": "g", "section": "postmix"},
        "$text_mentions_samourai": True,
    }
    assert custody_snapshot.config_facts('{"samourai": "not a block"}')["samourai"] is None
    assert custody_snapshot.config_facts("{broken") == {"$unparsable": True}
    assert custody_snapshot.config_facts(None) is None


def test_row_types_of_both_database_drivers_encode_as_mappings():
    class DriverRow:
        """Shaped like the SQLCipher driver's row: keys and indexing, no Mapping."""

        def keys(self):
            return ["id", "expires_at"]

        def __getitem__(self, key):
            return {"id": "d", "expires_at": None}[key]

    assert custody_snapshot.to_jsonable([DriverRow()]) == [{"id": "d", "expires_at": None}]


def test_recordings_are_owner_only_even_with_permissive_umask(owned_book, recorder_dir):
    import os
    import stat
    old = os.umask(0)
    try:
        _build(owned_book)
    finally:
        os.umask(old)
    assert all(stat.S_IMODE(path.stat().st_mode) == 0o600 for path in recorder_dir.iterdir())


def test_recorder_detects_a_match_omitted_from_restricted_index(owned_book, recorder_dir, monkeypatch):
    original = custody_snapshot.SnapshotRecorder._document

    def omit(recorder, **result):
        document = original(recorder, **result)
        snapshot = document["input"]
        encoded = snapshot["owned_index"]
        assert encoded["by_script"]
        # Corrupt only the artifact being audited. The loader and frozen-flow
        # gates still compare the valid production snapshot and computation.
        document["input"] = {**snapshot, "owned_index": {
            **encoded, "by_script": {}, "by_outpoint": {}, "by_scoped_outpoint": [],
            "txid_wallets": {}, "txid_wallets_by_scope": [],
        }}
        document["input_digest"] = custody_snapshot.digest({
            key: value for key, value in document["input"].items() if key != "as_of"
        })
        return document

    monkeypatch.setattr(custody_snapshot.SnapshotRecorder, "_document", omit)
    _build(owned_book)
    (document,) = [custody_snapshot.load(path) for path in _snapshots(recorder_dir)]
    assert custody_snapshot.uncovered_lookups(document)


def test_component_error_keeps_precedence_over_receipt_source_error(owned_book, monkeypatch):
    from kassiber.core import custody_components, custody_snapshot as product, source_overlap
    monkeypatch.setattr(source_overlap, "_receipt_overlap_candidate_groups", lambda rows: [rows])
    def source_failure(*args, **kwargs):
        raise RuntimeError("receipt source failure")
    def component_failure(*args, **kwargs):
        raise ValueError("component failure")
    monkeypatch.setattr(source_overlap, "build_profile_source_index", source_failure)
    monkeypatch.setattr(custody_components, "iter_authored_active_components", component_failure)
    with pytest.raises(ValueError, match="component failure"):
        product.load_custody_snapshot(owned_book, _profile(owned_book), as_of=AS_OF)


@pytest.mark.parametrize("suffix", ["json", "replay"])
def test_both_recording_file_kinds_are_created_owner_only(tmp_path, suffix):
    import os
    import stat
    path = tmp_path / f"recording.{suffix}"
    old = os.umask(0)
    try:
        with custody_snapshot.owner_only_file(path) as handle:
            handle.write("{}\n")
    finally:
        os.umask(old)
    assert stat.S_IMODE(path.stat().st_mode) == 0o600
    with pytest.raises(FileExistsError):
        custody_snapshot.owner_only_file(path)
