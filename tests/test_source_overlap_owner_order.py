"""Two wallets watching one descriptor: one owner, whatever the source order.

The sync prefetch filters targets with the operation's source index and the
publish re-check (and Esplora's online re-filter) without it. Both must name
the same canonical wallet, or a payment to a shared script is imported by
neither wallet (Esplora) or the sync refuses `source_overlap_retry` on every
retry (Electrum). See tests/rust_port/repro_source_overlap_index_order.py.
"""
import json
import subprocess
import sys

import pytest

from kassiber.core import source_overlap
from kassiber.core.sync import WalletSyncState
from kassiber.core.sync_backends import _offline_descriptor_targets
from kassiber.db import open_db
from kassiber.wallet_descriptors import load_descriptor_plan
from tests.rust_sync_chain import Chain, serve
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import connect, create_book
from tests.test_rust_wallet_publication import _wallet

ACTIVITY = (
    "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
    "direction,asset,amount,fee,raw_json,created_at) VALUES('t','w','p',?,'x','f','2025-01-01T00:00:00Z',"
    "'inbound','BTC',1000,0,'{}','fixed')"
)


def _seed(conn, config, active_wallet):
    for wallet_id, label in (("a", "Cold"), ("b", "Alpha")):
        conn.execute(
            "INSERT OR IGNORE INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
            "VALUES(?,'w','p',?,'descriptor',?,'fixed')",
            (wallet_id, label, json.dumps(config)),
        )
    if active_wallet:
        conn.execute(ACTIVITY, (active_wallet,))
    conn.commit()


def _kept(conn, config, wallet_id):
    """Targets the prefetch (indexed) and the publish re-check (fresh) keep."""
    profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet_id,)).fetchone()
    plan = load_descriptor_plan(config)
    targets = _offline_descriptor_targets(plan, {})
    state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=plan, policy_asset_id="",
                            targets=targets, tracked_scripts={}, history_cache={}, checkpoint={})
    index = source_overlap.build_profile_source_index(conn, "p")
    prefetch = source_overlap.filter_sync_state_for_canonical_owner(conn, profile, wallet, state, profile_index=index)
    fresh = source_overlap.filter_sync_state_for_canonical_owner(conn, profile, wallet, state)
    publish = source_overlap.filter_sync_state_for_canonical_owner(conn, profile, wallet, prefetch)
    return len(targets), len(prefetch.targets), len(fresh.targets), len(publish.targets)


@pytest.mark.parametrize("active_wallet", ["a", "b", None])
def test_prefetch_and_publish_name_the_same_owner(tmp_path, active_wallet):
    config, _scripts = _wallet("wpkh", 3)
    conn = open_db(tmp_path / "data")
    conn.executescript(
        "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');"
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Book','EUR','2025');"
    )
    _seed(conn, config, active_wallet)
    # The wallet with activity owns the shared descriptor; with none, the
    # label order ("Alpha" first) decides.
    owner = active_wallet or "b"
    for wallet_id in ("a", "b"):
        total, prefetch, fresh, publish = _kept(conn, config, wallet_id)
        expected = total if wallet_id == owner else 0
        assert (prefetch, fresh, publish) == (expected, expected, expected), wallet_id
    conn.close()


def test_overlap_payload_ignores_source_order():
    def source(kind, count):
        return source_overlap.SourceScript(
            profile_id="p", wallet_id="a", wallet_label="Cold", wallet_kind="descriptor", chain="bitcoin",
            network="main", script_pubkey="00", source=kind, active_transaction_count=count,
        )

    other = source_overlap.SourceScript(
        profile_id="p", wallet_id="b", wallet_label="Alpha", wallet_kind="descriptor", chain="bitcoin",
        network="main", script_pubkey="00", source="descriptor_config",
    )
    key = ("p", "bitcoin", "main", "00")
    forward = [source("sync_target", 0), source("descriptor_config", 4), other]
    payloads = [source_overlap._overlap_payload(key, order) for order in (forward, forward[::-1])]
    assert [p["recommended_canonical_wallet_id"] for p in payloads] == ["a", "a"]
    assert {w["id"]: w["active_transaction_count"] for w in payloads[0]["wallets"]} == {"a": 4, "b": 0}
    # Only the evidence list keeps first-seen order.
    rows = [[{k: v for k, v in w.items() if k != "sources"} for w in p["wallets"]] for p in payloads]
    assert rows[0] == rows[1]


def _cli_book(tmp_path, config):
    root = tmp_path / "book"
    root.mkdir()
    path = root / "kassiber.sqlite3"
    create_book(path, config)
    open_db(root).close()  # stamps the database instance id outside the fixed uuid source
    conn = connect(path)
    _seed(conn, config, "a")
    conn.execute("INSERT OR REPLACE INTO settings VALUES('context_workspace','w')")
    conn.execute("INSERT OR REPLACE INTO settings VALUES('context_profile','p')")
    conn.commit()
    conn.close()
    return root, path


# The pre-fix row builder: each wallet's overlap row took its transaction count
# from whichever of its sources came first.
FIRST_SOURCE_OWNER = """
import dataclasses, sys
from kassiber.core import source_overlap
from tests import rust_sync_cli_reference
payload = source_overlap._overlap_payload
def first_source(key, sources):
    first = {}
    for source in sources:
        first.setdefault(source.wallet_id, source)
    return payload(key, [dataclasses.replace(source, active_transaction_count=first[source.wallet_id]
                                             .active_transaction_count) for source in sources])
source_overlap._overlap_payload = first_source
raise SystemExit(rust_sync_cli_reference.run())
"""


def _sync(tmp_path, root, protocol, url, *, round_index=0, force_full=False, first_source=False):
    env = child_env(tmp_path)
    owners = tmp_path / "owners"
    owners.mkdir(exist_ok=True, mode=0o700)
    env.update({
        "KASSIBER_BACKEND_FIXTURE_KIND": protocol,
        "KASSIBER_BACKEND_FIXTURE_URL": url,
        "KASSIBER_BACKEND_FIXTURE_BATCH_SIZE": "2",
        "KASSIBER_DEFAULT_BACKEND": "fixture",
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_RUST_TEST_SYNC_PREFIX": f"r{round_index}",
        "KASSIBER_RUST_TEST_SYNC_NOW": str(round_index * 1000),
        "KASSIBER_RUST_TEST_SYNC_START": "1700000000",
    })
    result = subprocess.run(
        [sys.executable, *(["-c", FIRST_SOURCE_OWNER] if first_source else ["-m", "tests.rust_sync_cli_reference"]),
         "--machine", "--data-root", str(root),
         "wallets", "sync", "--all", *(["--force-full"] if force_full else [])],
        capture_output=True, env=env, timeout=90,
    )
    assert result.stderr == b"", result.stderr
    return json.loads(result.stdout)


def _imported_txids(path, wallet_id):
    conn = connect(path)
    try:
        return {
            row["external_id"]
            for row in conn.execute("SELECT external_id FROM transactions WHERE wallet_id=? AND external_id!='x'",
                                    (wallet_id,))
        }
    finally:
        conn.close()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_shared_descriptor_payment_is_imported_by_the_owner(tmp_path, protocol):
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    txid, _vout = chain.fund(scripts[0][1], 50_000)
    root, path = _cli_book(tmp_path, config)
    with serve(chain, protocol) as url:
        result = _sync(tmp_path, root, protocol, url)
    assert result["kind"] != "error", result
    alpha, cold = result["data"]
    assert (alpha["wallet"], alpha["status"]) == ("Alpha", "skipped"), alpha
    assert (cold["wallet"], cold["status"], cold["imported"]) == ("Cold", "synced", 1), cold
    assert _imported_txids(path, "a") == {txid}
    assert _imported_txids(path, "b") == set()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_next_ordinary_sync_repairs_a_book_the_old_rule_dropped(tmp_path, protocol):
    config, scripts = _wallet("wpkh", 3)
    chain = Chain()
    txid, _vout = chain.fund(scripts[0][1], 50_000)
    root, path = _cli_book(tmp_path, config)
    with serve(chain, protocol) as url:
        before = _sync(tmp_path, root, protocol, url, first_source=True)
        alpha, cold = before["data"]
        assert alpha["status"] == "skipped", alpha
        if protocol == "electrum":
            assert (cold["status"], cold["code"]) == ("error", "source_overlap_retry"), cold
        else:
            assert (cold["status"], cold["target_count"], cold["imported"]) == ("synced", 2, 0), cold
        assert _imported_txids(path, "a") == set()
        after = _sync(tmp_path, root, protocol, url, round_index=1)
    alpha, cold = after["data"]
    assert alpha["status"] == "skipped", alpha
    assert (cold["status"], cold["imported"]) == ("synced", 1), cold
    assert _imported_txids(path, "a") == {txid}
    assert _imported_txids(path, "b") == set()
