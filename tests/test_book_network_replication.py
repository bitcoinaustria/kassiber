"""Signed peer replay cannot weaken the receiving book's immutable scope."""
import json
import unittest

from kassiber.core.book_network import apply_book_network, plan_book_network, resolve_book_environment
from kassiber.core.sync_replication.bundle import build_bundle
from kassiber.core.sync_replication.merge import import_bundle
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available
from tests import test_sync_replication_s2 as fixtures


@unittest.skipUnless(sqlcipher_available(), "SQLCipher unavailable")
class NetworkReplicationTests(unittest.TestCase):
    def setUp(self):
        self.book = fixtures.SyncBundleReplayTests()
        self.book.setUp()
        self.addCleanup(self.book.tearDown)

    def bind(self, connection, environment="main"):
        profile = self.book.profile["id"]
        args = {"environment": environment}
        plan = plan_book_network(connection, profile, args)
        return apply_book_network(connection, profile, {**args, "plan_id": plan["plan_id"]})

    def test_initial_signed_bundle_preserves_binding_and_exact_replay(self):
        binding = self.bind(self.book.owner)
        _, _, bundle = self.book._initial_sync()
        self.assertEqual(resolve_book_environment(self.book.peer, self.book.profile["id"]), binding)
        import_bundle(self.book.peer, profile_id=self.book.profile["id"], ciphertext=bundle.ciphertext)
        self.assertEqual(resolve_book_environment(self.book.peer, self.book.profile["id"]), binding)

    def test_conflicting_peer_binding_does_not_relabel_existing_book(self):
        self.book._initial_sync()
        self.bind(self.book.owner, "main")
        peer_binding = self.bind(self.book.peer, "test")
        bundle = build_bundle(self.book.owner, profile_id=self.book.profile["id"], attachments_root=self.book.attachments_a)
        with self.assertRaises(AppError):
            import_bundle(self.book.peer, profile_id=self.book.profile["id"], ciphertext=bundle.ciphertext)
        self.assertEqual(resolve_book_environment(self.book.peer, self.book.profile["id"]), peer_binding)
