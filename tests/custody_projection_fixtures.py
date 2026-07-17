"""Authored-component fixtures for tests of stored custody readers."""

from __future__ import annotations

import hashlib
import sqlite3

from kassiber.core.custody_evidence import (
    QuantityObservation,
    canonical_event_key,
    enriched_quantity_rows,
)
from kassiber.core.custody_quantity import QuantityDomain


def insert_reviewed_projection(
    conn: sqlite3.Connection,
    *,
    projection_id: str,
    workspace_id: str,
    profile_id: str,
    source_transaction_id: str,
    target_transaction_id: str,
    source_asset: str,
    target_asset: str,
    source_amount_msat: int,
    target_amount_msat: int,
    review_kind: str,
    policy: str = "carrying-value",
    swap_fee_msat: int | None = None,
    swap_fee_kind: str | None = None,
    notes: str | None = None,
    occurred_at: str,
    target_occurred_at: str,
    relation_kind: str = "conversion",
) -> str:
    """Insert one authored review plus its minimal rebuilt projection row."""

    component_id = hashlib.sha256(
        f"fixture-component:{projection_id}".encode()
    ).hexdigest()
    source_leg_id = hashlib.sha256(
        f"fixture-source:{projection_id}".encode()
    ).hexdigest()
    target_leg_id = hashlib.sha256(
        f"fixture-target:{projection_id}".encode()
    ).hexdigest()
    term_id = hashlib.sha256(f"fixture-term:{projection_id}".encode()).hexdigest()
    allocation_id = hashlib.sha256(
        f"fixture-allocation:{projection_id}".encode()
    ).hexdigest()
    source_row_hash = hashlib.sha256(
        f"fixture-row:{projection_id}".encode()
    ).hexdigest()
    is_move = relation_kind == "move"
    transaction_rows = {
        str(row["id"]): dict(row)
        for row in conn.execute(
            "SELECT t.*, w.kind AS wallet_kind, w.config_json AS config_json "
            "FROM transactions t "
            "JOIN wallets w ON w.id = t.wallet_id WHERE t.id IN (?, ?)",
            (source_transaction_id, target_transaction_id),
        ).fetchall()
    }
    enriched_rows = {
        str(row["id"]): row
        for row in enriched_quantity_rows(tuple(transaction_rows.values()))
    }

    def native_domain(transaction_id: str) -> tuple[str, str, str | None]:
        row = enriched_rows[transaction_id]
        observation = QuantityObservation.from_transaction(
            row, canonical_event_key(row)
        )
        domain = QuantityDomain.from_observation(observation)
        return domain.rail, domain.network, transaction_rows[transaction_id][
            "wallet_id"
        ]

    source_rail, source_network, source_wallet_id = native_domain(
        source_transaction_id
    )
    target_rail, target_network, target_wallet_id = native_domain(
        target_transaction_id
    )
    conn.execute(
        """
        INSERT INTO custody_components(
            id, lineage_id, workspace_id, profile_id, revision,
            component_type, conservation_mode, state, conversion_policy,
            conversion_reviewed, expected_leg_count,
            expected_allocation_count, expected_economic_term_count,
            expected_evidence_count, authored_source, notes, activated_at,
            created_at
        ) VALUES(?, ?, ?, ?, 1, ?, ?, 'draft', ?, 1, 2, 1, 1, NULL,
                 'test_fixture', ?, NULL, ?)
        """,
        (
            component_id,
            component_id,
            workspace_id,
            profile_id,
            review_kind,
            "quantity" if is_move else "conversion",
            policy,
            notes,
            occurred_at,
        ),
    )
    conn.executemany(
        """
        INSERT INTO custody_component_legs(
            id, component_id, workspace_id, profile_id, ordinal, role,
            rail, chain, network, asset, exposure, conservation_unit,
            amount_msat, valuation_unit, valuation_amount, occurred_at,
            transaction_id, anchor_transaction_id, wallet_id, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'bitcoin', 'msat', ?, ?, ?,
                 ?, ?, ?, ?, ?)
        """,
        (
            (
                source_leg_id,
                component_id,
                workspace_id,
                profile_id,
                0,
                "source",
                source_rail,
                "liquid" if source_rail == "liquid" else "bitcoin",
                source_network,
                source_asset,
                source_amount_msat,
                None if is_move else "fixture-value",
                None if is_move else 1,
                occurred_at,
                source_transaction_id,
                source_transaction_id,
                source_wallet_id,
                occurred_at,
            ),
            (
                target_leg_id,
                component_id,
                workspace_id,
                profile_id,
                1,
                "destination",
                target_rail,
                "liquid" if target_rail == "liquid" else "bitcoin",
                target_network,
                target_asset,
                target_amount_msat,
                None if is_move else "fixture-value",
                None if is_move else 1,
                target_occurred_at,
                target_transaction_id,
                target_transaction_id,
                target_wallet_id,
                occurred_at,
            ),
        ),
    )
    conn.execute(
        """
        INSERT INTO custody_component_allocations(
            id, component_id, workspace_id, profile_id, ordinal,
            source_leg_id, sink_leg_id, source_amount_msat,
            sink_amount_msat, created_at
        ) VALUES(?, ?, ?, ?, 0, ?, ?, ?, ?, ?)
        """,
        (
            allocation_id,
            component_id,
            workspace_id,
            profile_id,
            source_leg_id,
            target_leg_id,
            source_amount_msat,
            target_amount_msat,
            occurred_at,
        ),
    )
    from kassiber.core.custody_quantity_store import capture_component_evidence

    expected_evidence_count = capture_component_evidence(
        conn,
        {
            "id": component_id,
            "workspace_id": workspace_id,
            "profile_id": profile_id,
            "legs": (
                {
                    "anchor_transaction_id": source_transaction_id,
                    "transaction_id": source_transaction_id,
                },
                {
                    "anchor_transaction_id": target_transaction_id,
                    "transaction_id": target_transaction_id,
                },
            ),
        },
        created_at=occurred_at,
    )
    conn.execute(
        "UPDATE custody_components SET state = 'active', activated_at = ?, "
        "expected_evidence_count = ? WHERE id = ?",
        (occurred_at, expected_evidence_count, component_id),
    )
    conn.execute(
        """
        INSERT INTO custody_component_economic_terms(
            id, component_id, workspace_id, profile_id, ordinal,
            source_leg_id, target_leg_id, term_kind, legacy_source_id,
            source_row_hash, review_kind, tax_policy, swap_fee_msat,
            swap_fee_kind, review_source, review_notes, created_at
        ) VALUES(?, ?, ?, ?, 0, ?, ?, 'transaction_pair', ?, ?, ?, ?, ?, ?,
                 'test_fixture', ?, ?)
        """,
        (
            term_id,
            component_id,
            workspace_id,
            profile_id,
            source_leg_id,
            target_leg_id,
            projection_id,
            source_row_hash,
            review_kind,
            policy,
            swap_fee_msat,
            swap_fee_kind,
            notes,
            occurred_at,
        ),
    )
    if is_move:
        conn.execute(
            """
            INSERT INTO journal_custody_decisions(
                decision_id, workspace_id, profile_id,
                source_transaction_id, target_transaction_id,
                source_observation_hash, source_start_msat, source_end_msat,
                target_observation_hash, target_start_msat, target_end_msat,
                source_network, target_network, source_rail, target_rail,
                source_asset, target_asset, state, basis_state, reason,
                component_id, occurred_at, target_occurred_at, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, 0, ?, ?, 0, ?, 'unknown', 'unknown',
                     'unknown', 'unknown', ?, ?, 'internal_reviewed', 'eligible',
                     'reviewed_custody_component', ?, ?, ?, ?)
            """,
            (
                projection_id,
                workspace_id,
                profile_id,
                source_transaction_id,
                target_transaction_id,
                "1" * 64,
                source_amount_msat,
                "2" * 64,
                target_amount_msat,
                source_asset,
                target_asset,
                component_id,
                occurred_at,
                target_occurred_at,
                occurred_at,
            ),
        )
    else:
        conn.execute(
            """
            INSERT INTO journal_custody_economic_relations(
                relation_id, workspace_id, profile_id, relation_kind,
                source_transaction_id, target_transaction_id, component_id,
                source_asset, target_asset, source_amount_msat,
                target_amount_msat, basis_state, occurred_at,
                target_occurred_at, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'eligible', ?, ?, ?)
            """,
            (
                projection_id,
                workspace_id,
                profile_id,
                relation_kind,
                source_transaction_id,
                target_transaction_id,
                component_id,
                source_asset,
                target_asset,
                source_amount_msat,
                target_amount_msat,
                occurred_at,
                target_occurred_at,
                occurred_at,
            ),
        )
    return component_id
