"""Python's Liquid compatibility observers, natively (round 2, C5).

Python never runs LWK for an Esplora backend, an Electrum transport LWK
cannot carry (proxy, custom CA, insecure TLS, a timeout other than 30), an
unusual change descriptor or a trimmed descriptor: it decodes and unblinds
the transactions itself (`record_components_from_liquid_tx`). Python's CLI
and the native binary sync copies of one book over loopback
Liquid fakes built from the LWK vector chain, which carries every foreign
parent; envelopes and every stored row must match.
"""
import json
import os
import subprocess
import sys

import pytest

from tests.rust_liquid_chain import electrum, esplora
from tests.rust_sync_parity import connect, dump_bytes
from tests.test_rust_lwk_execution import VECTORS
from tests.test_rust_lwk_sync_service import wallet_config
from tests.rust_sync_parity import fixed_elapsed_ms
from tests.test_rust_wallet_sync_cli import _args, _books
from tests.test_rust_wallet_sync_cli import _environment as environment

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust binary")
PHASES = VECTORS["phases"]
TRANSACTIONS = VECTORS["transactions"]


def _environment(tmp_path, protocol, url, index=0, **extra):
    env = environment(tmp_path, protocol, url, index)
    for key, value in extra.items():
        env[f"KASSIBER_BACKEND_FIXTURE_{key.upper()}"] = str(value)
    return env


def _round(tmp_path, protocol, url, roots, books, *, index=0, force_full=False, **extra):
    """Python's CLI and the native binary on copies of one book: the same
    envelope (elapsed time aside) and the same stored bytes."""
    env = _environment(tmp_path, protocol, url, index, **extra)
    reference = subprocess.run(
        [sys.executable, "-m", "tests.rust_sync_cli_reference", *_args(roots[0], force_full=force_full)],
        capture_output=True, env=env, timeout=120)
    actual = subprocess.run([RUST_BIN, *_args(roots[1], force_full=force_full)],
                            capture_output=True, env=env, timeout=120)
    assert actual.stderr == reference.stderr == b"", (reference.stderr, actual.stderr)
    expected, result = json.loads(reference.stdout), json.loads(actual.stdout)
    assert actual.returncode == reference.returncode, (expected, result)
    if expected.get("kind") != "error":
        expected["data"], result["data"] = fixed_elapsed_ms(expected["data"]), fixed_elapsed_ms(result["data"])
    assert result == expected
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    return expected


SEQUENCES = {
    "fresh": ["compat_initial"],
    "restart": ["compat_initial", "compat_initial"],
    "reorg": ["compat_initial", "compat_reorg"],
    "extend": ["compat_initial", "compat_more"],
}


@pytest.mark.parametrize("sequence", sorted(SEQUENCES))
@pytest.mark.parametrize("protocol,extra,reason", [
    ("electrum", {"timeout": 45}, "custom_timeout"),
    ("esplora", {}, "http_route_policy"),
])
def test_liquid_compatibility_books_match_python(tmp_path, sequence, protocol, extra, reason):
    roots, books = _books(tmp_path, wallet_config())
    state = {"phase": None}
    serve = electrum if protocol == "electrum" else esplora
    with serve(state) as url:
        for index, phase in enumerate(SEQUENCES[sequence]):
            state["phase"] = PHASES[phase]
            outcome = _round(tmp_path, protocol, url, roots, books, index=index, **extra)
    data = outcome["data"][0]
    assert data["status"] == "synced", data
    assert (data["observer_route"], data["observer_compatibility_reason"]) == ("compatibility", reason)


@pytest.mark.parametrize("protocol,extra", [("electrum", {"timeout": 45}), ("esplora", {})])
def test_liquid_compatibility_forced_rebuild_matches_python(tmp_path, protocol, extra):
    roots, books = _books(tmp_path, wallet_config())
    state = {"phase": PHASES["compat_initial"]}
    serve = electrum if protocol == "electrum" else esplora
    with serve(state) as url:
        _round(tmp_path, protocol, url, roots, books, **extra)
        state["phase"] = PHASES["compat_reorg"]
        _round(tmp_path, protocol, url, roots, books, index=1, force_full=True, **extra)


def test_liquid_compatibility_wrong_blinding_key_fails_like_python(tmp_path):
    # An owned script blinded to another key cannot be unblinded: Python's
    # compatibility observer fails the whole sync and writes nothing.
    roots, books = _books(tmp_path, wallet_config())
    state = {"phase": PHASES["compat_wrong_key"]}
    with electrum(state) as url:
        outcome = _round(tmp_path, "electrum", url, roots, books, timeout=45)
    assert outcome["kind"] == "error" or outcome["data"][0]["status"] == "error", outcome


def _owned_by_alpha(books, script):
    for book in books:
        conn = connect(book)
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES('b','w','p','Alpha','descriptor','{}','fixed')")
        conn.execute(
            "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,txid,vout,outpoint,asset,amount,chain,"
            "network,script_pubkey,confirmation_status,first_seen_at,last_seen_at) "
            "VALUES('u','w','p','b',?,0,?,'LBTC',1000,'liquid','elementsregtest',?,'confirmed','fixed','fixed')",
            ("ab" * 32, "ab" * 32 + ":0", script))
        conn.commit()
        conn.close()


def _receive_script(config, index):
    from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan

    return next(t.script_pubkey for t in derive_descriptor_targets(load_descriptor_plan(config), start=0, end=index + 1)
                if t.branch_index == 0 and t.address_index == index)


def test_liquid_dependency_discovery_overlap_matches_python(tmp_path):
    # LWK finds receive script 3 past the offline horizon (gap 3); Alpha owns
    # it, so Python discards LWK's update for its compatibility observer.
    config = wallet_config()
    roots, books = _books(tmp_path, config)
    _owned_by_alpha(books, _receive_script(config, 3))
    state = {"phase": PHASES["compat_initial"]}
    with electrum(state) as url:
        outcome = _round(tmp_path, "electrum", url, roots, books)
    data = outcome["data"][0]
    assert (data["observer_route"], data["observer_compatibility_reason"]) == (
        "compatibility", "source_overlap_dependency_discovery")


def test_liquid_trimmed_descriptor_matches_python(tmp_path):
    # "Alpha" holds inventory evidence for receive script 1 and sorts first,
    # so LWK cannot scan the whole descriptor: Python's compatibility
    # observer syncs the rest over the default Electrum transport.
    config = wallet_config()
    roots, books = _books(tmp_path, config)
    _owned_by_alpha(books, _receive_script(config, 1))
    state = {"phase": PHASES["compat_initial"]}
    with electrum(state) as url:
        outcome = _round(tmp_path, "electrum", url, roots, books)
    data = outcome["data"][0]
    assert (data["observer_route"], data["observer_compatibility_reason"]) == (
        "compatibility", "source_overlap_partial_descriptor")
