"""Build legacy-review books the captured test books do not cover.

Inputs for `gen_legacy_migration_vectors.py`, frozen with the Python of the
legacy-review era's storage (rows `transaction_pairs`/`direct_swap_payouts`
as v0.21.0 to v0.22.55 wrote them, without components) on the current
schema. Each book also lacks the `custody-legacy-pair-fee-review-v1` audit,
as every book from before that migration does, so one open runs the
consolidation and the fee review together.

    python -m tests.rust_port.build_legacy_scenarios OUT_DIR

Writes `OUT_DIR/<scenario>.sqlite3`. Synthetic data only.
"""
from __future__ import annotations

import os
from pathlib import Path
import sqlite3
import sys
import tempfile
from unittest.mock import patch

from kassiber import db
from kassiber.operator import project as ownership

NOW = "2026-01-01T00:00:00Z"
PAIR_FEE_AUDIT = "custody-legacy-pair-fee-review-v1"


def scope(conn, profile="profile", workspace="ws", wallet_prefix=""):
    if conn.execute("SELECT 1 FROM workspaces WHERE id=?", (workspace,)).fetchone() is None:
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES(?, 'Books', ?)", (workspace, NOW))
    conn.execute(
        """INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country,
               gains_algorithm, created_at) VALUES(?, ?, ?, 'EUR', 'generic', 'FIFO', ?)""",
        (profile, workspace, profile, NOW),
    )
    for wallet, chain in (("btc", "bitcoin"), ("btc2", "bitcoin"), ("liquid", "liquid")):
        conn.execute(
            """INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
               VALUES(?, ?, ?, ?, 'descriptor', ?, ?)""",
            (wallet_prefix + wallet, workspace, profile, wallet_prefix + wallet,
             f'{{"chain":"{chain}","network":"main"}}', NOW),
        )


def tx(conn, tx_id, wallet, direction, asset, amount, occurred_at, *, fee=0,
       includes_fee=0, profile="profile", workspace="ws", external_id=None):
    conn.execute(
        """INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id,
               fingerprint, occurred_at, direction, asset, amount, fee, amount_includes_fee,
               kind, raw_json, created_at)
           VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'transfer', '{}', ?)""",
        (tx_id, workspace, profile, wallet, external_id or tx_id, f"fp:{tx_id}", occurred_at,
         direction, asset, amount, fee, includes_fee, occurred_at),
    )


def pair(conn, pair_id, out_id, in_id, *, kind="manual", policy="carrying-value",
         notes=None, swap_fee=None, swap_fee_kind=None, confidence=None, source="user",
         out_amount=None, created_at=NOW, deleted_at=None, component_id=None,
         profile="profile", workspace="ws"):
    conn.execute(
        """INSERT INTO transaction_pairs(id, workspace_id, profile_id, out_transaction_id,
               in_transaction_id, kind, policy, notes, swap_fee_msat, swap_fee_kind,
               confidence_at_pair, pair_source, out_amount, component_id, deleted_at, created_at)
           VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (pair_id, workspace, profile, out_id, in_id, kind, policy, notes, swap_fee,
         swap_fee_kind, confidence, source, out_amount, component_id, deleted_at, created_at),
    )


def payout(conn, payout_id, out_id, *, policy="carrying-value", asset="LBTC", amount=1000,
           occurred_at=None, fiat=None, external_id=None, counterparty=None, notes=None,
           swap_fee=None, swap_fee_kind=None, out_amount=None, created_at=NOW,
           deleted_at=None, component_id=None, profile="profile", workspace="ws"):
    conn.execute(
        """INSERT INTO direct_swap_payouts(id, workspace_id, profile_id, out_transaction_id,
               kind, policy, payout_asset, payout_amount, payout_occurred_at, payout_fiat_value,
               payout_external_id, counterparty, notes, swap_fee_msat, swap_fee_kind, out_amount,
               component_id, deleted_at, created_at)
           VALUES(?, ?, ?, ?, 'direct-swap-payout', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (payout_id, workspace, profile, out_id, policy, asset, amount, occurred_at, fiat,
         external_id, counterparty, notes, swap_fee, swap_fee_kind, out_amount, component_id,
         deleted_at, created_at),
    )


def unfreeze(conn):
    """Older builds had no write freeze; the next open recreates it."""
    for trigger in ("trg_transaction_pairs_component_write_frozen",
                    "trg_direct_swap_payouts_component_write_frozen"):
        conn.execute(f"DROP TRIGGER IF EXISTS {trigger}")


def without_pair_fee_audit(conn):
    conn.execute("DELETE FROM schema_migration_audits WHERE migration_name=?", (PAIR_FEE_AUDIT,))


def mixed(conn):
    """Every consolidation outcome in one book with two profiles."""
    scope(conn)
    # A swap refund: the observed fee and the rest of the shortfall become
    # network-fee sinks (a legacy pair fee review candidate that validates).
    tx(conn, "refund-out", "btc", "outbound", "BTC", 5_000, "2026-01-02T00:00:00Z", fee=100)
    tx(conn, "refund-in", "btc2", "inbound", "BTC", 4_800, "2026-01-02T01:00:00Z")
    pair(conn, "refund", "refund-out", "refund-in", kind="swap-refund", notes="refund",
         confidence="strong", source="manual", created_at="2026-01-02T02:00:00Z")
    # A reviewed BTC to LBTC peg-in conversion with a service fee.
    tx(conn, "peg-out", "btc", "outbound", "BTC", 10_000, "2026-01-03T00:00:00Z", fee=50)
    tx(conn, "peg-in", "liquid", "inbound", "LBTC", 9_900, "2026-01-03T01:00:00Z")
    pair(conn, "peg", "peg-out", "peg-in", kind="peg-in", swap_fee=100, swap_fee_kind="service",
         created_at="2026-01-03T02:00:00Z")
    # A connected group mixing a conversion and a transfer is ambiguous.
    tx(conn, "amb-out", "btc", "outbound", "BTC", 20_000, "2026-01-04T00:00:00Z")
    tx(conn, "amb-in-l", "liquid", "inbound", "LBTC", 9_000, "2026-01-04T01:00:00Z")
    tx(conn, "amb-in-b", "btc2", "inbound", "BTC", 10_000, "2026-01-04T01:00:00Z")
    pair(conn, "amb-1", "amb-out", "amb-in-l", kind="chain-swap", out_amount=10_000,
         created_at="2026-01-04T02:00:00Z")
    pair(conn, "amb-2", "amb-out", "amb-in-b", out_amount=10_000,
         created_at="2026-01-04T02:00:01Z")
    # A review larger than its source.
    tx(conn, "big-out", "btc", "outbound", "BTC", 1_000, "2026-01-05T00:00:00Z")
    tx(conn, "big-in", "btc2", "inbound", "BTC", 1_000, "2026-01-05T01:00:00Z")
    pair(conn, "big", "big-out", "big-in", out_amount=2_000, created_at="2026-01-05T02:00:00Z")
    # A fee-inclusive outbound transfer.
    tx(conn, "incl-out", "btc", "outbound", "BTC", 3_030, "2026-01-06T00:00:00Z", fee=30,
       includes_fee=1)
    tx(conn, "incl-in", "btc2", "inbound", "BTC", 3_000, "2026-01-06T01:00:00Z")
    pair(conn, "incl", "incl-out", "incl-in", created_at="2026-01-06T02:00:00Z")
    # A deleted review.
    tx(conn, "ghost-out", "btc", "outbound", "BTC", 700, "2026-01-07T00:00:00Z")
    tx(conn, "ghost-in", "btc2", "inbound", "BTC", 700, "2026-01-07T01:00:00Z")
    pair(conn, "ghost", "ghost-out", "ghost-in",
         deleted_at="2026-01-07T03:00:00Z", created_at="2026-01-07T02:00:00Z")
    # Payouts: a partial taxable one typed as RFC 2822, a plain one,
    # a negative portion, and a deleted one.
    tx(conn, "po-out", "btc", "outbound", "BTC", 8_000, "2026-01-08T00:00:00Z", fee=40)
    payout(conn, "po-partial", "po-out", policy="taxable", asset="eur-token", amount=12_345,
           occurred_at="Thu, 08 Jan 2026 10:00:00 +0100", fiat=10.5, external_id="settle-1",
           counterparty="desk", notes="partial", swap_fee=-3, swap_fee_kind="provider-rebate",
           out_amount=4_000, created_at="2026-01-08T02:00:00Z")
    tx(conn, "po2-out", "btc", "outbound", "BTC", 900, "2026-01-09T00:00:00Z")
    payout(conn, "po-plain", "po2-out", amount=850,
           created_at="2026-01-09T02:00:00Z")
    tx(conn, "po3-out", "btc", "outbound", "BTC", 600, "2026-01-10T00:00:00Z")
    payout(conn, "po-negative", "po3-out", amount=500, out_amount=-5,
           fiat=0.1, created_at="2026-01-10T02:00:00Z")
    tx(conn, "po4-out", "btc", "outbound", "BTC", 400, "2026-01-11T00:00:00Z")
    payout(conn, "po-deleted", "po4-out", amount=390, fiat=1e17,
           deleted_at="2026-01-11T03:00:00Z", created_at="2026-01-11T02:00:00Z")
    # A second profile in the same book.
    scope(conn, profile="second", wallet_prefix="s-")
    tx(conn, "s-out", "s-btc", "outbound", "BTC", 2_000, "2026-02-01T00:00:00Z", fee=20,
       profile="second")
    tx(conn, "s-in", "s-btc2", "inbound", "BTC", 1_990, "2026-02-01T01:00:00Z", profile="second")
    pair(conn, "s-pair", "s-out", "s-in", out_amount=1_990, profile="second",
         created_at="2026-02-01T02:00:00Z")
    tx(conn, "s-po-out", "s-btc", "outbound", "BTC", 1_500, "2026-02-02T00:00:00Z",
       profile="second")
    payout(conn, "s-po", "s-po-out", amount=1_450, deleted_at="2026-02-02T03:00:00Z",
           profile="second", created_at="2026-02-02T02:00:00Z")
    without_pair_fee_audit(conn)


def base_rows(conn):
    scope(conn)
    tx(conn, "pair-out", "btc", "outbound", "BTC", 1_000, NOW, fee=10)
    tx(conn, "pair-in", "btc2", "inbound", "BTC", 900, "2026-01-02T00:00:00Z")
    pair(conn, "pair", "pair-out", "pair-in", kind="chain-swap", notes="reviewed pair",
         swap_fee=7, swap_fee_kind="service", confidence="strong", source="manual",
         out_amount=900)
    tx(conn, "payout-out", "btc", "outbound", "BTC", 2_000, "2026-01-03T00:00:00Z")
    payout(conn, "payout", "payout-out", amount=1_950, occurred_at="2026-01-04T00:00:00Z",
           fiat=123.45678901234567, external_id="settlement-1", counterparty="swap desk",
           notes="reviewed payout", swap_fee=-50, swap_fee_kind="provider-rebate",
           out_amount=2_000)


def staged(root: Path):
    """Rows a later sync replayed onto an already migrated book."""
    conn = db.open_db(root)
    base_rows(conn)
    tx(conn, "old-out", "btc", "outbound", "BTC", 300, "2026-01-05T00:00:00Z")
    tx(conn, "old-in", "btc2", "inbound", "BTC", 300, "2026-01-05T01:00:00Z")
    pair(conn, "old", "old-out", "old-in", created_at="2026-01-05T02:00:00Z")
    conn.commit()
    conn.close()
    conn = db.open_db(root)  # The era's migration links every row.
    # A late pair on the same outbound regroups the pair (supersede and
    # replace); an edited payout keeps its lineage (a revision); a replay
    # deletes a migrated pair under its active component.
    tx(conn, "pair-in-late", "btc2", "inbound", "BTC", 100, "2026-01-02T02:00:00Z")
    pair(conn, "pair-late", "pair-out", "pair-in-late", notes="delayed signed replay",
         source="sync", out_amount=100, created_at="2026-01-01T00:00:01Z")
    unfreeze(conn)
    conn.execute("UPDATE direct_swap_payouts SET notes='edited payout', component_id=NULL")
    conn.execute("UPDATE transaction_pairs SET deleted_at='2026-01-06T00:00:00Z' WHERE id='old'")
    without_pair_fee_audit(conn)
    conn.commit()
    conn.close()


def relink(root: Path):
    """Migrated rows whose link a peer's replay cleared: the pair's exact
    component is active (relinked as is); the payout's is revised."""
    conn = db.open_db(root)
    base_rows(conn)
    conn.commit()
    conn.close()
    conn = db.open_db(root)
    unfreeze(conn)
    conn.execute("UPDATE transaction_pairs SET component_id=NULL")
    conn.execute("UPDATE direct_swap_payouts SET component_id=NULL")
    without_pair_fee_audit(conn)
    conn.commit()
    conn.close()


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="legacy-scenarios-") as scratch:
        scratch = Path(scratch)
        home, locks = scratch / "home", scratch / "owners"
        for directory in (home, locks):
            directory.mkdir(mode=0o700)
        environment = {"HOME": str(home), "XDG_DATA_HOME": str(home / "xdg"),
                       "KASSIBER_NO_EGRESS": "1"}
        with patch.dict(os.environ, environment), \
                patch.object(ownership, "_owner_lock_root", return_value=locks):
            for name, build in (("mixed", None), ("staged", staged), ("relink", relink)):
                root = scratch / name
                root.mkdir()
                if build is None:
                    conn = db.open_db(root)
                    mixed(conn)
                    conn.commit()
                    conn.close()
                else:
                    build(root)
                source = sqlite3.connect(root / "kassiber.sqlite3")
                target = sqlite3.connect(out / f"scenario-{name}.sqlite3")
                try:
                    source.backup(target)
                finally:
                    target.close()
                    source.close()
                print(name)


if __name__ == "__main__":
    main()
