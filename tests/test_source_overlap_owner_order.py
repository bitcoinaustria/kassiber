"""Two wallets watching one descriptor: one owner, whatever the source order.

The sync prefetch filters targets with the operation's source index and the
publish re-check (and Esplora's online re-filter) without it. Both must name
the same canonical wallet, or a payment to a shared script is imported by
neither wallet (Esplora) or the sync refuses `source_overlap_retry` on every
retry (Electrum). The end-to-end sync checks run in both runtimes and arrive
with the Rust parity harness.
"""
import json

import pytest

from kassiber.core import source_overlap
from kassiber.core.sync import WalletSyncState
from kassiber.core.sync_backends import _offline_descriptor_targets
from kassiber.db import open_db
from kassiber.wallet_descriptors import load_descriptor_plan

# The descriptor-plan vectors' single-key wpkh wallet (a public test xpub).
WPKH = (
    "wpkh([4ba43603/84h/0h/0h]xpub6CFtfy4QXsEUW5CtgE7mZe1Lvs15Yw7ctjdyaDRy89JdhtyM1wFf8uY2BdyJ3JmAFfrHdw77hEit1eb"
    "VXxB2dytGAvq9mmQJ2c83G1q8P7A/0/*)"
)

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
    config = {"descriptor": WPKH, "gap_limit": 3, "backend": "fixture"}
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
