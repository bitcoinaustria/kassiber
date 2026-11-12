"""Check the sync prefetch and the publish re-check pick one canonical owner.

    uv run --locked python -m tests.rust_port.repro_source_overlap_index_order

`filter_sync_state_for_canonical_owner` built each overlap payload's wallet
row from the first source it saw for that wallet. With the prefetch's
`ProfileSourceIndex`, a candidate wallet's stored descriptor targets (which
carry its active transaction count) come before its sync targets (count 0).
Without the index, as `sync_wallet_from_backend` re-checks before publishing,
the sync targets come first, so the same wallet counted as having no activity.

Two wallets watching one descriptor: "Cold" has transactions, "Alpha" (sorts
first) has none. The prefetch kept Cold canonical and fetched its history;
the publish re-check then named Alpha canonical and refused Cold with
`source_overlap_retry`, on every retry. On Esplora the compatibility observer
re-filtered its online targets without the index, so Cold published only the
targets past Alpha's horizon: a payment to an overlapping script was imported
by neither wallet. `_overlap_payload` now aggregates every source of a
wallet, in Python and in `crates/wallets/src/source_overlap.rs`, so both
passes keep all of Cold's targets (`tests/test_source_overlap_owner_order.py`,
`tests/test_rust_wallet_sync_gaps.py::test_shared_descriptor_has_one_owner_like_python`).
Exit 0 means fixed. Scratch stays under target/.
"""
from __future__ import annotations

import json
from pathlib import Path
import tempfile

from kassiber.core import source_overlap
from kassiber.core.sync_backends import _offline_descriptor_targets
from kassiber.core.sync import WalletSyncState
from kassiber.db import open_db
from kassiber.wallet_descriptors import load_descriptor_plan
from tests.test_source_overlap import _descriptor_config


def main() -> int:
    scratch = Path(__file__).resolve().parents[2] / "target/repro-source-overlap-index-order"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch.resolve()) as directory:
        conn = open_db(Path(directory) / "data")
        config = _descriptor_config()
        conn.executescript(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');"
            "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Book','EUR','2025');"
        )
        for wallet_id, label in (("cold", "Cold"), ("alpha", "Alpha")):
            conn.execute(
                "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                "VALUES(?,'w','p',?,'descriptor',?,'2025')",
                (wallet_id, label, json.dumps(config)),
            )
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,raw_json,created_at) VALUES('t','w','p','cold','x','f','2025-01-01T00:00:00Z',"
            "'inbound','BTC',1000,0,'{}','2025')"
        )
        profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        wallet = conn.execute("SELECT * FROM wallets WHERE id='cold'").fetchone()
        plan = load_descriptor_plan(config)
        targets = _offline_descriptor_targets(plan, {})
        state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=plan, policy_asset_id="",
                                targets=targets, tracked_scripts={}, history_cache={}, checkpoint={})
        index = source_overlap.build_profile_source_index(conn, "p")
        prefetch = source_overlap.filter_sync_state_for_canonical_owner(conn, profile, wallet, state,
                                                                        profile_index=index)
        publish = source_overlap.filter_sync_state_for_canonical_owner(conn, profile, wallet, prefetch)
        conn.close()
    print(f"prefetch keeps {len(prefetch.targets)} of {len(targets)} targets")
    print(f"publish re-check keeps {len(publish.targets)}")
    return 0 if len(prefetch.targets) == len(publish.targets) == len(targets) else 1


if __name__ == "__main__":
    raise SystemExit(main())
