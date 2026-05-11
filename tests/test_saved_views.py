"""Saved-views CRUD pinning."""

import tempfile
import unittest
import uuid

from kassiber.core.saved_views import (
    SURFACE_SWAP_CANDIDATES,
    create_view,
    delete_view,
    list_views,
    update_view,
)
from kassiber.db import open_db
from kassiber.errors import AppError


def _now():
    return "2026-01-01T00:00:00Z"


def _seed_minimal_scope(conn):
    workspace_id = str(uuid.uuid4())
    profile_id = str(uuid.uuid4())
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace_id, f"ws-{workspace_id[:8]}", _now()),
    )
    conn.execute(
        """
        INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country,
                             tax_long_term_days, gains_algorithm, journal_input_version,
                             last_processed_input_version, last_processed_tx_count, created_at)
        VALUES(?, ?, ?, 'EUR', 'at', 365, 'FIFO', 0, 0, 0, ?)
        """,
        (profile_id, workspace_id, "main", _now()),
    )
    return workspace_id, profile_id


class SavedViewsTests(unittest.TestCase):
    def test_create_and_list(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                view = create_view(
                    conn,
                    workspace_id,
                    profile_id,
                    surface=SURFACE_SWAP_CANDIDATES,
                    name="Pegouts awaiting review",
                    filter_payload={"asset_pair": "BTC-LBTC", "min_confidence": "strong"},
                )
                self.assertEqual(view["name"], "Pegouts awaiting review")
                self.assertEqual(view["surface"], SURFACE_SWAP_CANDIDATES)
                self.assertEqual(view["filter"]["asset_pair"], "BTC-LBTC")
                self.assertIn("created_at", view)
                listed = list_views(conn, profile_id)
                self.assertEqual(len(listed), 1)
                self.assertEqual(listed[0]["id"], view["id"])
            finally:
                conn.close()

    def test_list_filters_by_surface(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                create_view(conn, workspace_id, profile_id, surface=SURFACE_SWAP_CANDIDATES, name="A")
                create_view(conn, workspace_id, profile_id, surface="other-surface", name="B")
                swap_views = list_views(conn, profile_id, surface=SURFACE_SWAP_CANDIDATES)
                self.assertEqual([v["name"] for v in swap_views], ["A"])
            finally:
                conn.close()

    def test_duplicate_name_on_surface_rejected(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                create_view(conn, workspace_id, profile_id, surface="swap", name="A")
                with self.assertRaises(AppError) as ctx:
                    create_view(conn, workspace_id, profile_id, surface="swap", name="A")
                self.assertEqual(ctx.exception.code, "conflict")
            finally:
                conn.close()

    def test_same_name_on_different_surface_allowed(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                create_view(conn, workspace_id, profile_id, surface="surface-a", name="View")
                create_view(conn, workspace_id, profile_id, surface="surface-b", name="View")
                listed = list_views(conn, profile_id)
                self.assertEqual(len(listed), 2)
            finally:
                conn.close()

    def test_delete_view(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                view = create_view(conn, workspace_id, profile_id, surface="swap", name="Tmp")
                result = delete_view(conn, profile_id, view["id"])
                self.assertEqual(result, {"deleted": view["id"]})
                self.assertEqual(list_views(conn, profile_id), [])
            finally:
                conn.close()

    def test_delete_other_profile_view_blocked(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                view = create_view(conn, workspace_id, profile_id, surface="swap", name="Mine")
                with self.assertRaises(AppError) as ctx:
                    delete_view(conn, "different-profile", view["id"])
                self.assertEqual(ctx.exception.code, "not_found")
            finally:
                conn.close()

    def test_update_view_name_and_filter(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                view = create_view(conn, workspace_id, profile_id, surface="swap", name="Old")
                updated = update_view(
                    conn,
                    profile_id,
                    view["id"],
                    name="New",
                    filter_payload={"foo": "bar"},
                )
                self.assertEqual(updated["name"], "New")
                self.assertEqual(updated["filter"], {"foo": "bar"})
                self.assertGreaterEqual(updated["updated_at"], view["created_at"])
            finally:
                conn.close()

    def test_update_to_duplicate_name_rejected(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                create_view(conn, workspace_id, profile_id, surface="swap", name="Taken")
                view = create_view(conn, workspace_id, profile_id, surface="swap", name="Free")
                with self.assertRaises(AppError) as ctx:
                    update_view(conn, profile_id, view["id"], name="Taken")
                self.assertEqual(ctx.exception.code, "conflict")
            finally:
                conn.close()

    def test_empty_name_rejected(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id = _seed_minimal_scope(conn)
                with self.assertRaises(AppError) as ctx:
                    create_view(conn, workspace_id, profile_id, surface="swap", name="   ")
                self.assertEqual(ctx.exception.code, "validation")
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
