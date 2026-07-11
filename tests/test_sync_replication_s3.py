from __future__ import annotations

from io import BytesIO
import base64
import json
from pathlib import Path
import socket
import tempfile
import unittest
import unittest.mock as mock
from urllib.error import HTTPError

from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.sync_replication.identity import enable_sync
from kassiber.core.sync_replication.mailbox import (
    _sign_head,
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
from kassiber.core.sync_replication import membership as membership_module
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

    def test_webdav_rejects_cleartext_credentials_off_loopback(self):
        with self.assertRaisesRegex(AppError, "require HTTPS"):
            WebDavTransport(
                base_url="http://storage.example/mail/",
                username="alice",
                password="secret",
            )

    def test_webdav_list_decodes_keys_once_and_rejects_encoded_separators(self):
        requests = []

        def opener(request, timeout):
            requests.append(request)
            if request.method == "PROPFIND":
                return _Response(
                    b'<?xml version="1.0"?><multistatus xmlns="DAV:">'
                    b'<response><href>/mail/book/a%20b</href></response>'
                    b'<response><href>/mail/book/caf%C3%A9</href></response>'
                    b'<response><href>/mail/book/percent%25value</href></response>'
                    b'</multistatus>'
                )
            return _Response(b"payload")

        transport = WebDavTransport(base_url="https://storage.example/mail/", opener=opener)
        keys = transport.list("book")
        self.assertEqual(keys, ["book/a b", "book/café", "book/percent%value"])
        for key in keys:
            transport.get(key)
        get_urls = [request.full_url for request in requests if request.method == "GET"]
        self.assertTrue(any(url.endswith("/book/a%20b") for url in get_urls))
        self.assertFalse(any("%2520" in url for url in get_urls))

        def unsafe_listing(request, timeout):
            return _Response(
                b'<?xml version="1.0"?><multistatus xmlns="DAV:">'
                b'<response><href>/mail/book/a%2Fb</href></response></multistatus>'
            )

        unsafe = WebDavTransport(base_url="https://storage.example/mail/", opener=unsafe_listing)
        with self.assertRaisesRegex(AppError, "unsafe"):
            unsafe.list("book")

    def test_webdav_retries_429_with_retry_after_and_bounds_503(self):
        attempts = []
        delays = []

        def recovers(request, timeout):
            attempts.append(request)
            if len(attempts) == 1:
                raise HTTPError(
                    request.full_url,
                    429,
                    "rate limited",
                    {"Retry-After": "2"},
                    None,
                )
            return _Response(b"sealed")

        transport = WebDavTransport(
            base_url="https://storage.example/mail/",
            opener=recovers,
            sleeper=delays.append,
        )
        self.assertEqual(transport.get("book/one.age"), b"sealed")
        self.assertEqual(len(attempts), 2)
        self.assertEqual(delays, [2.0])

        failures = []

        def unavailable(request, timeout):
            failures.append(request)
            raise HTTPError(request.full_url, 503, "unavailable", {}, None)

        transport = WebDavTransport(
            base_url="https://storage.example/mail/",
            opener=unavailable,
            sleeper=lambda _delay: None,
        )
        with self.assertRaises(AppError) as raised:
            transport.get("book/one.age")
        self.assertTrue(raised.exception.retryable)
        self.assertEqual(len(failures), 3)

    def test_webdav_timeout_is_retryable_without_hidden_retry_loop(self):
        attempts = []

        def timeout(request, timeout):
            attempts.append(request)
            raise TimeoutError("timed out")

        transport = WebDavTransport(
            base_url="https://storage.example/mail/",
            opener=timeout,
            sleeper=lambda _delay: None,
        )
        with self.assertRaises(AppError) as raised:
            transport.get("book/one.age")
        self.assertTrue(raised.exception.retryable)
        self.assertEqual(len(attempts), 1)

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

    def test_s3_requires_https_except_on_loopback(self):
        common = {
            "bucket": "bucket",
            "region": "eu-central-1",
            "access_key": "ACCESS",
            "secret_key": "SECRET",
        }
        with self.assertRaisesRegex(AppError, "require HTTPS") as raised:
            S3Transport(endpoint="http://s3.example", **common)
        self.assertEqual(raised.exception.code, "sync_transport_insecure")
        S3Transport(endpoint="https://s3.example", **common)
        S3Transport(endpoint="http://127.0.0.1:9000", **common)
        with self.assertRaisesRegex(AppError, "cannot use a proxy") as proxied:
            S3Transport(
                endpoint="http://127.0.0.1:9000",
                proxy_url="http://proxy.example:8080",
                **common,
            )
        self.assertEqual(proxied.exception.code, "sync_transport_insecure")

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

    def test_replayed_head_observation_does_not_hide_staleness(self):
        push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        pull_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        self.peer.execute(
            """
            UPDATE sync_peer_status
            SET last_seen_at = '2099-01-01T00:00:00Z',
                last_bundle_at = '2000-01-01T00:00:00Z'
            WHERE profile_id = ? AND transport_id = ?
              AND replica_id != (SELECT local_replica_id FROM sync_books WHERE profile_id = ?)
            """,
            (self.profile_id, self.peer_transport["id"], self.profile_id),
        )
        status = mailbox_status(
            self.peer,
            profile_id=self.profile_id,
            stale_after_seconds=1,
        )
        peer = status["transports"][0]["peers"][0]
        self.assertEqual(peer["status"], "stale")
        self.assertEqual(peer["last_seen_at"], "2099-01-01T00:00:00Z")

    def test_partial_push_is_retryable_and_does_not_advance_local_head(self):
        class PartialPushTransport:
            def __init__(self):
                self.objects = {}
                self.fail_head_once = True

            def put(self, key, payload, *, if_absent=False):
                if key.endswith("/head.json") and self.fail_head_once:
                    self.fail_head_once = False
                    raise AppError(
                        "injected partial push",
                        code="sync_transport_unavailable",
                        retryable=True,
                    )
                if if_absent and key in self.objects and self.objects[key] != payload:
                    raise AppError("collision", code="sync_mailbox_collision")
                self.objects[key] = payload

            def get(self, key):
                return self.objects[key]

            def list(self, prefix):
                return sorted(key for key in self.objects if key.startswith(prefix))

            def exists(self, key):
                return key in self.objects

        transport = PartialPushTransport()
        with self.assertRaisesRegex(AppError, "injected partial push"):
            push_mailbox(
                self.owner,
                profile_id=self.profile_id,
                transport_id=self.owner_transport["id"],
                transport_override=transport,
            )
        self.assertEqual(
            self.owner.execute("SELECT COUNT(*) FROM sync_mailbox_heads").fetchone()[0],
            0,
        )
        self.assertIsNone(
            self.owner.execute(
                "SELECT last_push_at FROM sync_transports WHERE id = ?",
                (self.owner_transport["id"],),
            ).fetchone()[0]
        )

        pushed = push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
            transport_override=transport,
        )
        self.assertFalse(pushed.up_to_date)
        self.assertEqual(
            self.owner.execute("SELECT COUNT(*) FROM sync_mailbox_heads").fetchone()[0],
            1,
        )

    def test_replayed_older_valid_head_surfaces_rollback_notice(self):
        first = push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        first_head = (self.mailbox_root / first.head_key).read_bytes()
        pull_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        self.owner.execute(
            "UPDATE profiles SET label = 'Second head' WHERE id = ?",
            (self.profile_id,),
        )
        second = push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        pull_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        (self.mailbox_root / second.head_key).write_bytes(first_head)
        pull_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        notice = self.peer.execute(
            "SELECT severity FROM sync_notices WHERE code = 'sync_mailbox_rollback'"
        ).fetchone()
        self.assertIsNotNone(notice)
        self.assertEqual(notice["severity"], "blocking")

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

    def test_signed_head_rejects_malformed_sequence_types_with_typed_error(self):
        pushed = push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        book = self.owner.execute(
            "SELECT * FROM sync_books WHERE profile_id = ?", (self.profile_id,)
        ).fetchone()
        head_path = self.mailbox_root / pushed.head_key
        original = json.loads(head_path.read_text())
        for malformed in ({"nested": 1}, True, None, -1, "1"):
            with self.subTest(malformed=malformed):
                core = {
                    key: value
                    for key, value in original.items()
                    if key not in {"head_hash", "signature"}
                }
                core["first_seq"] = malformed
                head_path.write_bytes(_sign_head(self.owner, book=book, core=core))
                with self.assertRaises(AppError) as raised:
                    pull_mailbox(
                        self.peer,
                        profile_id=self.profile_id,
                        transport_id=self.peer_transport["id"],
                    )
                self.assertEqual(raised.exception.code, "sync_mailbox_head_invalid")

        for created_at in (
            "0001-01-01T00:00:00+23:59",
            "9999-12-31T23:59:59-23:59",
        ):
            with self.subTest(created_at=created_at):
                core = {
                    key: value
                    for key, value in original.items()
                    if key not in {"head_hash", "signature"}
                }
                core["created_at"] = created_at
                head_path.write_bytes(_sign_head(self.owner, book=book, core=core))
                with self.assertRaises(AppError) as raised:
                    pull_mailbox(
                        self.peer,
                        profile_id=self.profile_id,
                        transport_id=self.peer_transport["id"],
                    )
                self.assertEqual(raised.exception.code, "sync_mailbox_head_invalid")

        core = {
            key: value
            for key, value in original.items()
            if key not in {"head_hash", "signature"}
        }
        core["first_seq"] = 2**63
        core["last_seq"] = 2**63
        head_path.write_bytes(_sign_head(self.owner, book=book, core=core))
        with self.assertRaises(AppError) as raised:
            pull_mailbox(
                self.peer,
                profile_id=self.profile_id,
                transport_id=self.peer_transport["id"],
            )
        self.assertEqual(raised.exception.code, "sync_mailbox_head_invalid")

    def test_invitation_decryption_output_and_ciphertext_are_bounded(self):
        request = create_join_request(
            self.peer, member_name="Second editor", device_label="Second peer"
        )

        def oversized_plaintext(_source, destination, **_kwargs):
            destination.write(b"x" * (membership_module._MAX_INVITATION_ENVELOPE_BYTES + 1))

        with mock.patch(
            "kassiber.core.sync_replication.membership.decrypt_age_stream",
            side_effect=oversized_plaintext,
        ):
            with self.assertRaises(AppError) as raised:
                join_invitation(
                    self.peer,
                    request_id=request["request_id"],
                    ciphertext=b"bounded-ciphertext",
                )
        self.assertEqual(raised.exception.code, "sync_invitation_invalid")

        with self.assertRaises(AppError) as raised:
            join_invitation(
                self.peer,
                request_id=request["request_id"],
                ciphertext=b"x" * (membership_module._MAX_INVITATION_CIPHERTEXT_BYTES + 1),
            )
        self.assertEqual(raised.exception.code, "sync_invitation_invalid")

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
