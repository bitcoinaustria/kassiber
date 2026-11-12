"""Differential runs for the wallet-sync gaps of the native port.

Each case builds the same book twice, syncs it with Python's CLI and with the
Rust binary directly (no hybrid handoff), and pins what each side does. A case
became a parity test when its gap was ported; see
docs/reference/rust-native-wallet-gaps.md.
"""
import copy
import json
import os
import subprocess
import sys

import pytest

from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan
from tests.rust_sync_chain import FOREIGN, Chain, serve
from tests.rust_sync_parity import connect, copy_book, dump, dump_bytes
from tests.test_rust_wallet_publication import _wallet
from tests.rust_sync_parity import fixed_elapsed_ms
from tests.test_rust_wallet_sync_cli import _args, _books, _environment
from tests.test_source_overlap_owner_order import FIRST_SOURCE_OWNER

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust binary")


def _book(path):
    return dump(path)


def _parity(chain, roots, books, env, *, all=True):
    """Python's CLI and the native binary agree on the envelope and every table,
    the Esplora history anchors included."""
    before = copy.deepcopy({k: v for k, v in chain.__dict__.items() if k not in {"lock", "hits"}})
    reference = subprocess.run(
        [sys.executable, "-m", "tests.rust_sync_cli_reference", *_args(roots[0], all=all)],
        capture_output=True, env=env, timeout=90,
    )
    with chain.lock:
        chain.__dict__.update(before)
    actual = subprocess.run([RUST_BIN, *_args(roots[1], all=all)], capture_output=True, env=env, timeout=90)
    assert actual.stderr == reference.stderr == b"", (reference.stderr, actual.stderr)
    expected, result = json.loads(reference.stdout), json.loads(actual.stdout)
    assert actual.returncode == reference.returncode, (expected, result)
    if expected.get("kind") != "error":
        expected["data"], result["data"] = fixed_elapsed_ms(expected["data"]), fixed_elapsed_ms(result["data"])
    assert result == expected
    assert _book(books[1]) == _book(books[0])
    return expected


def _add_wallet(books, wallet_id, label, config):
    for book in books:
        conn = connect(book)
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES(?,'w','p',?,'descriptor',?,'fixed')", (wallet_id, label, json.dumps(config)))
        conn.commit()
        conn.close()


def _address(config, branch=0, index=0):
    plan = load_descriptor_plan(config)
    return next(t for t in derive_descriptor_targets(plan, start=0, end=index + 1)
                if t.branch_index == branch and t.address_index == index)


def _address_wallet(addresses):
    return {"addresses": addresses, "chain": "bitcoin", "network": "main", "backend": "fixture"}


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_address_list_wallet(tmp_path, protocol):
    # Python's compatibility observer, route `bitcoin_script`, on both
    # protocols (Electrum's is gap E1, tests/test_rust_electrum_compatibility.py).
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    chain.fund(scripts[0][2], 20_000)
    first, third = _address(config), _address(config, index=2)
    roots, books = _books(tmp_path, _address_wallet([first.address, third.address]))
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url))
    row = expected["data"][0]
    assert (row["sync_mode"], row["observer_route"], row["addresses"], row["imported"]) == (
        "addresses", "bitcoin_script", f"{first.address},{third.address}", 2)


@pytest.mark.parametrize("all", [False, True], ids=["wallet", "all"])
def test_address_list_beside_its_descriptor_matches_python(tmp_path, all):
    # The descriptor wallet stays canonical for the shared script; the
    # address list keeps only the address the descriptor does not cover.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    shared = _address(config)
    foreign = _address(_wallet("tr", 3)[0])
    chain.fund(foreign.script_pubkey, 30_000)
    roots, books = _books(tmp_path, _address_wallet([shared.address, foreign.address]))
    _add_wallet(books, "b", "Hot", config)
    with serve(chain, "esplora") as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, "esplora", url), all=all)
    assert expected["data"][0]["target_count"] == 1


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("field,value,reason", [
    ("TIMEOUT", "45", "custom_timeout"),
    ("TIMEOUT", "30", "http_route_policy"),
    ("TOKEN", "secret-token", "custom_http_auth"),
    ("AUTH_HEADER", "Basic dXNlcjpwYXNz", "custom_http_auth"),
])
def test_backend_transport_options_match_python(tmp_path, protocol, field, value, reason):
    # Esplora always takes Python's compatibility observer; these options only
    # name its reason. Electrum keeps BDK with them.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        env[f"KASSIBER_BACKEND_FIXTURE_{field}"] = value
        expected = _parity(chain, roots, books, env)
    route = expected["data"][0].get("observer_compatibility_reason")
    assert route == (reason if protocol == "esplora" else None)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("all", [False, True], ids=["wallet", "all"])
def test_two_descriptor_wallets_match_python(tmp_path, protocol, all):
    first, first_scripts = _wallet("wpkh", 3)
    second, second_scripts = _wallet("tr", 3)
    chain = Chain()
    chain.fund(first_scripts[0][0], 50_000)
    chain.fund(second_scripts[0][1], 60_000)
    roots, books = _books(tmp_path, first)
    _add_wallet(books, "b", "Hot", second)
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        for index in range(2 if protocol == "electrum" else 1):
            env.update(_environment(tmp_path, protocol, url, index))
            expected = _parity(chain, roots, books, env, all=all)
    assert [row["status"] for row in expected["data"]] == ["synced", "synced"][: 2 if all else 1]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_duplicate_descriptor_wallet_is_skipped_like_python(tmp_path, protocol):
    # Both wallets watch the same descriptor: "Cold" sorts first and stays
    # canonical, so every target of "Hot" is skipped before any request.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][1], 50_000)
    roots, books = _books(tmp_path, config)
    _add_wallet(books, "b", "Hot", config)
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url))
        hot = expected["data"][1]
        assert (hot["status"], hot["filtered_overlap_targets"]) == ("skipped", 6)
        if protocol == "electrum":
            _parity(chain, roots, books, _environment(tmp_path, protocol, url, 1), all=False)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_overlap_with_an_address_list_keeps_the_descriptor_canonical(tmp_path, protocol):
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    roots, books = _books(tmp_path, config)
    address = _address(config)
    for book in books:
        conn = connect(book)
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES('b','w','p','Alpha','address',?,'fixed')",
                     (json.dumps({"addresses": [address.address], "chain": "bitcoin", "network": "main"}),))
        conn.commit()
        conn.close()
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url), all=False)
    assert expected["data"][0]["status"] == "synced"


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_inventory_owner_trims_a_descriptor(tmp_path, protocol):
    # "Alpha" holds inventory evidence for Cold's second receive script and
    # sorts first, so it is the canonical owner of that one script.
    config, scripts = _wallet("wpkh", 3)
    other, _ = _wallet("tr", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    chain.fund(scripts[0][1], 40_000)
    roots, books = _books(tmp_path, config)
    _add_wallet(books, "b", "Alpha", other)
    for book in books:
        conn = connect(book)
        conn.execute(
            "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,txid,vout,outpoint,asset,amount,chain,"
            "network,script_pubkey,confirmation_status,first_seen_at,last_seen_at) "
            "VALUES('u','w','p','b',?,0,?,'BTC',1000,'bitcoin','main',?,'confirmed','fixed','fixed')",
            ("ab" * 32, "ab" * 32 + ":0", scripts[0][1]))
        conn.commit()
        conn.close()
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url), all=False)
    # Online discovery widens the receive branch; Alpha's script is trimmed.
    assert expected["data"][0]["target_count"] == 7
    assert expected["data"][0]["observer_compatibility_reason"] == (
        "source_overlap_partial_descriptor" if protocol == "electrum" else "http_route_policy")


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_shared_descriptor_has_one_owner_like_python(tmp_path, protocol):
    # tests/rust_port/repro_source_overlap_index_order.py: the prefetch and
    # the publish re-check (and Esplora's online re-filter) name the same
    # owner, "Cold" (it has activity), so it imports the funded receive and
    # "Alpha" skips every shared target. Before the fix Python and Rust both
    # refused Cold on Electrum and imported the receive into neither wallet
    # on Esplora.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][1], 50_000)
    roots, books = _books(tmp_path, config)
    _add_wallet(books, "b", "Alpha", config)
    for book in books:
        conn = connect(book)
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,raw_json,created_at) VALUES('t','w','p','a','x','f','2025-01-01T00:00:00Z',"
            "'inbound','BTC',1000,0,'{}','fixed')")
        conn.commit()
        conn.close()
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url))
    alpha, cold = expected["data"]
    assert alpha["status"] == "skipped"
    assert (cold["status"], cold["imported"]) == ("synced", 1)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_sync_repairs_a_book_the_old_owner_rule_left_short(tmp_path, protocol):
    # Python's child emulates the old first-source owner rule for one round
    # (Electrum refuses Cold; Esplora syncs two targets and imports nothing),
    # then the native binary syncs a copy of that book and Cold imports the
    # receive, as Python's next round does on both protocols.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    txid, _ = chain.fund(scripts[0][1], 50_000)
    roots, books = _books(tmp_path, config)
    _add_wallet(books, "b", "Alpha", config)
    conn = connect(books[0])
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
        "direction,asset,amount,fee,raw_json,created_at) VALUES('t','w','p','a','x','f','2025-01-01T00:00:00Z',"
        "'inbound','BTC',1000,0,'{}','fixed')")
    conn.commit()
    conn.close()
    with serve(chain, protocol) as url:
        old = subprocess.run([sys.executable, "-c", FIRST_SOURCE_OWNER, *_args(roots[0], all=True)],
                             capture_output=True, env=_environment(tmp_path, protocol, url), timeout=90)
        alpha, cold = json.loads(old.stdout)["data"]
        assert alpha["status"] == "skipped"
        if protocol == "electrum":
            assert (cold["status"], cold["code"]) == ("error", "source_overlap_retry")
        else:
            assert (cold["status"], cold["target_count"], cold["imported"]) == ("synced", 2, 0)
        copy_book(books[0], books[1])
        result = _parity(chain, roots, books, _environment(tmp_path, protocol, url, 1))
    alpha, cold = result["data"]
    assert alpha["status"] == "skipped"
    assert (cold["status"], cold["imported"]) == ("synced", 1)
    conn = connect(books[1])
    assert {row[0] for row in conn.execute("SELECT external_id FROM transactions WHERE wallet_id='a'")} == {"x", txid}
    assert conn.execute("SELECT COUNT(*) FROM transactions WHERE wallet_id='b'").fetchone()[0] == 0
    conn.close()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_negative_balance_repair_matches_python(tmp_path, protocol):
    # A stored outbound with no funding is a prospective deficit, so Python
    # refetches once at the widened gap (500) before publishing and reports
    # the repair; it cannot resolve this one.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][1], 50_000)
    roots, books = _books(tmp_path, config)
    for book in books:
        conn = connect(book)
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,raw_json,created_at) VALUES('manual','w','p','a','manual-1','f',"
            "'2023-01-01T00:00:00Z','outbound','BTC',100000000,0,'{}','fixed')")
        conn.commit()
        conn.close()
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url), all=False)
    rescan = expected["data"][0]["negative_balance_rescan"]
    assert (rescan["triggered"], rescan["resolved"], rescan["rescan_gap_limit"]) == (True, False, 500)
    assert expected["data"][0]["gap_limit"] == 500


def _manual_outbound(books, amount_msat, occurred_at="2023-01-01T00:00:00Z"):
    for book in books:
        conn = connect(book)
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,raw_json,created_at) VALUES('manual','w','p','a','manual-1','f',"
            "?,'outbound','BTC',?,0,'{}','fixed')", (occurred_at, amount_msat))
        conn.commit()
        conn.close()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_address_list_rescan_after_apply_matches_python(tmp_path, protocol):
    # An address list has no gap to widen, so Python skips the prefetch
    # repair; the deficit left after applying re-syncs the wallet, forced,
    # inside the publication's savepoint (gap R2), and stays unresolved.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    roots, books = _books(tmp_path, _address_wallet([_address(config).address]))
    _manual_outbound(books, 100_000_000)
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url), all=False)
    row = expected["data"][0]
    rescan = row["negative_balance_rescan"]
    assert (rescan["triggered"], rescan["resolved"], rescan["original_gap_limit"], rescan["rescan_gap_limit"]) == (
        True, False, None, None)
    assert [event["external_id"] for event in rescan["initial_negative_events"]] == ["manual-1"]
    assert row["force_full"] is True
    assert dump_bytes(books[1]) == dump_bytes(books[0])


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_descriptor_rescan_after_apply_matches_python(tmp_path, protocol):
    # The book keeps the fetched receive excluded. The prospective overlay
    # counts the fetched record, so no deficit shows before applying; after
    # applying the receive is still excluded and the later outbound leaves a
    # deficit. Python's compatibility (Esplora) pass re-syncs at the widened
    # gap inside its savepoint; BDK (Electrum) passes never do.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    txid, _ = chain.fund(scripts[0][0], 50_000)
    roots, books = _books(tmp_path, config)
    for book in books:
        conn = connect(book)
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,raw_json,created_at,excluded) VALUES('kept','w','p','a',?,'g',"
            "'2023-01-01T00:00:00Z','inbound','BTC',50000000,0,'{}','fixed',1)", (txid,))
        conn.commit()
        conn.close()
    _manual_outbound(books, 40_000_000, "2030-01-01T00:00:00Z")
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url), all=False)
    rescan = expected["data"][0].get("negative_balance_rescan")
    if protocol == "electrum":
        assert rescan is None
    else:
        assert (rescan["triggered"], rescan["resolved"], rescan["original_gap_limit"], rescan["rescan_gap_limit"]) == (
            True, False, 3, 500)
    assert dump_bytes(books[1]) == dump_bytes(books[0])


def test_rescan_starts_from_the_published_checkpoint(tmp_path):
    # Round 2 drops a receive: the first pass retracts it and publishes a
    # history membership without it; the deficit then re-syncs from that
    # published checkpoint, so the rescan does not retract it again.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    dropped, _ = chain.fund(scripts[0][0], 50_000)
    chain.fund(scripts[0][0], 10_000)
    roots, books = _books(tmp_path, _address_wallet([_address(config).address]))
    _manual_outbound(books, 55_000_000, "2030-01-01T00:00:00Z")
    with serve(chain, "esplora") as url:
        first = _parity(chain, roots, books, _environment(tmp_path, "esplora", url), all=False)
        assert "negative_balance_rescan" not in first["data"][0]
        chain.drop(dropped)
        expected = _parity(chain, roots, books, _environment(tmp_path, "esplora", url, 1), all=False)
    row = expected["data"][0]
    assert row["negative_balance_rescan"]["triggered"] is True
    assert "bitcoinrpc_retracted_txids" not in row
    assert dump_bytes(books[1]) == dump_bytes(books[0])


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("name", ["wpkh-pair", "wpkh-multipath"])
def test_samourai_child_matches_python(tmp_path, protocol, name):
    # A Samourai child is a descriptor wallet observed under its parent's
    # logical wallet (`identities_for_wallet`); its identity's branch keys
    # follow `change_descriptor`, not the plan. The parent has no source.
    config, scripts = _wallet(name, 3)
    config["samourai"] = {"role": "child", "parent_wallet_id": "parent", "section": "postmix",
                          "script_type": "p2wpkh", "root_path": "m/84'/0'/2147483646'"}
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    chain.fund(scripts[1][1], 20_000)
    roots, books = _books(tmp_path, config)
    for book in books:
        conn = connect(book)
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES('parent','w','p','Samourai','samourai',?,'fixed')",
                     (json.dumps({"samourai": {"role": "parent"}, "backend": "fixture"}),))
        conn.commit()
        conn.close()
    with serve(chain, protocol) as url:
        for index in range(2):
            expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url, index))
    row = expected["data"][0]
    if protocol == "electrum" and name == "wpkh-multipath":
        # Python's identity keeps only `receive` without a change descriptor,
        # so the BDK observer's change coverage names an unknown branch.
        # Samourai imports always write the change descriptor separately.
        assert (row["status"], row["code"]) == ("error", "observer_state_invalid"), row
        return
    assert [row["status"], expected["data"][1]["status"]] == ["synced", "skipped"], row
    conn = connect(books[1])
    try:
        keys = conn.execute("SELECT logical_wallet_id, source_key FROM chain_observer_instances").fetchall()
    finally:
        conn.close()
    if protocol == "electrum":
        assert [tuple(row) for row in keys] == [("parent", "samourai:postmix:p2wpkh:m/84'/0'/2147483646'")]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_samourai_parent_with_a_descriptor_matches_python(tmp_path, protocol):
    # A parent observes nothing: Python's dependency route refuses typed
    # (`observer_identity_invalid`) rather than scanning it.
    config, scripts = _wallet("wpkh", 3)
    config["samourai"] = {"role": "parent"}
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, protocol, url), all=False)
    if protocol == "electrum":
        assert expected["error"]["code"] == "observer_identity_invalid"


@pytest.mark.parametrize("kind", ["coreln", "lnd"])
def test_node_kind_without_a_backend_syncs_its_chain_material(tmp_path, kind):
    # `sync_wallets` takes the node route only with a backend; otherwise a
    # node-kind wallet's addresses sync like any address list.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    first = _address(config)
    roots, books = _books(tmp_path, {"addresses": [first.address], "chain": "bitcoin", "network": "main"})
    for book in books:
        with connect(book) as conn:
            conn.execute("UPDATE wallets SET kind=?", (kind,))
    with serve(chain, "esplora") as url:
        expected = _parity(chain, roots, books, _environment(tmp_path, "esplora", url))
    assert expected["data"][0]["status"] == "synced", expected
