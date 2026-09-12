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


def test_normalization_persists_explicit_native_txid_type_without_raw_payload():
    txid = "ab" * 32

    normalized = normalize_import_record(
        {
            "txid": txid,
            "occurred_at": "2026-01-01T00:00:00Z",
            "direction": "outbound",
            "amount": "1",
            "raw_json": "{}",
        }
    )

    assert normalized["external_id"] == txid
    assert normalized["external_id_kind"] == "txid"


def test_normalization_does_not_type_bare_64_hex_provider_id_as_txid():
    provider_id = "cd" * 32

    normalized = normalize_import_record(
        {
            "id": provider_id,
            "occurred_at": "2026-01-01T00:00:00Z",
            "direction": "outbound",
            "amount": "1",
            "raw_json": "{}",
        }
    )

    assert normalized["external_id"] == provider_id
    assert normalized["external_id_kind"] is None


def test_resync_preserves_existing_native_txid_type_without_spurious_update():
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
            "VALUES('wallet', 'ws', 'profile', 'wallet', 'address', '{}', '2026-01-01T00:00:00Z')"
        )
        txid = "ab" * 32
        normalized = normalize_import_record(
            {
                "txid": txid,
                "occurred_at": "2026-01-01T00:00:00Z",
                "confirmed_at": "2026-01-01T00:00:00Z",
                "direction": "inbound",
                "amount": "0.00000001",
                "raw_json": "{}",
            }
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint, occurred_at, confirmed_at,
                direction, asset, amount, fee, raw_json, created_at
            ) VALUES(
                'tx', 'ws', 'profile', 'wallet', ?, 'txid', 'fp',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                'inbound', 'BTC', 1000, 0, '{}', '2026-01-01T00:00:00Z'
            )
            """,
            (txid,),
        )

        existing = _find_existing_transaction(conn, "wallet", normalized, "fp")
        updates = _transaction_merge_updates(existing, normalized, "fp")

        assert existing["external_id_kind"] == "txid"
        assert updates == {}
        conn.close()


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


_NATIVE_GRAPH = {
    "txid": "ab" * 32,
    "observer": "bdk",
    "chain": "bitcoin",
    "vin": [{"txid": "cd" * 32, "vout": 0, "prevout": {"value_sats": 1000, "role": "owned"}}],
    "vout": [{"n": 0, "value_sats": 900, "role": "owned"}],
}
# What a generic ledger CSV row looks like after normalization: no observed legs.
_SUPPORTING_ROW = {"txid": "ab" * 32, "source": "file:generic-ledger", "memo": "cold storage"}


def _closed_provenance(existing):
    """Give the row the commitment an authoritative observer apply persists."""
    from kassiber.core.chain_observer.provenance import (
        AUTHORITY_VERSION,
        canonical_graph_hash,
        canonical_observed_quantity_hash,
    )

    row = dict(existing)
    row.setdefault("wallet_id", "wallet")
    row.setdefault("external_id", "ab" * 32)
    row.setdefault("direction", "outbound")
    row.setdefault("asset", "BTC")
    row.setdefault("amount", 900_000)
    row.setdefault("fee", 100_000)
    row.setdefault("amount_includes_fee", 0)
    row["observation_authority_version"] = AUTHORITY_VERSION
    row["observation_graph_hash"] = canonical_graph_hash(row["raw_json"])
    row["observation_quantity_hash"] = canonical_observed_quantity_hash(row)
    return row


def test_supporting_price_enrichment_keeps_native_graph_payload():
    """A matched CSV price row may price the transaction; it may not rewrite its evidence."""
    existing, normalized = _records(_NATIVE_GRAPH, _SUPPORTING_ROW)
    normalized["pricing_source_kind"] = "fmv_provider"
    normalized["fiat_rate"] = 41000

    updates = _transaction_merge_updates(_closed_provenance(existing), normalized, "fp")

    assert updates["fiat_rate"] == 41000
    assert all(column in updates for column in PRICE_COLUMNS)
    assert "raw_json" not in updates


def test_supporting_metadata_enrichment_keeps_native_graph_payload():
    existing, normalized = _records(_NATIVE_GRAPH, _SUPPORTING_ROW)
    existing["counterparty"] = None
    normalized["counterparty"] = "Acme GmbH"

    updates = _transaction_merge_updates(_closed_provenance(existing), normalized, "fp")

    assert updates["counterparty"] == "Acme GmbH"
    assert "raw_json" not in updates


def test_closed_provenance_outranks_a_graph_shaped_supporting_payload():
    """Payload contents must never beat the commitment the sync boundary persisted.

    `ownership_graph_version` lives inside user-supplied JSON, so a CSV row can
    carry it. It must not become a key to the native payload.
    """
    existing, normalized = _records(_NATIVE_GRAPH, {**_SUPPORTING_ROW, "ownership_graph_version": 99})
    normalized["counterparty"] = "Acme GmbH"

    updates = _transaction_merge_updates(_closed_provenance(existing), normalized, "fp")

    assert updates["counterparty"] == "Acme GmbH"
    assert "raw_json" not in updates


def test_authoritative_refresh_still_replaces_native_graph_payload():
    """Native evidence is preserved, not frozen: authority may still correct it."""
    corrected = {**_NATIVE_GRAPH, "vout": [{"n": 0, "value_sats": 880, "role": "owned"}]}
    for flag in ("authoritative_chain_observer", "authoritative_settlement"):
        existing, normalized = _records(_NATIVE_GRAPH, corrected)
        # The tail rule needs at least one other update, so carry a real price change.
        normalized["pricing_source_kind"] = "fmv_provider"
        normalized["fiat_rate"] = 41000
        for key, value in (
            ("external_id", "ab" * 32), ("external_id_kind", "txid"),
            ("amount", "0.000009"), ("fee", "0.000001"), ("amount_includes_fee", False),
        ):
            normalized[key] = value
        row = _closed_provenance(existing)
        row["external_id_kind"] = "txid"

        updates = _transaction_merge_updates(row, normalized, "fp", **{flag: True})

        assert updates["raw_json"] == normalized["raw_json"], flag


def test_pre_provenance_row_without_legs_still_accepts_a_supporting_payload():
    """Unchanged behaviour for rows that carry no observation to protect."""
    existing, normalized = _records({"txid": "ab" * 32, "source": "wasabi_gethistory"}, _SUPPORTING_ROW)
    normalized["counterparty"] = "Acme GmbH"

    updates = _transaction_merge_updates(existing, normalized, "fp")

    assert updates["raw_json"] == normalized["raw_json"]
