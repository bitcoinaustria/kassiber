"""Python's compatibility Electrum observer, natively (gap E1).

Python routes a Bitcoin Electrum wallet to its compatibility observer for an
address list, a proxy or `.onion` endpoint, a custom certificate, and for
descriptors another wallet partly owns. Each case syncs copies of one book
with Python's CLI and the Rust binary over loopback fakes only (the chain,
a SOCKS5 proxy and a TLS listener with the public test CA) and compares the
envelope and every stored byte. See docs/reference/rust-native-wallet-gaps.md.
"""
import copy
import json
import os
import subprocess
import sys

import pytest

from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan
from tests.rust_sync_chain import FOREIGN, TEST_CA, Chain, serve, socks
from tests.rust_sync_parity import connect, dump_bytes
from tests.test_rust_wallet_publication import _wallet
from tests.rust_sync_parity import fixed_elapsed_ms
from tests.test_rust_wallet_sync_cli import _args, _books, _environment
from tests.test_rust_wallet_sync_gaps import _add_wallet, _address, _address_wallet

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust binary")


def _parity(chain, roots, books, env, *, all=False, force_full=False):
    """Python's CLI and the native binary agree on the envelope and on every
    stored byte; the chain is reset between the two runs."""
    before = copy.deepcopy({k: v for k, v in chain.__dict__.items() if k not in {"lock", "hits"}})
    reference = subprocess.run(
        [sys.executable, "-m", "tests.rust_sync_cli_reference",
         *_args(roots[0], all=all, force_full=force_full)],
        capture_output=True, env=env, timeout=120,
    )
    with chain.lock:
        chain.__dict__.update(before)
        chain.hits.clear()
    actual = subprocess.run([RUST_BIN, *_args(roots[1], all=all, force_full=force_full)],
                            capture_output=True, env=env, timeout=120)
    assert actual.stderr == reference.stderr == b"", (reference.stderr, actual.stderr)
    expected, result = json.loads(reference.stdout), json.loads(actual.stdout)
    assert actual.returncode == reference.returncode, (expected, result)
    if expected.get("kind") != "error":
        expected["data"], result["data"] = fixed_elapsed_ms(expected["data"]), fixed_elapsed_ms(result["data"])
    assert result == expected
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    return expected


def _funded():
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    txid, _ = chain.fund(scripts[0][2], 20_000)
    chain.spend([(txid, 0)], [(scripts[1][0], 9_000), (FOREIGN, 10_000)], height=15)
    return config, scripts, chain


def _synced(expected, reason, route="compatibility", *, imported=True):
    row = expected["data"][0]
    assert row["status"] == "synced", row
    assert (row["observer_route"], row["observer_compatibility_reason"]) == (route, reason)
    assert (row["imported"] > 0) == imported, row
    return row


def test_address_list_matches_python(tmp_path):
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    chain.fund(scripts[0][0], 50_000)
    chain.fund(scripts[0][2], 20_000)
    chain.mempool.append(chain.spend([chain.fund(scripts[0][2], 7_000)], [(FOREIGN, 6_000)]))
    first, third = _address(config), _address(config, index=2)
    roots, books = _books(tmp_path, _address_wallet([first.address, third.address]))
    with serve(chain, "electrum") as url:
        env = _environment(tmp_path, "electrum", url)
        row = _synced(_parity(chain, roots, books, env), "address_list", "bitcoin_script")
        assert (row["sync_mode"], row["addresses"]) == ("addresses", f"{first.address},{third.address}")
        # The next round reuses the settled history; the mempool one is
        # dirty and read again.
        row = _synced(_parity(chain, roots, books, _environment(tmp_path, "electrum", url, 1)),
                      "address_list", "bitcoin_script", imported=False)
        assert (row["scripts_changed"], row["scripts_unchanged"]) == (1, 1)


def test_tor_proxy_matches_python(tmp_path):
    config, scripts, chain = _funded()
    roots, books = _books(tmp_path, config)
    with serve(chain, "electrum") as url:
        port = int(url.rsplit(":", 1)[1])
        with socks({"127.0.0.1": port}) as (proxy, seen):
            env = _environment(tmp_path, "electrum", url)
            env["KASSIBER_BACKEND_FIXTURE_TOR_PROXY"] = proxy
            _synced(_parity(chain, roots, books, env), "proxy_transport")
            chain.fund(scripts[0][1], 33_000)
            env.update(_environment(tmp_path, "electrum", url, 1))
            _synced(_parity(chain, roots, books, env), "proxy_transport")
    # Every connection of both implementations went through the proxy.
    assert seen and {host for host, _ in seen} == {"127.0.0.1"}


def test_onion_endpoint_resolves_inside_the_proxy_like_python(tmp_path):
    config, _scripts, chain = _funded()
    roots, books = _books(tmp_path, config)
    with serve(chain, "electrum", tls=True) as url:
        port = int(url.rsplit(":", 1)[1])
        with socks({"hidden.onion": port}) as (proxy, seen):
            env = _environment(tmp_path, "electrum", "ssl://hidden.onion:50002")
            env["KASSIBER_BACKEND_FIXTURE_TOR_PROXY"] = proxy
            env["KASSIBER_BACKEND_FIXTURE_CERTIFICATE"] = str(TEST_CA)
            _synced(_parity(chain, roots, books, env), "proxy_transport")
    assert seen and set(seen) == {("hidden.onion", 50002)}


def test_custom_certificate_matches_python(tmp_path):
    config, _scripts, chain = _funded()
    roots, books = _books(tmp_path, config)
    with serve(chain, "electrum", tls=True) as url:
        env = _environment(tmp_path, "electrum", url)
        env["KASSIBER_BACKEND_FIXTURE_CERTIFICATE"] = str(TEST_CA)
        _synced(_parity(chain, roots, books, env), "custom_ca")


def test_trimmed_descriptor_matches_python(tmp_path):
    # "Alpha" holds inventory evidence for Cold's second receive script and
    # sorts first, so it owns that script; Cold syncs the rest online.
    config, scripts, chain = _funded()
    other, _ = _wallet("tr", 3)
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
    with serve(chain, "electrum") as url:
        row = _synced(_parity(chain, roots, books, _environment(tmp_path, "electrum", url)),
                      "source_overlap_partial_descriptor")
    # Online discovery widens both branches; Alpha's script is trimmed.
    assert row["target_count"] == 9


def test_dependency_discovery_overlap_matches_python(tmp_path):
    # BDK discovers Cold's receive script 4, past the offline horizon (gap 3);
    # Alpha's inventory owns it, so the BDK updates are discarded and the
    # compatibility observer syncs the wallet.
    config, _ = _wallet("wpkh", 3)
    plan = load_descriptor_plan(config)
    deep = [t for t in derive_descriptor_targets(plan, start=0, end=6) if t.branch_index == 0]
    chain = Chain()
    chain.fund(deep[2].script_pubkey, 50_000)
    chain.fund(deep[4].script_pubkey, 40_000)
    roots, books = _books(tmp_path, config)
    _add_wallet(books, "b", "Alpha", {"chain": "bitcoin"})
    for book in books:
        conn = connect(book)
        conn.execute(
            "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,txid,vout,outpoint,asset,amount,chain,"
            "network,script_pubkey,confirmation_status,first_seen_at,last_seen_at) "
            "VALUES('u','w','p','b',?,0,?,'BTC',1000,'bitcoin','main',?,'confirmed','fixed','fixed')",
            ("ab" * 32, "ab" * 32 + ":0", deep[4].script_pubkey))
        conn.commit()
        conn.close()
    with serve(chain, "electrum") as url:
        _synced(_parity(chain, roots, books, _environment(tmp_path, "electrum", url)),
                "source_overlap_dependency_discovery")


def _stored_backend(books, url):
    """The backend as a stored row, so Python reuses the wallet's stored
    Electrum graphs (`_electrum_stored_graph_wallets`)."""
    for book in books:
        conn = connect(book)
        conn.execute("INSERT INTO backends(name,kind,url,batch_size,tor_proxy,config_json,created_at,updated_at) "
                     "VALUES('fixture','electrum',?,2,?,'{}','fixed','fixed')", (url, None))
        conn.commit()
        conn.close()


def _stored_environment(tmp_path, url, index=0):
    env = _environment(tmp_path, "electrum", url, index)
    for key in ("KASSIBER_BACKEND_FIXTURE_KIND", "KASSIBER_BACKEND_FIXTURE_URL",
                "KASSIBER_BACKEND_FIXTURE_BATCH_SIZE", "KASSIBER_DEFAULT_BACKEND"):
        env.pop(key)
    return env


@pytest.mark.parametrize("force_full", [False, True], ids=["cached", "forced"])
def test_stored_graphs_are_offered_like_python(tmp_path, force_full):
    # An address list over a stored Electrum backend: later rounds offer the
    # wallet's stored graphs to the observer unless forced. Python's decoded
    # graph carries no `txid`, which `stored_transaction` requires, so no
    # stored graph is ever reused and every round refetches them; Rust reads
    # the same cache with the same rule.
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    funding, _ = chain.fund(scripts[0][0], 50_000)
    first = _address(config)
    roots, books = _books(tmp_path, _address_wallet([first.address]))
    with serve(chain, "electrum") as url:
        _stored_backend(books, url)
        expected = _parity(chain, roots, books, _stored_environment(tmp_path, url))
        assert expected["data"][0]["transactions_fetched"] == 2
        # A reorg changes the tip, so every history is read again.
        chain.reorg(1)
        chain.spend([(funding, 0)], [(FOREIGN, 49_000)], height=16)
        expected = _parity(chain, roots, books, _stored_environment(tmp_path, url, 1), force_full=force_full)
    row = expected["data"][0]
    assert row["status"] == "synced", row
    assert row["transactions_fetched"] == 3
