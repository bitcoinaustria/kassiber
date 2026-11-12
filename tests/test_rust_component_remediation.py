"""Custody component revision and remediation through Rust (main sync S11).

Blocking component errors name what resolves them (`details.remediation`):
a membership conflict points at the conflicting component, a second draft at
the existing one, a batch anchoring one transaction twice at the document.
Legs take qualified transaction references. Each session runs the Python
daemon on the book and the Rust daemon on a copy taken just before, and
compares the bytes and all 113 tables, plaintext and SQLCipher.
"""
import sqlite3
import unittest

from kassiber.secrets.sqlcipher import open_encrypted
from tests.rust_desktop_parity import RUST_BIN
from tests.rust_port.sync_parity import SyncParity

NOW = "2026-01-01T00:00:00Z"
SHARED = "d" * 64
FRESH = "e" * 64
SQL = [
    f"INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','{NOW}')",
    f"INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book','{NOW}')",
    "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) "
    f"VALUES('treasury','ws','profile','treasury','Treasury','asset','{NOW}')",
    *(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
        f"VALUES('{wallet}','ws','profile','{label}','custom','{{}}','{NOW}')"
        for wallet, label in (("source-wallet", "Source"), ("sink-wallet", "Sink"))
    ),
    *(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
        f"occurred_at,direction,asset,amount,fee,created_at) VALUES('{tx}','ws','profile','{wallet}',"
        f"'fingerprint-{tx}','{external}','{NOW}','{direction}','BTC',{amount},0,'{NOW}')"
        for tx, wallet, external, direction, amount in (
            ("out", "source-wallet", SHARED, "outbound", 100_000),
            ("in", "sink-wallet", SHARED, "inbound", 99_000),
            ("out2", "source-wallet", FRESH, "outbound", 100_000),
            ("in2", "sink-wallet", FRESH, "inbound", 99_000),
        )
    ),
]


def spec(note, out=f"{SHARED}:out", inbound=f"{SHARED}:Sink"):
    return {
        "component_type": "manual_bridge",
        "evidence_kind": "manual_claim",
        "evidence_grade": "reviewed",
        "notes": note,
        "legs": [
            {"role": "source", "transaction": out, "amount_msat": 100_000},
            {"role": "destination", "transaction": inbound, "amount_msat": 99_000},
            {"role": "fee", "transaction": out, "amount_msat": 1_000},
        ],
    }


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class ComponentRemediationParity(SyncParity, unittest.TestCase):
    SQL = SQL

    def read(self, query):
        path = self.root / "kassiber.sqlite3"
        conn = open_encrypted(path, self.passphrase) if self.passphrase else sqlite3.connect(path)
        try:
            return conn.execute(query).fetchall()
        finally:
            conn.close()

    def version(self):
        return self.read("SELECT journal_input_version FROM profiles WHERE id='profile'")[0][0]

    def apply(self, args):
        """Plan, then apply against the plan's input version, each compared."""
        plan = self.masked([args], kind="ui.transfers.components.plan")[0]
        self.assertEqual(plan["kind"], "ui.transfers.components.plan", plan)
        version = plan["data"]["input_version"]
        applied = self.masked([dict(args, expected_input_version=version)],
                              kind="ui.transfers.components.apply")[0]
        self.assertEqual(applied["kind"], "ui.transfers.components.apply", applied)
        return applied["data"]

    def test_blocking_errors_carry_remediation_like_python(self):
        existing = self.apply({"components": [spec("user interpretation")], "activate": True})
        existing_id = existing["components"][0]["id"]

        # A second interpretation of the same anchors names the conflicting one.
        conflict = self.masked(
            [{"components": [spec("second attempt")], "activate": True},
             {"components": [spec("a", out="out2", inbound="in2"),
                             spec("b", out=f"{FRESH}:out", inbound=f"{FRESH}:in")],
              "activate": True},
             {"components": [spec("bad ref", out=SHARED)], "activate": True},
             {"components": [spec("bad qualifier", out=f"{SHARED}:Sink:out")], "activate": True}],
            kind="ui.transfers.components.plan",
        )
        remediation = conflict[0]["error"]["details"]["remediation"]
        self.assertEqual(remediation["action"], "revise_conflicting_component")
        self.assertEqual(remediation["target"]["component_ids"], [existing_id])
        self.assertEqual(remediation["target"]["transaction_ids"], ["in", "out"])
        self.assertEqual(conflict[1]["error"]["details"]["remediation"]["action"],
                         "merge_planned_components")
        self.assertEqual(conflict[2]["error"]["code"], "ambiguous_reference")
        self.assertEqual(conflict[3]["error"]["code"], "reference_qualifier_unmatched")

        # A draft revision, then a second one on the same lineage.
        self.apply({"action": "revise", "component_id": existing_id,
                    "spec": {"notes": "draft", "legs": spec("draft")["legs"]},
                    "activate": False, "reason": "check"})
        drafts = self.masked(
            [{"action": "revise", "component_id": existing_id,
              "spec": {"notes": "again", "legs": spec("again")["legs"]}, "activate": False,
              "reason": "again"}],
            kind="ui.transfers.components.plan",
        )
        self.assertEqual(drafts[0]["error"]["code"], "custody_component_draft_exists")
        self.assertEqual(drafts[0]["error"]["details"]["remediation"]["action"],
                         "revise_existing_draft")

        # A separate draft on the same anchors cannot activate over the
        # active component; activation names the conflict.
        draft = self.apply({"components": [spec("separate draft")], "activate": False})
        draft_id = draft["components"][0]["id"]
        version = self.version()
        activation = self.masked(
            [{"action": "activate", "component_id": draft_id, "expected_input_version": version}],
            kind="ui.transfers.components.apply",
        )
        error = activation[0]["error"]
        self.assertEqual(error["code"], "custody_component_incomplete")
        self.assertEqual(error["details"]["remediation"]["target"]["component_ids"], [existing_id])
        self.compare([{}, {"transaction": f"{SHARED}:out"}], kind="ui.transfers.components.list")
        self.compare([{"component_id": draft_id}, {"component_id": existing_id}],
                     kind="ui.transfers.components.get")


    def test_non_text_actions_are_refused_like_python(self):
        """A list or object action is validation, after the scope and before
        the apply's input version (Python fix b2c0d9447)."""
        refused = self.compare(
            [{"action": ["create"], "components": [spec("x")]}, {"action": {"a": 1}},
             {"action": None}, {"action": 7}, {"action": "explode"},
             {"action": [], "profile": "missing"}],
            kind="ui.transfers.components.plan",
        )
        self.assertEqual(refused[0]["error"], {
            "code": "validation", "message": "Custody component review action is unsupported",
            "hint": None, "details": None, "retryable": False, "debug": None})
        self.assertEqual(refused[4]["error"]["details"], {"action": "explode"})
        applied = self.compare(
            [{"action": [], "expected_input_version": "stale"},
             {"action": None, "expected_input_version": -1},
             {"action": "explode", "expected_input_version": "stale"}],
            kind="ui.transfers.components.apply",
        )
        self.assertEqual(applied[0]["error"]["message"], "Custody component review action is unsupported")
        self.assertIn("input version", applied[2]["error"]["message"])


class EncryptedComponentRemediationParity(ComponentRemediationParity):
    PASSPHRASE = "sync-s11-passphrase"


if __name__ == "__main__":
    unittest.main()
