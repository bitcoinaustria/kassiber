"""Managed attachment opens must obey the same containment rule as deletion."""
from pathlib import Path
import tempfile
import unittest

from kassiber.daemon import _ui_attachment_payload_from_conn
from kassiber.db import open_db
from kassiber.errors import AppError


class AttachmentOpenScopeTest(unittest.TestCase):
    def test_open_refuses_parent_and_symlink_escapes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "data"
            conn = open_db(str(data))
            conn.executescript("""
                INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z');
                INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','Book','2025-01-01T00:00:00Z');
                INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');
                INSERT INTO attachments(id,workspace_id,profile_id,attachment_type,label,stored_relpath,created_at)
                VALUES('a','w','p','file','Evidence','../outside.txt','2025-01-01T00:00:00Z');
            """)
            outside = root / "outside.txt"
            outside.write_text("synthetic unrelated file", encoding="utf-8")
            managed = root / "attachments"
            managed.mkdir(exist_ok=True)
            (managed / "escape").symlink_to(root, target_is_directory=True)
            try:
                for relative in ("../outside.txt", "escape/outside.txt"):
                    with self.subTest(relative=relative):
                        conn.execute("UPDATE attachments SET stored_relpath=? WHERE id='a'", (relative,))
                        conn.commit()
                        with self.assertRaises(AppError) as raised:
                            _ui_attachment_payload_from_conn(
                                conn, str(data), "ui.attachments.open", {"attachment": "a"}
                            )
                        self.assertEqual(raised.exception.code, "validation")
                        self.assertEqual(outside.read_text(), "synthetic unrelated file")
            finally:
                conn.close()
