import unittest
from tests import test_transaction_graph as fixtures


class NetworkPartitionTests(unittest.TestCase):
    def setUp(self):
        self.book = fixtures.TransactionGraphTest()
        self.book.setUp()
        self.addCleanup(self.book.tearDown)
        self.conn = self.book.conn
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        self.book._tx("main", "wallet-a", "inbound", 1000000, "a" * 64, {"network": "main"})
        self.book._tx("regtest", "wallet-b", "inbound", 1000000, "b" * 64, {"network": "regtest"})
        self.args = {"environment": "main", "wallet_ids": ["wallet-a"], "declared_wallet_ids": ["wallet-a"]}

    def test_partition_preserves_ids_and_excludes_other_wallets_and_private_backends(self):
        import io
        import sqlite3
        import tarfile
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from kassiber.core.book_network_migration import plan_network_partition, export_network_partition
        plan = plan_network_partition(self.conn, "profile-1", self.args)
        self.assertTrue(plan["can_apply"], plan["blockers"])
        before = self.conn.total_changes
        def capture(source, output, **kwargs):
            self.assertEqual(kwargs, {"recipients": ["age1test"]})
            output.write(source.read())
        with tempfile.TemporaryDirectory() as folder, patch("kassiber.core.book_network_migration.encrypt_age_stream", side_effect=capture):
            output = Path(folder) / "partition.age"
            export_network_partition(self.conn, "profile-1", {**self.args, "plan_id": plan["plan_id"]}, data_root=folder, output_path=output, recipient="age1test")
            with tarfile.open(output) as archive:
                payload = archive.extractfile("kassiber.sqlite3").read()
            database = Path(folder) / "partition.sqlite3"
            database.write_bytes(payload)
            target = sqlite3.connect(database)
            self.addCleanup(target.close)
            self.assertEqual(target.execute("SELECT id FROM transactions").fetchall(), [("main",)])
            self.assertEqual(target.execute("SELECT id FROM wallets").fetchall(), [("wallet-a",)])
            self.assertEqual(target.execute("SELECT count(*) FROM backends").fetchone()[0], 0)
            self.assertEqual(target.execute("SELECT last_processed_at FROM profiles").fetchone()[0], None)
            self.assertEqual(target.execute("PRAGMA foreign_key_check").fetchall(), [])
        self.assertEqual(self.conn.total_changes, before)

    def test_authored_relation_across_partition_blocks_copy(self):
        from kassiber.core.book_network_migration import plan_network_partition
        from tests.custody_projection_fixtures import insert_reviewed_projection
        insert_reviewed_projection(self.conn, projection_id="c" * 64, workspace_id="ws-1", profile_id="profile-1", source_transaction_id="main", target_transaction_id="regtest", source_asset="BTC", target_asset="BTC", source_amount_msat=1000000, target_amount_msat=1000000, review_kind="manual_bridge", relation_kind="move", occurred_at=fixtures.NOW, target_occurred_at=fixtures.NOW)
        plan = plan_network_partition(self.conn, "profile-1", self.args)
        self.assertFalse(plan["can_apply"])
        self.assertIn("relation_crosses_partition", {row["code"] for row in plan["blockers"]})

    def test_supported_backup_restore_opens_a_fresh_usable_project(self):
        import tempfile
        from pathlib import Path
        from kassiber.backup.pack import import_backup
        from kassiber.db import open_db, database_instance_id
        from kassiber.core.book_network_migration import plan_network_partition, export_network_partition
        try:
            import pyrage  # noqa: F401
        except ImportError:
            self.skipTest("pyrage unavailable")
        plan = plan_network_partition(self.conn, "profile-1", self.args)
        with tempfile.TemporaryDirectory() as folder:
            archive = Path(folder) / "partition.kassiber"
            export_network_partition(self.conn, "profile-1", {**self.args, "plan_id": plan["plan_id"]}, data_root=folder, output_path=archive, backup_passphrase="partition-test-password")
            destination = Path(folder) / "restored"
            result = import_backup(archive, destination, backup_passphrase="partition-test-password", move_into_place=True)
            self.assertEqual(result.installed_data_root, destination)
            restored = open_db(destination, require_existing_schema=True)
            self.addCleanup(restored.close)
            self.assertEqual(restored.execute("SELECT id FROM transactions").fetchone()[0], "main")
            self.assertTrue(database_instance_id(restored))
            self.assertNotEqual(database_instance_id(restored), database_instance_id(self.conn))
            self.assertEqual(restored.execute("SELECT environment FROM book_network_bindings").fetchone()[0], "main")
