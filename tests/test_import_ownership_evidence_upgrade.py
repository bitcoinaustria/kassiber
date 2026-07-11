import json
import tempfile

from kassiber.core.imports import (
    PRICE_COLUMNS,
    _find_existing_transaction,
    _transaction_merge_updates,
    normalize_import_record,
)
from kassiber.db import open_db


def _records(existing_raw, incoming_raw):
    existing = {
        "occurred_at": "2026-01-01T00:00:00Z",
        "fingerprint": "fp",
        "confirmed_at": "2026-01-01T00:00:00Z",
        "kind": "withdrawal",
        "privacy_boundary": None,
        "description": "move",
        "counterparty": None,
        "payment_hash": None,
        "payment_hash_source": None,
        "swap_refund_funding_txid": None,
        "swap_refund_funding_vout": None,
        "raw_json": json.dumps(existing_raw, sort_keys=True),
    }
    normalized = {
        "occurred_at": existing["occurred_at"],
        "confirmed_at": existing["confirmed_at"],
        "kind": existing["kind"],
        "privacy_boundary": None,
        "description": existing["description"],
        "counterparty": None,
        "payment_hash": None,
        "payment_hash_source": None,
        "swap_refund_funding_txid": None,
        "swap_refund_funding_vout": None,
        "raw_json": json.dumps(incoming_raw, sort_keys=True),
    }
    for column in PRICE_COLUMNS:
        existing[column] = None
        normalized[column] = None
    return existing, normalized


def test_resync_upgrades_ownership_graph_without_an_unrelated_column_change():
    existing, normalized = _records(
        {"txid": "ab" * 32},
        {
            "txid": "ab" * 32,
            "ownership_graph_version": 1,
            "vin": [{"txid": "cd" * 32, "vout": 0, "prevout": {"value_sats": 1000}}],
            "vout": [{"n": 0, "value_sats": 900}],
        },
    )

    updates = _transaction_merge_updates(existing, normalized, "fp")

    assert updates == {"raw_json": normalized["raw_json"]}


def test_resync_does_not_replace_same_or_older_ownership_evidence_by_itself():
    existing, normalized = _records(
        {"txid": "ab" * 32, "ownership_graph_version": 2, "marker": "kept"},
        {"txid": "ab" * 32, "ownership_graph_version": 1, "marker": "old"},
    )

    assert _transaction_merge_updates(existing, normalized, "fp") == {}


def test_resync_accepts_strict_same_version_ownership_evidence_enrichment():
    existing, normalized = _records(
        {
            "txid": "ab" * 32,
            "ownership_graph_version": 1,
            "vout": [{"n": 0, "scriptpubkey": "0014aa"}],
        },
        {
            "txid": "ab" * 32,
            "ownership_graph_version": 1,
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": "0014aa",
                    "value_sats": 900,
                    "asset_id": "11" * 32,
                    "role": "owned",
                }
            ],
        },
    )

    updates = _transaction_merge_updates(existing, normalized, "fp")

    assert updates == {"raw_json": normalized["raw_json"]}


def test_resync_upgrades_same_claim_hash_to_unique_outpoint_evidence():
    existing, normalized = _records(
        {"txid": "ab" * 32},
        {"txid": "ab" * 32},
    )
    payment_hash = "ef" * 32
    existing["payment_hash"] = payment_hash
    existing["payment_hash_source"] = "chain_script"
    normalized["payment_hash"] = payment_hash
    normalized["payment_hash_source"] = "chain_script_unique_outpoint"

    updates = _transaction_merge_updates(existing, normalized, "fp")

    assert updates == {
        "payment_hash_source": "chain_script_unique_outpoint"
    }


def test_daily_routing_income_uses_stable_identity_and_updates_cumulative_amount():
    with tempfile.TemporaryDirectory() as root:
        conn = open_db(root)
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'ws', '2026-01-01T00:00:00Z')"
        )
        conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, created_at) "
            "VALUES('profile', 'ws', 'profile', '2026-01-01T00:00:00Z')"
        )
        conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at) "
            "VALUES('node', 'ws', 'profile', 'node', 'lnd', '{}', '2026-01-01T00:00:00Z')"
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                kind, raw_json, created_at
            ) VALUES(
                'routing', 'ws', 'profile', 'node', 'lnd:routing:2026-01-01',
                'old-fp', '2026-01-01T00:00:00Z', 'inbound', 'BTC', 1000, 0,
                'routing_income', '{}', '2026-01-01T00:00:00Z'
            )
            """
        )
        normalized = normalize_import_record(
            {
                "id": "lnd:routing:2026-01-01",
                "occurred_at": "2026-01-01T00:00:00Z",
                "confirmed_at": "2026-01-01T00:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": "0.00000002",
                "fee": 0,
                "kind": "routing_income",
                "raw_json": "{}",
            }
        )

        existing = _find_existing_transaction(conn, "node", normalized, "new-fp")
        updates = _transaction_merge_updates(existing, normalized, "new-fp")

        assert existing["id"] == "routing"
        assert updates["amount"] == 2000
        assert updates["fingerprint"] == "new-fp"
        conn.close()
