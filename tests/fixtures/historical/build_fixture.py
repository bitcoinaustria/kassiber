from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from kassiber.db import open_db


NOW = "2025-01-01T00:00:00Z"


def tx(conn, tx_id, wallet_id, external_id, occurred_at, direction, amount, fee=0):
    native_txid = hashlib.sha256(external_id.encode("utf-8")).hexdigest()
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, confirmed_at, direction, asset, amount, fee,
            fiat_currency, fiat_rate, fiat_value, kind, description, raw_json, created_at
        ) VALUES(?, 'ws', 'pf', ?, ?, ?, ?, ?, ?, 'BTC', ?, ?,
                 'EUR', 50000, 50000, 'transfer', ?, ?, ?)
        """,
        (
            tx_id,
            wallet_id,
            native_txid,
            f"fingerprint:{tx_id}",
            occurred_at,
            occurred_at,
            direction,
            amount,
            fee,
            tx_id,
            json.dumps(
                {
                    "txid": native_txid,
                    "chain": "bitcoin",
                    "network": "mainnet",
                },
                sort_keys=True,
            ),
            occurred_at,
        ),
    )


def component(conn, component_id, note):
    conn.execute(
        """
        INSERT INTO custody_components(
            id, lineage_id, workspace_id, profile_id, revision,
            component_type, conservation_mode, state, evidence_kind,
            evidence_grade, evidence_json, expected_leg_count,
            expected_allocation_count, authored_source, notes, activated_at,
            created_at
        ) VALUES(?, 'replicated-lineage', 'ws', 'pf', 1,
                 'missing-wallet-bridge', 'quantity', 'active', 'manual-review',
                 'reviewed', '{}', 2, 1, 'replication', ?, ?, ?)
        """,
        (component_id, note, NOW, NOW),
    )
    source_leg = f"{component_id}:source"
    sink_leg = f"{component_id}:sink"
    conn.execute(
        """
        INSERT INTO custody_component_legs(
            id, component_id, workspace_id, profile_id, ordinal, role,
            rail, chain, network, asset, exposure, conservation_unit,
            amount_msat, occurred_at, transaction_id, anchor_transaction_id,
            wallet_id, location_ref, created_at
        ) VALUES(?, ?, 'ws', 'pf', 0, 'source', 'bitcoin',
                 'bitcoin', 'mainnet', 'BTC', 'BTC', 'msat', 50000000000,
                 '2022-01-01T00:00:00Z', 'component-out', 'component-out',
                 'wallet-a', 'wallet-a', ?)
        """,
        (source_leg, component_id, NOW),
    )
    conn.execute(
        """
        INSERT INTO custody_component_legs(
            id, component_id, workspace_id, profile_id, ordinal, role,
            rail, chain, network, asset, exposure, conservation_unit,
            amount_msat, occurred_at, transaction_id, anchor_transaction_id,
            wallet_id, location_ref, created_at
        ) VALUES(?, ?, 'ws', 'pf', 1, 'destination', 'bitcoin',
                 'bitcoin', 'mainnet', 'BTC', 'BTC', 'msat', 50000000000,
                 '2022-01-02T00:00:00Z', 'component-in', 'component-in',
                 'wallet-c', 'wallet-c', ?)
        """,
        (sink_leg, component_id, NOW),
    )
    conn.execute(
        """
        INSERT INTO custody_component_allocations(
            id, component_id, workspace_id, profile_id, ordinal,
            source_leg_id, sink_leg_id, source_amount_msat,
            sink_amount_msat, created_at
        ) VALUES(?, ?, 'ws', 'pf', 0, ?, ?, 50000000000, 50000000000, ?)
        """,
        (f"{component_id}:allocation", component_id, source_leg, sink_leg, NOW),
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("data_root")
    parser.add_argument("schema_ref")
    args = parser.parse_args()
    data_root = Path(args.data_root)
    if data_root.exists():
        raise SystemExit(f"refusing to overwrite existing fixture root: {data_root}")
    conn = open_db(data_root)
    conn.executescript(
        """
        INSERT INTO settings(key, value) VALUES('historical_fixture_ref', 'placeholder');
        INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Historical', '2020-01-01T00:00:00Z');
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            tax_long_term_days, gains_algorithm, journal_input_version,
            last_processed_input_version, last_processed_at,
            last_processed_tx_count, created_at
        ) VALUES('pf', 'ws', 'Treasury', 'EUR', 'AT', 365, 'FIFO', 7, 7,
                 '2025-01-01T00:00:00Z', 8, '2020-01-01T00:00:00Z');
        INSERT INTO accounts(
            id, workspace_id, profile_id, code, label, account_type, asset, created_at
        ) VALUES('acct', 'ws', 'pf', 'treasury', 'Treasury', 'asset', 'BTC', '2020-01-01T00:00:00Z');
        """
    )
    conn.execute(
        "UPDATE settings SET value = ? WHERE key = 'historical_fixture_ref'",
        (args.schema_ref,),
    )
    wallets = [
        (
            "wallet-a",
            "Old Multisig",
            "descriptor",
            {
                "chain": "bitcoin",
                "network": "mainnet",
                "descriptor": (
                    "wsh(sortedmulti(2,"
                    "[deadbeef/48h/0h/0h/2h]xpub-old/0/*,"
                    "[cafebabe/48h/0h/0h/2h]xpub-old-2/0/*))"
                ),
            },
        ),
        (
            "wallet-c",
            "New Operative Wallet (descriptor missing)",
            "descriptor",
            {"chain": "bitcoin", "network": "mainnet", "gap_limit": 100},
        ),
        (
            "sam-parent",
            "Samourai Whirlpool",
            "samourai",
            {
                "chain": "bitcoin",
                "network": "mainnet",
                "gap_limit": 100,
                "samourai": {
                    "role": "parent",
                    "group_id": "whirlpool-group",
                    "watch_only": True,
                },
            },
        ),
        (
            "sam-postmix",
            "Samourai Whirlpool - Postmix",
            "descriptor",
            {
                "chain": "bitcoin",
                "network": "mainnet",
                "gap_limit": 100,
                "samourai": {
                    "role": "child",
                    "group_id": "whirlpool-group",
                    "parent_wallet_id": "sam-parent",
                    "section": "postmix",
                    "privacy_boundary": "coinjoin",
                    "whirlpool": True,
                    "watch_only": True,
                },
            },
        ),
    ]
    for wallet_id, label, kind, config in wallets:
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
            ) VALUES(?, 'ws', 'pf', 'acct', ?, ?, ?, '2020-01-01T00:00:00Z')
            """,
            (wallet_id, label, kind, json.dumps(config, sort_keys=True)),
        )

    tx(
        conn,
        "manual-out",
        "wallet-a",
        "manual-txid",
        "2021-01-01T00:00:00Z",
        "outbound",
        100_000_000_000,
        1_000_000,
    )
    tx(
        conn,
        "manual-in",
        "wallet-c",
        "manual-txid",
        "2021-01-01T00:00:00Z",
        "inbound",
        100_000_000_000,
    )
    tx(
        conn,
        "component-out",
        "wallet-a",
        "component-out-txid",
        "2022-01-01T00:00:00Z",
        "outbound",
        50_000_000_000,
    )
    tx(
        conn,
        "component-in",
        "wallet-c",
        "component-in-txid",
        "2022-01-02T00:00:00Z",
        "inbound",
        50_000_000_000,
    )
    tx(
        conn,
        "whirlpool-out",
        "wallet-a",
        "whirlpool-out-txid",
        "2023-01-01T00:00:00Z",
        "outbound",
        1_000_000_000_000,
        10_000_000,
    )
    tx(
        conn,
        "whirlpool-return",
        "wallet-c",
        "whirlpool-return-txid",
        "2024-01-01T00:00:00Z",
        "inbound",
        990_000_000_000,
    )
    tx(
        conn,
        "postmix-known",
        "sam-postmix",
        "known-postmix-txid",
        "2023-06-01T00:00:00Z",
        "inbound",
        5_000_000_000,
    )

    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, pair_source, out_amount, created_at
        ) VALUES('manual-pair', 'ws', 'pf', 'manual-out', 'manual-in',
                 'manual', 'carrying-value', 'Reviewed wallet roll', 'user',
                 100000000000, '2021-01-02T00:00:00Z')
        """
    )
    component(conn, "component-replica-a", "Replica A interpretation")
    component(conn, "component-replica-b", "Replica B interpretation")

    for seq, component_id in enumerate(("component-replica-a", "component-replica-b"), 1):
        conn.execute(
            """
            INSERT INTO sync_events(
                id, workspace_id, profile_id, replica_id, replica_seq, hlc,
                author_member_id, event_type, entity_table, entity_key,
                payload_json, context_json, previous_hash, event_hash,
                signature, created_at, applied_at
            ) VALUES(?, 'ws', 'pf', ?, ?, ?, ?, 'upsert', 'custody_components', ?,
                     ?, '{}', NULL, ?, ?, ?, ?)
            """,
            (
                f"sync-event-{seq}",
                f"replica-{seq}",
                seq,
                f"2025-01-01T00:00:0{seq}Z-0000-replica-{seq}",
                f"member-{seq}",
                component_id,
                json.dumps(
                    {
                        "id": component_id,
                        "lineage_id": "replicated-lineage",
                        "state": "active",
                    },
                    sort_keys=True,
                ),
                f"event-hash-{seq}",
                f"signature-{seq}",
                NOW,
                NOW,
            ),
        )
    conn.execute(
        """
        INSERT INTO sync_conflicts(
            id, workspace_id, profile_id, entity_table, entity_key, field,
            local_event_id, remote_event_id, local_value_json,
            remote_value_json, status, created_at
        ) VALUES('custody-conflict', 'ws', 'pf', 'custody_components',
                 'replicated-lineage', 'state', 'sync-event-1', 'sync-event-2',
                 '"component-replica-a"', '"component-replica-b"', 'open', ?)
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, transaction_id, wallet_id,
            occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
            cost_basis, proceeds, gain_loss, description, created_at
        ) VALUES('legacy-journal', 'ws', 'pf', 'manual-out', 'wallet-a',
                 '2021-01-01T00:00:00Z', 'move', 'BTC', 100000000000,
                 50000, 50000, 50000, 0, 0, 'Historical processed journal', ?)
        """,
        (NOW,),
    )
    conn.execute(
        """
        INSERT INTO journal_tax_summary(
            id, workspace_id, profile_id, year, asset, transaction_type,
            capital_gains_type, quantity, proceeds, cost_basis, gain_loss, created_at
        ) VALUES('legacy-tax-summary', 'ws', 'pf', 2021, 'BTC', 'move', NULL,
                 100000000000, 0, 50000, 0, ?)
        """,
        (NOW,),
    )
    conn.commit()
    result = conn.execute("PRAGMA integrity_check").fetchone()[0]
    if result != "ok":
        raise RuntimeError(result)
    print(conn.execute("SELECT file FROM pragma_database_list WHERE name = 'main'").fetchone()[0])
    conn.close()


if __name__ == "__main__":
    main()
