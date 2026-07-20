from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from kassiber.errors import AppError
from kassiber.operator.modes import (
    configured_unlock_mode,
    effective_unlock_mode,
    remembered_unlock_allowed,
    set_unlock_mode,
    unlock_mode_status,
)
from kassiber.operator.project import canonical_project
from kassiber.secrets.unlock_store import (
    enable_remembered_unlock_authenticated,
    set_cli_remembered_unlock_enabled,
)


TEST_DATABASE_IDENTITY = "1" * 32
TEST_ENROLLMENT_ID = "2" * 32


def _enable_legacy_marker(data_root) -> None:
    set_cli_remembered_unlock_enabled(
        data_root,
        True,
        database_identity=TEST_DATABASE_IDENTITY,
        enrollment_id=TEST_ENROLLMENT_ID,
    )


class OperatorModeTest(unittest.TestCase):
    def test_new_project_defaults_to_manual(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(configured_unlock_mode(tmp))
            self.assertEqual(effective_unlock_mode(tmp), "manual")
            self.assertFalse(remembered_unlock_allowed(tmp))

    def test_complete_remembered_enrollment_is_explicitly_unattended(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _enable_legacy_marker(tmp)
            self.assertEqual(effective_unlock_mode(tmp), "unattended")
            self.assertTrue(remembered_unlock_allowed(tmp))
            self.assertEqual(configured_unlock_mode(tmp), "unattended")
            self.assertFalse(unlock_mode_status(tmp)["legacy_inferred"])

    def test_explicit_brokered_mode_never_uses_remembered_store(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _enable_legacy_marker(tmp)
            set_unlock_mode(
                tmp,
                "brokered",
                database_identity=TEST_DATABASE_IDENTITY,
            )
            self.assertEqual(configured_unlock_mode(tmp), "brokered")
            self.assertFalse(remembered_unlock_allowed(tmp))

    def test_explicit_manual_mode_overrides_legacy_marker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _enable_legacy_marker(tmp)
            set_unlock_mode(tmp, "manual")
            self.assertEqual(effective_unlock_mode(tmp), "manual")
            self.assertFalse(remembered_unlock_allowed(tmp))

    @unittest.skipIf(os.name == "nt", "symlink creation is privilege-dependent")
    def test_mode_and_remembered_state_share_canonical_project_alias(self) -> None:
        with tempfile.TemporaryDirectory() as parent:
            project = Path(parent) / "project"
            project.mkdir()
            alias = Path(parent) / "alias"
            alias.symlink_to(project, target_is_directory=True)

            _enable_legacy_marker(alias)
            self.assertEqual(effective_unlock_mode(project), "unattended")

            set_unlock_mode(
                project,
                "brokered",
                database_identity=TEST_DATABASE_IDENTITY,
            )
            self.assertEqual(configured_unlock_mode(alias), "brokered")
            self.assertFalse(remembered_unlock_allowed(alias))

            set_unlock_mode(alias, "manual")
            self.assertEqual(configured_unlock_mode(project), "manual")

    def test_hardlink_aliases_cannot_select_divergent_unlock_policies(self) -> None:
        with tempfile.TemporaryDirectory() as parent:
            first = Path(parent) / "first"
            second = Path(parent) / "second"
            first.mkdir()
            second.mkdir()
            database = first / "kassiber.sqlite3"
            database.write_bytes(b"database")
            os.link(database, second / "kassiber.sqlite3")

            for data_root, mode in ((first, "brokered"), (second, "unattended")):
                with self.subTest(data_root=data_root, mode=mode):
                    with self.assertRaises(AppError) as raised:
                        set_unlock_mode(
                            data_root,
                            mode,
                            database_identity=TEST_DATABASE_IDENTITY,
                        )
                    self.assertEqual(raised.exception.code, "unsafe_project_database")
                    self.assertEqual(raised.exception.details, {"link_count": 2})

            for data_root in (first, second):
                with self.subTest(data_root=data_root, operation="remembered"):
                    with self.assertRaises(AppError) as raised:
                        set_cli_remembered_unlock_enabled(
                            data_root,
                            True,
                            database_identity=TEST_DATABASE_IDENTITY,
                            enrollment_id=TEST_ENROLLMENT_ID,
                        )
                    self.assertEqual(raised.exception.code, "unsafe_project_database")

    def test_moved_database_cannot_inherit_destination_unattended_policy(self) -> None:
        with tempfile.TemporaryDirectory() as parent:
            source = Path(parent) / "source"
            destination = Path(parent) / "destination"
            source.mkdir()
            destination.mkdir()
            source_database = source / "kassiber.sqlite3"
            destination_database = destination / "kassiber.sqlite3"
            source_database.write_bytes(b"source")
            destination_database.write_bytes(b"destination")
            source_identity = canonical_project(source).identity
            destination_identity = canonical_project(destination).identity

            set_unlock_mode(
                source,
                "brokered",
                database_identity="a" * 32,
                expected_project_identity=source_identity,
            )
            enable_remembered_unlock_authenticated(
                destination,
                database_identity="b" * 32,
                enrollment_id="c" * 32,
                expected_project_identity=destination_identity,
            )

            destination_database.unlink()
            source_database.replace(destination_database)

            status = unlock_mode_status(destination)
            self.assertEqual(status["configured"], "unattended")
            self.assertEqual(status["effective"], "manual")
            self.assertEqual(status["binding_state"], "mismatch")
            with self.assertRaises(AppError) as raised:
                effective_unlock_mode(destination)
            self.assertEqual(
                raised.exception.code,
                "operator_policy_binding_mismatch",
            )

    @mock.patch("kassiber.core.runtime.load_remembered_passphrase")
    def test_manual_runtime_does_not_read_credential_store(self, load) -> None:
        from kassiber.core.runtime import _open_db_with_resolved_passphrase
        from kassiber.errors import AppError

        with tempfile.TemporaryDirectory() as tmp:
            set_unlock_mode(tmp, "manual")
            with mock.patch("kassiber.core.runtime.open_db") as open_db:
                open_db.side_effect = AppError("locked", code="passphrase_required")
                with self.assertRaises(AppError):
                    _open_db_with_resolved_passphrase(
                        tmp,
                        None,
                        allow_prompt=False,
                    )
        load.assert_not_called()


if __name__ == "__main__":
    unittest.main()
