"""Ambiguous native hash evidence must not select a tax basis by omission."""
import json
from contextlib import contextmanager

import pytest

from kassiber.cli.handlers import process_journals
from kassiber.core.transfer_matching import suggest_swap_candidates
from kassiber.db import open_db
from tests.custody_tax_helpers import persist_authoritative_chain_observation
from tests import test_custody_native_transitions as native_tests
from tests.test_custody_native_transitions import native_transition_rows


def duplicate_rows(network=None):
    rows, refs = native_transition_rows()
    duplicate = {**rows[1], "id": "other-send", "wallet_id": "other-lightning", "external_id": "other-payment"}
    raw = dict(duplicate["raw_json"])
    raw.pop("network")
    if network:
        raw["network"] = network
    duplicate["raw_json"] = raw
    rows += [{**rows[0], "id": "other-initial", "wallet_id": "other-lightning", "external_id": "other-initial"}, duplicate]
    refs["other-lightning"] = {**refs["lightning-wallet"], "id": "other-lightning"}
    return rows, refs


@contextmanager
def native_book(root, network=None):
    conn = open_db(root)
    try:
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('workspace','W','2020')")
        conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,gains_algorithm,created_at) VALUES('profile','workspace','P','USD','generic','FIFO','2020')")
        rows, _ = duplicate_rows(network)
        for wallet_id in {row["wallet_id"] for row in rows}:
            kind = next(row["wallet_kind"] for row in rows if row["wallet_id"] == wallet_id)
            config = {"chain": "liquid", "network": "elementsregtest"} if kind != "lnd" else {"chain": "lightning"}
            if kind == "lnd" and (wallet_id != "other-lightning" or network):
                config["network"] = network or "regtest"
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'workspace','profile',?,?,?,'2020')", (wallet_id, wallet_id, kind, json.dumps(config)))
        columns = {row["name"] for row in conn.execute("PRAGMA table_info(transactions)")}
        for original in rows:
            row = {key: value for key, value in original.items() if key in columns}
            row["fingerprint"] = "fp-" + row["id"]
            if not isinstance(row["raw_json"], str):
                row["raw_json"] = json.dumps(row["raw_json"])
            names = tuple(row)
            conn.execute(f"INSERT INTO transactions ({','.join(names)}) VALUES ({','.join('?' for _ in names)})", tuple(row[name] for name in names))
            if original["wallet_kind"] != "lnd":
                persist_authoritative_chain_observation(conn, row["id"], observer_kind="lwk")
        conn.commit()
        yield conn
    finally:
        conn.close()


@pytest.mark.parametrize("network", [None, "regtest"])
def test_process_journals_holds_ambiguous_native_hash_route(tmp_path, network):
    with native_book(tmp_path, network) as conn:
        result = process_journals(conn, "workspace", "profile")
        assert result["transfers_detected"] == 0
        holds = {row["transaction_id"] for row in conn.execute("SELECT transaction_id FROM journal_quarantines WHERE reason='native_transition_ambiguous'")}
        assert holds == {"send", "claim", "other-send"}
        # Neither arbitrary carrying basis nor a fresh priced acquisition may
        # settle this route while the originating native payment is ambiguous.
        assert not conn.execute("SELECT 1 FROM journal_entries WHERE transaction_id IN ('send','claim','other-send') AND entry_type IN ('disposal','acquisition','transfer_in','transfer_out')").fetchone()
        assert conn.execute("SELECT COUNT(*) FROM transaction_pairs").fetchone()[0] == 0
        assert conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0] == 0


def test_unknown_hash_scope_competes_but_known_distinct_network_does_not():
    rows, _ = duplicate_rows()
    assert not suggest_swap_candidates(rows, include_heuristics=False)
    rows, _ = duplicate_rows("main")
    candidates = suggest_swap_candidates(rows, include_heuristics=False)
    assert [(c.out_id, c.in_id, c.confidence) for c in candidates] == [("send", "claim", "exact")]


def test_raw_provider_hashes_do_not_create_native_ambiguity_holds():
    rows, refs = duplicate_rows()
    rows[2].pop("observation_authority_version")
    compiled = native_tests.NativeTransitionAuthorityTest().compile(rows, refs)
    assert not any(row["reason"] == "native_transition_ambiguous" for row in compiled.quarantines)


def ambiguity_ids(compiled):
    return {row["transaction_id"] for row in compiled.quarantines if row["reason"] == "native_transition_ambiguous"}


@pytest.mark.parametrize("network", [None, "regtest"])
def test_full_population_survives_occupancy_and_pair_dismissals(network):
    rows, refs = duplicate_rows(network)
    compile_ = native_tests.NativeTransitionAuthorityTest().compile
    occupied = compile_(rows, refs, component_transaction_ids=("other-send",))
    assert ambiguity_ids(occupied) == {"send", "claim"}
    dismissed = compile_(rows, refs, swap_dismissals=[{
        "out_transaction_id": "other-send", "in_transaction_id": "claim",
    }])
    assert ambiguity_ids(dismissed) == {"send", "claim"}
    assert not any(claim.reason == "native_htlc_transition" for claim in dismissed.claims)
    all_dismissed = compile_(rows, refs, swap_dismissals=[
        {"out_transaction_id": source, "in_transaction_id": "claim"}
        for source in ("send", "other-send")
    ])
    assert not ambiguity_ids(all_dismissed)


def test_retracting_duplicate_recomputes_and_clears_derived_hold(tmp_path):
    with native_book(tmp_path) as conn:
        process_journals(conn, "workspace", "profile")
        assert conn.execute("SELECT 1 FROM journal_quarantines WHERE reason='native_transition_ambiguous'").fetchone()
        # Match the persisted result of an explicit duplicate-record exclusion;
        # rebuilding must not leave a durable authored hold behind.
        conn.execute("UPDATE transactions SET excluded=1 WHERE id='other-send'")
        conn.commit()
        result = process_journals(conn, "workspace", "profile")
        assert result["transfers_detected"] == 1
        assert not conn.execute("SELECT 1 FROM journal_quarantines WHERE reason='native_transition_ambiguous'").fetchone()


def test_unrelated_known_network_does_not_create_native_hold():
    rows, refs = duplicate_rows("main")
    compiled = native_tests.NativeTransitionAuthorityTest().compile(rows, refs)
    assert not ambiguity_ids(compiled)
    assert any(claim.reason == "native_htlc_transition" for claim in compiled.claims)
