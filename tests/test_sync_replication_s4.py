from __future__ import annotations

from dataclasses import replace
import tempfile
import threading
import unittest
from pathlib import Path
import unittest.mock as mock

from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.sync_replication.bundle import build_bundle
from kassiber.core.sync_replication.identity import enable_sync
from kassiber.core.sync_replication.lan import LanPairingOffer, LanSyncServer, connect_lan
from kassiber.core.sync_replication.membership import (
    create_invitation,
    create_join_request,
    join_invitation,
)
from kassiber.core.sync_replication.merge import import_bundle
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class LanSyncFastPathTests(unittest.TestCase):
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
        join_invitation(self.peer, request_id=request["request_id"], ciphertext=invitation)
        initial = build_bundle(self.owner, profile_id=self.profile_id)
        import_bundle(self.peer, profile_id=self.profile_id, ciphertext=initial.ciphertext)
        self.owner.commit()
        self.peer.commit()

    def tearDown(self):
        self.owner.close()
        self.peer.close()
        self.owner_temp.cleanup()
        self.peer_temp.cleanup()

    def _serve(self, server: LanSyncServer, output: dict):
        conn = open_db(self.owner_root, passphrase="owner-passphrase")
        try:
            output["result"] = server.serve_once(conn, timeout_seconds=5)
            conn.commit()
        except Exception as exc:  # test thread transports the exception to the assertion thread
            output["error"] = exc
        finally:
            conn.close()

    def test_two_devices_converge_over_pake_and_pinned_keys(self):
        self.owner.execute(
            "UPDATE workspaces SET label = 'Org from owner' WHERE id = ?",
            (self.workspace_id,),
        )
        self.peer.execute(
            "UPDATE profiles SET label = 'Book from editor' WHERE id = ?",
            (self.profile_id,),
        )
        self.owner.commit()
        self.peer.commit()
        server = LanSyncServer(
            self.owner,
            profile_id=self.profile_id,
            bind_host="127.0.0.1",
            advertise_mdns=False,
        )
        self.assertTrue(server.listening)
        self.assertNotIn(self.profile_id, server.offer.instance_name)
        output: dict = {}
        thread = threading.Thread(target=self._serve, args=(server, output), daemon=True)
        thread.start()
        client_result = connect_lan(
            self.peer,
            profile_id=self.profile_id,
            offer_code=server.offer.encode(),
            timeout_seconds=5,
        )
        self.peer.commit()
        thread.join(6)
        self.assertFalse(thread.is_alive())
        self.assertNotIn("error", output)
        self.assertGreaterEqual(client_result.applied_events, 1)
        self.assertGreaterEqual(output["result"].applied_events, 1)
        owner_check = open_db(self.owner_root, passphrase="owner-passphrase")
        try:
            self.assertEqual(
                owner_check.execute("SELECT label FROM workspaces WHERE id = ?", (self.workspace_id,)).fetchone()[0],
                "Org from owner",
            )
            self.assertEqual(
                owner_check.execute("SELECT label FROM profiles WHERE id = ?", (self.profile_id,)).fetchone()[0],
                "Book from editor",
            )
            self.assertEqual(
                self.peer.execute("SELECT label FROM workspaces WHERE id = ?", (self.workspace_id,)).fetchone()[0],
                "Org from owner",
            )
        finally:
            owner_check.close()
        self.assertFalse(server.listening)

    def test_offer_pin_change_is_refused_after_successful_pake(self):
        server = LanSyncServer(
            self.owner,
            profile_id=self.profile_id,
            bind_host="127.0.0.1",
            advertise_mdns=False,
        )
        tampered = replace(server.offer, server_device_pin="0" * 64)
        output: dict = {}
        thread = threading.Thread(target=self._serve, args=(server, output), daemon=True)
        thread.start()
        with self.assertRaisesRegex(AppError, "pin changed"):
            connect_lan(
                self.peer,
                profile_id=self.profile_id,
                offer_code=tampered.encode(),
                timeout_seconds=5,
            )
        thread.join(6)

    def test_wrong_short_code_fails_key_confirmation(self):
        server = LanSyncServer(
            self.owner,
            profile_id=self.profile_id,
            bind_host="127.0.0.1",
            advertise_mdns=False,
        )
        wrong = replace(server.offer, code="AAAA-BBBB-CCCC")
        output: dict = {}
        thread = threading.Thread(target=self._serve, args=(server, output), daemon=True)
        thread.start()
        with self.assertRaises(AppError):
            connect_lan(
                self.peer,
                profile_id=self.profile_id,
                offer_code=wrong.encode(),
                timeout_seconds=5,
            )
        thread.join(6)
        self.assertIn("error", output)

    def test_disabled_or_plaintext_book_never_binds_listener(self):
        disabled_temp = tempfile.TemporaryDirectory()
        encrypted = open_db(Path(disabled_temp.name), passphrase="encrypted")
        workspace = create_workspace(encrypted, "Disabled")
        profile = create_profile(
            encrypted, workspace["id"], "Book", "EUR", "FIFO", "generic", 365
        )
        try:
            with mock.patch.object(__import__("socket").socket, "bind", side_effect=AssertionError("bound")):
                with self.assertRaisesRegex(AppError, "disabled"):
                    LanSyncServer(
                        encrypted,
                        profile_id=profile["id"],
                        bind_host="127.0.0.1",
                        advertise_mdns=False,
                    )
        finally:
            encrypted.close()
            disabled_temp.cleanup()

        plaintext_temp = tempfile.TemporaryDirectory()
        plaintext = open_db(Path(plaintext_temp.name))
        workspace = create_workspace(plaintext, "Plain")
        profile = create_profile(
            plaintext, workspace["id"], "Book", "EUR", "FIFO", "generic", 365
        )
        try:
            with self.assertRaisesRegex(AppError, "unlocked encrypted"):
                LanSyncServer(
                    plaintext,
                    profile_id=profile["id"],
                    bind_host="127.0.0.1",
                    advertise_mdns=False,
                )
        finally:
            plaintext.close()
            plaintext_temp.cleanup()

    def test_pairing_offer_is_expiring_and_rotating(self):
        first = LanSyncServer(
            self.owner,
            profile_id=self.profile_id,
            bind_host="127.0.0.1",
            advertise_mdns=False,
        )
        second = LanSyncServer(
            self.owner,
            profile_id=self.profile_id,
            bind_host="127.0.0.1",
            advertise_mdns=False,
        )
        try:
            decoded = LanPairingOffer.decode(first.offer.encode())
            self.assertEqual(decoded, first.offer)
            self.assertNotEqual(first.offer.instance_name, second.offer.instance_name)
            self.assertRegex(first.offer.code, r"^[2-9A-HJ-NP-Z]{4}-[2-9A-HJ-NP-Z]{4}-[2-9A-HJ-NP-Z]{4}$")
        finally:
            first.close()
            second.close()
