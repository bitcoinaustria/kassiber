"""Focused schema and API tests for atomic custody interpretation."""

from __future__ import annotations

import json
import sqlite3
import tempfile
import unittest

from kassiber import db as db_module
from kassiber.cli.handlers import process_journals, require_processed_journals
from kassiber.core.custody_components import (
    activate_component,
    create_component,
    get_component,
    iter_authored_active_components,
    iter_effective_components,
    list_components,
    list_effective_components,
    reconcile_active_memberships,
    supersede_component,
    undo_supersede,
    update_component,
    validate_component_plan,
    validate_conservation,
)
from kassiber.core.chain_observer.provenance import (
    persist_chain_observation_provenance,
)
from kassiber.core.sync_replication.schema_allowlist import (
    SYNC_TABLE_MAP,
    serialize_row,
    validate_wire_row,
)
from kassiber.db import open_db
from kassiber.errors import AppError


NOW = "2026-01-01T00:00:00Z"


def _scope(conn):
    conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'ws', ?)", (NOW,))
    conn.execute(
        "INSERT INTO profiles(id, workspace_id, label, created_at) VALUES('profile', 'ws', 'main', ?)",
        (NOW,),
    )
    for wallet_id, kind in (("btc", "descriptor"), ("liquid", "elements"), ("node", "cln")):
        conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
            VALUES(?, 'ws', 'profile', ?, ?, '{}', ?)
            """,
            (wallet_id, wallet_id, kind, NOW),
        )


def _tx(conn, tx_id, wallet_id, direction, asset, amount):
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
            direction, asset, amount, fee, created_at
        ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, ?, 0, ?)
        """,
        (tx_id, wallet_id, f"fp-{tx_id}", NOW, direction, asset, amount, NOW),
    )


def _leg(role, amount, *, tx=None, wallet=None, rail="bitcoin", asset="BTC", occurred_at=None):
    value = {
        "role": role,
        "rail": rail,
        "chain": "liquid" if rail == "liquid" else ("bitcoin" if rail == "bitcoin" else None),
        "network": "regtest",
        "asset": asset,
        "exposure": "bitcoin",
        "conservation_unit": "msat",
        "amount_msat": amount,
        "transaction_id": tx,
        "wallet_id": wallet,
    }
    if occurred_at is not None:
        value["occurred_at"] = occurred_at
    return value


class CustodySchemaTests(unittest.TestCase):
    def test_fresh_schema_and_legacy_compatibility_columns(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            try:
                tables = {
                    row["name"]
                    for row in conn.execute("SELECT name FROM sqlite_master WHERE type = 'table'")
                }
                self.assertIn("custody_components", tables)
                self.assertIn("custody_component_legs", tables)
                self.assertIn("custody_component_allocations", tables)
                self.assertIn("custody_component_economic_terms", tables)
                self.assertIn("custody_component_transaction_memberships", tables)
                component_columns = {
                    row["name"]
                    for row in conn.execute("PRAGMA table_info(custody_components)")
                }
                self.assertIn("expected_leg_count", component_columns)
                self.assertIn("expected_allocation_count", component_columns)
                leg_columns = {
                    row["name"] for row in conn.execute("PRAGMA table_info(custody_component_legs)")
                }
                self.assertIn("occurred_at", leg_columns)
                self.assertIn("anchor_transaction_id", leg_columns)
                self.assertIn("conservation_unit", leg_columns)
                for table in ("transaction_pairs", "direct_swap_payouts"):
                    columns = {row["name"] for row in conn.execute(f"PRAGMA table_info({table})")}
                    self.assertIn("component_id", columns)
                indexes = {
                    row["name"]
                    for row in conn.execute("SELECT name FROM sqlite_master WHERE type = 'index'")
                }
                self.assertIn("idx_transaction_pairs_component", indexes)
                self.assertIn("idx_direct_swap_payouts_component", indexes)
                component_indexes = {
                    row["name"]: bool(row["unique"])
                    for row in conn.execute("PRAGMA index_list(custody_components)")
                }
                self.assertFalse(component_indexes["idx_custody_components_lineage_active"])
                self.assertFalse(component_indexes["idx_custody_components_lineage_draft"])
                self.assertIn("idx_custody_components_lineage_revision", component_indexes)
                self.assertFalse(
                    any(
                        row["table"] == "custody_components"
                        for row in conn.execute("PRAGMA foreign_key_list(custody_components)")
                    )
                )
            finally:
                conn.close()

    def test_uncommitted_legacy_wire_header_is_not_silently_sealed_on_reopen(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            _scope(conn)
            conn.execute(
                """
                INSERT INTO custody_components(
                    id, lineage_id, workspace_id, profile_id, revision,
                    component_type, state, created_at
                ) VALUES('legacy-wire', 'legacy-wire', 'ws', 'profile', 1,
                         'native_transfer', 'draft', ?)
                """,
                (NOW,),
            )
            conn.commit()
            conn.close()

            reopened = open_db(root)
            try:
                component = get_component(reopened, "legacy-wire")
                self.assertIsNone(component["expected_leg_count"])
                self.assertIsNone(component["expected_allocation_count"])
                self.assertIn(
                    "component_content_commitment_missing",
                    {issue["code"] for issue in component["validation"]["issues"]},
                )
            finally:
                reopened.close()

    def test_open_db_migrates_pre_component_compatibility_tables(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            db_path = conn.execute("PRAGMA database_list").fetchone()["file"]
            conn.close()
            legacy = sqlite3.connect(db_path)
            try:
                legacy.executescript(
                    """
                    DROP INDEX IF EXISTS idx_transaction_pairs_component;
                    DROP INDEX IF EXISTS idx_direct_swap_payouts_component;
                    DROP TABLE transaction_pairs;
                    CREATE TABLE transaction_pairs (
                        id TEXT PRIMARY KEY,
                        workspace_id TEXT NOT NULL,
                        profile_id TEXT NOT NULL,
                        out_transaction_id TEXT NOT NULL,
                        in_transaction_id TEXT NOT NULL,
                        kind TEXT NOT NULL DEFAULT 'manual',
                        policy TEXT NOT NULL DEFAULT 'carrying-value',
                        notes TEXT,
                        swap_fee_msat INTEGER,
                        swap_fee_kind TEXT,
                        confidence_at_pair TEXT,
                        pair_source TEXT,
                        out_amount INTEGER,
                        deleted_at TEXT,
                        created_at TEXT NOT NULL
                    );
                    DROP TABLE direct_swap_payouts;
                    CREATE TABLE direct_swap_payouts (
                        id TEXT PRIMARY KEY,
                        workspace_id TEXT NOT NULL,
                        profile_id TEXT NOT NULL,
                        out_transaction_id TEXT NOT NULL,
                        kind TEXT NOT NULL DEFAULT 'direct-swap-payout',
                        policy TEXT NOT NULL DEFAULT 'carrying-value',
                        payout_asset TEXT NOT NULL,
                        payout_amount INTEGER NOT NULL,
                        payout_occurred_at TEXT,
                        payout_fiat_value REAL,
                        payout_external_id TEXT,
                        counterparty TEXT,
                        notes TEXT,
                        swap_fee_msat INTEGER,
                        swap_fee_kind TEXT,
                        out_amount INTEGER,
                        deleted_at TEXT,
                        created_at TEXT NOT NULL
                    );
                    """
                )
                legacy.commit()
            finally:
                legacy.close()
            migrated = open_db(root)
            try:
                for table in ("transaction_pairs", "direct_swap_payouts"):
                    columns = {
                        row["name"] for row in migrated.execute(f"PRAGMA table_info({table})")
                    }
                    self.assertIn("component_id", columns)
            finally:
                migrated.close()

    def test_open_db_migrates_pre_durable_anchor_active_component_evidence(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            _scope(conn)
            _tx(conn, "legacy-out", "btc", "outbound", "BTC", 100)
            _tx(conn, "legacy-in", "btc", "inbound", "BTC", 100)
            component = create_component(
                conn,
                workspace_id="ws",
                profile_id="profile",
                component_id="legacy-active",
                component_type="native_transfer",
                legs=[
                    {
                        **_leg("source", 100, tx="legacy-out", wallet="btc"),
                        "id": "legacy-source",
                    },
                    {
                        **_leg("destination", 100, tx="legacy-in", wallet="btc"),
                        "id": "legacy-sink",
                    },
                ],
                allocations=[
                    {
                        "id": "legacy-edge",
                        "source_leg_id": "legacy-source",
                        "sink_leg_id": "legacy-sink",
                        "source_amount_msat": 100,
                        "sink_amount_msat": 100,
                    }
                ],
                created_at=NOW,
            )
            activated = activate_component(conn, component["id"], activated_at=NOW)
            snapshots_before = [
                tuple(row)
                for row in conn.execute(
                    "SELECT quantity_hash, detail_hash, payload_json, created_at "
                    "FROM custody_authored_evidence_snapshots "
                    "WHERE subject_kind = 'custody_component' AND subject_id = ? "
                    "ORDER BY quantity_hash, detail_hash",
                    (component["id"],),
                )
            ]
            commitments_before = [
                tuple(row)
                for row in conn.execute(
                    "SELECT id, ordinal, quantity_hash, detail_hash, created_at "
                    "FROM custody_component_evidence_commitments "
                    "WHERE component_id = ? ORDER BY ordinal, id",
                    (component["id"],),
                )
            ]
            self.assertEqual("active", activated["state"])
            self.assertGreater(len(snapshots_before), 0)
            self.assertEqual(len(snapshots_before), len(commitments_before))
            db_path = conn.execute("PRAGMA database_list").fetchone()["file"]
            conn.commit()
            conn.close()

            # Recreate the immediately preceding custody schema: activation
            # snapshots existed, but replicated commitments, the commitment
            # header, and durable transaction anchors did not yet exist.
            legacy = sqlite3.connect(db_path)
            try:
                legacy.execute("PRAGMA foreign_keys = OFF")
                trigger_names = [
                    row[0]
                    for row in legacy.execute(
                        "SELECT name FROM sqlite_master "
                        "WHERE type = 'trigger' AND name LIKE 'trg_custody_%'"
                    )
                ]
                for trigger_name in trigger_names:
                    legacy.execute(f'DROP TRIGGER "{trigger_name}"')
                legacy.execute("DROP TABLE custody_component_evidence_commitments")
                legacy.execute(
                    "ALTER TABLE custody_components DROP COLUMN expected_evidence_count"
                )
                legacy.execute(
                    "ALTER TABLE custody_component_legs DROP COLUMN anchor_transaction_id"
                )
                legacy.commit()
            finally:
                legacy.close()

            migrated = open_db(root)
            try:
                restored = get_component(migrated, component["id"])
                self.assertEqual("active", restored["state"])
                self.assertEqual(NOW, restored["activated_at"])
                self.assertEqual(
                    len(commitments_before), restored["expected_evidence_count"]
                )
                self.assertEqual(
                    ["legacy-in", "legacy-out"],
                    sorted(leg["anchor_transaction_id"] for leg in restored["legs"]),
                )
                snapshots_after = [
                    tuple(row)
                    for row in migrated.execute(
                        "SELECT quantity_hash, detail_hash, payload_json, created_at "
                        "FROM custody_authored_evidence_snapshots "
                        "WHERE subject_kind = 'custody_component' AND subject_id = ? "
                        "ORDER BY quantity_hash, detail_hash",
                        (component["id"],),
                    )
                ]
                commitments_after = [
                    tuple(row)
                    for row in migrated.execute(
                        "SELECT id, ordinal, quantity_hash, detail_hash, created_at "
                        "FROM custody_component_evidence_commitments "
                        "WHERE component_id = ? ORDER BY ordinal, id",
                        (component["id"],),
                    )
                ]
                self.assertEqual(snapshots_before, snapshots_after)
                self.assertEqual(commitments_before, commitments_after)
            finally:
                migrated.close()

    def test_custody_role_table_rebuild_preserves_rows_fks_and_guards(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            try:
                _scope(conn)
                _tx(conn, "out", "btc", "outbound", "BTC", 100)
                _tx(conn, "in", "btc", "inbound", "BTC", 100)
                component = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="native_transfer",
                    legs=[
                        {**_leg("source", 100, tx="out", wallet="btc"), "id": "source"},
                        {**_leg("destination", 100, tx="in", wallet="btc"), "id": "sink"},
                    ],
                    allocations=[
                        {
                            "id": "edge",
                            "source_leg_id": "source",
                            "sink_leg_id": "sink",
                            "source_amount_msat": 100,
                            "sink_amount_msat": 100,
                        }
                    ],
                )
                before_leg_ids = {
                    row["id"]
                    for row in conn.execute(
                        "SELECT id FROM custody_component_legs WHERE component_id = ?",
                        (component["id"],),
                    )
                }
                before_allocation_ids = {
                    row["id"]
                    for row in conn.execute(
                        "SELECT id FROM custody_component_allocations WHERE component_id = ?",
                        (component["id"],),
                    )
                }

                db_module._rebuild_custody_leg_role_schema(conn)
                db_module._ensure_custody_leg_role_schema(conn)

                self.assertEqual(
                    before_leg_ids,
                    {
                        row["id"]
                        for row in conn.execute(
                            "SELECT id FROM custody_component_legs WHERE component_id = ?",
                            (component["id"],),
                        )
                    },
                )
                self.assertEqual(
                    before_allocation_ids,
                    {
                        row["id"]
                        for row in conn.execute(
                            "SELECT id FROM custody_component_allocations WHERE component_id = ?",
                            (component["id"],),
                        )
                    },
                )
                self.assertFalse(conn.execute("PRAGMA foreign_key_check").fetchall())
                object_names = {
                    row["name"]
                    for row in conn.execute(
                        "SELECT name FROM sqlite_master WHERE type IN ('index', 'trigger')"
                    )
                }
                self.assertTrue(
                    {
                        "idx_custody_component_legs_component",
                        "idx_custody_allocations_component",
                        "trg_custody_allocation_scope_insert",
                        "trg_custody_component_scope_insert",
                        "trg_custody_component_leg_revision_immutable",
                        "trg_custody_component_allocation_revision_immutable",
                    }
                    <= object_names
                )
                self.assertEqual(
                    component["id"], get_component(conn, component["id"])["id"]
                )
            finally:
                conn.close()

    def test_sync_projection_excludes_local_evidence_and_location_reference(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            try:
                _scope(conn)
                component = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="manual_bridge",
                    evidence_kind="manual_claim",
                    evidence_grade="reviewed",
                    evidence={"secret_anchor": "do-not-sync"},
                    legs=[
                        _leg("source", 10, rail="untracked", occurred_at=NOW),
                        {
                            **_leg("destination", 10, wallet="btc", occurred_at=NOW),
                            "location_ref": "private-channel-or-script-reference",
                        },
                    ],
                )
                component_row = conn.execute(
                    "SELECT * FROM custody_components WHERE id = ?", (component["id"],)
                ).fetchone()
                header = serialize_row(
                    SYNC_TABLE_MAP["custody_components"],
                    component_row,
                    hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                )
                self.assertNotIn("evidence_json", header)
                self.assertNotIn("conversion_metadata_json", header)
                leg_row = conn.execute(
                    "SELECT * FROM custody_component_legs WHERE component_id = ? ORDER BY ordinal LIMIT 1",
                    (component["id"],),
                ).fetchone()
                leg = serialize_row(
                    SYNC_TABLE_MAP["custody_component_legs"],
                    leg_row,
                    hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                )
                self.assertNotIn("location_ref", leg)
                self.assertEqual(NOW, leg["occurred_at"])
            finally:
                conn.close()


class CustodyComponentApiTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.conn = open_db(self.root.name)
        _scope(self.conn)
        _tx(self.conn, "out", "btc", "outbound", "BTC", 100)
        _tx(self.conn, "in-1", "btc", "inbound", "BTC", 60)
        _tx(self.conn, "in-2", "btc", "inbound", "BTC", 39)
        _tx(self.conn, "other-out", "btc", "outbound", "BTC", 60)

    def tearDown(self):
        self.conn.close()
        self.root.cleanup()

    def _balanced_component(self):
        return create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            evidence_kind="ownership_graph",
            evidence_grade="exact",
            legs=[
                _leg("source", 100, tx="out", wallet="btc"),
                _leg("destination", 60, tx="in-1", wallet="btc"),
                _leg("destination", 39, tx="in-2", wallet="btc"),
                # The same evidence transaction may anchor principal and fee;
                # membership is unique per component+transaction, not per leg.
                _leg("fee", 1, tx="out", wallet="btc"),
            ],
        )

    def test_component_listing_scans_profile_routes_once(self):
        for index in range(6):
            out_id = f"perf-out-{index}"
            in_id = f"perf-in-{index}"
            _tx(self.conn, out_id, "btc", "outbound", "BTC", 100)
            _tx(self.conn, in_id, "btc", "inbound", "BTC", 100)
            draft = create_component(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                component_type="native_transfer",
                legs=[
                    _leg("source", 100, tx=out_id, wallet="btc"),
                    _leg("destination", 100, tx=in_id, wallet="btc"),
                ],
            )
            activate_component(self.conn, draft["id"])

        statements: list[str] = []
        self.conn.set_trace_callback(statements.append)
        try:
            components = list_components(
                self.conn,
                profile_id="profile",
                state="active",
                limit=1000,
            )
        finally:
            self.conn.set_trace_callback(None)

        normalized = [" ".join(statement.lower().split()) for statement in statements]
        profile_leg_scans = [
            statement
            for statement in normalized
            if "from custody_component_legs l join custody_components c" in statement
        ]
        profile_allocation_scans = [
            statement
            for statement in normalized
            if "from custody_component_allocations a join custody_components c"
            in statement
        ]
        self.assertEqual(len(components), 6)
        self.assertEqual(len(profile_leg_scans), 1)
        self.assertEqual(len(profile_allocation_scans), 1)

    def test_effective_listing_filters_before_applying_limit(self):
        active = self._balanced_component()
        activate_component(self.conn, active["id"])
        create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=[
                _leg("source", 60, tx="other-out", wallet="btc"),
                _leg("destination", 60, tx="in-1", wallet="btc"),
            ],
            created_at="2026-01-02T00:00:00Z",
        )

        components = list_components(
            self.conn,
            profile_id="profile",
            effective_only=True,
            limit=1,
        )

        self.assertEqual([item["id"] for item in components], [active["id"]])

    def test_new_nullable_sync_fields_are_backward_compatible(self):
        transaction = self.conn.execute("SELECT * FROM transactions WHERE id = 'out'").fetchone()
        tx_payload = serialize_row(
            SYNC_TABLE_MAP["transactions"],
            transaction,
            hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        tx_payload.pop("swap_refund_funding_vout")
        validate_wire_row("transactions", tx_payload)

        component = self._balanced_component()
        component_row = self.conn.execute(
            "SELECT * FROM custody_components WHERE id = ?", (component["id"],)
        ).fetchone()
        component_payload = serialize_row(
            SYNC_TABLE_MAP["custody_components"],
            component_row,
            hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        component_payload.pop("expected_leg_count")
        component_payload.pop("expected_allocation_count")
        validate_wire_row("custody_components", component_payload)

        self.conn.execute(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id,
                in_transaction_id, component_id, created_at
            ) VALUES('compat', 'ws', 'profile', 'out', 'in-1', ?, ?)
            """,
            (component["id"], NOW),
        )
        pair = self.conn.execute("SELECT * FROM transaction_pairs WHERE id = 'compat'").fetchone()
        pair_payload = serialize_row(
            SYNC_TABLE_MAP["transaction_pairs"],
            pair,
            hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        pair_payload.pop("component_id")
        validate_wire_row("transaction_pairs", pair_payload)

        leg = self.conn.execute(
            "SELECT * FROM custody_component_legs WHERE component_id = ? LIMIT 1",
            (component["id"],),
        ).fetchone()
        leg_payload = serialize_row(
            SYNC_TABLE_MAP["custody_component_legs"],
            leg,
            hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        leg_payload.pop("anchor_transaction_id")
        validate_wire_row("custody_component_legs", leg_payload)

    def test_component_header_created_at_must_be_a_timestamp(self):
        with self.assertRaises(AppError) as caught:
            create_component(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                component_type="native_transfer",
                legs=[
                    _leg("source", 100, tx="out", wallet="btc"),
                    _leg("destination", 60, tx="in-1", wallet="btc"),
                    _leg("destination", 39, tx="in-2", wallet="btc"),
                    _leg("fee", 1, tx="out", wallet="btc"),
                ],
                created_at="sort-before-everything",
            )

        self.assertEqual(caught.exception.code, "validation")

    def test_decimal_string_quantities_cannot_exceed_sqlite_integer_range(self):
        too_large = "9223372036854775808"
        base_legs = [
            {**_leg("source", "1", wallet="btc", occurred_at=NOW), "id": "s"},
            {
                **_leg("destination", "1", wallet="btc", occurred_at=NOW),
                "id": "d",
            },
        ]
        cases = []
        amount_legs = [dict(leg) for leg in base_legs]
        amount_legs[0]["amount_msat"] = too_large
        cases.append((amount_legs, []))

        valuation_legs = [dict(leg) for leg in base_legs]
        valuation_legs[0].update(
            {"valuation_unit": "eur-cent", "valuation_amount": too_large}
        )
        cases.append((valuation_legs, []))

        cases.append(
            (
                base_legs,
                [
                    {
                        "source_leg_id": "s",
                        "sink_leg_id": "d",
                        "source_amount_msat": too_large,
                        "sink_amount_msat": "1",
                    }
                ],
            )
        )

        for legs, allocations in cases:
            with self.subTest(legs=legs, allocations=allocations):
                with self.assertRaises(AppError) as raised:
                    create_component(
                        self.conn,
                        workspace_id="ws",
                        profile_id="profile",
                        component_type="manual_bridge",
                        legs=legs,
                        allocations=allocations,
                    )
                self.assertEqual(
                    "custody_component_validation", raised.exception.code
                )

    def test_n_to_m_activation_is_atomic_and_deduplicates_membership(self):
        component = self._balanced_component()
        self.assertEqual("draft", component["effective_state"])
        activated = activate_component(self.conn, component["id"], activated_at=NOW)
        self.assertEqual("active", activated["state"])
        self.assertEqual("active", activated["effective_state"])
        memberships = self.conn.execute(
            "SELECT transaction_id FROM custody_component_transaction_memberships ORDER BY transaction_id"
        ).fetchall()
        self.assertEqual(["in-1", "in-2", "out"], [row["transaction_id"] for row in memberships])
        self.assertEqual(0, activated["validation"]["by_asset"][0]["residual_msat"])
        coverage = {
            row["transaction_id"]: row
            for row in activated["validation"]["anchors"]["transaction_coverage"]
        }
        self.assertEqual(0, coverage["out"]["reviewed_minus_raw_msat"])

    def test_authoritative_chronology_refinement_warns_without_deactivation(self):
        txid = "ab" * 32
        raw_json = json.dumps(
            {
                "txid": txid,
                "chain": "bitcoin",
                "network": "regtest",
                "observer": "bdk",
                "vin": [],
                "vout": [],
            },
            sort_keys=True,
        )
        self.conn.execute(
            """
            UPDATE transactions
            SET external_id = ?, external_id_kind = 'txid', raw_json = ?
            WHERE id = 'in-1'
            """,
            (txid, raw_json),
        )
        persist_chain_observation_provenance(
            self.conn,
            self.conn.execute(
                "SELECT * FROM profiles WHERE id = 'profile'"
            ).fetchone(),
            self.conn.execute(
                "SELECT * FROM wallets WHERE id = 'btc'"
            ).fetchone(),
            application_revision="observer-apply-1",
            chain="bitcoin",
            network="regtest",
            entries=(
                {
                    "external_id": txid,
                    "asset": "BTC",
                    "direction": "inbound",
                    "observer_ids": ["descriptor:default"],
                    "observer_kinds": ["bdk"],
                },
            ),
        )
        activated = activate_component(
            self.conn,
            self._balanced_component()["id"],
            activated_at=NOW,
        )
        self.assertEqual("active", activated["effective_state"])

        reorg_raw_json = json.dumps(
            {
                "txid": txid,
                "chain": "bitcoin",
                "network": "regtest",
                "observer": "bdk",
                "status": {
                    "confirmed": True,
                    "block_height": 201,
                    "block_hash": "cd" * 32,
                },
                "vin": [],
                "vout": [],
            },
            sort_keys=True,
        )
        self.conn.execute(
            """
            UPDATE transactions
            SET occurred_at = '2026-01-01T00:00:01Z',
                confirmed_at = '2026-01-01T00:10:00Z',
                raw_json = ?
            WHERE id = 'in-1'
            """,
            (reorg_raw_json,),
        )
        persist_chain_observation_provenance(
            self.conn,
            self.conn.execute(
                "SELECT * FROM profiles WHERE id = 'profile'"
            ).fetchone(),
            self.conn.execute(
                "SELECT * FROM wallets WHERE id = 'btc'"
            ).fetchone(),
            application_revision="observer-apply-2",
            chain="bitcoin",
            network="regtest",
            entries=(
                {
                    "external_id": txid,
                    "asset": "BTC",
                    "direction": "inbound",
                    "observer_ids": ["descriptor:default"],
                    "observer_kinds": ["bdk"],
                },
            ),
        )
        refined = get_component(self.conn, activated["id"])

        self.assertEqual("active", refined["effective_state"])
        self.assertEqual("matched", refined["evidence_status"]["status"])
        warning = next(
            item
            for item in refined["validation"]["warnings"]
            if item["code"] == "anchor_observer_chronology_changed"
        )
        self.assertTrue(warning["review_required"])
        self.assertEqual("observer-apply-2", warning["application_revision"])
        self.assertNotIn(
            "anchor_occurred_at_mismatch",
            {item["code"] for item in refined["validation"]["issues"]},
        )

        # A quantity contradiction still fails closed; the chronology warning
        # cannot bless a changed economic anchor.
        self.conn.execute(
            "UPDATE transactions SET amount = amount + 1 WHERE id = 'in-1'"
        )
        contradicted = get_component(self.conn, activated["id"])
        self.assertEqual("draft", contradicted["effective_state"])
        self.assertIn(
            "anchor_coverage_mismatch",
            {item["code"] for item in contradicted["validation"]["issues"]},
        )

    def test_unproven_anchor_chronology_change_remains_blocking(self):
        activated = activate_component(
            self.conn,
            self._balanced_component()["id"],
            activated_at=NOW,
        )
        self.conn.execute(
            "UPDATE transactions SET occurred_at = '2026-01-01T00:00:01Z' "
            "WHERE id = 'in-1'"
        )

        changed = get_component(self.conn, activated["id"])

        self.assertEqual("draft", changed["effective_state"])
        self.assertIn(
            "anchor_occurred_at_mismatch",
            {item["code"] for item in changed["validation"]["issues"]},
        )

    def test_anchor_rail_chain_and_network_contradictions_fail_closed(self):
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = 'btc'",
            ('{"chain":"bitcoin","network":"regtest"}',),
        )
        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=[
                {
                    **_leg("source", 100, tx="out", wallet="btc"),
                    "rail": "liquid",
                    "chain": "liquid",
                    "network": "liquidv1",
                },
                _leg("destination", 60, tx="in-1", wallet="btc"),
                _leg("destination", 39, tx="in-2", wallet="btc"),
                _leg("fee", 1, tx="out", wallet="btc"),
            ],
        )

        issue_codes = {
            issue["code"] for issue in component["validation"]["issues"]
        }
        self.assertTrue(
            {
                "anchor_rail_mismatch",
                "anchor_chain_mismatch",
                "anchor_network_mismatch",
            }
            <= issue_codes
        )
        with self.assertRaises(AppError) as blocked:
            activate_component(self.conn, component["id"])
        self.assertEqual("custody_component_incomplete", blocked.exception.code)

    def test_retracted_transaction_anchor_stays_durable_and_fails_closed(self):
        component = activate_component(self.conn, self._balanced_component()["id"])

        self.conn.execute("DELETE FROM transactions WHERE id = 'out'")

        reloaded = get_component(self.conn, component["id"])
        self.assertEqual("active", reloaded["state"])
        self.assertEqual("draft", reloaded["effective_state"])
        source = next(leg for leg in reloaded["legs"] if leg["role"] == "source")
        self.assertIsNone(source["transaction_id"])
        self.assertEqual("out", source["anchor_transaction_id"])
        issue_codes = {issue["code"] for issue in reloaded["validation"]["issues"]}
        self.assertIn("anchor_transaction_retracted", issue_codes)
        processed = process_journals(self.conn, "ws", "profile")
        self.assertGreaterEqual(processed["quarantined"], 1)
        quarantine_ids = {
            row["transaction_id"]
            for row in self.conn.execute(
                "SELECT transaction_id FROM journal_quarantines"
            ).fetchall()
        }
        self.assertNotIn("out", quarantine_ids)
        self.assertTrue({"in-1", "in-2"} <= quarantine_ids)

        self.assertEqual(
            [component["id"]],
            [
                item["id"]
                for item in list_components(
                    self.conn,
                    profile_id="profile",
                    transaction_id="out",
                )
            ],
        )
        self.assertEqual(
            [component["id"]],
            [
                item["id"]
                for item in iter_authored_active_components(
                    self.conn,
                    profile_id="profile",
                    transaction_id="out",
                )
            ],
        )

        # Reusing the durable id does not silently reconnect the authored leg.
        # The changed row stays claimed by the invalid active component and
        # cannot fall back to an ordinary disposal.
        _tx(self.conn, "out", "btc", "outbound", "BTC", 120)
        reimported = get_component(self.conn, component["id"])
        source = next(leg for leg in reimported["legs"] if leg["role"] == "source")
        self.assertIsNone(source["transaction_id"])
        process_journals(self.conn, "ws", "profile")
        quarantine_ids = {
            row["transaction_id"]
            for row in self.conn.execute(
                "SELECT transaction_id FROM journal_quarantines"
            ).fetchall()
        }
        self.assertIn("out", quarantine_ids)
        self.assertFalse(
            any(
                row["transaction_id"] == "out" and row["entry_type"] == "disposal"
                for row in self.conn.execute(
                    "SELECT transaction_id, entry_type FROM journal_entries"
                ).fetchall()
            )
        )

    def test_retracted_active_anchor_still_blocks_overlapping_component(self):
        first = activate_component(self.conn, self._balanced_component()["id"])
        self.conn.execute("DELETE FROM transactions WHERE id = 'out'")
        _tx(self.conn, "out", "btc", "outbound", "BTC", 100)
        _tx(self.conn, "new-in", "btc", "inbound", "BTC", 100)
        overlapping = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=[
                _leg("source", 100, tx="out", wallet="btc"),
                _leg("destination", 100, tx="new-in", wallet="btc"),
            ],
        )

        with self.assertRaises(AppError) as blocked:
            activate_component(self.conn, overlapping["id"])

        self.assertEqual("custody_component_incomplete", blocked.exception.code)
        conflict_issue = next(
            issue
            for issue in blocked.exception.details["validation"]["issues"]
            if issue["code"] == "active_transaction_membership_conflict"
        )
        self.assertEqual(
            first["id"], conflict_issue["conflicts"][0]["component_id"]
        )

    def test_all_excluded_component_anchors_still_block_reports(self):
        component = activate_component(self.conn, self._balanced_component()["id"])
        # Exclude the complete fixture so projection has no live row on which it
        # could persist a quarantine. Report readiness must still fail closed.
        self.conn.execute("UPDATE transactions SET excluded = 1")

        reloaded = get_component(self.conn, component["id"])
        self.assertEqual("active", reloaded["state"])
        self.assertEqual("draft", reloaded["effective_state"])
        self.assertIn(
            "anchor_transaction_excluded",
            {issue["code"] for issue in reloaded["validation"]["issues"]},
        )

        processed = process_journals(self.conn, "ws", "profile")
        self.assertEqual(0, processed["quarantined"])
        self.assertEqual(
            component["id"],
            processed["custody_component_blockers"][0]["component_id"],
        )
        profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()
        with self.assertRaises(AppError) as blocked:
            require_processed_journals(self.conn, profile)
        self.assertEqual("custody_component_incomplete", blocked.exception.code)
        self.assertIn(
            "anchor_transaction_excluded",
            blocked.exception.details["components"][0]["issue_codes"],
        )

    def test_header_only_active_component_blocks_cli_reports_without_fake_quarantine(self):
        self.conn.execute(
            """
            INSERT INTO custody_components(
                id, lineage_id, workspace_id, profile_id, revision,
                component_type, conservation_mode, state, evidence_json,
                conversion_reviewed, conversion_metadata_json,
                expected_leg_count, expected_allocation_count, created_at
            ) VALUES(
                'partial-header', 'partial-header', 'ws', 'profile', 1,
                'manual_bridge', 'quantity', 'active', '{}', 0, '{}', 2, 0, ?
            )
            """,
            (NOW,),
        )

        processed = process_journals(self.conn, "ws", "profile")

        self.assertNotIn(
            "custody_component_blocked",
            {
                row["reason"]
                for row in self.conn.execute(
                    "SELECT reason FROM journal_quarantines"
                ).fetchall()
            },
        )
        self.assertEqual(
            "partial-header",
            processed["custody_component_blockers"][0]["component_id"],
        )
        profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()
        with self.assertRaises(AppError) as blocked:
            require_processed_journals(self.conn, profile)
        self.assertEqual("custody_component_incomplete", blocked.exception.code)
        self.assertEqual(
            "partial-header",
            blocked.exception.details["components"][0]["component_id"],
        )

    def test_true_n_to_m_requires_and_persists_exact_allocations(self):
        _tx(self.conn, "out-2", "btc", "outbound", "BTC", 40)
        _tx(self.conn, "in-3", "btc", "inbound", "BTC", 40)
        legs = [
            {**_leg("source", 100, tx="out", wallet="btc"), "id": "s1"},
            {**_leg("source", 40, tx="out-2", wallet="btc"), "id": "s2"},
            {**_leg("destination", 60, tx="in-1", wallet="btc"), "id": "d1"},
            {**_leg("destination", 39, tx="in-2", wallet="btc"), "id": "d2"},
            {**_leg("destination", 40, tx="in-3", wallet="btc"), "id": "d3"},
            {**_leg("fee", 1, tx="out", wallet="btc"), "id": "fee"},
        ]
        no_allocations = validate_conservation(legs)
        self.assertIn("allocation_required", {issue["code"] for issue in no_allocations["issues"]})
        allocations = [
            {"source_leg_id": "s1", "sink_leg_id": "d1", "source_amount_msat": 60, "sink_amount_msat": 60},
            {"source_leg_id": "s1", "sink_leg_id": "d2", "source_amount_msat": 39, "sink_amount_msat": 39},
            {"source_leg_id": "s1", "sink_leg_id": "fee", "source_amount_msat": 1, "sink_amount_msat": 1},
            {"source_leg_id": "s2", "sink_leg_id": "d3", "source_amount_msat": 40, "sink_amount_msat": 40},
        ]
        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=legs,
            allocations=allocations,
        )
        self.assertEqual(4, len(component["allocations"]))
        self.assertTrue(component["validation"]["activatable"])
        self.assertEqual("active", activate_component(self.conn, component["id"])["effective_state"])
        revision = update_component(self.conn, component["id"], notes="copied allocation graph")
        self.assertEqual(4, len(revision["allocations"]))
        self.assertTrue(revision["validation"]["activatable"])
        revision_leg_ids = {leg["id"] for leg in revision["legs"]}
        self.assertTrue(
            all(
                edge["source_leg_id"] in revision_leg_ids
                and edge["sink_leg_id"] in revision_leg_ids
                for edge in revision["allocations"]
            )
        )

    def test_unbalanced_and_nonzero_unresolved_components_cannot_activate(self):
        version_before = self.conn.execute(
            "SELECT journal_input_version FROM profiles WHERE id = 'profile'"
        ).fetchone()[0]
        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            legs=[
                _leg("source", 100, tx="out"),
                _leg("destination", 80, tx="in-1"),
                _leg("unresolved", 10, rail="untracked", occurred_at=NOW),
            ],
        )
        with self.assertRaises(AppError) as raised:
            activate_component(self.conn, component["id"])
        self.assertEqual("custody_component_incomplete", raised.exception.code)
        issue_codes = {issue["code"] for issue in raised.exception.details["validation"]["issues"]}
        self.assertIn("unresolved_value", issue_codes)
        self.assertIn("unbalanced_quantity", issue_codes)
        self.assertEqual("draft", get_component(self.conn, component["id"])["state"])
        version_after = self.conn.execute(
            "SELECT journal_input_version FROM profiles WHERE id = 'profile'"
        ).fetchone()[0]
        self.assertEqual(version_before, version_after)

    def test_reviewed_manual_bridge_activates_exact_residual_suspense(self):
        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            evidence_kind="manual_reconstruction",
            evidence_grade="reviewed",
            legs=[
                {**_leg("source", 100, tx="out", wallet="btc"), "id": "source"},
                {**_leg("destination", 60, tx="in-1", wallet="btc"), "id": "dest-1"},
                {**_leg("destination", 39, tx="in-2", wallet="btc"), "id": "dest-2"},
                {
                    **_leg("suspense", 1, rail="untracked", occurred_at=NOW),
                    "id": "suspense",
                },
            ],
            allocations=[
                {"source_leg_id": "source", "sink_leg_id": "dest-1", "source_amount_msat": 60, "sink_amount_msat": 60},
                {"source_leg_id": "source", "sink_leg_id": "dest-2", "source_amount_msat": 39, "sink_amount_msat": 39},
                {"source_leg_id": "source", "sink_leg_id": "suspense", "source_amount_msat": 1, "sink_amount_msat": 1},
            ],
        )

        self.assertTrue(component["validation"]["activatable"])
        self.assertEqual(1, component["validation"]["suspense_msat"])
        self.assertEqual(0, component["validation"]["unresolved_msat"])
        activated = activate_component(self.conn, component["id"])
        self.assertEqual("active", activated["effective_state"])
        self.assertEqual(
            {"out", "in-1", "in-2"},
            {
                row["transaction_id"]
                for row in self.conn.execute(
                    "SELECT transaction_id FROM custody_component_transaction_memberships"
                )
            },
        )
        suspense_row = self.conn.execute(
            "SELECT * FROM custody_component_legs "
            "WHERE component_id = ? AND role = 'suspense'",
            (component["id"],),
        ).fetchone()
        suspense_payload = serialize_row(
            SYNC_TABLE_MAP["custody_component_legs"],
            suspense_row,
            hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        validate_wire_row("custody_component_legs", suspense_payload)

    def test_suspense_requires_review_allocation_scope_and_source_time(self):
        base = [
            {**_leg("source", 100, tx="out", wallet="btc"), "id": "source"},
            {**_leg("destination", 99, tx="in-1", wallet="btc"), "id": "dest"},
            {
                **_leg(
                    "suspense",
                    1,
                    rail="untracked",
                    occurred_at="2026-01-02T00:00:00Z",
                ),
                "id": "suspense",
            },
        ]
        allocations = [
            {"source_leg_id": "source", "sink_leg_id": "dest", "source_amount_msat": 99, "sink_amount_msat": 99},
            {"source_leg_id": "source", "sink_leg_id": "suspense", "source_amount_msat": 1, "sink_amount_msat": 1},
        ]
        report = validate_conservation(
            base,
            allocations=allocations,
            component_type="native_transfer",
            evidence_grade="exact",
        )
        codes = {issue["code"] for issue in report["issues"]}
        self.assertIn("custody_suspense_review_required", codes)
        self.assertIn("custody_suspense_time_mismatch", codes)

        wrong_asset = [dict(leg) for leg in base]
        wrong_asset[-1]["asset"] = "LBTC"
        report = validate_conservation(
            wrong_asset,
            allocations=allocations,
            component_type="manual_bridge",
            evidence_grade="reviewed",
        )
        self.assertIn(
            "custody_suspense_asset_mismatch",
            {issue["code"] for issue in report["issues"]},
        )

    def test_reviewed_nm_bridge_allocates_residual_to_one_exact_source_slice(self):
        _tx(self.conn, "out-2", "btc", "outbound", "BTC", 40)
        _tx(self.conn, "in-3", "btc", "inbound", "BTC", 40)
        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            evidence_kind="manual_reconstruction",
            evidence_grade="reviewed",
            legs=[
                {**_leg("source", 100, tx="out", wallet="btc"), "id": "source-1"},
                {**_leg("source", 40, tx="out-2", wallet="btc"), "id": "source-2"},
                {**_leg("destination", 60, tx="in-1", wallet="btc"), "id": "dest-1"},
                {**_leg("destination", 39, tx="in-2", wallet="btc"), "id": "dest-2"},
                {**_leg("destination", 40, tx="in-3", wallet="btc"), "id": "dest-3"},
                {**_leg("suspense", 1, rail="untracked", occurred_at=NOW), "id": "suspense"},
            ],
            allocations=[
                {"source_leg_id": "source-1", "sink_leg_id": "dest-1", "source_amount_msat": 60, "sink_amount_msat": 60},
                {"source_leg_id": "source-1", "sink_leg_id": "dest-2", "source_amount_msat": 39, "sink_amount_msat": 39},
                {"source_leg_id": "source-1", "sink_leg_id": "suspense", "source_amount_msat": 1, "sink_amount_msat": 1},
                {"source_leg_id": "source-2", "sink_leg_id": "dest-3", "source_amount_msat": 40, "sink_amount_msat": 40},
            ],
        )
        self.assertTrue(component["validation"]["activatable"])
        self.assertEqual(1, component["validation"]["suspense_msat"])
        self.assertEqual(
            "active", activate_component(self.conn, component["id"])["effective_state"]
        )

    def test_suspense_preserves_observed_fee_as_a_separate_allocation(self):
        self.conn.execute("UPDATE transactions SET fee = 1 WHERE id = 'out'")
        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            evidence_kind="manual_reconstruction",
            evidence_grade="reviewed",
            legs=[
                {**_leg("source", 101, tx="out", wallet="btc"), "id": "source"},
                {**_leg("destination", 60, tx="in-1", wallet="btc"), "id": "dest-1"},
                {**_leg("destination", 39, tx="in-2", wallet="btc"), "id": "dest-2"},
                {**_leg("suspense", 1, rail="untracked", occurred_at=NOW), "id": "suspense"},
                {**_leg("fee", 1, tx="out", wallet="btc"), "id": "fee"},
            ],
            allocations=[
                {"source_leg_id": "source", "sink_leg_id": "dest-1", "source_amount_msat": 60, "sink_amount_msat": 60},
                {"source_leg_id": "source", "sink_leg_id": "dest-2", "source_amount_msat": 39, "sink_amount_msat": 39},
                {"source_leg_id": "source", "sink_leg_id": "suspense", "source_amount_msat": 1, "sink_amount_msat": 1},
                {"source_leg_id": "source", "sink_leg_id": "fee", "source_amount_msat": 1, "sink_amount_msat": 1},
            ],
        )
        self.assertTrue(component["validation"]["activatable"])

        bad = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            evidence_grade="reviewed",
            legs=[
                {**_leg("source", 101, tx="out", wallet="btc"), "id": "bad-source"},
                {**_leg("destination", 60, tx="in-1", wallet="btc"), "id": "bad-dest-1"},
                {**_leg("destination", 39, tx="in-2", wallet="btc"), "id": "bad-dest-2"},
                {**_leg("suspense", 2, rail="untracked", occurred_at=NOW), "id": "bad-suspense"},
            ],
            allocations=[
                {"source_leg_id": "bad-source", "sink_leg_id": "bad-dest-1", "source_amount_msat": 60, "sink_amount_msat": 60},
                {"source_leg_id": "bad-source", "sink_leg_id": "bad-dest-2", "source_amount_msat": 39, "sink_amount_msat": 39},
                {"source_leg_id": "bad-source", "sink_leg_id": "bad-suspense", "source_amount_msat": 2, "sink_amount_msat": 2},
            ],
        )
        self.assertFalse(bad["validation"]["activatable"])
        self.assertIn(
            "custody_suspense_fee_coverage_mismatch",
            {issue["code"] for issue in bad["validation"]["issues"]},
        )

    def test_unknown_future_role_returns_typed_validation(self):
        report = validate_conservation(
            [
                {**_leg("source", 1, wallet="btc", occurred_at=NOW), "id": "source"},
                {**_leg("future_custody_role", 1, occurred_at=NOW), "id": "future"},
            ]
        )
        self.assertFalse(report["activatable"])
        self.assertEqual(
            "custody_component_leg_role_unknown", report["issues"][0]["code"]
        )

        component = self._balanced_component()
        row = self.conn.execute(
            "SELECT * FROM custody_component_legs WHERE component_id = ? "
            "ORDER BY ordinal LIMIT 1",
            (component["id"],),
        ).fetchone()
        payload = serialize_row(
            SYNC_TABLE_MAP["custody_component_legs"],
            row,
            hmac_key_b64="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        payload["role"] = "future_custody_role"
        with self.assertRaises(AppError) as raised:
            validate_wire_row("custody_component_legs", payload)
        self.assertEqual("sync_schema_incompatible", raised.exception.code)

    def test_anchor_coverage_mismatch_cannot_activate_even_when_manually_reviewed(self):
        _tx(self.conn, "mismatch-in", "btc", "inbound", "BTC", 90)
        legs = [
            _leg("source", 90, tx="out", wallet="btc"),
            _leg("destination", 90, tx="mismatch-in", wallet="btc"),
        ]
        native = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            evidence_kind="ownership_graph",
            evidence_grade="exact",
            legs=legs,
        )
        with self.assertRaises(AppError) as raised:
            activate_component(self.conn, native["id"])
        self.assertIn(
            "anchor_coverage_mismatch",
            {
                issue["code"]
                for issue in raised.exception.details["validation"]["issues"]
            },
        )

        reviewed = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            evidence_kind="manual_reconstruction",
            evidence_grade="reviewed",
            legs=legs,
        )
        with self.assertRaises(AppError) as reviewed_error:
            activate_component(self.conn, reviewed["id"])
        self.assertIn(
            "anchor_coverage_mismatch",
            {
                issue["code"]
                for issue in reviewed_error.exception.details["validation"]["issues"]
            },
        )

    def test_transaction_membership_cannot_be_active_in_two_components(self):
        draft = self._balanced_component()
        first = activate_component(self.conn, draft["id"])
        second = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=[
                _leg("source", 60, tx="other-out"),
                _leg("destination", 60, tx="in-1"),
            ],
        )
        with self.assertRaises(AppError) as raised:
            activate_component(self.conn, second["id"])
        self.assertEqual("custody_component_incomplete", raised.exception.code)
        self.assertEqual("active", get_component(self.conn, first["id"])["state"])
        self.assertEqual("draft", get_component(self.conn, second["id"])["state"])

    def test_plan_membership_check_ignores_superseded_rows_like_activation(self):
        first = activate_component(self.conn, self._balanced_component()["id"])
        supersede_component(self.conn, first["id"], reason="retracted review")
        self.conn.executemany(
            "INSERT OR IGNORE INTO custody_component_transaction_memberships("
            "component_id, profile_id, transaction_id, created_at) "
            "VALUES(?, 'profile', ?, ?)",
            [(first["id"], transaction_id, NOW) for transaction_id in ("out", "in-1", "in-2")],
        )

        checked = validate_component_plan(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=first["legs"],
            allocations=first["allocations"],
        )
        self.assertTrue(checked["validation"]["activatable"])
        replacement = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=[
                _leg("source", 100, tx="out", wallet="btc"),
                _leg("destination", 60, tx="in-1", wallet="btc"),
                _leg("destination", 39, tx="in-2", wallet="btc"),
                _leg("fee", 1, tx="out", wallet="btc"),
            ],
        )
        self.assertEqual(
            "active", activate_component(self.conn, replacement["id"])["state"]
        )

    def test_revision_activation_supersedes_old_only_when_new_is_valid(self):
        draft = self._balanced_component()
        first = activate_component(self.conn, draft["id"], activated_at=NOW)
        revision = update_component(
            self.conn,
            first["id"],
            notes="reviewed revision",
            change_reason="add audit note",
            created_at="2026-01-02T00:00:00Z",
        )
        self.assertEqual(2, revision["revision"])
        self.assertEqual("active", get_component(self.conn, first["id"])["state"])
        activated = activate_component(
            self.conn, revision["id"], activated_at="2026-01-03T00:00:00Z"
        )
        self.assertEqual("active", activated["state"])
        old = get_component(self.conn, first["id"])
        self.assertEqual("superseded", old["state"])
        self.assertEqual(revision["id"], old["superseded_by_component_id"])

        supersede_component(self.conn, revision["id"], reason="undo")
        restored = undo_supersede(
            self.conn,
            revision["id"],
            new_component_id="restored",
            created_at="2026-01-04T00:00:00Z",
        )
        self.assertEqual(3, restored["revision"])
        self.assertEqual("draft", restored["state"])

    def test_bitcoin_layer_quantity_conserves_by_exposure_and_reports_asset_residuals(self):
        report = validate_conservation(
            [
                _leg("source", 100, rail="bitcoin", asset="BTC", occurred_at=NOW),
                _leg("destination", 99, rail="liquid", asset="LBTC", occurred_at=NOW),
                _leg("fee", 1, rail="bitcoin", asset="BTC", occurred_at=NOW),
            ]
        )
        self.assertTrue(report["activatable"])
        self.assertEqual(0, report["by_conservation_unit"][0]["residual_msat"])
        assets = {row["asset"]: row["residual_msat"] for row in report["by_asset"]}
        self.assertEqual(99, assets["BTC"])
        self.assertEqual(-99, assets["LBTC"])

    def test_quantity_allocations_respect_logical_bitcoin_network_domains(self):
        def scoped_leg(role, amount, *, leg_id, rail, chain, network, asset):
            return {
                **_leg(
                    role,
                    amount,
                    rail=rail,
                    asset=asset,
                    occurred_at=NOW,
                ),
                "id": leg_id,
                "chain": chain,
                "network": network,
            }

        for source_network, sink_network in (
            ("main", "liquidtestnet"),
            ("test", "elementsregtest"),
            ("regtest", "liquidv1"),
            ("signet", "liquidv1"),
        ):
            with self.subTest(source_network=source_network, sink_network=sink_network):
                legs = [
                    scoped_leg(
                        "source",
                        100,
                        leg_id="source",
                        rail="bitcoin",
                        chain="bitcoin",
                        network=source_network,
                        asset="BTC",
                    ),
                    scoped_leg(
                        "destination",
                        100,
                        leg_id="destination",
                        rail="liquid",
                        chain="liquid",
                        network=sink_network,
                        asset="LBTC",
                    ),
                ]
                inferred = validate_conservation(legs)
                explicit = validate_conservation(
                    legs,
                    allocations=[
                        {
                            "source_leg_id": "source",
                            "sink_leg_id": "destination",
                            "source_amount_msat": 100,
                            "sink_amount_msat": 100,
                        }
                    ],
                )
                for report in (inferred, explicit):
                    self.assertFalse(report["activatable"])
                    self.assertIn(
                        "allocation_network_mismatch",
                        {issue["code"] for issue in report["issues"]},
                    )

        compatible = validate_conservation(
            [
                scoped_leg(
                    "source",
                    100,
                    leg_id="source",
                    rail="lightning",
                    chain="bitcoin",
                    network="main",
                    asset="BTC",
                ),
                scoped_leg(
                    "destination",
                    100,
                    leg_id="destination",
                    rail="liquid",
                    chain="liquid",
                    network="liquidv1",
                    asset="LBTC",
                ),
            ]
        )
        self.assertTrue(compatible["activatable"])

    def test_unknown_gap_cannot_launder_known_network_domains(self):
        legs = [
            {
                **_leg(
                    "source",
                    100,
                    wallet="btc",
                    occurred_at="2021-01-01T00:00:00Z",
                ),
                "id": "main-source",
                "chain": "bitcoin",
                "network": "main",
            },
            {
                **_leg(
                    "retained",
                    100,
                    wallet="node",
                    rail="untracked",
                    occurred_at="2022-01-01T00:00:00Z",
                ),
                "id": "gap-in",
                "chain": None,
                "network": None,
            },
            {
                **_leg(
                    "source",
                    100,
                    wallet="node",
                    rail="untracked",
                    occurred_at="2023-01-01T00:00:00Z",
                ),
                "id": "gap-out",
                "chain": None,
                "network": None,
            },
            {
                **_leg(
                    "destination",
                    100,
                    wallet="liquid",
                    occurred_at="2024-01-01T00:00:00Z",
                ),
                "id": "regtest-destination",
                "chain": "bitcoin",
                "network": "regtest",
            },
        ]
        allocations = [
            {
                "source_leg_id": "main-source",
                "sink_leg_id": "gap-in",
                "source_amount_msat": 100,
                "sink_amount_msat": 100,
            },
            {
                "source_leg_id": "gap-out",
                "sink_leg_id": "regtest-destination",
                "source_amount_msat": 100,
                "sink_amount_msat": 100,
            },
        ]

        incompatible = validate_conservation(legs, allocations=allocations)
        self.assertFalse(incompatible["activatable"])
        self.assertIn(
            "custody_network_scope_laundering",
            {issue["code"] for issue in incompatible["issues"]},
        )

        compatible_legs = [dict(leg) for leg in legs]
        compatible_legs[-1]["network"] = "main"
        compatible = validate_conservation(
            compatible_legs,
            allocations=allocations,
        )
        self.assertTrue(compatible["activatable"])

    def test_allocation_destination_cannot_predate_source(self):
        legs = [
            {
                **_leg(
                    "source",
                    100,
                    wallet="btc",
                    occurred_at="2026-01-01T00:00:00Z",
                ),
                "id": "source",
            },
            {
                **_leg(
                    "destination",
                    100,
                    wallet="liquid",
                    occurred_at="2020-01-01T00:00:00Z",
                ),
                "id": "destination",
            },
        ]
        explicit = [
            {
                "source_leg_id": "source",
                "sink_leg_id": "destination",
                "source_amount_msat": 100,
                "sink_amount_msat": 100,
            }
        ]
        for allocations in (None, explicit):
            with self.subTest(explicit=allocations is not None):
                report = validate_conservation(legs, allocations=allocations)
                self.assertFalse(report["activatable"])
                self.assertIn(
                    "allocation_chronology_mismatch",
                    {issue["code"] for issue in report["issues"]},
                )

    def test_allocation_accepts_bounded_evidence_clock_skew(self):
        report = validate_conservation(
            [
                {
                    **_leg(
                        "source",
                        100,
                        wallet="btc",
                        occurred_at="2026-01-02T00:00:00Z",
                    ),
                    "id": "source",
                },
                {
                    **_leg(
                        "destination",
                        100,
                        wallet="liquid",
                        occurred_at="2026-01-01T22:00:00Z",
                    ),
                    "id": "destination",
                },
            ]
        )

        self.assertTrue(report["activatable"])
        self.assertNotIn(
            "allocation_chronology_mismatch",
            {issue["code"] for issue in report["issues"]},
        )

    def test_allocation_rejects_backward_route_beyond_two_days(self):
        report = validate_conservation(
            [
                {
                    **_leg(
                        "source",
                        100,
                        wallet="btc",
                        occurred_at="2026-01-04T01:00:01Z",
                    ),
                    "id": "source",
                },
                {
                    **_leg(
                        "destination",
                        100,
                        wallet="liquid",
                        occurred_at="2026-01-02T01:00:00Z",
                    ),
                    "id": "destination",
                },
            ]
        )

        self.assertFalse(report["activatable"])
        self.assertIn(
            "allocation_chronology_mismatch",
            {issue["code"] for issue in report["issues"]},
        )

    def test_location_continuity_accepts_bounded_evidence_clock_skew(self):
        report = validate_conservation(
            [
                {
                    **_leg(
                        "source",
                        100,
                        wallet="btc",
                        occurred_at="2026-01-01T00:00:00Z",
                    ),
                    "id": "source",
                },
                {
                    **_leg(
                        "retained",
                        100,
                        wallet="gap",
                        rail="untracked",
                        occurred_at="2026-01-02T02:00:00Z",
                    ),
                    "id": "gap-in",
                },
                {
                    **_leg(
                        "source",
                        100,
                        wallet="gap",
                        rail="untracked",
                        occurred_at="2026-01-02T00:00:00Z",
                    ),
                    "id": "gap-out",
                },
                {
                    **_leg(
                        "destination",
                        100,
                        wallet="liquid",
                        occurred_at="2026-01-03T00:00:00Z",
                    ),
                    "id": "destination",
                },
            ],
            allocations=[
                {
                    "source_leg_id": "source",
                    "sink_leg_id": "gap-in",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                },
                {
                    "source_leg_id": "gap-out",
                    "sink_leg_id": "destination",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                },
            ],
        )

        self.assertTrue(report["activatable"])
        self.assertNotIn(
            "custody_location_continuity_mismatch",
            {issue["code"] for issue in report["issues"]},
        )

    def test_future_bitcoin_layer_can_declare_a_known_network_domain(self):
        report = validate_conservation(
            [
                {
                    **_leg(
                        "source",
                        100,
                        wallet="btc",
                        occurred_at="2026-01-01T00:00:00Z",
                    ),
                    "id": "source",
                    "rail": "ark",
                    "chain": "bitcoin",
                    "network": "main",
                },
                {
                    **_leg(
                        "destination",
                        100,
                        wallet="liquid",
                        occurred_at="2026-01-02T00:00:00Z",
                    ),
                    "id": "destination",
                    "rail": "liquid",
                    "chain": "liquid",
                    "network": "liquidv1",
                    "asset": "LBTC",
                },
            ]
        )

        self.assertTrue(report["activatable"])

    def test_conversion_may_cross_network_domains_after_explicit_review(self):
        report = validate_conservation(
            [
                {
                    **_leg(
                        "source",
                        100,
                        rail="bitcoin",
                        asset="BTC",
                        occurred_at=NOW,
                    ),
                    "id": "source",
                    "chain": "bitcoin",
                    "network": "main",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": 1_000,
                },
                {
                    **_leg(
                        "destination",
                        250,
                        rail="liquid",
                        asset="USDT",
                        occurred_at=NOW,
                    ),
                    "id": "destination",
                    "chain": "liquid",
                    "network": "elementsregtest",
                    "exposure": "tether-usd",
                    "conservation_unit": "asset-quantum",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": 1_000,
                },
            ],
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )
        self.assertTrue(report["activatable"])
        self.assertNotIn(
            "allocation_network_mismatch",
            {issue["code"] for issue in report["issues"]},
        )

    def test_anchor_evidence_fills_omitted_networks_before_allocation_validation(self):
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = 'btc'",
            ('{"chain":"bitcoin","network":"main"}',),
        )
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = 'liquid'",
            ('{"chain":"liquid","network":"elementsregtest"}',),
        )
        _tx(self.conn, "cross-network-in", "liquid", "inbound", "LBTC", 100)
        source = _leg("source", 100, tx="out", wallet="btc")
        destination = _leg(
            "destination",
            100,
            tx="cross-network-in",
            wallet="liquid",
            rail="liquid",
            asset="LBTC",
        )
        for leg in (source, destination):
            leg.pop("chain")
            leg.pop("network")

        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            legs=[source, destination],
        )

        self.assertFalse(component["validation"]["activatable"])
        self.assertIn(
            "allocation_network_mismatch",
            {issue["code"] for issue in component["validation"]["issues"]},
        )

    def test_fee_only_source_is_rejected_before_projection(self):
        legs = [
            {**_leg("source", 90, wallet="btc", occurred_at=NOW), "id": "material"},
            {**_leg("source", 10, wallet="btc", occurred_at=NOW), "id": "fee-only"},
            {**_leg("destination", 90, wallet="btc", occurred_at=NOW), "id": "dest"},
            {**_leg("fee", 10, wallet="btc", occurred_at=NOW), "id": "fee"},
        ]
        report = validate_conservation(
            legs,
            allocations=[
                {
                    "source_leg_id": "material",
                    "sink_leg_id": "dest",
                    "source_amount_msat": 90,
                    "sink_amount_msat": 90,
                },
                {
                    "source_leg_id": "fee-only",
                    "sink_leg_id": "fee",
                    "source_amount_msat": 10,
                    "sink_amount_msat": 10,
                },
            ],
        )
        self.assertFalse(report["activatable"])
        self.assertIn(
            "custody_component_fee_orphaned",
            {issue["code"] for issue in report["issues"]},
        )

    def test_conversion_fee_amount_and_valuation_match_projected_source_loss(self):
        def conversion_legs(*, fee_amount=10, destination_value=900, fee_value=100):
            return [
                {
                    **_leg("source", 100, wallet="btc", occurred_at=NOW),
                    "id": "source",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": 1_000,
                },
                {
                    **_leg(
                        "destination",
                        250,
                        wallet="liquid",
                        rail="liquid",
                        asset="USDT",
                        occurred_at=NOW,
                    ),
                    "id": "destination",
                    "exposure": "tether-usd",
                    "conservation_unit": "asset-quantum",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": destination_value,
                },
                {
                    **_leg("fee", fee_amount, wallet="btc", occurred_at=NOW),
                    "id": "fee",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": fee_value,
                },
            ]

        def allocations(*, fee_sink_amount=10):
            return [
                {
                    "source_leg_id": "source",
                    "sink_leg_id": "destination",
                    "source_amount_msat": 90,
                    "sink_amount_msat": 250,
                },
                {
                    "source_leg_id": "source",
                    "sink_leg_id": "fee",
                    "source_amount_msat": 10,
                    "sink_amount_msat": fee_sink_amount,
                },
            ]

        valid = validate_conservation(
            conversion_legs(),
            allocations=allocations(),
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )
        self.assertTrue(valid["activatable"])

        quantity_mismatch = validate_conservation(
            conversion_legs(fee_amount=5),
            allocations=allocations(fee_sink_amount=5),
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )
        self.assertIn(
            "conversion_fee_quantity_mismatch",
            {issue["code"] for issue in quantity_mismatch["issues"]},
        )

        valuation_mismatch = validate_conservation(
            conversion_legs(destination_value=950, fee_value=50),
            allocations=allocations(),
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )
        self.assertIn(
            "conversion_fee_valuation_mismatch",
            {issue["code"] for issue in valuation_mismatch["issues"]},
        )

    def test_reviewed_conversion_fee_activates_and_projects_end_to_end(self):
        _tx(self.conn, "conversion-basis", "btc", "inbound", "BTC", 100)
        _tx(self.conn, "conversion-in", "liquid", "inbound", "USDT", 250)
        self.conn.execute(
            """
            UPDATE transactions
            SET occurred_at = '2025-01-01T00:00:00Z', kind = 'buy',
                fiat_rate = 100000, fiat_rate_exact = '100000'
            WHERE id = 'conversion-basis'
            """
        )
        self.conn.execute(
            """
            UPDATE transactions
            SET kind = CASE WHEN id = 'out' THEN 'sell' ELSE 'buy' END,
                fiat_rate = 100000, fiat_rate_exact = '100000'
            WHERE id IN ('out', 'conversion-in')
            """
        )
        self.conn.execute(
            """
            UPDATE transactions
            SET excluded = 1
            WHERE id NOT IN ('conversion-basis', 'out', 'conversion-in')
            """
        )
        component = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="swap",
            legs=[
                {
                    **_leg("source", 100, tx="out", wallet="btc"),
                    "id": "source",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": 1_000,
                },
                {
                    **_leg(
                        "destination",
                        250,
                        tx="conversion-in",
                        wallet="liquid",
                        rail="liquid",
                        asset="USDT",
                    ),
                    "id": "destination",
                    "exposure": "tether-usd",
                    "conservation_unit": "asset-quantum",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": 900,
                },
                {
                    **_leg("fee", 10, tx="out", wallet="btc"),
                    "id": "fee",
                    "valuation_unit": "eur-cent",
                    "valuation_amount": 100,
                },
            ],
            allocations=[
                {
                    "source_leg_id": "source",
                    "sink_leg_id": "destination",
                    "source_amount_msat": 90,
                    "sink_amount_msat": 250,
                },
                {
                    "source_leg_id": "source",
                    "sink_leg_id": "fee",
                    "source_amount_msat": 10,
                    "sink_amount_msat": 10,
                },
            ],
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )
        activated = activate_component(
            self.conn, component["id"], activated_at=NOW
        )
        self.assertEqual("active", activated["effective_state"])

        processed = process_journals(self.conn, "ws", "profile")

        self.assertFalse(processed.get("custody_component_blockers", []))
        self.assertFalse(processed["custody_quantity"]["blocked"])
        self.assertEqual(0, processed["custody_quantity"]["issues"])
        self.assertEqual(1, processed["cross_asset_pairs"])

        classified = [
            (row["location_kind"], row["amount_msat"])
            for row in self.conn.execute(
                """
                SELECT location_kind, amount_msat
                FROM journal_quantity_postings
                WHERE transaction_id = 'out'
                  AND location_kind IN ('external', 'fee')
                ORDER BY location_kind, amount_msat
                """
            ).fetchall()
        ]
        self.assertEqual([("external", 90), ("fee", 10)], classified)

        projected_entries = {
            (row["transaction_id"], row["entry_type"], row["asset"], row["quantity"])
            for row in self.conn.execute(
                """
                SELECT transaction_id, entry_type, asset, quantity
                FROM journal_entries
                WHERE transaction_id IN ('out', 'conversion-in')
                """
            ).fetchall()
        }
        self.assertIn(("out", "disposal", "BTC", -90), projected_entries)
        self.assertIn(
            ("conversion-in", "acquisition", "USDT", 250),
            projected_entries,
        )
        self.assertTrue(
            any(
                transaction_id == "out"
                and entry_type == "fee"
                and asset == "BTC"
                and quantity == -10
                for transaction_id, entry_type, asset, quantity in projected_entries
            )
        )

    def test_conversion_requires_review_policy_and_balanced_exact_valuations(self):
        legs = [
            {
                **_leg("source", 100, rail="bitcoin", asset="BTC", occurred_at=NOW),
                "exposure": "bitcoin",
                "valuation_unit": "eur-cent",
                "valuation_amount": 1000,
            },
            {
                **_leg("destination", 900, rail="liquid", asset="USDT", occurred_at=NOW),
                "exposure": "tether-usd",
                "conservation_unit": "asset-quantum",
                "valuation_unit": "eur-cent",
                "valuation_amount": 990,
            },
            {
                **_leg("fee", 0, rail="liquid", asset="USDT", occurred_at=NOW),
                "exposure": "tether-usd",
                "conservation_unit": "asset-quantum",
                "valuation_unit": "eur-cent",
                "valuation_amount": 10,
            },
        ]
        unreviewed = validate_conservation(legs, conservation_mode="conversion")
        self.assertFalse(unreviewed["activatable"])
        reviewed = validate_conservation(
            legs,
            conservation_mode="conversion",
            conversion_policy="reviewed-market-conversion",
            conversion_reviewed=True,
        )
        self.assertFalse(reviewed["activatable"])
        self.assertIn(
            "custody_component_value_only_loss_unsupported",
            {issue["code"] for issue in reviewed["issues"]},
        )

        # A conversion with no fiat-only synthetic loss remains projectable.
        projectable = validate_conservation(
            [
                {**legs[0], "valuation_amount": 990},
                legs[1],
            ],
            conservation_mode="conversion",
            conversion_policy="reviewed-market-conversion",
            conversion_reviewed=True,
        )
        self.assertTrue(projectable["activatable"])
        self.assertEqual(
            0, projectable["by_valuation_unit"][0]["residual_amount"]
        )

        for loss_role in ("fee", "external"):
            with self.subTest(loss_role=loss_role):
                value_only = validate_conservation(
                    [legs[0], legs[1], {**legs[2], "role": loss_role}],
                    conservation_mode="conversion",
                    conversion_policy="reviewed-market-conversion",
                    conversion_reviewed=True,
                )
                self.assertFalse(value_only["activatable"])
                self.assertIn(
                    "custody_component_value_only_loss_unsupported",
                    {issue["code"] for issue in value_only["issues"]},
                )

    def test_conversion_rejects_multi_source_or_multi_destination_topology(self):
        legs = [
            {
                **_leg("source", 60, rail="bitcoin", asset="BTC", occurred_at=NOW),
                "id": "s1",
                "valuation_unit": "eur-cent",
                "valuation_amount": 600,
            },
            {
                **_leg("source", 40, rail="liquid", asset="USDT", occurred_at=NOW),
                "id": "s2",
                "exposure": "tether-usd",
                "conservation_unit": "asset-quantum",
                "valuation_unit": "eur-cent",
                "valuation_amount": 400,
            },
            {
                **_leg("destination", 100, rail="liquid", asset="EURX", occurred_at=NOW),
                "id": "d1",
                "exposure": "eurx",
                "conservation_unit": "asset-quantum",
                "valuation_unit": "eur-cent",
                "valuation_amount": 1000,
            },
        ]
        report = validate_conservation(
            legs,
            allocations=[
                {
                    "source_leg_id": "s1",
                    "sink_leg_id": "d1",
                    "source_amount_msat": 60,
                    "sink_amount_msat": 60,
                },
                {
                    "source_leg_id": "s2",
                    "sink_leg_id": "d1",
                    "source_amount_msat": 40,
                    "sink_amount_msat": 40,
                },
            ],
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )
        self.assertFalse(report["activatable"])
        self.assertIn(
            "conversion_topology_unsupported",
            {issue["code"] for issue in report["issues"]},
        )

    def test_fee_allocation_must_name_the_asset_and_wallet_that_lost_value(self):
        legs = [
            {**_leg("source", 100, tx="out", wallet="btc"), "id": "source"},
            {
                **_leg(
                    "destination",
                    99,
                    wallet="liquid",
                    rail="liquid",
                    asset="LBTC",
                    occurred_at=NOW,
                ),
                "id": "dest",
            },
            {
                **_leg(
                    "fee",
                    1,
                    wallet="liquid",
                    rail="liquid",
                    asset="LBTC",
                    occurred_at=NOW,
                ),
                "id": "fee",
            },
        ]
        report = validate_conservation(
            legs,
            allocations=[
                {
                    "source_leg_id": "source",
                    "sink_leg_id": "dest",
                    "source_amount_msat": 99,
                    "sink_amount_msat": 99,
                },
                {
                    "source_leg_id": "source",
                    "sink_leg_id": "fee",
                    "source_amount_msat": 1,
                    "sink_amount_msat": 1,
                },
            ],
        )
        codes = {issue["code"] for issue in report["issues"]}
        self.assertIn("fee_source_asset_mismatch", codes)
        self.assertIn("fee_source_wallet_mismatch", codes)

    def test_transactionless_owned_leg_requires_occurrence_time(self):
        report = validate_conservation(
            [
                _leg("source", 10, rail="untracked"),
                _leg("destination", 10, rail="bitcoin", occurred_at=NOW),
            ]
        )
        self.assertFalse(report["activatable"])
        self.assertIn("leg_occurred_at_missing", {issue["code"] for issue in report["issues"]})

    def test_invalid_transactionless_timestamp_is_rejected_before_journal(self):
        report = validate_conservation(
            [
                _leg("source", 10, wallet="btc", occurred_at="not-a-date"),
                _leg("destination", 10, wallet="liquid", occurred_at=NOW),
            ]
        )
        self.assertFalse(report["activatable"])
        self.assertIn(
            "leg_occurred_at_invalid",
            {issue["code"] for issue in report["issues"]},
        )

    def test_anchored_leg_cannot_override_transaction_time(self):
        with self.assertRaises(AppError) as raised:
            create_component(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                component_type="native_transfer",
                legs=[
                    _leg(
                        "source",
                        100,
                        tx="out",
                        wallet="btc",
                        occurred_at="1900-01-01T00:00:00Z",
                    ),
                    _leg("destination", 100, wallet="liquid", occurred_at=NOW),
                ],
            )
        self.assertEqual(
            "custody_component_anchor_time_mismatch", raised.exception.code
        )

    def test_canonical_anchor_times_block_backward_allocation_activation(self):
        _tx(self.conn, "past-in", "btc", "inbound", "BTC", 100)
        self.conn.execute(
            "UPDATE transactions SET occurred_at = '2020-01-01T00:00:00Z' "
            "WHERE id = 'past-in'"
        )
        draft = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="native_transfer",
            legs=[
                _leg("source", 100, tx="out", wallet="btc"),
                _leg("destination", 100, tx="past-in", wallet="btc"),
            ],
        )

        self.assertFalse(draft["validation"]["activatable"])
        self.assertIn(
            "allocation_chronology_mismatch",
            {issue["code"] for issue in draft["validation"]["issues"]},
        )
        with self.assertRaises(AppError) as raised:
            activate_component(self.conn, draft["id"])
        self.assertEqual("custody_component_incomplete", raised.exception.code)

    def test_active_components_cannot_launder_scope_through_shared_gap(self):
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = 'btc'",
            ('{"chain":"bitcoin","network":"main"}',),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES('gap', 'ws', 'profile', 'gap', 'untracked', '{}', ?),
                    ('regtest', 'ws', 'profile', 'regtest', 'descriptor',
                     '{"chain":"bitcoin","network":"regtest"}', ?)
            """,
            (NOW, NOW),
        )
        _tx(self.conn, "regtest-in", "regtest", "inbound", "BTC", 100)
        self.conn.execute(
            "UPDATE transactions SET occurred_at = '2027-01-01T00:00:00Z' "
            "WHERE id = 'regtest-in'"
        )

        first = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            legs=[
                {
                    **_leg("source", 100, tx="out", wallet="btc"),
                    "id": "main-source",
                    "chain": None,
                    "network": None,
                },
                {
                    **_leg(
                        "retained",
                        100,
                        wallet="gap",
                        rail="untracked",
                        occurred_at="2026-06-01T00:00:00Z",
                    ),
                    "id": "gap-in",
                    "chain": None,
                    "network": None,
                },
            ],
        )
        activate_component(self.conn, first["id"])

        second = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            legs=[
                {
                    **_leg(
                        "source",
                        100,
                        wallet="gap",
                        rail="untracked",
                        occurred_at="2025-07-01T00:00:00Z",
                    ),
                    "id": "gap-out",
                    "chain": None,
                    "network": None,
                },
                {
                    **_leg(
                        "destination",
                        100,
                        tx="regtest-in",
                        wallet="regtest",
                    ),
                    "id": "regtest-destination",
                    "chain": None,
                    "network": None,
                },
            ],
        )

        self.assertFalse(second["validation"]["activatable"])
        laundering = [
            issue
            for issue in second["validation"]["issues"]
            if issue["code"] == "custody_network_scope_laundering"
        ]
        self.assertTrue(laundering)
        self.assertIn(
            "custody_location_continuity_mismatch",
            {issue["code"] for issue in second["validation"]["issues"]},
        )
        self.assertEqual(
            {first["id"], second["id"]},
            set(laundering[-1]["component_ids"]),
        )
        with self.assertRaises(AppError):
            activate_component(self.conn, second["id"])

        listed = next(
            item
            for item in list_components(self.conn, profile_id="profile", limit=1000)
            if item["id"] == second["id"]
        )
        detailed = get_component(self.conn, second["id"])
        self.assertEqual(
            detailed["validation"]["activatable"],
            listed["validation"]["activatable"],
        )
        self.assertEqual(
            {issue["code"] for issue in detailed["validation"]["issues"]},
            {issue["code"] for issue in listed["validation"]["issues"]},
        )

        # Replication can deliver an authored-active header without going
        # through local activation. Both routes must then become ineffective
        # instead of letting arrival order choose which network wins.
        self.conn.execute(
            "UPDATE custody_components SET state = 'active' WHERE id = ?",
            (second["id"],),
        )
        for component_id in (first["id"], second["id"]):
            replicated = get_component(self.conn, component_id)
            self.assertEqual("active", replicated["state"])
            self.assertEqual("draft", replicated["effective_state"])
            self.assertIn(
                "custody_network_scope_laundering",
                {issue["code"] for issue in replicated["validation"]["issues"]},
            )

    def test_anchor_scopes_block_single_component_unknown_gap_laundering(self):
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = 'btc'",
            ('{"chain":"bitcoin","network":"main"}',),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES('single-gap', 'ws', 'profile', 'single gap', 'untracked', '{}', ?),
                    ('single-regtest', 'ws', 'profile', 'single regtest', 'descriptor',
                     '{"chain":"bitcoin","network":"regtest"}', ?)
            """,
            (NOW, NOW),
        )
        _tx(self.conn, "single-regtest-in", "single-regtest", "inbound", "BTC", 100)
        self.conn.execute(
            "UPDATE transactions SET occurred_at = '2027-01-01T00:00:00Z' "
            "WHERE id = 'single-regtest-in'"
        )
        draft = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            legs=[
                {
                    **_leg("source", 100, tx="out", wallet="btc"),
                    "id": "single-main-source",
                    "chain": None,
                    "network": None,
                },
                {
                    **_leg(
                        "retained",
                        100,
                        wallet="single-gap",
                        rail="untracked",
                        occurred_at="2026-06-01T00:00:00Z",
                    ),
                    "id": "single-gap-in",
                    "chain": None,
                    "network": None,
                },
                {
                    **_leg(
                        "source",
                        100,
                        wallet="single-gap",
                        rail="untracked",
                        occurred_at="2026-07-01T00:00:00Z",
                    ),
                    "id": "single-gap-out",
                    "chain": None,
                    "network": None,
                },
                {
                    **_leg(
                        "destination",
                        100,
                        tx="single-regtest-in",
                        wallet="single-regtest",
                    ),
                    "id": "single-regtest-destination",
                    "chain": None,
                    "network": None,
                },
            ],
            allocations=[
                {
                    "source_leg_id": "single-main-source",
                    "sink_leg_id": "single-gap-in",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                },
                {
                    "source_leg_id": "single-gap-out",
                    "sink_leg_id": "single-regtest-destination",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                },
            ],
        )

        self.assertFalse(draft["validation"]["activatable"])
        self.assertIn(
            "custody_network_scope_laundering",
            {issue["code"] for issue in draft["validation"]["issues"]},
        )
        with self.assertRaises(AppError):
            activate_component(self.conn, draft["id"])

    def test_missing_wallet_hop_cannot_spend_before_component_credit(self):
        legs = [
            {
                **_leg("source", 100, wallet="btc", occurred_at="2021-01-01T00:00:00Z"),
                "id": "initial-source",
            },
            {
                **_leg("retained", 100, wallet="node", rail="untracked", occurred_at="2025-01-01T00:00:00Z"),
                "id": "gap-in",
            },
            {
                **_leg("source", 100, wallet="node", rail="untracked", occurred_at="2023-01-01T00:00:00Z"),
                "id": "gap-out",
            },
            {
                **_leg("destination", 100, wallet="liquid", occurred_at="2026-01-01T00:00:00Z"),
                "id": "final-destination",
            },
        ]
        report = validate_conservation(
            legs,
            allocations=[
                {
                    "source_leg_id": "initial-source",
                    "sink_leg_id": "gap-in",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                },
                {
                    "source_leg_id": "gap-out",
                    "sink_leg_id": "final-destination",
                    "source_amount_msat": 100,
                    "sink_amount_msat": 100,
                },
            ],
        )
        self.assertFalse(report["activatable"])
        self.assertIn(
            "custody_location_continuity_mismatch",
            {issue["code"] for issue in report["issues"]},
        )

    def test_anchor_direction_asset_wallet_and_transactionless_wallet_are_checked(self):
        wrong = create_component(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            component_type="manual_bridge",
            legs=[
                # An inbound row cannot anchor a source even when quantities balance.
                _leg("source", 60, tx="in-1", wallet="btc"),
                _leg("destination", 60, rail="untracked", occurred_at=NOW),
            ],
        )
        issues = {issue["code"] for issue in wrong["validation"]["issues"]}
        self.assertIn("source_anchor_direction_mismatch", issues)
        self.assertIn("transactionless_leg_wallet_missing", issues)
        with self.assertRaises(AppError):
            activate_component(self.conn, wrong["id"])

    def test_half_replicated_active_component_is_not_effective(self):
        self.conn.execute(
            """
            INSERT INTO custody_components(
                id, lineage_id, workspace_id, profile_id, revision,
                component_type, state, expected_leg_count,
                expected_allocation_count, expected_economic_term_count,
                created_at
            ) VALUES('remote', 'remote', 'ws', 'profile', 1,
                     'native_transfer', 'active', 2, 0, 0, ?)
            """,
            (NOW,),
        )
        remote = get_component(self.conn, "remote")
        self.assertEqual("draft", remote["effective_state"])
        self.assertIn(
            "component_leg_count_mismatch",
            {issue["code"] for issue in remote["validation"]["issues"]},
        )
        self.assertEqual([], list_effective_components(self.conn, profile_id="profile"))
        authored_active = list(
            iter_authored_active_components(self.conn, profile_id="profile")
        )
        self.assertEqual(["remote"], [item["id"] for item in authored_active])
        self.assertEqual("active", authored_active[0]["state"])
        self.assertEqual("draft", authored_active[0]["effective_state"])
        self.assertEqual(
            [], list(iter_effective_components(self.conn, profile_id="profile"))
        )
        result = reconcile_active_memberships(self.conn, profile_id="profile")
        self.assertEqual([], result["effective_component_ids"])
        self.assertEqual("remote", result["incomplete"][0]["component_id"])


if __name__ == "__main__":
    unittest.main()
