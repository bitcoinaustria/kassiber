from __future__ import annotations

import json
from pathlib import Path
import tempfile
import threading
import unittest

from kassiber.db import (
    ensure_settings_file,
    load_managed_settings,
    resolve_settings_path,
    update_managed_settings,
)


class ManagedSettingsTests(unittest.TestCase):
    def test_parallel_updates_preserve_every_writer_and_valid_json(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            settings_path = resolve_settings_path(data_root)
            failures: list[BaseException] = []
            start = threading.Barrier(9)

            def writer(index: int) -> None:
                try:
                    start.wait()
                    update_managed_settings(
                        data_root,
                        updates={f"agent_{index}": index},
                    )
                except BaseException as exc:  # pragma: no cover - assertion aid
                    failures.append(exc)

            threads = [threading.Thread(target=writer, args=(index,)) for index in range(8)]
            for thread in threads:
                thread.start()
            start.wait()
            for thread in threads:
                thread.join(timeout=5)

            self.assertFalse(failures)
            self.assertTrue(all(not thread.is_alive() for thread in threads))
            payload = json.loads(settings_path.read_text(encoding="utf-8"))
            for index in range(8):
                self.assertEqual(payload[f"agent_{index}"], index)

    def test_manifest_refresh_preserves_concurrent_non_secret_settings(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            update_managed_settings(data_root, updates={"cli_remembered_unlock": True})
            ensure_settings_file(data_root, Path(root) / "config" / "backends.env")

            payload = load_managed_settings(data_root)
            self.assertIs(payload["cli_remembered_unlock"], True)
            self.assertEqual(payload["app"], "kassiber")
            self.assertEqual(payload["schema_version"], 1)
            self.assertEqual(payload["paths"]["data_root"], str(data_root))


if __name__ == "__main__":
    unittest.main()
