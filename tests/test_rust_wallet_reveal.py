"""Descriptor reveal: Python's fresh-authentication guard and pasteable
material compared byte for byte with the Rust daemon."""
import json
import unittest

from kassiber.ai.tools import TOOL_CATALOG
from kassiber.db import open_db
from tests.rust_desktop_parity import DESKTOP_SQL, RUST_BIN
from tests.test_rust_wallet_onboarding import MASTER, UnlockingParity, XPUB

ACK = {"plaintext_reveal_ack": "COPY LOCAL SECRET"}


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustDescriptorRevealTest(UnlockingParity, unittest.TestCase):
    KIND = "wallets.reveal_descriptor"

    def seed(self):
        configs = [
            ("hot", "Hot", "descriptor", {"descriptor": f" wpkh({XPUB}/0/*) ", "change_descriptor": f"wpkh({XPUB}/1/*)",
                                          "blinding_key": "synthetic-not-revealed", "backend": "fulcrum"}),
            ("xp", "Xpub only", "xpub", {"xpub": MASTER, "script_types": ["p2wpkh"]}),
            ("sp", "Silent", "silent-payment", {"sp_descriptor": "sp(spscan1qsynthetic)", "descriptor": 0}),
        ]
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(DESKTOP_SQL.read_text())
            for wallet_id, label, kind, config in configs:
                conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                             "VALUES(?,?,?,?,?,?,?)", (wallet_id, "w", "p", label, kind, json.dumps(config), "2025"))
        self.change("")

    def test_plaintext_acknowledgement_and_material_match_python(self):
        self.compare([
            ..., None, [], {}, {"name": 3}, {"name": ""}, {"wallet": " ", "backend": "hot"},
            {"name": "hot"}, {"name": "hot", "auth_response": []}, {"name": "hot", "auth_response": {}},
            {"name": "hot", "auth_response": {"plaintext_reveal_ack": "copy local secret"}},
            {"name": "hot", "auth_response": ACK}, {"wallet": "HOT", "auth_response": ACK},
            {"backend": "xp", "auth_response": ACK}, {"name": "sp", "auth_response": ACK},
            {"name": "absent", "auth_response": ACK}, {"name": "cold", "auth_response": ACK},
            {"name": "hot", "workspace": "w", "profile": "p", "auth_response": ACK},
            {"name": "hot", "workspace": "w", "profile": "foreign", "auth_response": ACK},
            {"name": "hot", "profile": "nope", "auth_response": ACK},
        ])

    def test_encrypted_books_require_a_fresh_passphrase(self):
        self.encrypt("synthetic reveal key")
        wrong = {"name": "hot", "auth_response": {"passphrase_secret": "wrong"}}
        right = {"name": "hot", "auth_response": {"passphrase_secret": "synthetic reveal key"}}
        responses = self.compare([{"name": "hot"}, {"name": "hot", "auth_response": ACK},
                                  {"name": "hot", "auth_response": {"passphrase_secret": ""}}, wrong, right])
        kinds = [response["kind"] for response in responses]
        self.assertEqual(kinds, ["daemon.unlock", "auth_required", "auth_required", "auth_required", "error",
                                 self.KIND])
        self.assertEqual(responses[1]["data"]["scope"], "reveal_descriptor")
        self.assertNotIn("synthetic-not-revealed", json.dumps(responses))

    def test_no_ai_tool_reaches_the_reveal(self):
        self.assertNotIn(self.KIND, {getattr(tool, "daemon_kind", None) for tool in TOOL_CATALOG})
