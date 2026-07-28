import tempfile
import unittest
from pathlib import Path

from kassiber.backends import (
    create_db_backend,
    preferred_explorer_base,
    preferred_mempool_api_backend,
)
from kassiber.db import open_db


class BackendPrivacyTest(unittest.TestCase):
    def test_public_explorer_url_drops_operational_userinfo(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(Path(root) / "data")
            self.addCleanup(conn.close)
            credential_url = "https://alice:" + "secret@example.test:8443/api"
            create_db_backend(
                conn,
                "private-mempool",
                "mempool",
                credential_url,
                chain="bitcoin",
                network="main",
            )

            public = preferred_explorer_base(conn)
            operational = preferred_mempool_api_backend(conn)

            self.assertEqual(public["base_url"], "https://example.test:8443")
            self.assertNotIn("alice", repr(public))
            self.assertNotIn("secret", repr(public))
            self.assertEqual(
                operational["api_base_url"],
                credential_url,
            )
