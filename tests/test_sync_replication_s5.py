from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
import socket
import tempfile
import threading
import unittest
from unittest import mock

from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.sync_replication.bundle import build_bundle
from kassiber.core.sync_replication.gc import compact_tombstones, tombstone_gc_plan
from kassiber.core.sync_replication.identity import enable_sync
from kassiber.core.sync_replication.mailbox import pull_mailbox, push_mailbox
from kassiber.core.sync_replication.membership import (
    create_invitation,
    create_join_request,
    join_invitation,
)
from kassiber.core.sync_replication.merge import import_bundle
from kassiber.core.sync_replication.tor import TorOnionSyncServer, connect_onion
from kassiber.core.sync_replication.transports import configure_transport
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class SyncS5Base(unittest.TestCase):
    def setUp(self):
        self.owner_temp = tempfile.TemporaryDirectory()
        self.peer_temp = tempfile.TemporaryDirectory()
        self.owner_root = Path(self.owner_temp.name)
        self.peer_root = Path(self.peer_temp.name)
        self.owner = open_db(self.owner_root, passphrase="owner-passphrase")
        workspace = create_workspace(self.owner, "Org")
        profile = create_profile(
            self.owner, workspace["id"], "Book", "EUR", "FIFO", "generic", 365
        )
        self.workspace_id = workspace["id"]
        self.profile_id = profile["id"]
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
        joined = join_invitation(self.peer, request_id=request["request_id"], ciphertext=invitation)
        self.peer_replica_id = joined["replica_id"]
        initial = build_bundle(self.owner, profile_id=self.profile_id)
        import_bundle(self.peer, profile_id=self.profile_id, ciphertext=initial.ciphertext)
        self.owner.commit()
        self.peer.commit()

    def tearDown(self):
        self.owner.close()
        self.peer.close()
        self.owner_temp.cleanup()
        self.peer_temp.cleanup()


class TombstoneGcTests(SyncS5Base):
    def setUp(self):
        super().setUp()
        self.mailbox_temp = tempfile.TemporaryDirectory()
        self.mailbox_root = Path(self.mailbox_temp.name)
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
        self.mailbox_temp.cleanup()
        super().tearDown()

    def test_gc_waits_for_every_active_replica_ack_and_keeps_delete_fence(self):
        account_id = self.owner.execute(
            "SELECT id FROM accounts WHERE profile_id = ? LIMIT 1", (self.profile_id,)
        ).fetchone()[0]
        # Publish a peer edit before the deletion, but delay it at the owner.
        self.peer.execute("UPDATE accounts SET label = 'stale peer edit' WHERE id = ?", (account_id,))
        push_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        self.owner.execute("DELETE FROM accounts WHERE id = ?", (account_id,))
        push_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        self.owner.execute(
            "UPDATE sync_tombstones SET deleted_at = '2020-01-01T00:00:00Z' WHERE profile_id = ?",
            (self.profile_id,),
        )
        self.owner.execute(
            "UPDATE sync_replicas SET last_seen_at = '2020-01-01T00:00:00Z' WHERE id = ?",
            (self.peer_replica_id,),
        )
        blocked = tombstone_gc_plan(
            self.owner,
            profile_id=self.profile_id,
            horizon_days=180,
            now=datetime(2026, 7, 10, tzinfo=timezone.utc),
        )
        self.assertEqual(blocked["eligible"], 0)
        missing = blocked["items"][0]["missing_acknowledgements"]
        self.assertTrue(missing[0]["offline_past_horizon"])
        self.assertIn("re-invite", missing[0]["action"])

        # Peer ingests the delete and publishes a signed ack-only pointer.
        pull_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        push_mailbox(
            self.peer,
            profile_id=self.profile_id,
            transport_id=self.peer_transport["id"],
        )
        pull_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        eligible = tombstone_gc_plan(
            self.owner,
            profile_id=self.profile_id,
            horizon_days=180,
            now=datetime(2026, 7, 10, tzinfo=timezone.utc),
        )
        self.assertEqual(eligible["eligible"], 1)
        compacted = compact_tombstones(
            self.owner,
            profile_id=self.profile_id,
            horizon_days=180,
            dry_run=False,
            now=datetime(2026, 7, 10, tzinfo=timezone.utc),
        )
        self.assertEqual(compacted["compacted"], 1)
        self.assertEqual(self.owner.execute("SELECT COUNT(*) FROM sync_tombstones").fetchone()[0], 0)
        self.assertEqual(self.owner.execute("SELECT COUNT(*) FROM sync_tombstone_gc_log").fetchone()[0], 1)
        fence = self.owner.execute(
            "SELECT value_json FROM sync_field_state WHERE profile_id = ? AND entity_table = 'accounts' AND entity_key LIKE ? AND field = '__exists__'",
            (self.profile_id, f'%{account_id}%'),
        ).fetchone()
        self.assertEqual(fence[0], "false")

        # Replaying the pre-delete peer object remains harmless after compaction.
        pull_mailbox(
            self.owner,
            profile_id=self.profile_id,
            transport_id=self.owner_transport["id"],
        )
        self.assertEqual(
            self.owner.execute("SELECT COUNT(*) FROM accounts WHERE id = ?", (account_id,)).fetchone()[0],
            0,
        )


class TorOnionSyncTests(SyncS5Base):
    ONION_HOST = "a" * 56 + ".onion"

    @staticmethod
    def _free_port() -> int:
        probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        probe.bind(("127.0.0.1", 0))
        port = int(probe.getsockname()[1])
        probe.close()
        return port

    def test_onion_leg_reuses_pake_and_never_advertises_mdns(self):
        local_port = self._free_port()
        with mock.patch(
            "kassiber.core.sync_replication.lan.MdnsAdvertisement",
            side_effect=AssertionError("mDNS must stay off for Tor"),
        ):
            server = TorOnionSyncServer(
                self.owner,
                profile_id=self.profile_id,
                onion_host=self.ONION_HOST,
                onion_port=443,
                local_port=local_port,
            )
        output: dict = {}

        def serve():
            conn = open_db(self.owner_root, passphrase="owner-passphrase")
            try:
                output["result"] = server.serve_once(conn, timeout_seconds=5)
                conn.commit()
            except Exception as exc:
                output["error"] = exc
            finally:
                conn.close()

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        with mock.patch(
            "kassiber.core.sync_replication.tor.connect_via_socks5",
            side_effect=lambda _proxy, host, port, timeout: socket.create_connection(
                ("127.0.0.1", local_port), timeout=timeout
            ),
        ) as connector:
            result = connect_onion(
                self.peer,
                profile_id=self.profile_id,
                offer_code=server.offer.encode(),
                proxy_url="socks5h://127.0.0.1:9050",
                timeout_seconds=5,
            )
        thread.join(6)
        self.assertNotIn("error", output)
        self.assertEqual(result.peer_device_label, "Owner Mac")
        connector.assert_called_once_with(
            "socks5h://127.0.0.1:9050", self.ONION_HOST, 443, 5
        )

    def test_onion_leg_rejects_clearnet_and_missing_proxy(self):
        server = TorOnionSyncServer(
            self.owner,
            profile_id=self.profile_id,
            onion_host=self.ONION_HOST,
            onion_port=443,
            local_port=self._free_port(),
        )
        try:
            with self.assertRaisesRegex(AppError, "SOCKS5"):
                connect_onion(
                    self.peer,
                    profile_id=self.profile_id,
                    offer_code=server.offer.encode(),
                    proxy_url="",
                )
        finally:
            server.close()
