from __future__ import annotations

from io import BytesIO
import base64
import json
from pathlib import Path
import socket
import tempfile
import unittest
from unittest import mock

from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.sync_replication.identity import enable_sync
from kassiber.core.sync_replication.mailbox import (
    mailbox_head_key,
    mailbox_status,
    pull_mailbox,
    push_mailbox,
)
from kassiber.core.sync_replication.membership import (
    create_invitation,
    create_join_request,
    join_invitation,
)
from kassiber.core.sync_replication.transports import (
    FolderTransport,
    S3Transport,
    WebDavTransport,
    configure_transport,
)
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available
from kassiber.ai.tools import TOOL_CATALOG
from kassiber.daemon_sync_replication import SYNC_UI_KINDS


class _Response(BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()


class TransportAdapterTests(unittest.TestCase):
    def test_sync_desktop_kinds_are_never_exposed_as_ai_tools(self):
        daemon_kinds = {tool.daemon_kind for tool in TOOL_CATALOG}
        self.assertFalse(set(SYNC_UI_KINDS) & daemon_kinds)

    def test_folder_transport_is_atomic_idempotent_and_path_safe(self):
        with tempfile.TemporaryDirectory() as temporary:
            transport = FolderTransport(Path(temporary))
            transport.put("book/replica/one.age", b"ciphertext", if_absent=True)
            transport.put("book/replica/one.age", b"ciphertext", if_absent=True)
            self.assertEqual(transport.get("book/replica/one.age"), b"ciphertext")
            self.assertEqual(transport.list("book"), ["book/replica/one.age"])
            with self.assertRaises(AppError):
                transport.put("../escape", b"bad")
            with self.assertRaises(AppError):
                transport.put("book/replica/one.age", b"fork", if_absent=True)

    def test_webdav_uses_basic_auth_and_append_only_precondition(self):
        requests = []

        def opener(request, timeout):
            requests.append((request, timeout))
            if request.method == "PROPFIND":
                return _Response(
                    b'<?xml version="1.0"?><multistatus xmlns="DAV:">'
                    b'<response><href>/mail/book/one.age</href></response></multistatus>'
                )
            return _Response(b"payload")

        transport = WebDavTransport(
            base_url="https://storage.example/mail/",
            username="alice",
            password="secret",
            opener=opener,
        )
        transport.put("book/one.age", b"sealed", if_absent=True)
        self.assertEqual(transport.get("book/one.age"), b"payload")
        self.assertEqual(transport.list("book"), ["book/one.age"])
        put = next(request for request, _ in requests if request.method == "PUT")
        self.assertEqual(put.headers["If-none-match"], "*")
        self.assertTrue(put.headers["Authorization"].startswith("Basic "))
        self.assertNotIn("secret", put.full_url)

    def test_s3_signs_requests_without_leaking_secret(self):
        requests = []

        def opener(request, timeout):
            requests.append((request, timeout))
            if request.method == "GET" and "list-type=2" in request.full_url:
                return _Response(
                    b'<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
                    b'<IsTruncated>false</IsTruncated><Contents><Key>prefix/book/one.age</Key></Contents>'
                    b'</ListBucketResult>'
                )
            return _Response(b"sealed")

        transport = S3Transport(
            endpoint="https://s3.example",
            bucket="bucket",
            region="eu-central-1",
            prefix="prefix",
            access_key="ACCESS",
            secret_key="TOP-SECRET",
            opener=opener,
        )
        transport.put("book/one.age", b"sealed", if_absent=True)
        self.assertEqual(transport.list("book"), ["book/one.age"])
        request = requests[0][0]
        self.assertIn("AWS4-HMAC-SHA256", request.headers["Authorization"])
        self.assertNotIn("TOP-SECRET", request.headers["Authorization"])
        self.assertNotIn("TOP-SECRET", request.full_url)

    def test_plaintext_profile_cannot_store_transport_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            conn = open_db(Path(temporary))
            try:
                workspace = create_workspace(conn, "Plain")
                profile = create_profile(
                    conn, workspace["id"], "Book", "EUR", "FIFO", "generic", 365
                )
                with self.assertRaisesRegex(AppError, "enable sync"):
                    configure_transport(
                        conn,
                        profile_id=profile["id"],
                        kind="webdav",
                        label="Unsafe",
                        config={"url": "https://example.test/dav"},
                        credentials={"username": "alice", "password": "secret"},
                    )
                self.assertEqual(conn.execute("SELECT COUNT(*) FROM sync_transports").fetchone()[0], 0)
            finally:
                conn.close()


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class MailboxEndToEndTests(unittest.TestCase):
    def setUp(self):
        self.owner_temp = tempfile.TemporaryDirectory()
        self.peer_temp = tempfile.TemporaryDirectory()
        self.mailbox_temp = tempfile.TemporaryDirectory()
        self.owner_root = Path(self.owner_temp.name)
        self.peer_root = Path(self.peer_temp.name)
        self.mailbox_root = Path(self.mailbox_temp.name)
        self.owner = open_db(self.owner_root, passphrase="owner-passphrase")
        workspace = create_workspace(self.owner, "Org")
        profile = create_profile(
            self.owner, workspace["id"], "Book", "EUR", "FIFO", "generic", 365
        )
        self.profile_id = profile["id"]
        self.workspace_id = workspace["id"]
        enable_sync(
            self.owner,
            workspace_id=self.workspace_id,
            profile_id=self.profile_id,
            member_name="Owner",
            device_label="Owner Mac",
        )
        self.peer = open_db(self.peer_root, passphrase="peer-passphrase")
        request = create_join_request(self.peer, member_name="Editor", device_label="Peer Mac")
        invitation = create_invitation(
            self.owner,
            profile_id=self.profile_id,
            join_request=request,
            role="editor",
        )
        self.initial_invitation_qr_bytes = len(base64.b64encode(invitation))
        join_invitation(self.peer, request_id=request["request_id"], ciphertext=invitation)
        self.owner_transport = configure_transport(
            self.owner,
            profile_id=self.profile_id,
            kind="folder",
            label="Shared",
            config={"path": str(self.mailbox_root)},
        )
        self.peer_transport = configure_transport(
            self.peer,
            profile_id=self.profile_id,
            kind="folder",
            label="Shared",
            config={"path": str(self.mailbox_root)},
        )

    def tearDown(self):
        self.owner.close()
        self.peer.close()
        self.owner_temp.cleanup()
        self.peer_temp.cleanup()
        self.mailbox_temp.cleanup()

    def test_dumb_folder_mailbox_converges_without_listener_and_uses_opaque_paths(self):
        with mock.patch.object(socket.socket, "listen", side_effect=AssertionError("listener opened")):
            pushed = push_mailbox(
                self.owner,
                profile_id=self.profile_id,
                transport_id=self.owner_transport["id"],
            )
            pulled = pull_mailbox(
                self.peer,
                profile_id=self.profile_id,
                transport_id=self.peer_transport["id"],
            )
        self.assertFalse(pushed.up_to_date)
        self.assertGreater(pulled.applied_events, 0)
        paths = [path.relative_to(self.mailbox_root).as_posix() for path in self.mailbox_root.rglob("*")]
        joined_paths = "\n".join(paths)
        self.assertNotIn(self.profile_id, joined_paths)
        self.assertNotIn(self.workspace_id, joined_paths)
        self.assertNotIn("Owner", joined_paths)

        self.peer.execute("UPDATE profiles SET label = 'Edited remotely' WHERE id = ?", (self.profile_id,))
        push_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        result = pull_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        self.assertGreater(result.applied_events, 0)
        self.assertEqual(
            self.owner.execute("SELECT label FROM profiles WHERE id = ?", (self.profile_id,)).fetchone()[0],
            "Edited remotely",
        )
        status = mailbox_status(self.owner, profile_id=self.profile_id)
        self.assertEqual(status["transports"][0]["peers"][0]["status"], "fresh")

    def test_baseline_invitation_fits_one_local_qr(self):
        # Long invitations switch to low error correction (2,953-byte byte
        # mode capacity). Larger organizations retain the code/file fallback.
        self.assertLessEqual(self.initial_invitation_qr_bytes, 2953)

    def test_tampered_signed_head_is_rejected(self):
        push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        book = self.owner.execute("SELECT * FROM sync_books WHERE profile_id = ?", (self.profile_id,)).fetchone()
        head_path = self.mailbox_root / mailbox_head_key(book, book["local_replica_id"])
        document = json.loads(head_path.read_text())
        document["last_seq"] += 1
        head_path.write_text(json.dumps(document))
        with self.assertRaisesRegex(AppError, "head hash"):
            pull_mailbox(
                self.peer,
                profile_id=self.profile_id,
                transport_id=self.peer_transport["id"],
            )

    def test_owner_snapshot_bootstraps_new_recipient_past_old_ciphertext(self):
        # This setup already invited the peer, so create a third device only
        # after an incremental bundle has been sealed to the first two.
        push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        third_temp = tempfile.TemporaryDirectory()
        third = open_db(Path(third_temp.name), passphrase="third-passphrase")
        try:
            request = create_join_request(
                third, member_name="Late editor", device_label="Late Mac"
            )
            invitation = create_invitation(
                self.owner,
                profile_id=self.profile_id,
                join_request=request,
                role="editor",
            )
            join_invitation(third, request_id=request["request_id"], ciphertext=invitation)
            third_transport = configure_transport(
                third,
                profile_id=self.profile_id,
                kind="folder",
                label="Shared",
                config={"path": str(self.mailbox_root)},
            )
            self.owner.execute(
                "UPDATE profiles SET label = 'Snapshot state' WHERE id = ?",
                (self.profile_id,),
            )
            snapshot = push_mailbox(
                self.owner,
                profile_id=self.profile_id,
                transport_id=self.owner_transport["id"],
                snapshot=True,
            )
            self.assertIn("/snapshot-", snapshot.object_key)
            pulled = pull_mailbox(
                third,
                profile_id=self.profile_id,
                transport_id=third_transport["id"],
            )
            self.assertGreater(pulled.applied_events, 0)
            self.assertEqual(
                third.execute(
                    "SELECT label FROM profiles WHERE id = ?", (self.profile_id,)
                ).fetchone()[0],
                "Snapshot state",
            )
            notice = third.execute(
                "SELECT code FROM sync_notices WHERE profile_id = ? AND code = 'sync_snapshot_bootstrap'",
                (self.profile_id,),
            ).fetchone()
            self.assertIsNotNone(notice)
        finally:
            third.close()
            third_temp.cleanup()

    def test_transport_configuration_never_returns_credentials(self):
        configured = configure_transport(
            self.owner,
            profile_id=self.profile_id,
            kind="webdav",
            label="NAS",
            config={"url": "https://nas.example/private/path"},
            credentials={"username": "alice", "password": "secret"},
        )
        serialized = json.dumps(configured, sort_keys=True)
        self.assertNotIn("secret", serialized)
        self.assertNotIn("alice", serialized)
        self.assertNotIn("private/path", serialized)
        self.assertTrue(configured["credentials_configured"])
