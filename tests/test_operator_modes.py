from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from kassiber.operator.modes import (
    configured_unlock_mode,
    effective_unlock_mode,
    remembered_unlock_allowed,
    set_unlock_mode,
    unlock_mode_status,
)
from kassiber.secrets.unlock_store import set_cli_remembered_unlock_enabled


class OperatorModeTest(unittest.TestCase):
    def test_new_project_defaults_to_manual(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(configured_unlock_mode(tmp))
            self.assertEqual(effective_unlock_mode(tmp), "manual")
            self.assertFalse(remembered_unlock_allowed(tmp))

    def test_legacy_remembered_marker_is_inferred_as_unattended(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            set_cli_remembered_unlock_enabled(tmp, True)
            self.assertEqual(effective_unlock_mode(tmp), "unattended")
            self.assertTrue(remembered_unlock_allowed(tmp))
            self.assertTrue(unlock_mode_status(tmp)["legacy_inferred"])

    def test_explicit_brokered_mode_never_uses_remembered_store(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            set_cli_remembered_unlock_enabled(tmp, True)
            set_unlock_mode(tmp, "brokered")
            self.assertEqual(configured_unlock_mode(tmp), "brokered")
            self.assertFalse(remembered_unlock_allowed(tmp))

    def test_explicit_manual_mode_overrides_legacy_marker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            set_cli_remembered_unlock_enabled(tmp, True)
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

            set_cli_remembered_unlock_enabled(alias, True)
            self.assertEqual(effective_unlock_mode(project), "unattended")

            set_unlock_mode(project, "brokered")
            self.assertEqual(configured_unlock_mode(alias), "brokered")
            self.assertFalse(remembered_unlock_allowed(alias))

            set_unlock_mode(alias, "manual")
            self.assertEqual(configured_unlock_mode(project), "manual")

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
