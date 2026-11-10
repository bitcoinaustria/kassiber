#!/usr/bin/env python3
"""Freeze Python's reused-UTXO confirmation refresh (round 5, 7e8c323b9).

When an Esplora or Electrum sync reuses unchanged UTXO membership, the apply
step advances confirmation counts from the checkpoint's proven tip height.
Each case runs `refresh_wallet_output_confirmations` on an in-memory book
with outputs in and out of the wallet/backend/chain/network scope. The tip
height is chosen as `sync_wallet_from_backend` chooses it (`kind` is the
normalized backend kind). No data root or network is involved.

    mise exec rust@1.99.1 uv@latest -- uv run --locked python \\
        crates/wallets/scripts/generate_utxo_confirmation_vectors.py [--check]
"""
import json
from pathlib import Path
import sqlite3
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.core.output_inventory import refresh_wallet_output_confirmations  # noqa: E402
from kassiber.core.sync import WalletSyncState, normalize_backend_kind  # noqa: E402

OUT = ROOT / "crates/wallets/tests/vectors/python_utxo_confirmations.json"
SEED = """
INSERT INTO chain_index_clock(id,revision) VALUES(1,0);
INSERT INTO workspaces(id,label,created_at) VALUES('w','W','fixed');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','P','EUR','fixed');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES
 ('wallet','w','p','Wallet','descriptor','fixed'),('other','w','p','Other','descriptor','fixed');
"""
OUTPUTS = [
    # id, wallet, backend name, backend kind, chain, network, status, height, spent, confirmations
    ("in-scope", "wallet", "fixture", "esplora", "bitcoin", "main", "confirmed", 899990, None, 1),
    ("at-tip", "wallet", "fixture", "esplora", "bitcoin", "main", "confirmed", 900005, None, 1),
    ("above-tip", "wallet", "fixture", "esplora", "bitcoin", "main", "confirmed", 900006, None, 7),
    ("zero-height", "wallet", "fixture", "esplora", "bitcoin", "main", "confirmed", 0, None, 3),
    ("null-height", "wallet", "fixture", "esplora", "bitcoin", "main", "confirmed", None, None, None),
    ("mempool", "wallet", "fixture", "esplora", "bitcoin", "main", "mempool", 899990, None, None),
    ("spent", "wallet", "fixture", "esplora", "bitcoin", "main", "confirmed", 899990, "2026", 4),
    ("other-wallet", "other", "fixture", "esplora", "bitcoin", "main", "confirmed", 899990, None, 5),
    ("other-backend", "wallet", "second", "esplora", "bitcoin", "main", "confirmed", 899990, None, 6),
    ("other-kind", "wallet", "fixture", "electrum", "bitcoin", "main", "confirmed", 899990, None, 8),
    ("raw-kind", "wallet", "fixture", "mempool", "bitcoin", "main", "confirmed", 899990, None, 9),
    ("other-network", "wallet", "fixture", "esplora", "bitcoin", "test", "confirmed", 899990, None, 10),
    ("other-chain", "wallet", "fixture", "esplora", "liquid", "main", "confirmed", 899990, None, 11),
]


def book():
    conn = sqlite3.connect(":memory:")
    manifest = json.loads((ROOT / "crates/store/fixtures/current-schema.json").read_text())
    for kind in ("table", "index", "view", "trigger"):
        for obj in manifest["objects"]:
            if obj[0] == kind and obj[3]:
                conn.execute(obj[3])
    conn.executescript(SEED)
    for (ident, wallet, name, kind, chain, network, status, height, spent, confirmations) in OUTPUTS:
        conn.execute(
            """INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,backend_name,backend_kind,chain,network,
               asset,amount,txid,vout,outpoint,confirmation_status,confirmations,block_height,first_seen_at,last_seen_at,spent_at)
               VALUES(?,'w','p',?,?,?,?,?,'BTC',1000,?,0,?,?,?,?,'fixed','fixed',?)""",
            (ident, wallet, name, kind, chain, network, ident, ident + ":0", status, confirmations, height, spent))
    return conn


def cases():
    for kind, backend_kind in [("esplora", "esplora"), ("mempool", "mempool"), ("electrum", "electrum"),
                               ("bitcoinrpc", "bitcoinrpc"), (" Esplora ", "esplora")]:
        for label, height in [("int", 900005), ("zero", 0), ("negative", -1), ("bool", True),
                              ("float", 900005.0), ("string", "900005"), ("missing", None)]:
            checkpoint = {} if label == "missing" else {
                "esplora_tip_height": height, "electrum_tip_height": height}
            yield f"{kind.strip()}-{label}", kind, backend_kind, checkpoint


def build():
    result = []
    for name, kind, backend_kind, checkpoint in cases():
        conn = book()
        # sync_wallet_from_backend: fetch.kind is the normalized backend kind.
        fetch_kind = normalize_backend_kind(kind)
        tip_height = checkpoint.get(f"{fetch_kind}_tip_height") if fetch_kind in {"esplora", "electrum"} else None
        state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=None, policy_asset_id="",
                                targets=[], tracked_scripts={}, history_cache={})
        refresh_wallet_output_confirmations(conn, {"id": "p"}, {"id": "wallet"},
                                            {"name": "fixture", "kind": backend_kind}, state, tip_height)
        rows = conn.execute("SELECT id, confirmations FROM wallet_utxos ORDER BY id").fetchall()
        result.append(dict(name=name, fetch_kind=kind, backend_kind=backend_kind, checkpoint=checkpoint,
                           confirmations={ident: value for ident, value in rows}))
        conn.close()
    return json.dumps(dict(seed=SEED, outputs=OUTPUTS, cases=result), ensure_ascii=True, indent=1) + "\n"


if __name__ == "__main__":
    text = build()
    if "--check" in sys.argv:
        raise SystemExit(0 if OUT.read_text() == text else "UTXO confirmation vectors drifted")
    OUT.write_text(text)
    print(f"{len(json.loads(text)['cases'])} confirmation refresh cases")
