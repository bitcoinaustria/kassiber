"""Bitcoin Core RPC wallet sync, natively (round 2, C6).

Python's `bitcoinrpc_sync_adapter` keeps one watch-only wallet per Kassiber
wallet on the node, imports the descriptor's range (or the addresses) with a
rescan, then reads the wallet's transaction details, verbose transactions and
unspent outputs. Python's CLI and the native binary sync copies of one book
against a loopback Core fake over the shared chain model; the fake's wallet
state is reset between the two runs. Envelopes and every stored byte must
match.
"""
import copy
import json
import os
import subprocess
import sys

import pytest

from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan
from tests.rust_core_chain import Node, serve
from tests.rust_sync_chain import FOREIGN, Chain
from tests.rust_sync_parity import connect, dump_bytes
from tests.test_rust_wallet_publication import _wallet
from tests.rust_sync_parity import fixed_elapsed_ms
from tests.test_rust_wallet_sync_cli import _args, _books
from tests.test_rust_wallet_sync_cli import _environment as environment

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust binary")


def _environment(tmp_path, url, index=0, **extra):
    env = environment(tmp_path, "bitcoinrpc", url, index)
    env.update({"KASSIBER_BACKEND_FIXTURE_USERNAME": "fixture", "KASSIBER_BACKEND_FIXTURE_PASSWORD": "secret"})
    for key, value in extra.items():
        env[f"KASSIBER_BACKEND_FIXTURE_{key.upper()}"] = str(value)
    return env


def _parity(chain, node, roots, books, env, *, force_full=False):
    """Both implementations on copies of one book, from the same chain and
    node state: the same envelope (elapsed time aside), the same bytes and
    the same Core calls."""
    before = copy.deepcopy({k: v for k, v in chain.__dict__.items() if k not in {"lock", "hits"}})
    state = copy.deepcopy((node.wallets, node.loaded))
    node.calls.clear()
    reference = subprocess.run(
        [sys.executable, "-m", "tests.rust_sync_cli_reference", *_args(roots[0], force_full=force_full)],
        capture_output=True, env=env, timeout=120)
    python_calls = list(node.calls)
    with chain.lock:
        chain.__dict__.update(before)
    node.wallets, node.loaded = copy.deepcopy(state)
    node.calls.clear()
    actual = subprocess.run([RUST_BIN, *_args(roots[1], force_full=force_full)],
                            capture_output=True, env=env, timeout=120)
    assert actual.stderr == reference.stderr == b"", (reference.stderr, actual.stderr)
    expected, result = json.loads(reference.stdout), json.loads(actual.stdout)
    assert actual.returncode == reference.returncode, (expected, result)
    if expected.get("kind") != "error":
        expected["data"], result["data"] = fixed_elapsed_ms(expected["data"]), fixed_elapsed_ms(result["data"])
    assert result == expected
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    canonical = lambda calls: sorted(json.dumps(call, sort_keys=True) for call in calls)
    assert canonical(node.calls) == canonical(python_calls)
    return expected


def _targets(config, end=4):
    return [t for t in derive_descriptor_targets(load_descriptor_plan(config), start=0, end=end)]


def _funded_chain(config):
    targets = _targets(config)
    receive = [t for t in targets if t.branch_index == 0]
    change = [t for t in targets if t.branch_index == 1]
    chain = Chain()
    first, _ = chain.fund(receive[0].script_pubkey, 50_000)
    chain.fund(receive[2].script_pubkey, 20_000)
    # A spend with change: Core lists the foreign output as a send.
    chain.spend([(first, 0)], [(FOREIGN, 30_000), (change[0].script_pubkey, 19_000)], height=15)
    return chain, receive, change


def test_descriptor_wallet_rounds_match_python(tmp_path):
    config, _ = _wallet("wpkh", 3)
    chain, receive, _ = _funded_chain(config)
    node = Node(chain)
    roots, books = _books(tmp_path, config)
    with serve(node) as url:
        first = _parity(chain, node, roots, books, _environment(tmp_path, url))
        row = first["data"][0]
        assert (row["status"], row["observer_route"], row["bitcoinrpc_sync_mode"]) == (
            "synced", "bitcoin_core_rpc", "full_scan"), row
        assert row["core_wallet_created"] is True
        # Used scripts widen the imported range, which replays the history;
        # once the range is stable a round reads only what follows the last
        # block.
        chain.fund(receive[1].script_pubkey, 7_000)
        second = _parity(chain, node, roots, books, _environment(tmp_path, url, 1))
        assert second["data"][0]["imported_descriptors"] == 2
        # (The fixture rebuilds its blocks for every new funding, which a
        # stored last block cannot follow, so this round adds nothing.)
        third = _parity(chain, node, roots, books, _environment(tmp_path, url, 2))
        assert (third["data"][0]["bitcoinrpc_sync_mode"], third["data"][0]["imported_descriptors"]) == (
            "sinceblock", 0)
        # A forced round re-imports the range and replays the full history.
        forced = _parity(chain, node, roots, books, _environment(tmp_path, url, 3), force_full=True)
        assert forced["data"][0]["bitcoinrpc_sync_mode"] == "full_scan"


def test_address_list_matches_python(tmp_path):
    config, _ = _wallet("wpkh", 3)
    chain, receive, _ = _funded_chain(config)
    node = Node(chain)
    addresses = {"addresses": [receive[0].address, receive[2].address], "chain": "bitcoin",
                 "network": "main", "backend": "fixture"}
    roots, books = _books(tmp_path, addresses)
    with serve(node) as url:
        expected = _parity(chain, node, roots, books, _environment(tmp_path, url, wallet="kassiber-watch"))
        row = expected["data"][0]
        assert (row["status"], row["sync_mode"], row["core_wallet"]) == ("synced", "addresses", "kassiber-watch"), row
        # The node already watches both addresses: nothing is imported again.
        again = _parity(chain, node, roots, books, _environment(tmp_path, url, 1, wallet="kassiber-watch"))
    assert again["data"][0]["imported_addresses"] == 0, again


def test_recreated_node_wallet_replays_like_python(tmp_path):
    # A node wallet that disappeared since the last round is created again;
    # the stored checkpoint no longer describes it, so the range is imported
    # and the full history read again.
    config, _ = _wallet("wpkh", 3)
    chain, _, _ = _funded_chain(config)
    node = Node(chain)
    roots, books = _books(tmp_path, config)
    with serve(node) as url:
        _parity(chain, node, roots, books, _environment(tmp_path, url))
        node.wallets.clear()
        node.loaded.clear()
        again = _parity(chain, node, roots, books, _environment(tmp_path, url, 1))
    row = again["data"][0]
    assert (row["core_wallet_created"], row["imported_descriptors"], row["bitcoinrpc_sync_mode"]) == (
        True, 2, "full_scan"), row


def test_pruned_node_without_a_birthday_refuses_like_python(tmp_path):
    config, _ = _wallet("wpkh", 3)
    chain, _, _ = _funded_chain(config)
    node = Node(chain)
    node.pruned = True
    roots, books = _books(tmp_path, config)
    with serve(node) as url:
        expected = _parity(chain, node, roots, books, _environment(tmp_path, url))
    assert "bitcoinrpc_birthday_required" in json.dumps(expected)


def test_birthday_and_trimmed_descriptor_match_python(tmp_path):
    # Alpha owns receive script 1 and sorts first; Core still imports the
    # whole range and Python reports the trimmed targets.
    config, _ = _wallet("wpkh", 3)
    config["birthday"] = "2023-11-14"
    chain, receive, _ = _funded_chain(config)
    node = Node(chain)
    roots, books = _books(tmp_path, config)
    for book in books:
        conn = connect(book)
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES('b','w','p','Alpha','descriptor','{}','fixed')")
        conn.execute(
            "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,txid,vout,outpoint,asset,amount,chain,"
            "network,script_pubkey,confirmation_status,first_seen_at,last_seen_at) "
            "VALUES('u','w','p','b',?,0,?,'BTC',1000,'bitcoin','main',?,'confirmed','fixed','fixed')",
            ("ab" * 32, "ab" * 32 + ":0", receive[1].script_pubkey))
        conn.commit()
        conn.close()
    with serve(node) as url:
        expected = _parity(chain, node, roots, books, _environment(tmp_path, url))
    assert expected["data"][0]["status"] == "synced", expected


def test_collaborative_spend_and_foreign_parents_match_python(tmp_path):
    # A spend with a foreign input: Core's wallet cannot see that parent
    # (-5), so the graph stays incomplete and the wallet delta is booked.
    config, _ = _wallet("wpkh", 3)
    chain, receive, change = _funded_chain(config)
    foreign, _ = chain.fund(FOREIGN, 40_000)
    second, _ = chain.fund(receive[1].script_pubkey, 25_000)
    chain.spend([(second, 0), (foreign, 0)], [(FOREIGN, 60_000), (change[1].script_pubkey, 4_000)], height=18)
    node = Node(chain)
    roots, books = _books(tmp_path, config)
    with serve(node) as url:
        expected = _parity(chain, node, roots, books, _environment(tmp_path, url))
    assert expected["data"][0]["status"] == "synced", expected


def test_existing_node_wallet_is_loaded_like_python(tmp_path):
    config, _ = _wallet("wpkh", 3)
    chain, _, _ = _funded_chain(config)
    node = Node(chain)
    node.wallets["kassiber-a"] = {"scripts": set()}
    roots, books = _books(tmp_path, config)
    with serve(node) as url:
        expected = _parity(chain, node, roots, books, _environment(tmp_path, url))
    assert "core_wallet_created" not in expected["data"][0]


def test_spent_receive_raises_the_highest_used_index_like_python(tmp_path):
    # Receive script 3 is used and spent within one round: only the wallet
    # details record it, no unspent output does, and it still raises the
    # highest used index the next range derives from.
    config, _ = _wallet("wpkh", 3)
    chain, receive, _ = _funded_chain(config)
    node = Node(chain)
    roots, books = _books(tmp_path, config)
    with serve(node) as url:
        _parity(chain, node, roots, books, _environment(tmp_path, url))
        funding, _ = chain.fund(receive[3].script_pubkey, 9_000)
        chain.spend([(funding, 0)], [(FOREIGN, 8_000)], height=20)
        _parity(chain, node, roots, books, _environment(tmp_path, url, 1))
        third = _parity(chain, node, roots, books, _environment(tmp_path, url, 2))
    assert third["data"][0]["imported_descriptors"] == 1, third


def test_deficit_rescan_and_the_widened_range_match_python(tmp_path):
    # A manual outbound an empty chain cannot cover leaves a deficit: Python
    # re-syncs at the widened gap, which imports a far larger range. The
    # next round derives its targets from that stored range, not the gap.
    config, _ = _wallet("wpkh", 3)
    chain = Chain()
    node = Node(chain)
    roots, books = _books(tmp_path, config)
    for book in books:
        conn = connect(book)
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,raw_json,created_at) VALUES('manual','w','p','a','manual-1','f',"
            "'2023-01-01T00:00:00Z','outbound','BTC',100000000,0,'{}','fixed')")
        conn.commit()
        conn.close()
    with serve(node) as url:
        first = _parity(chain, node, roots, books, _environment(tmp_path, url))
        rescan = first["data"][0].get("negative_balance_rescan") or {}
        assert (rescan.get("triggered"), rescan.get("resolved")) == (True, False), first
        # Alpha sorts first and holds the whole widened range, far more than
        # the gap horizon, and this wallet owns nothing: the next round skips
        # it over all of that range.
        owned = _targets(config, 500)
        for book in books:
            conn = connect(book)
            # The deficit is gone, so the next round is an ordinary one.
            conn.execute("DELETE FROM transactions WHERE id='manual'")
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES('b','w','p','Alpha','descriptor','{}','fixed')")
            conn.executemany(
                "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,txid,vout,outpoint,asset,amount,"
                "chain,network,script_pubkey,confirmation_status,first_seen_at,last_seen_at) "
                "VALUES(?,'w','p','b',?,?,?,'BTC',1000,'bitcoin','main',?,'confirmed','fixed','fixed')",
                [(f"u{n}", "ab" * 32, n, f"{'ab' * 32}:{n}", target.script_pubkey)
                 for n, target in enumerate(owned)])
            conn.commit()
            conn.close()
        second = _parity(chain, node, roots, books, _environment(tmp_path, url, 1))
    row = second["data"][0]
    assert (row["status"], row["filtered_overlap_targets"]) == ("skipped", 1000), row


def _htlc_script(preimage):
    import hashlib

    digest = hashlib.new("ripemd160", hashlib.sha256(preimage).digest()).digest()
    return (bytes([0xa9, 20]) + digest + bytes([0x87, 0x63, 33]) + b"\x02" * 33
            + bytes([0x67, 3, 0x10, 0x27, 0x00, 0xb1, 0x75, 33]) + b"\x03" * 33 + bytes([0x68, 0xac]))


def test_htlc_claim_and_refund_match_python(tmp_path):
    # Single-input HTLC claim and refund spends paying the wallet: Core's
    # curated graph carries the attestation and the records the links.
    from embit.script import Script, Witness
    from embit.transaction import Transaction, TransactionInput, TransactionOutput

    config, _ = _wallet("wpkh", 3)
    chain, receive, change = _funded_chain(config)
    claim_parent, _ = chain.fund(FOREIGN, 21_000)
    refund_parent, _ = chain.fund(FOREIGN, 16_000)
    preimage = b"\x11" * 32
    for parent, witness, script, value, height in [
        (claim_parent, [b"\x30" * 71, preimage, b"\x01", _htlc_script(preimage)], receive[1].script_pubkey,
         20_000, 18),
        (refund_parent, [b"\x30" * 71, b"", _htlc_script(b"\x22" * 32)], change[2].script_pubkey, 15_000, 19),
    ]:
        tx = Transaction(vin=[TransactionInput(bytes.fromhex(parent), 0, witness=Witness(witness))],
                         vout=[TransactionOutput(value, Script(bytes.fromhex(script)))])
        chain.add(tx, height=height)
    node = Node(chain)
    roots, books = _books(tmp_path, config)
    with serve(node) as url:
        expected = _parity(chain, node, roots, books, _environment(tmp_path, url))
    assert expected["data"][0]["status"] == "synced", expected
    conn = connect(books[0])
    rows = conn.execute("SELECT payment_hash, swap_refund_funding_txid FROM transactions "
                        "WHERE payment_hash IS NOT NULL OR swap_refund_funding_txid IS NOT NULL").fetchall()
    conn.close()

    assert len(rows) == 2
