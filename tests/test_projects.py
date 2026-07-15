from __future__ import annotations

import json
import queue
import sqlite3
import tempfile
import threading
import unittest
from pathlib import Path

from kassiber.core.runtime import resolve_runtime_paths
from kassiber.db import DEFAULT_DB_FILENAME, open_db
from kassiber.errors import AppError
from kassiber.projects import (
    WORKSPACE_SPLIT_POLICY,
    catalog_path,
    create_project,
    load_catalog,
    migrate_legacy_default_layout_if_needed,
    project_for_runtime,
    refresh_project_metadata,
)
from kassiber.secrets.migration import create_empty_encrypted_database
from kassiber.secrets.sqlcipher import sqlcipher_available


class ProjectCatalogTests(unittest.TestCase):
    def test_workspace_split_inventory_keeps_review_transaction_scope(self):
        tables = WORKSPACE_SPLIT_POLICY["workspace_profile_scoped_tables"]
        self.assertLess(
            tables.index("custody_gap_reviews"),
            tables.index("custody_gap_review_transactions"),
        )

    def test_catalog_stores_only_non_secret_project_metadata(self):
        with tempfile.TemporaryDirectory() as root:
            entry = create_project("Family Treasury", state_root=root)
            entry.database.parent.mkdir(parents=True, exist_ok=True)
            entry.database.write_bytes(b"not a plaintext sqlite header")
            refresh_project_metadata(entry.id, data_root=entry.data_root, state_root=root)

            raw = json.loads(catalog_path(root).read_text(encoding="utf-8"))
            self.assertEqual(raw["schema_version"], 1)
            self.assertEqual(raw["selected_project_id"], "family-treasury")
            self.assertEqual(
                sorted(raw["projects"][0]),
                ["encrypted", "id", "last_opened_at", "name", "path"],
            )
            self.assertTrue(raw["projects"][0]["encrypted"])
            serialized = json.dumps(raw, sort_keys=True)
            self.assertNotIn("family-passphrase", serialized)
            self.assertNotIn("verifier", serialized.lower())
            self.assertNotIn("xpub", serialized.lower())

    def test_two_encrypted_projects_can_use_different_passphrases(self):
        if not sqlcipher_available():
            self.skipTest("SQLCipher driver is not installed")
        with tempfile.TemporaryDirectory() as root:
            alpha = create_project("Alpha", project_id="alpha", state_root=root)
            beta = create_project("Beta", project_id="beta", state_root=root, select=False)
            create_empty_encrypted_database(alpha.database, "alpha-passphrase-12345")
            create_empty_encrypted_database(beta.database, "beta-passphrase-12345")
            refresh_project_metadata(alpha.id, data_root=alpha.data_root, state_root=root)
            refresh_project_metadata(beta.id, data_root=beta.data_root, state_root=root)

            conn = open_db(str(alpha.data_root), passphrase="alpha-passphrase-12345")
            conn.close()
            with self.assertRaises(AppError) as ctx:
                open_db(str(beta.data_root), passphrase="alpha-passphrase-12345").close()
            self.assertEqual(ctx.exception.code, "unlock_failed")

            conn = open_db(str(beta.data_root), passphrase="beta-passphrase-12345")
            conn.close()

    def test_explicit_data_root_recovers_catalog_project_metadata(self):
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            old_state = projects_module.DEFAULT_STATE_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(Path(root) / ".kassiber")
                entry = create_project("Family", project_id="family")

                paths = resolve_runtime_paths(data_root=str(entry.data_root))

                self.assertEqual(paths.project_id, "family")
                self.assertEqual(paths.project_name, "Family")
                self.assertEqual(paths.project_root, str(entry.root))
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state

    def test_explicit_external_project_data_root_recovers_catalog_metadata(self):
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            old_state = projects_module.DEFAULT_STATE_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(Path(root) / ".kassiber")
                external_root = Path(root) / "external-family-project"
                entry = create_project("Family", project_id="family", root=external_root)

                paths = resolve_runtime_paths(data_root=str(entry.data_root))

                self.assertEqual(paths.project_id, "family")
                self.assertEqual(paths.project_name, "Family")
                self.assertEqual(paths.project_root, str(external_root))
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state

    def test_project_create_can_refuse_existing_catalog_id(self):
        with tempfile.TemporaryDirectory() as root:
            create_project("Family", project_id="family", state_root=root)

            with self.assertRaises(AppError) as ctx:
                create_project(
                    "Family Again",
                    project_id="family",
                    state_root=root,
                    replace_existing=False,
                )

            self.assertEqual(ctx.exception.code, "project_exists")

    def test_project_create_can_refuse_existing_database_path(self):
        with tempfile.TemporaryDirectory() as root:
            project_root = Path(root) / "external"
            db_path = project_root / "data" / DEFAULT_DB_FILENAME
            db_path.parent.mkdir(parents=True)
            db_path.write_bytes(b"not empty")

            with self.assertRaises(AppError) as ctx:
                create_project(
                    "External",
                    project_id="external",
                    state_root=root,
                    root=project_root,
                    allow_existing_database=False,
                )

            self.assertEqual(ctx.exception.code, "database_exists")

    def test_explicit_runtime_project_does_not_change_selection_before_open(self):
        with tempfile.TemporaryDirectory() as root:
            create_project("Alpha", project_id="alpha", state_root=root)
            create_project("Beta", project_id="beta", state_root=root, select=False)

            entry = project_for_runtime(project_id="beta", state_root=root)

            self.assertEqual(entry.id, "beta")
            self.assertEqual(load_catalog(catalog_path(root))["selected_project_id"], "alpha")

    def test_project_and_data_root_flags_conflict(self):
        with self.assertRaises(AppError) as ctx:
            resolve_runtime_paths(data_root="/tmp/example", project="family")

        self.assertEqual(ctx.exception.code, "invalid_flag_combination")


class LegacyProjectMigrationTests(unittest.TestCase):
    def test_multi_workspace_legacy_db_migrates_as_single_project_and_moves_old_artifacts(self):
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            state_root = Path(root) / ".kassiber"
            data_root = state_root / "data"
            data_root.mkdir(parents=True)
            attachments_root = state_root / "attachments"
            attachments_root.mkdir()
            (attachments_root / "evidence.txt").write_text("receipt", encoding="utf-8")
            config_root = state_root / "config"
            config_root.mkdir()
            (config_root / "backends.env").write_text("TOKEN=plaintext\n", encoding="utf-8")
            db_path = data_root / DEFAULT_DB_FILENAME
            conn = sqlite3.connect(db_path)
            try:
                conn.execute("CREATE TABLE workspaces(id TEXT PRIMARY KEY, label TEXT, created_at TEXT)")
                conn.execute("INSERT INTO workspaces VALUES('one', 'One', '2026-01-01T00:00:00Z')")
                conn.execute("INSERT INTO workspaces VALUES('two', 'Two', '2026-01-01T00:00:00Z')")
                conn.commit()
            finally:
                conn.close()

            old_state = projects_module.DEFAULT_STATE_ROOT
            old_data = projects_module.DEFAULT_DATA_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(state_root)
                projects_module.DEFAULT_DATA_ROOT = str(data_root)
                migrated = migrate_legacy_default_layout_if_needed()
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state
                projects_module.DEFAULT_DATA_ROOT = old_data

            self.assertEqual(migrated.id, "default")
            migrated_db = state_root / "projects" / "default" / "data" / DEFAULT_DB_FILENAME
            self.assertTrue(migrated_db.exists())
            migrated_conn = sqlite3.connect(migrated_db)
            try:
                self.assertEqual(migrated_conn.execute("SELECT COUNT(*) FROM workspaces").fetchone()[0], 2)
            finally:
                migrated_conn.close()
            self.assertFalse(db_path.exists())
            backups = sorted(state_root.glob("pre-project-migration-*"))
            self.assertEqual(len(backups), 1)
            self.assertTrue((backups[0] / "data" / DEFAULT_DB_FILENAME).exists())
            self.assertTrue((backups[0] / "attachments" / "evidence.txt").exists())
            self.assertTrue((backups[0] / "config" / "backends.env").exists())
            reports = sorted((state_root / "config" / "migration-reports").glob("*.json"))
            self.assertTrue(reports)
            payload = json.loads(reports[-1].read_text(encoding="utf-8"))
            self.assertEqual(payload["status"], "staged_multi_workspace_single_project_copy")
            self.assertEqual(payload["details"]["workspace_count"], 2)
            self.assertEqual(
                payload["split_policy"]["project_shared_tables_copied_to_each_split_project"],
                WORKSPACE_SPLIT_POLICY["project_shared_tables_copied_to_each_split_project"],
            )

    def test_legacy_xdg_database_is_migrated_instead_of_orphaned(self):
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            state_root = Path(root) / ".kassiber"
            default_data_root = state_root / "data"
            xdg_root = Path(root) / ".local" / "share" / "kassiber"
            xdg_root.mkdir(parents=True)
            db_path = xdg_root / DEFAULT_DB_FILENAME
            conn = sqlite3.connect(db_path)
            try:
                conn.execute("CREATE TABLE workspaces(id TEXT PRIMARY KEY, label TEXT, created_at TEXT)")
                conn.execute("INSERT INTO workspaces VALUES('one', 'One', '2026-01-01T00:00:00Z')")
                conn.commit()
            finally:
                conn.close()

            old_state = projects_module.DEFAULT_STATE_ROOT
            old_data = projects_module.DEFAULT_DATA_ROOT
            old_xdg = projects_module.LEGACY_XDG_DATA_ROOT
            old_legacy = projects_module.LEGACY_DATA_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(state_root)
                projects_module.DEFAULT_DATA_ROOT = str(default_data_root)
                projects_module.LEGACY_XDG_DATA_ROOT = str(xdg_root)
                projects_module.LEGACY_DATA_ROOT = str(Path(root) / ".local" / "share" / "satbooks")
                migrated = migrate_legacy_default_layout_if_needed()
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state
                projects_module.DEFAULT_DATA_ROOT = old_data
                projects_module.LEGACY_XDG_DATA_ROOT = old_xdg
                projects_module.LEGACY_DATA_ROOT = old_legacy

            self.assertEqual(migrated.id, "default")
            self.assertTrue((state_root / "projects" / "default" / "data" / DEFAULT_DB_FILENAME).exists())
            self.assertFalse(db_path.exists())
            backups = sorted(xdg_root.glob("pre-project-migration-*"))
            self.assertEqual(len(backups), 1)
            self.assertTrue((backups[0] / DEFAULT_DB_FILENAME).exists())

    def test_promoted_project_recovery_moves_legacy_artifacts_aside(self):
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            state_root = Path(root) / ".kassiber"
            data_root = state_root / "data"
            data_root.mkdir(parents=True)
            db_path = data_root / DEFAULT_DB_FILENAME
            conn = sqlite3.connect(db_path)
            try:
                conn.execute("CREATE TABLE workspaces(id TEXT PRIMARY KEY, label TEXT, created_at TEXT)")
                conn.execute("INSERT INTO workspaces VALUES('one', 'One', '2026-01-01T00:00:00Z')")
                conn.commit()
            finally:
                conn.close()
            config_root = state_root / "config"
            config_root.mkdir()
            (config_root / "backends.env").write_text("TOKEN=plaintext\n", encoding="utf-8")
            target_db = state_root / "projects" / "default" / "data" / DEFAULT_DB_FILENAME
            target_db.parent.mkdir(parents=True)
            target_conn = sqlite3.connect(target_db)
            try:
                target_conn.execute("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT)")
                target_conn.commit()
            finally:
                target_conn.close()

            old_state = projects_module.DEFAULT_STATE_ROOT
            old_data = projects_module.DEFAULT_DATA_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(state_root)
                projects_module.DEFAULT_DATA_ROOT = str(data_root)
                migrated = migrate_legacy_default_layout_if_needed()
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state
                projects_module.DEFAULT_DATA_ROOT = old_data

            self.assertEqual(migrated.id, "default")
            self.assertTrue(target_db.exists())
            self.assertFalse(db_path.exists())
            self.assertFalse((config_root / "backends.env").exists())
            backups = sorted(state_root.glob("pre-project-migration-*"))
            self.assertEqual(len(backups), 1)
            self.assertTrue((backups[0] / "data" / DEFAULT_DB_FILENAME).exists())
            self.assertTrue((backups[0] / "config" / "backends.env").exists())

    def test_stale_legacy_migrating_directory_is_retried(self):
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            state_root = Path(root) / ".kassiber"
            data_root = state_root / "data"
            data_root.mkdir(parents=True)
            db_path = data_root / DEFAULT_DB_FILENAME
            conn = sqlite3.connect(db_path)
            try:
                conn.execute("CREATE TABLE workspaces(id TEXT PRIMARY KEY, label TEXT, created_at TEXT)")
                conn.execute("INSERT INTO workspaces VALUES('one', 'One', '2026-01-01T00:00:00Z')")
                conn.commit()
            finally:
                conn.close()
            stale = state_root / "projects" / "default.migrating"
            stale.mkdir(parents=True)
            (stale / "partial.txt").write_text("interrupted", encoding="utf-8")

            old_state = projects_module.DEFAULT_STATE_ROOT
            old_data = projects_module.DEFAULT_DATA_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(state_root)
                projects_module.DEFAULT_DATA_ROOT = str(data_root)
                migrated = migrate_legacy_default_layout_if_needed()
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state
                projects_module.DEFAULT_DATA_ROOT = old_data

            self.assertEqual(migrated.id, "default")
            self.assertFalse(stale.exists())
            self.assertTrue((state_root / "projects" / "default" / "data" / DEFAULT_DB_FILENAME).exists())


class DaemonProjectSwitchTests(unittest.TestCase):
    def test_switch_keeps_current_project_if_target_open_fails(self):
        try:
            from kassiber import daemon as daemon_runtime
        except ModuleNotFoundError as exc:
            if exc.name == "embit":
                self.skipTest("embit is not installed")
            raise
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            state_root = Path(root) / ".kassiber"
            old_state = projects_module.DEFAULT_STATE_ROOT
            old_data = projects_module.DEFAULT_DATA_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(state_root)
                projects_module.DEFAULT_DATA_ROOT = str(state_root / "data")
                alpha = create_project("Alpha", project_id="alpha")
                beta = create_project("Beta", project_id="beta", select=False)
                beta.database.parent.mkdir(parents=True, exist_ok=True)
                beta_conn = sqlite3.connect(beta.database)
                try:
                    beta_conn.execute("CREATE TABLE not_kassiber(id TEXT)")
                    beta_conn.commit()
                finally:
                    beta_conn.close()

                alpha_conn = open_db(str(alpha.data_root))
                alpha_conn.execute(
                    "INSERT INTO settings(key, value) VALUES('project', 'alpha')"
                )
                alpha_conn.commit()

                ctx = daemon_runtime.DaemonContext(
                    conn=alpha_conn,
                    data_root=str(alpha.data_root),
                    runtime_config={
                        "env_file": str(alpha.root / "config" / "backends.env"),
                        "default_backend": None,
                        "backends": {},
                    },
                    active_ai_chats=daemon_runtime.ActiveAiChats(),
                    main_thread_tasks=queue.Queue(),
                    auth_backoff=daemon_runtime.AuthAttemptBackoff(None),
                    input_lines=queue.Queue(),
                    deferred_input_lines=[],
                    out=object(),
                    freshness_stop_event=threading.Event(),
                    project_id="alpha",
                    project_root=str(alpha.root),
                )

                with self.assertRaises(AppError) as raised:
                    daemon_runtime.handle_request(
                        ctx,
                        {
                            "kind": "ui.projects.select",
                            "request_id": "switch",
                            "args": {
                                "project_id": "beta",
                                "require_existing_project": True,
                            },
                        },
                        out=None,  # type: ignore[arg-type]
                    )

                self.assertEqual(raised.exception.code, "invalid_project_database")
                self.assertIs(ctx.conn, alpha_conn)
                self.assertEqual(ctx.project_id, "alpha")
                self.assertEqual(
                    alpha_conn.execute(
                        "SELECT value FROM settings WHERE key='project'"
                    ).fetchone()[0],
                    "alpha",
                )
                self.assertEqual(
                    load_catalog(catalog_path(state_root))["selected_project_id"],
                    "alpha",
                )
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state
                projects_module.DEFAULT_DATA_ROOT = old_data
                if "ctx" in locals() and ctx.conn is not None:
                    ctx.conn.close()

    def test_switch_keeps_current_project_open_until_next_passphrase_succeeds(self):
        if not sqlcipher_available():
            self.skipTest("SQLCipher driver is not installed")
        from kassiber import daemon as daemon_runtime
        from kassiber import projects as projects_module

        with tempfile.TemporaryDirectory() as root:
            state_root = Path(root) / ".kassiber"
            old_state = projects_module.DEFAULT_STATE_ROOT
            old_data = projects_module.DEFAULT_DATA_ROOT
            try:
                projects_module.DEFAULT_STATE_ROOT = str(state_root)
                projects_module.DEFAULT_DATA_ROOT = str(state_root / "data")
                alpha = create_project("Alpha", project_id="alpha")
                beta = create_project("Beta", project_id="beta", select=False)
                create_empty_encrypted_database(alpha.database, "alpha-passphrase-12345")
                create_empty_encrypted_database(beta.database, "beta-passphrase-12345")
                refresh_project_metadata(alpha.id, data_root=alpha.data_root)
                refresh_project_metadata(beta.id, data_root=beta.data_root)

                alpha_conn = open_db(str(alpha.data_root), passphrase="alpha-passphrase-12345")
                alpha_conn.execute("INSERT INTO settings(key, value) VALUES('project', 'alpha')")
                alpha_conn.commit()

                ctx = daemon_runtime.DaemonContext(
                    conn=alpha_conn,
                    data_root=str(alpha.data_root),
                    runtime_config={
                        "env_file": str(alpha.root / "config" / "backends.env"),
                        "default_backend": None,
                        "backends": {},
                    },
                    active_ai_chats=daemon_runtime.ActiveAiChats(),
                    main_thread_tasks=queue.Queue(),
                    auth_backoff=daemon_runtime.AuthAttemptBackoff(None),
                    input_lines=queue.Queue(),
                    deferred_input_lines=[],
                    out=object(),
                    freshness_stop_event=threading.Event(),
                    project_id="alpha",
                    project_root=str(alpha.root),
                )

                response, shutdown = daemon_runtime.handle_request(
                    ctx,
                    {
                        "kind": "ui.projects.select",
                        "request_id": "switch",
                        "args": {"project_id": "beta"},
                    },
                    out=None,  # type: ignore[arg-type]
                )
                self.assertFalse(shutdown)
                self.assertEqual(response["kind"], "auth_required")
                self.assertIs(ctx.conn, alpha_conn)
                self.assertEqual(ctx.project_id, "alpha")
                self.assertEqual(
                    alpha_conn.execute("SELECT value FROM settings WHERE key='project'").fetchone()[0],
                    "alpha",
                )
                self.assertEqual(
                    load_catalog(catalog_path(state_root))["selected_project_id"],
                    "alpha",
                )

                response, _ = daemon_runtime.handle_request(
                    ctx,
                    {
                        "kind": "ui.projects.select",
                        "request_id": "switch2",
                        "args": {
                            "project_id": "beta",
                            "auth_response": {"passphrase_secret": "beta-passphrase-12345"},
                        },
                    },
                    out=None,  # type: ignore[arg-type]
                )
                self.assertEqual(response["kind"], "ui.projects.select")
                self.assertIsNotNone(ctx.conn)
                self.assertEqual(ctx.project_id, "beta")
                with self.assertRaises(Exception) as closed:
                    alpha_conn.execute("SELECT 1")
                self.assertIn("closed database", str(closed.exception))
                self.assertEqual(
                    ctx.conn.execute("SELECT COUNT(*) FROM settings").fetchone()[0],
                    0,
                )
            finally:
                projects_module.DEFAULT_STATE_ROOT = old_state
                projects_module.DEFAULT_DATA_ROOT = old_data
                if "ctx" in locals() and ctx.conn is not None:
                    ctx.conn.close()


if __name__ == "__main__":
    unittest.main()
